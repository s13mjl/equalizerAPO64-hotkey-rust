//! Colour palette that follows the Windows high-contrast setting.
//!
//! The editor paints everything itself, so system colours have to be read
//! explicitly instead of relying on themed controls.

use std::sync::atomic::{AtomicBool, Ordering};

use windows::Win32::Foundation::COLORREF;
use windows::Win32::Graphics::Gdi::{
    GetSysColor, COLOR_3DSHADOW, COLOR_BTNFACE, COLOR_GRAYTEXT, COLOR_HIGHLIGHT,
    COLOR_HOTLIGHT, COLOR_WINDOW, COLOR_WINDOWTEXT, SYS_COLOR_INDEX,
};
use windows::Win32::UI::Accessibility::{HCF_HIGHCONTRASTON, HIGHCONTRASTW};
use windows::Win32::UI::WindowsAndMessaging::{
    SystemParametersInfoW, SPI_GETHIGHCONTRAST, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
};

static HIGH_CONTRAST: AtomicBool = AtomicBool::new(false);

/// Re-reads the system high-contrast flag. Called at startup and whenever
/// Windows broadcasts a settings or theme change.
pub(crate) fn refresh() {
    let mut info = HIGHCONTRASTW {
        cbSize: std::mem::size_of::<HIGHCONTRASTW>() as u32,
        ..Default::default()
    };
    let enabled = unsafe {
        SystemParametersInfoW(
            SPI_GETHIGHCONTRAST,
            info.cbSize,
            Some(&mut info as *mut _ as *mut core::ffi::c_void),
            SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0),
        )
        .is_ok()
            && info.dwFlags.0 & HCF_HIGHCONTRASTON.0 != 0
    };
    HIGH_CONTRAST.store(enabled, Ordering::Relaxed);
}

pub(crate) fn high_contrast() -> bool {
    HIGH_CONTRAST.load(Ordering::Relaxed)
}

fn system(index: SYS_COLOR_INDEX, light: COLORREF) -> COLORREF {
    if high_contrast() {
        COLORREF(unsafe { GetSysColor(index) })
    } else {
        light
    }
}

pub(crate) fn white() -> COLORREF {
    system(COLOR_WINDOW, COLORREF(0x00FF_FFFF))
}

pub(crate) fn table_bg() -> COLORREF {
    system(COLOR_WINDOW, COLORREF(0x00FA_FAFA))
}

pub(crate) fn chrome_bg() -> COLORREF {
    system(COLOR_BTNFACE, COLORREF(0x00EE_EEEE))
}

pub(crate) fn border() -> COLORREF {
    system(COLOR_3DSHADOW, COLORREF(0x00C8_C8C8))
}

pub(crate) fn text() -> COLORREF {
    system(COLOR_WINDOWTEXT, COLORREF(0x0020_2020))
}

pub(crate) fn muted() -> COLORREF {
    system(COLOR_GRAYTEXT, COLORREF(0x0070_7070))
}

pub(crate) fn selected() -> COLORREF {
    system(COLOR_HIGHLIGHT, COLORREF(0x00FF_E8D8))
}

pub(crate) fn bar() -> COLORREF {
    system(COLOR_HIGHLIGHT, COLORREF(0x00C0_8040))
}

pub(crate) fn plot_bg() -> COLORREF {
    system(COLOR_WINDOW, COLORREF(0x00FF_FFFF))
}

pub(crate) fn grid() -> COLORREF {
    system(COLOR_3DSHADOW, COLORREF(0x00E0_E0E0))
}

pub(crate) fn curve() -> COLORREF {
    system(COLOR_WINDOWTEXT, COLORREF(0x0080_3030))
}

pub(crate) fn curve_pos() -> COLORREF {
    system(COLOR_HIGHLIGHT, COLORREF(0x0020_20C0))
}

pub(crate) fn fill_pos() -> COLORREF {
    system(COLOR_WINDOW, COLORREF(0x00E8_E8FF))
}

pub(crate) fn warning() -> COLORREF {
    system(COLOR_HOTLIGHT, COLORREF(0x0000_80FF))
}

pub(crate) fn hover() -> COLORREF {
    system(COLOR_BTNFACE, COLORREF(0x00F5_E9D8))
}
