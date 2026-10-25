//! Minimal pure-Rust `IAudioMediaType` implementation used when the APO has
//! to suggest a supported format.
//!
//! The original calls `CreateAudioMediaTypeFromUncompressedAudioFormat` from
//! `audiomediatypecrt.lib`. Linking that static C++ library would violate the
//! pure-Rust constraint, so the same COM object is implemented here with raw
//! vtables.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::core::{BOOL, GUID, HRESULT};
use windows::Win32::Foundation::{E_NOINTERFACE, E_OUTOFMEMORY, E_POINTER};
use windows::Win32::Media::Audio::Apo::{
    AUDIOMEDIATYPE_EQUAL_FORMAT_DATA, AUDIOMEDIATYPE_EQUAL_FORMAT_TYPES, UNCOMPRESSEDAUDIOFORMAT,
};
use windows::Win32::Media::Audio::{WAVEFORMATEX, WAVEFORMATEXTENSIBLE, WAVEFORMATEXTENSIBLE_0};

const S_OK: HRESULT = HRESULT(0);
const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
const IID_IAUDIO_MEDIA_TYPE: GUID = GUID::from_u128(0x4e997f73_b71f_4798_873b_ed7dfcf15b4d);
const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

type QiFn = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type RefFn = unsafe extern "system" fn(*mut c_void) -> u32;

#[repr(C)]
struct MediaTypeVtbl {
    query_interface: QiFn,
    add_ref: RefFn,
    release: RefFn,
    is_compressed_format: unsafe extern "system" fn(*mut c_void, *mut BOOL) -> HRESULT,
    is_equal: unsafe extern "system" fn(*mut c_void, *mut c_void, *mut u32) -> HRESULT,
    get_audio_format: unsafe extern "system" fn(*mut c_void) -> *mut WAVEFORMATEX,
    get_uncompressed_audio_format:
        unsafe extern "system" fn(*mut c_void, *mut UNCOMPRESSEDAUDIOFORMAT) -> HRESULT,
}

unsafe impl Sync for MediaTypeVtbl {}

static MEDIA_TYPE_VTBL: MediaTypeVtbl = MediaTypeVtbl {
    query_interface: media_type_query_interface,
    add_ref: media_type_add_ref,
    release: media_type_release,
    is_compressed_format: media_type_is_compressed_format,
    is_equal: media_type_is_equal,
    get_audio_format: media_type_get_audio_format,
    get_uncompressed_audio_format: media_type_get_uncompressed_audio_format,
};

#[repr(C)]
struct AudioMediaType {
    vtbl: *const MediaTypeVtbl,
    ref_count: AtomicU32,
    uncompressed: UNCOMPRESSEDAUDIOFORMAT,
    wave: WAVEFORMATEXTENSIBLE,
}

/// Creates an `IAudioMediaType` COM object with one reference, equivalent to
/// `CreateAudioMediaTypeFromUncompressedAudioFormat`.
pub(crate) unsafe fn create_audio_media_type_from_uncompressed_audio_format(
    format: &UNCOMPRESSEDAUDIOFORMAT,
) -> *mut c_void {
    let channels = format.dwSamplesPerFrame as u16;
    let bytes_per_sample = format.dwBytesPerSampleContainer as u16;
    let block_align = channels.wrapping_mul(bytes_per_sample);

    let mut wave = WAVEFORMATEXTENSIBLE::default();
    wave.Format.wFormatTag = WAVE_FORMAT_EXTENSIBLE;
    wave.Format.nChannels = channels;
    wave.Format.nSamplesPerSec = format.fFramesPerSecond as u32;
    wave.Format.nAvgBytesPerSec = format.fFramesPerSecond as u32 * block_align as u32;
    wave.Format.nBlockAlign = block_align;
    wave.Format.wBitsPerSample = (format.dwBytesPerSampleContainer * 8) as u16;
    wave.Format.cbSize = 22;
    wave.Samples = WAVEFORMATEXTENSIBLE_0 {
        wValidBitsPerSample: format.dwValidBitsPerSample as u16,
    };
    wave.dwChannelMask = format.dwChannelMask;
    wave.SubFormat = format.guidFormatType;

    let object = Box::new(AudioMediaType {
        vtbl: &MEDIA_TYPE_VTBL,
        ref_count: AtomicU32::new(1),
        uncompressed: *format,
        wave,
    });
    Box::into_raw(object) as *mut c_void
}

unsafe extern "system" fn media_type_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() {
        return E_POINTER;
    }
    unsafe {
        *ppv = std::ptr::null_mut();
    }
    if iid.is_null() {
        return E_POINTER;
    }
    let iid = unsafe { *iid };
    if iid == IID_IUNKNOWN || iid == IID_IAUDIO_MEDIA_TYPE {
        unsafe {
            media_type_add_ref(this);
            *ppv = this;
        }
        S_OK
    } else {
        E_NOINTERFACE
    }
}

unsafe extern "system" fn media_type_add_ref(this: *mut c_void) -> u32 {
    let object = unsafe { &*(this as *const AudioMediaType) };
    object.ref_count.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn media_type_release(this: *mut c_void) -> u32 {
    let object = unsafe { &*(this as *const AudioMediaType) };
    let old = object.ref_count.fetch_sub(1, Ordering::SeqCst);
    if old == 1 {
        unsafe {
            drop(Box::from_raw(this as *mut AudioMediaType));
        }
        0
    } else {
        old - 1
    }
}

unsafe extern "system" fn media_type_is_compressed_format(
    this: *mut c_void,
    compressed: *mut BOOL,
) -> HRESULT {
    let _ = this;
    if compressed.is_null() {
        return E_POINTER;
    }
    unsafe {
        *compressed = BOOL(0);
    }
    S_OK
}

unsafe extern "system" fn media_type_is_equal(
    this: *mut c_void,
    other: *mut c_void,
    flags: *mut u32,
) -> HRESULT {
    if this.is_null() || other.is_null() || flags.is_null() {
        return E_POINTER;
    }
    let this_object = unsafe { &*(this as *const AudioMediaType) };
    let other_vtbl = unsafe { *(other as *const *const MediaTypeVtbl) };
    let mut other_format = UNCOMPRESSEDAUDIOFORMAT::default();
    let hr = unsafe { ((*other_vtbl).get_uncompressed_audio_format)(other, &mut other_format) };
    if hr.is_err() {
        return hr;
    }
    let a = &this_object.uncompressed;
    let equal = a.guidFormatType == other_format.guidFormatType
        && a.dwSamplesPerFrame == other_format.dwSamplesPerFrame
        && a.dwBytesPerSampleContainer == other_format.dwBytesPerSampleContainer
        && a.dwValidBitsPerSample == other_format.dwValidBitsPerSample
        && a.fFramesPerSecond == other_format.fFramesPerSecond
        && a.dwChannelMask == other_format.dwChannelMask;
    unsafe {
        *flags = if equal {
            AUDIOMEDIATYPE_EQUAL_FORMAT_TYPES | AUDIOMEDIATYPE_EQUAL_FORMAT_DATA
        } else {
            0
        };
    }
    S_OK
}

unsafe extern "system" fn media_type_get_audio_format(this: *mut c_void) -> *mut WAVEFORMATEX {
    let object = unsafe { &mut *(this as *mut AudioMediaType) };
    std::ptr::addr_of_mut!(object.wave.Format)
}

unsafe extern "system" fn media_type_get_uncompressed_audio_format(
    this: *mut c_void,
    format: *mut UNCOMPRESSEDAUDIOFORMAT,
) -> HRESULT {
    if format.is_null() {
        return E_POINTER;
    }
    let object = unsafe { &*(this as *const AudioMediaType) };
    unsafe {
        *format = object.uncompressed;
    }
    S_OK
}

/// Keeps the error code import referenced for readers of the ABI table.
#[allow(dead_code)]
const _: HRESULT = E_OUTOFMEMORY;
