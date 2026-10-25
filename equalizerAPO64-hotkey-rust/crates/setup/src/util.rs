//! Small Windows helpers shared by the installer and uninstaller.

use std::ffi::{c_void, OsStr, OsString};
use std::fs;
use std::mem::size_of;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus};
use std::ptr::null_mut;

use windows::core::{s, w, PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    CloseHandle, FreeLibrary, GetLastError, LocalFree, ERROR_FILE_NOT_FOUND, ERROR_SUCCESS, HANDLE,
    HLOCAL, LUID, NTSTATUS, RPC_E_CHANGED_MODE, WIN32_ERROR,
};
use windows::Win32::Security::Authorization::{
    GetNamedSecurityInfoW, SetEntriesInAclW, SetNamedSecurityInfoW, EXPLICIT_ACCESS_W, SET_ACCESS,
    SE_FILE_OBJECT, TRUSTEE_IS_GROUP, TRUSTEE_IS_SID, TRUSTEE_W,
};
use windows::Win32::Security::{
    AdjustTokenPrivileges, CreateWellKnownSid, LookupPrivilegeValueW, WinBuiltinUsersSid, ACL,
    DACL_SECURITY_INFORMATION, LUID_AND_ATTRIBUTES, PSECURITY_DESCRIPTOR, PSID,
    SE_PRIVILEGE_ENABLED, SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_ADJUST_PRIVILEGES,
    TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::Storage::FileSystem::{
    GetDiskFreeSpaceExW, MoveFileExW, FILE_ALL_ACCESS, MOVEFILE_DELAY_UNTIL_REBOOT,
};
use windows::Win32::System::Com::{CoInitializeEx, CoTaskMemFree, COINIT_APARTMENTTHREADED};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::Shutdown::{
    InitiateShutdownW, SHTDN_REASON_MAJOR_APPLICATION, SHTDN_REASON_MINOR_MAINTENANCE,
    SHUTDOWN_FLAGS, SHUTDOWN_GRACE_OVERRIDE, SHUTDOWN_RESTART,
};
use windows::Win32::System::SystemInformation::{GetSystemDirectoryW, OSVERSIONINFOW};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::{
    FOLDERID_ProgramFiles, FOLDERID_Programs, SHBrowseForFolderW, SHGetKnownFolderPath,
    SHGetPathFromIDListW, SHParseDisplayName, ShellExecuteW, BIF_EDITBOX, BIF_NEWDIALOGSTYLE,
    BIF_RETURNONLYFSDIRS, BROWSEINFOW,
};
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_ICONSTOP, MB_ICONWARNING, MB_OK, MB_YESNO, MESSAGEBOX_RESULT,
    MESSAGEBOX_STYLE, SW_SHOWNORMAL,
};

/// Converts a string to a null-terminated UTF-16 buffer.
pub fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Converts a path to a null-terminated UTF-16 buffer.
pub fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

pub fn last_error_string() -> String {
    let code = unsafe { GetLastError().0 } as i32;
    std::io::Error::from_raw_os_error(code).to_string()
}

pub fn message_box(text: &str, caption: &str, style: MESSAGEBOX_STYLE) -> MESSAGEBOX_RESULT {
    let text = wide(text);
    let caption = wide(caption);
    unsafe { MessageBoxW(None, PCWSTR(text.as_ptr()), PCWSTR(caption.as_ptr()), style) }
}

pub fn message_error(text: &str, caption: &str) {
    message_box(text, caption, MB_OK | MB_ICONSTOP);
}

pub fn confirm_yes_no(text: &str, caption: &str) -> bool {
    message_box(text, caption, MB_YESNO | MB_ICONWARNING) == IDYES
}

/// Directory that contains the currently running executable.
pub fn current_exe_dir() -> PathBuf {
    std::env::current_exe()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| PathBuf::from("."))
}

fn known_folder(id: &windows::core::GUID) -> Option<PathBuf> {
    unsafe {
        let path = SHGetKnownFolderPath(id, Default::default(), None).ok()?;
        let result = path.to_string().ok().map(PathBuf::from);
        CoTaskMemFree(Some(path.0 as *const c_void));
        result
    }
}

/// `$PROGRAMFILES64` in this x64 installer.
pub fn program_files_dir() -> PathBuf {
    known_folder(&FOLDERID_ProgramFiles)
        .or_else(|| std::env::var_os("ProgramFiles").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from(r"C:\Program Files"))
}

/// `$SMPROGRAMS` for the current shell context (NSIS does not call
/// `SetShellVarContext all`, so this is the per-user Programs folder).
pub fn start_menu_programs_dir() -> PathBuf {
    known_folder(&FOLDERID_Programs)
        .or_else(|| {
            std::env::var_os("APPDATA").map(|appdata| {
                PathBuf::from(appdata).join(r"Microsoft\Windows\Start Menu\Programs")
            })
        })
        .unwrap_or_default()
}

/// `$SYSDIR`.
pub fn system_directory() -> PathBuf {
    let mut buffer = [0u16; 260];
    let length = unsafe { GetSystemDirectoryW(Some(&mut buffer)) } as usize;
    if length == 0 {
        return PathBuf::from(r"C:\Windows\System32");
    }
    if length >= buffer.len() {
        let mut larger = vec![0u16; length + 1];
        let copied = unsafe { GetSystemDirectoryW(Some(&mut larger)) } as usize;
        if copied == 0 || copied >= larger.len() {
            return PathBuf::from(r"C:\Windows\System32");
        }
        return PathBuf::from(String::from_utf16_lossy(&larger[..copied]));
    }
    PathBuf::from(String::from_utf16_lossy(&buffer[..length]))
}

/// Runs a program and waits for it to exit.  Used for `regsvr32.exe`,
/// `DeviceSelector.exe` and `UpdateChecker.exe` exactly like `ExecWait`.
pub fn run_and_wait(
    program: &Path,
    args: &[&OsStr],
    working_dir: Option<&Path>,
    hide_window: bool,
) -> std::io::Result<ExitStatus> {
    let mut command = Command::new(program);
    command.args(args);
    if let Some(dir) = working_dir {
        command.current_dir(dir);
    }
    if hide_window {
        // CREATE_NO_WINDOW
        command.creation_flags(0x0800_0000);
    }
    command.status()
}

/// Native "Browse for Folder" dialog used by the MUI directory page.
pub fn pick_folder(title: &str, initial: &Path) -> Option<PathBuf> {
    let _com = com_initialize_sta();
    let title_wide = wide(title);
    let initial_wide = wide_path(initial);

    let mut root = null_mut();
    let _ = unsafe { SHParseDisplayName(PCWSTR(initial_wide.as_ptr()), None, &mut root, 0, None) };

    let mut display_name = [0u16; 260];
    let browse = BROWSEINFOW {
        hwndOwner: windows::Win32::Foundation::HWND::default(),
        pidlRoot: root,
        pszDisplayName: PWSTR(display_name.as_mut_ptr()),
        lpszTitle: PCWSTR(title_wide.as_ptr()),
        ulFlags: BIF_RETURNONLYFSDIRS | BIF_NEWDIALOGSTYLE | BIF_EDITBOX,
        lpfn: None,
        lParam: windows::Win32::Foundation::LPARAM(0),
        iImage: 0,
    };

    let pidl = unsafe { SHBrowseForFolderW(&browse) };
    if !root.is_null() {
        unsafe {
            CoTaskMemFree(Some(root as *const c_void));
        }
    }
    if pidl.is_null() {
        return None;
    }

    let mut path = [0u16; 260];
    let ok = unsafe { SHGetPathFromIDListW(pidl, &mut path) }.as_bool();
    unsafe {
        CoTaskMemFree(Some(pidl as *const c_void));
    }
    if !ok {
        return None;
    }
    let length = path
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(path.len());
    Some(PathBuf::from(String::from_utf16_lossy(&path[..length])))
}

pub fn open_url(url: &str) -> Result<(), String> {
    let operation = w!("open");
    let file = wide(url);
    let result = unsafe {
        ShellExecuteW(
            None,
            operation,
            PCWSTR(file.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        )
    };
    if result.0 as usize > 32 {
        Ok(())
    } else {
        Err(format!(
            "ShellExecute(open, {url}) failed: {}",
            last_error_string()
        ))
    }
}

/// Port of the `RenameAndDelete` NSIS macro: renames a file to `.old`,
/// `.old.0`, `.old.1`, ... so an in-use file can be replaced and deleted
/// after the Device Selector has restarted the audio service.
pub fn rename_then_delete(path: &Path) -> Option<PathBuf> {
    if !path.exists() {
        return None;
    }

    let mut base = OsString::from(path.as_os_str());
    base.push(".old");
    let mut candidate = PathBuf::from(&base);
    let mut index = 0u32;
    while candidate.exists() {
        let mut name = OsString::from(path.as_os_str());
        name.push(format!(".old.{index}"));
        candidate = PathBuf::from(name);
        index += 1;
        if index > 10_000 {
            return None;
        }
    }

    clear_readonly(path);
    match fs::rename(path, &candidate) {
        Ok(()) => Some(candidate),
        Err(_) => None,
    }
}

/// Registers a file/directory for deletion at the next reboot.
pub fn schedule_delete_at_reboot(path: &Path) -> bool {
    let wide = wide_path(path);
    unsafe {
        MoveFileExW(
            PCWSTR(wide.as_ptr()),
            PCWSTR::null(),
            MOVEFILE_DELAY_UNTIL_REBOOT,
        )
        .is_ok()
    }
}

fn clear_readonly(path: &Path) {
    if let Ok(metadata) = fs::metadata(path) {
        if metadata.permissions().readonly() {
            let mut permissions = metadata.permissions();
            permissions.set_readonly(false);
            let _ = fs::set_permissions(path, permissions);
        }
    }
}

fn clear_readonly_recursive(path: &Path) {
    clear_readonly(path);
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let child = entry.path();
            if child.is_dir() {
                clear_readonly_recursive(&child);
            } else {
                clear_readonly(&child);
            }
        }
    }
}

fn schedule_tree_at_reboot(path: &Path) {
    if let Ok(entries) = fs::read_dir(path) {
        for entry in entries.flatten() {
            let child = entry.path();
            if child.is_dir() {
                schedule_tree_at_reboot(&child);
            } else {
                schedule_delete_at_reboot(&child);
            }
        }
    }
    schedule_delete_at_reboot(path);
}

/// Port of NSIS `Delete` (no /REBOOTOK): clear the read-only attribute and
/// ignore failures on missing or locked files.
pub fn delete_file(path: &Path) -> bool {
    clear_readonly(path);
    match fs::remove_file(path) {
        Ok(()) => true,
        Err(error) => error.kind() == std::io::ErrorKind::NotFound,
    }
}

/// Port of `Delete /REBOOTOK`: try immediately, otherwise mark the path for
/// removal on reboot.
pub fn delete_reboot_ok(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    clear_readonly(path);
    match fs::remove_file(path) {
        Ok(()) => true,
        Err(_) => schedule_delete_at_reboot(path),
    }
}

/// Port of `RMDir /REBOOTOK /r`.
pub fn remove_dir_all_reboot_ok(path: &Path) -> bool {
    if !path.exists() {
        return true;
    }
    clear_readonly_recursive(path);
    match fs::remove_dir_all(path) {
        Ok(()) => true,
        Err(_) => {
            // PendingFileRenameOperations removes the children first and the
            // directory afterwards, matching RMDir /REBOOTOK /r.
            schedule_tree_at_reboot(path);
            false
        }
    }
}

/// Port of the `AtLeastWin10` check used by `Setup.nsi` for the UCRT check.
pub fn at_least_windows_10() -> bool {
    type RtlGetVersionFn = unsafe extern "system" fn(*mut OSVERSIONINFOW) -> NTSTATUS;

    unsafe {
        let Ok(ntdll) = LoadLibraryW(w!("ntdll.dll")) else {
            return true;
        };
        let proc = GetProcAddress(ntdll, s!("RtlGetVersion"));
        let result = if let Some(proc) = proc {
            let function: RtlGetVersionFn = std::mem::transmute(proc);
            let mut info = OSVERSIONINFOW {
                dwOSVersionInfoSize: size_of::<OSVERSIONINFOW>() as u32,
                ..Default::default()
            };
            if function(&mut info).0 >= 0 {
                info.dwMajorVersion >= 10
            } else {
                true
            }
        } else {
            true
        };
        let _ = FreeLibrary(ntdll);
        result
    }
}

/// `System::Call 'KERNEL32::LoadLibrary(t "ucrtbase.dll")p.r0'`.
pub fn ucrt_available() -> bool {
    unsafe {
        match LoadLibraryW(w!("ucrtbase.dll")) {
            Ok(module) => {
                let _ = FreeLibrary(module);
                true
            }
            Err(_) => false,
        }
    }
}

/// RAII guard balancing `CoInitializeEx`.
pub struct ComGuard {
    uninitialize: bool,
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { windows::Win32::System::Com::CoUninitialize() };
        }
    }
}

pub fn com_initialize_sta() -> ComGuard {
    let hr = unsafe { CoInitializeEx(None, COINIT_APARTMENTTHREADED) };
    if hr == RPC_E_CHANGED_MODE {
        ComGuard {
            uninitialize: false,
        }
    } else {
        ComGuard {
            uninitialize: hr.0 >= 0,
        }
    }
}

/// Port of `AccessControl::GrantOnFile "$INSTDIR\config" "(S-1-5-32-545)" "FullAccess"`.
pub fn grant_users_full_access(path: &Path) -> Result<(), String> {
    const SECURITY_MAX_SID_SIZE: usize = 68;
    let path_wide = wide_path(path);

    let mut sid_buffer = [0u32; SECURITY_MAX_SID_SIZE / 4];
    let sid = PSID(sid_buffer.as_mut_ptr() as *mut c_void);
    let mut sid_size = SECURITY_MAX_SID_SIZE as u32;
    unsafe {
        CreateWellKnownSid(WinBuiltinUsersSid, None, Some(sid), &mut sid_size)
            .map_err(|error| format!("CreateWellKnownSid failed: {error}"))?;
    }

    let mut old_dacl: *mut ACL = null_mut();
    let mut descriptor = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(path_wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(&mut old_dacl),
            None,
            &mut descriptor,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "GetNamedSecurityInfo failed for {}: {}",
            path.display(),
            error_text(status)
        ));
    }

    let trustee = TRUSTEE_W {
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_GROUP,
        ptstrName: PWSTR(sid.0 as *mut u16),
        ..Default::default()
    };
    let access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: FILE_ALL_ACCESS.0,
        grfAccessMode: SET_ACCESS,
        grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        Trustee: trustee,
    };

    let mut new_dacl: *mut ACL = null_mut();
    let old = if old_dacl.is_null() {
        None
    } else {
        Some(old_dacl as *const ACL)
    };
    let status =
        unsafe { SetEntriesInAclW(Some(std::slice::from_ref(&access)), old, &mut new_dacl) };
    if status != ERROR_SUCCESS {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        }
        return Err(format!(
            "SetEntriesInAcl failed for {}: {}",
            path.display(),
            error_text(status)
        ));
    }

    let status = unsafe {
        SetNamedSecurityInfoW(
            PCWSTR(path_wide.as_ptr()),
            SE_FILE_OBJECT,
            DACL_SECURITY_INFORMATION,
            None,
            None,
            Some(new_dacl),
            None,
        )
    };

    unsafe {
        let _ = LocalFree(Some(HLOCAL(descriptor.0)));
        let _ = LocalFree(Some(HLOCAL(new_dacl as *mut c_void)));
    }

    if status != ERROR_SUCCESS {
        return Err(format!(
            "SetNamedSecurityInfo failed for {}: {}",
            path.display(),
            error_text(status)
        ));
    }
    Ok(())
}

pub fn error_text(status: WIN32_ERROR) -> String {
    if status == ERROR_FILE_NOT_FOUND {
        return "The system cannot find the file specified.".to_string();
    }
    std::io::Error::from_raw_os_error(status.0 as i32).to_string()
}

pub fn free_space(path: &Path) -> u64 {
    let mut probe = path.to_path_buf();
    while !probe.exists() {
        match probe.parent() {
            Some(parent) if parent != probe => probe = parent.to_path_buf(),
            _ => break,
        }
    }
    let wide = wide_path(&probe);
    let mut available = 0u64;
    let ok =
        unsafe { GetDiskFreeSpaceExW(PCWSTR(wide.as_ptr()), Some(&mut available), None, None) };
    if ok.is_ok() {
        available
    } else {
        0
    }
}

/// Reboots the machine like `DeviceSelector` does when the APO test requires
/// it (SE_SHUTDOWN_NAME + InitiateShutdown).
pub fn reboot() -> Result<(), String> {
    let mut token = HANDLE::default();
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .map_err(|error| format!("OpenProcessToken failed: {error}"))?;
    }

    let mut luid = LUID::default();
    let privilege =
        unsafe { LookupPrivilegeValueW(PCWSTR::null(), w!("SeShutdownPrivilege"), &mut luid) };
    if let Err(error) = privilege {
        unsafe {
            let _ = CloseHandle(token);
        }
        return Err(format!("LookupPrivilegeValue failed: {error}"));
    }

    let privileges = TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: SE_PRIVILEGE_ENABLED,
        }],
    };
    let adjusted = unsafe { AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None) };
    unsafe {
        let _ = CloseHandle(token);
    }
    adjusted.map_err(|error| format!("AdjustTokenPrivileges failed: {error}"))?;

    let flags = SHUTDOWN_FLAGS(SHUTDOWN_RESTART.0 | SHUTDOWN_GRACE_OVERRIDE.0);
    let reason = windows::Win32::System::Shutdown::SHUTDOWN_REASON(
        SHTDN_REASON_MAJOR_APPLICATION.0 | SHTDN_REASON_MINOR_MAINTENANCE.0,
    );
    let result = unsafe { InitiateShutdownW(PCWSTR::null(), PCWSTR::null(), 0, flags, reason) };
    if result == ERROR_SUCCESS.0 {
        Ok(())
    } else {
        Err(format!(
            "InitiateShutdown failed: {}",
            error_text(WIN32_ERROR(result))
        ))
    }
}

/// Command-line quoting compatible with `CommandLineToArgvW`; used when
/// relaunching Setup.exe elevated.
pub fn build_command_line(args: &[String]) -> String {
    args.iter()
        .map(|arg| quote_argument(arg))
        .collect::<Vec<_>>()
        .join(" ")
}

fn quote_argument(arg: &str) -> String {
    if !arg.is_empty() && !arg.chars().any(|c| c == ' ' || c == '\t' || c == '"') {
        return arg.to_string();
    }
    let mut result = String::from("\"");
    let mut backslashes = 0usize;
    for character in arg.chars() {
        match character {
            '\\' => backslashes += 1,
            '"' => {
                result.push_str(&"\\".repeat(backslashes * 2 + 1));
                result.push('"');
                backslashes = 0;
            }
            _ => {
                result.push_str(&"\\".repeat(backslashes));
                backslashes = 0;
                result.push(character);
            }
        }
    }
    result.push_str(&"\\".repeat(backslashes * 2));
    result.push('"');
    result
}

pub fn file_name_is(path: &Path, name: &str) -> bool {
    path.file_name()
        .and_then(OsStr::to_str)
        .map(|value| value.eq_ignore_ascii_case(name))
        .unwrap_or(false)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn quotes_arguments_like_windows() {
        assert_eq!(quote_argument("simple"), "simple");
        assert_eq!(quote_argument("with space"), "\"with space\"");
        assert_eq!(quote_argument(""), "\"\"");
        assert_eq!(quote_argument(r#"a"b"#), r#""a\"b""#);
        assert_eq!(quote_argument(r"C:\path\"), r"C:\path\");
        assert_eq!(
            quote_argument(r"C:\Program Files\EqualizerAPO\"),
            r#""C:\Program Files\EqualizerAPO\\""#
        );
        assert_eq!(
            build_command_line(&[
                "/uninstall".to_string(),
                "C:\\Program Files\\EqualizerAPO".to_string()
            ]),
            "/uninstall \"C:\\Program Files\\EqualizerAPO\""
        );
    }

    #[test]
    fn program_files_and_start_menu_are_available() {
        assert!(!program_files_dir().as_os_str().is_empty());
        assert!(!start_menu_programs_dir().as_os_str().is_empty());
        assert!(!system_directory().as_os_str().is_empty());
    }
}
