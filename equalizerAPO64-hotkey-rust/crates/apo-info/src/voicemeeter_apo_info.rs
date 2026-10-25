//! Port of `VoicemeeterAPOInfo.h/.cpp` (518 lines).

use std::any::Any;
use std::fmt;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

use eapo_common::channel_helper::KSAUDIO_SPEAKER_7POINT1_SURROUND;
use eapo_common::registry_helper::RegistryException;
use eapo_common::string_helper::to_lower_case;
use eapo_common::USER_REGPATH;
use windows::core::Interface;
use windows::core::{GUID, HSTRING, PCSTR, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, FreeLibrary, LocalFree, HANDLE, HLOCAL, LPARAM, WPARAM,
};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, SE_DEBUG_NAME, SE_PRIVILEGE_ENABLED,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::Storage::FileSystem::DeleteFileW;
use windows::Win32::System::Com::{
    CoCreateInstance, CoTaskMemFree, IPersistFile, CLSCTX_INPROC_SERVER, STGM_READ,
};
use windows::Win32::System::Diagnostics::Debug::ReadProcessMemory;
use windows::Win32::System::Diagnostics::ToolHelp::{
    CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, Thread32First, Thread32Next,
    PROCESSENTRY32W, TH32CS_SNAPPROCESS, TH32CS_SNAPTHREAD, THREADENTRY32,
};
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, GetProcAddress, LoadLibraryW};
use windows::Win32::System::Threading::GetCurrentProcess;
use windows::Win32::System::Threading::{
    OpenProcess, OpenProcessToken, PEB, PROCESS_BASIC_INFORMATION, PROCESS_QUERY_INFORMATION,
    PROCESS_TERMINATE, PROCESS_VM_READ, RTL_USER_PROCESS_PARAMETERS,
};
use windows::Win32::UI::Shell::{
    CommandLineToArgvW, FOLDERID_Startup, IShellLinkW, PathRemoveFileSpecW, SHGetKnownFolderPath,
    ShellExecuteW, KF_FLAG_DONT_UNEXPAND,
};
use windows::Win32::UI::WindowsAndMessaging::{PostThreadMessageW, SW_SHOWDEFAULT, WM_QUIT};

use crate::abstract_apo_info::AbstractAPOInfo;
use crate::registry;

const VOICEMEETER_KEY_PATH: &str = r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\VB:Voicemeeter {17359A74-1236-5467}";
const VOICEMEETER_WOW_KEY_PATH: &str = r"HKEY_LOCAL_MACHINE\SOFTWARE\Wow6432Node\Microsoft\Windows\CurrentVersion\Uninstall\VB:Voicemeeter {17359A74-1236-5467}";
const UNINSTALL_STRING_VALUE_NAME: &str = "UninstallString";
const STARTUP_FILENAME: &str = "Equalizer APO Voicemeeter Client.lnk";
const CLIENT_FILENAME: &str = "VoicemeeterClient.exe";
const SAMPLE_RATE_VALUE_NAME: &str = "sampleRate";
const MAX_PATH: usize = 260;

fn voicemeeter_client_key_path() -> String {
    format!(r"{USER_REGPATH}\Voicemeeter Client")
}

const CLSID_SHELL_LINK: GUID = GUID::from_u128(0x00021401_0000_0000_c000_000000000046);

/// Port of the plain `std::exception`-derived failures of
/// `ensureVoicemeeterClientRunning`.
#[derive(Debug, Clone)]
pub struct VoicemeeterException {
    message: String,
}

impl VoicemeeterException {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for VoicemeeterException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for VoicemeeterException {}

/// Port of `VoicemeeterAPOInfo`.
pub struct VoicemeeterAPOInfo {
    connection_name: String,
    sample_rate: u32,
    installed: bool,
    default_device: AtomicBool,
    voicemeeter_installed: bool,
    changes: bool,
}

impl VoicemeeterAPOInfo {
    /// Port of the `prependInfos` static.
    pub fn prepend_infos(
        list: &mut Vec<Arc<dyn AbstractAPOInfo>>,
    ) -> Result<(), RegistryException> {
        let mut voicemeeter_directory = String::new();
        if registry::key_exists(VOICEMEETER_KEY_PATH) {
            voicemeeter_directory =
                registry::read_value(VOICEMEETER_KEY_PATH, UNINSTALL_STRING_VALUE_NAME)?;
        } else if registry::key_exists(VOICEMEETER_WOW_KEY_PATH) {
            voicemeeter_directory =
                registry::read_value(VOICEMEETER_WOW_KEY_PATH, UNINSTALL_STRING_VALUE_NAME)?;
        }

        if !voicemeeter_directory.is_empty() {
            let index = voicemeeter_directory.rfind('\\');
            let setup_filename = to_lower_case(match index {
                Some(index) => &voicemeeter_directory[index + 1..],
                None => voicemeeter_directory.as_str(),
            });

            let voicemeeter_type = if setup_filename == "voicemeeterprosetup.exe" {
                2 // banana
            } else if setup_filename == "voicemeeter8setup.exe" {
                3 // potato
            } else {
                1
            };

            let output_count: usize = match voicemeeter_type {
                3 => 5,
                2 => 3,
                _ => 1,
            };

            let mut default_device = false;
            list.retain(|info| {
                if info.get_device_name().contains("VB-Audio VoiceMeeter") {
                    if info.is_default_device() {
                        default_device = true;
                    }
                    return false;
                }
                true
            });

            let mut any_installed = false;
            for i in 0..output_count {
                let info = Arc::new(VoicemeeterAPOInfo::new(format!("Output A{}", i + 1), true));
                if info.is_installed() {
                    any_installed = true;
                }
                list.insert(i, info);
            }

            if default_device {
                for i in 0..output_count {
                    if let Some(info) = list[i].as_any().downcast_ref::<VoicemeeterAPOInfo>() {
                        if !any_installed || info.is_installed() {
                            info.set_default_device(true);
                            break;
                        }
                    }
                }
            }
        } else {
            // Voicemeeter was uninstalled but Voicemeeter Client might still be installed
            let startup_file_path = get_startup_path();
            let arg_string = get_link_args(&startup_file_path, None);
            let args = split_args(&arg_string);

            for (i, arg) in args.into_iter().enumerate() {
                list.insert(i, Arc::new(VoicemeeterAPOInfo::new(arg, false)));
            }
        }

        Ok(())
    }

    pub fn new(connection_name: impl Into<String>, voicemeeter_installed: bool) -> Self {
        let connection_name = connection_name.into();
        let startup_file_path = get_startup_path();
        let mut path = String::new();
        let arg_string = get_link_args(&startup_file_path, Some(&mut path));
        let args = split_args(&arg_string);
        let installed = args.contains(&connection_name);

        let mut sample_rate = 48000u32;
        let client_key = voicemeeter_client_key_path();
        if registry::key_exists(&client_key)
            && registry::value_exists(&client_key, SAMPLE_RATE_VALUE_NAME)
        {
            // RegistryException is ignored, exactly like the C++ catch block
            if let Ok(value) = registry::read_dword_value(&client_key, SAMPLE_RATE_VALUE_NAME) {
                sample_rate = value;
            }
        }

        let client_path = get_client_path();
        let changes = path != client_path;

        Self {
            connection_name,
            sample_rate,
            installed,
            default_device: AtomicBool::new(false),
            voicemeeter_installed,
            changes,
        }
    }

    pub fn is_voicemeeter_installed(&self) -> bool {
        self.voicemeeter_installed
    }

    fn set_default_device(&self, value: bool) {
        self.default_device.store(value, Ordering::Relaxed);
    }

    /// Port of `ensureVoicemeeterClientRunning`.
    pub fn ensure_voicemeeter_client_running() -> Result<(), VoicemeeterException> {
        let startup_file_path = get_startup_path();
        let arg_string = get_link_args(&startup_file_path, None);
        let args = split_args(&arg_string);
        let client_path = get_client_path();

        let mut token_handle = HANDLE::default();
        unsafe {
            OpenProcessToken(
                GetCurrentProcess(),
                TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
                &mut token_handle,
            )
        }
        .map_err(|_| {
            VoicemeeterException::new("Error in OpenProcessToken while taking ownership")
        })?;
        let token_guard = HandleGuard(token_handle);

        let mut luid = windows::Win32::Foundation::LUID::default();
        unsafe { LookupPrivilegeValueW(PCWSTR::null(), SE_DEBUG_NAME, &mut luid) }.map_err(
            |_| VoicemeeterException::new("Error in LookupPrivilegeValue while taking ownership"),
        )?;

        let mut tp = TOKEN_PRIVILEGES::default();
        tp.PrivilegeCount = 1;
        tp.Privileges[0].Luid = luid;
        tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;

        unsafe {
            AdjustTokenPrivileges(
                token_guard.0,
                false,
                Some(&tp),
                std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
                None,
                None,
            )
        }
        .map_err(|_| {
            VoicemeeterException::new("Error in AdjustTokenPrivileges while taking ownership")
        })?;

        let module = unsafe { LoadLibraryW(windows::core::w!("ntdll.dll")) }
            .map_err(|_| VoicemeeterException::new("Could not load ntdll.dll"))?;
        let module_guard = ModuleGuard(module);

        let proc = unsafe {
            GetProcAddress(
                module_guard.0,
                PCSTR(b"NtQueryInformationProcess\0".as_ptr()),
            )
        };
        let Some(proc) = proc else {
            return Err(VoicemeeterException::new(
                "Function NtQueryInformationProcess not found in ntdll.dll",
            ));
        };
        let nt_query_information_process: NtQueryInformationProcessFn =
            unsafe { std::mem::transmute(proc) };

        let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) }
            .map_err(|_| VoicemeeterException::new("Could not take a snapshot of all processes"))?;
        let snapshot_guard = HandleGuard(snapshot);

        let mut matching_process_exists = false;

        let mut entry = PROCESSENTRY32W::default();
        entry.dwSize = std::mem::size_of::<PROCESSENTRY32W>() as u32;
        let mut more = unsafe { Process32FirstW(snapshot_guard.0, &mut entry) }.is_ok();
        while more {
            let exe_name = wide_z_to_string(&entry.szExeFile);
            if exe_name == CLIENT_FILENAME {
                let process_handle = unsafe {
                    OpenProcess(
                        PROCESS_QUERY_INFORMATION | PROCESS_VM_READ | PROCESS_TERMINATE,
                        false,
                        entry.th32ProcessID,
                    )
                }
                .map_err(|_| VoicemeeterException::new("Could not open process"))?;
                let process_guard = HandleGuard(process_handle);

                let mut basic_information = PROCESS_BASIC_INFORMATION::default();
                let status = unsafe {
                    nt_query_information_process(
                        process_guard.0,
                        0, // ProcessBasicInformation
                        &mut basic_information as *mut _ as *mut core::ffi::c_void,
                        std::mem::size_of::<PROCESS_BASIC_INFORMATION>() as u32,
                        std::ptr::null_mut(),
                    )
                };
                if status < 0 {
                    return Err(VoicemeeterException::new(
                        "Could not query process information",
                    ));
                }

                let mut peb = PEB::default();
                unsafe {
                    ReadProcessMemory(
                        process_guard.0,
                        basic_information.PebBaseAddress as *const core::ffi::c_void,
                        &mut peb as *mut _ as *mut core::ffi::c_void,
                        std::mem::size_of::<PEB>(),
                        None,
                    )
                }
                .map_err(|_| VoicemeeterException::new("Could not read peb from process memory"))?;

                let mut process_params = RTL_USER_PROCESS_PARAMETERS::default();
                unsafe {
                    ReadProcessMemory(
                        process_guard.0,
                        peb.ProcessParameters as *const core::ffi::c_void,
                        &mut process_params as *mut _ as *mut core::ffi::c_void,
                        std::mem::size_of::<RTL_USER_PROCESS_PARAMETERS>(),
                        None,
                    )
                }
                .map_err(|_| {
                    VoicemeeterException::new(
                        "Could not read process parameters from process memory",
                    )
                })?;

                let char_count = process_params.CommandLine.Length as usize / 2;
                let mut command_line_buf = vec![0u16; char_count];
                unsafe {
                    ReadProcessMemory(
                        process_guard.0,
                        process_params.CommandLine.Buffer.0 as *const core::ffi::c_void,
                        command_line_buf.as_mut_ptr() as *mut core::ffi::c_void,
                        process_params.CommandLine.Length as usize,
                        None,
                    )
                }
                .map_err(|_| {
                    VoicemeeterException::new("Could not read command line from process memory")
                })?;
                let command_line = String::from_utf16_lossy(&command_line_buf);

                let mut process_args = split_args(&command_line);
                if !process_args.is_empty() {
                    let path = process_args.remove(0);
                    if path != client_path || process_args != args {
                        close_process(entry.th32ProcessID)?;
                    } else {
                        matching_process_exists = true;
                    }
                }
            }

            more = unsafe { Process32NextW(snapshot_guard.0, &mut entry) }.is_ok();
        }

        if !matching_process_exists && !args.is_empty() {
            let file = HSTRING::from(client_path);
            let params = HSTRING::from(arg_string);
            unsafe {
                ShellExecuteW(
                    None,
                    PCWSTR::null(),
                    &file,
                    &params,
                    PCWSTR::null(),
                    SW_SHOWDEFAULT,
                );
            }
        }

        Ok(())
    }

    /// Port of `saveVoicemeeterSampleRate`.
    pub fn save_voicemeeter_sample_rate(sample_rate: u32) {
        let key = voicemeeter_client_key_path();
        // RegistryException is ignored
        let _ = registry::create_key(&key)
            .and_then(|_| registry::write_dword_value(&key, SAMPLE_RATE_VALUE_NAME, sample_rate));
    }
}

impl AbstractAPOInfo for VoicemeeterAPOInfo {
    fn get_connection_name(&self) -> String {
        self.connection_name.clone()
    }

    fn get_device_name(&self) -> String {
        "Voicemeeter".to_string()
    }

    fn get_device_guid(&self) -> String {
        String::new()
    }

    fn get_device_string(&self) -> String {
        format!("{} {}", self.get_connection_name(), self.get_device_name())
    }

    fn get_channel_count(&self) -> u32 {
        8
    }

    fn get_sample_rate(&self) -> u32 {
        self.sample_rate
    }

    fn get_channel_mask(&self) -> u32 {
        KSAUDIO_SPEAKER_7POINT1_SURROUND
    }

    fn is_input(&self) -> bool {
        false
    }

    fn is_installed(&self) -> bool {
        self.installed
    }

    fn can_be_upgraded(&self) -> bool {
        false
    }

    fn has_changes(&self) -> bool {
        self.changes
    }

    fn is_experimental(&self) -> bool {
        false
    }

    fn is_enhancements_disabled(&self) -> bool {
        false
    }

    fn is_default_device(&self) -> bool {
        self.default_device.load(Ordering::Relaxed)
    }

    fn is_disabled(&self) -> bool {
        false
    }

    fn is_unplugged(&self) -> bool {
        false
    }

    fn install(&self) -> Result<(), RegistryException> {
        let startup_file_path = get_startup_path();
        let mut args = split_args(&get_link_args(&startup_file_path, None));
        if !args.contains(&self.connection_name) {
            args.push(self.connection_name.clone());
        }
        args.sort();
        let arg_string = join_args(&args);
        let client_path = get_client_path();
        create_link(&startup_file_path, &client_path, &arg_string);
        Ok(())
    }

    fn uninstall(&self) -> Result<(), RegistryException> {
        let startup_file_path = get_startup_path();
        let mut args = split_args(&get_link_args(&startup_file_path, None));
        if let Some(index) = args.iter().position(|arg| arg == &self.connection_name) {
            args.remove(index);
        }
        if !args.is_empty() {
            args.sort();
            let arg_string = join_args(&args);
            let client_path = get_client_path();
            create_link(&startup_file_path, &client_path, &arg_string);
        } else {
            let startup = HSTRING::from(startup_file_path);
            unsafe {
                let _ = DeleteFileW(&startup);
            }
        }
        Ok(())
    }

    fn reinstall(&self) -> Result<(), RegistryException> {
        self.uninstall()?;
        self.install()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

struct HandleGuard(HANDLE);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = CloseHandle(self.0);
            }
        }
    }
}

struct ModuleGuard(windows::Win32::Foundation::HMODULE);

impl Drop for ModuleGuard {
    fn drop(&mut self) {
        unsafe {
            let _ = FreeLibrary(self.0);
        }
    }
}

type NtQueryInformationProcessFn =
    unsafe extern "system" fn(HANDLE, u32, *mut core::ffi::c_void, u32, *mut u32) -> i32;

fn wide_z_to_string(buffer: &[u16]) -> String {
    let len = buffer.iter().position(|&c| c == 0).unwrap_or(buffer.len());
    String::from_utf16_lossy(&buffer[..len])
}

/// Port of `getStartupPath`.
fn get_startup_path() -> String {
    let startup_path =
        unsafe { SHGetKnownFolderPath(&FOLDERID_Startup, KF_FLAG_DONT_UNEXPAND, None) };
    let mut result = match startup_path {
        Ok(path) => {
            let s = unsafe { path.to_string() }.unwrap_or_default();
            unsafe {
                CoTaskMemFree(Some(path.0 as *const core::ffi::c_void));
            }
            s
        }
        Err(_) => String::new(),
    };
    result.push('\\');
    result.push_str(STARTUP_FILENAME);
    result
}

/// Port of `getClientPath`.
fn get_client_path() -> String {
    let mut filename = [0u16; MAX_PATH];
    unsafe {
        GetModuleFileNameW(None, &mut filename);
        let _ = PathRemoveFileSpecW(PWSTR(filename.as_mut_ptr()));
    }
    let len = filename.iter().position(|&c| c == 0).unwrap_or(MAX_PATH);
    let mut client_path = String::from_utf16_lossy(&filename[..len]);
    client_path.push('\\');
    client_path.push_str(CLIENT_FILENAME);
    client_path
}

/// Port of `createLink`. All COM failures are ignored, like the original.
fn create_link(lnk_path: &str, path: &str, args: &str) {
    let shell_link: windows::core::Result<IShellLinkW> =
        unsafe { CoCreateInstance(&CLSID_SHELL_LINK, None, CLSCTX_INPROC_SERVER) };
    let Ok(shell_link) = shell_link else {
        return;
    };

    let path_wide = HSTRING::from(path);
    let args_wide = HSTRING::from(args);
    unsafe {
        let _ = shell_link.SetPath(&path_wide);
        let _ = shell_link.SetArguments(&args_wide);
    }

    if let Ok(persist_file) = shell_link.cast::<IPersistFile>() {
        let lnk_wide = HSTRING::from(lnk_path);
        unsafe {
            let _ = persist_file.Save(&lnk_wide, true);
        }
    }
}

/// Port of `getLinkArgs`. Returns an empty string on any failure.
fn get_link_args(lnk_path: &str, mut path: Option<&mut String>) -> String {
    let shell_link: windows::core::Result<IShellLinkW> =
        unsafe { CoCreateInstance(&CLSID_SHELL_LINK, None, CLSCTX_INPROC_SERVER) };
    let Ok(shell_link) = shell_link else {
        return String::new();
    };
    let Ok(persist_file) = shell_link.cast::<IPersistFile>() else {
        return String::new();
    };

    let lnk_wide = HSTRING::from(lnk_path);
    if unsafe { persist_file.Load(&lnk_wide, STGM_READ) }.is_err() {
        return String::new();
    }

    let mut result = String::new();
    let mut buf = [0u16; MAX_PATH];
    if unsafe { shell_link.GetArguments(&mut buf) }.is_ok() {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(MAX_PATH);
        result = String::from_utf16_lossy(&buf[..len]);
    }

    if let Some(path) = path.as_mut() {
        let mut path_buf = [0u16; MAX_PATH];
        if unsafe { shell_link.GetPath(&mut path_buf, std::ptr::null_mut(), 0) }.is_ok() {
            let len = path_buf.iter().position(|&c| c == 0).unwrap_or(MAX_PATH);
            **path = String::from_utf16_lossy(&path_buf[..len]);
        }
    }

    result
}

/// Port of `splitArgs` via `CommandLineToArgvW`.
fn split_args(arg_string: &str) -> Vec<String> {
    let mut result = Vec::new();
    if arg_string.is_empty() {
        return result;
    }

    let arg_wide = HSTRING::from(arg_string);
    let mut argc = 0i32;
    let argv = unsafe { CommandLineToArgvW(&arg_wide, &mut argc) };
    if argv.is_null() {
        return result;
    }
    for i in 0..argc {
        let ptr = unsafe { *argv.add(i as usize) };
        result.push(unsafe { ptr.to_string() }.unwrap_or_default());
    }
    unsafe {
        LocalFree(Some(HLOCAL(argv as *mut core::ffi::c_void)));
    }
    result
}

/// Port of `joinArgs`.
fn join_args(args: &[String]) -> String {
    let mut result = String::new();
    for arg in args {
        if !result.is_empty() {
            result.push(' ');
        }
        if arg.contains(' ') {
            result.push('"');
            result.push_str(arg);
            result.push('"');
        } else {
            result.push_str(arg);
        }
    }
    result
}

/// Port of `closeProcess`; failures to post to threads are ignored.
fn close_process(process_id: u32) -> Result<(), VoicemeeterException> {
    let snapshot = unsafe { CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0) }
        .map_err(|_| VoicemeeterException::new("Could not take a snapshot of all processes"))?;
    let snapshot_guard = HandleGuard(snapshot);

    let mut entry = THREADENTRY32::default();
    entry.dwSize = std::mem::size_of::<THREADENTRY32>() as u32;
    let mut more = unsafe { Thread32First(snapshot_guard.0, &mut entry) }.is_ok();
    while more {
        if entry.th32OwnerProcessID == process_id {
            // will just fail for threads not having a message queue
            unsafe {
                let _ = PostThreadMessageW(entry.th32ThreadID, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
        more = unsafe { Thread32Next(snapshot_guard.0, &mut entry) }.is_ok();
    }
    Ok(())
}
