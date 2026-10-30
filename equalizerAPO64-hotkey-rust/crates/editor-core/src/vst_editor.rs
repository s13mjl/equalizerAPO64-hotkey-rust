//! Native VST 2.4 plugin editor window.
//!
//! Port of the non-embedded mode of `VSTPluginFilterGUI`: the plugin's own
//! editor window is parented to a modal Win32 frame with Apply/OK/Cancel
//! commands. `Auto apply` polls the effect state every second, like the Qt
//! event dispatcher's `aboutToBlock` hook in the original.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::Path;

use eapo_filters::vst_plugin_instance::VstPluginInstance;
use eapo_filters::vst_plugin_library::VSTPluginLibrary;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Foundation::{HWND, LPARAM, LRESULT, WPARAM};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CreateMenu, CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW,
    SetForegroundWindow, SetMenu, SetWindowPos, ShowWindow,
    TranslateMessage, CREATESTRUCTW, CW_USEDEFAULT, MF_STRING, MSG, SW_SHOW, WM_CLOSE,
    WM_COMMAND, WM_DESTROY, WM_NCCREATE, WNDCLASSEXW, WS_OVERLAPPEDWINDOW, WS_VISIBLE,
};

use crate::model::VstEditor;

const ID_APPLY: usize = 1001;
const ID_OK: usize = 1002;
const ID_CANCEL: usize = 1003;

/// Checks whether the file is readable by the audio service (LOCAL SERVICE).
pub fn audio_service_readable(path: &Path) -> Option<bool> {
    let mask =
        eapo_common::registry_helper::get_file_access_for_user(&path.to_string_lossy(), 19).ok()?;
    let generic_read = windows::Win32::Foundation::GENERIC_READ.0;
    let file_generic_read = windows::Win32::Storage::FileSystem::FILE_GENERIC_READ.0;
    Some(
        mask & generic_read == generic_read || mask & file_generic_read == file_generic_read,
    )
}

struct HostState {
    apply: bool,
    close: bool,
}

thread_local! {
    static HOST_STATE: RefCell<HostState> = RefCell::new(HostState { apply: false, close: false });
}

unsafe extern "system" fn host_wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            let _ = unsafe { &*(lparam.0 as *const CREATESTRUCTW) };
            LRESULT(1)
        }
        WM_COMMAND => {
            match (wparam.0 & 0xffff) as usize {
                ID_APPLY => HOST_STATE.with(|state| state.borrow_mut().apply = true),
                ID_OK => HOST_STATE.with(|state| {
                    let mut state = state.borrow_mut();
                    state.apply = true;
                    state.close = true;
                }),
                ID_CANCEL => HOST_STATE.with(|state| state.borrow_mut().close = true),
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            HOST_STATE.with(|state| state.borrow_mut().close = true);
            LRESULT(0)
        }
        WM_DESTROY => {
            // Do not post WM_QUIT here: this window lives inside the host
            // application's message loop and closing the panel must not end
            // the whole process.
            HOST_STATE.with(|state| state.borrow_mut().close = true);
            LRESULT(0)
        }
        _ => unsafe { DefWindowProcW(hwnd, message, wparam, lparam) },
    }
}

fn create_host_window(title: &str) -> Option<HWND> {
    unsafe {
        let module = GetModuleHandleW(None).ok()?;
        let class_name = w!("EapoVstEditorHost");
        let wc = WNDCLASSEXW {
            cbSize: std::mem::size_of::<WNDCLASSEXW>() as u32,
            lpfnWndProc: Some(host_wnd_proc),
            hInstance: module.into(),
            lpszClassName: class_name,
            ..Default::default()
        };
        let _ = windows::Win32::UI::WindowsAndMessaging::RegisterClassExW(&wc);
        let title = HSTRING::from(title);
        let hwnd = CreateWindowExW(
            Default::default(),
            class_name,
            &title,
            WS_OVERLAPPEDWINDOW | WS_VISIBLE,
            CW_USEDEFAULT,
            CW_USEDEFAULT,
            520,
            420,
            None,
            None,
            Some(module.into()),
            None,
        )
        .ok()?;

        if let Ok(menu) = CreateMenu() {
            let file = CreateMenu().ok();
            if let Some(file) = file {
                let _ = AppendMenuW(menu, MF_STRING, ID_APPLY, w!("Apply"));
                let _ = AppendMenuW(menu, MF_STRING, ID_OK, w!("OK"));
                let _ = AppendMenuW(menu, MF_STRING, ID_CANCEL, w!("Cancel"));
                let _ = windows::Win32::UI::WindowsAndMessaging::AppendMenuW(
                    menu,
                    windows::Win32::UI::WindowsAndMessaging::MF_POPUP,
                    file.0 as usize,
                    w!("File"),
                );
                let _ = SetMenu(hwnd, Some(menu));
            }
        }
        Some(hwnd)
    }
}

fn resolve_library_path(path: &str) -> String {
    let path_ref = Path::new(path);
    if path_ref.is_absolute() {
        path.to_string()
    } else {
        let plugin_dir = VSTPluginLibrary::get_default_plugin_path();
        Path::new(&plugin_dir)
            .join(path_ref)
            .to_string_lossy()
            .into_owned()
    }
}

fn params_to_map(params: &[(String, f64)]) -> HashMap<String, f32> {
    params
        .iter()
        .map(|(name, value)| (name.clone(), *value as f32))
        .collect()
}

fn apply_effect_state(effect: &VstPluginInstance, v: &mut VstEditor) {
    let mut chunk = String::new();
    let mut params = HashMap::new();
    effect.read_from_effect(&mut chunk, &mut params);
    v.chunk_data = chunk;
    let mut params: Vec<(String, f64)> = params
        .into_iter()
        .map(|(name, value)| (name, value as f64))
        .collect();
    params.sort_by(|a, b| a.0.cmp(&b.0));
    v.params = params;
}

/// Opens the plugin's native editor. Returns true if the stored chunk or
/// parameters changed.
pub fn open_plugin_editor(v: &mut VstEditor) -> bool {
    if v.path.is_empty() {
        v.status = "No file selected.".to_string();
        return false;
    }

    let library = VSTPluginLibrary::get_instance(&resolve_library_path(&v.path));
    let result = library.initialize();
    if result < 0 {
        use eapo_common::abstract_library::{
            FILE_NOT_FOUND, FUNCTIONS_MISSING, LOADING_FAILED, WRONG_ARCHITECTURE,
        };
        v.status = match result {
            FILE_NOT_FOUND => "File not found.".to_string(),
            LOADING_FAILED => "Library could not be loaded.".to_string(),
            FUNCTIONS_MISSING => "Library does not contain needed functions.".to_string(),
            WRONG_ARCHITECTURE => {
                "Library has the wrong architecture. Only 64-bit libraries are supported."
                    .to_string()
            }
            _ => "Library could not be loaded.".to_string(),
        };
        return false;
    }

    let mut effect = VstPluginInstance::new(library, 1);
    if !effect.initialize() {
        v.status = "Plugin crashed during initialization.".to_string();
        return false;
    }
    let name = effect.get_name();
    v.status = name.clone();
    effect.set_language(1);
    effect.write_to_effect(&v.chunk_data, &params_to_map(&v.params));

    let Some(hwnd) = create_host_window(&name) else {
        v.status = "Could not open plugin editor window.".to_string();
        return false;
    };

    HOST_STATE.with(|state| {
        *state.borrow_mut() = HostState {
            apply: false,
            close: false,
        }
    });

    let mut width: i16 = 0;
    let mut height: i16 = 0;
    effect.start_editing(hwnd, &mut width, &mut height);
    if width > 0 && height > 0 {
        unsafe {
            let _ = SetWindowPos(
                hwnd,
                None,
                0,
                0,
                width as i32 + 16,
                height as i32 + 68,
                windows::Win32::UI::WindowsAndMessaging::SWP_NOMOVE,
            );
            let _ = ShowWindow(hwnd, SW_SHOW);
            let _ = SetForegroundWindow(hwnd);
        }
    }

    let mut changed = false;
    let mut last_read = std::time::Instant::now();
    unsafe {
        let mut msg = MSG::default();
        loop {
            while windows::Win32::UI::WindowsAndMessaging::PeekMessageW(
                &mut msg,
                None,
                0,
                0,
                windows::Win32::UI::WindowsAndMessaging::PM_REMOVE,
            )
            .as_bool()
            {
                if msg.message == windows::Win32::UI::WindowsAndMessaging::WM_QUIT {
                    // The application is shutting down; re-post so the outer
                    // loop sees it after this panel closes.
                    windows::Win32::UI::WindowsAndMessaging::PostQuitMessage(0);
                    HOST_STATE.with(|state| state.borrow_mut().close = true);
                    break;
                }
                let _ = TranslateMessage(&msg);
                DispatchMessageW(&msg);
            }
            let (close, apply) = HOST_STATE.with(|state| {
                let state = state.borrow();
                (state.close, state.apply)
            });
            if apply || v.auto_apply {
                if apply || last_read.elapsed() >= std::time::Duration::from_millis(1000) {
                    apply_effect_state(&effect, v);
                    changed = true;
                    last_read = std::time::Instant::now();
                }
            }
            if close {
                if apply || v.auto_apply {
                    apply_effect_state(&effect, v);
                    changed = true;
                }
                break;
            }
            effect.do_idle();
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        effect.stop_editing();
        let _ = DestroyWindow(hwnd);
    }

    changed
}

/// Loads the plugin to show its name/status without opening the editor.
pub fn refresh_plugin_status(v: &mut VstEditor) {
    if v.path.is_empty() {
        v.status = "No file selected.".to_string();
        return;
    }
    let library = VSTPluginLibrary::get_instance(&resolve_library_path(&v.path));
    if library.initialize() < 0 {
        v.status = "Library could not be loaded.".to_string();
        return;
    }
    let mut effect = VstPluginInstance::new(library, 1);
    if effect.initialize() {
        v.status = effect.get_name();
    } else {
        v.status = "Plugin crashed during initialization.".to_string();
    }
}

#[allow(dead_code)]
fn _pcwstr_null() -> PCWSTR {
    PCWSTR::null()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Opens the test plugin's editor window and closes it from a helper
    /// thread. Requires an interactive desktop and the built test plugin.
    #[test]
    #[ignore]
    fn opens_native_plugin_editor_and_closes() {
        use windows::core::w;
        use windows::Win32::UI::WindowsAndMessaging::{
            FindWindowW, PostMessageW, WM_CLOSE,
        };

        let plugin = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("testdata")
            .join("vst-test-plugin")
            .join("target")
            .join("release")
            .join("TestVST.dll");
        assert!(plugin.exists(), "build the test plugin first");
        let mut v = VstEditor {
            path: plugin.to_string_lossy().into_owned(),
            chunk_data: "AAAAAAAA4D8=".into(),
            ..Default::default()
        };

        std::thread::spawn(|| {
            for _ in 0..100 {
                std::thread::sleep(std::time::Duration::from_millis(50));
                unsafe {
                    if let Ok(hwnd) = FindWindowW(w!("EapoVstEditorHost"), PCWSTR::null()) {
                        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                        return;
                    }
                }
            }
        });

        let changed = open_plugin_editor(&mut v);
        assert!(!changed);
        assert_eq!(v.status, "Rust Test VST");
    }
}
