//! Port of the Equalizer APO `VoicemeeterClient`.
//!
//! Loads `VoicemeeterRemote64.dll` from the Voicemeeter installation directory,
//! registers the output insert audio callback and runs one `FilterEngine` per
//! Voicemeeter output bus (A1..A5) that was selected on the command line.
//! The process has no window; it only runs a Win32 thread message loop with a
//! 500 ms timer, exactly like the original application.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use std::ffi::{c_char, c_void, CStr};

use eapo_common::registry_helper;
use eapo_engine::FilterEngine;
use libloading::{Library, Symbol};
use windows::core::HSTRING;
use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Threading::{GetCurrentThreadId, Sleep};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, KillTimer, MessageBoxW, PostThreadMessageW, SetTimer, MB_APPLMODAL, MB_ICONERROR,
    MB_OK, MSG, WM_COMMAND, WM_QUIT, WM_TIMER,
};

const VOICEMEETER_KEY_PATH: &str =
    r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\VB:Voicemeeter {17359A74-1236-5467}";
const VOICEMEETER_WOW_KEY_PATH: &str =
    r"HKEY_LOCAL_MACHINE\SOFTWARE\Wow6432Node\Microsoft\Windows\CurrentVersion\Uninstall\VB:Voicemeeter {17359A74-1236-5467}";
const UNINSTALL_STRING_VALUE_NAME: &str = "UninstallString";
#[cfg(target_pointer_width = "64")]
const VOICEMEETER_REMOTE_FILE_NAME: &str = "VoicemeeterRemote64.dll";
#[cfg(not(target_pointer_width = "64"))]
const VOICEMEETER_REMOTE_FILE_NAME: &str = "VoicemeeterRemote.dll";

const VOICEMEETER_CLIENT_KEY_PATH: &str = r"HKEY_CURRENT_USER\SOFTWARE\EqualizerAPO\Voicemeeter Client";
const SAMPLE_RATE_VALUE_NAME: &str = "sampleRate";

const IDM_RESTART: usize = 200;

const VBVMR_CBCOMMAND_STARTING: i32 = 1;
const VBVMR_CBCOMMAND_ENDING: i32 = 2;
const VBVMR_CBCOMMAND_CHANGE: i32 = 3;
const VBVMR_CBCOMMAND_BUFFER_OUT: i32 = 11;

const VBVMR_AUDIOCALLBACK_OUT: i32 = 0x00000002;

/// `VBVMR_T_AUDIOINFO`
#[repr(C)]
struct AudioInfo {
    samplerate: i32,
    nb_sample_per_frame: i32,
}

/// `VBVMR_T_AUDIOBUFFER`
#[repr(C)]
struct AudioBuffer {
    audiobuffer_sr: i32,
    audiobuffer_nbs: i32,
    audiobuffer_nbi: i32,
    audiobuffer_nbo: i32,
    audiobuffer_r: [*mut f32; 128],
    audiobuffer_w: [*mut f32; 128],
}

type LoginFn = unsafe extern "system" fn() -> i32;
type LogoutFn = unsafe extern "system" fn() -> i32;
type GetVoicemeeterTypeFn = unsafe extern "system" fn(*mut i32) -> i32;
type IsParametersDirtyFn = unsafe extern "system" fn() -> i32;
type AudioCallback = unsafe extern "system" fn(*mut c_void, i32, *mut c_void, i32) -> i32;
type AudioCallbackRegisterFn =
    unsafe extern "system" fn(i32, AudioCallback, *mut c_void, *mut c_char) -> i32;
type AudioCallbackStartFn = unsafe extern "system" fn() -> i32;
type AudioCallbackStopFn = unsafe extern "system" fn() -> i32;
type AudioCallbackUnregisterFn = unsafe extern "system" fn() -> i32;

struct VmInterface {
    login: LoginFn,
    logout: LogoutFn,
    get_voicemeeter_type: GetVoicemeeterTypeFn,
    is_parameters_dirty: IsParametersDirtyFn,
    audio_callback_register: AudioCallbackRegisterFn,
    audio_callback_start: AudioCallbackStartFn,
    #[allow(dead_code)]
    audio_callback_stop: AudioCallbackStopFn,
    audio_callback_unregister: AudioCallbackUnregisterFn,
}

pub struct InitError(pub String);

struct VoicemeeterClient {
    outputs: Vec<String>,
    main_thread_id: u32,
    vmr: VmInterface,
    timer: usize,
    connected: bool,
    sample_rate: f32,
    max_frame_count: u32,
    engines: Vec<Option<FilterEngine>>,
    idle_sample_counts: Vec<i32>,
    /// Keeps the DLL loaded for the lifetime of the process, like the
    /// original which never calls `FreeLibrary`.
    _library: Library,
}

fn main() {
    let outputs: Vec<String> = std::env::args().skip(1).collect();

    match VoicemeeterClient::create(outputs) {
        Ok(mut client) => {
            // `create` already registered the callback with a pointer to the
            // boxed client. Box allocation is stable for the whole program.
            client.run();
        }
        Err(error) => {
            let message = HSTRING::from(error.0);
            let title = HSTRING::from("Equalizer APO Voicemeeter Client Initialization Error");
            unsafe {
                MessageBoxW(
                    None,
                    &message,
                    &title,
                    MB_APPLMODAL | MB_OK | MB_ICONERROR,
                );
            }
            std::process::exit(-1);
        }
    }
}

impl VoicemeeterClient {
    fn create(outputs: Vec<String>) -> Result<Box<Self>, InitError> {
        let main_thread_id = unsafe { GetCurrentThreadId() };

        let mut voicemeeter_directory = String::new();
        if registry_helper::key_exists(VOICEMEETER_KEY_PATH) {
            voicemeeter_directory =
                registry_helper::read_value(VOICEMEETER_KEY_PATH, UNINSTALL_STRING_VALUE_NAME)
                    .unwrap_or_default();
        } else if registry_helper::key_exists(VOICEMEETER_WOW_KEY_PATH) {
            voicemeeter_directory = registry_helper::read_value(
                VOICEMEETER_WOW_KEY_PATH,
                UNINSTALL_STRING_VALUE_NAME,
            )
            .unwrap_or_default();
        }

        let directory = match voicemeeter_directory.rfind('\\') {
            Some(index) => voicemeeter_directory[..index].to_string(),
            None => return Err(InitError("Voicemeeter is not installed".to_string())),
        };

        let dll_path = format!("{}\\{}", directory, VOICEMEETER_REMOTE_FILE_NAME);
        let library = unsafe { Library::new(&dll_path) }.map_err(|_| {
            InitError(format!(
                "Failed to load {}",
                VOICEMEETER_REMOTE_FILE_NAME
            ))
        })?;

        unsafe {
            macro_rules! load_proc {
                ($name:literal, $ty:ty) => {{
                    let symbol: Symbol<$ty> = library.get(concat!($name, "\0").as_bytes()).map_err(
                        |_| {
                            InitError(format!(
                                "Did not find function \"{}\" in {}",
                                $name, VOICEMEETER_REMOTE_FILE_NAME
                            ))
                        },
                    )?;
                    *symbol
                }};
            }

            let vmr = VmInterface {
                login: load_proc!("VBVMR_Login", LoginFn),
                logout: load_proc!("VBVMR_Logout", LogoutFn),
                get_voicemeeter_type: load_proc!(
                    "VBVMR_GetVoicemeeterType",
                    GetVoicemeeterTypeFn
                ),
                is_parameters_dirty: load_proc!(
                    "VBVMR_IsParametersDirty",
                    IsParametersDirtyFn
                ),
                audio_callback_register: load_proc!(
                    "VBVMR_AudioCallbackRegister",
                    AudioCallbackRegisterFn
                ),
                audio_callback_start: load_proc!(
                    "VBVMR_AudioCallbackStart",
                    AudioCallbackStartFn
                ),
                audio_callback_stop: load_proc!("VBVMR_AudioCallbackStop", AudioCallbackStopFn),
                audio_callback_unregister: load_proc!(
                    "VBVMR_AudioCallbackUnregister",
                    AudioCallbackUnregisterFn
                ),
            };

            let mut client = Box::new(VoicemeeterClient {
                outputs,
                main_thread_id,
                vmr,
                timer: 0,
                connected: true,
                sample_rate: 0.0,
                max_frame_count: 0,
                engines: Vec::new(),
                idle_sample_counts: Vec::new(),
                _library: library,
            });

            let this = &mut *client as *mut VoicemeeterClient;
            client.init_software(this)?;
            Ok(client)
        }
    }

    fn init_software(&mut self, this: *mut VoicemeeterClient) -> Result<(), InitError> {
        let rep = unsafe { (self.vmr.login)() };
        if rep < 0 {
            return Err(InitError("Failed To Login".to_string()));
        }
        if unsafe { (self.vmr.is_parameters_dirty)() } == 0 {
            self.detect_voicemeeter_type();
        } else {
            self.connected = false;
        }

        let mut tries = 30u32;
        loop {
            let mut client_name = [0i8; 64];
            let name_bytes = b"Equalizer APO";
            for (index, byte) in name_bytes.iter().enumerate() {
                client_name[index] = *byte as c_char;
            }
            let rep = unsafe {
                (self.vmr.audio_callback_register)(
                    VBVMR_AUDIOCALLBACK_OUT,
                    audio_callback,
                    this as *mut c_void,
                    client_name.as_mut_ptr(),
                )
            };
            if rep == 1 {
                let registered_by = unsafe { CStr::from_ptr(client_name.as_ptr()) }
                    .to_string_lossy()
                    .to_string();
                return Err(InitError(format!(
                    "Voicemeeter Output Insert already in use by:\n{}",
                    registered_by
                )));
            } else if rep != 0 {
                if tries > 1 {
                    // Sometimes fails temporarily after restarting VoicemeeterClient.
                    unsafe { Sleep(100) };
                    tries -= 1;
                    continue;
                }
                return Err(InitError("Failed to register audio callback".to_string()));
            }
            break;
        }
        Ok(())
    }

    fn run(&mut self) {
        unsafe {
            self.timer = SetTimer(None, 0, 500, None);
            let _ = PostThreadMessageW(
                self.main_thread_id,
                WM_COMMAND,
                WPARAM(IDM_RESTART),
                LPARAM(0),
            );

            let mut msg = MSG::default();
            loop {
                let result = GetMessageW(&mut msg, None, 0, 0);
                if result.0 <= 0 {
                    break;
                }
                if msg.message == WM_QUIT {
                    break;
                }
                match msg.message {
                    WM_COMMAND => {
                        self.handle_command(msg.wParam);
                    }
                    WM_TIMER => {
                        if msg.wParam.0 == self.timer {
                            // Check if the Voicemeeter type has changed.
                            if (self.vmr.is_parameters_dirty)() >= 0 {
                                if !self.connected {
                                    self.detect_voicemeeter_type();
                                }
                            } else {
                                // Voicemeeter has been shut down.
                                self.connected = false;
                            }
                        }
                    }
                    _ => {}
                }
            }
        }
    }

    fn handle(&mut self, n_command: i32, lp_data: *mut c_void) {
        match n_command {
            VBVMR_CBCOMMAND_STARTING => {
                let audio_info = unsafe { &*(lp_data as *const AudioInfo) };
                self.sample_rate = audio_info.samplerate as f32;
                self.max_frame_count = audio_info.nb_sample_per_frame as u32;
                for engine in self.engines.iter_mut().flatten() {
                    engine.initialize(
                        self.sample_rate,
                        8,
                        8,
                        8,
                        0,
                        self.max_frame_count,
                        "",
                    );
                }
                save_voicemeeter_sample_rate(audio_info.samplerate as u32);
            }
            VBVMR_CBCOMMAND_ENDING => {}
            VBVMR_CBCOMMAND_CHANGE => {
                unsafe {
                    let _ = PostThreadMessageW(
                        self.main_thread_id,
                        WM_COMMAND,
                        WPARAM(IDM_RESTART),
                        LPARAM(0),
                    );
                }
            }
            VBVMR_CBCOMMAND_BUFFER_OUT => {
                let audio_buffer = unsafe { &*(lp_data as *const AudioBuffer) };
                let nbi = audio_buffer.audiobuffer_nbi;
                let nbo = audio_buffer.audiobuffer_nbo;
                let n = nbi.min(nbo) as usize / 8;
                let frames = audio_buffer.audiobuffer_nbs;

                for i in 0..n {
                    let engine = self.engines.get_mut(i).and_then(|engine| engine.as_mut());
                    let mut idle = true;
                    let mut input_silent = true;
                    if let Some(engine) = &engine {
                        unsafe {
                            input_silent = is_buffer_silent(
                                audio_buffer.audiobuffer_r.as_ptr().add(8 * i),
                                frames,
                            );
                        }
                        idle = input_silent
                            && self.idle_sample_counts.get(i).copied().unwrap_or(0)
                                > (10.0 * engine.get_sample_rate()) as i32;
                    }

                    // Avoid processing when idle (Voicemeeter still calls this
                    // when no audio is played).
                    if !idle {
                        if let Some(engine) = engine {
                            unsafe {
                                engine.process_f32(
                                    audio_buffer.audiobuffer_w.as_ptr().add(8 * i) as *mut *mut f32,
                                    audio_buffer.audiobuffer_r.as_ptr().add(8 * i) as *mut *mut f32,
                                    frames as u32,
                                );
                            }
                            let output_silent = unsafe {
                                is_buffer_silent(
                                    audio_buffer.audiobuffer_w.as_ptr().add(8 * i),
                                    frames,
                                )
                            };
                            if input_silent && output_silent {
                                if let Some(count) = self.idle_sample_counts.get_mut(i) {
                                    *count += frames;
                                }
                            } else if let Some(count) = self.idle_sample_counts.get_mut(i) {
                                *count = 0;
                            }
                        }
                    } else {
                        for j in 0..8 {
                            unsafe {
                                std::ptr::copy_nonoverlapping(
                                    *audio_buffer.audiobuffer_r.get_unchecked(8 * i + j),
                                    *audio_buffer.audiobuffer_w.get_unchecked(8 * i + j),
                                    frames as usize,
                                );
                            }
                        }
                    }
                }
            }
            _ => {}
        }
    }

    fn detect_voicemeeter_type(&mut self) {
        let mut vm_type: i32 = 0;
        let rep = unsafe { (self.vmr.get_voicemeeter_type)(&mut vm_type) };
        if rep == 0 {
            self.connected = true;

            let output_count = if vm_type == 3 {
                5
            } else if vm_type == 2 {
                3
            } else {
                1
            };

            if output_count != self.engines.len() {
                self.idle_sample_counts.clear();
                self.engines.clear();

                for i in 0..output_count {
                    let output = format!("Output A{}", i + 1);
                    if self.outputs.iter().any(|candidate| candidate == &output) {
                        let mut engine = FilterEngine::new();
                        engine.set_device_info(
                            false,
                            true,
                            "Voicemeeter",
                            &output,
                            "",
                            &format!("Voicemeeter {}", output),
                        );
                        if self.sample_rate != 0.0f32 && self.max_frame_count != 0 {
                            engine.initialize(
                                self.sample_rate,
                                8,
                                8,
                                8,
                                0,
                                self.max_frame_count,
                                "",
                            );
                        }
                        self.engines.push(Some(engine));
                    } else {
                        self.engines.push(None);
                    }
                    self.idle_sample_counts.push(0);
                }
            }
        }
    }

    fn end_software(&mut self) {
        unsafe {
            let _ = (self.vmr.logout)();
            let _ = (self.vmr.audio_callback_unregister)();
        }
    }

    fn handle_command(&mut self, wparam: WPARAM) {
        if (wparam.0 as u32) & 0xffff == IDM_RESTART as u32 {
            unsafe { Sleep(50) };
            unsafe {
                let _ = (self.vmr.audio_callback_start)();
            }
        }
    }
}

impl Drop for VoicemeeterClient {
    fn drop(&mut self) {
        if self.timer != 0 {
            unsafe {
                let _ = KillTimer(None, self.timer);
            }
            self.timer = 0;
        }
        self.end_software();
    }
}

fn save_voicemeeter_sample_rate(sample_rate: u32) {
    let _ = registry_helper::create_key(VOICEMEETER_CLIENT_KEY_PATH);
    let _ = registry_helper::write_dword_value(
        VOICEMEETER_CLIENT_KEY_PATH,
        SAMPLE_RATE_VALUE_NAME,
        sample_rate,
    );
}

unsafe fn is_buffer_silent(sample_data: *const *mut f32, sample_count: i32) -> bool {
    for j in 0..8 {
        let buf = *sample_data.add(j);
        if buf.is_null() {
            continue;
        }
        for k in 0..sample_count as isize {
            if *buf.offset(k) != 0.0f32 {
                return false;
            }
        }
    }
    true
}

unsafe extern "system" fn audio_callback(
    lp_user: *mut c_void,
    n_command: i32,
    lp_data: *mut c_void,
    _nnn: i32,
) -> i32 {
    // The user pointer is the stable Box<VoicemeeterClient> allocation created
    // in `create`. Voicemeeter guarantees the callback is non-reentrant.
    let _ = std::panic::catch_unwind(|| {
        let client = &mut *(lp_user as *mut VoicemeeterClient);
        client.handle(n_command, lp_data);
    });
    0
}
