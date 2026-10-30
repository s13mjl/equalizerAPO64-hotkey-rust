//! Minimal Unicode clipboard access for the native Editor.

use std::ptr::copy_nonoverlapping;

use windows::Win32::Foundation::{HANDLE, HGLOBAL};
use windows::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard,
    SetClipboardData,
};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::System::Ole::CF_UNICODETEXT;

pub unsafe fn set_text(text: &str) -> bool {
    if OpenClipboard(None).is_err() {
        return false;
    }
    let _ = EmptyClipboard();
    let mut units: Vec<u16> = text.encode_utf16().collect();
    units.push(0);
    let bytes = units.len() * 2;
    let Ok(memory) = GlobalAlloc(GMEM_MOVEABLE, bytes) else {
        let _ = CloseClipboard();
        return false;
    };
    let pointer = GlobalLock(memory) as *mut u16;
    if !pointer.is_null() {
        copy_nonoverlapping(units.as_ptr(), pointer, units.len());
        let _ = GlobalUnlock(memory);
        let _ = SetClipboardData(CF_UNICODETEXT.0 as u32, Some(HANDLE(memory.0)));
    }
    let _ = CloseClipboard();
    true
}

pub unsafe fn get_text() -> Option<String> {
    if IsClipboardFormatAvailable(CF_UNICODETEXT.0 as u32).is_err() {
        return None;
    }
    if OpenClipboard(None).is_err() {
        return None;
    }
    let result = (|| {
        let handle = GetClipboardData(CF_UNICODETEXT.0 as u32).ok()?;
        let pointer = GlobalLock(HGLOBAL(handle.0)) as *const u16;
        if pointer.is_null() {
            return None;
        }
        let mut length = 0usize;
        while *pointer.add(length) != 0 {
            length += 1;
        }
        let text = String::from_utf16_lossy(std::slice::from_raw_parts(pointer, length));
        let _ = GlobalUnlock(HGLOBAL(handle.0));
        Some(text)
    })();
    let _ = CloseClipboard();
    result
}
