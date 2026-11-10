//! Native Win32 memory PoC for the Equalizer APO Editor rewrite.
//!
//! Deliberately uses no egui/OpenGL/accessibility stack: a single window,
//! cached GDI fonts, system font linking and a virtualized 10k-row table.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clipboard;
mod calibration_dialog;
mod analysis_panel;
mod chooser;
mod chrome_panel;
mod copy_dialog;
mod draw;
mod geometry;
mod hotkeys;
mod input;
mod layout;
mod menus;
mod paint_core;
mod prompt;
mod rows_panel;

pub(crate) use draw::*;
pub(crate) use layout::*;
pub(crate) use paint_core::*;

use prompt::{prompt_hotkey, prompt_text};

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::mem::size_of;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, OnceLock};

use eapo_apo_info::{AbstractAPOInfo, DeviceAPOInfo};
use eapo_common::channel_helper;
use eapo_common::device_files;
use eapo_editor_core::analysis::{AnalysisRequest, AnalysisResult, AnalysisThread, DeviceSnapshot};
use eapo_editor_core::calibration;
use eapo_editor_core::config_file::{load_config_lines, save_config_file};
use eapo_editor_core::i18n;
use eapo_editor_core::file_dialog::FileDialog;
use eapo_editor_core::hotkey::HotkeyManager;
use eapo_editor_core::keyseq::{KeyCode, KeySequence, Modifiers};
use eapo_editor_core::live_files::{live_files, LiveMode};
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
    CLIP_DEFAULT_PRECIS, DC_BRUSH, DEFAULT_CHARSET, DEFAULT_PITCH,
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
    CREATESTRUCTW, DefWindowProcW, DeleteMenu, DestroyMenu, DestroyWindow, DispatchMessageW, DrawMenuBar,
    GetClientRect,
    GetMenu, GetMessageW, GetParent, GetSubMenu, GetWindowLongPtrW,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, IsDialogMessageW, IsIconic, IsWindow, IsZoomed, KillTimer, LoadCursorW,
    MessageBoxW, PostMessageW, PostQuitMessage, RegisterClassW, SendMessageW, SetTimer,
    SetWindowLongPtrW, SetWindowPos, SetWindowTextW, SetForegroundWindow, ShowWindow,
    TrackPopupMenuEx, TranslateMessage, CS_DBLCLKS, CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, ES_AUTOHSCROLL,
    GWLP_USERDATA, GWLP_WNDPROC, HMENU, IDCANCEL, IDC_ARROW, IDYES,
    MB_ICONINFORMATION, MB_ICONQUESTION, MB_ICONWARNING, MB_OK, MB_YESNO, MB_YESNOCANCEL,
    MF_BYCOMMAND, MF_BYPOSITION, MF_CHECKED,
    MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MF_GRAYED, MSG, SB_BOTTOM, SB_LINEDOWN,
    SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT,
    SCROLLINFO, SC_KEYMENU, SIF_PAGE, SIF_POS, SIF_RANGE, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE,
    SW_SHOW, SW_SHOWMAXIMIZED, SW_SHOWNORMAL,
    TPM_RETURNCMD, WM_APP, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_DPICHANGED,
    WM_ERASEBKGND, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_PRINTCLIENT, WM_RBUTTONDOWN, WM_SETFONT,
    WM_SIZE, WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSCOMMAND,
    WM_TIMER, WM_VSCROLL, WM_RBUTTONUP, WNDCLASSW, WNDPROC, WINDOW_EX_STYLE,
    WINDOW_STYLE, WS_BORDER, WS_CLIPCHILDREN, WS_TABSTOP,
    WS_CAPTION, WS_CHILD, WS_OVERLAPPEDWINDOW, WS_POPUP, WS_SYSMENU, WS_VISIBLE, WS_VSCROLL,
    SB_ENDSCROLL,
};

#[cfg(debug_assertions)]
const ROWS: usize = 10_000;
const ID_EXIT: usize = 199;
const ID_SAVE: usize = 103;
const ID_DELETE: usize = 114;
#[cfg(debug_assertions)]
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

#[cfg(debug_assertions)]
static SCROLL_TEST: AtomicBool = AtomicBool::new(false);
static SUPPRESS_KEYMENU: AtomicBool = AtomicBool::new(false);
static SKIP_CHECKS: AtomicBool = AtomicBool::new(false);
static CONFIG_PATH: OnceLock<Option<PathBuf>> = OnceLock::new();

/// The synthetic scroll benchmark is a development aid; release builds never
/// contain it.
#[cfg(debug_assertions)]
fn scroll_test_enabled() -> bool {
    SCROLL_TEST.load(Ordering::Relaxed)
}

#[cfg(not(debug_assertions))]
fn scroll_test_enabled() -> bool {
    false
}

const WHITE: COLORREF = COLORREF(0x00FF_FFFF);
const TABLE_BG: COLORREF = COLORREF(0x00FA_FAFA);
const CHROME_BG: COLORREF = COLORREF(0x00EE_EEEE);
const BORDER: COLORREF = COLORREF(0x00C8_C8C8);
const TEXT: COLORREF = COLORREF(0x0020_2020);
const MUTED: COLORREF = COLORREF(0x0070_7070);
const SELECTED: COLORREF = COLORREF(0x00FF_E8_D8);
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

/// Rebuilds the File menu in place, so newly opened files show up in the
/// recent-files list without restarting the editor.
unsafe fn build_file_menu(file: HMENU, recent: &[String], labels: &mut Vec<Vec<u16>>) {
    while DeleteMenu(file, 0, MF_BYPOSITION).is_ok() {}
    labels.clear();
    let _ = AppendMenuW(file, MF_STRING, 101, menu_text(labels, "New"));
    let _ = AppendMenuW(file, MF_STRING, 102, menu_text(labels, "Open..."));
    let _ = AppendMenuW(file, MF_STRING, ID_SAVE, menu_text(labels, "Save"));
    let _ = AppendMenuW(file, MF_STRING, 104, menu_text(labels, "Save as..."));
    if !recent.is_empty() {
        let _ = AppendMenuW(file, MF_SEPARATOR, 0, w!(""));
        for (index, path) in recent.iter().take(10).enumerate() {
            let _ = AppendMenuW(file, MF_STRING, 500 + index, menu_raw(labels, path));
        }
    }
    let _ = AppendMenuW(file, MF_SEPARATOR, 0, w!(""));
    let _ = AppendMenuW(file, MF_STRING, ID_EXIT, menu_text(labels, "Exit"));
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

#[cfg(debug_assertions)]
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
    prefs: serde_json::Map<String, serde_json::Value>,
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
    selection: HashSet<usize>,
    selection_anchor: Option<usize>,
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

#[cfg(debug_assertions)]
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

#[cfg(debug_assertions)]
fn scroll_test_startup() -> (Vec<String>, Option<PathBuf>) {
    (synthetic_lines(), None)
}

#[cfg(not(debug_assertions))]
fn scroll_test_startup() -> (Vec<String>, Option<PathBuf>) {
    (Vec::new(), None)
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

/// Screen-space analysis curve, rebuilt only when the data or the view
/// transform changes so hovering stays cheap.
struct AnalysisCurve {
    key: (u64, i32, i32, i32, i32, u32, u32, u64, u64),
    runs: Vec<(COLORREF, Vec<PointF>)>,
    fills: Vec<(i32, i32, i32, i32)>,
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
    /// Label buffers of the rebuildable File menu; `AppendMenuW` keeps pointers.
    menu_labels: Vec<Vec<u16>>,
    #[cfg(debug_assertions)]
    scroll_test_left: usize,
    doc_path: Option<PathBuf>,
    dirty: bool,
    edit: Option<EditState>,
    value_edit: Option<ValueEdit>,
    analysis: AnalysisThread,
    analysis_result: Option<AnalysisResult>,
    analysis_curve: Option<AnalysisCurve>,
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
    /// Last client width, used to keep the tab strip inside the window.
    client_width: i32,
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
    /// True while the channel selection follows the selected device.
    channel_mask_from_device: bool,
    volume_controller: Option<VolumeController>,
    master_volume: Option<f64>,
    /// Pending analysis restart after the system volume stopped moving.
    volume_analysis_due: Option<std::time::Instant>,
    plot: PlotState,
    plot_fitted_for: (i32, i32),
    /// Saved analysis view from the preferences, applied after the first fit.
    plot_restore: Option<(f64, f64, i32, i32)>,
    plot_rect: RECT,
    plot_hover: Option<(i32, i32)>,
    plot_pan: Option<(i32, i32)>,
    geq_pan: Option<(usize, i32, i32)>,
    geq_table_scroll_drag: Option<usize>,
    hover_pos: Option<(i32, i32)>,
    hot_rect: Option<RECT>,
    paint_offset: (i32, i32),
    /// Which of the open documents the audio engine should follow.
    live_mode: LiveMode,
    /// Last manifest content published, with the device it belongs to, so an
    /// unchanged list never rewrites the file (each write reloads the engine).
    live_published: Option<(String, Vec<PathBuf>)>,
    /// File accessibility for VST rows, queried once per path rather than on
    /// every repaint.
    vst_readable_cache: HashMap<String, Option<bool>>,
    live_warning_shown: bool,
}

impl App {
    unsafe fn new(dpi: u32) -> Self {
        let prefs = GlobalPreferences::load();
        let instant_mode = prefs.instant_mode;
        let live_mode = prefs.live_mode;
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
        let channel_mask_from_device = prefs.selected_channel_mask == 0;
        // A file that cannot be read must not keep its path in the tab: saving
        // would otherwise overwrite the real configuration with the error row.
        let (startup_lines, startup_path) = if scroll_test_enabled() {
            scroll_test_startup()
        } else {
            match CONFIG_PATH.get().cloned().flatten() {
                Some(path) => match load_config_lines(&path.to_string_lossy()) {
                    Ok(lines) => (lines, Some(path)),
                    Err(error) => (vec![format!("(cannot read {}: {error})", path.display())], None),
                },
                None => (Vec::new(), None),
            }
        };
        let plot_restore = if prefs.analysis_scroll_x != i32::MIN
            || prefs.analysis_scroll_y != i32::MIN
        {
            Some((
                prefs.analysis_zoom_x,
                prefs.analysis_zoom_y,
                prefs.analysis_scroll_x.max(0),
                prefs.analysis_scroll_y.max(0),
            ))
        } else {
            None
        };
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
            menu_labels: Vec::new(),
            #[cfg(debug_assertions)]
            scroll_test_left: if scroll_test_enabled() { 1500 } else { 0 },
            doc_path: startup_path,
            dirty: false,
            edit: None,
            value_edit: None,
            analysis: AnalysisThread::new(),
            analysis_result: None,
            analysis_curve: None,
            analysis_generation: 0,
            show_toolbar,
            show_analysis: !scroll_test_enabled() && prefs.show_analysis,
            channel_names,
            drag: None,
            drag_moved: false,
            reorder: None,
            reorder_drop: None,
            next_uid: 1,
            tabs: Vec::new(),
            active_tab: 0,
            client_width: 0,
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
            channel_mask_from_device,
            volume_controller: None,
            master_volume: None,
            volume_analysis_due: None,
            plot: PlotState::default(),
            plot_fitted_for: (0, 0),
            plot_restore,
            plot_rect: RECT::default(),
            plot_hover: None,
            plot_pan: None,
            geq_pan: None,
            geq_table_scroll_drag: None,
            hover_pos: None,
            hot_rect: None,
            paint_offset: (0, 0),
            live_mode,
            live_published: None,
            vst_readable_cache: HashMap::new(),
            live_warning_shown: false,
        };
        app.create_fonts();
        app.set_lines(startup_lines);
        let title = app.doc_title();
        app.tabs.push(TabInfo {
            title,
            path: app.doc_path.clone(),
            lines: app.rows.iter().map(|row| row.line.clone()).collect(),
            offset: 0,
            selected: None,
            selection: HashSet::new(),
            selection_anchor: None,
            dirty: false,
        });
        if live_mode != LiveMode::Off && !scroll_test_enabled() {
            // Adopt the list published by an earlier session, so the title marks
            // the live document and a first switch does not rewrite it needlessly.
            if let Some(device) = &app.selected_device {
                let guid = device.get_device_guid();
                let files =
                    device_files::files_for(&settings::config_dir_from_registry(), &guid);
                if !files.is_empty() {
                    app.live_published = Some((guid, files));
                }
            }
        }
        app.reload_hotkeys();
        app.start_analysis();
        app
    }

    /// The document of every tab. The frontmost tab comes from the live state
    /// because `tabs` is only refreshed when a tab is captured.
    fn tab_paths(&self) -> Vec<Option<PathBuf>> {
        let mut paths: Vec<Option<PathBuf>> =
            self.tabs.iter().map(|tab| tab.path.clone()).collect();
        if let Some(slot) = paths.get_mut(self.active_tab) {
            *slot = self.doc_path.clone();
        }
        paths
    }

    /// Publishes the files the audio engine should load for the selected
    /// device, so switching tabs switches the live configuration. While live
    /// mode is off the manifest is never touched, which keeps the original
    /// "always read config.txt" behaviour.
    unsafe fn publish_live_files(&mut self, hwnd: HWND) {
        if scroll_test_enabled() {
            return;
        }
        let guid = match self.selected_device.as_ref() {
            Some(device) => device.get_device_guid(),
            None => return,
        };
        if guid.trim().is_empty() {
            return;
        }
        let files = live_files(self.live_mode, &self.tab_paths(), self.active_tab);
        if self
            .live_published
            .as_ref()
            .is_some_and(|(last_guid, last_files)| {
                *last_guid == guid && *last_files == files
            })
        {
            return;
        }
        let config_dir = settings::config_dir_from_registry();
        match device_files::update_device(&config_dir, &guid, &files) {
            Ok(_) => {
                self.live_published = Some((guid, files));
                self.update_title(hwnd);
            }
            Err(error) => {
                if !self.live_warning_shown {
                    self.live_warning_shown = true;
                    let text = wide_z(&format!(
                        "{}: {} ({error})",
                        i18n::tr("Could not update the live configuration list"),
                        config_dir.display()
                    ));
                    let _ = MessageBoxW(
                        Some(hwnd),
                        PCWSTR(text.as_ptr()),
                        w!("Equalizer APO"),
                        MB_ICONWARNING,
                    );
                }
            }
        }
    }

    /// Stops following the editor for the selected device: the device goes back
    /// to the configuration file the engine reads by default.
    unsafe fn clear_live_files(&mut self) {
        let guid = match self.selected_device.as_ref() {
            Some(device) => device.get_device_guid(),
            None => return,
        };
        if guid.trim().is_empty() {
            return;
        }
        let config_dir = settings::config_dir_from_registry();
        let _ = device_files::remove_device(&config_dir, &guid);
        self.live_published = None;
    }

    unsafe fn set_live_mode(&mut self, hwnd: HWND, mode: LiveMode) {
        if mode == self.live_mode {
            return;
        }
        self.live_mode = mode;
        self.prefs.live_mode = mode;
        self.prefs.save();
        match mode {
            LiveMode::Off => self.clear_live_files(),
            _ => self.publish_live_files(hwnd),
        }
        self.sync_live_menu(hwnd);
        self.update_title(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    fn doc_title(&self) -> String {
        self.doc_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| i18n::tr("Unsaved"))
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
        tab.selection = self.selection.clone();
        tab.selection_anchor = self.selection_anchor;
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
        let selection = tab.selection.clone();
        let selection_anchor = tab.selection_anchor;
        let dirty = tab.dirty;
        self.active_tab = index;
        self.doc_path = path;
        self.set_lines(lines);
        self.reload_hotkeys();
        self.offset = offset;
        self.selected = selected;
        self.selection = if selection.is_empty() {
            selected.into_iter().collect()
        } else {
            selection
        };
        self.selection_anchor = selection_anchor.or(selected);
        self.dirty = dirty;
        self.edit = None;
        self.drag = None;
        self.analysis_result = None;
        self.update_scrollbar(hwnd);
        self.update_title(hwnd);
        self.start_analysis();
        self.publish_live_files(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    unsafe fn switch_tab(&mut self, hwnd: HWND, index: usize) {
        if index == self.active_tab || index >= self.tabs.len() {
            return;
        }
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        self.capture_active_tab();
        self.load_tab(hwnd, index);
    }

    unsafe fn new_tab(&mut self, hwnd: HWND) {
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        self.capture_active_tab();
        let index = self.tabs.len();
        self.tabs.push(TabInfo {
            title: i18n::tr("Unsaved"),
            path: None,
            lines: vec![String::new()],
            offset: 0,
            selected: None,
            selection: HashSet::new(),
            selection_anchor: None,
            dirty: false,
        });
        self.load_tab(hwnd, index);
    }

    unsafe fn open_paths(&mut self, hwnd: HWND, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        self.capture_active_tab();
        let mut last = None;
        for path in paths {
            if let Some(index) = self
                .tabs
                .iter()
                .position(|tab| tab.path.as_ref().is_some_and(|open| same_file(open, &path)))
            {
                settings::push_recent(
                    &mut self.recent_files,
                    &path.to_string_lossy(),
                );
                self.rebuild_file_menu(hwnd);
                last = Some(index);
                continue;
            }
            let path_string = path.to_string_lossy().into_owned();
            let lines = match load_config_lines(&path_string) {
                Ok(lines) => lines,
                Err(error) => {
                    let message = wide_z(&i18n::tr_arg(
                        "Could not read %0: %1",
                        &[&path.display().to_string(), &error.to_string()],
                    ));
                    let _ = MessageBoxW(
                        Some(hwnd),
                        PCWSTR(message.as_ptr()),
                        w!("Equalizer APO"),
                        MB_ICONWARNING,
                    );
                    continue;
                }
            };
            settings::push_recent(&mut self.recent_files, &path_string);
            self.rebuild_file_menu(hwnd);
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
                selection: HashSet::new(),
                selection_anchor: None,
                dirty: false,
            });
            last = Some(self.tabs.len() - 1);
        }
        if let Some(index) = last {
            self.load_tab(hwnd, index);
        }
    }

    unsafe fn open_dialog(&mut self, hwnd: HWND) {
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        let mut dialog = FileDialog::new()
            .set_title(&i18n::tr("Open configuration"))
            .add_filter("E-APO configurations (*.txt)", &["txt"]);
        if let Some(parent) = self.doc_path.as_ref().and_then(|path| path.parent()) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(paths) = dialog.pick_files() {
            self.open_paths(hwnd, paths);
        }
    }

    unsafe fn save_as(&mut self, hwnd: HWND) -> bool {
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        let mut dialog = FileDialog::new()
            .set_title(&i18n::tr("Save configuration as"))
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
        let previous_path = self.doc_path.clone();
        self.doc_path = Some(path);
        let saved = self.save(hwnd);
        if saved {
            let path_string = self
                .doc_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            settings::push_recent(&mut self.recent_files, &path_string);
            self.rebuild_file_menu(hwnd);
            self.capture_active_tab();
            self.save_hotkeys();
            self.update_title(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
        } else {
            self.doc_path = previous_path;
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
        let mut updated = false;
        while let Some(result) = self.analysis.try_recv() {
            if result.generation == self.analysis_generation {
                self.analysis_result = Some(result);
                updated = true;
            }
        }
        if updated {
            unsafe {
                self.invalidate_analysis(hwnd);
            }
        }
        updated
    }

    /// Repaints only the analysis dock; a new result does not affect the table.
    unsafe fn invalidate_analysis(&self, hwnd: HWND) {
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let top = client.bottom - self.analysis_height();
        if self.show_analysis && top >= 0 {
            let rect = RECT {
                left: 0,
                top,
                right: client.right,
                bottom: client.bottom,
            };
            let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
        } else {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    fn poll_volume(&mut self, hwnd: HWND) {
        // Recompute the loudness curve only after the volume stopped changing,
        // otherwise dragging the system slider restarts the analysis on every
        // poll.
        if let Some(due) = self.volume_analysis_due {
            if std::time::Instant::now() >= due {
                self.volume_analysis_due = None;
                self.start_analysis();
            }
        }
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
        let mut changed_rows = Vec::new();
        for (index, row) in self.rows.iter_mut().enumerate() {
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
                        changed_rows.push(index);
                    }
                }
            }
        }
        if !changed_rows.is_empty() {
            unsafe {
                for index in changed_rows {
                    self.invalidate_row(hwnd, index);
                }
            }
        }
        self.volume_analysis_due = Some(
            std::time::Instant::now() + std::time::Duration::from_millis(300),
        );
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
        let row_state = self
            .doc_path
            .as_ref()
            .map(|path| settings::load_row_state(&path.to_string_lossy()))
            .unwrap_or_default();
        for (position, line) in lines.into_iter().enumerate() {
            let structural = parse_line(&line);
            let kind = structural.kind;
            let enabled = structural.enabled;
            let height = row_height(kind, self.dpi);
            let uid = self.next_uid;
            self.next_uid += 1;
            let command = line.split(':').next().unwrap_or("").trim();
            let mut prefs = row_state
                .get(&(position + 1))
                .filter(|(stored, _)| stored == command)
                .map(|(_, prefs)| prefs.clone())
                .unwrap_or_default();
            prefs.remove("hotkey");
            self.rows.push(Row {
                uid,
                line,
                kind,
                height,
                enabled,
                item: None,
                hotkey: None,
                prefs,
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
        let prefs = self.rows[index].prefs.clone();
        let mut item = FilterItem::new(uid, line, &context);
        item.prefs = prefs;
        item.rebuild(&context);
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

    /// Cached accessibility check for a VST plugin path.  The ACL query is
    /// expensive and used to run on every repaint of the row.
    fn vst_readable(&mut self, path: &str) -> Option<bool> {
        if path.is_empty() {
            return None;
        }
        if let Some(value) = self.vst_readable_cache.get(path) {
            return *value;
        }
        let path_ref = std::path::Path::new(path);
        let absolute = if path_ref.is_absolute() {
            path_ref.to_path_buf()
        } else {
            std::path::Path::new(
                &eapo_filters::vst_plugin_library::VSTPluginLibrary::get_default_plugin_path(),
            )
            .join(path_ref)
        };
        let value = if absolute.exists() {
            vst_editor::audio_service_readable(&absolute)
        } else {
            None
        };
        self.vst_readable_cache.insert(path.to_string(), value);
        value
    }

    fn sync_item(&mut self, hwnd: HWND, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        let Some((line, prefs)) = self.rows[index].item.as_mut().map(|item| {
            item.sync_from_editor();
            let line = item.line.clone();
            let mut prefs = item.collect_prefs();
            prefs.remove("hotkey");
            (line, prefs)
        }) else {
            return;
        };
        self.rows[index].prefs = prefs;
        self.apply_row_line(index, line);
        self.dirty = true;
        unsafe {
            self.update_title(hwnd);
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

    unsafe fn rebuild_file_menu(&mut self, hwnd: HWND) {
        let file = GetSubMenu(GetMenu(hwnd), 0);
        if file.0.is_null() {
            return;
        }
        build_file_menu(file, &self.recent_files, &mut self.menu_labels);
        let _ = DrawMenuBar(hwnd);
    }

    unsafe fn update_title(&self, hwnd: HWND) {
        let name = self
            .doc_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| i18n::tr("Unsaved"));
        let suffix = if self.dirty { " *" } else { "" };
        // Mark the document the audio engine currently follows for this device.
        let live = self.live_mode != LiveMode::Off
            && self
                .doc_path
                .as_ref()
                .is_some_and(|path| self.live_published.as_ref().is_some_and(|(_, files)| files.contains(path)));
        let live_mark = if live { " [live]" } else { "" };
        let title = format!("Equalizer APO Configuration Editor - {name}{suffix}{live_mark}");
        let mut current = [0u16; 512];
        let length = GetWindowTextW(hwnd, &mut current).max(0) as usize;
        if length > 0
            && String::from_utf16_lossy(&current[..length.min(current.len())]) == title
        {
            return;
        }
        let title = wide_z(&title);
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
                self.publish_live_files(hwnd);
                true
            }
            Err(error) => {
                let text = wide_z(&i18n::tr_arg(
                    "Could not save %0: %1",
                    &[&path.display().to_string(), &error.to_string()],
                ));
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
        if !file_specific {
            // The restart comes back with live mode off, so drop this device's
            // manifest entry instead of leaving it applied behind the scenes.
            self.clear_live_files();
        }
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

    /// Keeps the live-configuration radio in the Settings menu in sync.
    unsafe fn sync_live_menu(&self, hwnd: HWND) {
        let settings = GetSubMenu(GetMenu(hwnd), 3);
        if settings.0.is_null() {
            return;
        }
        let checked = |mode: LiveMode| {
            if self.live_mode == mode {
                MF_CHECKED.0
            } else {
                MF_UNCHECKED.0
            }
        };
        let _ = CheckMenuItem(settings, 130, MF_BYCOMMAND.0 | checked(LiveMode::Off));
        let _ = CheckMenuItem(settings, 131, MF_BYCOMMAND.0 | checked(LiveMode::ActiveTab));
        let _ = CheckMenuItem(settings, 132, MF_BYCOMMAND.0 | checked(LiveMode::AllTabs));
    }

    /// Asks for every dirty tab before the window closes.  Saving a tab that is
    /// not in front requires loading it, so the original tab is restored when
    /// the user cancels.
    unsafe fn confirm_close(&mut self, hwnd: HWND) -> bool {
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        self.capture_active_tab();
        let original = self.active_tab;
        for index in 0..self.tabs.len() {
            if !self.tabs[index].dirty {
                continue;
            }
            let title = wide_z(&i18n::tr_arg(
                "Save changes to %0 before closing?",
                &[&self.tabs[index].title],
            ));
            let answer = MessageBoxW(
                Some(hwnd),
                PCWSTR(title.as_ptr()),
                w!("Equalizer APO"),
                MB_YESNOCANCEL | MB_ICONWARNING,
            );
            if answer.0 == IDCANCEL.0 {
                self.load_tab(hwnd, original);
                return false;
            }
            if answer.0 == IDYES.0 {
                self.load_tab(hwnd, index);
                if !self.save(hwnd) {
                    self.load_tab(hwnd, original);
                    return false;
                }
            }
        }
        true
    }

    unsafe fn save_preferences(&mut self, hwnd: HWND) {
        let mut prefs = self.prefs.clone();
        prefs.instant_mode = self.instant_mode;
        prefs.live_mode = self.live_mode;
        prefs.show_toolbar = self.show_toolbar;
        prefs.show_analysis = self.show_analysis;
        prefs.recent_files = self.recent_files.clone();
        prefs.analysis_resolution = self.analysis_resolution;
        prefs.analysis_start_from = self.analysis_start_from;
        // Keep the previous view when the analysis panel was never laid out in
        // this session, otherwise closing the editor would reset it.
        if self.plot_fitted_for != (0, 0) {
            prefs.analysis_zoom_x = self.plot.zoom_x;
            prefs.analysis_zoom_y = self.plot.zoom_y;
            prefs.analysis_scroll_x = self.plot.scroll_x.round() as i32;
            prefs.analysis_scroll_y = self.plot.scroll_y.round() as i32;
        }
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
        prefs.selected_channel_mask = if self.channel_mask_from_device {
            0
        } else {
            self.channel_mask as i32
        };
        // Use the real screen rectangle so the saved position matches the
        // coordinates CreateWindowExW expects.  Minimized and maximized
        // windows have no meaningful normal rectangle; keep the last values.
        if !IsIconic(hwnd).as_bool() {
            prefs.window_maximized = IsZoomed(hwnd).as_bool();
        }
        if !IsIconic(hwnd).as_bool() && !IsZoomed(hwnd).as_bool() {
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_ok() {
                let width = rect.right - rect.left;
                let height = rect.bottom - rect.top;
                if width >= 320 && height >= 240 && rect.left > -30000 && rect.top > -30000 {
                    prefs.window_size = Some((width as f32, height as f32));
                    prefs.window_pos = Some((rect.left as f32, rect.top as f32));
                }
            }
        }
        prefs.save();
        self.prefs = prefs;
        // Persist the per-row GUI preferences together with the hotkeys so
        // VST embedding, GEQ zoom and similar state survives a restart.
        self.save_hotkeys();
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
            let capacity = GetWindowTextLengthW(edit.hwnd).max(0) as usize + 1;
            let mut buffer = vec![0u16; capacity];
            let length = GetWindowTextW(edit.hwnd, &mut buffer).max(0) as usize;
            let text = String::from_utf16_lossy(&buffer[..length.min(buffer.len())]);
            if edit.row < self.rows.len() && self.rows[edit.row].line != text {
                self.apply_row_line(edit.row, text);
                // A raw text edit is authoritative; force a re-parse so the
                // next slider edit does not overwrite it with stale state.
                self.rows[edit.row].item = None;
                self.dirty = true;
                self.update_title(hwnd);
                self.update_scrollbar(hwnd);
                self.save_hotkeys();
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
                    self.save_hotkeys();
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
                prefs: serde_json::Map::new(),
                geq_table_top: 0,
            },
        );
        self.selected = Some(index);
        self.dirty = true;
        self.recompute_prefix();
        self.update_scrollbar(hwnd);
        self.ensure_visible(hwnd, index);
        self.update_title(hwnd);
        self.save_hotkeys();
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
        self.update_scrollbar(hwnd);
        self.save_hotkeys();
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
                    prefs: serde_json::Map::new(),
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
        self.update_scrollbar(hwnd);
        self.update_title(hwnd);
        self.save_hotkeys();
        self.start_analysis();
        self.autosave(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
        true
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


    unsafe fn close_tab(&mut self, hwnd: HWND, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if index != self.active_tab {
            self.switch_tab(hwnd, index);
        }
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        self.capture_active_tab();
        if self.tabs[index].dirty {
            let title = wide_z(&i18n::tr_arg(
                "Save changes to %0 before closing?",
                &[&self.tabs[index].title],
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
        let no_saved_tabs = self.tabs.iter().all(|tab| tab.path.is_none());
        if self.tabs.is_empty() {
            self.tabs.push(TabInfo {
                title: i18n::tr("Unsaved"),
                path: None,
                lines: vec![String::new()],
                offset: 0,
                selected: None,
                selection: HashSet::new(),
                selection_anchor: None,
                dirty: false,
            });
        }
        let next = index.min(self.tabs.len() - 1);
        self.active_tab = usize::MAX;
        self.load_tab(hwnd, next);
        // Closing the last saved document means the editor no longer follows
        // any file for this device, so remove the live manifest and let the
        // engine fall back to its default configuration.
        if no_saved_tabs && self.live_mode != LiveMode::Off && self.live_published.is_some() {
            self.clear_live_files();
            self.update_title(hwnd);
        }
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


    #[cfg(debug_assertions)]
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
        self.client_width = client.right;
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

}

impl Drop for App {
    fn drop(&mut self) {
        unsafe { self.destroy_fonts() };
    }
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

unsafe extern "system" fn edit_proc(
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
    if SKIP_CHECKS.load(Ordering::Relaxed) || scroll_test_enabled() {
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
        let text = wide_z(&i18n::tr(
            "Equalizer APO is not installed correctly: the configuration path is not registered, so gain and filter changes will not affect audio.\nDo you want to start the installer (Setup.exe) to repair the installation?",
        ));
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
            if let Some(state) = app(hwnd) {
                state.commit_value_edit(hwnd, true);
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
                state.commit_value_edit(hwnd, true);
                state.commit_edit(hwnd, true);
                state.set_dpi(dpi);
                state.update_scrollbar(hwnd);
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            LRESULT(0)
        }
        WM_TIMER => {
            #[cfg(debug_assertions)]
            {
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
                }
            }
            if wparam.0 == TIMER_ANALYSIS {
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
                    130 | 131 | 132 => {
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
            if let Some(state) = app(hwnd) {
                allow = state.confirm_close(hwnd);
                if !allow {
                    // A pending reset/restart must not fire after the user
                    // cancelled the save prompt.
                    state.restart_after_close = false;
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
    #[cfg(debug_assertions)]
    {
        let synthetic = args.iter().any(|argument| argument == "--scroll-test");
        SCROLL_TEST.store(synthetic, Ordering::Relaxed);
    }
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
            style: CS_HREDRAW | CS_VREDRAW | CS_DBLCLKS,
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
        build_file_menu(file, &recent_files, &mut menu_texts);
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
                | check_flag(!scroll_test_enabled() && window_prefs.show_analysis),
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
            130,
            menu_text(&mut menu_texts, "Live configuration: none (use config.txt)"),
        );
        let _ = AppendMenuW(
            settings_menu,
            MF_STRING,
            131,
            menu_text(&mut menu_texts, "Live configuration: active tab"),
        );
        let _ = AppendMenuW(
            settings_menu,
            MF_STRING,
            132,
            menu_text(&mut menu_texts, "Live configuration: all open tabs"),
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
        if window_prefs.window_maximized {
            let _ = ShowWindow(hwnd, SW_SHOWMAXIMIZED);
        }
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
