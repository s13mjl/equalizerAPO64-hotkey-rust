//! Native Win32 memory PoC for the Equalizer APO Editor rewrite.
//!
//! Deliberately uses no egui/OpenGL/accessibility stack: a single window,
//! cached GDI fonts, system font linking and a virtualized 10k-row table.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clipboard;
mod calibration_dialog;
mod chooser;
mod copy_dialog;

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::mem::size_of;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use eapo_apo_info::{AbstractAPOInfo, DeviceAPOInfo};
use eapo_common::channel_helper;
use eapo_editor_core::analysis::{AnalysisRequest, AnalysisResult, AnalysisThread, DeviceSnapshot};
use eapo_editor_core::calibration;
use eapo_editor_core::config_file::{load_config_lines, save_config_file};
use eapo_editor_core::i18n;
use eapo_editor_core::file_dialog::FileDialog;
use eapo_editor_core::hotkey::HotkeyManager;
use eapo_editor_core::keyseq::{KeyCode, KeySequence, Modifiers};
use eapo_editor_core::plot_math::PlotState;
use eapo_editor_core::vst_editor;
use eapo_editor_core::model::{
    device_matches, parse_line, BiquadKind, FilterContext, FilterEditor, FilterItem, FilterKind,
    FilterNode, GraphicEqEditor, QMode,
};
use eapo_editor_core::settings::{self, GlobalPreferences};
use eapo_filters::loudness_correction::volume_controller::VolumeController;
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{COLORREF, HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM};
use windows::Win32::Graphics::GdiPlus::{
    GdipAddPathLine2, GdipClosePathFigure, GdipCreateFromHDC, GdipCreatePath, GdipCreatePen1,
    GdipCreateSolidFill, GdipDeleteBrush, GdipDeleteGraphics, GdipDeletePath, GdipDeletePen,
    GdipDrawPath, GdipFillPath, GdipSetClipRect, GdipSetPixelOffsetMode, GdipSetSmoothingMode,
    GdiplusStartup, GdiplusStartupInput, CombineModeReplace, FillModeAlternate,
    PixelOffsetModeHighQuality, PointF, SmoothingModeAntiAlias, UnitPixel, GpBrush, GpGraphics,
    GpPath, GpPen, GpSolidFill,
};
use windows::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, ClientToScreen, CreateCompatibleBitmap, CreateCompatibleDC, CreateFontW,
    CreatePen, DeleteDC, DeleteObject, EndPaint, FillRect, GetClipBox, GetStockObject, HBRUSH, HDC,
    HFONT, HGDIOBJ, IntersectClipRect, InvalidateRect, MonitorFromRect, RestoreDC, SaveDC,
    SelectObject,
    SetBkMode, SetDCBrushColor, SetTextColor, SetViewportOrgEx, ScreenToClient, TextOutW,
    UpdateWindow, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DC_BRUSH, DEFAULT_CHARSET, DEFAULT_GUI_FONT, DEFAULT_PITCH,
    MONITOR_DEFAULTTONULL,
    OUT_DEFAULT_PRECIS, PAINTSTRUCT, PS_SOLID, Polyline, SRCCOPY, TRANSPARENT,
};
use windows::Win32::System::LibraryLoader::GetModuleHandleW;
use windows::Win32::UI::Controls::{SetScrollInfo, WM_MOUSELEAVE};
use windows::Win32::UI::HiDpi::{
    GetDpiForWindow, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    EnableWindow, GetKeyState, ReleaseCapture, SetCapture, SetFocus, TrackMouseEvent, TME_LEAVE,
    TRACKMOUSEEVENT, VK_A, VK_CONTROL, VK_C, VK_DELETE, VK_DOWN, VK_END, VK_ESCAPE, VK_F2, VK_HOME,
    VK_LWIN, VK_MENU, VK_N, VK_NEXT, VK_O, VK_PRIOR, VK_RETURN, VK_RWIN, VK_S, VK_SHIFT, VK_SPACE,
    VK_UP, VK_V, VK_W, VK_X,
};
use windows::Win32::UI::WindowsAndMessaging::{
    AppendMenuW, CallWindowProcW, CheckMenuItem, CreateMenu, CreatePopupMenu, CreateWindowExW,
    CREATESTRUCTW, DefWindowProcW, DestroyMenu, DestroyWindow, DispatchMessageW, GetClientRect,
    GetMenu, GetMessageW, GetParent, GetSubMenu, GetWindowLongPtrW, GetWindowPlacement,
    GetWindowRect, GetWindowTextW, IsDialogMessageW, IsIconic, IsWindow, KillTimer, LoadCursorW,
    MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW, SetTimer,
    SetWindowLongPtrW, SetWindowPos, SetWindowTextW, SetForegroundWindow, ShowWindow,
    TrackPopupMenuEx, TranslateMessage, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, ES_AUTOHSCROLL,
    GWLP_USERDATA, GWLP_WNDPROC, HMENU, IDCANCEL, IDC_ARROW, IDYES,
    MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_YESNO, MB_YESNOCANCEL,
    MF_BYCOMMAND, MF_CHECKED,
    MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MSG, SB_BOTTOM, SB_LINEDOWN,
    SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT,
    SCROLLINFO, SC_KEYMENU, SIF_PAGE, SIF_POS, SIF_RANGE, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE,
    SW_SHOW, SW_SHOWNORMAL,
    TPM_RETURNCMD, WM_APP, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_DPICHANGED,
    WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_PRINTCLIENT, WM_RBUTTONDOWN, WM_SETFONT,
    WM_SIZE, WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSCOMMAND,
    WM_TIMER, WM_VSCROLL, WM_RBUTTONUP, WNDCLASSW, WNDPROC, WINDOWPLACEMENT, WINDOW_EX_STYLE,
    WINDOW_STYLE, WS_BORDER, WS_CLIPCHILDREN, WS_TABSTOP,
    WS_CAPTION, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_SYSMENU, WS_VISIBLE, WS_VSCROLL,
    SB_ENDSCROLL,
};

const ROWS: usize = 10_000;
const ID_EXIT: usize = 199;
const ID_SAVE: usize = 103;
const ID_DELETE: usize = 114;
const TIMER_SCROLL_TEST: usize = 1;
const TIMER_ANALYSIS: usize = 2;
const WM_APP_COMMIT_EDIT: u32 = WM_APP + 1;
const WM_APP_CANCEL_EDIT: u32 = WM_APP + 2;
const WM_APP_PROMPT_ACCEPT: u32 = WM_APP + 3;
const WM_APP_HOTKEY_ACCEPT: u32 = WM_APP + 4;
const WM_APP_COMMIT_VALUE: u32 = WM_APP + 5;
const WM_APP_CANCEL_VALUE: u32 = WM_APP + 6;

const MENU_TOGGLE: usize = 300;
const MENU_EDIT: usize = 301;
const MENU_REMOVE: usize = 302;
const MENU_SET_HOTKEY: usize = 310;
const MENU_CLEAR_HOTKEY: usize = 311;
const MENU_OPEN_VST: usize = 320;
const MENU_VST_EMBED: usize = 330;
const MENU_VST_AUTO_APPLY: usize = 331;

const ANALYSIS_RESOLUTIONS: [i32; 16] = [
    128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072, 262144, 524288, 1048576,
    2097152, 4194304,
];

const ADD_GROUPS: &[(&str, &[(&str, &str)])] = &[    (
        "Control",
        &[
            ("Include...", "Include:"),
            ("Device...", "Device: all"),
            ("Channel...", "Channel: all"),
            ("Stage...", "Stage: post-mix"),
        ],
    ),
    (
        "Basic filters",
        &[
            ("Preamp", "Preamp: 0 dB"),
            ("Delay", "Delay: 0 ms"),
            ("Copy", "Copy: "),
        ],
    ),
    (
        "Parametric filters",
        &[
            ("Peaking filter", "Filter: ON PK Fc 100 Hz Gain 0 dB Q 10"),
            ("Low-pass filter", "Filter: ON LP Fc 100 Hz"),
            ("High-pass filter", "Filter: ON HP Fc 100 Hz"),
            ("Band-pass filter", "Filter: ON BP Fc 100 Hz Q 10"),
            ("Low-shelf filter", "Filter: ON LS Fc 100 Hz Gain 0 dB"),
            ("High-shelf filter", "Filter: ON HS Fc 100 Hz Gain 0 dB"),
            ("Notch filter", "Filter: ON NO Fc 100 Hz"),
            ("All-pass filter", "Filter: ON AP Fc 100 Hz Q 10"),
        ],
    ),
    (
        "Graphic equalizers",
        &[
            (
                "15-band graphic equalizer",
                "GraphicEQ: 25 0; 40 0; 63 0; 100 0; 160 0; 250 0; 400 0; 630 0; 1000 0; 1600 0; 2500 0; 4000 0; 6300 0; 10000 0; 16000 0",
            ),
            (
                "31-band graphic equalizer",
                "GraphicEQ: 20 0; 25 0; 31.5 0; 40 0; 50 0; 63 0; 80 0; 100 0; 125 0; 160 0; 200 0; 250 0; 315 0; 400 0; 500 0; 630 0; 800 0; 1000 0; 1250 0; 1600 0; 2000 0; 2500 0; 3150 0; 4000 0; 5000 0; 6300 0; 8000 0; 10000 0; 12500 0; 16000 0; 20000 0",
            ),
            ("Graphic equalizer with variable bands", "GraphicEQ: "),
        ],
    ),
    (
        "Advanced filters",
        &[
            (
                "Convolution...",
                "Convolution:",
            ),
            (
                "Loudness correction",
                "LoudnessCorrection: State 1 ReferenceLevel 0 ReferenceOffset 0 Attenuation 1.0",
            ),
        ],
    ),
    ("Plugins", &[("VST plugin...", "VSTPlugin:")]),
    ("Other", &[("Comment", "# ")]),
];

static SCROLL_TEST: AtomicBool = AtomicBool::new(false);
static SUPPRESS_KEYMENU: AtomicBool = AtomicBool::new(false);
static SKIP_CHECKS: AtomicBool = AtomicBool::new(false);
static CONFIG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

const WHITE: COLORREF = COLORREF(0x00FF_FFFF);
const TABLE_BG: COLORREF = COLORREF(0x00FA_FAFA);
const CHROME_BG: COLORREF = COLORREF(0x00EE_EEEE);
const BORDER: COLORREF = COLORREF(0x00C8_C8C8);
const TEXT: COLORREF = COLORREF(0x0020_2020);
const MUTED: COLORREF = COLORREF(0x0070_7070);
const SELECTED: COLORREF = COLORREF(0x00FF_E8_D8);
const EXPANDED_BG: COLORREF = COLORREF(0x00F2_F2F2);
const BAR: COLORREF = COLORREF(0x00C0_8040);
const PLOT_BG: COLORREF = COLORREF(0x00FF_FFFF);
const GRID: COLORREF = COLORREF(0x00E0_E0E0);
const CURVE: COLORREF = COLORREF(0x0080_3030);
const CURVE_POS: COLORREF = COLORREF(0x0020_20C0);
const FILL_POS: COLORREF = COLORREF(0x00E8_E8FF);
const WARNING: COLORREF = COLORREF(0x0000_80FF);
/// Background of a clickable control under the mouse cursor.
const HOVER: COLORREF = COLORREF(0x00F5_E9_D8);

fn wide_z(text: &str) -> Vec<u16> {
    let mut units: Vec<u16> = text.encode_utf16().collect();
    units.push(0);
    units
}

fn wide(text: &str) -> Vec<u16> {
    text.encode_utf16().collect()
}

/// Translates and widens a UI string for Win32 APIs.
fn trw(text: &str) -> Vec<u16> {
    wide(&i18n::tr(text))
}

/// Returns a menu label that stays valid as long as `store`.
///
/// `AppendMenuW` keeps the string pointer, so translated labels must not be
/// temporaries: the buffer is stored until the menu has been destroyed.
fn menu_raw(store: &mut Vec<Vec<u16>>, text: &str) -> PCWSTR {
    store.push(wide_z(text));
    PCWSTR(store.last().expect("just pushed").as_ptr())
}

fn menu_text(store: &mut Vec<Vec<u16>>, text: &str) -> PCWSTR {
    let translated = i18n::tr(text);
    menu_raw(store, &translated)
}

fn scale(value: i32, dpi: u32) -> i32 {
    (value * dpi as i32 + 48) / 96
}

/// Rough width of a UI string at 96 DPI; CJK glyphs are about twice as wide
/// as Latin ones. Used to size buttons from their translated labels.
fn text_width_estimate(text: &str) -> i32 {
    text.chars()
        .map(|ch| if (ch as u32) >= 0x2E80 { 13 } else { 7 })
        .sum()
}

/// Truncates a translated label with an ellipsis if it would not fit the
/// space before the next row control.
fn fit_text(text: &str, max_px: i32) -> String {
    if text_width_estimate(text) <= max_px {
        return text.to_string();
    }
    let mut used = 0;
    let mut out = String::new();
    for ch in text.chars() {
        let width = if (ch as u32) >= 0x2E80 { 13 } else { 7 };
        if used + width > max_px.saturating_sub(21) {
            out.push_str("...");
            break;
        }
        used += width;
        out.push(ch);
    }
    out
}

/// Starts GDI+ once for anti-aliased plot curves.
fn ensure_gdiplus() {
    static TOKEN: OnceLock<usize> = OnceLock::new();
    TOKEN.get_or_init(|| unsafe {
        let input = GdiplusStartupInput {
            GdiplusVersion: 1,
            ..Default::default()
        };
        let mut token = 0usize;
        let _ = GdiplusStartup(&mut token, &input, std::ptr::null_mut());
        token
    });
}

unsafe fn fill(hdc: HDC, left: i32, top: i32, right: i32, bottom: i32, color: COLORREF) {
    if right <= left || bottom <= top {
        return;
    }
    let rect = RECT { left, top, right, bottom };
    let brush = HBRUSH(GetStockObject(DC_BRUSH).0);
    SetDCBrushColor(hdc, color);
    let _ = FillRect(hdc, &rect, brush);
}

unsafe fn draw_text(hdc: HDC, x: i32, y: i32, text: &[u16], color: COLORREF) {
    if text.is_empty() {
        return;
    }
    SetTextColor(hdc, color);
    let _ = TextOutW(hdc, x, y, text);
}

fn argb_color(color: COLORREF) -> u32 {
    let value = color.0;
    0xFF00_0000
        | ((value & 0x0000_00FF) << 16)
        | (value & 0x0000_FF00)
        | ((value >> 16) & 0x0000_00FF)
}

/// GDI+ draws in device coordinates and ignores the GDI viewport origin used
/// by the double buffer, so the origin is cleared while GDI+ is active and
/// callers pass points in client coordinates (shifted here by `offset`).
struct AaContext {
    graphics: *mut GpGraphics,
    previous: POINT,
    dx: f32,
    dy: f32,
}

unsafe fn begin_aa(hdc: HDC, offset: (i32, i32), clip: RECT) -> Option<AaContext> {
    ensure_gdiplus();
    let mut previous = POINT::default();
    let _ = SetViewportOrgEx(hdc, 0, 0, Some(&mut previous));
    let mut graphics: *mut GpGraphics = std::ptr::null_mut();
    let _ = GdipCreateFromHDC(hdc, &mut graphics);
    if graphics.is_null() {
        let _ = SetViewportOrgEx(hdc, previous.x, previous.y, None);
        return None;
    }
    let _ = GdipSetSmoothingMode(graphics, SmoothingModeAntiAlias);
    let _ = GdipSetPixelOffsetMode(graphics, PixelOffsetModeHighQuality);
    // Intersect with the DC clip so curves in partially scrolled rows stay
    // below the header; an empty intersection still gets a clip so nothing
    // is drawn.
    let mut dc_clip = RECT::default();
    let _ = GetClipBox(hdc, &mut dc_clip);
    let left = (clip.left - offset.0).max(dc_clip.left);
    let top = (clip.top - offset.1).max(dc_clip.top);
    let right = (clip.right - offset.0).min(dc_clip.right);
    let bottom = (clip.bottom - offset.1).min(dc_clip.bottom);
    let _ = GdipSetClipRect(
        graphics,
        left as f32,
        top as f32,
        (right - left).max(0) as f32,
        (bottom - top).max(0) as f32,
        CombineModeReplace,
    );
    Some(AaContext {
        graphics,
        previous,
        dx: offset.0 as f32,
        dy: offset.1 as f32,
    })
}

unsafe fn end_aa(hdc: HDC, context: AaContext) {
    let _ = GdipDeleteGraphics(context.graphics);
    let _ = SetViewportOrgEx(hdc, context.previous.x, context.previous.y, None);
}

unsafe fn draw_aa_runs(hdc: HDC, offset: (i32, i32), runs: &[(COLORREF, Vec<PointF>)], clip: RECT) {
    if runs.is_empty() {
        return;
    }
    if let Some(context) = begin_aa(hdc, offset, clip) {
        for (color, points) in runs {
            if points.len() < 2 {
                continue;
            }
            let shifted: Vec<PointF> = points
                .iter()
                .map(|point| PointF {
                    X: point.X - context.dx,
                    Y: point.Y - context.dy,
                })
                .collect();
            let mut path: *mut GpPath = std::ptr::null_mut();
            let _ = GdipCreatePath(FillModeAlternate, &mut path);
            if path.is_null() {
                continue;
            }
            let _ = GdipAddPathLine2(path, shifted.as_ptr(), shifted.len() as i32);
            let mut pen: *mut GpPen = std::ptr::null_mut();
            let _ = GdipCreatePen1(argb_color(*color), 1.0, UnitPixel, &mut pen);
            if !pen.is_null() {
                let _ = GdipDrawPath(context.graphics, pen, path);
                let _ = GdipDeletePen(pen);
            }
            let _ = GdipDeletePath(path);
        }
        end_aa(hdc, context);
        return;
    }
    // Never lose the curve if GDI+ is unavailable: plain GDI polyline.
    let saved = SaveDC(hdc);
    let _ = IntersectClipRect(hdc, clip.left, clip.top, clip.right, clip.bottom);
    for (color, points) in runs {
        if points.len() < 2 {
            continue;
        }
        let gdi_points: Vec<POINT> = points
            .iter()
            .map(|point| POINT {
                x: point.X as i32,
                y: point.Y as i32,
            })
            .collect();
        let pen = CreatePen(PS_SOLID, 1, *color);
        let old_pen = SelectObject(hdc, HGDIOBJ(pen.0));
        let _ = Polyline(hdc, &gdi_points);
        SelectObject(hdc, old_pen);
        let _ = DeleteObject(HGDIOBJ(pen.0));
    }
    let _ = RestoreDC(hdc, saved);
}

/// Fills each open run down to `baseline` as one closed path so the top edge
/// stays anti-aliased as well.
unsafe fn draw_aa_filled_areas(
    hdc: HDC,
    offset: (i32, i32),
    areas: &[Vec<PointF>],
    baseline: f32,
    clip: RECT,
    color: COLORREF,
) {
    if areas.is_empty() {
        return;
    }
    if let Some(context) = begin_aa(hdc, offset, clip) {
        for points in areas {
            if points.len() < 2 {
                continue;
            }
            let first = points[0];
            let last = *points.last().unwrap();
            let mut shape: Vec<PointF> = points
                .iter()
                .map(|point| PointF {
                    X: point.X - context.dx,
                    Y: point.Y - context.dy,
                })
                .collect();
            shape.push(PointF {
                X: last.X - context.dx,
                Y: baseline - context.dy,
            });
            shape.push(PointF {
                X: first.X - context.dx,
                Y: baseline - context.dy,
            });
            let mut path: *mut GpPath = std::ptr::null_mut();
            let _ = GdipCreatePath(FillModeAlternate, &mut path);
            if path.is_null() {
                continue;
            }
            let _ = GdipAddPathLine2(path, shape.as_ptr(), shape.len() as i32);
            let _ = GdipClosePathFigure(path);
            let mut brush: *mut GpSolidFill = std::ptr::null_mut();
            let _ = GdipCreateSolidFill(argb_color(color), &mut brush);
            if !brush.is_null() {
                let _ = GdipFillPath(context.graphics, brush as *mut GpBrush, path);
                let _ = GdipDeleteBrush(brush as *mut GpBrush);
            }
            let _ = GdipDeletePath(path);
        }
        end_aa(hdc, context);
        return;
    }
    // Fallback: one rectangle per sampled column, clipped to the plot.
    let saved = SaveDC(hdc);
    let _ = IntersectClipRect(hdc, clip.left, clip.top, clip.right, clip.bottom);
    let baseline_y = baseline.round() as i32;
    for points in areas {
        for point in points {
            let x = point.X.floor() as i32;
            let top = (point.Y.round() as i32).min(baseline_y);
            fill(hdc, x, top, x + 1, baseline_y, color);
        }
    }
    let _ = RestoreDC(hdc, saved);
}

fn push_u32(buffer: &mut [u16], len: &mut usize, mut value: u32) {
    let mut digits = [0u8; 10];
    let mut count = 0;
    loop {
        digits[count] = b'0' + (value % 10) as u8;
        count += 1;
        value /= 10;
        if value == 0 {
            break;
        }
    }
    while count > 0 {
        count -= 1;
        if *len < buffer.len() {
            buffer[*len] = digits[count] as u16;
            *len += 1;
        }
    }
}

fn row_is_expanded(index: usize) -> bool {
    index % 20 == 7
}

struct Row {
    uid: u64,
    line: String,
    kind: FilterKind,
    height: u32,
    enabled: bool,
    item: Option<Box<FilterItem>>,
    hotkey: Option<KeySequence>,
    /// First node row shown in the GraphicEQ table.
    geq_table_top: usize,
}

struct EditState {
    row: usize,
    hwnd: HWND,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum ValueField {
    PreampDb,
    BiquadFreq,
    BiquadGain,
    BiquadQ,
    Delay,
    LoudnessRef,
    LoudnessOffset,
    LoudnessAtt,
    GeqFreq(usize),
    GeqGain(usize),
}

struct ValueEdit {
    row: usize,
    hwnd: HWND,
    field: ValueField,
}

struct TabInfo {
    title: String,
    path: Option<PathBuf>,
    lines: Vec<String>,
    offset: i32,
    selected: Option<usize>,
    dirty: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DragField {
    PreampDb,
    BiquadFreq,
    BiquadGain,
    BiquadQ,
    DelayValue,
    LoudnessRef,
    LoudnessOffset,
    LoudnessAtt,
    GeqNode,
}

/// Numeric fields with an inline editor; a plain click on one of them opens
/// the text editor instead of only scrubbing the value.
fn drag_value_field(field: DragField) -> Option<ValueField> {
    match field {
        DragField::BiquadFreq => Some(ValueField::BiquadFreq),
        DragField::BiquadGain => Some(ValueField::BiquadGain),
        DragField::BiquadQ => Some(ValueField::BiquadQ),
        DragField::DelayValue => Some(ValueField::Delay),
        DragField::LoudnessRef => Some(ValueField::LoudnessRef),
        DragField::LoudnessOffset => Some(ValueField::LoudnessOffset),
        DragField::LoudnessAtt => Some(ValueField::LoudnessAtt),
        DragField::PreampDb | DragField::GeqNode => None,
    }
}

#[derive(Clone, Copy)]
enum BiquadMenu {
    Kind,
    QMode,
}

#[derive(Clone, Copy)]
struct DragState {
    row: usize,
    field: DragField,
    start_x: i32,
    start_value: f64,
    node: usize,
}

struct ReorderState {
    start_y: i32,
    active: bool,
}

fn row_height(kind: FilterKind, dpi: u32) -> u32 {
    let px = match kind {
        FilterKind::GraphicEq | FilterKind::Copy => scale(220, dpi),
        FilterKind::Channel | FilterKind::Device => scale(72, dpi),
        FilterKind::Convolution => scale(56, dpi),
        FilterKind::Vst => scale(84, dpi),
        FilterKind::Loudness => scale(40, dpi),
        FilterKind::Include | FilterKind::Stage | FilterKind::Biquad | FilterKind::Delay
        | FilterKind::Preamp => scale(28, dpi),
        FilterKind::Unknown => scale(24, dpi),
    };
    px.max(scale(18, dpi)) as u32
}

fn geq_table_rect(dpi: u32, y: i32, row_height: i32, width: i32) -> RECT {
    let right = (width - scale(16, dpi)).max(scale(340, dpi));
    RECT {
        left: (right - scale(150, dpi)).max(scale(240, dpi)),
        top: y + scale(30, dpi),
        right,
        bottom: y + row_height - scale(10, dpi),
    }
}

fn geq_plot_rect(dpi: u32, y: i32, row_height: i32, width: i32) -> RECT {
    let table = geq_table_rect(dpi, y, row_height, width);
    RECT {
        left: scale(120, dpi),
        top: y + scale(30, dpi),
        right: (table.left - scale(8, dpi)).max(scale(200, dpi)),
        bottom: y + row_height - scale(10, dpi),
    }
}

/// Returns the top of the first node row and the row height.
fn geq_table_metrics(dpi: u32, table: &RECT) -> (i32, i32) {
    (table.top + scale(18, dpi), scale(20, dpi))
}

fn geq_table_visible_rows(dpi: u32, table: &RECT) -> usize {
    let (first, row_height) = geq_table_metrics(dpi, table);
    if row_height <= 0 {
        return 0;
    }
    ((table.bottom - first).max(0) / row_height) as usize
}

fn geq_table_column_split(dpi: u32, table: &RECT) -> i32 {
    table.left + scale(86, dpi)
}

fn geq_table_top_clamped(table_top: usize, nodes: usize, visible: usize) -> usize {
    table_top.min(nodes.saturating_sub(visible))
}

fn geq_table_cell_rect(
    dpi: u32,
    table: &RECT,
    table_top: usize,
    visible: usize,
    nodes: usize,
    index: usize,
    column: usize,
) -> Option<RECT> {
    if index < table_top || index >= table_top.saturating_add(visible) || index >= nodes {
        return None;
    }
    let (first, row_height) = geq_table_metrics(dpi, table);
    let split = geq_table_column_split(dpi, table);
    let top = first + (index - table_top) as i32 * row_height;
    let (left, right) = if column == 0 {
        (table.left, split)
    } else {
        (split, table.right)
    };
    Some(RECT {
        left,
        top,
        right,
        bottom: top + row_height,
    })
}

fn geq_table_cell_at(
    dpi: u32,
    table: &RECT,
    table_top: usize,
    visible: usize,
    nodes: usize,
    x: i32,
    y: i32,
) -> Option<(usize, usize)> {
    if !point_in(table, x, y) {
        return None;
    }
    let (first, row_height) = geq_table_metrics(dpi, table);
    if row_height <= 0 || y < first {
        return None;
    }
    let offset = ((y - first) / row_height) as usize;
    if offset >= visible {
        return None;
    }
    let index = table_top + offset;
    if index >= nodes {
        return None;
    }
    let split = geq_table_column_split(dpi, table);
    Some((index, if x < split { 0 } else { 1 }))
}

fn format_geq_value(value: f64) -> String {
    let rounded = if value.abs() < 0.005 {
        0.0
    } else {
        (value * 100.0).round() / 100.0
    };
    if (rounded - rounded.round()).abs() < 1e-9 {
        format!("{rounded:.0}")
    } else if (rounded * 10.0 - (rounded * 10.0).round()).abs() < 1e-9 {
        format!("{rounded:.1}")
    } else {
        format!("{rounded:.2}")
    }
}

fn geq_button_labels(band_count: i32) -> [String; 6] {
    // Short English captions keep the compact layout; other languages use the
    // original Editor translations (e.g. 15 频段 / 反转响应).
    let caption = |english: &str, source: &str| {
        if i18n::language() == i18n::Language::English {
            english.to_string()
        } else {
            i18n::tr(source)
        }
    };
    let marked = |label: String, active: bool| {
        if active {
            format!("{label} *")
        } else {
            label
        }
    };
    [
        marked(caption("15", "15-band"), band_count == 15),
        marked(caption("31", "31-band"), band_count == 31),
        marked(caption("Var", "variable"), band_count < 0),
        caption("Invert", "Invert response"),
        caption("Normalize", "Normalize response"),
        caption("Reset", "Reset response"),
    ]
}

fn geq_button_rects(dpi: u32, y: i32, band_count: i32) -> ([RECT; 6], [String; 6]) {
    let labels = geq_button_labels(band_count);
    let top = y + scale(4, dpi);
    let bottom = y + scale(24, dpi);
    let mut rects = [RECT::default(); 6];
    let mut left = scale(42, dpi);
    for (index, (rect, label)) in rects.iter_mut().zip(labels.iter()).enumerate() {
        // Widths ignore the active-band marker (with room reserved for it) so
        // the buttons do not move when the band count changes.
        let base = label.strip_suffix(" *").unwrap_or(label);
        let marker = if index < 3 { 14 } else { 0 };
        let width = scale((text_width_estimate(base) + marker + 12).max(44), dpi);
        *rect = RECT {
            left,
            top,
            right: left + width,
            bottom,
        };
        left += width + scale(5, dpi);
    }
    (rects, labels)
}

fn preamp_slider_rect(dpi: u32, y: i32) -> RECT {
    RECT {
        left: scale(120, dpi),
        top: y + scale(8, dpi),
        right: scale(370, dpi),
        bottom: y + scale(21, dpi),
    }
}

fn preamp_value_rect(dpi: u32, y: i32) -> RECT {
    RECT {
        left: scale(478, dpi),
        top: y + scale(4, dpi),
        right: scale(610, dpi),
        bottom: y + scale(24, dpi),
    }
}

fn biquad_field_rects(dpi: u32, y: i32) -> [RECT; 5] {
    let top = y + scale(4, dpi);
    let bottom = y + scale(24, dpi);
    let make = |left: i32, right: i32| RECT {
        left: scale(left, dpi),
        top,
        right: scale(right, dpi),
        bottom,
    };
    [
        make(42, 120),
        make(128, 230),
        make(238, 320),
        make(328, 400),
        make(408, 490),
    ]
}

fn delay_field_rects(dpi: u32, y: i32) -> [RECT; 2] {
    let top = y + scale(4, dpi);
    let bottom = y + scale(24, dpi);
    let make = |left: i32, right: i32| RECT {
        left: scale(left, dpi),
        top,
        right: scale(right, dpi),
        bottom,
    };
    [make(100, 220), make(228, 310)]
}

fn stage_field_rects(dpi: u32, y: i32) -> [RECT; 3] {
    let top = y + scale(4, dpi);
    let bottom = y + scale(24, dpi);
    let make = |left: i32, right: i32| RECT {
        left: scale(left, dpi),
        top,
        right: scale(right, dpi),
        bottom,
    };
    [make(90, 170), make(178, 258), make(266, 346)]
}

fn path_row_rects(dpi: u32, y: i32) -> [RECT; 2] {
    let top = y + scale(4, dpi);
    let bottom = y + scale(24, dpi);
    let make = |left: i32, right: i32| RECT {
        left: scale(left, dpi),
        top,
        right: scale(right, dpi),
        bottom,
    };
    [make(120, 500), make(508, 624)]
}

/// Second VST row line: "Open panel" and "Options" buttons.
fn vst_action_rects(dpi: u32, y: i32) -> [RECT; 2] {
    let top = y + scale(30, dpi);
    let bottom = y + scale(50, dpi);
    let make = |left: i32, right: i32| RECT {
        left: scale(left, dpi),
        top,
        right: scale(right, dpi),
        bottom,
    };
    [make(320, 420), make(428, 520)]
}

fn loudness_field_rects(dpi: u32, y: i32) -> [RECT; 4] {
    let top = y + scale(4, dpi);
    let bottom = y + scale(24, dpi);
    let make = |left: i32, right: i32| RECT {
        left: scale(left, dpi),
        top,
        right: scale(right, dpi),
        bottom,
    };
    [make(80, 190), make(198, 310), make(318, 420), make(428, 520)]
}

fn change_button_rect(dpi: u32, y: i32) -> RECT {
    RECT {
        left: scale(420, dpi),
        top: y + scale(4, dpi),
        right: scale(520, dpi),
        bottom: y + scale(24, dpi),
    }
}

/// Speaker positions present in `mask`, in the order used by the C++ channel
/// dialog: C, L, R, SL, SR, RL, RR, RC, LFE.
fn speaker_positions(mask: u32) -> Vec<(&'static str, &'static str)> {
    const DEFS: &[(u32, &str, &str)] = &[
        (channel_helper::SPEAKER_FRONT_CENTER, "C", "Center"),
        (channel_helper::SPEAKER_FRONT_LEFT, "L", "Left"),
        (channel_helper::SPEAKER_FRONT_RIGHT, "R", "Right"),
        (channel_helper::SPEAKER_SIDE_LEFT, "SL", "Side left"),
        (channel_helper::SPEAKER_SIDE_RIGHT, "SR", "Side right"),
        (channel_helper::SPEAKER_BACK_LEFT, "RL", "Rear left"),
        (channel_helper::SPEAKER_BACK_RIGHT, "RR", "Rear right"),
        (channel_helper::SPEAKER_BACK_CENTER, "RC", "Rear center"),
        (channel_helper::SPEAKER_LOW_FREQUENCY, "LFE", "Subwoofer"),
    ];
    DEFS.iter()
        .filter(|(bit, _, _)| mask & bit != 0)
        .map(|(_, name, label)| (*name, *label))
        .collect()
}

fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut result: String = text.chars().take(max.saturating_sub(3)).collect();
        result.push_str("...");
        result
    }
}

fn point_in(rect: &RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

thread_local! {
    /// Cursor position (client coordinates) used by free paint helpers to
    /// highlight the control under the mouse.
    static HOVER_POS: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
}

fn set_hover_pos(pos: Option<(i32, i32)>) {
    HOVER_POS.with(|cell| cell.set(pos));
}

fn hovering(rect: &RECT) -> bool {
    HOVER_POS
        .with(|cell| cell.get())
        .map(|(x, y)| point_in(rect, x, y))
        .unwrap_or(false)
}

fn biquad_kind_label(kind: BiquadKind) -> &'static str {
    match kind {
        BiquadKind::Peaking => "Peak",
        BiquadKind::LowPass => "Low-pass",
        BiquadKind::HighPass => "High-pass",
        BiquadKind::BandPass => "Band-pass",
        BiquadKind::LowShelf => "Low-shelf",
        BiquadKind::HighShelf => "High-shelf",
        BiquadKind::Notch => "Notch",
        BiquadKind::AllPass => "All-pass",
    }
}

fn q_mode_label(mode: QMode) -> &'static str {
    match mode {
        QMode::Fixed => "Fixed",
        QMode::Q => "Q",
        QMode::Bandwidth => "Bandwidth",
        QMode::Slope => "Slope",
    }
}

fn biquad_kind_source(kind: BiquadKind) -> &'static str {
    match kind {
        BiquadKind::Peaking => "Peaking filter",
        BiquadKind::LowPass => "Low-pass filter",
        BiquadKind::HighPass => "High-pass filter",
        BiquadKind::BandPass => "Band-pass filter",
        BiquadKind::LowShelf => "Low-shelf filter",
        BiquadKind::HighShelf => "High-shelf filter",
        BiquadKind::Notch => "Notch filter",
        BiquadKind::AllPass => "All-pass filter",
    }
}

/// Short English caption in the narrow combo; the original Editor wording
/// (and its translations) otherwise.
fn biquad_kind_display(kind: BiquadKind) -> String {
    if i18n::language() == i18n::Language::English {
        biquad_kind_label(kind).to_string()
    } else {
        i18n::tr(biquad_kind_source(kind))
    }
}

unsafe fn draw_frame(hdc: HDC, left: i32, top: i32, right: i32, bottom: i32) {
    fill(hdc, left, top, right, top + 1, BORDER);
    fill(hdc, left, bottom - 1, right, bottom, BORDER);
    fill(hdc, left, top, left + 1, bottom, BORDER);
    fill(hdc, right - 1, top, right, bottom, BORDER);
}

unsafe fn draw_control(
    hdc: HDC,
    dpi: u32,
    rect: &RECT,
    text: &str,
    enabled: bool,
    checked: Option<bool>,
) {
    draw_control_state(hdc, dpi, rect, text, enabled, checked, true);
}

/// Like [`draw_control`] but without hover feedback, for fields that merely
/// display a value.
unsafe fn draw_control_flat(
    hdc: HDC,
    dpi: u32,
    rect: &RECT,
    text: &str,
    enabled: bool,
    checked: Option<bool>,
) {
    draw_control_state(hdc, dpi, rect, text, enabled, checked, false);
}

unsafe fn draw_control_state(
    hdc: HDC,
    dpi: u32,
    rect: &RECT,
    text: &str,
    enabled: bool,
    checked: Option<bool>,
    interactive: bool,
) {
    let background = if enabled && interactive && hovering(rect) {
        HOVER
    } else {
        WHITE
    };
    fill(hdc, rect.left, rect.top, rect.right, rect.bottom, background);
    draw_frame(hdc, rect.left, rect.top, rect.right, rect.bottom);
    let mut x = rect.left + scale(4, dpi);
    if let Some(checked) = checked {
        let size = scale(11, dpi);
        let y = rect.top + (rect.bottom - rect.top - size) / 2;
        fill(hdc, x, y, x + size, y + size, WHITE);
        draw_frame(hdc, x, y, x + size, y + size);
        if checked {
            let inset = scale(3, dpi).max(1);
            fill(hdc, x + inset, y + inset, x + size - inset, y + size - inset, BAR);
        }
        x += size + scale(5, dpi);
    }
    let saved = SaveDC(hdc);
    let _ = IntersectClipRect(hdc, rect.left + 1, rect.top, rect.right - 1, rect.bottom);
    draw_text(
        hdc,
        x,
        rect.top + scale(3, dpi),
        &wide(text),
        if enabled { TEXT } else { MUTED },
    );
    let _ = RestoreDC(hdc, saved);
}

unsafe fn paint_preamp_row(hdc: HDC, dpi: u32, y: i32, db: f64, enabled: bool) {
    let title = fit_text(&i18n::tr("Preamp"), 76);
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide(&title), if enabled { TEXT } else { MUTED });
    let mut track = preamp_slider_rect(dpi, y);
    track.right = scale(470, dpi);
    let track_bg = if hovering(&track) {
        HOVER
    } else {
        COLORREF(0x00E8_E8E8)
    };
    fill(hdc, track.left, track.top, track.right, track.bottom, track_bg);
    draw_frame(hdc, track.left, track.top, track.right, track.bottom);
    let t = ((db.clamp(-20.0, 20.0) + 20.0) / 40.0) as f32;
    let thumb_x = track.left + ((track.right - track.left) as f32 * t) as i32;
    fill(hdc, thumb_x - scale(2, dpi), track.top, thumb_x + scale(2, dpi), track.bottom, BAR);
    draw_control(
        hdc,
        dpi,
        &preamp_value_rect(dpi, y),
        &format!("{db:.1} dB"),
        enabled,
        None,
    );
}

unsafe fn paint_biquad_row(hdc: HDC, dpi: u32, y: i32, editor: &eapo_editor_core::model::BiquadEditor, enabled: bool) {
    let rects = biquad_field_rects(dpi, y);
    draw_control(hdc, dpi, &rects[0], &biquad_kind_display(editor.kind), enabled, None);
    draw_control(hdc, dpi, &rects[1], &format!("Fc {:.0} Hz", editor.freq), enabled, None);
    draw_control(hdc, dpi, &rects[2], &format!("{:.1} dB", editor.gain), enabled, None);
    draw_control(hdc, dpi, &rects[3], &format!("Q {:.2}", editor.q), enabled, None);
    draw_control(hdc, dpi, &rects[4], &i18n::tr(q_mode_label(editor.q_mode)), enabled, None);
}

unsafe fn paint_delay_row(hdc: HDC, dpi: u32, y: i32, delay: f64, is_ms: bool, enabled: bool) {
    let title = fit_text(&i18n::tr("Delay"), 76);
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide(&title), if enabled { TEXT } else { MUTED });
    let rects = delay_field_rects(dpi, y);
    draw_control(hdc, dpi, &rects[0], &format!("{delay:.2}"), enabled, None);
    let unit = if is_ms {
        "ms".to_string()
    } else {
        i18n::tr(" samples").trim().to_string()
    };
    draw_control(hdc, dpi, &rects[1], &unit, enabled, None);
}

unsafe fn paint_stage_row(hdc: HDC, dpi: u32, y: i32, pre_mix: bool, post_mix: bool, capture: bool, enabled: bool) {
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide("Stage"), if enabled { TEXT } else { MUTED });
    let rects = stage_field_rects(dpi, y);
    draw_control(hdc, dpi, &rects[0], &i18n::tr("Pre-mix"), enabled, Some(pre_mix));
    draw_control(hdc, dpi, &rects[1], &i18n::tr("Post-mix"), enabled, Some(post_mix));
    draw_control(hdc, dpi, &rects[2], &i18n::tr("Capture"), enabled, Some(capture));
}

/// Wheel-zoomable view of the GraphicEQ plot: 20 Hz..20 kHz and +20..-20 dB
/// at zoom 1, both axes scrollable once zoomed in. Zoom and scroll are kept
/// in the editor so they survive node edits.
#[derive(Clone, Copy)]
struct GeqView {
    zoom_x: f64,
    zoom_y: f64,
    scroll_x: f32,
    scroll_y: f32,
}

/// Content coordinates are `t_x = log10(f / 20 Hz) / 3` and
/// `t_y = (20 - dB) / 40`, so zoom 1 covers 20 Hz..20 kHz and +20..-20 dB.
/// The reachable world matches the C++ scene (1 Hz..30 kHz, +100..-100 dB)
/// so nodes beyond the default window can still be viewed.
const GEQ_WORLD_MIN_X: f32 = -0.433_676_7;
const GEQ_WORLD_MAX_X: f32 = 1.058_697_1;
const GEQ_WORLD_MIN_Y: f32 = -2.0;
const GEQ_WORLD_MAX_Y: f32 = 3.0;
const GEQ_WORLD_MIN_DB: f64 = -100.0;
const GEQ_WORLD_MAX_DB: f64 = 100.0;

/// Keeps a viewport of `span` inside `min..max`, centering it when it is
/// larger than the world.
fn clamp_axis(value: f32, min: f32, max: f32, span: f32) -> f32 {
    if span >= max - min {
        (min + max - span) / 2.0
    } else {
        value.clamp(min, max - span)
    }
}

impl GeqView {
    fn new(editor: &GraphicEqEditor) -> Self {
        let mut view = Self {
            zoom_x: editor.zoom_x.clamp(0.5, 30.0),
            zoom_y: editor.zoom_y.clamp(0.5, 30.0),
            scroll_x: 0.0,
            scroll_y: 0.0,
        };
        // The model default is the C++ scene-pixel scroll, which is far
        // outside these content units; treat it as "not set".
        let scroll_x = if editor.scroll.0.is_finite()
            && (GEQ_WORLD_MIN_X..=GEQ_WORLD_MAX_X).contains(&editor.scroll.0)
        {
            editor.scroll.0
        } else {
            0.0
        };
        let scroll_y = if editor.scroll.1.is_finite()
            && (GEQ_WORLD_MIN_Y..=GEQ_WORLD_MAX_Y).contains(&editor.scroll.1)
        {
            editor.scroll.1
        } else {
            0.0
        };
        view.set_scroll(scroll_x, scroll_y);
        view
    }

    fn set_scroll(&mut self, x: f32, y: f32) {
        self.scroll_x = clamp_axis(
            x,
            GEQ_WORLD_MIN_X,
            GEQ_WORLD_MAX_X,
            1.0 / self.zoom_x as f32,
        );
        self.scroll_y = clamp_axis(
            y,
            GEQ_WORLD_MIN_Y,
            GEQ_WORLD_MAX_Y,
            1.0 / self.zoom_y as f32,
        );
    }

    fn store(&self, editor: &mut GraphicEqEditor) {
        editor.zoom_x = self.zoom_x;
        editor.zoom_y = self.zoom_y;
        editor.scroll = (self.scroll_x, self.scroll_y);
    }

    fn x_of(&self, freq: f64, plot: &RECT) -> f32 {
        let width = (plot.right - plot.left).max(1) as f64;
        let t = (freq / 20.0).log10() / 3.0;
        (plot.left as f64 + (t - self.scroll_x as f64) * self.zoom_x * width) as f32
    }

    fn y_of(&self, db: f64, plot: &RECT) -> f32 {
        let height = (plot.bottom - plot.top).max(1) as f64;
        let t = (20.0 - db) / 40.0;
        (plot.top as f64 + (t - self.scroll_y as f64) * self.zoom_y * height) as f32
    }

    fn frequency_at(&self, x: f32, plot: &RECT) -> f64 {
        let width = (plot.right - plot.left).max(1) as f32;
        let local = ((x - plot.left as f32) / width).clamp(0.0, 1.0);
        let t = (self.scroll_x + local / self.zoom_x as f32)
            .clamp(GEQ_WORLD_MIN_X, GEQ_WORLD_MAX_X);
        20.0 * 1000.0f64.powf(t as f64)
    }

    fn db_at(&self, y: f32, plot: &RECT) -> f64 {
        let height = (plot.bottom - plot.top).max(1) as f32;
        let local = ((y - plot.top as f32) / height).clamp(0.0, 1.0);
        let t = (self.scroll_y + local / self.zoom_y as f32)
            .clamp(GEQ_WORLD_MIN_Y, GEQ_WORLD_MAX_Y);
        20.0 - t as f64 * 40.0
    }

    fn pan(&mut self, delta_x: f32, delta_y: f32, plot: &RECT) {
        let width = (plot.right - plot.left).max(1) as f32;
        let height = (plot.bottom - plot.top).max(1) as f32;
        let x = self.scroll_x - delta_x / (self.zoom_x as f32 * width);
        let y = self.scroll_y - delta_y / (self.zoom_y as f32 * height);
        self.set_scroll(x, y);
    }

    fn zoom_at(&mut self, delta: f32, local_x: f32, local_y: f32, plot: &RECT) {
        if delta == 0.0 {
            return;
        }
        let width = (plot.right - plot.left).max(1) as f32;
        let height = (plot.bottom - plot.top).max(1) as f32;
        let fx = (local_x / width).clamp(0.0, 1.0);
        let fy = (local_y / height).clamp(0.0, 1.0);
        let anchor_x = self.scroll_x + fx / self.zoom_x as f32;
        let anchor_y = self.scroll_y + fy / self.zoom_y as f32;
        let factor = 1.001f64.powf(delta as f64 * 2.4);
        self.zoom_x = (self.zoom_x * factor).clamp(0.5, 30.0);
        self.zoom_y = (self.zoom_y * factor).clamp(0.5, 30.0);
        self.set_scroll(
            anchor_x - fx / self.zoom_x as f32,
            anchor_y - fy / self.zoom_y as f32,
        );
    }
}

/// Picks a 1/2/5 dB grid step giving roughly 56 px spacing.
fn db_grid_step(range: f64, height: i32) -> f64 {
    if !(range > 0.0) || height <= 0 {
        return 10.0;
    }
    let target = range * 56.0 / height as f64;
    let base = 10f64.powf(target.log10().floor());
    if target >= 5.0 * base {
        5.0 * base
    } else if target >= 2.0 * base {
        2.0 * base
    } else {
        base
    }
}

fn geq_gain_at(nodes: &[FilterNode], freq: f64) -> f64 {
    if nodes.is_empty() {
        return 0.0;
    }
    if freq <= nodes[0].freq {
        return nodes[0].db_gain;
    }
    if freq >= nodes[nodes.len() - 1].freq {
        return nodes[nodes.len() - 1].db_gain;
    }
    for pair in nodes.windows(2) {
        if freq >= pair[0].freq && freq <= pair[1].freq {
            let t = (freq / pair[0].freq).ln() / (pair[1].freq / pair[0].freq).ln().max(1e-9);
            return pair[0].db_gain + t * (pair[1].db_gain - pair[0].db_gain);
        }
    }
    0.0
}

unsafe fn paint_graphic_eq_row(
    hdc: HDC,
    offset: (i32, i32),
    dpi: u32,
    y: i32,
    row_height: i32,
    width: i32,
    editor: &eapo_editor_core::model::GraphicEqEditor,
    table_top: usize,
    enabled: bool,
) {
    let (buttons, labels) = geq_button_rects(dpi, y, editor.band_count);
    for (rect, label) in buttons.iter().zip(labels.iter()) {
        draw_control(hdc, dpi, rect, label, enabled, None);
    }

    let plot = geq_plot_rect(dpi, y, row_height, width);
    fill(hdc, plot.left, plot.top, plot.right, plot.bottom, WHITE);
    draw_frame(hdc, plot.left, plot.top, plot.right, plot.bottom);
    let view = GeqView::new(editor);
    let db_high = view.db_at(plot.top as f32, &plot);
    let db_low = view.db_at((plot.bottom - 1).max(plot.top) as f32, &plot);
    let step = db_grid_step(db_high - db_low, plot.bottom - plot.top);
    let mut db = (db_high / step).floor() * step;
    while db >= db_low - 1e-9 {
        let line_y = view.y_of(db, &plot).round() as i32;
        fill(hdc, plot.left, line_y, plot.right, line_y + 1, GRID);
        db -= step;
    }
    if db_low <= 0.0 && db_high >= 0.0 {
        let zero_line = view.y_of(0.0, &plot).round() as i32;
        fill(hdc, plot.left, zero_line, plot.right, zero_line + 1, BORDER);
    }
    for frequency in [
        1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0,
        20000.0, 30000.0,
    ] {
        let x = view.x_of(frequency, &plot).round() as i32;
        if x >= plot.left && x < plot.right {
            fill(hdc, x, plot.top, x + 1, plot.bottom, GRID);
        }
    }
    let zero_y = view.y_of(0.0, &plot);
    let mut curve: Vec<PointF> = Vec::with_capacity((plot.right - plot.left).max(0) as usize);
    let mut positive_areas: Vec<Vec<PointF>> = Vec::new();
    let mut positive: Vec<PointF> = Vec::new();
    for px in plot.left..plot.right {
        let frequency = view.frequency_at(px as f32 + 0.5, &plot);
        let db =
            geq_gain_at(&editor.nodes, frequency).clamp(GEQ_WORLD_MIN_DB, GEQ_WORLD_MAX_DB);
        let point = PointF {
            X: px as f32 + 0.5,
            Y: view.y_of(db, &plot),
        };
        curve.push(point);
        if db > 0.0 {
            positive.push(point);
        } else if !positive.is_empty() {
            if positive.len() >= 2 {
                positive_areas.push(std::mem::take(&mut positive));
            } else {
                positive.clear();
            }
        }
    }
    if positive.len() >= 2 {
        positive_areas.push(positive);
    }
    draw_aa_filled_areas(hdc, offset, &positive_areas, zero_y, plot, FILL_POS);
    draw_aa_runs(hdc, offset, &[(CURVE, curve)], plot);
    let half = scale(3, dpi);
    for (index, node) in editor.nodes.iter().enumerate() {
        let x = view.x_of(node.freq, &plot).round() as i32;
        let py = view
            .y_of(node.db_gain.clamp(GEQ_WORLD_MIN_DB, GEQ_WORLD_MAX_DB), &plot)
            .round() as i32;
        if x < plot.left - half
            || x > plot.right + half
            || py < plot.top - half
            || py > plot.bottom + half
        {
            continue;
        }
        let color = if editor.selected.contains(&index) { CURVE_POS } else { CURVE };
        fill(hdc, x - half, py - half, x + half, py + half, color);
    }

    // Node table (original Editor: Freq./Gain columns next to the plot).
    let table = geq_table_rect(dpi, y, row_height, width);
    fill(hdc, table.left, table.top, table.right, table.bottom, WHITE);
    draw_frame(hdc, table.left, table.top, table.right, table.bottom);
    let (first_row, row_h) = geq_table_metrics(dpi, &table);
    let split = geq_table_column_split(dpi, &table);
    let visible = geq_table_visible_rows(dpi, &table);
    let top = geq_table_top_clamped(table_top, editor.nodes.len(), visible);
    fill(hdc, table.left + 1, table.top + 1, table.right - 1, first_row - 1, CHROME_BG);
    fill(hdc, split, table.top + 1, split + 1, table.bottom - 1, BORDER);
    fill(hdc, table.left + 1, first_row - 1, table.right - 1, first_row, BORDER);
    draw_text(
        hdc,
        table.left + scale(4, dpi),
        table.top + scale(2, dpi),
        &wide(&i18n::tr("Freq.")),
        TEXT,
    );
    draw_text(
        hdc,
        split + scale(4, dpi),
        table.top + scale(2, dpi),
        &wide(&i18n::tr("Gain")),
        TEXT,
    );
    for (offset, index) in (top..editor.nodes.len()).take(visible).enumerate() {
        let cell_top = first_row + offset as i32 * row_h;
        let node = editor.nodes[index];
        let selected = editor.selected.contains(&index);
        for column in 0..2 {
            let (left, right) = if column == 0 {
                (table.left, split)
            } else {
                (split, table.right)
            };
            let cell = RECT {
                left,
                top: cell_top,
                right,
                bottom: cell_top + row_h,
            };
            let background = if selected {
                SELECTED
            } else if enabled && hovering(&cell) {
                HOVER
            } else {
                WHITE
            };
            fill(hdc, left, cell_top, right, cell_top + row_h, background);
            fill(hdc, left, cell_top + row_h - 1, right, cell_top + row_h, BORDER);
            let text = if column == 0 {
                format_geq_value(node.freq)
            } else {
                format_geq_value(node.db_gain)
            };
            draw_text(
                hdc,
                left + scale(4, dpi),
                cell_top + scale(3, dpi),
                &wide(&text),
                if enabled { TEXT } else { MUTED },
            );
        }
    }
    if visible > 0 && editor.nodes.len() > visible {
        let track_top = first_row;
        let track_bottom = table.bottom - 1;
        let track_height = (track_bottom - track_top).max(1);
        let thumb_height = ((track_height as f32 * visible as f32 / editor.nodes.len() as f32)
            as i32)
            .max(scale(12, dpi))
            .min(track_height);
        let max_top = editor.nodes.len() - visible;
        let thumb_top = track_top
            + ((track_height - thumb_height) as f32 * top as f32 / max_top.max(1) as f32) as i32;
        let bar_x = table.right - scale(5, dpi);
        fill(
            hdc,
            bar_x,
            track_top,
            bar_x + scale(3, dpi),
            track_bottom,
            COLORREF(0x00E8_E8E8),
        );
        fill(
            hdc,
            bar_x,
            thumb_top,
            bar_x + scale(3, dpi),
            thumb_top + thumb_height,
            BAR,
        );
    }
}

unsafe fn paint_path_row(
    hdc: HDC,
    dpi: u32,
    y: i32,
    title: &str,
    path: &str,
    button: &str,
    enabled: bool,
) {
    let title = fit_text(&i18n::tr(title), 76);
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide(&title), if enabled { TEXT } else { MUTED });
    let rects = path_row_rects(dpi, y);
    let shown = if path.is_empty() { "(none)" } else { path };
    draw_control_flat(hdc, dpi, &rects[0], &shorten(shown, 60), enabled, None);
    draw_control(hdc, dpi, &rects[1], &i18n::tr(button), enabled, None);
}

/// Returns the status line for a VST row plus a warning flag for unreadable
/// plugin files (the audio service runs as LOCAL SERVICE).
fn vst_status(vst: &eapo_editor_core::model::VstEditor) -> (String, bool) {
    if vst.path.is_empty() {
        return (i18n::tr("No file selected."), true);
    }
    let path = std::path::Path::new(&vst.path);
    let absolute = if path.is_absolute() {
        path.to_path_buf()
    } else {
        std::path::Path::new(&eapo_filters::vst_plugin_library::VSTPluginLibrary::get_default_plugin_path())
            .join(path)
    };
    let unreadable =
        absolute.exists() && vst_editor::audio_service_readable(&absolute) == Some(false);
    let text = if vst.status.is_empty() {
        "VST plugin editor".to_string()
    } else {
        vst.status.clone()
    };
    (text, unreadable)
}

unsafe fn paint_vst_row(hdc: HDC, dpi: u32, y: i32, vst: &eapo_editor_core::model::VstEditor, enabled: bool) {
    let rects = path_row_rects(dpi, y);
    let shown = if vst.path.is_empty() { "(none)" } else { &vst.path };
    let title = fit_text(&i18n::tr("VST plugin"), 76);
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide(&title), if enabled { TEXT } else { MUTED });
    draw_control(hdc, dpi, &rects[0], &shorten(shown, 60), enabled, None);
    draw_control(hdc, dpi, &rects[1], &i18n::tr("Select file"), enabled, None);
    let actions = vst_action_rects(dpi, y);
    let (status, warning) = vst_status(vst);
    draw_text(
        hdc,
        scale(42, dpi),
        y + scale(35, dpi),
        &wide(&status),
        if warning { WARNING } else if enabled { TEXT } else { MUTED },
    );
    draw_control(hdc, dpi, &actions[0], &i18n::tr("Open panel"), enabled, None);
    draw_control(hdc, dpi, &actions[1], &i18n::tr("Options"), enabled, None);
}

unsafe fn paint_loudness_row(
    hdc: HDC,
    dpi: u32,
    y: i32,
    ref_level: i32,
    ref_offset: i32,
    att: f64,
    enabled: bool,
) {
    // There is no short "Loudness" translation in the original catalog and the
    // row title only has ~38 px before the first field, so keep it short.
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide("Loudness"), if enabled { TEXT } else { MUTED });
    let field = |short: &str, source: &str, value: String| {
        if i18n::language() == i18n::Language::English {
            format!("{short} {value}")
        } else {
            format!("{} {value}", i18n::tr(source))
        }
    };
    let rects = loudness_field_rects(dpi, y);
    draw_control(
        hdc,
        dpi,
        &rects[0],
        &field("Ref", "Reference level:", ref_level.to_string()),
        enabled,
        None,
    );
    draw_control(
        hdc,
        dpi,
        &rects[1],
        &field("Offset", "Reference offset:", ref_offset.to_string()),
        enabled,
        None,
    );
    draw_control(
        hdc,
        dpi,
        &rects[2],
        &field("Att", "Attenuation:", format!("{att:.2}")),
        enabled,
        None,
    );
    draw_control(hdc, dpi, &rects[3], &i18n::tr("Calibrate"), enabled, None);
}

unsafe fn paint_summary_row(
    hdc: HDC,
    dpi: u32,
    y: i32,
    title: &str,
    summary: &str,
    enabled: bool,
) {
    let title = fit_text(&i18n::tr(title), 76);
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide(&title), if enabled { TEXT } else { MUTED });
    draw_text(
        hdc,
        scale(120, dpi),
        y + scale(5, dpi),
        &wide(&shorten(summary, 70)),
        if enabled { TEXT } else { MUTED },
    );
    let button = change_button_rect(dpi, y);
    draw_control(hdc, dpi, &button, &i18n::tr("Change..."), enabled, None);
}

unsafe fn paint_copy_row(hdc: HDC, dpi: u32, y: i32, parameters: &str, enabled: bool) {
    let title = fit_text(&i18n::tr("Copy"), 76);
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide(&title), if enabled { TEXT } else { MUTED });
    let mut line_y = y + scale(28, dpi);
    for part in parameters.split(';').take(3) {
        draw_text(
            hdc,
            scale(48, dpi),
            line_y,
            &wide(&shorten(part.trim(), 80)),
            if enabled { TEXT } else { MUTED },
        );
        line_y += scale(18, dpi);
    }
    let button = change_button_rect(dpi, y);
    draw_control(hdc, dpi, &button, &format!("{}...", i18n::tr("Edit")), enabled, None);
}

fn synthetic_lines() -> Vec<String> {
    (0..ROWS)
        .map(|index| {
            if row_is_expanded(index) {
                "GraphicEQ: 25 0; 40 0; 63 0; 100 0; 160 0; 250 0; 400 0; 630 0; 1000 0"
                    .to_string()
            } else if index % 2 == 0 {
                "Preamp: -6.0 dB".to_string()
            } else {
                "Filter: ON PK Fc 1000 Hz Gain 3 dB Q 1".to_string()
            }
        })
        .collect()
}

fn push_utf16(buffer: &mut [u16], len: &mut usize, text: &str) {
    for unit in text.encode_utf16() {
        if *len < buffer.len() {
            buffer[*len] = unit;
            *len += 1;
        }
    }
}

fn format_row_text(buffer: &mut [u16], row: &Row) -> usize {
    let mut len = 0;
    push_utf16(buffer, &mut len, &row.line);
    len
}

fn format_row_number(buffer: &mut [u16], index: usize) -> usize {
    let mut len = 0;
    push_u32(buffer, &mut len, (index + 1) as u32);
    len
}

struct Labels {
    new: Vec<u16>,
    open: Vec<u16>,
    save: Vec<u16>,
    instant: Vec<u16>,
}

impl Labels {
    fn new() -> Self {
        Self {
            new: trw("New"),
            open: trw("Open"),
            save: trw("Save"),
            instant: trw("Instant mode"),
        }
    }
}

#[derive(Clone, Copy)]
enum AnalysisSetting {
    StartFrom,
    Channel,
    Resolution,
}

struct App {
    dpi: u32,
    body: HFONT,
    bold: HFONT,
    small: HFONT,
    rows: Vec<Row>,
    prefix: Vec<u32>,
    total: u32,
    offset: i32,
    selected: Option<usize>,
    selection: HashSet<usize>,
    selection_anchor: Option<usize>,
    labels: Labels,
    scroll_test_left: usize,
    doc_path: Option<PathBuf>,
    dirty: bool,
    edit: Option<EditState>,
    value_edit: Option<ValueEdit>,
    analysis: AnalysisThread,
    analysis_result: Option<AnalysisResult>,
    analysis_generation: u64,
    show_toolbar: bool,
    show_analysis: bool,
    channel_names: Vec<String>,
    drag: Option<DragState>,
    drag_moved: bool,
    reorder: Option<ReorderState>,
    reorder_drop: Option<usize>,
    next_uid: u64,
    tabs: Vec<TabInfo>,
    active_tab: usize,
    hotkeys: HotkeyManager,
    prefs: GlobalPreferences,
    skip_save_preferences: bool,
    restart_after_close: bool,
    instant_mode: bool,
    recent_files: Vec<String>,
    analysis_resolution: i32,
    analysis_effective_resolution: i32,
    analysis_start_from: i32,
    analysis_channel_index: usize,
    menu_guard: Option<(u8, std::time::Instant)>,
    analysis_hits: Vec<(AnalysisSetting, RECT)>,
    devices: Vec<Arc<dyn AbstractAPOInfo>>,
    selected_device: Option<Arc<dyn AbstractAPOInfo>>,
    channel_mask: u32,
    volume_controller: Option<VolumeController>,
    master_volume: Option<f64>,
    plot: PlotState,
    plot_fitted_for: (i32, i32),
    plot_rect: RECT,
    plot_hover: Option<(i32, i32)>,
    plot_pan: Option<(i32, i32)>,
    geq_pan: Option<(usize, i32, i32)>,
    hover_pos: Option<(i32, i32)>,
    hot_rect: Option<RECT>,
    paint_offset: (i32, i32),
}

impl App {
    unsafe fn new(dpi: u32) -> Self {
        let prefs = GlobalPreferences::load();
        let instant_mode = prefs.instant_mode;
        let show_toolbar = prefs.show_toolbar;
        let recent_files = prefs.recent_files.clone();
        let analysis_resolution = prefs.analysis_resolution.max(128);
        let analysis_start_from = prefs.analysis_start_from;
        let mut devices = DeviceAPOInfo::load_all_infos(false).unwrap_or_default();
        devices.extend(DeviceAPOInfo::load_all_infos(true).unwrap_or_default());
        let selected_device = devices
            .iter()
            .find(|device| device.get_device_string() == prefs.selected_device)
            .cloned()
            .or_else(|| devices.iter().find(|device| device.is_default_device()).cloned())
            .or_else(|| devices.first().cloned());
        let channel_mask = if prefs.selected_channel_mask != 0 {
            prefs.selected_channel_mask as u32
        } else {
            selected_device
                .as_ref()
                .map(|device| device.get_channel_mask())
                .filter(|mask| *mask != 0)
                .unwrap_or(3)
        };
        let channel_names = if let Some(device) = &selected_device {
            channel_helper::get_channel_names(device.get_channel_count() as i32, channel_mask)
        } else {
            channel_helper::get_channel_names(16, channel_mask)
        };
        let analysis_channel_index = channel_names
            .iter()
            .position(|name| name.eq_ignore_ascii_case(prefs.analysis_channel.trim()))
            .unwrap_or(0);
        let mut app = Self {
            dpi: dpi.max(96),
            body: HFONT::default(),
            bold: HFONT::default(),
            small: HFONT::default(),
            rows: Vec::new(),
            prefix: Vec::new(),
            total: 0,
            offset: 0,
            selected: None,
            selection: HashSet::new(),
            selection_anchor: None,
            labels: Labels::new(),
            scroll_test_left: if SCROLL_TEST.load(Ordering::Relaxed) { 1500 } else { 0 },
            doc_path: CONFIG_PATH.get().cloned().flatten(),
            dirty: false,
            edit: None,
            value_edit: None,
            analysis: AnalysisThread::new(),
            analysis_result: None,
            analysis_generation: 0,
            show_toolbar,
            show_analysis: !SCROLL_TEST.load(Ordering::Relaxed) && prefs.show_analysis,
            channel_names,
            drag: None,
            drag_moved: false,
            reorder: None,
            reorder_drop: None,
            next_uid: 1,
            tabs: Vec::new(),
            active_tab: 0,
            hotkeys: HotkeyManager::new(),
            prefs,
            skip_save_preferences: false,
            restart_after_close: false,
            instant_mode,
            recent_files,
            analysis_resolution,
            analysis_effective_resolution: analysis_resolution,
            analysis_start_from,
            analysis_channel_index,
            menu_guard: None,
            analysis_hits: Vec::new(),
            devices,
            selected_device,
            channel_mask,
            volume_controller: None,
            master_volume: None,
            plot: PlotState::default(),
            plot_fitted_for: (0, 0),
            plot_rect: RECT::default(),
            plot_hover: None,
            plot_pan: None,
            geq_pan: None,
            hover_pos: None,
            hot_rect: None,
            paint_offset: (0, 0),
        };
        app.create_fonts();
        let lines = if SCROLL_TEST.load(Ordering::Relaxed) {
            synthetic_lines()
        } else {
            match CONFIG_PATH.get().cloned().flatten() {
                Some(path) => match load_config_lines(&path.to_string_lossy()) {
                    Ok(lines) => lines,
                    Err(error) => vec![format!("(cannot read {}: {error})", path.display())],
                },
                None => Vec::new(),
            }
        };
        app.set_lines(lines);
        let title = app.doc_title();
        app.tabs.push(TabInfo {
            title,
            path: app.doc_path.clone(),
            lines: app.rows.iter().map(|row| row.line.clone()).collect(),
            offset: 0,
            selected: None,
            dirty: false,
        });
        app.reload_hotkeys();
        app.start_analysis();
        app
    }

    fn reload_hotkeys(&mut self) {
        self.hotkeys.clear_all();
        for row in &mut self.rows {
            row.hotkey = None;
        }
        let Some(path) = self.doc_path.clone() else {
            return;
        };
        let hotkeys = settings::load_row_hotkeys(&path.to_string_lossy());
        for (index, sequence) in hotkeys {
            if index == 0 || index > self.rows.len() {
                continue;
            }
            let row = &mut self.rows[index - 1];
            row.hotkey = Some(sequence);
            let filter_id = format!("{:x}", row.uid);
            let _ = self.hotkeys.register_hotkey(&filter_id, Some(sequence));
        }
    }

    fn save_hotkeys(&self) {
        let Some(path) = self.doc_path.clone() else {
            return;
        };
        let updates: Vec<(usize, String, Option<KeySequence>)> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let command = row
                    .line
                    .split(':')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                (index + 1, command, row.hotkey)
            })
            .collect();
        settings::save_row_hotkeys(&path.to_string_lossy(), &updates);
    }

    unsafe fn set_hotkey_for_row(&mut self, hwnd: HWND, row: usize) {
        if row >= self.rows.len() {
            return;
        }
        self.ensure_item(row);
        if self.rows[row]
            .item
            .as_ref()
            .and_then(|item| item.editor.as_ref())
            .is_none()
        {
            let text = wide_z("This row does not have a filter editor.");
            let _ = MessageBoxW(
                Some(hwnd),
                PCWSTR(text.as_ptr()),
                w!("Equalizer APO"),
                MB_ICONWARNING,
            );
            return;
        }
        let Some(sequence) = prompt_hotkey(hwnd, "Set filter hotkey", self.rows[row].hotkey) else {
            return;
        };
        // A held Alt key would otherwise activate the editor menu bar (a
        // visible flash) right after the dialog closes.
        if GetKeyState(VK_MENU.0 as i32) < 0 {
            SUPPRESS_KEYMENU.store(true, Ordering::Relaxed);
        }
        let filter_id = format!("{:x}", self.rows[row].uid);
        self.hotkeys.unregister_hotkey(&filter_id);
        if let Some(sequence) = sequence {
            if !self.hotkeys.register_hotkey(&filter_id, Some(sequence)) {
                let message = wide_z("The hotkey could not be registered. It may already be in use.");
                let _ = MessageBoxW(
                    Some(hwnd),
                    PCWSTR(message.as_ptr()),
                    w!("Equalizer APO"),
                    MB_ICONWARNING,
                );
                self.reload_hotkeys();
                return;
            }
        }
        self.rows[row].hotkey = sequence;
        self.save_hotkeys();
        self.invalidate_row(hwnd, row);
    }

    unsafe fn clear_hotkey_for_row(&mut self, hwnd: HWND, row: usize) {
        if row >= self.rows.len() {
            return;
        }
        let filter_id = format!("{:x}", self.rows[row].uid);
        self.hotkeys.unregister_hotkey(&filter_id);
        self.rows[row].hotkey = None;
        self.save_hotkeys();
        self.invalidate_row(hwnd, row);
    }

    unsafe fn handle_hotkey(&mut self, hwnd: HWND, filter_id: &str) {
        if let Some(index) = self
            .rows
            .iter()
            .position(|row| format!("{:x}", row.uid) == filter_id)
        {
            self.toggle_row(hwnd, index);
            self.start_analysis();
        }
    }

    fn doc_title(&self) -> String {
        self.doc_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unsaved".to_string())
    }

    fn capture_active_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        let index = self.active_tab.min(self.tabs.len() - 1);
        let lines: Vec<String> = self.rows.iter().map(|row| row.line.clone()).collect();
        let title = self.doc_title();
        let tab = &mut self.tabs[index];
        tab.title = title;
        tab.path = self.doc_path.clone();
        tab.lines = lines;
        tab.offset = self.offset;
        tab.selected = self.selected;
        tab.dirty = self.dirty;
    }

    unsafe fn load_tab(&mut self, hwnd: HWND, index: usize) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let lines = tab.lines.clone();
        let path = tab.path.clone();
        let offset = tab.offset;
        let selected = tab.selected;
        let dirty = tab.dirty;
        self.active_tab = index;
        self.doc_path = path;
        self.set_lines(lines);
        self.reload_hotkeys();
        self.offset = offset;
        self.selected = selected;
        self.selection = selected.into_iter().collect();
        self.selection_anchor = selected;
        self.dirty = dirty;
        self.edit = None;
        self.drag = None;
        self.analysis_result = None;
        self.update_scrollbar(hwnd);
        self.update_title(hwnd);
        self.start_analysis();
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    unsafe fn switch_tab(&mut self, hwnd: HWND, index: usize) {
        if index == self.active_tab || index >= self.tabs.len() {
            return;
        }
        self.commit_edit(hwnd, true);
        self.capture_active_tab();
        self.load_tab(hwnd, index);
    }

    unsafe fn new_tab(&mut self, hwnd: HWND) {
        self.commit_edit(hwnd, true);
        self.capture_active_tab();
        let index = self.tabs.len();
        self.tabs.push(TabInfo {
            title: "Unsaved".to_string(),
            path: None,
            lines: vec![String::new()],
            offset: 0,
            selected: None,
            dirty: false,
        });
        self.load_tab(hwnd, index);
    }

    unsafe fn open_paths(&mut self, hwnd: HWND, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        self.commit_edit(hwnd, true);
        self.capture_active_tab();
        let mut last = self.active_tab;
        for path in paths {
            let path_string = path.to_string_lossy().into_owned();
            settings::push_recent(&mut self.recent_files, &path_string);
            let lines = match load_config_lines(&path_string) {
                Ok(lines) => lines,
                Err(error) => vec![format!("(cannot read {}: {error})", path.display())],
            };
            let title = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            self.tabs.push(TabInfo {
                title,
                path: Some(path),
                lines,
                offset: 0,
                selected: None,
                dirty: false,
            });
            last = self.tabs.len() - 1;
        }
        self.load_tab(hwnd, last);
    }

    unsafe fn open_dialog(&mut self, hwnd: HWND) {
        let mut dialog = FileDialog::new()
            .set_title("Open configuration")
            .add_filter("E-APO configurations (*.txt)", &["txt"]);
        if let Some(parent) = self.doc_path.as_ref().and_then(|path| path.parent()) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(paths) = dialog.pick_files() {
            self.open_paths(hwnd, paths);
        }
    }

    unsafe fn save_as(&mut self, hwnd: HWND) -> bool {
        let mut dialog = FileDialog::new()
            .set_title("Save configuration as")
            .add_filter("E-APO configurations (*.txt)", &["txt"]);
        if let Some(path) = &self.doc_path {
            if let Some(parent) = path.parent() {
                dialog = dialog.set_directory(parent);
            }
            if let Some(name) = path.file_name() {
                dialog = dialog.set_file_name(name.to_string_lossy());
            }
        } else {
            dialog = dialog.set_file_name("config.txt");
        }
        let Some(path) = dialog.save_file() else {
            return false;
        };
        let path = if path.extension().is_none() {
            path.with_extension("txt")
        } else {
            path
        };
        self.doc_path = Some(path);
        let saved = self.save(hwnd);
        if saved {
            let path_string = self
                .doc_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            settings::push_recent(&mut self.recent_files, &path_string);
            self.capture_active_tab();
            self.update_title(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        saved
    }

    fn start_analysis(&mut self) {
        if !self.show_analysis {
            return;
        }
        let path = if self.analysis_start_from == 1 {
            self.doc_path.clone()
        } else {
            Some(settings::config_dir_from_registry().join("config.txt"))
        };
        let Some(path) = path else {
            self.analysis_result = None;
            return;
        };
        let mut resolution = self.analysis_resolution.max(128);
        const MEMORY_LIMIT: i32 = 1_048_576;
        while resolution > MEMORY_LIMIT {
            resolution /= 2;
        }
        self.analysis_effective_resolution = resolution;
        self.analysis_generation = self.analysis_generation.wrapping_add(1);
        let (device_name, connection_name, device_guid, device_string, channel_count, sample_rate, device_mask) =
            if let Some(device) = &self.selected_device {
                (
                    device.get_device_name(),
                    device.get_connection_name(),
                    device.get_device_guid(),
                    device.get_device_string(),
                    device.get_channel_count(),
                    device.get_sample_rate(),
                    device.get_channel_mask(),
                )
            } else {
                (
                    "Benchmark".to_string(),
                    "File output".to_string(),
                    String::new(),
                    String::new(),
                    2,
                    48000,
                    3,
                )
            };
        let channel_mask = if self.channel_mask != 0 {
            self.channel_mask
        } else {
            device_mask
        };
        self.analysis.set_parameters(AnalysisRequest {
            device: DeviceSnapshot {
                capture: false,
                device_name,
                connection_name,
                device_guid,
                device_string,
                channel_count,
                sample_rate,
                channel_mask: device_mask,
            },
            channel_mask,
            channel_index: self.analysis_channel_index.min(channel_count.saturating_sub(1) as usize),
            config_path: path.to_string_lossy().into_owned(),
            frame_count: resolution as usize,
            generation: self.analysis_generation,
        });
    }

    fn poll_analysis(&mut self, hwnd: HWND) -> bool {
        if let Some(result) = self.analysis.try_recv() {
            if result.generation == self.analysis_generation {
                self.analysis_result = Some(result);
                unsafe {
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
                return true;
            }
        }
        false
    }

    fn poll_volume(&mut self, hwnd: HWND) {
        if !self.rows.iter().any(|row| row.kind == FilterKind::Loudness) {
            return;
        }
        if self.volume_controller.is_none() {
            self.volume_controller = Some(VolumeController::new());
        }
        let mut volume = 0.0;
        let Some(controller) = &self.volume_controller else {
            return;
        };
        if controller.get_volume(&mut volume).ok().is_err() {
            return;
        }
        if self.master_volume == Some(volume) {
            return;
        }
        self.master_volume = Some(volume);
        let mut repaint = false;
        for row in &mut self.rows {
            if row.kind != FilterKind::Loudness {
                continue;
            }
            if let Some(item) = row.item.as_mut() {
                if let Some(FilterEditor::Loudness {
                    volume: shown, ..
                }) = item.editor.as_mut()
                {
                    if *shown != volume {
                        *shown = volume;
                        repaint = true;
                    }
                }
            }
        }
        if repaint {
            unsafe {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        self.start_analysis();
    }

    unsafe fn create_fonts(&mut self) {
        let dpi = self.dpi as i32;
        let height = |points: i32| -((points * dpi + 36) / 72);
        self.body = CreateFontW(
            height(9),
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
        self.bold = CreateFontW(
            height(9),
            0,
            0,
            0,
            700,
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
        self.small = CreateFontW(
            height(8),
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
    }

    unsafe fn destroy_fonts(&mut self) {
        for font in [self.body, self.bold, self.small] {
            if !font.is_invalid() {
                let _ = DeleteObject(HGDIOBJ(font.0));
            }
        }
        self.body = HFONT::default();
        self.bold = HFONT::default();
        self.small = HFONT::default();
    }

    fn set_lines(&mut self, lines: Vec<String>) {
        self.rows.clear();
        self.rows.reserve(lines.len());
        for line in lines {
            let structural = parse_line(&line);
            let kind = structural.kind;
            let enabled = structural.enabled;
            let height = row_height(kind, self.dpi);
            let uid = self.next_uid;
            self.next_uid += 1;
            self.rows.push(Row {
                uid,
                line,
                kind,
                height,
                enabled,
                item: None,
                hotkey: None,
                geq_table_top: 0,
            });
        }
        self.recompute_prefix();
        self.offset = 0;
    }

    fn recompute_prefix(&mut self) {
        self.prefix.clear();
        self.prefix.reserve(self.rows.len() + 1);
        self.prefix.push(0);
        for row in &self.rows {
            let last = *self.prefix.last().unwrap();
            self.prefix.push(last + row.height);
        }
        self.total = *self.prefix.last().unwrap();
        if let Some(selected) = self.selected {
            self.selected = (selected < self.rows.len()).then_some(selected);
        }
    }

    fn rebuild_rows(&mut self) {
        for row in &mut self.rows {
            row.height = row_height(row.kind, self.dpi);
        }
        self.recompute_prefix();
    }

    fn ensure_item(&mut self, index: usize) {
        if index >= self.rows.len() || self.rows[index].item.is_some() {
            return;
        }
        let names = self.channel_names.clone();
        let sample_rate = self
            .selected_device
            .as_ref()
            .map(|device| device.get_sample_rate())
            .unwrap_or(48000);
        let context = FilterContext {
            config_path: "",
            channel_names: &names,
            channel_mask: self.channel_mask,
            device_sample_rate: sample_rate,
        };
        let uid = self.rows[index].uid;
        let kind = self.rows[index].kind;
        let line = self.rows[index].line.clone();
        let mut item = FilterItem::new(uid, line, &context);
        if kind == FilterKind::Loudness {
            if let Some(volume) = self.master_volume {
                if let Some(FilterEditor::Loudness {
                    volume: shown, ..
                }) = item.editor.as_mut()
                {
                    *shown = volume;
                }
            }
        }
        self.rows[index].item = Some(Box::new(item));
    }

    fn sync_item(&mut self, hwnd: HWND, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        if let Some(item) = self.rows[index].item.as_mut() {
            item.sync_from_editor();
            let line = item.line.clone();
            self.apply_row_line(index, line);
            self.dirty = true;
            unsafe {
                self.update_title(hwnd);
            }
        }
    }

    unsafe fn prune_items(&mut self, hwnd: HWND) {
        if self.rows.len() < 512 {
            return;
        }
        let viewport = self.viewport_height(hwnd);
        let start = (self.offset.max(0) as u32).saturating_sub(200);
        let end = (self.offset.max(0) as u32)
            .saturating_add(viewport.max(0) as u32)
            .saturating_add(400);
        for (index, row) in self.rows.iter_mut().enumerate() {
            let row_start = self.prefix[index];
            let row_end = self.prefix[index + 1];
            let keep = self.edit.as_ref().map(|edit| edit.row) == Some(index);
            if !keep && (row_end < start || row_start > end) {
                row.item = None;
            }
        }
        let _ = hwnd;
    }

    unsafe fn update_title(&self, hwnd: HWND) {
        let name = self
            .doc_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| "Unsaved".to_string());
        let suffix = if self.dirty { " *" } else { "" };
        let title = wide_z(&format!("Equalizer APO Configuration Editor - {name}{suffix}"));
        let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
    }

    unsafe fn save(&mut self, hwnd: HWND) -> bool {
        if self.doc_path.is_none() {
            return self.save_as(hwnd);
        }        let Some(path) = self.doc_path.clone() else {
            return false;
        };
        let lines: Vec<String> = self.rows.iter().map(|row| row.line.clone()).collect();
        match save_config_file(&path.to_string_lossy(), &lines) {
            Ok(()) => {
                self.dirty = false;
                self.capture_active_tab();
                self.update_title(hwnd);
                self.start_analysis();
                true
            }
            Err(error) => {
                let text = wide_z(&format!("Could not save {}: {error}", path.display()));
                let _ = MessageBoxW(
                    Some(hwnd),
                    PCWSTR(text.as_ptr()),
                    w!("Equalizer APO"),
                    MB_ICONWARNING,
                );
                false
            }
        }
    }

    unsafe fn autosave(&mut self, hwnd: HWND) {
        if self.instant_mode && self.doc_path.is_some() {
            self.save(hwnd);
        }
    }

    /// Clears the editor preferences (like the original Settings menu) and
    /// restarts the editor so the defaults take effect.
    unsafe fn reset_preferences(&mut self, hwnd: HWND, file_specific: bool) {
        let message = wide_z(&i18n::tr(
            "Configuration Editor will be restarted to apply the changed settings. Proceed?",
        ));
        let title = wide_z(&i18n::tr("Restart required"));
        let answer = MessageBoxW(
            Some(hwnd),
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_YESNO | MB_ICONQUESTION,
        );
        if answer.0 != IDYES.0 {
            return;
        }
        use eapo_common::registry_helper;
        if file_specific {
            let _ = registry_helper::delete_key_tree(settings::EDITOR_PER_FILE_REGPATH);
        } else {
            let root = settings::EDITOR_REGPATH;
            for name in registry_helper::enum_value_names(root).unwrap_or_default() {
                let _ = registry_helper::delete_value(root, &name);
            }
            for child in registry_helper::enum_sub_keys(root).unwrap_or_default() {
                if child.eq_ignore_ascii_case("file-specific") {
                    continue;
                }
                let _ = registry_helper::delete_key_tree(&format!("{root}\\{child}"));
            }
        }
        self.skip_save_preferences = true;
        self.restart_after_close = true;
        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
    }

    /// Keeps the View menu check marks in sync with the current layout.
    unsafe fn sync_view_menu(&self, hwnd: HWND) {
        let view = GetSubMenu(GetMenu(hwnd), 2);
        if view.0.is_null() {
            return;
        }
        let checked = |on: bool| if on { MF_CHECKED.0 } else { MF_UNCHECKED.0 };
        let _ = CheckMenuItem(view, 123, MF_BYCOMMAND.0 | checked(self.show_toolbar));
        let _ = CheckMenuItem(view, 122, MF_BYCOMMAND.0 | checked(self.show_analysis));
    }

    unsafe fn save_preferences(&mut self, hwnd: HWND) {
        let mut prefs = self.prefs.clone();
        prefs.instant_mode = self.instant_mode;
        prefs.show_toolbar = self.show_toolbar;
        prefs.show_analysis = self.show_analysis;
        prefs.recent_files = self.recent_files.clone();
        prefs.analysis_resolution = self.analysis_resolution;
        prefs.analysis_start_from = self.analysis_start_from;
        prefs.analysis_channel = self
            .channel_names
            .get(self.analysis_channel_index)
            .cloned()
            .unwrap_or_default();
        prefs.selected_device = self
            .selected_device
            .as_ref()
            .map(|device| device.get_device_string())
            .unwrap_or_default();
        prefs.selected_channel_mask = self.channel_mask as i32;
        // Use the restored geometry: when the window is minimized,
        // GetWindowRect reports the off-screen icon position (-32000) and a
        // tiny size, which must never be persisted.
        let mut placement = WINDOWPLACEMENT {
            length: size_of::<WINDOWPLACEMENT>() as u32,
            ..Default::default()
        };
        if GetWindowPlacement(hwnd, &mut placement).is_ok() {
            let rect = placement.rcNormalPosition;
            let width = rect.right - rect.left;
            let height = rect.bottom - rect.top;
            if width >= 320 && height >= 240 && rect.left > -30000 && rect.top > -30000 {
                prefs.window_size = Some((width as f32, height as f32));
                prefs.window_pos = Some((rect.left as f32, rect.top as f32));
            }
        }
        prefs.save();
        self.prefs = prefs;
    }

    fn apply_row_line(&mut self, index: usize, line: String) {
        let structural = parse_line(&line);
        let kind = structural.kind;
        let enabled = structural.enabled;
        let dpi = self.dpi;
        let row = &mut self.rows[index];
        row.line = line;
        row.kind = kind;
        row.enabled = enabled;
        row.height = row_height(kind, dpi);
        self.recompute_prefix();
    }

    unsafe fn toggle_row(&mut self, hwnd: HWND, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        let names = vec!["L".to_string(), "R".to_string()];
        let ctx = FilterContext {
            config_path: "",
            channel_names: &names,
            channel_mask: 3,
            device_sample_rate: 48000,
        };
        let mut item = FilterItem::new(0, self.rows[index].line.clone(), &ctx);
        item.toggle_enabled();
        // Keep the materialized editor in sync: otherwise a later slider drag
        // recomposes the line from a stale `enabled` flag and comments the row
        // out again ("adjusting the slider disables the filter").
        if let Some(existing) = self.rows[index].item.as_mut() {
            existing.line = item.line.clone();
            existing.enabled = item.enabled;
        }
        self.apply_row_line(index, item.line);
        self.dirty = true;
        self.update_title(hwnd);
        self.autosave(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    unsafe fn ensure_visible(&mut self, hwnd: HWND, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        let viewport = self.viewport_height(hwnd);
        let top = self.prefix[index] as i32;
        let bottom = self.prefix[index + 1] as i32;
        let mut offset = self.offset;
        if top < offset {
            offset = top;
        }
        if bottom > offset + viewport {
            offset = bottom - viewport;
        }
        if offset != self.offset {
            self.set_offset(hwnd, offset);
        }
    }

    unsafe fn begin_edit(&mut self, hwnd: HWND, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        self.ensure_visible(hwnd, index);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let chrome = self.chrome_height();
        let top = chrome + self.prefix[index] as i32 - self.offset;
        let height = (self.rows[index].height as i32).max(scale(22, self.dpi));
        let mut point = POINT {
            x: scale(40, self.dpi),
            y: top,
        };
        let _ = ClientToScreen(hwnd, &mut point);
        let width = (client.right - scale(48, self.dpi)).max(scale(80, self.dpi));
        let text = wide_z(&self.rows[index].line);
        let style = WINDOW_STYLE(
            WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32,
        );
        let edit = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            PCWSTR(text.as_ptr()),
            style,
            point.x,
            point.y,
            width,
            height,
            Some(hwnd),
            None,
            None,
            None,
        ) {
            Ok(edit) => edit,
            Err(_) => return,
        };
        SendMessageW(
            edit,
            WM_SETFONT,
            Some(WPARAM(self.body.0 as usize)),
            Some(LPARAM(1)),
        );
        let previous = SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as isize);
        SetWindowLongPtrW(edit, GWLP_USERDATA, previous);
        self.edit = Some(EditState { row: index, hwnd: edit });
        let _ = SetFocus(Some(edit));
    }

    unsafe fn commit_edit(&mut self, hwnd: HWND, apply: bool) {
        let Some(edit) = self.edit.take() else {
            return;
        };
        if apply {
            let mut buffer = [0u16; 4096];
            let length = GetWindowTextW(edit.hwnd, &mut buffer).max(0) as usize;
            let text = String::from_utf16_lossy(&buffer[..length.min(buffer.len())]);
            if edit.row < self.rows.len() && self.rows[edit.row].line != text {
                self.apply_row_line(edit.row, text);
                // A raw text edit is authoritative; force a re-parse so the
                // next slider edit does not overwrite it with stale state.
                self.rows[edit.row].item = None;
                self.dirty = true;
                self.update_title(hwnd);
                self.autosave(hwnd);
            }
        }
        let _ = DestroyWindow(edit.hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    unsafe fn begin_value_edit(&mut self, hwnd: HWND, row: usize, field: ValueField) {
        self.commit_value_edit(hwnd, true);
        if row >= self.rows.len() {
            return;
        }
        self.ensure_item(row);
        let Some(item) = self.rows[row].item.as_ref() else {
            return;
        };
        let Some(editor) = item.editor.as_ref() else {
            return;
        };
        let dpi = self.dpi;
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let top = self.chrome_height() + self.prefix[row] as i32 - self.offset;
        let (rect, text) = match (field, editor) {
            (ValueField::PreampDb, FilterEditor::Preamp { db }) => {
                (preamp_value_rect(dpi, top), format!("{db}"))
            }
            (ValueField::BiquadFreq, FilterEditor::Biquad(biquad)) => {
                (biquad_field_rects(dpi, top)[1], format!("{:.1}", biquad.freq))
            }
            (ValueField::BiquadGain, FilterEditor::Biquad(biquad)) => {
                (biquad_field_rects(dpi, top)[2], format!("{:.1}", biquad.gain))
            }
            (ValueField::BiquadQ, FilterEditor::Biquad(biquad)) => {
                (biquad_field_rects(dpi, top)[3], format!("{:.2}", biquad.q))
            }
            (ValueField::Delay, FilterEditor::Delay { delay, .. }) => {
                (delay_field_rects(dpi, top)[0], format!("{delay:.2}"))
            }
            (ValueField::LoudnessRef, FilterEditor::Loudness { ref_level, .. }) => {
                (loudness_field_rects(dpi, top)[0], format!("{ref_level}"))
            }
            (ValueField::LoudnessOffset, FilterEditor::Loudness { ref_offset, .. }) => {
                (loudness_field_rects(dpi, top)[1], format!("{ref_offset}"))
            }
            (ValueField::LoudnessAtt, FilterEditor::Loudness { att, .. }) => {
                (loudness_field_rects(dpi, top)[2], format!("{att:.2}"))
            }
            (ValueField::GeqFreq(index), FilterEditor::GraphicEq(graphic)) => {
                let table = geq_table_rect(dpi, top, self.rows[row].height as i32, client.right);
                let visible = geq_table_visible_rows(dpi, &table);
                let table_top = geq_table_top_clamped(
                    self.rows[row].geq_table_top,
                    graphic.nodes.len(),
                    visible,
                );
                let Some(node) = graphic.nodes.get(index) else {
                    return;
                };
                let Some(cell) = geq_table_cell_rect(
                    dpi,
                    &table,
                    table_top,
                    visible,
                    graphic.nodes.len(),
                    index,
                    0,
                ) else {
                    return;
                };
                (cell, format!("{}", node.freq))
            }
            (ValueField::GeqGain(index), FilterEditor::GraphicEq(graphic)) => {
                let table = geq_table_rect(dpi, top, self.rows[row].height as i32, client.right);
                let visible = geq_table_visible_rows(dpi, &table);
                let table_top = geq_table_top_clamped(
                    self.rows[row].geq_table_top,
                    graphic.nodes.len(),
                    visible,
                );
                let Some(node) = graphic.nodes.get(index) else {
                    return;
                };
                let Some(cell) = geq_table_cell_rect(
                    dpi,
                    &table,
                    table_top,
                    visible,
                    graphic.nodes.len(),
                    index,
                    1,
                ) else {
                    return;
                };
                (cell, format!("{}", node.db_gain))
            }
            _ => return,
        };
        let text = wide_z(&text);
        let edit = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            PCWSTR(text.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            Some(hwnd),
            None,
            None,
            None,
        ) {
            Ok(edit) => edit,
            Err(_) => return,
        };
        SendMessageW(
            edit,
            WM_SETFONT,
            Some(WPARAM(self.body.0 as usize)),
            Some(LPARAM(1)),
        );
        let previous = SetWindowLongPtrW(edit, GWLP_WNDPROC, value_edit_proc as *const () as isize);
        SetWindowLongPtrW(edit, GWLP_USERDATA, previous);
        SendMessageW(edit, 0x00B1, Some(WPARAM(0)), Some(LPARAM(-1)));
        self.value_edit = Some(ValueEdit {
            row,
            hwnd: edit,
            field,
        });
        let _ = SetFocus(Some(edit));
    }

    unsafe fn commit_value_edit(&mut self, hwnd: HWND, apply: bool) {
        let Some(edit) = self.value_edit.take() else {
            return;
        };
        if apply {
            let mut buffer = [0u16; 64];
            let length = GetWindowTextW(edit.hwnd, &mut buffer).max(0) as usize;
            let text = String::from_utf16_lossy(&buffer[..length.min(buffer.len())]);
            if let Some(value) = eapo_editor_core::model::parse_num(&text) {
                let mut changed = false;
                if let Some(item) = self
                    .rows
                    .get_mut(edit.row)
                    .and_then(|row| row.item.as_mut())
                {
                    if let Some(editor) = item.editor.as_mut() {
                        match (edit.field, editor) {
                            (ValueField::PreampDb, FilterEditor::Preamp { db }) => {
                                let value = value.clamp(-100.0, 100.0);
                                if *db != value {
                                    *db = value;
                                    changed = true;
                                }
                            }
                            (ValueField::BiquadFreq, FilterEditor::Biquad(biquad)) => {
                                let value = value.clamp(1.0, 30000.0);
                                if biquad.freq != value {
                                    biquad.freq = value;
                                    changed = true;
                                }
                            }
                            (ValueField::BiquadGain, FilterEditor::Biquad(biquad)) => {
                                let value = value.clamp(-30.0, 30.0);
                                if biquad.gain != value {
                                    biquad.gain = value;
                                    changed = true;
                                }
                            }
                            (ValueField::BiquadQ, FilterEditor::Biquad(biquad)) => {
                                let value = value.clamp(0.1, 100.0);
                                if biquad.q != value {
                                    biquad.q = value;
                                    changed = true;
                                }
                            }
                            (ValueField::Delay, FilterEditor::Delay { delay, .. }) => {
                                // Samples can legitimately exceed the drag range
                                // (e.g. 4800 samples = 100 ms), so only guard
                                // against negative/huge values here.
                                let value = value.clamp(0.0, 1_000_000.0);
                                if *delay != value {
                                    *delay = value;
                                    changed = true;
                                }
                            }
                            (ValueField::LoudnessRef, FilterEditor::Loudness { ref_level, .. }) => {
                                let value = value.clamp(-999.0, 999.0).round() as i32;
                                if *ref_level != value {
                                    *ref_level = value;
                                    changed = true;
                                }
                            }
                            (
                                ValueField::LoudnessOffset,
                                FilterEditor::Loudness { ref_offset, .. },
                            ) => {
                                let value = value.clamp(-999.0, 999.0).round() as i32;
                                if *ref_offset != value {
                                    *ref_offset = value;
                                    changed = true;
                                }
                            }
                            (ValueField::LoudnessAtt, FilterEditor::Loudness { att, .. }) => {
                                let value = (value.clamp(0.0, 1.0) * 100.0).round() / 100.0;
                                if *att != value {
                                    *att = value;
                                    changed = true;
                                }
                            }
                            (ValueField::GeqFreq(index), FilterEditor::GraphicEq(graphic)) => {
                                if let Some(node) = graphic.nodes.get_mut(index) {
                                    let value = value.clamp(1.0, 30000.0);
                                    if (node.freq - value).abs() > 1e-9 {
                                        let db_gain = node.db_gain;
                                        node.freq = value;
                                        graphic.sort_nodes();
                                        graphic.selected = graphic
                                            .nodes
                                            .iter()
                                            .position(|candidate| {
                                                (candidate.freq - value).abs() < 1e-9
                                                    && (candidate.db_gain - db_gain).abs() < 1e-9
                                            })
                                            .map(|position| vec![position])
                                            .unwrap_or_default();
                                        changed = true;
                                    }
                                }
                            }
                            (ValueField::GeqGain(index), FilterEditor::GraphicEq(graphic)) => {
                                if let Some(node) = graphic.nodes.get_mut(index) {
                                    let value =
                                        value.clamp(GEQ_WORLD_MIN_DB, GEQ_WORLD_MAX_DB);
                                    if (node.db_gain - value).abs() > 1e-9 {
                                        node.db_gain = value;
                                        changed = true;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if changed {
                    self.sync_item(hwnd, edit.row);
                    self.start_analysis();
                    self.autosave(hwnd);
                }
            }
        }
        let _ = DestroyWindow(edit.hwnd);
        let _ = SetFocus(Some(hwnd));
        self.invalidate_row(hwnd, edit.row);
        if matches!(
            edit.field,
            ValueField::GeqFreq(_) | ValueField::GeqGain(_)
        ) {
            self.ensure_geq_table_visible(hwnd, edit.row);
        }
    }

    /// Scrolls the GraphicEQ node table so the selected node stays visible.
    unsafe fn ensure_geq_table_visible(&mut self, hwnd: HWND, row: usize) {
        if row >= self.rows.len() {
            return;
        }
        self.ensure_item(row);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let row_y = self.chrome_height() + self.prefix[row] as i32 - self.offset;
        let table = geq_table_rect(self.dpi, row_y, self.rows[row].height as i32, client.right);
        let visible = geq_table_visible_rows(self.dpi, &table);
        let (nodes, selected) = match self.rows[row].item.as_ref().and_then(|item| item.editor.as_ref()) {
            Some(FilterEditor::GraphicEq(graphic)) => {
                (graphic.nodes.len(), graphic.selected.first().copied())
            }
            _ => return,
        };
        let Some(selected) = selected else {
            return;
        };
        let current = geq_table_top_clamped(self.rows[row].geq_table_top, nodes, visible);
        let mut top = current;
        if selected < current {
            top = selected;
        } else if visible > 0 && selected >= current + visible {
            top = selected + 1 - visible;
        }
        top = geq_table_top_clamped(top, nodes, visible);
        if top != self.rows[row].geq_table_top {
            self.rows[row].geq_table_top = top;
            self.invalidate_row(hwnd, row);
        }
    }

    unsafe fn add_row(&mut self, hwnd: HWND, line: &str, before: Option<usize>) {
        self.commit_edit(hwnd, true);
        let structural = parse_line(line);
        let height = row_height(structural.kind, self.dpi);
        let index = before.unwrap_or(self.rows.len()).min(self.rows.len());
        let uid = self.next_uid;
        self.next_uid += 1;
        self.rows.insert(
            index,
            Row {
                uid,
                line: line.to_string(),
                kind: structural.kind,
                height,
                enabled: structural.enabled,
                item: None,
                hotkey: None,
                geq_table_top: 0,
            },
        );
        self.selected = Some(index);
        self.dirty = true;
        self.recompute_prefix();
        self.ensure_visible(hwnd, index);
        self.update_title(hwnd);
        self.autosave(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    unsafe fn remove_selected(&mut self, hwnd: HWND) {
        let mut indices: Vec<usize> = self.selection.iter().copied().collect();
        if indices.is_empty() {
            if let Some(index) = self.selected {
                indices.push(index);
            }
        }
        if indices.is_empty() {
            return;
        }
        self.commit_edit(hwnd, true);
        indices.sort_unstable();
        indices.dedup();
        let first = indices[0].min(self.rows.len().saturating_sub(1));
        for index in indices.into_iter().rev() {
            if index < self.rows.len() {
                self.rows.remove(index);
            }
        }
        self.selected = if self.rows.is_empty() {
            None
        } else {
            Some(first.min(self.rows.len() - 1))
        };
        self.selection = self.selected.into_iter().collect();
        self.selection_anchor = self.selected;
        self.dirty = true;
        self.recompute_prefix();
        self.reload_hotkeys();
        self.update_title(hwnd);
        self.start_analysis();
        self.autosave(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    fn copy_selection(&self) -> bool {
        let mut indices: Vec<usize> = self.selection.iter().copied().collect();
        if indices.is_empty() {
            if let Some(index) = self.selected {
                indices.push(index);
            }
        }
        if indices.is_empty() {
            return false;
        }
        indices.sort_unstable();
        let text = indices
            .iter()
            .filter_map(|index| self.rows.get(*index))
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        unsafe { clipboard::set_text(&text) }
    }

    unsafe fn cut_selection(&mut self, hwnd: HWND) -> bool {
        if self.copy_selection() {
            self.remove_selected(hwnd);
            true
        } else {
            false
        }
    }

    unsafe fn paste_clipboard(&mut self, hwnd: HWND) -> bool {
        let Some(text) = clipboard::get_text() else {
            return false;
        };
        let mut lines: Vec<String> = text
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
            .collect();
        if lines.is_empty() {
            return false;
        }
        self.commit_edit(hwnd, true);
        let insert = self
            .selected
            .unwrap_or(self.rows.len())
            .min(self.rows.len());
        let mut inserted = Vec::with_capacity(lines.len());
        for (offset, line) in lines.drain(..).enumerate() {
            let (kind, enabled) = {
                let structural = parse_line(&line);
                (structural.kind, structural.enabled)
            };
            let height = row_height(kind, self.dpi);
            let uid = self.next_uid;
            self.next_uid += 1;
            self.rows.insert(
                insert + offset,
                Row {
                    uid,
                    line,
                    kind,
                    height,
                    enabled,
                    item: None,
                    hotkey: None,
                    geq_table_top: 0,
                },
            );
            inserted.push(insert + offset);
        }
        self.selection = inserted.iter().copied().collect();
        self.selected = inserted.last().copied();
        self.selection_anchor = inserted.first().copied();
        self.dirty = true;
        self.recompute_prefix();
        self.update_title(hwnd);
        self.start_analysis();
        self.autosave(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
        true
    }

    unsafe fn on_context_menu(&mut self, hwnd: HWND, lparam: LPARAM) {
        let x = ((lparam.0 & 0xFFFF) as u16 as i16) as i32;
        let y = (((lparam.0 >> 16) & 0xFFFF) as u16 as i16) as i32;
        let chrome = self.chrome_height();
        let table_bottom = self.table_bottom(hwnd);
        if y >= table_bottom {
            return;
        }
        if y >= chrome && self.total > 0 {
            let content = y - chrome + self.offset;
            if content >= 0 && (content as u32) < self.total {
                let row = self
                    .prefix
                    .partition_point(|&start| start <= content as u32)
                    .saturating_sub(1)
                    .min(self.rows.len().saturating_sub(1));
                self.selected = Some(row);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        let menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => return,
        };
        let mut menu_texts: Vec<Vec<u16>> = Vec::new();
        let _ = AppendMenuW(menu, MF_STRING, MENU_TOGGLE, menu_text(&mut menu_texts, "Enable / disable"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_SET_HOTKEY, menu_text(&mut menu_texts, "Set hotkey..."));
        let _ = AppendMenuW(menu, MF_STRING, MENU_CLEAR_HOTKEY, menu_text(&mut menu_texts, "Clear hotkey"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_EDIT, menu_text(&mut menu_texts, "Edit text"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_OPEN_VST, menu_text(&mut menu_texts, "Open VST panel"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_REMOVE, menu_text(&mut menu_texts, "Remove"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, w!(""));
        let add_menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => {
                let _ = DestroyMenu(menu);
                return;
            }
        };
        let mut add_commands: Vec<(usize, String)> = Vec::new();
        for (group, entries) in ADD_GROUPS {
            let submenu = match CreatePopupMenu() {
                Ok(menu) => menu,
                Err(_) => continue,
            };
            for (label, line) in *entries {
                let id = 600 + add_commands.len();
                let _ = AppendMenuW(submenu, MF_STRING, id, menu_text(&mut menu_texts, label));
                add_commands.push((id, line.to_string()));
            }
            let _ = AppendMenuW(
                add_menu,
                MF_POPUP,
                submenu.0 as usize,
                menu_text(&mut menu_texts, group),
            );
        }
        let _ = AppendMenuW(menu, MF_POPUP, add_menu.0 as usize, menu_text(&mut menu_texts, "Add filter"));
        let point = Self::screen_point(hwnd, x, y);
        let command = TrackPopupMenuEx(menu, TPM_RETURNCMD.0, point.x, point.y, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        let before = self.selected;
        match command {
            MENU_TOGGLE => {
                if let Some(index) = self.selected {
                    self.toggle_row(hwnd, index);
                }
            }
            MENU_EDIT => {
                if let Some(index) = self.selected {
                    self.begin_edit(hwnd, index);
                }
            }
            MENU_SET_HOTKEY => {
                if let Some(index) = self.selected {
                    self.set_hotkey_for_row(hwnd, index);
                }
            }
            MENU_CLEAR_HOTKEY => {
                if let Some(index) = self.selected {
                    self.clear_hotkey_for_row(hwnd, index);
                }
            }
            MENU_OPEN_VST => {
                if let Some(index) = self.selected {
                    self.ensure_item(index);
                    let mut opened = false;
                    if let Some(item) = self.rows[index].item.as_mut() {
                        if let Some(FilterEditor::Vst(vst)) = item.editor.as_mut() {
                            vst_editor::open_plugin_editor(vst);
                            opened = true;
                        }
                    }
                    if opened {
                        self.sync_item(hwnd, index);
                        self.start_analysis();
                        self.autosave(hwnd);
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            MENU_REMOVE => self.remove_selected(hwnd),
            _ => {
                if let Some((_, line)) = add_commands.iter().find(|(id, _)| *id == command) {
                    let line = line.clone();
                    self.add_row(hwnd, &line, before);
                }
            }
        }
    }

    unsafe fn open_device_menu(&mut self, hwnd: HWND, x: i32, y: i32) {
        let menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => return,
        };
        let mut commands = Vec::new();
        let mut menu_texts: Vec<Vec<u16>> = Vec::new();
        for (index, device) in self.devices.iter().enumerate() {
            let label = format!(
                "{}: {} - {}",
                if device.is_input() { "Capture" } else { "Playback" },
                device.get_connection_name(),
                device.get_device_name()
            );
            let id = 800 + index;
            let _ = AppendMenuW(menu, MF_STRING, id, menu_raw(&mut menu_texts, &label));
            commands.push((id, index));
        }
        let point = Self::screen_point(hwnd, x, y);
        let command = TrackPopupMenuEx(menu, TPM_RETURNCMD.0, point.x, point.y, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        if command == 0 {
            self.menu_guard = Some((1, std::time::Instant::now()));
        }
        let Some((_, index)) = commands.into_iter().find(|(id, _)| *id == command) else {
            return;
        };
        let Some(device) = self.devices.get(index).cloned() else {
            return;
        };
        self.selected_device = Some(device.clone());
        self.channel_mask = device.get_channel_mask().max(1);
        self.channel_names = channel_helper::get_channel_names(
            device.get_channel_count() as i32,
            self.channel_mask,
        );
        self.analysis_channel_index = self
            .analysis_channel_index
            .min(self.channel_names.len().saturating_sub(1));
        for row in &mut self.rows {
            row.item = None;
        }
        self.start_analysis();
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    unsafe fn open_channel_menu(&mut self, hwnd: HWND, x: i32, y: i32) {
        let options = [
            ("From device", 0u32),
            ("Mono", channel_helper::KSAUDIO_SPEAKER_MONO),
            ("Stereo", channel_helper::KSAUDIO_SPEAKER_STEREO),
            ("Quadraphonic", channel_helper::KSAUDIO_SPEAKER_QUAD),
            (
                "5.1 Surround",
                channel_helper::KSAUDIO_SPEAKER_5POINT1_SURROUND,
            ),
            (
                "7.1 Surround",
                channel_helper::KSAUDIO_SPEAKER_7POINT1_SURROUND,
            ),
        ];
        let menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => return,
        };
        let mut menu_texts: Vec<Vec<u16>> = Vec::new();
        for (index, (label, _)) in options.iter().enumerate() {
            let _ = AppendMenuW(menu, MF_STRING, 830 + index, menu_text(&mut menu_texts, label));
        }
        let point = Self::screen_point(hwnd, x, y);
        let command = TrackPopupMenuEx(menu, TPM_RETURNCMD.0, point.x, point.y, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        if command == 0 {
            self.menu_guard = Some((2, std::time::Instant::now()));
        }
        if !(830..830 + options.len()).contains(&command) {
            return;
        }
        let mask = options[command - 830].1;
        self.channel_mask = if mask == 0 {
            self.selected_device
                .as_ref()
                .map(|device| device.get_channel_mask())
                .unwrap_or(3)
        } else {
            mask
        };
        let channel_count = self
            .selected_device
            .as_ref()
            .map(|device| device.get_channel_count() as i32)
            .unwrap_or(16);
        self.channel_names = channel_helper::get_channel_names(channel_count, self.channel_mask);
        self.analysis_channel_index = self
            .analysis_channel_index
            .min(self.channel_names.len().saturating_sub(1));
        for row in &mut self.rows {
            row.item = None;
        }
        self.start_analysis();
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    unsafe fn open_analysis_menu(&mut self, hwnd: HWND, x: i32, y: i32, kind: AnalysisSetting) {
        let menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => return,
        };
        let mut menu_texts: Vec<Vec<u16>> = Vec::new();
        match kind {
            AnalysisSetting::StartFrom => {
                let _ = AppendMenuW(menu, MF_STRING, 700, menu_text(&mut menu_texts, "config.txt"));
                let _ = AppendMenuW(menu, MF_STRING, 701, menu_text(&mut menu_texts, "Current file"));
            }
            AnalysisSetting::Channel => {
                for (index, name) in self.channel_names.iter().enumerate() {
                    let flags = if index == self.analysis_channel_index {
                        MF_STRING | MF_CHECKED
                    } else {
                        MF_STRING
                    };
                    let _ = AppendMenuW(menu, flags, 720 + index, menu_raw(&mut menu_texts, name));
                }
            }
            AnalysisSetting::Resolution => {
                for (index, resolution) in ANALYSIS_RESOLUTIONS.iter().enumerate() {
                    let _ = AppendMenuW(
                        menu,
                        MF_STRING,
                        740 + index,
                        menu_raw(&mut menu_texts, &resolution.to_string()),
                    );
                }
            }
        }
        let point = Self::screen_point(hwnd, x, y);
        let command = TrackPopupMenuEx(menu, TPM_RETURNCMD.0, point.x, point.y, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        if command == 0 {
            let guard = match kind {
                AnalysisSetting::StartFrom => 3,
                AnalysisSetting::Channel => 4,
                AnalysisSetting::Resolution => 5,
            };
            self.menu_guard = Some((guard, std::time::Instant::now()));
        }
        let mut changed = false;
        match kind {
            AnalysisSetting::StartFrom => {
                if command == 700 {
                    self.analysis_start_from = 0;
                    changed = true;
                } else if command == 701 {
                    self.analysis_start_from = 1;
                    changed = true;
                }
            }
            AnalysisSetting::Channel => {
                if (720..720 + self.channel_names.len()).contains(&command) {
                    self.analysis_channel_index = command - 720;
                    changed = true;
                }
            }
            AnalysisSetting::Resolution => {
                if (740..740 + ANALYSIS_RESOLUTIONS.len()).contains(&command) {
                    self.analysis_resolution = ANALYSIS_RESOLUTIONS[command - 740];
                    changed = true;
                }
            }
        }
        if changed {
            self.start_analysis();
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    fn chrome_height(&self) -> i32 {
        self.tab_top() + scale(33, self.dpi)
    }

    /// Top of the document tab strip (the toolbar above it can be hidden).
    fn tab_top(&self) -> i32 {
        if self.show_toolbar {
            scale(31, self.dpi)
        } else {
            0
        }
    }

    fn is_hot_rect(&self, rect: &RECT) -> bool {
        self.hover_pos
            .map(|(x, y)| point_in(rect, x, y))
            .unwrap_or(false)
    }

    /// Tab rectangles in paint order (client coordinates).
    fn tab_layout(&self) -> Vec<(usize, RECT)> {
        let dpi = self.dpi;
        let chrome = self.chrome_height();
        let top = self.tab_top() + scale(2, dpi);
        let mut tab_x = scale(8, dpi);
        let mut rects = Vec::with_capacity(self.tabs.len());
        for (index, tab) in self.tabs.iter().enumerate() {
            let title_len = tab.title.chars().count() as i32 + if tab.dirty { 2 } else { 0 };
            let width = (title_len * scale(7, dpi) + scale(34, dpi))
                .clamp(scale(90, dpi), scale(240, dpi));
            rects.push((
                index,
                RECT {
                    left: tab_x,
                    top,
                    right: tab_x + width,
                    bottom: chrome,
                },
            ));
            tab_x += width + scale(2, dpi);
        }
        rects
    }

    /// Rect of the clickable control under the cursor, used so that only the
    /// controls whose hover state changed get repainted.
    unsafe fn hot_control_at(&self, hwnd: HWND, x: i32, y: i32) -> Option<RECT> {
        let dpi = self.dpi;
        let chrome = self.chrome_height();
        let table_bottom = self.table_bottom(hwnd);
        if y >= table_bottom {
            return self
                .analysis_hits
                .iter()
                .find(|(_, rect)| point_in(rect, x, y))
                .map(|(_, rect)| *rect);
        }
        if self.show_toolbar && y < scale(30, dpi) {
            let top = scale(6, dpi);
            let bottom = top + scale(22, dpi);
            let mut left = scale(8, dpi);
            for _ in 0..3 {
                let rect = RECT {
                    left,
                    top,
                    right: left + scale(54, dpi),
                    bottom,
                };
                if point_in(&rect, x, y) {
                    return Some(rect);
                }
                left += scale(58, dpi);
            }
            for (l, r) in [(180, 260), (256, 384), (392, 504)] {
                let rect = RECT {
                    left: scale(l, dpi),
                    top,
                    right: scale(r, dpi),
                    bottom,
                };
                if point_in(&rect, x, y) {
                    return Some(rect);
                }
            }
            return None;
        }
        if y < chrome {
            return self
                .tab_layout()
                .into_iter()
                .find(|(_, rect)| point_in(rect, x, y))
                .map(|(_, rect)| rect);
        }
        if self.rows.is_empty() {
            return None;
        }
        let content = y - chrome + self.offset;
        if content < 0 || content as u32 >= self.total {
            return None;
        }
        let row = self
            .prefix
            .partition_point(|&start| start <= content as u32)
            .saturating_sub(1)
            .min(self.rows.len() - 1);
        let row_y = chrome + self.prefix[row] as i32 - self.offset;
        let height = self.rows[row].height as i32;
        let box_size = scale(12, dpi);
        let box_x = scale(20, dpi);
        let box_y = row_y + (height - box_size) / 2;
        let power_rect = RECT {
            left: box_x,
            top: box_y,
            right: box_x + box_size,
            bottom: box_y + box_size,
        };
        if point_in(&power_rect, x, y) {
            return Some(power_rect);
        }
        if !self.rows[row].enabled {
            return None;
        }
        let item = self.rows[row].item.as_ref()?;
        let editor = item.editor.as_ref()?;
        let hit = |rects: &[RECT]| rects.iter().find(|rect| point_in(rect, x, y)).copied();
        match editor {
            FilterEditor::Preamp { .. } => {
                let value = preamp_value_rect(dpi, row_y);
                if point_in(&value, x, y) {
                    return Some(value);
                }
                let mut track = preamp_slider_rect(dpi, row_y);
                track.right = scale(470, dpi);
                if point_in(&track, x, y) {
                    Some(track)
                } else {
                    None
                }
            }
            FilterEditor::Biquad(_) => hit(&biquad_field_rects(dpi, row_y)),
            FilterEditor::Delay { .. } => hit(&delay_field_rects(dpi, row_y)),
            FilterEditor::Stage { .. } => hit(&stage_field_rects(dpi, row_y)),
            FilterEditor::Include { .. } | FilterEditor::Convolution { .. } => {
                let rects = path_row_rects(dpi, row_y);
                if point_in(&rects[1], x, y) {
                    Some(rects[1])
                } else {
                    None
                }
            }
            FilterEditor::Vst(_) => {
                let mut rects = path_row_rects(dpi, row_y).to_vec();
                rects.extend_from_slice(&vst_action_rects(dpi, row_y));
                hit(&rects)
            }
            FilterEditor::Loudness { .. } => hit(&loudness_field_rects(dpi, row_y)),
            FilterEditor::Channel { .. } | FilterEditor::Device { .. } | FilterEditor::Copy { .. } => {
                let rect = change_button_rect(dpi, row_y);
                if point_in(&rect, x, y) {
                    Some(rect)
                } else {
                    None
                }
            }
            FilterEditor::GraphicEq(graphic) => {
                let (buttons, _) = geq_button_rects(dpi, row_y, graphic.band_count);
                if let Some(rect) = hit(&buttons) {
                    return Some(rect);
                }
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let table = geq_table_rect(dpi, row_y, self.rows[row].height as i32, client.right);
                let visible = geq_table_visible_rows(dpi, &table);
                let table_top = geq_table_top_clamped(
                    self.rows[row].geq_table_top,
                    graphic.nodes.len(),
                    visible,
                );
                if let Some((index, column)) = geq_table_cell_at(
                    dpi,
                    &table,
                    table_top,
                    visible,
                    graphic.nodes.len(),
                    x,
                    y,
                ) {
                    return geq_table_cell_rect(
                        dpi,
                        &table,
                        table_top,
                        visible,
                        graphic.nodes.len(),
                        index,
                        column,
                    );
                }
                None
            }
        }
    }

    fn analysis_height(&self) -> i32 {
        if self.show_analysis {
            scale(320, self.dpi)
        } else {
            0
        }
    }

    unsafe fn table_bottom(&self, hwnd: HWND) -> i32 {
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        client.bottom - self.analysis_height()
    }

    unsafe fn viewport_height(&self, hwnd: HWND) -> i32 {
        (self.table_bottom(hwnd) - self.chrome_height()).max(0)
    }

    unsafe fn update_scrollbar(&mut self, hwnd: HWND) {
        let viewport = self.viewport_height(hwnd);
        let max_offset = (self.total as i32 - viewport).max(0);
        self.offset = self.offset.clamp(0, max_offset);
        let info = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: self.total.saturating_sub(1) as i32,
            nPage: viewport as u32,
            nPos: self.offset,
            nTrackPos: self.offset,
        };
        SetScrollInfo(hwnd, SB_VERT, &info, true);
    }

    unsafe fn set_offset(&mut self, hwnd: HWND, offset: i32) {
        let viewport = self.viewport_height(hwnd);
        let max_offset = (self.total as i32 - viewport).max(0);
        self.offset = offset.clamp(0, max_offset);
        let info = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: self.total.saturating_sub(1) as i32,
            nPage: viewport as u32,
            nPos: self.offset,
            nTrackPos: self.offset,
        };
        SetScrollInfo(hwnd, SB_VERT, &info, true);
        self.prune_items(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    unsafe fn on_vscroll(&mut self, hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
        let command = (wparam.0 & 0xFFFF) as i32;
        let viewport = self.viewport_height(hwnd);
        let line = scale(24, self.dpi);
        let max_offset = (self.total as i32 - viewport).max(0);
        let mut offset = self.offset;
        match command {
            c if c == SB_LINEUP.0 => offset -= line,
            c if c == SB_LINEDOWN.0 => offset += line,
            c if c == SB_PAGEUP.0 => offset -= viewport,
            c if c == SB_PAGEDOWN.0 => offset += viewport,
            c if c == SB_THUMBPOSITION.0 || c == SB_THUMBTRACK.0 => {
                offset = ((lparam.0 >> 16) & 0xFFFF) as i32;
            }
            c if c == SB_TOP.0 => offset = 0,
            c if c == SB_BOTTOM.0 => offset = max_offset,
            c if c == SB_ENDSCROLL.0 => {}
            _ => {}
        }
        self.set_offset(hwnd, offset);
    }

    unsafe fn on_wheel(&mut self, hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
        self.commit_value_edit(hwnd, true);
        let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        let mut cursor = POINT {
            x: (lparam.0 & 0xFFFF) as u16 as i16 as i32,
            y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
        };
        let _ = ScreenToClient(hwnd, &mut cursor);
        if self.show_analysis && point_in(&self.plot_rect, cursor.x, cursor.y) {
            let view = (
                (self.plot_rect.right - self.plot_rect.left) as f32,
                (self.plot_rect.bottom - self.plot_rect.top) as f32,
            );
            self.plot.zoom_at(
                delta as f32,
                (cursor.x - self.plot_rect.left) as f32,
                (cursor.y - self.plot_rect.top) as f32,
                view.0,
                view.1,
            );
            self.invalidate_plot(hwnd);
            return;
        }
        if let Some((row, table)) = self.geq_row_table_at(hwnd, cursor.x, cursor.y) {
            self.ensure_item(row);
            let visible = geq_table_visible_rows(self.dpi, &table);
            let nodes = match self.rows[row]
                .item
                .as_ref()
                .and_then(|item| item.editor.as_ref())
            {
                Some(FilterEditor::GraphicEq(graphic)) => graphic.nodes.len(),
                _ => 0,
            };
            if nodes > 0 {
                let max_top = nodes.saturating_sub(visible);
                let current = self.rows[row].geq_table_top.min(max_top);
                let step = 3usize;
                let next = if delta > 0 {
                    current.saturating_sub(step)
                } else {
                    current.saturating_add(step).min(max_top)
                };
                if next != current {
                    self.rows[row].geq_table_top = next;
                    self.invalidate_row(hwnd, row);
                }
                return;
            }
        }
        if let Some((row, plot)) = self.geq_row_plot_at(hwnd, cursor.x, cursor.y) {
            self.ensure_item(row);
            let mut zoomed = false;
            if let Some(item) = self.rows[row].item.as_mut() {
                if let Some(FilterEditor::GraphicEq(graphic)) = &mut item.editor {
                    let mut view = GeqView::new(graphic);
                    view.zoom_at(
                        delta as f32,
                        (cursor.x - plot.left) as f32,
                        (cursor.y - plot.top) as f32,
                        &plot,
                    );
                    view.store(graphic);
                    zoomed = true;
                }
            }
            if zoomed {
                self.invalidate_row(hwnd, row);
                return;
            }
        }
        let line = scale(24, self.dpi);
        let offset = self.offset - delta / 120 * line * 3;
        self.set_offset(hwnd, offset);
    }

    /// Finds the GraphicEQ row plot under a client point, if any.
    unsafe fn geq_row_plot_at(&self, hwnd: HWND, x: i32, y: i32) -> Option<(usize, RECT)> {
        let chrome = self.chrome_height();
        if self.rows.is_empty() || y < chrome || y >= self.table_bottom(hwnd) {
            return None;
        }
        let content = y - chrome + self.offset;
        if content < 0 || content as u32 >= self.total {
            return None;
        }
        let row = self
            .prefix
            .partition_point(|&start| start <= content as u32)
            .saturating_sub(1)
            .min(self.rows.len() - 1);
        if self.rows[row].kind != FilterKind::GraphicEq {
            return None;
        }
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let row_y = chrome + self.prefix[row] as i32 - self.offset;
        let plot = geq_plot_rect(self.dpi, row_y, self.rows[row].height as i32, client.right);
        point_in(&plot, x, y).then_some((row, plot))
    }

    /// Finds the GraphicEQ row node table under a client point, if any.
    unsafe fn geq_row_table_at(&self, hwnd: HWND, x: i32, y: i32) -> Option<(usize, RECT)> {
        let chrome = self.chrome_height();
        if self.rows.is_empty() || y < chrome || y >= self.table_bottom(hwnd) {
            return None;
        }
        let content = y - chrome + self.offset;
        if content < 0 || content as u32 >= self.total {
            return None;
        }
        let row = self
            .prefix
            .partition_point(|&start| start <= content as u32)
            .saturating_sub(1)
            .min(self.rows.len() - 1);
        if self.rows[row].kind != FilterKind::GraphicEq {
            return None;
        }
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let row_y = chrome + self.prefix[row] as i32 - self.offset;
        let table = geq_table_rect(self.dpi, row_y, self.rows[row].height as i32, client.right);
        point_in(&table, x, y).then_some((row, table))
    }

    unsafe fn on_key(&mut self, hwnd: HWND, wparam: WPARAM) {
        let key = wparam.0 as u16;
        let control = GetKeyState(VK_CONTROL.0 as i32) < 0;
        if control && key == VK_S.0 {
            self.save(hwnd);
            return;
        }
        if control && key == VK_N.0 {
            self.new_tab(hwnd);
            return;
        }
        if control && key == VK_O.0 {
            self.open_dialog(hwnd);
            return;
        }
        if control && key == VK_W.0 {
            self.close_tab(hwnd, self.active_tab);
            return;
        }
        if control && key == VK_C.0 {
            self.copy_selection();
            return;
        }
        if control && key == VK_X.0 {
            self.cut_selection(hwnd);
            return;
        }
        if control && key == VK_V.0 {
            self.paste_clipboard(hwnd);
            return;
        }
        if control && key == VK_A.0 {
            self.selection = (0..self.rows.len()).collect();
            if !self.rows.is_empty() {
                self.selected = Some(0);
                self.selection_anchor = Some(0);
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            return;
        }
        if self.rows.is_empty() {
            return;
        }
        if key == VK_DELETE.0 {
            self.remove_selected(hwnd);
            return;
        }
        if key == VK_F2.0 {
            if let Some(index) = self.selected {
                self.begin_edit(hwnd, index);
            }
            return;
        }
        if key == VK_SPACE.0 {
            if let Some(index) = self.selected {
                let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
                if ctrl && self.selection.remove(&index) {
                    // toggled off
                } else {
                    self.selection.insert(index);
                }
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            return;
        }
        let viewport = self.viewport_height(hwnd);
        let page_rows = (viewport / scale(24, self.dpi)).max(1) as usize;
        let current = self.selected.unwrap_or(0);
        let last = self.rows.len() - 1;
        let target = if key == VK_UP.0 {
            current.saturating_sub(1)
        } else if key == VK_DOWN.0 {
            (current + 1).min(last)
        } else if key == VK_PRIOR.0 {
            current.saturating_sub(page_rows)
        } else if key == VK_NEXT.0 {
            (current + page_rows).min(last)
        } else if key == VK_HOME.0 {
            0
        } else if key == VK_END.0 {
            last
        } else {
            return;
        };
        self.commit_edit(hwnd, true);
        let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
        let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
        if shift {
            self.select_row(target, (false, true));
        } else if ctrl {
            self.selected = Some(target);
        } else {
            self.select_row(target, (false, false));
        }
        self.ensure_visible(hwnd, target);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    unsafe fn close_tab(&mut self, hwnd: HWND, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if index != self.active_tab {
            self.switch_tab(hwnd, index);
        }
        self.commit_edit(hwnd, true);
        self.capture_active_tab();
        if self.tabs[index].dirty {
            let title = wide_z(&format!(
                "Save changes to {} before closing?",
                self.tabs[index].title
            ));
            let answer = MessageBoxW(
                Some(hwnd),
                PCWSTR(title.as_ptr()),
                w!("Equalizer APO"),
                MB_YESNOCANCEL | MB_ICONWARNING,
            );
            if answer.0 == IDCANCEL.0 {
                return;
            }
            if answer.0 == IDYES.0 && !self.save(hwnd) {
                return;
            }
        }
        self.tabs.remove(index);
        if self.tabs.is_empty() {
            self.tabs.push(TabInfo {
                title: "Unsaved".to_string(),
                path: None,
                lines: vec![String::new()],
                offset: 0,
                selected: None,
                dirty: false,
            });
        }
        let next = index.min(self.tabs.len() - 1);
        self.active_tab = usize::MAX;
        self.load_tab(hwnd, next);
    }

    fn tab_hit(&self, x: i32) -> Option<(usize, bool)> {
        self.tab_layout().into_iter().find_map(|(index, rect)| {
            if x >= rect.left && x < rect.right {
                Some((index, x >= rect.right - scale(22, self.dpi)))
            } else {
                None
            }
        })
    }

    unsafe fn on_click(&mut self, hwnd: HWND, lparam: LPARAM) {
        let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        let chrome = self.chrome_height();
        let table_bottom = self.table_bottom(hwnd);
        if y >= table_bottom {
            let hit = self
                .analysis_hits
                .iter()
                .find(|(_, rect)| point_in(rect, x, y))
                .copied();
            if let Some((kind, rect)) = hit {
                let guard = match kind {
                    AnalysisSetting::StartFrom => 3,
                    AnalysisSetting::Channel => 4,
                    AnalysisSetting::Resolution => 5,
                };
                if self.menu_toggle_allowed(guard) {
                    self.open_analysis_menu(hwnd, rect.left, rect.bottom, kind);
                }
            }
            return;
        }
        if self.show_toolbar && y < scale(30, self.dpi) {
            let new_left = scale(8, self.dpi);
            let new_right = scale(62, self.dpi);
            let open_right = scale(120, self.dpi);
            let save_right = scale(178, self.dpi);
            if x >= new_left && x < new_right {
                self.new_tab(hwnd);
            } else if x >= scale(66, self.dpi) && x < open_right {
                self.open_dialog(hwnd);
            } else if x >= scale(124, self.dpi) && x < save_right {
                self.save(hwnd);
            } else if x >= scale(180, self.dpi) && x < scale(260, self.dpi) {
                self.instant_mode = !self.instant_mode;
                let _ = InvalidateRect(Some(hwnd), None, false);
            } else if x >= scale(256, self.dpi) && x < scale(384, self.dpi) {
                if self.menu_toggle_allowed(1) {
                    self.open_device_menu(hwnd, scale(256, self.dpi), scale(30, self.dpi));
                }
            } else if x >= scale(392, self.dpi) && x < scale(504, self.dpi) {
                if self.menu_toggle_allowed(2) {
                    self.open_channel_menu(hwnd, scale(392, self.dpi), scale(30, self.dpi));
                }
            }
            return;
        }
        if y >= self.tab_top() && y < chrome {
            if let Some((index, close)) = self.tab_hit(x) {
                if close {
                    self.close_tab(hwnd, index);
                } else {
                    self.switch_tab(hwnd, index);
                }
            }
            return;
        }
        if y < chrome || y >= table_bottom {
            return;
        }
        let content = y - chrome + self.offset;
        if content < 0 || content as u32 >= self.total || self.rows.is_empty() {
            return;
        }
        let row = self
            .prefix
            .partition_point(|&start| start <= content as u32)
            .saturating_sub(1)
            .min(self.rows.len() - 1);
        self.commit_edit(hwnd, true);
        let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
        let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
        let power_left = scale(18, self.dpi);
        let power_right = scale(36, self.dpi);
        if x < power_left {
            self.select_row(row, (false, false));
            self.reorder = Some(ReorderState {
                start_y: y,
                active: false,
            });
            self.reorder_drop = Some(row);
            let _ = SetCapture(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
            return;
        }
        if x >= power_left && x <= power_right {
            self.select_row(row, (false, false));
            self.toggle_row(hwnd, row);
            return;
        }
        self.select_row(row, (ctrl, shift));
        self.ensure_item(row);
        let row_y = self.chrome_height() + self.prefix[row] as i32 - self.offset;
        if self.rows[row].enabled {
            self.handle_editor_click(hwnd, row, x, y, row_y);
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    fn select_row(&mut self, row: usize, modifiers: (bool, bool)) {
        let (ctrl, shift) = modifiers;
        if shift {
            let anchor = self.selection_anchor.unwrap_or(row);
            let start = anchor.min(row);
            let end = anchor.max(row);
            self.selection = (start..=end).collect();
            self.selected = Some(row);
        } else if ctrl {
            if !self.selection.remove(&row) {
                self.selection.insert(row);
            }
            self.selected = Some(row);
            self.selection_anchor = Some(row);
        } else {
            self.selection.clear();
            self.selection.insert(row);
            self.selected = Some(row);
            self.selection_anchor = Some(row);
        }
    }

    unsafe fn handle_editor_click(
        &mut self,
        hwnd: HWND,
        row: usize,
        x: i32,
        mouse_y: i32,
        row_y: i32,
    ) {
        let dpi = self.dpi;
        let row_height_px = self.rows[row].height as i32;
        let geq_table_top = self.rows[row].geq_table_top;
        let mut sync = false;
        let mut analyze = false;
        let mut drag = None;
        let mut calibrate = None;
        let mut vst_options = None;
        let mut edit_value: Option<ValueField> = None;
        let mut pan_geq = false;
        let mut biquad_menu: Option<(BiquadMenu, RECT)> = None;
        if let Some(item) = self.rows[row].item.as_mut() {
            match &mut item.editor {
                Some(FilterEditor::Preamp { db }) => {
                    let track = preamp_slider_rect(dpi, row_y);
                    let right = RECT { right: scale(470, dpi), ..track };
                    if point_in(&preamp_value_rect(dpi, row_y), x, mouse_y) {
                        edit_value = Some(ValueField::PreampDb);
                    } else if point_in(&right, x, mouse_y) {
                        let t = ((x - right.left) as f64 / (right.right - right.left).max(1) as f64)
                            .clamp(0.0, 1.0);
                        *db = ((-20.0 + t * 40.0) * 10.0).round() / 10.0;
                        drag = Some(DragState {
                            row,
                            field: DragField::PreampDb,
                            start_x: x,
                            start_value: *db,
                            node: 0,
                        });
                        sync = true;
                        analyze = true;
                    }
                }
                Some(FilterEditor::Biquad(biquad)) => {
                    let rects = biquad_field_rects(dpi, row_y);
                    if point_in(&rects[0], x, mouse_y) {
                        biquad_menu = Some((BiquadMenu::Kind, rects[0]));
                    } else if point_in(&rects[4], x, mouse_y) {
                        biquad_menu = Some((BiquadMenu::QMode, rects[4]));
                    } else if point_in(&rects[1], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::BiquadFreq,
                            start_x: x,
                            start_value: biquad.freq,
                            node: 0,
                        });
                    } else if point_in(&rects[2], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::BiquadGain,
                            start_x: x,
                            start_value: biquad.gain,
                            node: 0,
                        });
                    } else if point_in(&rects[3], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::BiquadQ,
                            start_x: x,
                            start_value: biquad.q,
                            node: 0,
                        });
                    }
                }
                Some(FilterEditor::Delay { delay, is_ms }) => {
                    let rects = delay_field_rects(dpi, row_y);
                    if point_in(&rects[0], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::DelayValue,
                            start_x: x,
                            start_value: *delay,
                            node: 0,
                        });
                    } else if point_in(&rects[1], x, mouse_y) {
                        *is_ms = !*is_ms;
                        sync = true;
                        analyze = true;
                    }
                }
                Some(FilterEditor::Stage {
                    pre_mix,
                    post_mix,
                    capture,
                }) => {
                    let rects = stage_field_rects(dpi, row_y);
                    if point_in(&rects[0], x, mouse_y) {
                        *pre_mix = !*pre_mix;
                        sync = true;
                        analyze = true;
                    } else if point_in(&rects[1], x, mouse_y) {
                        *post_mix = !*post_mix;
                        sync = true;
                        analyze = true;
                    } else if point_in(&rects[2], x, mouse_y) {
                        *capture = !*capture;
                        sync = true;
                        analyze = true;
                    }
                }
                Some(FilterEditor::Include { path }) => {
                    let rects = path_row_rects(dpi, row_y);
                    if point_in(&rects[1], x, mouse_y) {
                        if let Some(chosen) = FileDialog::new()
                            .set_title("Include configuration file")
                            .add_filter("Configuration files", &["txt"])
                            .pick_file()
                        {
                            *path = chosen.to_string_lossy().into_owned();
                            sync = true;
                            analyze = true;
                        }
                    }
                }
                Some(FilterEditor::Convolution { path, .. }) => {
                    let rects = path_row_rects(dpi, row_y);
                    if point_in(&rects[1], x, mouse_y) {
                        if let Some(chosen) = FileDialog::new()
                            .set_title("Select impulse response")
                            .add_filter("Impulse responses", &["wav", "flac", "ogg"])
                            .pick_file()
                        {
                            *path = chosen.to_string_lossy().into_owned();
                            sync = true;
                            analyze = true;
                        }
                    }
                }
                Some(FilterEditor::Vst(vst)) => {
                    let rects = path_row_rects(dpi, row_y);
                    let actions = vst_action_rects(dpi, row_y);
                    if point_in(&rects[0], x, mouse_y) {
                        let current = vst.path.clone();
                        if let Some(text) = prompt_text(hwnd, "VST plugin path", &current) {
                            vst.path = text.trim().to_string();
                            vst_editor::refresh_plugin_status(vst);
                            sync = true;
                            analyze = true;
                        }
                    } else if point_in(&rects[1], x, mouse_y) {
                        if let Some(chosen) = FileDialog::new()
                            .set_title("Select VST plugin")
                            .add_filter("VST plugins", &["dll"])
                            .pick_file()
                        {
                            vst.path =
                                eapo_editor_core::model::vst_config_path(&chosen.to_string_lossy());
                            vst_editor::refresh_plugin_status(vst);
                            sync = true;
                            analyze = true;
                        }
                    } else if point_in(&actions[0], x, mouse_y) {
                        if vst_editor::open_plugin_editor(vst) {
                            sync = true;
                            analyze = true;
                        }
                    } else if point_in(&actions[1], x, mouse_y) {
                        vst_options = Some((actions[1].left, actions[1].bottom));
                    }
                }
                Some(FilterEditor::Loudness {
                    state,
                    ref_level,
                    ref_offset,
                    att,
                    volume,
                    ..
                }) => {
                    let rects = loudness_field_rects(dpi, row_y);
                    if point_in(&rects[0], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::LoudnessRef,
                            start_x: x,
                            start_value: *ref_level as f64,
                            node: 0,
                        });
                    } else if point_in(&rects[1], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::LoudnessOffset,
                            start_x: x,
                            start_value: *ref_offset as f64,
                            node: 0,
                        });
                    } else if point_in(&rects[2], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::LoudnessAtt,
                            start_x: x,
                            start_value: *att,
                            node: 0,
                        });
                    } else if point_in(&rects[3], x, mouse_y) {
                        *state = false;
                        calibrate = Some(*volume);
                        sync = true;
                        analyze = true;
                    }
                }
                Some(FilterEditor::Copy {
                    assignments,
                    channel_names,
                    ..
                }) => {
                    let button = change_button_rect(dpi, row_y);
                    if point_in(&button, x, mouse_y) {
                        if let Some(result) =
                            copy_dialog::edit_copy(hwnd, assignments, channel_names)
                        {
                            *assignments = result;
                            for assignment in assignments.iter() {
                                if !assignment.target.is_empty()
                                    && !channel_names
                                        .iter()
                                        .any(|name| name.eq_ignore_ascii_case(&assignment.target))
                                {
                                    channel_names.push(assignment.target.clone());
                                }
                                for summand in &assignment.summands {
                                    let name = summand.channel.trim();
                                    if !name.is_empty()
                                        && summand.channel != " "
                                        && !channel_names
                                            .iter()
                                            .any(|existing| existing.eq_ignore_ascii_case(name))
                                    {
                                        channel_names.push(name.to_string());
                                    }
                                }
                            }
                            sync = true;
                            analyze = true;
                        }
                    }
                }
                Some(FilterEditor::Channel {
                    selected,
                    channel_names,
                    channel_mask,
                    ..
                }) => {
                    let button = change_button_rect(dpi, row_y);
                    if point_in(&button, x, mouse_y) {
                        let mut names: Vec<String> = Vec::new();
                        let mut labels: Vec<String> = Vec::new();
                        for (name, label) in speaker_positions(*channel_mask) {
                            names.push(name.to_string());
                            labels.push(format!("{name} ({label})"));
                        }
                        for name in channel_names.iter() {
                            if !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                                names.push(name.clone());
                                labels.push(name.clone());
                            }
                        }
                        for name in selected.iter() {
                            if name != "ALL"
                                && !names.iter().any(|n| n.eq_ignore_ascii_case(name))
                            {
                                names.push(name.clone());
                                labels.push(name.clone());
                            }
                        }
                        let checked: Vec<usize> = names
                            .iter()
                            .enumerate()
                            .filter(|(_, name)| {
                                selected.iter().any(|s| s.eq_ignore_ascii_case(name))
                            })
                            .map(|(index, _)| index)
                            .collect();
                        let all = selected.iter().any(|s| s == "ALL");
                        let options = chooser::CheckListOptions {
                            all_label: Some(i18n::tr("Select all channels")),
                            all_checked: all,
                            filter: None,
                        };
                        if let Some(result) = chooser::check_list(
                            hwnd,
                            &i18n::tr("Channel selection"),
                            &labels,
                            &checked,
                            options,
                        ) {
                            *selected = if result.all {
                                vec!["ALL".to_string()]
                            } else {
                                result
                                    .checked
                                    .iter()
                                    .filter_map(|index| names.get(*index))
                                    .map(|name| name.to_uppercase())
                                    .collect()
                            };
                            sync = true;
                            analyze = true;
                        }
                    }
                }
                Some(FilterEditor::Device { pattern, .. }) => {
                    let button = change_button_rect(dpi, row_y);
                    if point_in(&button, x, mouse_y) {
                        let all = pattern.trim().eq_ignore_ascii_case("all");
                        let mut labels = Vec::with_capacity(self.devices.len());
                        let mut keys = Vec::with_capacity(self.devices.len());
                        let mut keep = Vec::with_capacity(self.devices.len());
                        for device in &self.devices {
                            let kind = if device.is_input() {
                                "Capture"
                            } else {
                                "Playback"
                            };
                            let apo = if device.is_installed() {
                                "APO installed"
                            } else {
                                "APO not installed"
                            };
                            labels.push(format!(
                                "{kind} | {} | {} | {apo}",
                                device.get_connection_name(),
                                device.get_device_name()
                            ));
                            keys.push(device.get_device_string());
                            keep.push(device.is_installed());
                        }
                        let checked: Vec<usize> = if all {
                            Vec::new()
                        } else {
                            keys.iter()
                                .enumerate()
                                .filter(|(_, key)| device_matches(key, pattern))
                                .map(|(index, _)| index)
                                .collect()
                        };
                        let options = chooser::CheckListOptions {
                            all_label: Some(i18n::tr("Select all devices")),
                            all_checked: all,
                            filter: Some((
                                i18n::tr("Show only devices with APO installed"),
                                true,
                                keep,
                            )),
                        };
                        if let Some(result) = chooser::check_list(
                            hwnd,
                            &i18n::tr("Device selection"),
                            &labels,
                            &checked,
                            options,
                        ) {
                            *pattern = if result.all {
                                "all".to_string()
                            } else {
                                let selected: Vec<String> = result
                                    .checked
                                    .iter()
                                    .filter_map(|index| keys.get(*index))
                                    .cloned()
                                    .collect();
                                selected.join("; ")
                            };
                            sync = true;
                            analyze = true;
                        }
                    }
                }
                Some(FilterEditor::GraphicEq(graphic)) => {
                    let (buttons, _) = geq_button_rects(dpi, row_y, graphic.band_count);
                    let mut client = RECT::default();
                    let _ = GetClientRect(hwnd, &mut client);
                    let height = row_height_px;
                    let plot = geq_plot_rect(dpi, row_y, height, client.right);
                    let table = geq_table_rect(dpi, row_y, height, client.right);
                    let visible = geq_table_visible_rows(dpi, &table);
                    let table_top =
                        geq_table_top_clamped(geq_table_top, graphic.nodes.len(), visible);
                    if point_in(&buttons[0], x, mouse_y) {
                        graphic.set_band_count(15);
                        sync = true;
                        analyze = true;
                    } else if point_in(&buttons[1], x, mouse_y) {
                        graphic.set_band_count(31);
                        sync = true;
                        analyze = true;
                    } else if point_in(&buttons[2], x, mouse_y) {
                        graphic.set_band_count(-1);
                        sync = true;
                        analyze = true;
                    } else if point_in(&buttons[3], x, mouse_y) {
                        for node in &mut graphic.nodes {
                            node.db_gain = -node.db_gain;
                        }
                        sync = true;
                        analyze = true;
                    } else if point_in(&buttons[4], x, mouse_y) {
                        let max = graphic
                            .nodes
                            .iter()
                            .map(|node| node.db_gain)
                            .fold(f64::NEG_INFINITY, f64::max);
                        if max.is_finite() {
                            for node in &mut graphic.nodes {
                                node.db_gain -= max;
                            }
                        }
                        sync = true;
                        analyze = true;
                    } else if point_in(&buttons[5], x, mouse_y) {
                        for node in &mut graphic.nodes {
                            node.db_gain = 0.0;
                        }
                        sync = true;
                        analyze = true;
                    } else if let Some((index, column)) = geq_table_cell_at(
                        dpi,
                        &table,
                        table_top,
                        visible,
                        graphic.nodes.len(),
                        x,
                        mouse_y,
                    ) {
                        graphic.selected = vec![index];
                        edit_value = Some(if column == 0 {
                            ValueField::GeqFreq(index)
                        } else {
                            ValueField::GeqGain(index)
                        });
                    } else if point_in(&plot, x, mouse_y) {
                        let view = GeqView::new(graphic);
                        let margin = scale(10, dpi);
                        let mut best: Option<(usize, i32)> = None;
                        for (index, node) in graphic.nodes.iter().enumerate() {
                            let nx = view.x_of(node.freq, &plot).round() as i32;
                            let ny = view
                                .y_of(node.db_gain.clamp(GEQ_WORLD_MIN_DB, GEQ_WORLD_MAX_DB), &plot)
                                .round() as i32;
                            if nx < plot.left - margin
                                || nx > plot.right + margin
                                || ny < plot.top - margin
                                || ny > plot.bottom + margin
                            {
                                continue;
                            }
                            let distance = (nx - x).abs() + (ny - mouse_y).abs();
                            if distance <= scale(10, dpi)
                                && best.map(|(_, best_distance)| distance < best_distance).unwrap_or(true)
                            {
                                best = Some((index, distance));
                            }
                        }
                        if let Some((index, _)) = best {
                            graphic.selected = vec![index];
                            let value = graphic.nodes[index].db_gain;
                            drag = Some(DragState {
                                row,
                                field: DragField::GeqNode,
                                start_x: x,
                                start_value: value,
                                node: index,
                            });
                        } else {
                            graphic.selected.clear();
                            pan_geq = true;
                        }
                    }
                }
                _ => {}
            }
        }
        if pan_geq {
            self.geq_pan = Some((row, x, mouse_y));
            let _ = SetCapture(hwnd);
        }
        if let Some(field) = edit_value {
            self.begin_value_edit(hwnd, row, field);
            return;
        }
        if let Some((menu_kind, rect)) = biquad_menu {
            let guard = if matches!(menu_kind, BiquadMenu::Kind) { 7 } else { 8 };
            if self.menu_toggle_allowed(guard) {
                self.biquad_type_menu(hwnd, row, rect.left, rect.bottom, menu_kind);
            }
        }
        if let Some((menu_x, menu_y)) = vst_options {
            if !self.menu_toggle_allowed(6) {
                return;
            }
            if let Some(item) = self.rows[row].item.as_mut() {
                if let Some(FilterEditor::Vst(vst)) = item.editor.as_mut() {
                    let menu = match CreatePopupMenu() {
                        Ok(menu) => menu,
                        Err(_) => return,
                    };
                    let _ = AppendMenuW(
                        menu,
                        if vst.embed {
                            MF_STRING | MF_CHECKED
                        } else {
                            MF_STRING
                        },
                        MENU_VST_EMBED,
                        w!("Embed"),
                    );
                    let _ = AppendMenuW(
                        menu,
                        if vst.auto_apply {
                            MF_STRING | MF_CHECKED
                        } else {
                            MF_STRING
                        },
                        MENU_VST_AUTO_APPLY,
                        w!("Auto apply"),
                    );
                    let mut point = POINT { x: menu_x, y: menu_y };
                    let _ = ClientToScreen(hwnd, &mut point);
                    let command =
                        TrackPopupMenuEx(menu, TPM_RETURNCMD.0, point.x, point.y, hwnd, None).0
                            as usize;
                    let _ = DestroyMenu(menu);
                    if command == 0 {
                        self.menu_guard = Some((6, std::time::Instant::now()));
                    }
                    match command {
                        MENU_VST_EMBED => vst.embed = !vst.embed,
                        MENU_VST_AUTO_APPLY => vst.auto_apply = !vst.auto_apply,
                        _ => {}
                    }
                    if command == MENU_VST_EMBED || command == MENU_VST_AUTO_APPLY {
                        self.dirty = true;
                        self.autosave(hwnd);
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            return;
        }
        if let Some(volume) = calibrate {
            self.sync_item(hwnd, row);
            self.autosave(hwnd);
            self.start_analysis();
            calibration::stop();
            let measured_level = calibration_dialog::calibrate(hwnd, volume);
            if let Some(item) = self.rows[row].item.as_mut() {
                if let Some(FilterEditor::Loudness {
                    state,
                    ref_level,
                    measured,
                    ..
                }) = item.editor.as_mut()
                {
                    *state = true;
                    if let Some(value) = measured_level {
                        *ref_level = ((75 - value) as f64 + volume) as i32;
                        *measured = value;
                    }
                }
            }
            sync = true;
            analyze = true;
        }
        if let Some(drag) = drag {
            self.drag = Some(drag);
            self.drag_moved = false;
            let _ = SetCapture(hwnd);
        }
        if sync {
            self.sync_item(hwnd, row);
            if drag.is_none() {
                self.autosave(hwnd);
            }
        }
        if analyze {
            self.start_analysis();
        }
        if matches!(
            self.rows[row].item.as_ref().and_then(|item| item.editor.as_ref()),
            Some(FilterEditor::GraphicEq(_))
        ) {
            self.ensure_geq_table_visible(hwnd, row);
        }
    }

    unsafe fn handle_editor_double_click(
        &mut self,
        hwnd: HWND,
        row: usize,
        x: i32,
        mouse_y: i32,
        row_y: i32,
    ) -> bool {
        self.ensure_item(row);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let height = self.rows[row].height as i32;
        let plot = geq_plot_rect(self.dpi, row_y, height, client.right);
        if !point_in(&plot, x, mouse_y) {
            return false;
        }
        let mut added = false;
        if let Some(item) = self.rows[row].item.as_mut() {
            if let Some(FilterEditor::GraphicEq(graphic)) = &mut item.editor {
                if graphic.band_count < 0 {
                    let view = GeqView::new(graphic);
                    let frequency = view.frequency_at(x as f32, &plot);
                    let db_gain = (view.db_at(mouse_y as f32, &plot) * 10.0).round() / 10.0;
                    graphic.nodes.push(FilterNode {
                        freq: frequency,
                        db_gain,
                    });
                    graphic.sort_nodes();
                    graphic.selected = vec![graphic.nodes.len().saturating_sub(1)];
                    added = true;
                }
            }
        }
        if added {
            self.sync_item(hwnd, row);
            self.start_analysis();
            self.ensure_geq_table_visible(hwnd, row);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        added
    }

    unsafe fn biquad_type_menu(
        &mut self,
        hwnd: HWND,
        row: usize,
        x: i32,
        y: i32,
        menu_kind: BiquadMenu,
    ) {
        let menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => return,
        };
        let mut menu_texts: Vec<Vec<u16>> = Vec::new();
        match menu_kind {
            BiquadMenu::Kind => {
                for (offset, label) in [
                    "Peaking filter",
                    "Low-pass filter",
                    "High-pass filter",
                    "Band-pass filter",
                    "Low-shelf filter",
                    "High-shelf filter",
                    "Notch filter",
                    "All-pass filter",
                ]
                .iter()
                .enumerate()
                {
                    let _ = AppendMenuW(
                        menu,
                        MF_STRING,
                        400 + offset,
                        menu_text(&mut menu_texts, label),
                    );
                }
            }
            BiquadMenu::QMode => {
                for (offset, label) in ["Fixed", "Q", "Bandwidth", "Slope"].iter().enumerate() {
                    let _ = AppendMenuW(
                        menu,
                        MF_STRING,
                        420 + offset,
                        menu_text(&mut menu_texts, label),
                    );
                }
            }
        }
        let point = Self::screen_point(hwnd, x, y);
        let command = TrackPopupMenuEx(menu, TPM_RETURNCMD.0, point.x, point.y, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        if command == 0 {
            let guard = if matches!(menu_kind, BiquadMenu::Kind) { 7 } else { 8 };
            self.menu_guard = Some((guard, std::time::Instant::now()));
        }
        let mut changed = false;
        if let Some(item) = self.rows[row].item.as_mut() {
            if let Some(FilterEditor::Biquad(biquad)) = &mut item.editor {
                match menu_kind {
                    BiquadMenu::Kind => {
                        let kinds = [
                            BiquadKind::Peaking,
                            BiquadKind::LowPass,
                            BiquadKind::HighPass,
                            BiquadKind::BandPass,
                            BiquadKind::LowShelf,
                            BiquadKind::HighShelf,
                            BiquadKind::Notch,
                            BiquadKind::AllPass,
                        ];
                        if (400..408).contains(&command) {
                            biquad.kind = kinds[command - 400];
                            changed = true;
                        }
                    }
                    BiquadMenu::QMode => {
                        let modes = [QMode::Fixed, QMode::Q, QMode::Bandwidth, QMode::Slope];
                        if (420..424).contains(&command) {
                            biquad.q_mode = modes[command - 420];
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            self.sync_item(hwnd, row);
            self.start_analysis();
            self.autosave(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    /// Converts client coordinates to screen coordinates for popup menus.
    unsafe fn screen_point(hwnd: HWND, x: i32, y: i32) -> POINT {
        let mut point = POINT { x, y };
        let _ = ClientToScreen(hwnd, &mut point);
        point
    }

    /// Returns false when this click only dismissed a menu that was just
    /// closed, so dropdown buttons behave as toggles instead of reopening.
    fn menu_toggle_allowed(&mut self, kind: u8) -> bool {
        if let Some((previous, at)) = self.menu_guard {
            if previous == kind
                && at.elapsed() < std::time::Duration::from_millis(400)
            {
                self.menu_guard = None;
                return false;
            }
        }
        self.menu_guard = None;
        true
    }

    unsafe fn invalidate_row(&self, hwnd: HWND, row: usize) {
        if row >= self.rows.len() {
            return;
        }
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let top = self.chrome_height() + self.prefix[row] as i32 - self.offset;
        let rect = RECT {
            left: 0,
            top,
            right: client.right,
            bottom: top + self.rows[row].height as i32,
        };
        let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
    }

    unsafe fn invalidate_plot(&self, hwnd: HWND) {
        if self.plot_rect.right > self.plot_rect.left && self.plot_rect.bottom > self.plot_rect.top
        {
            let _ = InvalidateRect(Some(hwnd), Some(&self.plot_rect), false);
        } else {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    unsafe fn on_mouse_move(&mut self, hwnd: HWND, lparam: LPARAM) {
        let move_x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
        let move_y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        self.hover_pos = Some((move_x, move_y));
        let hot = self.hot_control_at(hwnd, move_x, move_y);
        if hot != self.hot_rect {
            if let Some(old) = self.hot_rect {
                let _ = InvalidateRect(Some(hwnd), Some(&old), false);
            }
            if let Some(new) = hot {
                let _ = InvalidateRect(Some(hwnd), Some(&new), false);
            }
            self.hot_rect = hot;
        }
        if let Some(last) = self.plot_pan {
            if self.show_analysis {
                let view = (
                    (self.plot_rect.right - self.plot_rect.left) as f32,
                    (self.plot_rect.bottom - self.plot_rect.top) as f32,
                );
                self.plot
                    .pan((move_x - last.0) as f32, (move_y - last.1) as f32, view.0, view.1);
                self.plot_pan = Some((move_x, move_y));
                self.invalidate_plot(hwnd);
            } else {
                self.plot_pan = None;
            }
            return;
        }
        if let Some((row, last_x, last_y)) = self.geq_pan {
            if row >= self.rows.len() {
                self.geq_pan = None;
                let _ = ReleaseCapture();
                return;
            }
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let row_y = self.chrome_height() + self.prefix[row] as i32 - self.offset;
            let height = self.rows[row].height as i32;
            let plot = geq_plot_rect(self.dpi, row_y, height, client.right);
            let mut moved = false;
            if let Some(item) = self.rows[row].item.as_mut() {
                if let Some(FilterEditor::GraphicEq(graphic)) = &mut item.editor {
                    let mut view = GeqView::new(graphic);
                    view.pan(
                        (move_x - last_x) as f32,
                        (move_y - last_y) as f32,
                        &plot,
                    );
                    view.store(graphic);
                    moved = true;
                }
            }
            self.geq_pan = Some((row, move_x, move_y));
            if moved {
                self.invalidate_row(hwnd, row);
            }
            return;
        }
        let over_plot = self.show_analysis && point_in(&self.plot_rect, move_x, move_y);
        let hover_changed = match (self.plot_hover, over_plot) {
            (None, false) => false,
            (Some(_), true) | (None, true) | (Some(_), false) => true,
        };
        if hover_changed {
            self.plot_hover = if over_plot {
                Some((move_x, move_y))
            } else {
                None
            };
            self.invalidate_plot(hwnd);
        } else if over_plot {
            self.plot_hover = Some((move_x, move_y));
            self.invalidate_plot(hwnd);
        }
        if over_plot {
            return;
        }
        if let Some(reorder) = self.reorder.as_mut() {
            let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
            if (y - reorder.start_y).abs() > scale(4, self.dpi) {
                reorder.active = true;
            }
            let chrome = self.chrome_height();
            let content = (y - chrome + self.offset).max(0) as u32;
            let drop = self
                .prefix
                .partition_point(|&start| start <= content)
                .saturating_sub(1)
                .min(self.rows.len());
            self.reorder_drop = Some(drop);
            let _ = InvalidateRect(Some(hwnd), None, false);
            return;
        }
        let Some(drag) = self.drag else {
            return;
        };
        if drag.row >= self.rows.len() {
            self.drag = None;
            let _ = ReleaseCapture();
            return;
        }
        let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
        let mouse_y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        // Treat a press without horizontal movement as a click (which opens
        // the inline number editor) rather than a value drag.
        if drag_value_field(drag.field).is_some()
            && !self.drag_moved
            && (x - drag.start_x).abs() < scale(3, self.dpi)
        {
            return;
        }
        self.drag_moved = true;
        let delta = (x - drag.start_x) as f64;
        let geq_plot = if drag.field == DragField::GeqNode {
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let row_y = self.chrome_height()
                + self.prefix[drag.row] as i32
                - self.offset;
            let height = self.rows[drag.row].height as i32;
            Some(geq_plot_rect(self.dpi, row_y, height, client.right))
        } else {
            None
        };
        let mut changed = false;
        if let Some(item) = self.rows[drag.row].item.as_mut() {
            match (&mut item.editor, drag.field) {
                (Some(FilterEditor::Preamp { db }), DragField::PreampDb) => {
                    *db = (((drag.start_value + delta * 0.1) * 10.0).round() / 10.0)
                        .clamp(-20.0, 20.0);
                    changed = true;
                }
                (Some(FilterEditor::Biquad(biquad)), DragField::BiquadFreq) => {
                    biquad.freq = (drag.start_value * 2.0f64.powf(delta / 50.0))
                        .clamp(1.0, 30000.0);
                    changed = true;
                }
                (Some(FilterEditor::Biquad(biquad)), DragField::BiquadGain) => {
                    biquad.gain = (drag.start_value + delta * 0.1).clamp(-30.0, 30.0);
                    changed = true;
                }
                (Some(FilterEditor::Biquad(biquad)), DragField::BiquadQ) => {
                    biquad.q = (drag.start_value * 2.0f64.powf(delta / 60.0))
                        .clamp(0.1, 100.0);
                    changed = true;
                }
                (Some(FilterEditor::Delay { delay, .. }), DragField::DelayValue) => {
                    *delay = (drag.start_value + delta * 0.1).clamp(0.0, 10000.0);
                    changed = true;
                }
                (Some(FilterEditor::Loudness { ref_level, .. }), DragField::LoudnessRef) => {
                    *ref_level = ((drag.start_value + delta / 8.0).round() as i32)
                        .clamp(-999, 999);
                    changed = true;
                }
                (Some(FilterEditor::Loudness { ref_offset, .. }), DragField::LoudnessOffset) => {
                    *ref_offset = ((drag.start_value + delta / 8.0).round() as i32)
                        .clamp(-999, 999);
                    changed = true;
                }
                (Some(FilterEditor::Loudness { att, .. }), DragField::LoudnessAtt) => {
                    *att = ((drag.start_value + delta * 0.01).clamp(0.0, 1.0) * 100.0)
                        .round()
                        / 100.0;
                    changed = true;
                }
                (Some(FilterEditor::GraphicEq(graphic)), DragField::GeqNode) => {
                    if let Some(plot) = geq_plot {
                        if drag.node < graphic.nodes.len() {
                            let view = GeqView::new(graphic);
                            let db = view.db_at(mouse_y as f32, &plot);
                            graphic.nodes[drag.node].db_gain = (db * 10.0).round() / 10.0;
                            changed = true;
                        }
                    }
                }
                _ => {}
            }
        }
        if changed {
            self.sync_item(hwnd, drag.row);
            // Only the dragged row changed; repainting the whole window on
            // every mouse move caused the visible flicker.
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let row_top =
                self.chrome_height() + self.prefix[drag.row] as i32 - self.offset;
            let row_rect = RECT {
                left: 0,
                top: row_top,
                right: client.right,
                bottom: row_top + self.rows[drag.row].height as i32,
            };
            let _ = InvalidateRect(Some(hwnd), Some(&row_rect), false);
        }
    }

    unsafe fn on_mouse_up(&mut self, hwnd: HWND) {
        if self.plot_pan.take().is_some() {
            let _ = ReleaseCapture();
            self.invalidate_plot(hwnd);
            return;
        }
        if let Some((row, _, _)) = self.geq_pan.take() {
            let _ = ReleaseCapture();
            self.invalidate_row(hwnd, row);
            return;
        }
        if self.reorder.is_some() {
            self.finish_reorder(hwnd);
            return;
        }
        if let Some(drag) = self.drag.take() {
            let _ = ReleaseCapture();
            let open_editor = if self.drag_moved {
                None
            } else {
                drag_value_field(drag.field)
            };
            self.drag_moved = false;
            if self.instant_mode && self.doc_path.is_some() {
                self.save(hwnd);
            } else {
                self.start_analysis();
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            if let Some(field) = open_editor {
                self.begin_value_edit(hwnd, drag.row, field);
            }
        }
    }

    unsafe fn finish_reorder(&mut self, hwnd: HWND) {
        let Some(reorder) = self.reorder.take() else {
            return;
        };
        let drop = self.reorder_drop.take();
        let _ = ReleaseCapture();
        if !reorder.active {
            let _ = InvalidateRect(Some(hwnd), None, false);
            return;
        }
        let Some(drop) = drop else {
            return;
        };
        let mut indices: Vec<usize> = self.selection.iter().copied().collect();
        if indices.is_empty() {
            if let Some(index) = self.selected {
                indices.push(index);
            }
        }
        indices.sort_unstable();
        indices.dedup();
        if indices.is_empty() {
            return;
        }
        let selected: HashSet<usize> = indices.into_iter().collect();
        let mut moving = Vec::new();
        let mut kept = Vec::with_capacity(self.rows.len());
        let mut removed_before = 0;
        for (index, row) in self.rows.drain(..).enumerate() {
            if selected.contains(&index) {
                if index < drop {
                    removed_before += 1;
                }
                moving.push(row);
            } else {
                kept.push(row);
            }
        }
        let count = moving.len();
        if count == 0 {
            self.rows = kept;
            return;
        }
        let insert = drop.saturating_sub(removed_before).min(kept.len());
        kept.splice(insert..insert, moving);
        self.rows = kept;
        self.selection = (insert..insert + count).collect();
        self.selected = self.selection.iter().next().copied();
        self.selection_anchor = self.selected;
        self.dirty = true;
        self.recompute_prefix();
        self.update_title(hwnd);
        self.start_analysis();
        self.autosave(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    fn scroll_test_tick(&mut self) -> bool {
        if self.scroll_test_left == 0 {
            true
        } else {
            self.scroll_test_left -= 1;
            false
        }
    }

    unsafe fn set_dpi(&mut self, dpi: u32) {
        self.dpi = dpi.max(96);
        self.destroy_fonts();
        self.create_fonts();
        self.rebuild_rows();
    }

    unsafe fn paint(&mut self, hwnd: HWND, hdc: HDC) {
        set_hover_pos(self.hover_pos);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        SetBkMode(hdc, TRANSPARENT);
        let chrome = self.chrome_height();

        fill(hdc, 0, 0, client.right, client.bottom, TABLE_BG);
        let table_bottom = client.bottom - self.analysis_height();
        self.paint_chrome(hdc, client.right, chrome);
        let mut table_client = client;
        table_client.bottom = table_bottom;
        self.paint_rows(hwnd, hdc, table_client, chrome);
        if self.show_analysis {
            self.paint_analysis(hdc, client.right, table_bottom, client.bottom);
        }
    }

    unsafe fn paint_chrome(&mut self, hdc: HDC, width: i32, chrome: i32) {
        let old_font = SelectObject(hdc, HGDIOBJ(self.body.0));
        fill(hdc, 0, 0, width, chrome, CHROME_BG);

        if self.show_toolbar {
            let margin = scale(8, self.dpi);
            let button_y = scale(6, self.dpi);
            let button_h = scale(22, self.dpi);
            let mut x = margin;
            for label in [&self.labels.new, &self.labels.open, &self.labels.save] {
                self.paint_button(hdc, x, button_y, scale(54, self.dpi), button_h, label);
                x += scale(58, self.dpi);
            }

            let instant_rect = RECT {
                left: scale(180, self.dpi),
                top: button_y,
                right: scale(260, self.dpi),
                bottom: button_y + button_h,
            };
            if self.is_hot_rect(&instant_rect) {
                fill(
                    hdc,
                    instant_rect.left,
                    instant_rect.top,
                    instant_rect.right,
                    instant_rect.bottom,
                    HOVER,
                );
            }
            let check_size = scale(13, self.dpi);
            let check_y = button_y + (button_h - check_size) / 2;
            fill(hdc, x, check_y, x + check_size, check_y + check_size, WHITE);
            self.frame(hdc, x, check_y, x + check_size, check_y + check_size);
            if self.instant_mode {
                let inset = scale(3, self.dpi).max(1);
                fill(
                    hdc,
                    x + inset,
                    check_y + inset,
                    x + check_size - inset,
                    check_y + check_size - inset,
                    BAR,
                );
            }
            x += check_size + scale(5, self.dpi);
            draw_text(hdc, x, button_y + scale(4, self.dpi), &self.labels.instant, TEXT);
            x += scale(56, self.dpi);

            let combo_w = scale(128, self.dpi);
            let device_text = self
                .selected_device
                .as_ref()
                .map(|device| {
                    format!(
                        "{} - {}",
                        device.get_connection_name(),
                        device.get_device_name()
                    )
                })
                .unwrap_or_else(|| "No device".to_string());
            self.paint_combo(
                hdc,
                x,
                button_y,
                combo_w,
                button_h,
                &wide(&shorten(&device_text, 20)),
            );
            x += combo_w + scale(8, self.dpi);
            let channel_text = if self.channel_names.is_empty() {
                "From device".to_string()
            } else {
                self.channel_names.join(" ")
            };
            self.paint_combo(
                hdc,
                x,
                button_y,
                scale(112, self.dpi),
                button_h,
                &wide(&shorten(&channel_text, 14)),
            );
        }

        let tab_top = self.tab_top();
        fill(hdc, 0, tab_top, width, chrome, CHROME_BG);
        for (index, rect) in self.tab_layout() {
            let tab = &self.tabs[index];
            let title = format!("{}{}", tab.title, if tab.dirty { " *" } else { "" });
            let active = index == self.active_tab;
            let hover = self.is_hot_rect(&rect);
            fill(
                hdc,
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                if active {
                    WHITE
                } else if hover {
                    HOVER
                } else {
                    CHROME_BG
                },
            );
            self.frame(hdc, rect.left, rect.top, rect.right, rect.bottom);
            SelectObject(hdc, HGDIOBJ(if active { self.bold.0 } else { self.body.0 }));
            draw_text(
                hdc,
                rect.left + scale(8, self.dpi),
                tab_top + scale(6, self.dpi),
                &wide(&shorten(&title, 28)),
                if active { TEXT } else { MUTED },
            );
            draw_text(
                hdc,
                rect.right - scale(16, self.dpi),
                tab_top + scale(6, self.dpi),
                &wide("x"),
                MUTED,
            );
        }
        fill(hdc, 0, chrome - 1, width, chrome, BORDER);
        SelectObject(hdc, old_font);
    }

    unsafe fn paint_button(&self, hdc: HDC, x: i32, y: i32, w: i32, h: i32, label: &[u16]) {
        let rect = RECT {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        };
        fill(
            hdc,
            x,
            y,
            x + w,
            y + h,
            if self.is_hot_rect(&rect) { HOVER } else { WHITE },
        );
        self.frame(hdc, x, y, x + w, y + h);
        draw_text(hdc, x + scale(8, self.dpi), y + scale(4, self.dpi), label, TEXT);
    }

    unsafe fn paint_combo(&self, hdc: HDC, x: i32, y: i32, w: i32, h: i32, label: &[u16]) {
        let rect = RECT {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        };
        fill(
            hdc,
            x,
            y,
            x + w,
            y + h,
            if self.is_hot_rect(&rect) { HOVER } else { WHITE },
        );
        self.frame(hdc, x, y, x + w, y + h);
        draw_text(hdc, x + scale(6, self.dpi), y + scale(4, self.dpi), label, TEXT);
        let arrow_x = x + w - scale(14, self.dpi);
        let arrow_y = y + h / 2 - scale(2, self.dpi);
        for row in 0..scale(4, self.dpi) {
            let half = scale(4, self.dpi) - row;
            fill(
                hdc,
                arrow_x + half,
                arrow_y + row,
                arrow_x + scale(8, self.dpi) - half,
                arrow_y + row + 1,
                MUTED,
            );
        }
    }

    unsafe fn frame(&self, hdc: HDC, left: i32, top: i32, right: i32, bottom: i32) {
        fill(hdc, left, top, right, top + 1, BORDER);
        fill(hdc, left, bottom - 1, right, bottom, BORDER);
        fill(hdc, left, top, left + 1, bottom, BORDER);
        fill(hdc, right - 1, top, right, bottom, BORDER);
    }

    unsafe fn paint_analysis(&mut self, hdc: HDC, width: i32, top: i32, bottom: i32) {
        if bottom <= top {
            return;
        }
        fill(hdc, 0, top, width, bottom, PLOT_BG);
        fill(hdc, 0, top, width, top + 1, BORDER);

        let left_width = scale(250, self.dpi);
        let margin = scale(12, self.dpi);
        SelectObject(hdc, HGDIOBJ(self.bold.0));
        draw_text(hdc, margin, top + scale(8, self.dpi), &trw("Analysis"), TEXT);
        SelectObject(hdc, HGDIOBJ(self.body.0));

        let mut line_y = top + scale(30, self.dpi);
        self.analysis_hits.clear();
        match &self.analysis_result {
            Some(result) => {
                let peak_color = if result.peak_gain > 0.0 {
                    COLORREF(0x0020_20C0)
                } else {
                    TEXT
                };
                for (label, value_color) in [
                    (format!("{} {:.2} dB", i18n::tr("Peak gain:"), result.peak_gain), peak_color),
                    (
                        format!(
                            "{} {} {} ({:.2} ms)",
                            i18n::tr("Latency:"),
                            result.latency,
                            i18n::tr(" samples"),
                            result.latency as f64 * 1000.0 / 48000.0
                        ),
                        TEXT,
                    ),
                    (format!("{} {:.1} ms", i18n::tr("Init:"), result.initialization_time), TEXT),
                    (format!("{} {:.2} %", i18n::tr("CPU usage:"), result.cpu_usage()), TEXT),
                    (
                        format!("{} {}", i18n::tr("Resolution:"), self.analysis_effective_resolution),
                        MUTED,
                    ),
                    (
                        format!("{} {}", i18n::tr("Channels:"), shorten(&self.channel_names.join(" "), 28)),
                        MUTED,
                    ),
                ] {
                    draw_text(hdc, margin, line_y, &wide(&label), value_color);
                    line_y += scale(18, self.dpi);
                }
            }
            None => {
                draw_text(hdc, margin, line_y, &trw("Analyzing..."), MUTED);
            }
        }

        let start_value = if self.analysis_start_from == 1 {
            i18n::tr("Current file")
        } else {
            "config.txt".to_string()
        };
        let channel_value = self
            .channel_names
            .get(self.analysis_channel_index)
            .cloned()
            .unwrap_or_default();
        let controls = [
            (AnalysisSetting::StartFrom, i18n::tr("Start from"), start_value),
            (AnalysisSetting::Channel, i18n::tr("Channel"), channel_value),
            (
                AnalysisSetting::Resolution,
                i18n::tr("Resolution"),
                self.analysis_resolution.to_string(),
            ),
        ];
        let controls_top = line_y.max(top + scale(196, self.dpi)) + scale(6, self.dpi);
        let mut controls_y = controls_top;
        for (kind, label, value) in controls {
            let rect = RECT {
                left: margin,
                top: controls_y,
                right: left_width - scale(10, self.dpi),
                bottom: controls_y + scale(20, self.dpi),
            };
            draw_control(
                hdc,
                self.dpi,
                &rect,
                &format!("{label}: {value}"),
                true,
                None,
            );
            self.analysis_hits.push((kind, rect));
            controls_y += scale(24, self.dpi);
        }
        if self.analysis_effective_resolution != self.analysis_resolution {
            draw_text(
                hdc,
                margin,
                controls_y + scale(2, self.dpi),
                &wide(&i18n::tr_arg(
                    "Resolution reduced to %0 to limit memory",
                    &[&self.analysis_effective_resolution.to_string()],
                )),
                WARNING,
            );
        }

        let plot_left = left_width;
        let plot_right = width - scale(14, self.dpi);
        let plot_top = top + scale(22, self.dpi);
        let plot_bottom = bottom - scale(20, self.dpi);
        if plot_right <= plot_left + scale(20, self.dpi) || plot_bottom <= plot_top + scale(20, self.dpi) {
            return;
        }
        let plot_width = (plot_right - plot_left) as f32;
        let plot_height = (plot_bottom - plot_top) as f32;
        self.plot_rect = RECT {
            left: plot_left,
            top: plot_top,
            right: plot_right,
            bottom: plot_bottom,
        };
        if self.plot_fitted_for != (plot_width as i32, plot_height as i32) {
            // Always fit the audible range (20 Hz..20 kHz) to the current
            // plot width, so resizing the window scales the chart instead of
            // squeezing it (wider window => wider, filled plot).
            self.plot.fit(plot_width, plot_height);
            self.plot.fit_frequency_range(20.0, 20000.0, plot_width);
            self.plot.default_scroll();
            self.plot.pan(0.0, 0.0, plot_width, plot_height);
            self.plot_fitted_for = (plot_width as i32, plot_height as i32);
        }
        let plot = &self.plot;

        for db in [-100i32, -80, -60, -40, -20, 0, 20, 40, 60, 80, 100] {
            let Some((_, scene_y)) = plot.pos_for(1000.0, db as f64) else {
                continue;
            };
            let y = plot_top + scene_y as i32;
            if y < plot_top || y >= plot_bottom {
                continue;
            }
            fill(hdc, plot_left, y, plot_right, y + 1, if db == 0 { BORDER } else { GRID });
            draw_text(
                hdc,
                plot_left + scale(2, self.dpi),
                y + scale(1, self.dpi),
                &wide(&format!("{db}")),
                MUTED,
            );
        }
        for step in [1u32, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000] {
            let x_scene = plot.hz_to_x(step as f64);
            if x_scene < 0.0 {
                continue;
            }
            let x = plot_left + x_scene as i32 - plot.scroll_x as i32;
            if x < plot_left || x >= plot_right {
                continue;
            }
            fill(hdc, x, plot_top, x + 1, plot_bottom, GRID);
            draw_text(
                hdc,
                x + scale(2, self.dpi),
                plot_bottom - scale(14, self.dpi),
                &wide(&format!("{step:.0}")),
                MUTED,
            );
        }

        // Crosshair and readout under the cursor.
        if let Some((hover_x, hover_y)) = self.plot_hover {
            if hover_x >= plot_left && hover_x < plot_right && hover_y >= plot_top && hover_y < plot_bottom {
                fill(hdc, hover_x, plot_top, hover_x + 1, plot_bottom, BORDER);
                fill(hdc, plot_left, hover_y, plot_right, hover_y + 1, BORDER);
                let hz = plot.freq_at((hover_x - plot_left) as f32);
                let db = plot.db_at((hover_y - plot_top) as f32);
                let label = if hz >= 1000.0 {
                    format!("{:.2} kHz, {:.1} dB", hz / 1000.0, db)
                } else {
                    format!("{:.0} Hz, {:.1} dB", hz, db)
                };
                let text = wide(&label);
                let text_width = scale(110, self.dpi);
                let label_x = (hover_x + scale(6, self.dpi)).min(plot_right - text_width);
                let label_y = plot_top + scale(2, self.dpi);
                fill(
                    hdc,
                    label_x - scale(2, self.dpi),
                    label_y - scale(1, self.dpi),
                    label_x + text_width,
                    label_y + scale(16, self.dpi),
                    PLOT_BG,
                );
                draw_text(hdc, label_x, label_y, &text, TEXT);
            }
        }

        let Some(result) = &self.analysis_result else {
            return;
        };
        let sample_rate = result.freq_data_sample_rate.max(1) as f64;
        let bins = result.freq_data.len().max(1);
        let saved_dc = SaveDC(hdc);
        let _ = IntersectClipRect(hdc, plot_left, plot_top, plot_right, plot_bottom);
        // 0 dB reference in view coordinates (pos_for already subtracts the
        // scroll offset, unlike the raw scene-space db_to_y).
        let zero_y = plot
            .pos_for(1000.0, 0.0)
            .map(|(_, view_y)| plot_top + view_y as i32)
            .unwrap_or(plot_bottom);
        let mut runs: Vec<(COLORREF, Vec<PointF>)> = Vec::new();
        let mut run_points: Vec<PointF> = Vec::new();
        let mut run_positive = false;
        let mut path_open = false;
        for px in plot_left..plot_right {
            let hz = plot.freq_at((px - plot_left) as f32);
            if hz <= 0.0 {
                continue;
            }
            // Linear interpolation between FFT bins: nearest-bin lookup made
            // the curve piecewise constant ("staircase") at lower resolutions.
            let position = (hz / sample_rate) * result.freq_data_length as f64;
            let lower = position.floor().max(0.0) as usize;
            let upper = (lower + 1).min(bins - 1);
            let fraction = (position - lower as f64).clamp(0.0, 1.0);
            let magnitude_at = |index: usize| {
                let value = result.freq_data[index.min(bins - 1)];
                (value.re * value.re + value.im * value.im).sqrt()
            };
            let magnitude =
                magnitude_at(lower) * (1.0 - fraction) + magnitude_at(upper) * fraction;
            let db = (20.0 * magnitude.log10()).clamp(-100.0, 100.0);
            let Some((_, scene_y)) = plot.pos_for(hz, db) else {
                if path_open && run_points.len() >= 2 {
                    runs.push((
                        if run_positive { CURVE_POS } else { CURVE },
                        std::mem::take(&mut run_points),
                    ));
                }
                run_points.clear();
                path_open = false;
                continue;
            };
            // Keep the sub-pixel Y for the curve; rounding to integer pixels
            // here produced the visible staircase even with anti-aliasing.
            let y_subpixel = plot_top as f32 + scene_y;
            let y = y_subpixel as i32;
            let positive = db > 0.0;
            if positive && y < zero_y {
                fill(hdc, px, y, px + 1, zero_y, FILL_POS);
            }
            if path_open && positive != run_positive {
                if run_points.len() >= 2 {
                    runs.push((
                        if run_positive { CURVE_POS } else { CURVE },
                        std::mem::take(&mut run_points),
                    ));
                }
                run_points.clear();
            }
            run_positive = positive;
            run_points.push(PointF {
                X: px as f32 + 0.5,
                Y: y_subpixel.clamp(
                    plot_top as f32,
                    plot_bottom.saturating_sub(1).max(plot_top) as f32,
                ),
            });
            path_open = true;
        }
        if path_open && run_points.len() >= 2 {
            runs.push((if run_positive { CURVE_POS } else { CURVE }, run_points));
        }
        draw_aa_runs(hdc, self.paint_offset, &runs, self.plot_rect);
        let _ = RestoreDC(hdc, saved_dc);
    }

    unsafe fn paint_rows(&mut self, hwnd: HWND, hdc: HDC, client: RECT, chrome: i32) {
        if client.bottom <= chrome || self.total == 0 {
            return;
        }
        let saved = SaveDC(hdc);
        let _ = IntersectClipRect(hdc, 0, chrome, client.right, client.bottom);

        let first = self
            .prefix
            .partition_point(|&start| start <= self.offset as u32)
            .saturating_sub(1);
        let mut index = first;
        let mut y = chrome + self.prefix[first] as i32 - self.offset;

        let while_guard = self.rows.len() + 1;
        let mut guard = 0;
        while index < self.rows.len() && y < client.bottom && guard < while_guard {
            guard += 1;
            self.ensure_item(index);
            let row = &self.rows[index];
            let height = row.height as i32;
            let expanded = false;
            if y + height > chrome {
                let background = if self.selection.contains(&index) {
                    SELECTED
                } else if expanded {
                    EXPANDED_BG
                } else {
                    WHITE
                };
                fill(
                    hdc,
                    0,
                    y.max(chrome),
                    client.right,
                    (y + height).min(client.bottom),
                    background,
                );

                let mut number = [0u16; 24];
                let number_len = format_row_number(&mut number, index);
                SelectObject(hdc, HGDIOBJ(self.small.0));
                draw_text(
                    hdc,
                    scale(6, self.dpi),
                    y + scale(5, self.dpi),
                    &number[..number_len],
                    MUTED,
                );

                let box_size = scale(12, self.dpi);
                let box_x = scale(20, self.dpi);
                let box_y = y + (height - box_size) / 2;
                let box_rect = RECT {
                    left: box_x,
                    top: box_y,
                    right: box_x + box_size,
                    bottom: box_y + box_size,
                };
                let box_bg = if self.is_hot_rect(&box_rect) {
                    HOVER
                } else {
                    WHITE
                };
                fill(hdc, box_x, box_y, box_x + box_size, box_y + box_size, box_bg);
                self.frame(hdc, box_x, box_y, box_x + box_size, box_y + box_size);
                if row.enabled {
                    let inset = scale(3, self.dpi).max(1);
                    fill(
                        hdc,
                        box_x + inset,
                        box_y + inset,
                        box_x + box_size - inset,
                        box_y + box_size - inset,
                        BAR,
                    );
                }

                let mut painted = false;
                if let Some(item) = &row.item {
                    if let Some(editor) = &item.editor {
                        match editor {
                            FilterEditor::Preamp { db } => {
                                paint_preamp_row(hdc, self.dpi, y, *db, row.enabled);
                                painted = true;
                            }
                            FilterEditor::Biquad(biquad) => {
                                paint_biquad_row(hdc, self.dpi, y, biquad, row.enabled);
                                painted = true;
                            }
                            FilterEditor::Delay { delay, is_ms } => {
                                paint_delay_row(hdc, self.dpi, y, *delay, *is_ms, row.enabled);
                                painted = true;
                            }
                            FilterEditor::Stage {
                                pre_mix,
                                post_mix,
                                capture,
                            } => {
                                paint_stage_row(
                                    hdc, self.dpi, y, *pre_mix, *post_mix, *capture, row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::Include { path } => {
                                paint_path_row(
                                    hdc, self.dpi, y, "Include", path, "Select file", row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::Convolution { path, .. } => {
                                paint_path_row(
                                    hdc,
                                    self.dpi,
                                    y,
                                    "Convolution",
                                    path,
                                    "Select file",
                                    row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::Vst(vst) => {
                                paint_vst_row(hdc, self.dpi, y, vst, row.enabled);
                                painted = true;
                            }
                            FilterEditor::Loudness {
                                ref_level,
                                ref_offset,
                                att,
                                ..
                            } => {
                                paint_loudness_row(
                                    hdc,
                                    self.dpi,
                                    y,
                                    *ref_level,
                                    *ref_offset,
                                    *att,
                                    row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::Channel { selected, .. } => {
                                paint_summary_row(
                                    hdc,
                                    self.dpi,
                                    y,
                                    "Channel",
                                    &selected.join(" "),
                                    row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::Device { pattern, .. } => {
                                paint_summary_row(
                                    hdc, self.dpi, y, "Device", pattern, row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::GraphicEq(graphic) => {
                                paint_graphic_eq_row(
                                    hdc,
                                    self.paint_offset,
                                    self.dpi,
                                    y,
                                    height,
                                    client.right,
                                    graphic,
                                    row.geq_table_top,
                                    row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::Copy { .. } => {
                                let parameters = row
                                    .line
                                    .split_once(':')
                                    .map(|(_, value)| value.trim())
                                    .unwrap_or("");
                                paint_copy_row(hdc, self.dpi, y, parameters, row.enabled);
                                painted = true;
                            }
                        }
                    }
                }
                if !painted {
                    let mut text = [0u16; 1024];
                    let text_len = format_row_text(&mut text, row);
                    SelectObject(hdc, HGDIOBJ(self.body.0));
                    draw_text(
                        hdc,
                        scale(42, self.dpi),
                        y + scale(4, self.dpi),
                        &text[..text_len],
                        if row.enabled { TEXT } else { MUTED },
                    );
                }
                if let Some(hotkey) = row.hotkey.as_ref() {
                    SelectObject(hdc, HGDIOBJ(self.small.0));
                    draw_text(
                        hdc,
                        client.right - scale(170, self.dpi),
                        y + scale(5, self.dpi),
                        &wide(&hotkey.to_string_qt()),
                        MUTED,
                    );
                }

                if expanded {
                    let bar_top = y + scale(28, self.dpi);
                    let bar_x = scale(46, self.dpi);
                    let bar_w = scale(7, self.dpi);
                    let gap = scale(3, self.dpi);
                    for bar in 0..15usize {
                        let bar_h = scale(4 + ((index + bar * 7) % 30) as i32, self.dpi);
                        let left = bar_x + bar as i32 * (bar_w + gap);
                        let bottom = (bar_top + scale(32, self.dpi)).min(y + height - scale(4, self.dpi));
                        fill(hdc, left, bottom - bar_h, left + bar_w, bottom, BAR);
                    }
                }

                fill(
                    hdc,
                    0,
                    (y + height - 1).max(chrome),
                    client.right,
                    (y + height).min(client.bottom),
                    BORDER,
                );
            }
            y += height;
            index += 1;
        }
        if self.reorder.as_ref().map(|state| state.active).unwrap_or(false) {
            if let Some(drop) = self.reorder_drop {
                let drop = drop.min(self.rows.len());
                let line_y = (chrome + self.prefix[drop] as i32 - self.offset)
                    .clamp(chrome, client.bottom.saturating_sub(1));
                fill(hdc, 0, line_y, client.right, line_y + 2, CURVE_POS);
            }
        }
        let _ = RestoreDC(hdc, saved);
        let _ = hwnd;
    }
}

impl Drop for App {
    fn drop(&mut self) {
        unsafe { self.destroy_fonts() };
    }
}

struct PromptState {
    edit: HWND,
    initial: String,
    result: Rc<RefCell<Option<String>>>,
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
            let width = (client.right - scale(24, 96)).max(scale(120, 96));
            let initial = wide_z(&state.initial);
            let edit = CreateWindowExW(
                WINDOW_EX_STYLE(0),
                w!("EDIT"),
                PCWSTR(initial.as_ptr()),
                WINDOW_STYLE(
                    WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32,
                ),
                scale(12, 96),
                scale(36, 96),
                width,
                scale(24, 96),
                Some(hwnd),
                None,
                None,
                None,
            );
            if let Ok(edit) = edit {
                SendMessageW(
                    edit,
                    WM_SETFONT,
                    Some(WPARAM(GetStockObject(DEFAULT_GUI_FONT).0 as usize)),
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
            SelectObject(hdc, GetStockObject(DEFAULT_GUI_FONT));
            draw_text(
                hdc,
                scale(12, 96),
                scale(12, 96),
                &wide("Enter a key sequence, e.g. Ctrl+Alt+X:"),
                TEXT,
            );
            draw_text(
                hdc,
                scale(12, 96),
                client.bottom - scale(22, 96),
                &wide("Enter = OK, Esc = Cancel"),
                MUTED,
            );
            let _ = EndPaint(hwnd, &paint);
            LRESULT(0)
        }
        WM_APP_PROMPT_ACCEPT => {
            if let Some(state) = (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut PromptState).as_mut()
            {
                let mut buffer = [0u16; 512];
                let length = GetWindowTextW(state.edit, &mut buffer).max(0) as usize;
                *state.result.borrow_mut() = Some(
                    String::from_utf16_lossy(&buffer[..length.min(buffer.len())])
                        .trim()
                        .to_string(),
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
            if !pointer.is_null() {
                drop(Box::from_raw(pointer));
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

unsafe fn prompt_text(parent: HWND, title: &str, initial: &str) -> Option<String> {
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
    let mut parent_rect = RECT::default();
    let _ = GetWindowRect(parent, &mut parent_rect);
    let width = 520;
    let height = 130;
    let x = parent_rect.left + ((parent_rect.right - parent_rect.left) - width) / 2;
    let y = parent_rect.top + ((parent_rect.bottom - parent_rect.top) - height) / 2;
    let result = Rc::new(RefCell::new(None));
    let state = Box::new(PromptState {
        edit: HWND::default(),
        initial: initial.to_string(),
        result: result.clone(),
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
                let message = wide_z(
                    "This key combination is reserved by Windows. Please choose another one.",
                );
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
                scale(12, 96),
                scale(36, 96),
                (client.right - scale(24, 96)).max(scale(120, 96)),
                scale(24, 96),
                Some(hwnd),
                None,
                None,
                None,
            );
            if let Ok(edit) = edit {
                SendMessageW(
                    edit,
                    WM_SETFONT,
                    Some(WPARAM(GetStockObject(DEFAULT_GUI_FONT).0 as usize)),
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
                    scale(72, 96),
                    scale(80, 96),
                    scale(26, 96),
                    Some(hwnd),
                    Some(HMENU(id as *mut core::ffi::c_void)),
                    None,
                    None,
                );
            };
            make_button("OK", client.right - scale(190, 96), 1, 0x0000_0001);
            make_button("Cancel", client.right - scale(100, 96), 2, 0);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut paint);
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            fill(hdc, 0, 0, client.right, client.bottom, WHITE);
            SelectObject(hdc, GetStockObject(DEFAULT_GUI_FONT));
            draw_text(
                hdc,
                scale(12, 96),
                scale(12, 96),
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
unsafe fn prompt_hotkey(
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
    let mut parent_rect = RECT::default();
    let _ = GetWindowRect(parent, &mut parent_rect);
    let width = 520;
    let height = 150;
    let x = parent_rect.left + ((parent_rect.right - parent_rect.left) - width) / 2;
    let y = parent_rect.top + ((parent_rect.bottom - parent_rect.top) - height) / 2;
    let result = Rc::new(RefCell::new(None));
    let state = Box::new(HotkeyPromptState {
        edit: HWND::default(),
        sequence: initial,
        result: result.clone(),
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

unsafe extern "system" fn value_edit_proc(
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

unsafe extern "system" fn edit_proc(    hwnd: HWND,
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

unsafe fn app(hwnd: HWND) -> Option<&'static mut App> {
    let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
    pointer.as_mut()
}

unsafe fn launch_sibling(name: &str) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(directory) = exe.parent() else {
        return;
    };
    let mut candidates = vec![directory.join(name)];
    if let Some(parent) = directory.parent() {
        candidates.push(parent.join(name));
    }
    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }
        let path = wide_z(&candidate.to_string_lossy());
        for operation in [w!("open"), w!("runas")] {
            let result = ShellExecuteW(
                None,
                operation,
                PCWSTR(path.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            );
            if result.0 as isize > 32 {
                return;
            }
        }
    }
}

unsafe fn startup_checks(hwnd: HWND) {
    if SKIP_CHECKS.load(Ordering::Relaxed) || SCROLL_TEST.load(Ordering::Relaxed) {
        return;
    }
    // Without the registered configuration path the APO engine never loads a
    // configuration, so gain/EQ edits cannot affect audio.
    let config_registered = eapo_common::registry_helper::read_value(
        eapo_common::APP_REGPATH,
        "ConfigPath",
    )
    .map(|value| !value.trim().is_empty())
    .unwrap_or(false);
    if !config_registered {
        let text = wide_z(
            "Equalizer APO is not installed correctly: the configuration path is not registered, so gain and filter changes will not affect audio.
Do you want to start the installer (Setup.exe) to repair the installation?",
        );
        let answer = MessageBoxW(
            Some(hwnd),
            PCWSTR(text.as_ptr()),
            w!("Equalizer APO"),
            MB_YESNO | MB_ICONWARNING,
        );
        if answer.0 == IDYES.0 {
            launch_sibling("Setup.exe");
        }
        return;
    }
    let mut message: Option<String> = None;
    if !DeviceAPOInfo::check_apo_registration(false)
        || !DeviceAPOInfo::check_protected_audio_dg(false).unwrap_or(false)
    {
        message = Some(
            "A registry value required by Equalizer APO is not set correctly.\nDo you want to open the Device Selector to fix it?"
                .to_string(),
        );
    } else if let Ok(devices) = DeviceAPOInfo::load_all_infos(false) {
        if let Some(default) = devices.iter().find(|device| device.is_default_device()) {
            if !default.is_installed() {
                message = Some(
                    "Equalizer APO is not installed to the default playback device.\nDo you want to open the Device Selector?"
                        .to_string(),
                );
            } else if default.is_enhancements_disabled() {
                message = Some(
                    "Audio enhancements are disabled for the default playback device.\nDo you want to open the Device Selector?"
                        .to_string(),
                );
            }
        }
    }
    if let Some(message) = message {
        let text = wide_z(&i18n::tr(&message));
        let answer = MessageBoxW(
            Some(hwnd),
            PCWSTR(text.as_ptr()),
            w!("Equalizer APO"),
            MB_YESNO | MB_ICONWARNING,
        );
        if answer.0 == IDYES.0 {
            launch_sibling("DeviceSelector.exe");
        }
    }
}

unsafe extern "system" fn main_window_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let dpi = GetDpiForWindow(hwnd).max(96);
            let state = Box::new(App::new(dpi));
            SetWindowLongPtrW(hwnd, GWLP_USERDATA, Box::into_raw(state) as isize);
            if let Some(state) = app(hwnd) {
                state.update_title(hwnd);
                SetTimer(Some(hwnd), TIMER_ANALYSIS, 100, None);
            }
            if SCROLL_TEST.load(Ordering::Relaxed) {
                SetTimer(Some(hwnd), TIMER_SCROLL_TEST, 10, None);
            }
            LRESULT(0)
        }
        WM_SIZE => {
            if let Some(state) = app(hwnd) {
                state.update_scrollbar(hwnd);
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_PAINT => {
            let mut paint = PAINTSTRUCT::default();
            let hdc = BeginPaint(hwnd, &mut paint);
            if let Some(state) = app(hwnd) {
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
            if let Some(state) = app(hwnd) {
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
            if let Some(state) = app(hwnd) {
                state.commit_edit(hwnd, true);
                state.commit_value_edit(hwnd, true);
                state.on_vscroll(hwnd, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_MOUSEWHEEL => {
            if let Some(state) = app(hwnd) {
                state.commit_edit(hwnd, true);
                state.on_wheel(hwnd, wparam, lparam);
            }
            LRESULT(0)
        }
        WM_KEYDOWN => {
            if let Some(state) = app(hwnd) {
                state.on_key(hwnd, wparam);
            }
            LRESULT(0)
        }
        WM_LBUTTONDOWN => {
            let _ = SetFocus(Some(hwnd));
            if let Some(state) = app(hwnd) {
                state.on_click(hwnd, lparam);
            }
            LRESULT(0)
        }
        WM_MOUSEMOVE => {
            if let Some(state) = app(hwnd) {
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
            if let Some(state) = app(hwnd) {
                state.hover_pos = None;
                if let Some(rect) = state.hot_rect.take() {
                    let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
                }
            }
            LRESULT(0)
        }
        WM_LBUTTONUP => {
            if let Some(state) = app(hwnd) {
                state.on_mouse_up(hwnd);
            }
            LRESULT(0)
        }
        WM_LBUTTONDBLCLK => {
            if let Some(state) = app(hwnd) {
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
            if let Some(state) = app(hwnd) {
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
            if let Some(state) = app(hwnd) {
                if state.plot_pan.take().is_some() {
                    let _ = ReleaseCapture();
                    state.invalidate_plot(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_APP_COMMIT_EDIT => {
            if let Some(state) = app(hwnd) {
                state.commit_edit(hwnd, true);
            }
            LRESULT(0)
        }
        WM_APP_CANCEL_EDIT => {
            if let Some(state) = app(hwnd) {
                state.commit_edit(hwnd, false);
            }
            LRESULT(0)
        }
        WM_APP_COMMIT_VALUE => {
            if let Some(state) = app(hwnd) {
                state.commit_value_edit(hwnd, true);
            }
            LRESULT(0)
        }
        WM_APP_CANCEL_VALUE => {
            if let Some(state) = app(hwnd) {
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
            if let Some(state) = app(hwnd) {
                state.commit_edit(hwnd, true);
                state.set_dpi(dpi);
                state.update_scrollbar(hwnd);
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_TIMER => {
            if wparam.0 == TIMER_SCROLL_TEST {
                let finished = app(hwnd)
                    .map(|state| state.scroll_test_tick())
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
            } else if wparam.0 == TIMER_ANALYSIS {
                if let Some(state) = app(hwnd) {
                    state.poll_analysis(hwnd);
                    let ids = state.hotkeys.poll();
                    for filter_id in ids {
                        state.handle_hotkey(hwnd, &filter_id);
                    }
                    state.poll_volume(hwnd);
                }
            }
            LRESULT(0)
        }
        WM_COMMAND => {
            let id = (wparam.0 & 0xFFFF) as usize;
            if let Some(state) = app(hwnd) {
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
            if let Some(state) = app(hwnd) {
                state.commit_edit(hwnd, true);
                if state.dirty {
                    let answer = MessageBoxW(
                        Some(hwnd),
                        w!("Save changes before closing?"),
                        w!("Equalizer APO"),
                        MB_YESNOCANCEL | MB_ICONWARNING,
                    );
                    if answer.0 == IDCANCEL.0 {
                        allow = false;
                    } else if answer.0 == IDYES.0 {
                        allow = state.save(hwnd);
                    }
                }
            }
            if allow {
                if let Some(state) = app(hwnd) {
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
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut App;
            if !pointer.is_null() {
                drop(Box::from_raw(pointer));
                let _ = SetWindowLongPtrW(hwnd, GWLP_USERDATA, 0);
            }
            DefWindowProcW(hwnd, msg, wparam, lparam)
        }
        _ => DefWindowProcW(hwnd, msg, wparam, lparam),
    }
}

fn main() -> windows::core::Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let synthetic = args.iter().any(|argument| argument == "--scroll-test");
    SCROLL_TEST.store(synthetic, Ordering::Relaxed);
    SKIP_CHECKS.store(
        args.iter().any(|argument| argument == "--no-checks"),
        Ordering::Relaxed,
    );
    let explicit = args
        .iter()
        .find(|argument| !argument.starts_with("--"))
        .map(PathBuf::from);
    let config_path = explicit.or_else(|| {
        // Only open the default config when it actually exists; otherwise
        // start with an empty document instead of a "cannot read" row.
        let candidate = settings::config_dir_from_registry().join("config.txt");
        candidate.exists().then_some(candidate)
    });
    let _ = CONFIG_PATH.set(config_path);
    let window_prefs = GlobalPreferences::load();
    let language = window_prefs
        .language
        .as_deref()
        .map(i18n::from_code)
        .unwrap_or_else(i18n::system_language);
    i18n::set_language(language);
    let (window_width, window_height) = window_prefs
        .window_size
        .map(|(width, height)| ((width as i32).max(480), (height as i32).max(320)))
        .unwrap_or((1100, 800));
    let (window_x, window_y) = window_prefs
        .window_pos
        .map(|(x, y)| (x as i32, y as i32))
        .filter(|(x, y)| {
            // Discard saved positions outside every monitor (minimized or
            // removed displays) so the window cannot open off-screen.
            let rect = RECT {
                left: *x,
                top: *y,
                right: *x + window_width,
                bottom: *y + window_height,
            };
            unsafe { !MonitorFromRect(&rect, MONITOR_DEFAULTTONULL).is_invalid() }
        })
        .unwrap_or((CW_USEDEFAULT, CW_USEDEFAULT));
    let recent_files = window_prefs.recent_files.clone();
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
        let module = GetModuleHandleW(None)?;
        let instance = HINSTANCE(module.0);
        let cursor = LoadCursorW(None, IDC_ARROW)?;
        let class_name = wide_z("EapoNativeEditorMain");
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(main_window_proc),
            hInstance: instance,
            hCursor: cursor,
            lpszClassName: PCWSTR(class_name.as_ptr()),
            ..Default::default()
        };
        if RegisterClassW(&class) == 0 {
            return Err(windows::core::Error::from_win32());
        }
        let menu = CreateMenu()?;
        // Menu labels must outlive the menu handles attached to the window.
        let mut menu_texts: Vec<Vec<u16>> = Vec::new();
        let file = CreatePopupMenu()?;
        let _ = AppendMenuW(file, MF_STRING, 101, menu_text(&mut menu_texts, "New"));
        let _ = AppendMenuW(file, MF_STRING, 102, menu_text(&mut menu_texts, "Open..."));
        let _ = AppendMenuW(file, MF_STRING, ID_SAVE, menu_text(&mut menu_texts, "Save"));
        let _ = AppendMenuW(file, MF_STRING, 104, menu_text(&mut menu_texts, "Save as..."));
        if !recent_files.is_empty() {
            let _ = AppendMenuW(file, MF_SEPARATOR, 0, w!(""));
            for (index, path) in recent_files.iter().take(10).enumerate() {
                let _ = AppendMenuW(
                    file,
                    MF_STRING,
                    500 + index,
                    menu_raw(&mut menu_texts, path),
                );
            }
        }
        let _ = AppendMenuW(file, MF_SEPARATOR, 0, w!(""));
        let _ = AppendMenuW(file, MF_STRING, ID_EXIT, menu_text(&mut menu_texts, "Exit"));
        let _ = AppendMenuW(menu, MF_POPUP, file.0 as usize, menu_text(&mut menu_texts, "File"));
        let edit = CreatePopupMenu()?;
        let _ = AppendMenuW(edit, MF_STRING, 111, menu_text(&mut menu_texts, "Cut"));
        let _ = AppendMenuW(edit, MF_STRING, 112, menu_text(&mut menu_texts, "Copy"));
        let _ = AppendMenuW(edit, MF_STRING, 113, menu_text(&mut menu_texts, "Paste"));
        let _ = AppendMenuW(edit, MF_STRING, ID_DELETE, menu_text(&mut menu_texts, "Delete"));
        let _ = AppendMenuW(edit, MF_SEPARATOR, 0, w!(""));
        let _ = AppendMenuW(edit, MF_STRING, 115, menu_text(&mut menu_texts, "Select all"));
        let _ = AppendMenuW(edit, MF_SEPARATOR, 0, w!(""));
        let _ = AppendMenuW(edit, MF_STRING, 116, menu_text(&mut menu_texts, "Set filter hotkey..."));
        let _ = AppendMenuW(edit, MF_STRING, 117, menu_text(&mut menu_texts, "Clear filter hotkey"));
        let _ = AppendMenuW(menu, MF_POPUP, edit.0 as usize, menu_text(&mut menu_texts, "Edit"));
        let view = CreatePopupMenu()?;
        let check_flag = |on: bool| if on { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(
            view,
            MF_STRING | check_flag(window_prefs.show_toolbar),
            123,
            menu_text(&mut menu_texts, "Toolbar"),
        );
        let _ = AppendMenuW(
            view,
            MF_STRING
                | check_flag(!SCROLL_TEST.load(Ordering::Relaxed) && window_prefs.show_analysis),
            122,
            menu_text(&mut menu_texts, "Analysis panel"),
        );
        let _ = AppendMenuW(menu, MF_POPUP, view.0 as usize, menu_text(&mut menu_texts, "View"));
        let settings_menu = CreatePopupMenu()?;
        let language_menu = CreatePopupMenu()?;
        for (index, language) in [
            ("en", "English"),
            ("de", "Deutsch"),
            ("fr", "Français"),
            ("zh", "中文"),
        ]
        .iter()
        .enumerate()
        {
            let _ = AppendMenuW(
                language_menu,
                MF_STRING,
                860 + index,
                menu_raw(&mut menu_texts, language.1),
            );
        }
        let _ = AppendMenuW(
            settings_menu,
            MF_POPUP,
            language_menu.0 as usize,
            menu_text(&mut menu_texts, "Language"),
        );
        let _ = AppendMenuW(settings_menu, MF_SEPARATOR, 0, w!(""));
        let _ = AppendMenuW(
            settings_menu,
            MF_STRING,
            124,
            menu_text(&mut menu_texts, "Reset all global preferences"),
        );
        let _ = AppendMenuW(
            settings_menu,
            MF_STRING,
            125,
            menu_text(&mut menu_texts, "Reset all file-specific preferences"),
        );
        let _ = AppendMenuW(
            menu,
            MF_POPUP,
            settings_menu.0 as usize,
            menu_text(&mut menu_texts, "Settings"),
        );

        let title = wide_z("Equalizer APO Configuration Editor");
        let hwnd = CreateWindowExW(
            WINDOW_EX_STYLE(0),
            PCWSTR(class_name.as_ptr()),
            PCWSTR(title.as_ptr()),
            WS_OVERLAPPEDWINDOW | WS_VISIBLE | WS_VSCROLL | WS_CLIPCHILDREN,
            window_x,
            window_y,
            window_width,
            window_height,
            None,
            Some(menu),
            Some(instance),
            None,
        )?;
        let _ = ShowWindow(hwnd, SW_SHOW);
        if IsIconic(hwnd).as_bool() {
            let _ = ShowWindow(hwnd, SW_RESTORE);
        }
        let _ = UpdateWindow(hwnd);
        startup_checks(hwnd);

        let mut message = MSG::default();
        while GetMessageW(&mut message, None, 0, 0).as_bool() {
            let _ = TranslateMessage(&message);
            DispatchMessageW(&message);
        }
    }
    Ok(())
}
