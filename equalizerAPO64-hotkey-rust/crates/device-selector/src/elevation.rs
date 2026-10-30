//! UAC elevation for DeviceSelector.
//!
//! The C++ project sets `UACExecutionLevel=RequireAdministrator` for this
//! executable. `build.rs` asks the MSVC linker to embed the same manifest
//! (`/MANIFESTUAC`).  As a fallback for toolchains or launch mechanisms that
//! ignore the manifest, the process checks its token and relaunches itself
//! with `ShellExecuteW(..., "runas", ...)`.

use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GetLastError, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

fn wide(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn is_elevated() -> bool {
    let mut token = HANDLE::default();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) }.is_err() {
        return false;
    }

    let mut elevation = TOKEN_ELEVATION::default();
    let mut returned_size = 0u32;
    let result = unsafe {
        GetTokenInformation(
            token,
            TokenElevation,
            Some(&mut elevation as *mut TOKEN_ELEVATION as *mut c_void),
            size_of::<TOKEN_ELEVATION>() as u32,
            &mut returned_size,
        )
    };
    unsafe {
        let _ = CloseHandle(token);
    }
    result.is_ok() && elevation.TokenIsElevated != 0
}

/// Relaunches `executable` elevated with the same arguments.  Returns `Ok`
/// after the elevated process has been started; the caller must exit.
pub fn relaunch_elevated(executable: &Path, args: &[String]) -> Result<(), String> {
    let operation = w!("runas");
    let file = wide(&executable.to_string_lossy());
    let parameters = if args.is_empty() {
        None
    } else {
        Some(wide(&build_command_line(args)))
    };
    let directory = executable
        .parent()
        .map(|path| wide(&path.to_string_lossy()));

    let result = unsafe {
        ShellExecuteW(
            None,
            operation,
            PCWSTR(file.as_ptr()),
            parameters
                .as_ref()
                .map(|value| PCWSTR(value.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            directory
                .as_ref()
                .map(|value| PCWSTR(value.as_ptr()))
                .unwrap_or(PCWSTR::null()),
            SW_SHOWNORMAL,
        )
    };

    if result.0 as usize > 32 {
        Ok(())
    } else {
        let code = unsafe { GetLastError().0 };
        Err(format!(
            "ShellExecute(runas) failed: {}",
            std::io::Error::from_raw_os_error(code as i32)
        ))
    }
}

fn build_command_line(args: &[String]) -> String {
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
