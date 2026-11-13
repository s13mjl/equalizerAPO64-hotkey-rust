//! Native Win32 memory PoC for the Equalizer APO Editor rewrite.
//!
//! Deliberately uses no egui/OpenGL/accessibility stack: a single window,
//! cached GDI fonts, system font linking and a virtualized 10k-row table.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis_panel;
mod analysis_state;
mod calibration_dialog;
mod chooser;
mod chrome_panel;
mod clipboard;
mod copy_dialog;
mod documents;
mod draw;
mod geometry;
mod hotkeys;
mod input;
mod layout;
mod menus;
mod paint_core;
mod palette;
mod prompt;
mod rows_edit;
mod rows_model;
mod rows_panel;
mod scroll;
mod settings_ui;
mod startup;
mod window_proc;

pub(crate) use draw::*;
pub(crate) use layout::*;
pub(crate) use paint_core::*;
pub(crate) use startup::*;
pub(crate) use window_proc::*;

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
use windows::core::{w, Interface, IUnknown, PCWSTR};
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
use windows::Win32::UI::Accessibility::{
    CreateStdAccessibleObject, IAccessible, LresultFromObject,
};
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
    MF_BYCOMMAND, MF_BYPOSITION, MF_CHECKED, OBJID_CLIENT,
    MF_POPUP, MF_SEPARATOR, MF_STRING, MF_UNCHECKED, MF_GRAYED, MSG, SB_BOTTOM, SB_LINEDOWN,
    SB_LINEUP, SB_PAGEDOWN, SB_PAGEUP, SB_THUMBPOSITION, SB_THUMBTRACK, SB_TOP, SB_VERT,
    SCROLLINFO, SC_KEYMENU, SIF_PAGE, SIF_POS, SIF_RANGE, SWP_NOACTIVATE, SWP_NOZORDER, SW_RESTORE,
    SW_SHOW, SW_SHOWMAXIMIZED, SW_SHOWNORMAL,
    TPM_RETURNCMD, WM_APP, WM_CHAR, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_DESTROY, WM_DPICHANGED,
    WM_ERASEBKGND, WM_GETOBJECT, WM_KEYDOWN, WM_KILLFOCUS, WM_LBUTTONDBLCLK, WM_LBUTTONDOWN, WM_LBUTTONUP,
    WM_MOUSEMOVE, WM_MOUSEWHEEL, WM_NCDESTROY, WM_PAINT, WM_PRINTCLIENT, WM_RBUTTONDOWN, WM_SETFONT,
    WM_SIZE, WM_SYSCHAR, WM_SYSKEYDOWN, WM_SYSCOMMAND,
    WM_SETTINGCHANGE, WM_THEMECHANGED,
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
    /// Instant-mode writes are debounced; dragging a slider must not rewrite
    /// the configuration dozens of times per second.
    autosave_due: Option<std::time::Instant>,
    /// Background writer so the UI thread never waits for the engine to
    /// release the configuration file.
    instant_save_tx: Option<std::sync::mpsc::Sender<(u64, PathBuf, Vec<String>)>>,
    instant_save_rx: std::sync::mpsc::Receiver<(u64, std::io::Result<()>)>,
    instant_save_thread: Option<std::thread::JoinHandle<()>>,
    instant_save_serial: u64,
    instant_save_in_flight: usize,
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
        // Serialize instant-mode writes on a worker thread.  Snapshots are
        // written in order, so an older write can never overwrite a newer one.
        let (instant_save_tx, instant_save_jobs) =
            std::sync::mpsc::channel::<(u64, PathBuf, Vec<String>)>();
        let (instant_save_results, instant_save_rx) = std::sync::mpsc::channel();
        let instant_save_thread = std::thread::spawn(move || {
            while let Ok((serial, path, lines)) = instant_save_jobs.recv() {
                let result = save_config_file(&path.to_string_lossy(), &lines);
                if instant_save_results.send((serial, result)).is_err() {
                    break;
                }
            }
        });
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
            autosave_due: None,
            instant_save_tx: Some(instant_save_tx),
            instant_save_rx,
            instant_save_thread: Some(instant_save_thread),
            instant_save_serial: 0,
            instant_save_in_flight: 0,
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

        fill(hdc, 0, 0, client.right, client.bottom, palette::table_bg());
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
        // Close the job channel and wait for the worker to write everything
        // that was already queued; a snapshot must never be dropped on exit.
        self.instant_save_tx.take();
        if let Some(thread) = self.instant_save_thread.take() {
            let _ = thread.join();
        }
        unsafe { self.destroy_fonts() };
    }
}

fn main() -> windows::core::Result<()> {
    palette::refresh();
    // Hotkeys now live in `rowPrefs`/`hotkeyBindings` next to the filter they
    // belong to.  Drop the legacy per-filter registry tree so IDs from older
    // sessions do not linger.
    eapo_editor_core::hotkey::save_hotkeys_to(eapo_editor_core::hotkey::HOTKEY_REGPATH, &[]);
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
