//! Modal Copy-filter routing editor with Graph and Assignments tabs.

use std::cell::RefCell;
use std::mem::size_of;
use std::rc::Rc;
use std::sync::OnceLock;

use eapo_editor_core::i18n;
use eapo_editor_core::model::{parse_num, qt_num, Assignment, Summand};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, CreateFontW, CreatePen, CreateSolidBrush, DeleteObject, DrawTextW, EndPaint,
    FillRect, GetSysColorBrush, HGDIOBJ, HFONT, InvalidateRect, LineTo, MoveToEx, SelectObject,
    SetBkMode, SetTextColor, CLEARTYPE_QUALITY, CLIP_DEFAULT_PRECIS, COLOR_BTNFACE, COLOR_WINDOW,
    DEFAULT_CHARSET, DEFAULT_PITCH, DT_CENTER, DT_SINGLELINE, DT_VCENTER, HDC, OUT_DEFAULT_PRECIS,
    PAINTSTRUCT, PS_SOLID, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::SetScrollInfo;
use windows::Win32::UI::HiDpi::GetDpiForWindow;
use windows::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetMessageW,
    GetParent, GetWindow, GetWindowLongPtrW, GetWindowRect, GetWindowTextW, IsDialogMessageW, IsWindow,
    LoadCursorW, RegisterClassW, SendMessageW, SetForegroundWindow, SetWindowLongPtrW, SetWindowPos,
    ShowWindow, TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW,
    GWLP_USERDATA, GW_CHILD, GW_HWNDNEXT, IDC_ARROW, MSG, SB_BOTTOM, SB_LINEDOWN, SB_LINEUP,
    SB_PAGEDOWN, SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT, SCROLLINFO, SIF_PAGE,
    SIF_POS, SIF_RANGE, SWP_NOACTIVATE, SWP_NOZORDER, SW_HIDE, SW_SHOW, SW_SHOWNA, WM_CLOSE,
    WM_COMMAND, WM_CREATE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_PRINTCLIENT, WM_SETFONT,
    WM_SIZE, WM_VSCROLL, WNDCLASSW, WINDOW_EX_STYLE, WINDOW_STYLE, WS_CAPTION, WS_CHILD,
    WS_CLIPCHILDREN, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

const ID_OK: usize = 1;
const ID_CANCEL: usize = 2;
const ID_TAB_GRAPH: usize = 10;
const ID_TAB_ASSIGN: usize = 11;
const ID_ADD_ASSIGNMENT: usize = 20;
const ID_ROW_BASE: usize = 3000;
const ID_MODE_BASE: usize = 8000;

const ES_AUTOHSCROLL: u32 = 0x0080;
const CBS_DROPDOWNLIST: u32 = 0x0003;
const BS_AUTORADIOBUTTON: u32 = 0x0009;
const BS_DEFPUSHBUTTON: u32 = 0x0001;
const WS_GROUP: u32 = 0x0002_0000;
const CB_ADDSTRING: u32 = 0x0143;
const CB_SETCURSEL: u32 = 0x014E;
const CB_GETCURSEL: u32 = 0x0147;
const BM_SETCHECK: u32 = 0x00F1;
const BST_CHECKED: usize = 1;

fn modes() -> [String; 4] {
    [
        i18n::tr("Channel"),
        i18n::tr("Factor * channel"),
        i18n::tr("dB factor * channel"),
        i18n::tr("Constant value"),
    ]
}

struct RowCtl {
    assignment: usize,
    summand: usize,
    target: Option<HWND>,
    lead: HWND,
    mode: HWND,
    factor: HWND,
    star: HWND,
    channel: HWND,
    add: HWND,
    remove: HWND,
}

struct CopyState {
    working: Vec<Assignment>,
    channel_names: Vec<String>,
    tab: usize,
    scroll: i32,
    rows: Vec<RowCtl>,
    tab_graph: HWND,
    tab_assign: HWND,
    add_assignment: HWND,
    ok: HWND,
    cancel: HWND,
    font: HFONT,
    small: HFONT,
    dpi: u32,
    content_top: i32,
    content_bottom: i32,
    content_height: i32,
    visible_height: i32,
    result: Rc<RefCell<Option<Vec<Assignment>>>>,
}

static COPY_CLASS: OnceLock<()> = OnceLock::new();

fn scale(px: i32, dpi: u32) -> i32 {
    (px as f64 * dpi as f64 / 96.0).round() as i32
}

fn wide_z(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn mode_of(summand: &Summand) -> usize {
    if summand.channel.is_empty() {
        3
    } else if summand.is_decibel {
        2
    } else if summand.factor != 1.0 {
        1
    } else {
        0
    }
}

unsafe fn state_of(hwnd: HWND) -> Option<&'static mut CopyState> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CopyState).as_mut()
}

unsafe fn get_text(hwnd: HWND) -> String {
    let mut buffer = vec![0u16; 512];
    let len = GetWindowTextW(hwnd, &mut buffer);
    String::from_utf16_lossy(&buffer[..len.max(0) as usize])
}

unsafe fn combo_selection(combo: HWND) -> usize {
    SendMessageW(combo, CB_GETCURSEL, None, None).0 as usize
}

#[allow(clippy::too_many_arguments)]
unsafe fn create_child(
    parent: HWND,
    class: PCWSTR,
    text: &str,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: usize,
    font: HFONT,
) -> Option<HWND> {
    let text = wide_z(text);
    let control = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        class,
        PCWSTR(text.as_ptr()),
        WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_TABSTOP.0 | style),
        x,
        y,
        width,
        height,
        Some(parent),
        Some(windows::Win32::UI::WindowsAndMessaging::HMENU(
            id as *mut core::ffi::c_void,
        )),
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

unsafe fn destroy_children(dialog: HWND) {
    let mut child = GetWindow(dialog, GW_CHILD);
    while let Ok(current) = child {
        let next = GetWindow(current, GW_HWNDNEXT);
        let _ = DestroyWindow(current);
        child = next;
    }
}

unsafe fn collect(state: &mut CopyState) {
    for row in &state.rows {
        let Some(assignment) = state.working.get_mut(row.assignment) else {
            continue;
        };
        if let Some(target) = row.target {
            assignment.target = get_text(target);
        }
        let Some(summand) = assignment.summands.get_mut(row.summand) else {
            continue;
        };
        let mode = combo_selection(row.mode);
        let factor = parse_num(&get_text(row.factor)).unwrap_or(summand.factor);
        let channel = get_text(row.channel);
        match mode {
            0 => {
                summand.channel = if channel.trim().is_empty() {
                    " ".to_string()
                } else {
                    channel.trim().to_string()
                };
                summand.factor = 1.0;
                summand.is_decibel = false;
            }
            1 => {
                summand.channel = if channel.trim().is_empty() {
                    " ".to_string()
                } else {
                    channel.trim().to_string()
                };
                summand.factor = factor;
                summand.is_decibel = false;
            }
            2 => {
                summand.channel = if channel.trim().is_empty() {
                    " ".to_string()
                } else {
                    channel.trim().to_string()
                };
                summand.factor = factor;
                summand.is_decibel = true;
            }
            _ => {
                summand.channel = String::new();
                summand.factor = factor;
                summand.is_decibel = false;
            }
        }
    }
}

unsafe fn update_modes(state: &CopyState) {
    for row in &state.rows {
        let mode = combo_selection(row.mode);
        let _ = EnableWindow(row.factor, mode != 0);
        let _ = EnableWindow(row.channel, mode != 3);
        if row.star.0.is_null() {
            continue;
        }
        let _ = ShowWindow(row.star, if mode == 1 || mode == 2 { SW_SHOW } else { SW_HIDE });
    }
}

unsafe fn layout_rows(dialog: HWND, state: &mut CopyState) {
    let dpi = state.dpi;
    let row_height = scale(28, dpi);
    let margin = scale(12, dpi);
    let sx = scale(92, dpi);
    let mut y = state.content_top - state.scroll;
    for row in &state.rows {
        let top = y + scale(3, dpi);
        let edit_height = scale(22, dpi);
        if let Some(target) = row.target {
            let _ = SetWindowPos(
                target,
                None,
                margin,
                top,
                scale(64, dpi),
                edit_height,
                SWP_NOZORDER | SWP_NOACTIVATE,
            );
        }
        let _ = SetWindowPos(
            row.lead,
            None,
            margin + scale(70, dpi),
            top + scale(2, dpi),
            scale(16, dpi),
            scale(18, dpi),
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let _ = SetWindowPos(
            row.mode,
            None,
            sx,
            top,
            scale(150, dpi),
            scale(200, dpi),
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let _ = SetWindowPos(
            row.factor,
            None,
            sx + scale(156, dpi),
            top,
            scale(64, dpi),
            edit_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let _ = SetWindowPos(
            row.star,
            None,
            sx + scale(224, dpi),
            top + scale(2, dpi),
            scale(12, dpi),
            scale(18, dpi),
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let _ = SetWindowPos(
            row.channel,
            None,
            sx + scale(238, dpi),
            top,
            scale(70, dpi),
            edit_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let _ = SetWindowPos(
            row.add,
            None,
            sx + scale(314, dpi),
            top,
            scale(24, dpi),
            edit_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let _ = SetWindowPos(
            row.remove,
            None,
            sx + scale(342, dpi),
            top,
            scale(24, dpi),
            edit_height,
            SWP_NOZORDER | SWP_NOACTIVATE,
        );
        let visible = y + row_height >= state.content_top && y < state.content_bottom;
        for control in [
            row.target.unwrap_or_default(),
            row.lead,
            row.mode,
            row.factor,
            row.star,
            row.channel,
            row.add,
            row.remove,
        ] {
            if control.0.is_null() {
                continue;
            }
            let _ = ShowWindow(control, if visible { SW_SHOWNA } else { SW_HIDE });
        }
        y += row_height;
    }
    state.content_height = state.rows.len() as i32 * row_height + scale(8, dpi);
    state.visible_height = state.content_bottom - state.content_top;
    update_modes(state);
    update_scrollbar(dialog, state);
}

unsafe fn update_scrollbar(dialog: HWND, state: &mut CopyState) {
    let max_scroll = (state.content_height - state.visible_height).max(0);
    if state.scroll > max_scroll {
        state.scroll = max_scroll;
    }
    if state.scroll < 0 {
        state.scroll = 0;
    }
    let info = SCROLLINFO {
        cbSize: size_of::<SCROLLINFO>() as u32,
        fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
        nMin: 0,
        nMax: state.content_height.max(1),
        nPage: state.visible_height.max(1) as u32,
        nPos: state.scroll,
        ..Default::default()
    };
    let _ = SetScrollInfo(dialog, SB_VERT, &info, true);
}

unsafe fn build_rows(dialog: HWND, state: &mut CopyState) {
    let dpi = state.dpi;
    for ai in 0..state.working.len() {
        let count = state.working[ai].summands.len();
        for si in 0..count {
            let summand = state.working[ai].summands[si].clone();
            let mode = mode_of(&summand);
            let row_index = state.rows.len();
            let target = if si == 0 {
                create_child(
                    dialog,
                    w!("EDIT"),
                    &state.working[ai].target,
                    ES_AUTOHSCROLL,
                    0,
                    0,
                    scale(64, dpi),
                    scale(22, dpi),
                    0,
                    state.font,
                )
            } else {
                None
            };
            let lead = create_child(
                dialog,
                w!("STATIC"),
                if si == 0 { "=" } else { "+" },
                0,
                0,
                0,
                scale(16, dpi),
                scale(18, dpi),
                0,
                state.font,
            )
            .unwrap_or_default();
            let mode_combo = create_child(
                dialog,
                w!("COMBOBOX"),
                "",
                CBS_DROPDOWNLIST | WS_VSCROLL.0,
                0,
                0,
                scale(150, dpi),
                scale(200, dpi),
                ID_MODE_BASE + row_index,
                state.font,
            )
            .unwrap_or_default();
            if !mode_combo.0.is_null() {
                for label in modes() {
                    let text = wide_z(&label);
                    SendMessageW(
                        mode_combo,
                        CB_ADDSTRING,
                        Some(WPARAM(0)),
                        Some(LPARAM(text.as_ptr() as isize)),
                    );
                }
                SendMessageW(mode_combo, CB_SETCURSEL, Some(WPARAM(mode)), None);
            }
            let factor_text = if mode == 0 {
                "1".to_string()
            } else {
                qt_num(summand.factor)
            };
            let factor = create_child(
                dialog,
                w!("EDIT"),
                &factor_text,
                ES_AUTOHSCROLL,
                0,
                0,
                scale(64, dpi),
                scale(22, dpi),
                0,
                state.font,
            )
            .unwrap_or_default();
            let star = create_child(
                dialog,
                w!("STATIC"),
                "*",
                0,
                0,
                0,
                scale(12, dpi),
                scale(18, dpi),
                0,
                state.font,
            )
            .unwrap_or_default();
            let channel_text = if summand.channel == " " {
                String::new()
            } else {
                summand.channel.clone()
            };
            let channel = create_child(
                dialog,
                w!("EDIT"),
                &channel_text,
                ES_AUTOHSCROLL,
                0,
                0,
                scale(70, dpi),
                scale(22, dpi),
                0,
                state.font,
            )
            .unwrap_or_default();
            let add = create_child(
                dialog,
                w!("BUTTON"),
                "+",
                0,
                0,
                0,
                scale(24, dpi),
                scale(22, dpi),
                ID_ROW_BASE + row_index * 2,
                state.font,
            )
            .unwrap_or_default();
            let remove = create_child(
                dialog,
                w!("BUTTON"),
                "x",
                0,
                0,
                0,
                scale(24, dpi),
                scale(22, dpi),
                ID_ROW_BASE + row_index * 2 + 1,
                state.font,
            )
            .unwrap_or_default();
            state.rows.push(RowCtl {
                assignment: ai,
                summand: si,
                target,
                lead,
                mode: mode_combo,
                factor,
                star,
                channel,
                add,
                remove,
            });
        }
    }
}

unsafe fn rebuild(dialog: HWND) {
    let Some(state) = state_of(dialog) else {
        return;
    };
    if state.tab == 1 && !state.rows.is_empty() {
        collect(state);
    }
    destroy_children(dialog);
    state.rows.clear();
    let mut client = RECT::default();
    let _ = GetClientRect(dialog, &mut client);
    let dpi = state.dpi;
    let margin = scale(12, dpi);
    state.content_top = scale(48, dpi);
    state.content_bottom = client.bottom - scale(56, dpi);
    let _ = ShowWindow(state.add_assignment, SW_HIDE);
    state.tab_graph = create_child(
        dialog,
        w!("BUTTON"),
        &i18n::tr("Graph"),
        BS_AUTORADIOBUTTON | WS_GROUP,
        scale(120, dpi),
        scale(10, dpi),
        scale(70, dpi),
        scale(26, dpi),
        ID_TAB_GRAPH,
        state.font,
    )
    .unwrap_or_default();
    state.tab_assign = create_child(
        dialog,
        w!("BUTTON"),
        &i18n::tr("Assignments"),
        BS_AUTORADIOBUTTON,
        scale(196, dpi),
        scale(10, dpi),
        scale(104, dpi),
        scale(26, dpi),
        ID_TAB_ASSIGN,
        state.font,
    )
    .unwrap_or_default();
    if state.tab == 0 {
        SendMessageW(state.tab_graph, BM_SETCHECK, Some(WPARAM(BST_CHECKED)), None);
    } else {
        SendMessageW(state.tab_assign, BM_SETCHECK, Some(WPARAM(BST_CHECKED)), None);
    }
    let _ = create_child(
        dialog,
        w!("STATIC"),
        &i18n::tr("Copy channels:"),
        0,
        margin,
        scale(14, dpi),
        scale(104, dpi),
        scale(20, dpi),
        0,
        state.font,
    );
    state.add_assignment = create_child(
        dialog,
        w!("BUTTON"),
        &i18n::tr("Add assignment"),
        0,
        margin,
        client.bottom - scale(38, dpi),
        scale(120, dpi),
        scale(26, dpi),
        ID_ADD_ASSIGNMENT,
        state.font,
    )
    .unwrap_or_default();
    state.ok = create_child(
        dialog,
        w!("BUTTON"),
        &i18n::tr("OK"),
        BS_DEFPUSHBUTTON,
        client.right - scale(190, dpi),
        client.bottom - scale(38, dpi),
        scale(80, dpi),
        scale(26, dpi),
        ID_OK,
        state.font,
    )
    .unwrap_or_default();
    state.cancel = create_child(
        dialog,
        w!("BUTTON"),
        &i18n::tr("Cancel"),
        0,
        client.right - scale(100, dpi),
        client.bottom - scale(38, dpi),
        scale(80, dpi),
        scale(26, dpi),
        ID_CANCEL,
        state.font,
    )
    .unwrap_or_default();
    if state.tab == 1 {
        let _ = ShowWindow(state.add_assignment, SW_SHOW);
        build_rows(dialog, state);
    } else {
        let _ = ShowWindow(state.add_assignment, SW_HIDE);
        state.content_height = 0;
        state.scroll = 0;
    }
    layout_rows(dialog, state);
    let _ = InvalidateRect(Some(dialog), None, true);
}

unsafe fn scroll_by(dialog: HWND, delta: i32) {
    let Some(state) = state_of(dialog) else {
        return;
    };
    let max_scroll = (state.content_height - state.visible_height).max(0);
    let next = (state.scroll + delta).clamp(0, max_scroll);
    if next == state.scroll {
        return;
    }
    state.scroll = next;
    layout_rows(dialog, state);
    let _ = InvalidateRect(Some(dialog), None, true);
}

unsafe fn paint_graph(dialog: HWND, hdc: windows::Win32::Graphics::Gdi::HDC, state: &CopyState) {
    let dpi = state.dpi;
    let mut client = RECT::default();
    let _ = GetClientRect(dialog, &mut client);
    let area = RECT {
        left: scale(12, dpi),
        top: state.content_top,
        right: client.right - scale(12, dpi),
        bottom: state.content_bottom,
    };
    let background = GetSysColorBrush(COLOR_WINDOW);
    let _ = FillRect(hdc, &area, background);
    let _ = SetBkMode(hdc, TRANSPARENT);
    let _ = SetTextColor(hdc, COLORREF(0));
    let previous = SelectObject(hdc, HGDIOBJ(state.small.0));
    let box_width = scale(50, dpi);
    let box_height = scale(16, dpi);
    let row_height = scale(18, dpi);
    let input_x = area.left + scale(8, dpi);
    let output_x = area.right - box_width - scale(8, dpi);
    let mut inputs: Vec<String> = state.channel_names.clone();
    inputs.push(String::new());
    for assignment in &state.working {
        for summand in &assignment.summands {
            let channel = summand.channel.trim();
            if !channel.is_empty() && !inputs.iter().any(|n| n.eq_ignore_ascii_case(channel)) {
                inputs.push(channel.to_string());
            }
        }
    }
    let mut outputs: Vec<String> = state.channel_names.clone();
    for assignment in &state.working {
        if !assignment.target.is_empty()
            && !outputs
                .iter()
                .any(|n| n.eq_ignore_ascii_case(&assignment.target))
        {
            outputs.push(assignment.target.clone());
        }
    }
    let green = CreateSolidBrush(COLORREF(0x00B0_FFB0));
    let red = CreateSolidBrush(COLORREF(0x00C2_C2FF));
    let mut input_points: Vec<(String, i32, i32)> = Vec::new();
    for (index, name) in inputs.iter().enumerate() {
        let y = area.top + scale(6, dpi) + index as i32 * row_height;
        if y + box_height > area.bottom {
            break;
        }
        let rect = RECT {
            left: input_x,
            top: y,
            right: input_x + box_width,
            bottom: y + box_height,
        };
        let _ = FillRect(hdc, &rect, green);
        let mut text: Vec<u16> = if name.is_empty() {
            "0".encode_utf16().collect()
        } else {
            name.encode_utf16().collect()
        };
        let mut text_rect = rect;
        DrawTextW(
            hdc,
            &mut text,
            &mut text_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        input_points.push((
            name.to_uppercase(),
            (rect.left + rect.right) / 2,
            (rect.top + rect.bottom) / 2,
        ));
    }
    for (index, name) in outputs.iter().enumerate() {
        let y = area.top + scale(6, dpi) + index as i32 * row_height;
        if y + box_height > area.bottom {
            break;
        }
        let rect = RECT {
            left: output_x,
            top: y,
            right: output_x + box_width,
            bottom: y + box_height,
        };
        let _ = FillRect(hdc, &rect, red);
        let mut text: Vec<u16> = name.encode_utf16().collect();
        let mut text_rect = rect;
        DrawTextW(
            hdc,
            &mut text,
            &mut text_rect,
            DT_CENTER | DT_VCENTER | DT_SINGLELINE,
        );
        let output_center = ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
        for assignment in state
            .working
            .iter()
            .filter(|a| a.target.eq_ignore_ascii_case(name))
        {
            for summand in &assignment.summands {
                let key = summand.channel.trim().to_uppercase();
                let Some((_, input_x, input_y)) =
                    input_points.iter().find(|(name, _, _)| *name == key)
                else {
                    continue;
                };
                let color = if summand.is_decibel {
                    COLORREF(0x00FF_78A0)
                } else {
                    COLORREF(0x0082_8282)
                };
                let pen = CreatePen(PS_SOLID, 1, color);
                let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
                let _ = MoveToEx(hdc, *input_x, *input_y, None);
                let _ = LineTo(hdc, output_center.0, output_center.1);
                SelectObject(hdc, old_pen);
                let _ = DeleteObject(HGDIOBJ(pen.0));
                if summand.factor != 1.0 || summand.is_decibel || summand.channel.trim().is_empty()
                {
                    let label = if summand.channel.trim().is_empty() {
                        qt_num(summand.factor)
                    } else if summand.is_decibel {
                        format!("{} dB", qt_num(summand.factor))
                    } else {
                        format!("x{}", qt_num(summand.factor))
                    };
                    let mut text: Vec<u16> = label.encode_utf16().collect();
                    let x = (*input_x + output_center.0) / 2;
                    let y = (*input_y + output_center.1) / 2;
                    let _ = SetTextColor(hdc, color);
                    let mut rect = RECT {
                        left: x - scale(40, dpi),
                        top: y - scale(8, dpi),
                        right: x + scale(40, dpi),
                        bottom: y + scale(8, dpi),
                    };
                    DrawTextW(
                        hdc,
                        &mut text,
                        &mut rect,
                        DT_CENTER | DT_VCENTER | DT_SINGLELINE,
                    );
                    let _ = SetTextColor(hdc, COLORREF(0));
                }
            }
        }
    }
    SelectObject(hdc, previous);
    let _ = DeleteObject(HGDIOBJ(green.0));
    let _ = DeleteObject(HGDIOBJ(red.0));
}

/// Re-enables the owner before the modal dialog disappears so Windows hands
/// activation back to the editor instead of leaving it in the background.
unsafe fn reenable_owner(hwnd: HWND) {
    if let Ok(parent) = GetParent(hwnd) {
        let _ = EnableWindow(parent, true);
    }
}

unsafe extern "system" fn copy_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            let state = Box::from_raw(create.lpCreateParams as *mut CopyState);
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as usize;
            let code = ((wparam.0 >> 16) & 0xFFFF) as u32;
            match id {
                ID_OK => {
                    if let Some(state) = state_of(hwnd) {
                        if state.tab == 1 {
                            collect(state);
                        }
                        *state.result.borrow_mut() = Some(state.working.clone());
                    }
                    reenable_owner(hwnd);
                    let _ = DestroyWindow(hwnd);
                }
                ID_CANCEL => {
                    reenable_owner(hwnd);
                    let _ = DestroyWindow(hwnd);
                }
                ID_TAB_GRAPH | ID_TAB_ASSIGN => {
                    if let Some(state) = state_of(hwnd) {
                        state.tab = if id == ID_TAB_GRAPH { 0 } else { 1 };
                        state.scroll = 0;
                        rebuild(hwnd);
                    }
                }
                ID_ADD_ASSIGNMENT => {
                    if let Some(state) = state_of(hwnd) {
                        collect(state);
                        state.working.push(Assignment {
                            target: String::new(),
                            summands: vec![Summand {
                                channel: String::new(),
                                factor: 1.0,
                                is_decibel: false,
                            }],
                        });
                        rebuild(hwnd);
                    }
                }
                _ => {
                    if code == 1 {
                        if let Some(state) = state_of(hwnd) {
                            update_modes(state);
                        }
                    } else if code == 0 && id >= ID_ROW_BASE && id < ID_MODE_BASE {
                        if let Some(state) = state_of(hwnd) {
                            let index = id - ID_ROW_BASE;
                            let row = index / 2;
                            let remove = index % 2 == 1;
                            collect(state);
                            if let Some(control) = state.rows.get(row) {
                                let assignment = control.assignment;
                                let summand = control.summand;
                                if remove {
                                    if state.working[assignment].summands.len() <= 1 {
                                        state.working.remove(assignment);
                                    } else {
                                        state.working[assignment].summands.remove(summand);
                                    }
                                } else {
                                    state.working[assignment].summands.push(Summand {
                                        channel: String::new(),
                                        factor: 1.0,
                                        is_decibel: false,
                                    });
                                }
                                rebuild(hwnd);
                            }
                        }
                    }
                }
            }
            LRESULT(0)
        }
        WM_VSCROLL => {
            if let Some(state) = state_of(hwnd) {
                let code = (wparam.0 & 0xFFFF) as u32;
                let row_height = scale(28, state.dpi);
                let delta = match code {
                    x if x == SB_LINEUP.0 as u32 => -row_height,
                    x if x == SB_LINEDOWN.0 as u32 => row_height,
                    x if x == SB_PAGEUP.0 as u32 => -state.visible_height,
                    x if x == SB_PAGEDOWN.0 as u32 => state.visible_height,
                    x if x == SB_THUMBTRACK.0 as u32 || x == SB_THUMBPOSITION.0 as u32 => {
                        let target = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                        target - state.scroll
                    }
                    x if x == SB_TOP.0 as u32 => -state.scroll,
                    x if x == SB_BOTTOM.0 as u32 => state.content_height,
                    _ => 0,
                };
                scroll_by(hwnd, delta);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            if let Some(state) = state_of(hwnd) {
                let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
                scroll_by(hwnd, -delta * scale(28, state.dpi) * 3 / 120);
            }
            LRESULT(0)
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut paint);
            if let Some(state) = state_of(hwnd) {
                if state.tab == 0 {
                    paint_graph(hwnd, hdc, state);
                }
            }
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_PRINTCLIENT => {
            if let Some(state) = state_of(hwnd) {
                if state.tab == 0 {
                    paint_graph(hwnd, HDC(wparam.0 as *mut core::ffi::c_void), state);
                }
            }
            LRESULT(0)
        }
        WM_SIZE => {
            if let Some(state) = state_of(hwnd) {
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                state.content_bottom = client.bottom - scale(56, state.dpi);
                layout_rows(hwnd, state);
                let _ = InvalidateRect(Some(hwnd), None, true);
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            reenable_owner(hwnd);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CopyState;
            if let Some(state) = pointer.as_mut() {
                for font in [state.font, state.small] {
                    if !font.is_invalid() {
                        let _ = DeleteObject(HGDIOBJ(font.0));
                    }
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

/// Shows the modal copy editor. Returns the edited assignments or `None` when
/// the user cancels.
pub unsafe fn edit_copy(
    parent: HWND,
    assignments: &[Assignment],
    channel_names: &[String],
) -> Option<Vec<Assignment>> {
    COPY_CLASS.get_or_init(|| {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let Ok(cursor) = LoadCursorW(None, IDC_ARROW) else {
            return;
        };
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(copy_proc),
            hInstance: HINSTANCE(module.0),
            hCursor: cursor,
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: w!("EapoCopyDialog"),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
    });
    let dpi = GetDpiForWindow(parent).max(96);
    let width = scale(640, dpi);
    let height = scale(480, dpi);
    let mut parent_rect = RECT::default();
    let _ = GetWindowRect(parent, &mut parent_rect);
    let x = parent_rect.left + ((parent_rect.right - parent_rect.left) - width) / 2;
    let y = parent_rect.top + ((parent_rect.bottom - parent_rect.top) - height) / 2;
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
    let small = CreateFontW(
        -(8 * dpi as i32) / 72,
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
    let result = Rc::new(RefCell::new(None));
    let state = Box::new(CopyState {
        working: assignments.to_vec(),
        channel_names: channel_names.to_vec(),
        tab: 1,
        scroll: 0,
        rows: Vec::new(),
        tab_graph: HWND::default(),
        tab_assign: HWND::default(),
        add_assignment: HWND::default(),
        ok: HWND::default(),
        cancel: HWND::default(),
        font,
        small,
        dpi,
        content_top: scale(48, dpi),
        content_bottom: height - scale(56, dpi),
        content_height: 0,
        visible_height: height - scale(104, dpi),
        result: result.clone(),
    });
    let title = wide_z(&i18n::tr("Copy"));
    let dialog = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("EapoCopyDialog"),
        PCWSTR(title.as_ptr()),
        WINDOW_STYLE(
            WS_POPUP.0
                | WS_CAPTION.0
                | WS_SYSMENU.0
                | WS_CLIPCHILDREN.0
                | WS_VSCROLL.0
                | WS_VISIBLE.0,
        ),
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
    rebuild(dialog);
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
