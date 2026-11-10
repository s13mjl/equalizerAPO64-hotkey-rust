//! Modal text and hotkey prompts for the native editor.

use super::*;

struct PromptState {
    edit: HWND,
    initial: String,
    hint: String,
    result: Rc<RefCell<Option<String>>>,
    dpi: u32,
    font: HFONT,
}

static PROMPT_CLASS: OnceLock<()> = OnceLock::new();

unsafe extern "system" fn prompt_edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let previous = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    if msg == WM_KEYDOWN {
        if wparam.0 == VK_RETURN.0 as usize {
            if let Ok(parent) = GetParent(hwnd) {
                let _ = PostMessageW(Some(parent), WM_APP_PROMPT_ACCEPT, WPARAM(0), LPARAM(0));
            }
            return LRESULT(0);
        }
        if wparam.0 == VK_ESCAPE.0 as usize {
            if let Ok(parent) = GetParent(hwnd) {
                let _ = PostMessageW(Some(parent), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            return LRESULT(0);
        }
    }
    if previous != 0 {
        let procedure: WNDPROC = std::mem::transmute(previous);
        CallWindowProcW(procedure, hwnd, msg, wparam, lparam)
    } else {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

unsafe extern "system" fn prompt_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            let state = Box::from_raw(create.lpCreateParams as *mut PromptState);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
            let Some(state) = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PromptState).as_mut()
            else {
                return LRESULT(-1);
            };
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let dpi = state.dpi;
            let width = (client.right - scale(24, dpi)).max(scale(120, dpi));
            let initial = wide_z(&state.initial);
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                PCWSTR(initial.as_ptr()),
                WINDOW_STYLE(
                    WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32,
                ),
                scale(12, dpi),
                scale(36, dpi),
                width,
                scale(24, dpi),
                Some(hwnd),
                None,
                None,
                None,
            );
            if let Ok(edit) = edit {
                SendMessageW(
                    edit,
                    WM_SETFONT,
                    Some(WPARAM(state.font.0 as usize)),
                    Some(LPARAM(1)),
                );
                let previous =
                    SetWindowLongPtrW(edit, GWLP_WNDPROC, prompt_edit_proc as *const () as isize);
                SetWindowLongPtrW(edit, GWLP_USERDATA, previous);
                state.edit = edit;
                let _ = SetFocus(Some(edit));
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut paint);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            fill(hdc, 0, 0, client.right, client.bottom, WHITE);
            let state = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PromptState).as_mut();
            let dpi = state.as_ref().map(|state| state.dpi).unwrap_or(96);
            if let Some(state) = state.as_ref() {
                SelectObject(hdc, HGDIOBJ(state.font.0));
            }
            draw_text(
                hdc,
                scale(12, dpi),
                scale(12, dpi),
                &wide(state.as_ref().map(|state| state.hint.as_str()).unwrap_or("")),
                TEXT,
            );
            draw_text(
                hdc,
                scale(12, dpi),
                client.bottom - scale(22, dpi),
                &trw("Enter = OK, Esc = Cancel"),
                MUTED,
            );
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_APP_PROMPT_ACCEPT => {
            if let Some(state) = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PromptState).as_mut()
            {
                let length = GetWindowTextLengthW(state.edit).max(0) as usize;
                let mut buffer = vec![0u16; length + 1];
                let read = GetWindowTextW(state.edit, &mut buffer).max(0) as usize;
                *state.result.borrow_mut() = Some(
                    String::from_utf16_lossy(&buffer[..read]).trim().to_string(),
                );
            }
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_CLOSE => {
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PromptState;
            if let Some(state) = pointer.as_mut() {
                if !state.font.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(state.font.0));
                }
            }
            if !pointer.is_null() {
                drop(Box::from_raw(pointer));
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

pub(super) unsafe fn prompt_text(
    parent: HWND,
    title: &str,
    hint: &str,
    initial: &str,
) -> Option<String> {
    PROMPT_CLASS.get_or_init(|| {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let Ok(cursor) = LoadCursorW(None, IDC_ARROW) else {
            return;
        };
        let name = wide_z("EapoPromptDialog");
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(prompt_wndproc),
            hInstance: HINSTANCE(module.0),
            hCursor: cursor,
            lpszClassName: PCWSTR(name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
    });
    let name = wide_z("EapoPromptDialog");
    let title = wide_z(title);
    let dpi = GetDpiForWindow(parent).max(96);
    let font = CreateFontW(
        -((9 * dpi as i32 + 36) / 72),
        0,
        0,
        0,
        400,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS,
        CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY,
        DEFAULT_PITCH.0 as u32,
        w!("Segoe UI"),
    );
    let mut parent_rect = RECT::default();
    let _ = GetWindowRect(parent, &mut parent_rect);
    let width = scale(520, dpi);
    let height = scale(130, dpi);
    let x = parent_rect.left + ((parent_rect.right - parent_rect.left) - width) / 2;
    let y = parent_rect.top + ((parent_rect.bottom - parent_rect.top) - height) / 2;
    let result = Rc::new(RefCell::new(None));
    let state = Box::new(PromptState {
        edit: HWND::default(),
        initial: initial.to_string(),
        hint: hint.to_string(),
        result: result.clone(),
        dpi,
        font,
    });
    let dialog = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR(name.as_ptr()),
        PCWSTR(title.as_ptr()),
        WINDOW_STYLE(WS_POPUP.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_VISIBLE.0),
        x,
        y,
        width,
        height,
        None,
        None,
        None,
        Some(Box::into_raw(state) as *const core::ffi::c_void),
    );
    let Ok(dialog) = dialog else {
        return None;
    };
    let _ = ShowWindow(dialog, SW_SHOW);
    let _ = SetForegroundWindow(dialog);
    let _ = EnableWindow(parent, false);
    let mut message = MSG::default();
    while IsWindow(Some(dialog)).as_bool() {
        if !GetMessageW(&mut message, None, 0, 0).as_bool() {
            break;
        }
        let _ = TranslateMessage(&message);
        DispatchMessageW(&message);
    }
    let _ = EnableWindow(parent, true);
    let value = result.borrow_mut().take();
    value
}

struct HotkeyPromptState {
    edit: HWND,
    sequence: Option<KeySequence>,
    result: Rc<RefCell<Option<Option<KeySequence>>>>,
    dpi: u32,
    font: HFONT,
}

static HOTKEY_PROMPT_CLASS: OnceLock<()> = OnceLock::new();

/// Handles a key press captured by the modal loop.
unsafe fn handle_hotkey_key(dialog: HWND, edit: HWND, vk: u32) {
    let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
    let alt = GetKeyState(VK_MENU.0 as i32) < 0;
    let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
    let meta = GetKeyState(VK_LWIN.0 as i32) < 0 || GetKeyState(VK_RWIN.0 as i32) < 0;
    let has_modifier = ctrl || alt || shift || meta;
    // Modifier keys alone do not complete a chord.
    if matches!(vk, 0x10 | 0x11 | 0x12 | 0x5B | 0x5C) {
        return;
    }
    if !has_modifier {
        match vk {
            0x1B => {
                // Hand activation back before destroying, like the OK/Cancel paths.
                if let Ok(parent) = GetParent(dialog) {
                    let _ = EnableWindow(parent, true);
                }
                let _ = DestroyWindow(dialog);
                return;
            }
            0x0D => {
                accept_hotkey_dialog(dialog);
                return;
            }
            0x08 | 0x2E => {
                if let Some(state) = (GetWindowLongPtrW(dialog, GWLP_USERDATA)
                    as *mut HotkeyPromptState)
                    .as_mut()
                {
                    state.sequence = None;
                }
                let empty = wide_z("");
                let _ = SetWindowTextW(edit, PCWSTR(empty.as_ptr()));
                return;
            }
            _ => {}
        }
    }
    if let Some(key) = KeyCode::from_vk(vk) {
        let sequence = KeySequence::new(Modifiers { ctrl, alt, shift, meta }, key);
        if let Some(state) = (GetWindowLongPtrW(dialog, GWLP_USERDATA)
            as *mut HotkeyPromptState)
            .as_mut()
        {
            state.sequence = Some(sequence);
        }
        let text = wide_z(&sequence.to_string_qt());
        let _ = SetWindowTextW(edit, PCWSTR(text.as_ptr()));
    }
}

/// Chords that Windows reserves for its own window/shell commands. Trying to
/// register them either fails or makes Windows minimize/close/switch the
/// focused window when the chord is pressed.
fn reserved_hotkey(sequence: &KeySequence) -> bool {
    let modifiers = sequence.modifiers;
    if modifiers.meta {
        return true;
    }
    if modifiers.alt {
        if matches!(
            sequence.key,
            KeyCode::Function(4)
                | KeyCode::Function(7)
                | KeyCode::Function(8)
                | KeyCode::Function(9)
                | KeyCode::Function(10)
                | KeyCode::Space
                | KeyCode::Tab
                | KeyCode::Escape
                | KeyCode::Print
        ) {
            return true;
        }
    }
    if modifiers.ctrl && !modifiers.alt && matches!(sequence.key, KeyCode::Escape) {
        return true;
    }
    if !modifiers.ctrl
        && !modifiers.alt
        && !modifiers.shift
        && matches!(sequence.key, KeyCode::Print)
    {
        return true;
    }
    false
}

unsafe fn accept_hotkey_dialog(hwnd: HWND) {
    if let Some(state) =
        (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HotkeyPromptState).as_mut()
    {
        if let Some(sequence) = state.sequence {
            if reserved_hotkey(&sequence) {
                let message = wide_z(&i18n::tr(
                    "This key combination is reserved by Windows. Please choose another one.",
                ));
                let _ = MessageBoxW(
                    Some(hwnd),
                    PCWSTR(message.as_ptr()),
                    w!("Equalizer APO"),
                    MB_OK | MB_ICONWARNING,
                );
                return;
            }
        }
        *state.result.borrow_mut() = Some(state.sequence);
    }
    // Re-enable the owner before destroying the modal window so Windows can
    // hand activation back to the editor instead of dropping it to the
    // background (or minimizing it).
    if let Ok(parent) = GetParent(hwnd) {
        let _ = EnableWindow(parent, true);
    }
    let _ = DestroyWindow(hwnd);
}

unsafe extern "system" fn hotkey_prompt_wndproc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            let state = Box::from_raw(create.lpCreateParams as *mut HotkeyPromptState);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
            let Some(state) =
                (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HotkeyPromptState).as_mut()
            else {
                return LRESULT(-1);
            };
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let dpi = state.dpi;
            let initial = state
                .sequence
                .map(|sequence| sequence.to_string_qt())
                .unwrap_or_default();
            let initial = wide_z(&initial);
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                PCWSTR(initial.as_ptr()),
                WINDOW_STYLE(
                    WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32,
                ),
                scale(12, dpi),
                scale(36, dpi),
                (client.right - scale(24, dpi)).max(scale(120, dpi)),
                scale(24, dpi),
                Some(hwnd),
                None,
                None,
                None,
            );
            if let Ok(edit) = edit {
                SendMessageW(
                    edit,
                    WM_SETFONT,
                    Some(WPARAM(state.font.0 as usize)),
                    Some(LPARAM(1)),
                );
                state.edit = edit;
                let _ = SetFocus(Some(edit));
            }
            let make_button = |label: &str, x: i32, id: usize, style: u32| unsafe {
                let text = wide_z(label);
                let _ = CreateWindowExW(
                    WINDOW_EX_STYLE(0),
                    w!("BUTTON"),
                    PCWSTR(text.as_ptr()),
                    WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | style),
                    x,
                    scale(72, dpi),
                    scale(80, dpi),
                    scale(26, dpi),
                    Some(hwnd),
                    Some(HMENU(id as *mut core::ffi::c_void)),
                    None,
                    None,
                );
            };
            make_button("OK", client.right - scale(190, dpi), 1, 0x0000_0001);
            make_button("Cancel", client.right - scale(100, dpi), 2, 0);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut paint);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            fill(hdc, 0, 0, client.right, client.bottom, WHITE);
            let state =
                (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HotkeyPromptState).as_mut();
            let dpi = state.as_ref().map(|state| state.dpi).unwrap_or(96);
            if let Some(state) = state {
                SelectObject(hdc, HGDIOBJ(state.font.0));
            }
            draw_text(
                hdc,
                scale(12, dpi),
                scale(12, dpi),
                &trw("Press the desired key combination (Enter = OK, Esc = Cancel):"),
                TEXT,
            );
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as usize;
            if id == 1 {
                accept_hotkey_dialog(hwnd);
            } else if id == IDCANCEL.0 as usize {
                if let Ok(parent) = GetParent(hwnd) {
                    let _ = EnableWindow(parent, true);
                }
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_APP_HOTKEY_ACCEPT => {
            accept_hotkey_dialog(hwnd);
            LRESULT(0)
        }
        WM_CLOSE => {
            if let Ok(parent) = GetParent(hwnd) {
                let _ = EnableWindow(parent, true);
            }
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut HotkeyPromptState;
            if let Some(state) = pointer.as_mut() {
                if !state.font.is_invalid() {
                    let _ = DeleteObject(HGDIOBJ(state.font.0));
                }
            }
            if !pointer.is_null() {
                drop(Box::from_raw(pointer));
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

/// Prompts for a hotkey by capturing actual key presses. `None` means the
/// dialog was cancelled; `Some(None)` clears the hotkey.
pub(super) unsafe fn prompt_hotkey(
    parent: HWND,
    title: &str,
    initial: Option<KeySequence>,
) -> Option<Option<KeySequence>> {
    HOTKEY_PROMPT_CLASS.get_or_init(|| {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let Ok(cursor) = LoadCursorW(None, IDC_ARROW) else {
            return;
        };
        let name = wide_z("EapoHotkeyPromptDialog");
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(hotkey_prompt_wndproc),
            hInstance: HINSTANCE(module.0),
            hCursor: cursor,
            lpszClassName: PCWSTR(name.as_ptr()),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
    });
    let name = wide_z("EapoHotkeyPromptDialog");
    let title = wide_z(title);
    let dpi = GetDpiForWindow(parent).max(96);
    let font = CreateFontW(
        -((9 * dpi as i32 + 36) / 72),
        0,
        0,
        0,
        400,
        0,
        0,
        0,
        DEFAULT_CHARSET,
        OUT_DEFAULT_PRECIS,
        CLIP_DEFAULT_PRECIS,
        CLEARTYPE_QUALITY,
        DEFAULT_PITCH.0 as u32,
        w!("Segoe UI"),
    );
    let mut parent_rect = RECT::default();
    let _ = GetWindowRect(parent, &mut parent_rect);
    let width = scale(520, dpi);
    let height = scale(150, dpi);
    let x = parent_rect.left + ((parent_rect.right - parent_rect.left) - width) / 2;
    let y = parent_rect.top + ((parent_rect.bottom - parent_rect.top) - height) / 2;
    let result = Rc::new(RefCell::new(None));
    let state = Box::new(HotkeyPromptState {
        edit: HWND::default(),
        sequence: initial,
        result: result.clone(),
        dpi,
        font,
    });
    let dialog = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        PCWSTR(name.as_ptr()),
        PCWSTR(title.as_ptr()),
        WINDOW_STYLE(WS_POPUP.0 | WS_CAPTION.0 | WS_SYSMENU.0 | WS_VISIBLE.0),
        x,
        y,
        width,
        height,
        Some(parent),
        None,
        None,
        Some(Box::into_raw(state) as *const core::ffi::c_void),
    );
    let Ok(dialog) = dialog else {
        return None;
    };
    let _ = ShowWindow(dialog, SW_SHOW);
    let _ = SetForegroundWindow(dialog);
    let _ = EnableWindow(parent, false);
    let edit = (GetWindowLongPtrW(dialog, GWLP_USERDATA) as *mut HotkeyPromptState)
        .as_ref()
        .map(|state| state.edit)
        .unwrap_or_default();
    let mut message = MSG::default();
    while IsWindow(Some(dialog)).as_bool() {
        if !GetMessageW(&mut message, None, 0, 0).as_bool() {
            break;
        }
        // Capture chords before the dialog manager sees them; the edit is a
        // plain display control and must not process the keys itself.
        if message.hwnd == edit
            && matches!(message.message, WM_KEYDOWN | WM_SYSKEYDOWN)
        {
            handle_hotkey_key(dialog, edit, message.wParam.0 as u32);
            continue;
        }
        if message.hwnd == edit && matches!(message.message, WM_CHAR | WM_SYSCHAR) {
            continue;
        }
        if !IsDialogMessageW(dialog, &mut message).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    let _ = EnableWindow(parent, true);
    let value = result.borrow_mut().take();
    value
}
