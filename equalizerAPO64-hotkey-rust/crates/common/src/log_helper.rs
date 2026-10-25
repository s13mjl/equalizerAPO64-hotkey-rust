//! Port of `helpers/LogHelper.{h,cpp}`.
//!
//! Writes `%TEMP%\EqualizerAPO.log` in append mode.  Tracing is enabled unless
//! `HKLM\SOFTWARE\EqualizerAPO\EnableTrace` exists and equals `false`.
//!
//! The original opens the file with `_wfopen_s(..., L"at")`, so every call
//! appends one line.  The line prefix is
//! `YYYY-MM-DD HH:MM:SS.mmm threadid POINTER (file:line): `; in Rust the file
//! and line come from `file!()`/`line!()` at the macro call site, which is the
//! closest textual equivalent of `__FILE__`/`__LINE__`.

use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

use once_cell::sync::Lazy;
use windows::Win32::System::Console::{
    GetStdHandle, SetConsoleTextAttribute, CONSOLE_CHARACTER_ATTRIBUTES, STD_OUTPUT_HANDLE,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::System::Threading::GetCurrentThreadId;

use crate::registry_helper;

struct LogState {
    initialized: bool,
    log_path: PathBuf,
    /// Rust equivalent of the C++ `presetFP`: when set, log lines go there
    /// instead of the default file.
    preset_path: Option<PathBuf>,
    /// C++ callers passed `stderr` as preset `FILE*`; this flag reproduces it.
    preset_stderr: bool,
    enable_trace: bool,
    compact: bool,
    use_console_colors: bool,
}

impl Default for LogState {
    fn default() -> Self {
        Self {
            initialized: false,
            log_path: PathBuf::new(),
            preset_path: None,
            preset_stderr: false,
            enable_trace: false,
            compact: false,
            use_console_colors: false,
        }
    }
}

static STATE: Lazy<Mutex<LogState>> = Lazy::new(|| Mutex::new(LogState::default()));

fn temp_log_path() -> PathBuf {
    let mut buf = [0u16; 260];
    let len = unsafe { windows::Win32::Storage::FileSystem::GetTempPathW(Some(&mut buf)) };
    if len > 0 {
        return PathBuf::from(String::from_utf16_lossy(&buf[..len as usize]))
            .join("EqualizerAPO.log");
    }
    std::env::temp_dir().join("EqualizerAPO.log")
}

fn lock_state() -> std::sync::MutexGuard<'static, LogState> {
    STATE
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

/// Lazily initializes path and trace flag once, like the C++ `initialized`
/// static.  A failed registry read is logged and disables tracing.
fn ensure_initialized() {
    let pending_error = {
        let mut state = lock_state();
        if state.initialized {
            return;
        }
        state.initialized = true;
        state.log_path = temp_log_path();

        match registry_helper::read_value(crate::APP_REGPATH, "EnableTrace") {
            Ok(value) => {
                state.enable_trace = value != "false";
                None
            }
            Err(error) => {
                state.enable_trace = false;
                Some(error.to_string())
            }
        }
    };

    if let Some(message) = pending_error {
        // Do not call log() here: initialization is already marked complete,
        // but this keeps the recursive call explicit and deadlock-free.
        write_line("log_helper.rs", 0, std::ptr::null(), false, &message);
    }
}

fn set_console_color(color: u16) {
    unsafe {
        if let Ok(handle) = GetStdHandle(STD_OUTPUT_HANDLE) {
            let _ = SetConsoleTextAttribute(handle, CONSOLE_CHARACTER_ATTRIBUTES(color));
        }
    }
}

/// Writes one formatted line.  Assumes initialization has already happened.
fn write_line(file: &str, line: u32, caller: *const c_void, trace: bool, message: &str) {
    let (target, stderr, enable_trace, compact, use_colors) = {
        let state = lock_state();
        (
            state
                .preset_path
                .clone()
                .unwrap_or_else(|| state.log_path.clone()),
            state.preset_stderr,
            state.enable_trace,
            state.compact,
            state.use_console_colors,
        )
    };

    if trace && !enable_trace {
        return;
    }

    let mut text = String::new();
    if !compact {
        let system_time = unsafe { GetLocalTime() };
        let thread_id = unsafe { GetCurrentThreadId() };
        text.push_str(&format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03} {} {:08X} ({}:{}): ",
            system_time.wYear,
            system_time.wMonth,
            system_time.wDay,
            system_time.wHour,
            system_time.wMinute,
            system_time.wSecond,
            system_time.wMilliseconds,
            thread_id,
            caller as usize as u32,
            file,
            line
        ));
    }
    if trace {
        text.push_str("(TRACE) ");
    }
    text.push_str(message);
    // The C++ stream is opened in text mode ("at"), so "\n" becomes CRLF.
    text.push_str("\r\n");

    if use_colors {
        set_console_color(if trace { 2 } else { 12 });
    }

    if stderr {
        let _ = std::io::stderr().write_all(text.as_bytes());
    } else if let Ok(mut file) = OpenOptions::new().create(true).append(true).open(target) {
        let _ = file.write_all(text.as_bytes());
    }

    if use_colors {
        set_console_color(7);
    }
}

/// Port of `LogHelper::log`.  Prefer the [`crate::trace_log!`] /
/// [`crate::error_log!`] macros, which fill in `file!()` and `line!()`.
pub fn log(file: &str, line: u32, caller: *const c_void, trace: bool, message: &str) {
    ensure_initialized();
    write_line(file, line, caller, trace, message);
}

/// Compatibility helper for `LogFStatic(L"%s", message)`.
pub fn log_error_static(message: &str) {
    log("log_helper.rs", 0, std::ptr::null(), false, message);
}

/// Compatibility helper for `TraceFStatic(L"%s", message)`.
pub fn log_trace_static(message: &str) {
    log("log_helper.rs", 0, std::ptr::null(), true, message);
}

/// Compatibility helper matching the old stub's `log_impl(trace, msg)`.
pub fn log_impl(trace: bool, message: String) {
    log("log_helper.rs", 0, std::ptr::null(), trace, &message);
}

/// Port of `LogHelper::reset`.  Like the C++ code this does not clear the
/// preset target, `compact` or `use_console_colors`.
pub fn reset() {
    let mut state = lock_state();
    state.initialized = false;
    state.log_path.clear();
    state.enable_trace = false;
}

/// Port of `LogHelper::set(FILE*, enableTrace, compact, useConsoleColors)`.
///
/// The C++ `FILE*` preset is replaced by an optional path; this overload keeps
/// the default `%TEMP%\EqualizerAPO.log` target.
pub fn set(enable_trace: bool, compact: bool, use_console_colors: bool) {
    let mut state = lock_state();
    state.initialized = true;
    if state.log_path.as_os_str().is_empty() {
        state.log_path = temp_log_path();
    }
    state.preset_path = None;
    state.preset_stderr = false;
    state.enable_trace = enable_trace;
    state.compact = compact;
    state.use_console_colors = use_console_colors;
}

/// Rust-friendly variant of `LogHelper::set` that redirects output to `path`
/// (the equivalent of the C++ preset `FILE*`).
pub fn set_to_file(
    path: impl AsRef<Path>,
    enable_trace: bool,
    compact: bool,
    use_console_colors: bool,
) {
    let mut state = lock_state();
    state.initialized = true;
    if state.log_path.as_os_str().is_empty() {
        state.log_path = temp_log_path();
    }
    state.preset_path = Some(path.as_ref().to_path_buf());
    state.preset_stderr = false;
    state.enable_trace = enable_trace;
    state.compact = compact;
    state.use_console_colors = use_console_colors;
}

/// Rust equivalent of `LogHelper::set(stderr, ...)`.
pub fn set_to_stderr(enable_trace: bool, compact: bool, use_console_colors: bool) {
    let mut state = lock_state();
    state.initialized = true;
    if state.log_path.as_os_str().is_empty() {
        state.log_path = temp_log_path();
    }
    state.preset_path = None;
    state.preset_stderr = true;
    state.enable_trace = enable_trace;
    state.compact = compact;
    state.use_console_colors = use_console_colors;
}

/// Trace-level log with `file!()` / `line!()` from the call site.
#[macro_export]
macro_rules! trace_log {
    ($($arg:tt)*) => {
        $crate::log_helper::log(
            file!(),
            line!(),
            ::core::ptr::null(),
            true,
            &format!($($arg)*),
        )
    };
}

/// Error-level log with `file!()` / `line!()` from the call site.
#[macro_export]
macro_rules! error_log {
    ($($arg:tt)*) => {
        $crate::log_helper::log(
            file!(),
            line!(),
            ::core::ptr::null(),
            false,
            &format!($($arg)*),
        )
    };
}
