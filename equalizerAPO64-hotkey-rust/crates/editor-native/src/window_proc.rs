//! Window and inline-edit window procedures for the native editor.

use std::cell::{RefCell, RefMut};
use std::collections::VecDeque;
use std::ops::{Deref, DerefMut};

use super::*;

/// The window stores the editor state behind a `RefCell` so a nested message
/// loop (message boxes, popup menus, the VST host window) can never create a
/// second `&mut App` while a handler still holds the first one.
type AppCell = RefCell<App>;

thread_local! {
    /// Messages that arrived while the state was borrowed.  They are posted
    /// by the inline edit controls and must not be dropped, otherwise an edit
    /// committed by a menu that opens inside another handler would be lost.
    static DEFERRED_MESSAGES: RefCell<VecDeque<(isize, u32, WPARAM, LPARAM)>> =
        const { RefCell::new(VecDeque::new()) };
}

pub(crate) struct AppRef(Option<RefMut<'static, App>>);

impl AppRef {
    fn new(borrow: RefMut<'static, App>) -> Self {
        Self(Some(borrow))
    }
}

impl Deref for AppRef {
    type Target = App;

    fn deref(&self) -> &App {
        self.0.as_deref().expect("app borrow")
    }
}

impl DerefMut for AppRef {
    fn deref_mut(&mut self) -> &mut App {
        self.0.as_deref_mut().expect("app borrow")
    }
}

impl Drop for AppRef {
    fn drop(&mut self) {
        // Release the borrow before dispatching deferred messages: their
        // handlers need to borrow the same state again.
        self.0.take();
        flush_deferred_messages();
    }
}

unsafe fn app_cell(hwnd: HWND) -> Option<&'static AppCell> {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppCell;
    pointer.as_ref()
}

pub(crate) unsafe fn app(hwnd: HWND) -> Option<AppRef> {
    let cell = app_cell(hwnd)?;
    cell.try_borrow_mut().ok().map(AppRef::new)
}

/// Messages that mutate the document from a control and therefore must be
/// replayed after the current handler releases the state.  Timer ticks are
/// not deferred: dropping one only delays the next poll.
fn deferrable(msg: u32) -> bool {
    matches!(
        msg,
        WM_APP_COMMIT_EDIT
            | WM_APP_CANCEL_EDIT
            | WM_APP_COMMIT_VALUE
            | WM_APP_CANCEL_VALUE
    )
}

fn flush_deferred_messages() {
    loop {
        let next = DEFERRED_MESSAGES.with(|queue| queue.borrow_mut().pop_front());
        let Some((hwnd, msg, wparam, lparam)) = next else {
            break;
        };
        unsafe {
            SendMessageW(HWND(hwnd as *mut core::ffi::c_void), msg, Some(wparam), Some(lparam));
        }
    }
}

pub(crate) unsafe extern "system" fn value_edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let previous = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    match msg {
        WM_KEYDOWN if wparam.0 == VK_RETURN.0 as usize => {
            if let Ok(parent) = GetParent(hwnd) {
                let _ = PostMessageW(Some(parent), WM_APP_COMMIT_VALUE, WPARAM(0), LPARAM(0));
            }
            return LRESULT(0);
        }
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
            if let Ok(parent) = GetParent(hwnd) {
                let _ = PostMessageW(Some(parent), WM_APP_CANCEL_VALUE, WPARAM(0), LPARAM(0));
            }
            return LRESULT(0);
        }
        WM_KILLFOCUS => {
            if let Ok(parent) = GetParent(hwnd) {
                let _ = PostMessageW(Some(parent), WM_APP_COMMIT_VALUE, WPARAM(0), LPARAM(0));
            }
        }
        _ => {}
    }
    if previous != 0 {
        let procedure: WNDPROC = std::mem::transmute(previous);
        CallWindowProcW(procedure, hwnd, msg, wparam, lparam)
    } else {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

pub(crate) unsafe extern "system" fn edit_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    let previous = GetWindowLongPtrW(hwnd, GWLP_USERDATA);
    match msg {
        WM_KEYDOWN if wparam.0 == VK_RETURN.0 as usize => {
            if let Ok(parent) = GetParent(hwnd) {
                let _ = PostMessageW(Some(parent), WM_APP_COMMIT_EDIT, WPARAM(0), LPARAM(0));
            }
            return LRESULT(0);
        }
        WM_KEYDOWN if wparam.0 == VK_ESCAPE.0 as usize => {
            if let Ok(parent) = GetParent(hwnd) {
                let _ = PostMessageW(Some(parent), WM_APP_CANCEL_EDIT, WPARAM(0), LPARAM(0));
            }
            return LRESULT(0);
        }
        WM_KILLFOCUS => {
            if let Ok(parent) = GetParent(hwnd) {
                let _ = PostMessageW(Some(parent), WM_APP_COMMIT_EDIT, WPARAM(0), LPARAM(0));
            }
        }
        _ => {}
    }
    if previous != 0 {
        let procedure: WNDPROC = std::mem::transmute(previous);
        CallWindowProcW(procedure, hwnd, msg, wparam, lparam)
    } else {
        DefWindowProcW(hwnd, msg, wparam, lparam)
    }
}

pub(crate) unsafe extern "system" fn main_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    // A nested message loop may deliver an edit-commit message while the
    // handler that opened the loop still owns the state.  Replay it once the
    // borrow is released instead of losing the edit.
    if deferrable(msg) {
        if let Some(cell) = app_cell(hwnd) {
            if cell.try_borrow_mut().is_err() {
                DEFERRED_MESSAGES.with(|queue| {
                    queue
                        .borrow_mut()
                        .push_back((hwnd.0 as isize, msg, wparam, lparam));
                });
                return LRESULT(0);
            }
        }
    }
    match msg {
        WM_CREATE => {
            let dpi = GetDpiForWindow(hwnd).max(96);
            let state = Box::new(RefCell::new(App::new(dpi)));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
            if let Some(state) = app(hwnd) {
                state.update_title(hwnd);
                state.sync_live_menu(hwnd);
                SetTimer(Some(hwnd), TIMER_ANALYSIS, 100, None);
            }
            #[cfg(debug_assertions)]
            if scroll_test_enabled() {
                SetTimer(Some(hwnd), TIMER_SCROLL_TEST, 10, None);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            if let Some(mut state) = app(hwnd) {
                state.commit_value_edit(hwnd, true);
                state.update_scrollbar(hwnd);
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut paint);
            if let Some(mut state) = app(hwnd) {
                // Double-buffer the invalid region: GDI painting without a
                // back buffer flickers heavily while hovering/dragging.
                let rc = paint.rcPaint;
                let width = (rc.right - rc.left).max(1);
                let height = (rc.bottom - rc.top).max(1);
                let mem = CreateCompatibleDC(Some(hdc));
                let bitmap = CreateCompatibleBitmap(hdc, width, height);
                if !mem.0.is_null() && !bitmap.0.is_null() {
                    let old_bitmap = SelectObject(mem, HGDIOBJ(bitmap.0));
                    let mut previous = POINT::default();
                    let _ = SetViewportOrgEx(mem, -rc.left, -rc.top, Some(&mut previous));
                    state.paint_offset = (rc.left, rc.top);
                    state.paint(hwnd, mem);
                    let _ = SetViewportOrgEx(mem, previous.x, previous.y, None);
                    let _ = BitBlt(hdc, rc.left, rc.top, width, height, Some(mem), 0, 0, SRCCOPY);
                    SelectObject(mem, old_bitmap);
                    let _ = DeleteObject(HGDIOBJ(bitmap.0));
                    let _ = DeleteDC(mem);
                } else {
                    if !mem.0.is_null() {
                        let _ = DeleteDC(mem);
                    }
                    state.paint_offset = (0, 0);
                    state.paint(hwnd, hdc);
                }
            }
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_ERASEBKGND => LRESULT(1),
        WM_PRINTCLIENT => {
            if let Some(mut state) = app(hwnd) {
                state.paint_offset = (0, 0);
                state.paint(hwnd, HDC(wparam.0 as *mut core::ffi::c_void));
            }
            LRESULT(0)
        }
        WM_SYSCOMMAND => {
            if (wparam.0 & 0xFFF0) == SC_KEYMENU as usize
                && SUPPRESS_KEYMENU.swap(false, Ordering::Relaxed)
            {
                return LRESULT(0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        WM_VSCROLL => {
            if let Some(mut state) = app(hwnd) {
                state.commit_edit(hwnd, true);
                state.commit_value_edit(hwnd, true);
                state.on_vscroll(hwnd, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            if let Some(mut state) = app(hwnd) {
                state.commit_edit(hwnd, true);
                state.on_wheel(hwnd, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if let Some(mut state) = app(hwnd) {
                state.on_key(hwnd, wparam);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let _ = SetFocus(Some(hwnd));
            if let Some(mut state) = app(hwnd) {
                state.on_click(hwnd, lparam);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Some(mut state) = app(hwnd) {
                let mut track = TRACKMOUSEEVENT {
                    cbSize: size_of::<TRACKMOUSEEVENT>() as u32,
                    dwFlags: TME_LEAVE,
                    hwndTrack: hwnd,
                    dwHoverTime: 0,
                };
                let _ = TrackMouseEvent(&mut track);
                state.on_mouse_move(hwnd, lparam);
            }
            LRESULT(0)
        }
        WM_MOUSELEAVE => {
            if let Some(mut state) = app(hwnd) {
                state.hover_pos = None;
                if let Some(rect) = state.hot_rect.take() {
                    let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if let Some(mut state) = app(hwnd) {
                state.on_mouse_up(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            if let Some(mut state) = app(hwnd) {
                state.on_click(hwnd, lparam);
                if let Some(index) = state.selected {
                    let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
                    let mouse_y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                    let row_y = state.chrome_height()
                        + state.prefix[index] as i32
                        - state.offset;
                    if !state.handle_editor_double_click(hwnd, index, x, mouse_y, row_y) {
                        state.begin_edit(hwnd, index);
                    }
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONDOWN => {
            let _ = SetFocus(Some(hwnd));
            if let Some(mut state) = app(hwnd) {
                let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
                let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                if state.show_analysis && point_in(&state.plot_rect, x, y) {
                    state.plot_pan = Some((x, y));
                    let _ = SetCapture(hwnd);
                } else {
                    state.on_context_menu(hwnd, lparam);
                }
            }
            LRESULT(0)
        }
        WM_RBUTTONUP => {
            if let Some(mut state) = app(hwnd) {
                if state.plot_pan.take().is_some() {
                    let _ = ReleaseCapture();
                    state.invalidate_plot(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_APP_COMMIT_EDIT => {
            if let Some(mut state) = app(hwnd) {
                state.commit_edit(hwnd, true);
            }
            LRESULT(0)
        }
        WM_APP_CANCEL_EDIT => {
            if let Some(mut state) = app(hwnd) {
                state.commit_edit(hwnd, false);
            }
            LRESULT(0)
        }
        WM_APP_COMMIT_VALUE => {
            if let Some(mut state) = app(hwnd) {
                state.commit_value_edit(hwnd, true);
            }
            LRESULT(0)
        }
        WM_APP_CANCEL_VALUE => {
            if let Some(mut state) = app(hwnd) {
                state.commit_value_edit(hwnd, false);
            }
            LRESULT(0)
        }
        WM_DPICHANGED => {
            let dpi = (wparam.0 & 0xFFFF) as u32;
            let suggested = &*(lparam.0 as *const RECT);
            let _ = SetWindowPos(
                hwnd,
                None,
                suggested.left,
                suggested.top,
                suggested.right - suggested.left,
                suggested.bottom - suggested.top,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
            if let Some(mut state) = app(hwnd) {
                state.commit_value_edit(hwnd, true);
                state.commit_edit(hwnd, true);
                state.set_dpi(dpi);
                state.update_scrollbar(hwnd);
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_SETTINGCHANGE | WM_THEMECHANGED => {
            palette::refresh();
            let _ = InvalidateRect(Some(hwnd), None, true);
            LRESULT(0)
        }
        WM_TIMER => {
            #[cfg(debug_assertions)]
            {
                if wparam.0 == TIMER_SCROLL_TEST {
                    let finished = app(hwnd)
                        .map(|mut state| state.scroll_test_tick())
                        .unwrap_or(true);
                    if finished {
                        let _ = KillTimer(Some(hwnd), TIMER_SCROLL_TEST);
                        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    } else {
                        let _ = SendMessageW(
                            hwnd,
                            WM_VSCROLL,
                            Some(WPARAM(SB_LINEDOWN.0 as usize)),
                            Some(LPARAM(0)),
                        );
                    }
                }
            }
            if wparam.0 == TIMER_ANALYSIS {
                if let Some(mut state) = app(hwnd) {
                    state.poll_analysis(hwnd);
                    let ids = state.hotkeys.poll();
                    for filter_id in ids {
                        state.handle_hotkey(hwnd, &filter_id);
                    }
                    state.poll_volume(hwnd);
                    if state
                        .autosave_due
                        .is_some_and(|due| std::time::Instant::now() >= due)
                    {
                        state.flush_autosave(hwnd);
                    }
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = wparam.0 & 0xFFFF;
            if let Some(mut state) = app(hwnd) {
                match id {
                    ID_EXIT => {
                        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
                    }
                    101 => state.new_tab(hwnd),
                    102 => state.open_dialog(hwnd),
                    ID_SAVE => {
                        state.save(hwnd);
                    }
                    104 => {
                        state.save_as(hwnd);
                    }
                    111 => {
                        state.cut_selection(hwnd);
                    }
                    112 => {
                        state.copy_selection();
                    }
                    113 => {
                        state.paste_clipboard(hwnd);
                    }
                    115 => {
                        state.selection = (0..state.rows.len()).collect();
                        if !state.rows.is_empty() {
                            state.selected = Some(0);
                            state.selection_anchor = Some(0);
                        }
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                    116 => {
                        if let Some(index) = state.selected {
                            state.set_hotkey_for_row(hwnd, index);
                        }
                    }
                    117 => {
                        if let Some(index) = state.selected {
                            state.clear_hotkey_for_row(hwnd, index);
                        }
                    }
                    ID_DELETE => state.remove_selected(hwnd),
                    122 => {
                        state.show_analysis = !state.show_analysis;
                        if state.show_analysis {
                            state.start_analysis();
                        }
                        state.sync_view_menu(hwnd);
                        state.update_scrollbar(hwnd);
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                    123 => {
                        state.show_toolbar = !state.show_toolbar;
                        state.sync_view_menu(hwnd);
                        state.update_scrollbar(hwnd);
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                    124 => {
                        state.reset_preferences(hwnd, false);
                    }
                    125 => {
                        state.reset_preferences(hwnd, true);
                    }
                    130..=132 => {
                        let mode = match id {
                            130 => LiveMode::Off,
                            131 => LiveMode::ActiveTab,
                            _ => LiveMode::AllTabs,
                        };
                        state.set_live_mode(hwnd, mode);
                    }
                    _ => {}
                }
                if (500..510).contains(&id) {
                    if let Some(path) = state.recent_files.get(id - 500).cloned() {
                        state.open_paths(hwnd, vec![PathBuf::from(path)]);
                    }
                }
                if (860..864).contains(&id) {
                    let codes = ["en", "de", "fr", "zh"];
                    state.prefs.language = Some(codes[id - 860].to_string());
                    state.prefs.save();
                    // Win32 message boxes require NUL-terminated strings;
                    // `trw` is length-based and must not be passed directly.
                    let message = wide_z(&i18n::tr(
                        "Please restart the application to apply the language change.",
                    ));
                    let title = wide_z(&i18n::tr("Language"));
                    MessageBoxW(
                        Some(hwnd),
                        PCWSTR(message.as_ptr()),
                        PCWSTR(title.as_ptr()),
                        MB_OK | MB_ICONINFORMATION,
                    );
                }
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            let mut allow = true;
            if let Some(mut state) = app(hwnd) {
                allow = state.confirm_close(hwnd);
                if !allow {
                    // A pending reset/restart must not fire after the user
                    // cancelled the save prompt.
                    state.restart_after_close = false;
                }
            }
            if allow {
                if let Some(mut state) = app(hwnd) {
                    if !state.skip_save_preferences {
                        state.save_preferences(hwnd);
                    }
                    if state.restart_after_close {
                        if let Ok(exe) = std::env::current_exe() {
                            let mut command = std::process::Command::new(exe);
                            command.args(std::env::args_os().skip(1));
                            let _ = command.spawn();
                        }
                    }
                }
                let _ = DestroyWindow(hwnd);
            }
            LRESULT(0)
        }
        WM_DESTROY => {
            PostQuitMessage(0);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut AppCell;
            if !pointer.is_null() {
                drop(Box::from_raw(pointer));
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}
