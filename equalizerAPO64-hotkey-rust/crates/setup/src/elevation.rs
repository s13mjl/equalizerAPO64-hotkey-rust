//! UAC elevation.
//!
//! The NSIS installer declares `RequestExecutionLevel admin`, which puts a
//! `requestedExecutionLevel level="requireAdministrator"` manifest into the
//! executable. `build.rs` asks the MSVC linker to embed the same manifest
//! (`/MANIFESTUAC`).  In addition (and as fallback for toolchains or launch
//! mechanisms that ignore the manifest), this module checks the token and
//! relaunches the current executable with `ShellExecuteW(..., "runas", ...)`,
//! exactly like an NSIS admin installer behaves if it is started without
//! elevation.

use std::ffi::c_void;
use std::mem::size_of;
use std::path::Path;

use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, HANDLE};
use windows::Win32::Security::{GetTokenInformation, TokenElevation, TOKEN_ELEVATION, TOKEN_QUERY};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

use crate::util::{build_command_line, last_error_string, wide};

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
        Err(format!(
            "ShellExecute(runas) failed: {}",
            last_error_string()
        ))
    }
}
