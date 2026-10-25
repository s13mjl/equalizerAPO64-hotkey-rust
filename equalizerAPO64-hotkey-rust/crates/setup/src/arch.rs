//! Native architecture detection.
//!
//! `Setup64.nsi` includes the shared script with `TARGET_ARCH "x64"` and
//! aborts on any other *native* architecture.  `IsWow64Process2` distinguishes
//! a native x64 Windows from x64 emulation on ARM64 (where `Setup64` must not
//! run, because the ARM64 installer is the correct one).
//!
//! `IsWow64Process2` only exists on Windows 10 1709+, so it is resolved
//! dynamically; on older systems `GetNativeSystemInfo` provides the native
//! processor architecture.

use windows::core::{s, w, BOOL};
use windows::Win32::Foundation::{FreeLibrary, HANDLE};
use windows::Win32::System::LibraryLoader::{GetProcAddress, LoadLibraryW};
use windows::Win32::System::SystemInformation::{
    GetNativeSystemInfo, IMAGE_FILE_MACHINE, IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64,
    IMAGE_FILE_MACHINE_I386, IMAGE_FILE_MACHINE_UNKNOWN, PROCESSOR_ARCHITECTURE_AMD64,
    PROCESSOR_ARCHITECTURE_ARM64, PROCESSOR_ARCHITECTURE_INTEL, SYSTEM_INFO,
};
use windows::Win32::System::Threading::GetCurrentProcess;

type IsWow64Process2Fn =
    unsafe extern "system" fn(HANDLE, *mut IMAGE_FILE_MACHINE, *mut IMAGE_FILE_MACHINE) -> BOOL;

/// Maps `(processMachine, nativeMachine)` to the NSIS architecture name.
pub fn arch_from_machines(process: IMAGE_FILE_MACHINE, native: IMAGE_FILE_MACHINE) -> &'static str {
    // A 64-bit process running natively has processMachine UNKNOWN; under
    // emulation processMachine is the emulated architecture.
    if native == IMAGE_FILE_MACHINE_AMD64 && process == IMAGE_FILE_MACHINE_UNKNOWN {
        "x64"
    } else if native == IMAGE_FILE_MACHINE_ARM64 {
        "ARM64"
    } else if native == IMAGE_FILE_MACHINE_I386 {
        "x86"
    } else if native == IMAGE_FILE_MACHINE_AMD64 {
        "x64"
    } else {
        // Includes IMAGE_FILE_MACHINE_UNKNOWN; callers fall back to
        // GetNativeSystemInfo for this case.
        "unknown"
    }
}

fn arch_from_processor(architecture: u16) -> &'static str {
    if architecture == PROCESSOR_ARCHITECTURE_AMD64.0 {
        "x64"
    } else if architecture == PROCESSOR_ARCHITECTURE_ARM64.0 {
        "ARM64"
    } else if architecture == PROCESSOR_ARCHITECTURE_INTEL.0 {
        "x86"
    } else {
        "unknown"
    }
}

fn native_info_arch() -> &'static str {
    let mut info = SYSTEM_INFO::default();
    unsafe { GetNativeSystemInfo(&mut info) };
    let architecture = unsafe { info.Anonymous.Anonymous.wProcessorArchitecture.0 };
    arch_from_processor(architecture)
}

pub fn native_arch() -> &'static str {
    unsafe {
        let Ok(kernel32) = LoadLibraryW(w!("kernel32.dll")) else {
            return native_info_arch();
        };
        let procedure = GetProcAddress(kernel32, s!("IsWow64Process2"));
        let result = procedure.map(|procedure| {
            let function: IsWow64Process2Fn = std::mem::transmute(procedure);
            let mut process_machine = IMAGE_FILE_MACHINE_UNKNOWN;
            let mut native_machine = IMAGE_FILE_MACHINE_UNKNOWN;
            let ok = function(
                GetCurrentProcess(),
                &mut process_machine,
                &mut native_machine,
            )
            .as_bool();
            if ok {
                let architecture = arch_from_machines(process_machine, native_machine);
                if architecture != "unknown" {
                    return architecture;
                }
            }
            native_info_arch()
        });
        let _ = FreeLibrary(kernel32);
        result.unwrap_or_else(native_info_arch)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn native_x64_is_x64() {
        assert_eq!(
            arch_from_machines(IMAGE_FILE_MACHINE_UNKNOWN, IMAGE_FILE_MACHINE_AMD64),
            "x64"
        );
    }

    #[test]
    fn x64_emulation_on_arm64_is_arm64() {
        assert_eq!(
            arch_from_machines(IMAGE_FILE_MACHINE_AMD64, IMAGE_FILE_MACHINE_ARM64),
            "ARM64"
        );
    }

    #[test]
    fn native_x86_is_x86() {
        assert_eq!(
            arch_from_machines(IMAGE_FILE_MACHINE_UNKNOWN, IMAGE_FILE_MACHINE_I386),
            "x86"
        );
    }

    #[test]
    fn current_machine_is_known() {
        assert_ne!(native_arch(), "unknown");
    }
}
