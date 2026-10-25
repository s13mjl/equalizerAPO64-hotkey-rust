//! Port of `ReceiveThread.{h,cpp}`.
//!
//! A message-mode named pipe server running on its own thread. The APO writes
//! one JSON message per device test stage to the pipe; `wait_until` mirrors the
//! C++ condition-variable based receiver with an absolute deadline.

use std::ffi::c_void;
use std::sync::{Arc, Condvar, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use eapo_common::string_helper::get_system_error_string;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, ERROR_PIPE_CONNECTED, GENERIC_WRITE, INVALID_HANDLE_VALUE,
};
use windows::Win32::Security::{
    InitializeSecurityDescriptor, SetSecurityDescriptorDacl, PSECURITY_DESCRIPTOR,
    SECURITY_ATTRIBUTES, SECURITY_DESCRIPTOR,
};
use windows::Win32::System::SystemServices::SECURITY_DESCRIPTOR_REVISION;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, ReadFile, WriteFile, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_MODE,
    OPEN_EXISTING, PIPE_ACCESS_INBOUND,
};
use windows::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, DisconnectNamedPipe, WaitNamedPipeW,
    PIPE_READMODE_MESSAGE, PIPE_REJECT_REMOTE_CLIENTS, PIPE_TYPE_MESSAGE, PIPE_WAIT,
};

#[derive(Debug, Clone)]
pub struct ReceiveException(pub String);

impl ReceiveException {
    #[allow(dead_code)]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    pub fn get_message(&self) -> String {
        self.0.clone()
    }
}

struct State {
    answers: Vec<String>,
    caught_exception: Option<ReceiveException>,
}

struct Inner {
    pipe_name: Mutex<String>,
    state: Mutex<State>,
    cond: Condvar,
}

pub struct ReceiveThread {
    inner: Arc<Inner>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl ReceiveThread {
    pub fn new(pipe_name: &str) -> Self {
        let inner = Arc::new(Inner {
            pipe_name: Mutex::new(pipe_name.to_string()),
            state: Mutex::new(State {
                answers: Vec::new(),
                caught_exception: None,
            }),
            cond: Condvar::new(),
        });

        let thread_inner = inner.clone();
        let handle = thread::spawn(move || thread_inner.run());

        Self {
            inner,
            thread: Mutex::new(Some(handle)),
        }
    }

    /// Blocking read with an absolute deadline. An empty string means timeout.
    pub fn wait_until(&self, deadline: Instant) -> Result<String, ReceiveException> {
        let mut state = self.inner.state.lock().unwrap();
        while state.answers.is_empty() && state.caught_exception.is_none() {
            let now = Instant::now();
            if now >= deadline {
                break;
            }
            let (next, _timeout) = self.inner.cond.wait_timeout(state, deadline - now).unwrap();
            state = next;
        }

        if let Some(exception) = &state.caught_exception {
            return Err(exception.clone());
        }

        if state.answers.is_empty() {
            Ok(String::new())
        } else {
            Ok(state.answers.remove(0))
        }
    }

    pub fn stop(&self) {
        let pipe_name = {
            let mut name = self.inner.pipe_name.lock().unwrap();
            std::mem::take(&mut *name)
        };
        if !pipe_name.is_empty() {
            let full_name = format!(r"\\.\pipe\{}", pipe_name);
            let wide: Vec<u16> = full_name.encode_utf16().chain(std::iter::once(0)).collect();
            unsafe {
                let mut pipe = CreateFileW(
                    PCWSTR(wide.as_ptr()),
                    GENERIC_WRITE.0,
                    FILE_SHARE_MODE(0),
                    None,
                    OPEN_EXISTING,
                    FILE_ATTRIBUTE_NORMAL,
                    None,
                );
                if pipe.is_err() && WaitNamedPipeW(PCWSTR(wide.as_ptr()), 1000).as_bool() {
                    pipe = CreateFileW(
                        PCWSTR(wide.as_ptr()),
                        GENERIC_WRITE.0,
                        FILE_SHARE_MODE(0),
                        None,
                        OPEN_EXISTING,
                        FILE_ATTRIBUTE_NORMAL,
                        None,
                    );
                }
                if let Ok(pipe) = pipe {
                    let mut written = 0u32;
                    let _ = WriteFile(pipe, Some(b"stop"), Some(&mut written), None);
                    let _ = FlushFileBuffers(pipe);
                    let _ = CloseHandle(pipe);
                }
            }
        }

        if let Some(handle) = self.thread.lock().unwrap().take() {
            let _ = handle.join();
        }
    }
}

impl Drop for ReceiveThread {
    fn drop(&mut self) {
        self.stop();
    }
}

impl Inner {
    fn set_exception(&self, exception: ReceiveException) {
        let mut state = self.state.lock().unwrap();
        state.caught_exception = Some(exception);
        self.cond.notify_all();
    }

    fn run(&self) {
        let pipe_name = self.pipe_name.lock().unwrap().clone();
        if pipe_name.is_empty() {
            return;
        }

        let mut descriptor = SECURITY_DESCRIPTOR::default();
        let sd = PSECURITY_DESCRIPTOR(&mut descriptor as *mut _ as *mut c_void);
        unsafe {
            if InitializeSecurityDescriptor(sd, SECURITY_DESCRIPTOR_REVISION).is_err() {
                self.set_exception(ReceiveException(format!(
                    "Could not initialize security descriptor: {}",
                    get_system_error_string(GetLastError().0 as i32)
                )));
                return;
            }
            if SetSecurityDescriptorDacl(sd, true, None, false).is_err() {
                self.set_exception(ReceiveException(format!(
                    "Could not set security descriptor DACL: {}",
                    get_system_error_string(GetLastError().0 as i32)
                )));
                return;
            }
        }

        let attributes = SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: sd.0,
            bInheritHandle: false.into(),
        };

        let full_name = format!(r"\\.\pipe\{}", pipe_name);
        let wide: Vec<u16> = full_name.encode_utf16().chain(std::iter::once(0)).collect();
        let mut buffer = [0u8; 1024];

        loop {
            let pipe = unsafe {
                CreateNamedPipeW(
                    PCWSTR(wide.as_ptr()),
                    PIPE_ACCESS_INBOUND,
                    PIPE_TYPE_MESSAGE
                        | PIPE_READMODE_MESSAGE
                        | PIPE_WAIT
                        | PIPE_REJECT_REMOTE_CLIENTS,
                    255, // PIPE_UNLIMITED_INSTANCES
                    0,
                    buffer.len() as u32,
                    0,
                    Some(&attributes),
                )
            };
            if pipe == INVALID_HANDLE_VALUE {
                self.set_exception(ReceiveException(format!(
                    "Could not create named pipe: {}",
                    get_system_error_string(unsafe { GetLastError() }.0 as i32)
                )));
                break;
            }

            let connected = match unsafe { ConnectNamedPipe(pipe, None) } {
                Ok(()) => true,
                Err(error) => {
                    let code = error.code().0 as u32;
                    let win32 = if (code >> 16) & 0x1fff == 7 {
                        code & 0xffff
                    } else {
                        code
                    };
                    win32 == ERROR_PIPE_CONNECTED.0
                }
            };
            if !connected {
                unsafe {
                    let _ = CloseHandle(pipe);
                }
                continue;
            }

            let mut bytes_read = 0u32;
            let read_result =
                unsafe { ReadFile(pipe, Some(&mut buffer[..]), Some(&mut bytes_read), None) };
            if read_result.is_err() || bytes_read == 0 {
                unsafe {
                    let _ = DisconnectNamedPipe(pipe);
                    let _ = CloseHandle(pipe);
                }
                self.set_exception(ReceiveException(format!(
                    "Could not read from pipe: {}",
                    get_system_error_string(unsafe { GetLastError() }.0 as i32)
                )));
                break;
            }

            let message = String::from_utf8_lossy(&buffer[..bytes_read as usize]).to_string();
            unsafe {
                let _ = DisconnectNamedPipe(pipe);
                let _ = CloseHandle(pipe);
            }

            if message == "stop" {
                break;
            }

            let mut state = self.state.lock().unwrap();
            state.answers.push(message);
            self.cond.notify_all();
        }

        *self.pipe_name.lock().unwrap() = String::new();
    }
}
