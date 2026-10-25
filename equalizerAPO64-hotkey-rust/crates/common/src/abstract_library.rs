//! Port of `helpers/AbstractLibrary.{h,cpp}`.
//!
//! The C++ class is an abstract base class with virtual `getLibPath()` and
//! `loadFunctions()` methods.  Rust has no implementation inheritance, so this
//! port splits the same behaviour into:
//!
//! * [`AbstractLibrary`], which owns the library path and the `HMODULE` and
//!   implements existence check, architecture check, loading/unloading and the
//!   C++ return codes.
//! * [`LibraryError`], the typed equivalent of the negative codes.
//!
//! A user (e.g. the VST plugin library) creates an [`AbstractLibrary`] with the
//! DLL path and passes a closure to [`AbstractLibrary::initialize`] that
//! resolves its exports, mirroring `loadFunctions()`.

use std::ffi::c_void;
use std::fmt;
use std::mem::size_of;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, FreeLibrary, GENERIC_READ};
use windows::Win32::Storage::FileSystem::{
    CreateFileW, GetFileAttributesW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_READ,
    INVALID_FILE_ATTRIBUTES, OPEN_EXISTING,
};
use windows::Win32::System::Diagnostics::Debug::ImageNtHeader;
use windows::Win32::System::LibraryLoader::{GetModuleFileNameW, LoadLibraryW};
use windows::Win32::System::Memory::{
    CreateFileMappingW, MapViewOfFileEx, UnmapViewOfFile, FILE_MAP_READ, PAGE_READONLY,
};
use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE_AMD64;
#[cfg(target_pointer_width = "32")]
use windows::Win32::System::SystemInformation::IMAGE_FILE_MACHINE_I386;

/// `AbstractLibrary::FILE_NOT_FOUND`.
pub const FILE_NOT_FOUND: i32 = -1;
/// `AbstractLibrary::LOADING_FAILED`.
pub const LOADING_FAILED: i32 = -2;
/// `AbstractLibrary::FUNCTIONS_MISSING`.
pub const FUNCTIONS_MISSING: i32 = -3;
/// `AbstractLibrary::WRONG_ARCHITECTURE`.
pub const WRONG_ARCHITECTURE: i32 = -4;

/// Typed equivalent of the C++ return codes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LibraryError {
    /// `FILE_NOT_FOUND`: the DLL path does not exist.
    FileNotFound,
    /// `LOADING_FAILED`: `LoadLibraryW` failed for another reason.
    LoadingFailed,
    /// `FUNCTIONS_MISSING`: `loadFunctions()` returned false.
    FunctionsMissing,
    /// `WRONG_ARCHITECTURE`: the DLL has the wrong machine type.
    WrongArchitecture,
    /// A negative value returned by `customInitialize()`.
    Custom(i32),
}

impl LibraryError {
    /// The C++ integer code for this error.
    pub const fn code(self) -> i32 {
        match self {
            LibraryError::FileNotFound => FILE_NOT_FOUND,
            LibraryError::LoadingFailed => LOADING_FAILED,
            LibraryError::FunctionsMissing => FUNCTIONS_MISSING,
            LibraryError::WrongArchitecture => WRONG_ARCHITECTURE,
            LibraryError::Custom(code) => code,
        }
    }

    /// Converts a C++ style return code to a [`LibraryError`].
    pub const fn from_code(code: i32) -> Option<Self> {
        match code {
            FILE_NOT_FOUND => Some(LibraryError::FileNotFound),
            LOADING_FAILED => Some(LibraryError::LoadingFailed),
            FUNCTIONS_MISSING => Some(LibraryError::FunctionsMissing),
            WRONG_ARCHITECTURE => Some(LibraryError::WrongArchitecture),
            code if code < 0 => Some(LibraryError::Custom(code)),
            _ => None,
        }
    }
}

impl fmt::Display for LibraryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            LibraryError::FileNotFound => f.write_str("Library file not found"),
            LibraryError::LoadingFailed => f.write_str("Library could not be loaded"),
            LibraryError::FunctionsMissing => f.write_str("Library functions missing"),
            LibraryError::WrongArchitecture => f.write_str("Library has the wrong architecture"),
            LibraryError::Custom(code) => {
                write!(f, "Library initialization failed (code {})", code)
            }
        }
    }
}

impl std::error::Error for LibraryError {}

/// Backwards compatible name for the stub's exception type.
pub type LibraryException = LibraryError;

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Returns the `IMAGE_FILE_MACHINE_*` value of a PE file, or 0 if it cannot be
/// determined.  Mirrors `AbstractLibrary::getFileArchitecture` (which uses a
/// file mapping and `ImageNtHeader`).
pub fn get_file_architecture(file_path: &str) -> u16 {
    let mut result = 0u16;
    let path = to_wide(file_path);

    unsafe {
        let Ok(file) = CreateFileW(
            PCWSTR(path.as_ptr()),
            GENERIC_READ.0,
            FILE_SHARE_READ,
            None,
            OPEN_EXISTING,
            FILE_ATTRIBUTE_NORMAL,
            None,
        ) else {
            return 0;
        };

        if let Ok(mapping) = CreateFileMappingW(file, None, PAGE_READONLY, 0, 0, PCWSTR::null()) {
            let view = MapViewOfFileEx(mapping, FILE_MAP_READ, 0, 0, 0, None);
            if !view.Value.is_null() {
                let nt_headers = ImageNtHeader(view.Value);
                if !nt_headers.is_null() {
                    result = (*nt_headers).FileHeader.Machine.0;
                }
                let _ = UnmapViewOfFile(view);
            }
            let _ = CloseHandle(mapping);
        }
        let _ = CloseHandle(file);
    }

    result
}

/// Rust-friendly replacement for `AbstractLibrary`.  Stores the library path
/// and module handle, but leaves export resolution to the caller's closure.
pub struct AbstractLibrary {
    path: String,
    module: Option<windows::Win32::Foundation::HMODULE>,
}

impl AbstractLibrary {
    /// Creates a library object for `path` (not loaded yet).
    pub fn new(path: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            module: None,
        }
    }

    /// The DLL path passed to the constructor / `getLibPath()`.
    pub fn path(&self) -> &str {
        &self.path
    }

    /// Alias for [`AbstractLibrary::path`], named after the C++ method.
    pub fn get_lib_path(&self) -> &str {
        &self.path
    }

    /// The loaded `HMODULE`, if any (`module` in the C++ class).
    pub fn module(&self) -> Option<windows::Win32::Foundation::HMODULE> {
        self.module
    }

    pub fn is_loaded(&self) -> bool {
        self.module.is_some()
    }

    /// Resolves an export from the loaded module (`GetProcAddress`).
    pub fn get_proc_address(&self, name: &str) -> windows::Win32::Foundation::FARPROC {
        let module = self.module?;
        let name = std::ffi::CString::new(name).ok()?;
        unsafe {
            windows::Win32::System::LibraryLoader::GetProcAddress(
                module,
                windows::core::PCSTR(name.as_ptr() as *const u8),
            )
        }
    }

    /// Port of `AbstractLibrary::initialize` without `customInitialize`.
    ///
    /// Returns `Ok(true)` when the library was loaded now, `Ok(false)` when it
    /// was already loaded (C++ returns 1 / 0) and a [`LibraryError`] otherwise.
    pub fn initialize<F>(&mut self, load_functions: F) -> Result<bool, LibraryError>
    where
        F: FnOnce(windows::Win32::Foundation::HMODULE) -> bool,
    {
        self.initialize_with_custom(load_functions, || 0)
    }

    /// Full port of `AbstractLibrary::initialize`, including
    /// `customInitialize()`.  A negative custom result leaves the module
    /// loaded, exactly like the C++ code.
    pub fn initialize_with_custom<F, C>(
        &mut self,
        load_functions: F,
        custom_initialize: C,
    ) -> Result<bool, LibraryError>
    where
        F: FnOnce(windows::Win32::Foundation::HMODULE) -> bool,
        C: FnOnce() -> i32,
    {
        if self.module.is_some() {
            return Ok(false);
        }

        let path = to_wide(&self.path);
        if unsafe { GetFileAttributesW(PCWSTR(path.as_ptr())) } == INVALID_FILE_ATTRIBUTES {
            return Err(LibraryError::FileNotFound);
        }

        let module = match unsafe { LoadLibraryW(PCWSTR(path.as_ptr())) } {
            Ok(module) => module,
            Err(_) => {
                let arch = get_file_architecture(&self.path);
                #[cfg(target_pointer_width = "64")]
                let expected_arch = IMAGE_FILE_MACHINE_AMD64.0;
                #[cfg(target_pointer_width = "32")]
                let expected_arch = IMAGE_FILE_MACHINE_I386.0;

                if arch != 0 && arch != expected_arch {
                    return Err(LibraryError::WrongArchitecture);
                }
                return Err(LibraryError::LoadingFailed);
            }
        };
        self.module = Some(module);

        if !load_functions(module) {
            unsafe {
                let _ = FreeLibrary(module);
            }
            self.module = None;
            return Err(LibraryError::FunctionsMissing);
        }

        let result = custom_initialize();
        if result < 0 {
            return Err(LibraryError::from_code(result).unwrap_or(LibraryError::Custom(result)));
        }

        crate::trace_log!("Loaded library {}", self.path);
        Ok(true)
    }

    /// Convenience wrapper returning the C++ integer code (1 = loaded,
    /// 0 = already loaded, negative = error).
    pub fn initialize_code<F>(&mut self, load_functions: F) -> i32
    where
        F: FnOnce(windows::Win32::Foundation::HMODULE) -> bool,
    {
        match self.initialize(load_functions) {
            Ok(true) => 1,
            Ok(false) => 0,
            Err(error) => error.code(),
        }
    }

    /// Explicitly frees the library.  Dropping the object does the same.
    pub fn unload(&mut self) {
        if let Some(module) = self.module.take() {
            let mut path = [0u16; 260];
            let len = unsafe { GetModuleFileNameW(Some(module), &mut path) };
            let name = String::from_utf16_lossy(&path[..len as usize]);
            unsafe {
                let _ = FreeLibrary(module);
            }
            crate::trace_log!("Unloaded library {}", name);
        }
    }
}

impl Default for AbstractLibrary {
    fn default() -> Self {
        Self::new(String::new())
    }
}

impl Drop for AbstractLibrary {
    fn drop(&mut self) {
        self.unload();
    }
}

// `size_of` is used by the conceptual equivalent of the C++ PE checks; keep a
// compile-time assertion that our HMODULE wrapper has pointer size.
const _: () = assert!(size_of::<windows::Win32::Foundation::HMODULE>() == size_of::<*mut c_void>());

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn error_codes_match_cpp() {
        assert_eq!(LibraryError::FileNotFound.code(), -1);
        assert_eq!(LibraryError::LoadingFailed.code(), -2);
        assert_eq!(LibraryError::FunctionsMissing.code(), -3);
        assert_eq!(LibraryError::WrongArchitecture.code(), -4);
        assert_eq!(
            LibraryError::from_code(-1),
            Some(LibraryError::FileNotFound)
        );
        assert_eq!(
            LibraryError::from_code(-2),
            Some(LibraryError::LoadingFailed)
        );
        assert_eq!(
            LibraryError::from_code(-3),
            Some(LibraryError::FunctionsMissing)
        );
        assert_eq!(
            LibraryError::from_code(-4),
            Some(LibraryError::WrongArchitecture)
        );
    }

    #[test]
    fn current_executable_architecture_is_detected() {
        let exe = std::env::current_exe().expect("current exe");
        let arch = get_file_architecture(&exe.to_string_lossy());
        #[cfg(target_pointer_width = "64")]
        assert_eq!(arch, IMAGE_FILE_MACHINE_AMD64.0);
        #[cfg(target_pointer_width = "32")]
        assert_eq!(arch, IMAGE_FILE_MACHINE_I386.0);
    }

    #[test]
    fn missing_library_is_reported() {
        let mut library = AbstractLibrary::new(r"C:\Does\Not\Exist\eapo-test.dll");
        assert_eq!(
            library.initialize(|_| true).unwrap_err(),
            LibraryError::FileNotFound
        );
        assert_eq!(library.initialize_code(|_| true), FILE_NOT_FOUND);
    }

    #[test]
    fn functions_missing_is_reported_and_module_unloaded() {
        let system_root = std::env::var("SystemRoot").expect("SystemRoot");
        let mut library = AbstractLibrary::new(format!(r"{}\System32\kernel32.dll", system_root));
        assert_eq!(
            library.initialize(|_| false).unwrap_err(),
            LibraryError::FunctionsMissing
        );
        assert!(!library.is_loaded());
    }
}
