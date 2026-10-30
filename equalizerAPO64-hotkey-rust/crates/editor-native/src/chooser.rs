//! Modal check-list dialog used by the Channel and Device row editors.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use windows::core::w;
use windows::Win32::Foundation::{HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, GetSysColorBrush, HGDIOBJ, HFONT, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, COLOR_BTNFACE, DEFAULT_CHARSET, DEFAULT_PITCH, OUT_DEFAULT_PRECIS,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetParent, GetWindowLongPtrW, GetWindowRect, IsDialogMessageW, IsWindow, LoadCursorW, RegisterClassW,
    SendMessageW, SetForegroundWindow, SetWindowLongPtrW, ShowWindow, TranslateMessage,
    CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HMENU, IDC_ARROW, MSG, SW_SHOW,
    WM_CLOSE, WM_COMMAND, WM_CREATE, WM_NCDESTROY, WM_SETFONT, WNDCLASSW, WINDOW_EX_STYLE,
    WINDOW_STYLE, WS_CAPTION, WS_CHILD, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

const ID_OK: usize = 1;
const ID_CANCEL: usize = 2;
const ID_ALL: usize = 100;
const ID_FILTER: usize = 101;
const ID_ITEM_BASE: usize = 200;

const BS_AUTOCHECKBOX: u32 = 0x0000_0003;
const BS_DEFPUSHBUTTON: u32 = 0x0000_0001;
const BM_SETCHECK: u32 = 0x00F1;
const BM_GETCHECK: u32 = 0x00F0;
const BST_CHECKED: usize = 1;

/// Result of a modal check list: either "all" was selected, or the indices of
/// the checked items.
pub struct CheckList {
    pub all: bool,
    pub checked: Vec<usize>,
}

pub struct CheckListOptions {
    pub all_label: Option<String>,
    pub all_checked: bool,
    /// Optional filter checkbox: label, initial state, and which items remain
    /// selectable while it is checked.
    pub filter: Option<(String, bool, Vec<bool>)>,
}

struct ChooserState {
    items: Vec<String>,
    checked: Vec<bool>,
    keep: Vec<bool>,
    all_label: Option<String>,
    all_checked: bool,
    filter_label: Option<String>,
    filter_checked: bool,
    buttons: Vec<HWND>,
    all: Option<HWND>,
    filter: Option<HWND>,
    font: HFONT,
    result: Rc<RefCell<Option<CheckList>>>,
}

static CHOOSER_CLASS: OnceLock<()> = OnceLock::new();

fn scale(px: i32, dpi: u32) -> i32 {
    (px as f64 * dpi as f64 / 96.0).round() as i32
}

fn column_count(items: usize) -> i32 {
    (((items + 17) / 18).clamp(1, 3)) as i32
}

fn row_count(items: usize, columns: i32) -> i32 {
    let columns = columns.max(1) as usize;
    ((items + columns - 1) / columns) as i32
}

unsafe fn is_checked(hwnd: HWND) -> bool {
    SendMessageW(hwnd, BM_GETCHECK, None, None).0 as usize == BST_CHECKED
}

unsafe fn set_checked(hwnd: HWND) {
    SendMessageW(hwnd, BM_SETCHECK, Some(WPARAM(BST_CHECKED)), None);
}

unsafe fn state_of(hwnd: HWND) -> Option<&'static mut ChooserState> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ChooserState).as_mut()
}

unsafe fn refresh_enabled(hwnd: HWND) {
    let Some(state) = state_of(hwnd) else {
        return;
    };
    let all_on = state.all.map(|all| is_checked(all)).unwrap_or(false);
    let filter_on = state.filter.map(|filter| is_checked(filter)).unwrap_or(false);
    for (index, button) in state.buttons.iter().enumerate() {
        let keep = !filter_on || state.keep.get(index).copied().unwrap_or(true);
        let _ = EnableWindow(*button, !all_on && keep);
    }
}

/// Re-enables the owner before the modal dialog disappears so Windows hands
/// activation back to the editor instead of leaving it in the background.
unsafe fn reenable_owner(hwnd: HWND) {
    if let Ok(parent) = GetParent(hwnd) {
        let _ = EnableWindow(parent, true);
    }
}

unsafe extern "system" fn chooser_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            let state = Box::from_raw(create.lpCreateParams as *mut ChooserState);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
            let Some(state) = state_of(hwnd) else {
                return LRESULT(-1);
            };
            build_controls(hwnd, state);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as usize;
            match id {
                ID_OK => {
                    if let Some(state) = state_of(hwnd) {
                        let all_checked = state.all.map(|all| is_checked(all)).unwrap_or(false);
                        let checked = if all_checked {
                            Vec::new()
                        } else {
                            state
                                .buttons
                                .iter()
                                .enumerate()
                                .filter(|(_, button)| is_checked(**button))
                                .map(|(index, _)| index)
                                .collect()
                        };
                        *state.result.borrow_mut() = Some(CheckList {
                            all: all_checked,
                            checked,
                        });
                    }
                    reenable_owner(hwnd);
                    let _ = DestroyWindow(hwnd);
                }
                ID_CANCEL => {
                    reenable_owner(hwnd);
                    let _ = DestroyWindow(hwnd);
                }
                ID_ALL | ID_FILTER => refresh_enabled(hwnd),
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            reenable_owner(hwnd);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut ChooserState;
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

#[allow(clippy::too_many_arguments)]
unsafe fn create_control(
    parent: HWND,
    class: windows::core::PCWSTR,
    label: &str,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: usize,
    font: HFONT,
) -> Option<HWND> {
    let text: Vec<u16> = label.encode_utf16().chain(std::iter::once(0)).collect();
    let control = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        class,
        windows::core::PCWSTR(text.as_ptr()),
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | style),
        x,
        y,
        width,
        height,
        Some(parent),
        Some(HMENU(id as *mut core::ffi::c_void)),
        None,
        None,
    )
    .ok()?;
    if !font.is_invalid() {
        SendMessageW(
            control,
            WM_SETFONT,
            Some(WPARAM(font.0 as usize)),
            Some(LPARAM(1)),
        );
    }
    Some(control)
}

unsafe fn build_controls(hwnd: HWND, state: &mut ChooserState) {
    let dpi = GetDpiForWindow(hwnd).max(96);
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let margin = scale(12, dpi);
    let line = scale(24, dpi);
    let columns = column_count(state.items.len());
    let rows = row_count(state.items.len(), columns);
    let column_width = (client.right - margin * 2) / columns.max(1);
    let mut y = margin;
    if let Some(label) = state.all_label.clone() {
        if let Some(all) = create_control(
            hwnd,
            w!("BUTTON"),
            &label,
            BS_AUTOCHECKBOX,
            margin,
            y,
            client.right - margin * 2,
            line,
            ID_ALL,
            state.font,
        ) {
            if state.all_checked {
                set_checked(all);
            }
            state.all = Some(all);
        }
        y += line;
    }
    if let Some(label) = state.filter_label.clone() {
        if let Some(filter) = create_control(
            hwnd,
            w!("BUTTON"),
            &label,
            BS_AUTOCHECKBOX,
            margin,
            y,
            client.right - margin * 2,
            line,
            ID_FILTER,
            state.font,
        ) {
            if state.filter_checked {
                set_checked(filter);
            }
            state.filter = Some(filter);
        }
        y += line;
    }
    if state.all_label.is_some() || state.filter_label.is_some() {
        y += scale(4, dpi);
    }
    let top = y;
    for index in 0..state.items.len() {
        let column = index as i32 / rows.max(1);
        let row = index as i32 % rows.max(1);
        let x = margin + column * column_width;
        if let Some(control) = create_control(
            hwnd,
            w!("BUTTON"),
            &state.items[index],
            BS_AUTOCHECKBOX,
            x,
            top + row * line,
            column_width - scale(8, dpi),
            line,
            ID_ITEM_BASE + index,
            state.font,
        ) {
            if state.checked.get(index).copied().unwrap_or(false) {
                set_checked(control);
            }
            state.buttons.push(control);
        }
    }
    let bottom = client.bottom - scale(40, dpi);
    let _ = create_control(
        hwnd,
        w!("BUTTON"),
        "OK",
        BS_DEFPUSHBUTTON,
        client.right - scale(190, dpi),
        bottom,
        scale(80, dpi),
        scale(26, dpi),
        ID_OK,
        state.font,
    );
    let _ = create_control(
        hwnd,
        w!("BUTTON"),
        "Cancel",
        0,
        client.right - scale(100, dpi),
        bottom,
        scale(80, dpi),
        scale(26, dpi),
        ID_CANCEL,
        state.font,
    );
    refresh_enabled(hwnd);
}

/// Shows a modal check list. Returns `None` when the user cancels.
pub unsafe fn check_list(
    parent: HWND,
    title: &str,
    items: &[String],
    checked: &[usize],
    options: CheckListOptions,
) -> Option<CheckList> {
    CHOOSER_CLASS.get_or_init(|| {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let Ok(cursor) = LoadCursorW(None, IDC_ARROW) else {
            return;
        };
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(chooser_proc),
            hInstance: HINSTANCE(module.0),
            hCursor: cursor,
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: w!("EapoChooserDialog"),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
    });
    let dpi = GetDpiForWindow(parent).max(96);
    let margin = scale(12, dpi);
    let line = scale(24, dpi);
    let columns = column_count(items.len());
    let rows = row_count(items.len(), columns);
    let width = margin * 2 + columns * scale(460, dpi);
    let height = (margin
        + if options.all_label.is_some() { line } else { 0 }
        + if options.filter.is_some() { line } else { 0 }
        + scale(4, dpi)
        + rows * line
        + scale(52, dpi))
        .max(scale(160, dpi));
    let mut parent_rect = RECT::default();
    let _ = GetWindowRect(parent, &mut parent_rect);
    let x = parent_rect.left + ((parent_rect.right - parent_rect.left) - width) / 2;
    let y = parent_rect.top + ((parent_rect.bottom - parent_rect.top) - height) / 2;
    let title: Vec<u16> = title.encode_utf16().chain(std::iter::once(0)).collect();
    let result = Rc::new(RefCell::new(None));
    let font = CreateFontW(
        -(9 * dpi as i32) / 72,
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
    let filter = options.filter;
    let keep = filter
        .as_ref()
        .map(|(_, _, keep)| {
            (0..items.len())
                .map(|index| keep.get(index).copied().unwrap_or(true))
                .collect()
        })
        .unwrap_or_default();
    let state = Box::new(ChooserState {
        items: items.to_vec(),
        checked: (0..items.len()).map(|index| checked.contains(&index)).collect(),
        keep,
        all_label: options.all_label.clone(),
        all_checked: options.all_checked,
        filter_label: filter.as_ref().map(|(label, _, _)| label.clone()),
        filter_checked: filter.as_ref().map(|(_, checked, _)| *checked).unwrap_or(false),
        buttons: Vec::with_capacity(items.len()),
        all: None,
        filter: None,
        font,
        result: result.clone(),
    });
    let dialog = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("EapoChooserDialog"),
        windows::core::PCWSTR(title.as_ptr()),
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
    let mut message = MSG::default();
    while IsWindow(Some(dialog)).as_bool() {
        if !GetMessageW(&mut message, None, 0, 0).as_bool() {
            break;
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
