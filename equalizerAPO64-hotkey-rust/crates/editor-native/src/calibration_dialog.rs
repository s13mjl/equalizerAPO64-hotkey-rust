//! Modal LoudnessCorrection calibration dialog with pink-noise playback.

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::OnceLock;

use eapo_editor_core::calibration;
use eapo_editor_core::i18n;
use windows::core::{w, PCWSTR};
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
    GetParent, GetWindowLongPtrW, GetWindowRect, GetWindowTextW, IsDialogMessageW, IsWindow, LoadCursorW,
    RegisterClassW, SendMessageW, SetForegroundWindow, SetWindowLongPtrW, SetWindowTextW,
    ShowWindow, TranslateMessage, CREATESTRUCTW, CS_HREDRAW, CS_VREDRAW, GWLP_USERDATA, HMENU,
    IDC_ARROW, MSG, SW_SHOW, WM_CLOSE, WM_COMMAND, WM_CREATE, WM_NCDESTROY, WM_SETFONT, WNDCLASSW,
    WINDOW_EX_STYLE, WINDOW_STYLE, WS_CAPTION, WS_CHILD, WS_POPUP, WS_SYSMENU, WS_TABSTOP,
    WS_VISIBLE,
};

const ID_OK: usize = 1;
const ID_CANCEL: usize = 2;
const ID_CH_LEFT: usize = 10;
const ID_CH_RIGHT: usize = 11;
const ID_CH_BOTH: usize = 12;
const ID_PLAY: usize = 20;

const ES_AUTOHSCROLL: u32 = 0x0080;
const ES_NUMBER: u32 = 0x2000;
const BS_AUTORADIOBUTTON: u32 = 0x0009;
const BS_DEFPUSHBUTTON: u32 = 0x0001;
const BS_GROUPBOX: u32 = 0x0007;
const WS_GROUP: u32 = 0x0002_0000;
const BM_SETCHECK: u32 = 0x00F1;
const BST_CHECKED: usize = 1;

struct CalState {
    edit: HWND,
    play: HWND,
    font: HFONT,
    volume: f64,
    result: Rc<RefCell<Option<i32>>>,
}

static CAL_CLASS: OnceLock<()> = OnceLock::new();

fn scale(px: i32, dpi: u32) -> i32 {
    (px as f64 * dpi as f64 / 96.0).round() as i32
}

fn wide_z(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

unsafe fn state_of(hwnd: HWND) -> Option<&'static mut CalState> {
    (GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CalState).as_mut()
}

#[allow(clippy::too_many_arguments)]
unsafe fn create_control(
    parent: HWND,
    class: PCWSTR,
    label: &str,
    style: u32,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
    id: usize,
    font: HFONT,
) -> Option<HWND> {
    let text = wide_z(&label);
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

unsafe fn set_playing_text(state: &CalState) {
    let label = if calibration::is_playing() {
        i18n::tr("Stop")
    } else {
        i18n::tr("Play")
    };
    let text = wide_z(&label);
    let _ = SetWindowTextW(state.play, PCWSTR(text.as_ptr()));
}

unsafe fn build_controls(hwnd: HWND, state: &mut CalState) {
    let dpi = GetDpiForWindow(hwnd).max(96);
    let mut client = RECT::default();
    let _ = GetClientRect(hwnd, &mut client);
    let margin = scale(12, dpi);
    let width = client.right - margin * 2;
    let font = state.font;
    let _ = create_control(
        hwnd,
        w!("STATIC"),
        &i18n::tr("For a correct calibration, a sound level meter is required."),
        0,
        margin,
        scale(12, dpi),
        width,
        scale(18, dpi),
        0,
        font,
    );
    let _ = create_control(
        hwnd,
        w!("STATIC"),
        &i18n::tr("Disable loudness correction during measurement (done automatically while this dialog is open)."),
        0,
        margin,
        scale(32, dpi),
        width,
        scale(18, dpi),
        0,
        font,
    );
    let _ = create_control(
        hwnd,
        w!("STATIC"),
        &i18n::tr("Enable only one speaker and play pink noise at full application volume."),
        0,
        margin,
        scale(52, dpi),
        width,
        scale(18, dpi),
        0,
        font,
    );
    let _ = create_control(
        hwnd,
        w!("BUTTON"),
        &i18n::tr("Channel"),
        BS_GROUPBOX | WS_GROUP,
        margin,
        scale(78, dpi),
        scale(300, dpi),
        scale(48, dpi),
        0,
        font,
    );
    let selected = calibration::selected_channel();
    let channels = [
        (ID_CH_LEFT, calibration::CHANNEL_LEFT, &i18n::tr("Left"), scale(24, dpi)),
        (ID_CH_RIGHT, calibration::CHANNEL_RIGHT, &i18n::tr("Right"), scale(100, dpi)),
        (ID_CH_BOTH, calibration::CHANNEL_BOTH, &i18n::tr("Both"), scale(176, dpi)),
    ];
    for (id, channel, label, x) in channels {
        if let Some(radio) = create_control(
            hwnd,
            w!("BUTTON"),
            label,
            BS_AUTORADIOBUTTON,
            margin + x,
            scale(96, dpi),
            scale(70, dpi),
            scale(22, dpi),
            id,
            font,
        ) {
            if selected == channel {
                SendMessageW(radio, BM_SETCHECK, Some(WPARAM(BST_CHECKED)), None);
            }
        }
    }
    let play_label = if calibration::is_playing() {
        i18n::tr("Stop")
    } else {
        i18n::tr("Play")
    };
    state.play = create_control(
        hwnd,
        w!("BUTTON"),
        &play_label,
        0,
        margin + scale(300, dpi),
        scale(92, dpi),
        scale(76, dpi),
        scale(28, dpi),
        ID_PLAY,
        font,
    )
    .unwrap_or_default();
    let _ = create_control(
        hwnd,
        w!("STATIC"),
        &i18n::tr("Adjust volume via Windows volume control so that the sound level meter shows 75 dB at listening position."),
        0,
        margin,
        scale(136, dpi),
        width,
        scale(36, dpi),
        0,
        font,
    );
    let _ = create_control(
        hwnd,
        w!("STATIC"),
        &i18n::tr("Level at listening position:"),
        0,
        margin,
        scale(182, dpi),
        scale(170, dpi),
        scale(20, dpi),
        0,
        font,
    );
    state.edit = create_control(
        hwnd,
        w!("EDIT"),
        "0",
        ES_AUTOHSCROLL | ES_NUMBER,
        margin + scale(176, dpi),
        scale(178, dpi),
        scale(70, dpi),
        scale(24, dpi),
        30,
        font,
    )
    .unwrap_or_default();
    let _ = create_control(
        hwnd,
        w!("STATIC"),
        "dB",
        0,
        margin + scale(252, dpi),
        scale(182, dpi),
        scale(30, dpi),
        scale(20, dpi),
        0,
        font,
    );
    let volume = format!("{} {:.1} dB", i18n::tr("Master volume:"), state.volume);
    let _ = create_control(
        hwnd,
        w!("STATIC"),
        &volume,
        0,
        margin,
        scale(212, dpi),
        scale(220, dpi),
        scale(20, dpi),
        0,
        font,
    );
    let bottom = client.bottom - scale(40, dpi);
    let _ = create_control(
        hwnd,
        w!("BUTTON"),
        &i18n::tr("OK"),
        BS_DEFPUSHBUTTON,
        client.right - scale(190, dpi),
        bottom,
        scale(80, dpi),
        scale(26, dpi),
        ID_OK,
        font,
    );
    let _ = create_control(
        hwnd,
        w!("BUTTON"),
        &i18n::tr("Cancel"),
        0,
        client.right - scale(100, dpi),
        bottom,
        scale(80, dpi),
        scale(26, dpi),
        ID_CANCEL,
        font,
    );
}

/// Re-enables the owner before the modal dialog disappears so Windows hands
/// activation back to the editor instead of leaving it in the background.
unsafe fn reenable_owner(hwnd: HWND) {
    if let Ok(parent) = GetParent(hwnd) {
        let _ = EnableWindow(parent, true);
    }
}

unsafe extern "system" fn cal_proc(
    hwnd: HWND,
    msg: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match msg {
        WM_CREATE => {
            let create = &*(lparam.0 as *const CREATESTRUCTW);
            let state = Box::from_raw(create.lpCreateParams as *mut CalState);
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
                        let mut buffer = [0u16; 32];
                        let len = GetWindowTextW(state.edit, &mut buffer);
                        let text = String::from_utf16_lossy(&buffer[..len.max(0) as usize]);
                        let measured = text.trim().parse::<i32>().unwrap_or(0).clamp(0, 200);
                        *state.result.borrow_mut() = Some(measured);
                    }
                    calibration::stop();
                    reenable_owner(hwnd);
                    let _ = DestroyWindow(hwnd);
                }
                ID_CANCEL => {
                    calibration::stop();
                    reenable_owner(hwnd);
                    let _ = DestroyWindow(hwnd);
                }
                ID_CH_LEFT => calibration::set_selected_channel(calibration::CHANNEL_LEFT),
                ID_CH_RIGHT => calibration::set_selected_channel(calibration::CHANNEL_RIGHT),
                ID_CH_BOTH => calibration::set_selected_channel(calibration::CHANNEL_BOTH),
                ID_PLAY => {
                    if calibration::is_playing() {
                        calibration::stop();
                    } else {
                        calibration::play(calibration::selected_channel());
                    }
                    if let Some(state) = state_of(hwnd) {
                        set_playing_text(state);
                    }
                }
                _ => {}
            }
            LRESULT(0)
        }
        WM_CLOSE => {
            calibration::stop();
            reenable_owner(hwnd);
            let _ = DestroyWindow(hwnd);
            LRESULT(0)
        }
        WM_NCDESTROY => {
            calibration::stop();
            let pointer = GetWindowLongPtrW(hwnd, GWLP_USERDATA) as *mut CalState;
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

/// Shows the calibration dialog. Returns the measured level when confirmed.
pub unsafe fn calibrate(parent: HWND, volume: f64) -> Option<i32> {
    CAL_CLASS.get_or_init(|| {
        let Ok(module) = GetModuleHandleW(None) else {
            return;
        };
        let Ok(cursor) = LoadCursorW(None, IDC_ARROW) else {
            return;
        };
        let class = WNDCLASSW {
            style: CS_HREDRAW | CS_VREDRAW,
            lpfnWndProc: Some(cal_proc),
            hInstance: HINSTANCE(module.0),
            hCursor: cursor,
            hbrBackground: GetSysColorBrush(COLOR_BTNFACE),
            lpszClassName: w!("EapoCalibrationDialog"),
            ..Default::default()
        };
        let _ = RegisterClassW(&class);
    });
    let dpi = GetDpiForWindow(parent).max(96);
    let width = scale(520, dpi);
    let height = scale(300, dpi);
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
    let result = Rc::new(RefCell::new(None));
    let state = Box::new(CalState {
        edit: HWND::default(),
        play: HWND::default(),
        font,
        volume,
        result: result.clone(),
    });
    let title = wide_z(&i18n::tr("Calibration"));
    let dialog = CreateWindowExW(
        WINDOW_EX_STYLE(0),
        w!("EapoCalibrationDialog"),
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
