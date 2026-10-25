//! Port of `EqualizerAPO.h` / `EqualizerAPO.cpp`.
//!
//! `CBaseAudioProcessingObject` does not exist for Rust, so this module
//! implements the complete COM object with raw `#[repr(C)]` vtables:
//!
//! * `IAudioProcessingObject` (primary vtable, including the methods the WDK
//!   base class used to provide: `Reset`, `GetRegistrationProperties`,
//!   `IsOutputFormatSupported`, `GetInputChannelCount`),
//! * `IAudioProcessingObjectRT` (`APOProcess`, `CalcInputFrames`,
//!   `CalcOutputFrames`),
//! * `IAudioProcessingObjectConfiguration` (`LockForProcess`,
//!   `UnlockForProcess`),
//! * `IAudioSystemEffects` (IUnknown only).
//!
//! Each interface lives at its own offset in the object, mirroring the C++
//! multiple-inheritance subobjects, and every vtable thunk recovers the
//! object base with `offset_of!`.

use std::cell::{Cell, UnsafeCell};
use std::ffi::c_void;
use std::mem::offset_of;
use std::ptr;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::OnceLock;

use eapo_apo_info::device_apo_info::{DeviceAPOInfo, APOGUID_NOKEY, APOGUID_NOVALUE, APOGUID_NULL};
use eapo_apo_info::{guid_to_string, AbstractAPOInfo};
use eapo_engine::FilterEngine;
use windows::core::{Interface, GUID, HRESULT, HSTRING};
use windows::Win32::Foundation::{
    CloseHandle, GetLastError, E_FAIL, E_INVALIDARG, E_NOINTERFACE, E_OUTOFMEMORY, E_POINTER,
    GENERIC_WRITE, HANDLE,
};
use windows::Win32::Media::Audio::Apo::{
    IAudioMediaType, IAudioProcessingObject, IAudioProcessingObjectConfiguration,
    IAudioProcessingObjectRT, APOERR_ALREADY_UNLOCKED, APOERR_APO_LOCKED,
    APOERR_FORMAT_NOT_SUPPORTED, APOERR_NUM_CONNECTIONS_INVALID, APO_CONNECTION_DESCRIPTOR,
    APO_CONNECTION_PROPERTY, APO_FLAG, APO_FLAG_BITSPERSAMPLE_MUST_MATCH,
    APO_FLAG_FRAMESPERSECOND_MUST_MATCH, APO_FLAG_INPLACE, APO_REG_PROPERTIES, BUFFER_SILENT,
    BUFFER_VALID, UNCOMPRESSEDAUDIOFORMAT,
};
use windows::Win32::Media::Audio::PKEY_AudioEndpoint_GUID;
use windows::Win32::Storage::FileSystem::{
    CreateFileW, FlushFileBuffers, WriteFile, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_NONE,
    OPEN_EXISTING,
};
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemAlloc, CLSCTX_INPROC_SERVER};
use windows::Win32::System::Pipes::WaitNamedPipeW;
use windows::Win32::UI::Shell::PropertiesSystem::IPropertyStore;

use crate::registration::{read_value, value_exists};

const S_OK: HRESULT = HRESULT(0);
const S_FALSE: HRESULT = HRESULT(1);

pub(crate) const IID_IUNKNOWN: GUID = GUID::from_u128(0x00000000_0000_0000_c000_000000000046);
pub(crate) const IID_IAUDIO_PROCESSING_OBJECT: GUID =
    GUID::from_u128(0xfd7f2b29_24d0_4b5c_b177_592c39f9ca10);
pub(crate) const IID_IAUDIO_PROCESSING_OBJECT_RT: GUID =
    GUID::from_u128(0x9e1d6a6d_ddbc_4e95_a4c7_ad64ba37846c);
pub(crate) const IID_IAUDIO_PROCESSING_OBJECT_CONFIGURATION: GUID =
    GUID::from_u128(0x0e5ed805_aba6_49c3_8f9a_2b8c889c4fa8);
pub(crate) const IID_IAUDIO_SYSTEM_EFFECTS: GUID =
    GUID::from_u128(0x5fa00f27_add6_499a_8a9d_6b98521fa75b);
pub(crate) const IID_IAUDIO_SYSTEM_EFFECTS2: GUID =
    GUID::from_u128(0xbafe99d2_7436_44ce_9e0e_4d89afbfff56);
pub(crate) const IID_IAUDIO_SYSTEM_EFFECTS3: GUID =
    GUID::from_u128(0xc58b31cd_fc6a_4255_bc1f_ad29bb0a4a17);
pub(crate) const IID_IAUDIO_PROCESSING_OBJECT_NOTIFICATIONS: GUID =
    GUID::from_u128(0x56b0c76f_02fd_4b21_a52e_9f8219fc86e4);
pub(crate) const IID_IAUDIO_PROCESSING_OBJECT_NOTIFICATIONS2: GUID =
    GUID::from_u128(0xca2cfbde_a9d6_4eb0_bc95_c4d026b380f0);
pub(crate) const IID_IAUDIO_PROCESSING_OBJECT_VBR: GUID =
    GUID::from_u128(0x7ba1db8f_78ad_49cd_9591_f79d80a17c81);
pub(crate) const IID_IAUDIO_SYSTEM_EFFECTS_CUSTOM_FORMATS: GUID =
    GUID::from_u128(0xb1176e34_bb7f_4f05_bebd_1b18a534e097);
pub(crate) const IID_IAUDIO_DEVICE_MODULES_CLIENT: GUID =
    GUID::from_u128(0x98f37dac_d0b6_49f5_896a_aa4d169a4c48);
pub(crate) const IID_IAUDIO_PROCESSING_OBJECT_RT_QUEUE_SERVICE: GUID =
    GUID::from_u128(0xacd65e2f_955b_4b57_b9bf_ac297bb752c9);

const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID =
    GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

const APP_REGPATH: &str = r"HKEY_LOCAL_MACHINE\SOFTWARE\EqualizerAPO";

/// Number of live APO instances (`EqualizerAPO::instCount`).
pub(crate) static INST_COUNT: AtomicI32 = AtomicI32::new(0);

extern "system" {
    fn eapo_cxx_base_is_format_supported(
        wrapper: *mut c_void,
        opposite_format: *mut c_void,
        requested_format: *mut c_void,
        pp_supported_format: *mut *mut c_void,
        is_input: bool,
    ) -> HRESULT;
    fn eapo_cxx_base_lock_for_process(
        wrapper: *mut c_void,
        num_input_connections: u32,
        pp_input_connections: *const *const APO_CONNECTION_DESCRIPTOR,
        num_output_connections: u32,
        pp_output_connections: *const *const APO_CONNECTION_DESCRIPTOR,
    ) -> HRESULT;
    fn eapo_cxx_base_unlock_for_process(wrapper: *mut c_void) -> HRESULT;
}

/// Diagnostic build (`live-probe`) entry point for the C++ wrapper: records the
/// audio engine's calls on the wrapper into the shared trace log. Never compiled
/// into a shipped build.
#[cfg(feature = "live-probe")]
#[no_mangle]
pub unsafe extern "system" fn eapo_rust_probe_log(text: *const core::ffi::c_char) {
    if text.is_null() {
        return;
    }
    let message = unsafe { core::ffi::CStr::from_ptr(text) }.to_string_lossy();
    eapo_common::trace_log!("PROBE {message}");
}

type QiFn = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type RefFn = unsafe extern "system" fn(*mut c_void) -> u32;
type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;

#[repr(C)]
struct IUnknownVtbl {
    query_interface: QiFn,
    add_ref: RefFn,
    release: ReleaseFn,
}

unsafe impl Sync for IUnknownVtbl {}

#[repr(C)]
struct ApoVtbl {
    query_interface: QiFn,
    add_ref: RefFn,
    release: ReleaseFn,
    reset: unsafe extern "system" fn(*mut c_void) -> HRESULT,
    get_latency: unsafe extern "system" fn(*mut c_void, *mut i64) -> HRESULT,
    get_registration_properties:
        unsafe extern "system" fn(*mut c_void, *mut *mut APO_REG_PROPERTIES) -> HRESULT,
    initialize: unsafe extern "system" fn(*mut c_void, u32, *const u8) -> HRESULT,
    is_input_format_supported: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT,
    is_output_format_supported: unsafe extern "system" fn(
        *mut c_void,
        *mut c_void,
        *mut c_void,
        *mut *mut c_void,
    ) -> HRESULT,
    get_input_channel_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
}

unsafe impl Sync for ApoVtbl {}

#[repr(C)]
struct RtVtbl {
    query_interface: QiFn,
    add_ref: RefFn,
    release: ReleaseFn,
    apo_process: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *const *const APO_CONNECTION_PROPERTY,
        u32,
        *mut *mut APO_CONNECTION_PROPERTY,
    ),
    calc_input_frames: unsafe extern "system" fn(*mut c_void, u32) -> u32,
    calc_output_frames: unsafe extern "system" fn(*mut c_void, u32) -> u32,
}

unsafe impl Sync for RtVtbl {}

#[repr(C)]
struct CfgVtbl {
    query_interface: QiFn,
    add_ref: RefFn,
    release: ReleaseFn,
    lock_for_process: unsafe extern "system" fn(
        *mut c_void,
        u32,
        *const *const APO_CONNECTION_DESCRIPTOR,
        u32,
        *const *const APO_CONNECTION_DESCRIPTOR,
    ) -> HRESULT,
    unlock_for_process: unsafe extern "system" fn(*mut c_void) -> HRESULT,
}

unsafe impl Sync for CfgVtbl {}

#[repr(C)]
struct SeVtbl {
    query_interface: QiFn,
    add_ref: RefFn,
    release: ReleaseFn,
    get_effects_list: unsafe extern "system" fn(
        *mut c_void,
        *mut *mut GUID,
        *mut u32,
        *mut c_void,
    ) -> HRESULT,
    get_controllable_system_effects_list: unsafe extern "system" fn(
        *mut c_void,
        *mut *mut c_void,
        *mut u32,
        *mut c_void,
    ) -> HRESULT,
    set_audio_system_effect_state:
        unsafe extern "system" fn(*mut c_void, *const GUID, i32) -> HRESULT,
}

unsafe impl Sync for SeVtbl {}

#[repr(C)]
struct NotifVtbl {
    query_interface: QiFn,
    add_ref: RefFn,
    release: ReleaseFn,
    get_registration_info:
        unsafe extern "system" fn(*mut c_void, *mut *mut c_void, *mut u32) -> HRESULT,
    handle_notification: unsafe extern "system" fn(*mut c_void, *mut c_void),
    get_registration_info2:
        unsafe extern "system" fn(*mut c_void, i32, *mut *mut c_void, *mut u32) -> HRESULT,
}

unsafe impl Sync for NotifVtbl {}

#[repr(C)]
struct ExtraSubobject {
    vtbl: *const c_void,
    base: *mut EqualizerApo,
}

unsafe impl Sync for ExtraSubobject {}

#[repr(C)]
struct ExtraVbrVtbl {
    query_interface: QiFn,
    add_ref: RefFn,
    release: ReleaseFn,
    calc_max_input: unsafe extern "system" fn(*mut c_void, u32, *mut u32) -> HRESULT,
    calc_max_output: unsafe extern "system" fn(*mut c_void, u32, *mut u32) -> HRESULT,
}

#[repr(C)]
struct ExtraFormatsVtbl {
    query_interface: QiFn,
    add_ref: RefFn,
    release: ReleaseFn,
    get_format_count: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
    get_format: unsafe extern "system" fn(*mut c_void, u32, *mut *mut c_void) -> HRESULT,
    get_format_representation: unsafe extern "system" fn(*mut c_void, u32, *mut *mut u16) -> HRESULT,
}

#[repr(C)]
struct ExtraDevModulesVtbl {
    query_interface: QiFn,
    add_ref: RefFn,
    release: ReleaseFn,
    set_audio_device_modules_manager: unsafe extern "system" fn(*mut c_void, *mut c_void) -> HRESULT,
}

#[repr(C)]
struct ExtraQueueVtbl {
    query_interface: QiFn,
    add_ref: RefFn,
    release: ReleaseFn,
    get_real_time_work_queue: unsafe extern "system" fn(*mut c_void, *mut u32) -> HRESULT,
}

unsafe impl Sync for ExtraVbrVtbl {}
unsafe impl Sync for ExtraFormatsVtbl {}
unsafe impl Sync for ExtraDevModulesVtbl {}
unsafe impl Sync for ExtraQueueVtbl {}

#[repr(C)]
struct NdeVtbl {
    query_interface: QiFn,
    add_ref: RefFn,
    release: ReleaseFn,
}

unsafe impl Sync for NdeVtbl {}

static APO_VTBL: ApoVtbl = ApoVtbl {
    query_interface: apo_query_interface,
    add_ref: apo_add_ref,
    release: apo_release,
    reset: apo_reset,
    get_latency: apo_get_latency,
    get_registration_properties: apo_get_registration_properties,
    initialize: apo_initialize,
    is_input_format_supported: apo_is_input_format_supported,
    is_output_format_supported: apo_is_output_format_supported,
    get_input_channel_count: apo_get_input_channel_count,
};

static RT_VTBL: RtVtbl = RtVtbl {
    query_interface: rt_query_interface,
    add_ref: rt_add_ref,
    release: rt_release,
    apo_process: rt_apo_process,
    calc_input_frames: rt_calc_input_frames,
    calc_output_frames: rt_calc_output_frames,
};

static CFG_VTBL: CfgVtbl = CfgVtbl {
    query_interface: cfg_query_interface,
    add_ref: cfg_add_ref,
    release: cfg_release,
    lock_for_process: cfg_lock_for_process,
    unlock_for_process: cfg_unlock_for_process,
};

static SE_VTBL: SeVtbl = SeVtbl {
    query_interface: se_query_interface,
    add_ref: se_add_ref,
    release: se_release,
    get_effects_list: se_get_effects_list,
    get_controllable_system_effects_list: se_get_controllable_system_effects_list,
    set_audio_system_effect_state: se_set_audio_system_effect_state,
};

static NOTIF_VTBL: NotifVtbl = NotifVtbl {
    query_interface: notif_query_interface,
    add_ref: notif_add_ref,
    release: notif_release,
    get_registration_info: notif_get_registration_info,
    handle_notification: notif_handle_notification,
    get_registration_info2: notif_get_registration_info2,
};

static VBR_VTBL: ExtraVbrVtbl = ExtraVbrVtbl {
    query_interface: extra_query_interface,
    add_ref: extra_add_ref,
    release: extra_release,
    calc_max_input: extra_calc_max_input,
    calc_max_output: extra_calc_max_output,
};

static FORMATS_VTBL: ExtraFormatsVtbl = ExtraFormatsVtbl {
    query_interface: extra_query_interface,
    add_ref: extra_add_ref,
    release: extra_release,
    get_format_count: extra_get_format_count,
    get_format: extra_get_format,
    get_format_representation: extra_get_format_representation,
};

static DEVMODULES_VTBL: ExtraDevModulesVtbl = ExtraDevModulesVtbl {
    query_interface: extra_query_interface,
    add_ref: extra_add_ref,
    release: extra_release,
    set_audio_device_modules_manager: extra_set_audio_device_modules_manager,
};

static QUEUE_VTBL: ExtraQueueVtbl = ExtraQueueVtbl {
    query_interface: extra_query_interface,
    add_ref: extra_add_ref,
    release: extra_release,
    get_real_time_work_queue: extra_get_real_time_work_queue,
};

static NDE_VTBL: NdeVtbl = NdeVtbl {
    query_interface: nde_query_interface,
    add_ref: nde_add_ref,
    release: nde_release,
};

#[derive(Default)]
struct ChildApo {
    apo: Option<IAudioProcessingObject>,
    rt: Option<IAudioProcessingObjectRT>,
    cfg: Option<IAudioProcessingObjectConfiguration>,
}

#[repr(C)]
pub(crate) struct EqualizerApo {
    // Keep every interface pointer at the front: the COM subobject pointer is
    // the address of the corresponding field and thunks subtract its offset.
    apo_vtbl: *const ApoVtbl,
    rt_vtbl: *const RtVtbl,
    cfg_vtbl: *const CfgVtbl,
    se_vtbl: *const SeVtbl,
    notif_vtbl: *const NotifVtbl,
    vbr_sub: ExtraSubobject,
    formats_sub: ExtraSubobject,
    devmodules_sub: ExtraSubobject,
    queue_sub: ExtraSubobject,
    nde_vtbl: *const NdeVtbl,
    ref_count: AtomicU32,
    clsid: GUID,
    outer: *mut c_void,
    /// The C++ `CBaseAudioProcessingObject` wrapper that owns this object, or
    /// null when the object is used standalone (unit tests).
    cxx_base: Cell<*mut c_void>,
    engine: UnsafeCell<FilterEngine>,
    allow_silent_buffer_modification: Cell<bool>,
    locked: Cell<bool>,
    initialized: Cell<bool>,
    input_channel_count: Cell<u32>,
    child: UnsafeCell<ChildApo>,
}

impl EqualizerApo {
    /// Gives the object access to the C++ `CBaseAudioProcessingObject` that
    /// owns it, so format/lock calls reach the real WDK base class.
    pub(crate) unsafe fn attach_cxx_base(interface: *mut c_void, cxx_base: *mut c_void) {
        if interface.is_null() {
            return;
        }
        unsafe {
            (*interface.cast::<EqualizerApo>())
                .cxx_base
                .set(cxx_base);
        }
    }

    /// Allocates one object with refcount 1 (C++ `new EqualizerAPO`).
    pub(crate) unsafe fn create_instance(outer: *mut c_void, clsid: GUID) -> *mut EqualizerApo {
        let mut object = Box::new(EqualizerApo {
            apo_vtbl: &APO_VTBL,
            rt_vtbl: &RT_VTBL,
            cfg_vtbl: &CFG_VTBL,
            se_vtbl: &SE_VTBL,
            notif_vtbl: &NOTIF_VTBL,
            vbr_sub: ExtraSubobject {
                vtbl: &VBR_VTBL as *const ExtraVbrVtbl as *const c_void,
                base: ptr::null_mut(),
            },
            formats_sub: ExtraSubobject {
                vtbl: &FORMATS_VTBL as *const ExtraFormatsVtbl as *const c_void,
                base: ptr::null_mut(),
            },
            devmodules_sub: ExtraSubobject {
                vtbl: &DEVMODULES_VTBL as *const ExtraDevModulesVtbl as *const c_void,
                base: ptr::null_mut(),
            },
            queue_sub: ExtraSubobject {
                vtbl: &QUEUE_VTBL as *const ExtraQueueVtbl as *const c_void,
                base: ptr::null_mut(),
            },
            nde_vtbl: &NDE_VTBL,
            ref_count: AtomicU32::new(1),
            clsid,
            outer,
            cxx_base: Cell::new(ptr::null_mut()),
            engine: UnsafeCell::new(FilterEngine::new()),
            allow_silent_buffer_modification: Cell::new(false),
            locked: Cell::new(false),
            initialized: Cell::new(false),
            input_channel_count: Cell::new(0),
            child: UnsafeCell::new(ChildApo::default()),
        });
        let base = &mut *object as *mut EqualizerApo;
        object.vbr_sub.base = base;
        object.formats_sub.base = base;
        object.devmodules_sub.base = base;
        object.queue_sub.base = base;
        INST_COUNT.fetch_add(1, Ordering::SeqCst);
        Box::into_raw(object)
    }

    pub(crate) unsafe fn non_delegating_query_interface(
        this: *mut EqualizerApo,
        iid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> HRESULT {
        if ppv.is_null() {
            return E_POINTER;
        }
        unsafe {
            *ppv = ptr::null_mut();
        }
        if iid.is_null() {
            return E_POINTER;
        }
        let iid = unsafe { *iid };
        let object = unsafe { &mut *this };
        let ptr: *mut c_void = if iid == IID_IUNKNOWN {
            // C++ returns static_cast<INonDelegatingUnknown*>(this), whose first
            // three slots are the NonDelegating* methods.
            std::ptr::addr_of_mut!(object.nde_vtbl) as *mut c_void
        } else if iid == IID_IAUDIO_PROCESSING_OBJECT {
            this as *mut c_void
        } else if iid == IID_IAUDIO_PROCESSING_OBJECT_RT {
            std::ptr::addr_of_mut!(object.rt_vtbl) as *mut c_void
        } else if iid == IID_IAUDIO_PROCESSING_OBJECT_CONFIGURATION {
            std::ptr::addr_of_mut!(object.cfg_vtbl) as *mut c_void
        } else if iid == IID_IAUDIO_SYSTEM_EFFECTS {
            std::ptr::addr_of_mut!(object.se_vtbl) as *mut c_void
        } else if iid == IID_IAUDIO_SYSTEM_EFFECTS2 || iid == IID_IAUDIO_SYSTEM_EFFECTS3 {
            std::ptr::addr_of_mut!(object.se_vtbl) as *mut c_void
        } else if iid == IID_IAUDIO_PROCESSING_OBJECT_NOTIFICATIONS
            || iid == IID_IAUDIO_PROCESSING_OBJECT_NOTIFICATIONS2
        {
            std::ptr::addr_of_mut!(object.notif_vtbl) as *mut c_void
        } else if iid == IID_IAUDIO_PROCESSING_OBJECT_VBR {
            std::ptr::addr_of_mut!(object.vbr_sub) as *mut c_void
        } else if iid == IID_IAUDIO_SYSTEM_EFFECTS_CUSTOM_FORMATS {
            std::ptr::addr_of_mut!(object.formats_sub) as *mut c_void
        } else if iid == IID_IAUDIO_DEVICE_MODULES_CLIENT {
            std::ptr::addr_of_mut!(object.devmodules_sub) as *mut c_void
        } else if iid == IID_IAUDIO_PROCESSING_OBJECT_RT_QUEUE_SERVICE {
            std::ptr::addr_of_mut!(object.queue_sub) as *mut c_void
        } else {
            return E_NOINTERFACE;
        };

        object.ref_count.fetch_add(1, Ordering::SeqCst);
        unsafe {
            *ppv = ptr;
        }
        S_OK
    }

    pub(crate) unsafe fn non_delegating_release(this: *mut EqualizerApo) -> u32 {
        let object = unsafe { &*this };
        let old = object.ref_count.fetch_sub(1, Ordering::SeqCst);
        if old == 1 {
            #[cfg(feature = "live-probe")]
            eapo_common::trace_log!("PROBE inner-drop this={this:p} clsid1={:08X}", object.clsid.data1);
            unsafe {
                drop(Box::from_raw(this));
            }
            0
        } else {
            old - 1
        }
    }

    fn reset_child(&self) {
        let child = unsafe { &mut *self.child.get() };
        // Release order matches the C++ resetChild ordering.
        child.apo.take();
        child.rt.take();
        child.cfg.take();
    }

    fn child_has_configuration(&self) -> bool {
        unsafe { (*self.child.get()).cfg.is_some() }
    }

    fn child_has_apo(&self) -> bool {
        unsafe { (*self.child.get()).apo.is_some() }
    }

    unsafe fn get_uncompressed_format(
        media_type: *mut c_void,
    ) -> Result<UNCOMPRESSEDAUDIOFORMAT, HRESULT> {
        let media_ptr = media_type;
        let media = unsafe { IAudioMediaType::from_raw_borrowed(&media_ptr) };
        let Some(media) = media else {
            return Err(E_POINTER);
        };
        let mut format = UNCOMPRESSEDAUDIOFORMAT::default();
        match unsafe { media.GetUncompressedAudioFormat(&mut format) } {
            Ok(()) => Ok(format),
            Err(e) => Err(e.code()),
        }
    }

    /// `CBaseAudioProcessingObject::IsInputFormatSupported` /
    /// `IsOutputFormatSupported`. When the object is owned by the C++ wrapper
    /// the real WDK base class answers; standalone (unit tests) the Rust
    /// reimplementation below is used. Both follow the same rule: IEEE float32
    /// with matching frame rate and bit depth, channel count may differ
    /// (`APO_FLAG_SAMPLESPERFRAME_MUST_MATCH` is not used).
    unsafe fn base_is_format_supported(
        &self,
        opposite_format: *mut c_void,
        requested_format: *mut c_void,
        pp_supported_format: *mut *mut c_void,
        is_input: bool,
    ) -> HRESULT {
        let cxx_base = self.cxx_base.get();
        if !cxx_base.is_null() {
            return unsafe {
                eapo_cxx_base_is_format_supported(
                    cxx_base,
                    opposite_format,
                    requested_format,
                    pp_supported_format,
                    is_input,
                )
            };
        }
        Self::rust_base_is_format_supported(
            opposite_format,
            requested_format,
            pp_supported_format,
        )
    }

    unsafe fn rust_base_is_format_supported(
        opposite_format: *mut c_void,
        requested_format: *mut c_void,
        pp_supported_format: *mut *mut c_void,
    ) -> HRESULT {
        if requested_format.is_null() {
            return E_POINTER;
        }
        let format = match unsafe { Self::get_uncompressed_format(requested_format) } {
            Ok(f) => f,
            Err(e) => {
                eapo_common::error_log!("Error in GetUncompressedAudioFormat");
                return e;
            }
        };
        if opposite_format.is_null() {
            return E_POINTER;
        }
        let opposite = match unsafe { Self::get_uncompressed_format(opposite_format) } {
            Ok(f) => f,
            Err(e) => {
                eapo_common::error_log!("Error in second GetUncompressedAudioFormat");
                return e;
            }
        };

        let requested_is_float32 = format.guidFormatType == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
            && format.dwBytesPerSampleContainer == 4;
        let opposite_is_float32 = opposite.guidFormatType == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
            && opposite.dwBytesPerSampleContainer == 4;

        let formats_match = requested_is_float32
            && opposite_is_float32
            && format.fFramesPerSecond == opposite.fFramesPerSecond
            && format.dwBytesPerSampleContainer == opposite.dwBytesPerSampleContainer
            && format.dwValidBitsPerSample == opposite.dwValidBitsPerSample;

        if formats_match {
            // IAudioProcessingObject::IsInputFormatSupported requires the
            // engine's own pRequestedInputFormat pointer to be returned (with
            // an added reference) on S_OK.  NULL here crashes audioeng.dll.
            if !pp_supported_format.is_null() {
                #[repr(C)]
                struct UnknownVtbl {
                    query_interface: unsafe extern "system" fn(
                        *mut c_void,
                        *const GUID,
                        *mut *mut c_void,
                    ) -> HRESULT,
                    add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
                    release: unsafe extern "system" fn(*mut c_void) -> u32,
                }
                unsafe {
                    let vtbl = *(requested_format as *const *const UnknownVtbl);
                    ((*vtbl).add_ref)(requested_format);
                    *pp_supported_format = requested_format;
                }
            }
            return S_OK;
        }

        if !pp_supported_format.is_null() {
            if opposite_is_float32 {
                let media = unsafe {
                    crate::media_type::create_audio_media_type_from_uncompressed_audio_format(
                        &opposite,
                    )
                };
                if !media.is_null() {
                    unsafe {
                        *pp_supported_format = media;
                    }
                    return S_FALSE;
                }
                return E_OUTOFMEMORY;
            }
        }

        APOERR_FORMAT_NOT_SUPPORTED
    }

    unsafe fn is_input_format_supported_impl(
        &mut self,
        output_format: *mut c_void,
        requested_input_format: *mut c_void,
        pp_supported_input_format: *mut *mut c_void,
    ) -> HRESULT {
        if requested_input_format.is_null() {
            return E_POINTER;
        }

        let in_format = match unsafe { Self::get_uncompressed_format(requested_input_format) } {
            Ok(f) => f,
            Err(e) => {
                eapo_common::error_log!("Error in GetUncompressedAudioFormat");
                return e;
            }
        };
        trace_format("RequestedInputFormat", &in_format);

        if output_format.is_null() {
            return E_POINTER;
        }
        let out_format = match unsafe { Self::get_uncompressed_format(output_format) } {
            Ok(f) => f,
            Err(e) => {
                eapo_common::error_log!("Error in second GetUncompressedAudioFormat");
                return e;
            }
        };
        trace_format("Output format", &out_format);

        let mut hr = E_FAIL;
        if let Some(child_apo) = unsafe { (*self.child.get()).apo.as_ref() } {
            let vtable = Interface::vtable(child_apo);
            hr = unsafe {
                (vtable.IsInputFormatSupported)(
                    Interface::as_raw(child_apo),
                    output_format,
                    requested_input_format,
                    pp_supported_input_format,
                )
            };
            if hr.is_ok() {
                eapo_common::trace_log!("Success in IsInputFormatSupported of child apo");
            } else {
                eapo_common::error_log!("Failure in IsInputFormatSupported of child apo");
                self.reset_child();
            }
        }

        if !self.child_has_apo() || !hr.is_ok() {
            hr = unsafe {
                self.base_is_format_supported(
                    output_format,
                    requested_input_format,
                    pp_supported_input_format,
                    true,
                )
            };

            // we do not support downmixing currently
            if hr == S_OK
                && in_format.dwSamplesPerFrame > 2
                && in_format.dwSamplesPerFrame > out_format.dwSamplesPerFrame
            {
                if !pp_supported_input_format.is_null() {
                    let media = unsafe {
                        crate::media_type::create_audio_media_type_from_uncompressed_audio_format(
                            &out_format,
                        )
                    };
                    unsafe {
                        *pp_supported_input_format = media;
                    }
                    hr = S_FALSE;
                }
            }
        }

        if hr == S_FALSE {
            if pp_supported_input_format.is_null() {
                return E_POINTER;
            }
            let supported = unsafe { *pp_supported_input_format };
            if supported.is_null() {
                eapo_common::error_log!("Error in third GetUncompressedAudioFormat");
                return E_POINTER;
            }
            match unsafe { Self::get_uncompressed_format(supported) } {
                Ok(supported_format) => {
                    trace_format(
                        "InputFormat not accepted, SupportedInputFormat",
                        &supported_format,
                    );
                }
                Err(e) => {
                    eapo_common::error_log!("Error in third GetUncompressedAudioFormat");
                    return e;
                }
            }
        } else if hr == S_OK {
            eapo_common::trace_log!("InputFormat accepted");
        }

        hr
    }

    unsafe fn lock_for_process_impl(
        &mut self,
        num_input_connections: u32,
        pp_input_connections: *const *const APO_CONNECTION_DESCRIPTOR,
        num_output_connections: u32,
        pp_output_connections: *const *const APO_CONNECTION_DESCRIPTOR,
    ) -> HRESULT {
        if pp_input_connections.is_null() || pp_output_connections.is_null() {
            return APOERR_NUM_CONNECTIONS_INVALID;
        }
        let in_descriptor = unsafe { *pp_input_connections };
        let out_descriptor = unsafe { *pp_output_connections };
        if in_descriptor.is_null() || out_descriptor.is_null() {
            return APOERR_NUM_CONNECTIONS_INVALID;
        }

        let in_format_ptr = unsafe {
            let format = &*(*in_descriptor).pFormat;
            format
                .as_ref()
                .map(|format| format.as_raw())
                .unwrap_or(ptr::null_mut())
        };
        let in_format = match unsafe { Self::get_uncompressed_format(in_format_ptr) } {
            Ok(f) => f,
            Err(e) => {
                eapo_common::error_log!("Error in GetUncompressedAudioFormat in LockForProcess");
                return e;
            }
        };
        let max_input_frame_count = unsafe { (*in_descriptor).u32MaxFrameCount };
        trace_format_with_frames(
            "Input format in LockForProcess",
            &in_format,
            max_input_frame_count,
        );

        let out_format_ptr = unsafe {
            let format = &*(*out_descriptor).pFormat;
            format
                .as_ref()
                .map(|format| format.as_raw())
                .unwrap_or(ptr::null_mut())
        };
        let out_format = match unsafe { Self::get_uncompressed_format(out_format_ptr) } {
            Ok(f) => f,
            Err(e) => {
                eapo_common::error_log!(
                    "Error in second GetUncompressedAudioFormat in LockForProcess"
                );
                return e;
            }
        };
        let max_output_frame_count = unsafe { (*out_descriptor).u32MaxFrameCount };
        trace_format_with_frames(
            "Output format in LockForProcess",
            &out_format,
            max_output_frame_count,
        );

        if let Some(child_cfg) = unsafe { (*self.child.get()).cfg.as_ref() } {
            let inputs = unsafe {
                std::slice::from_raw_parts(pp_input_connections, num_input_connections as usize)
            };
            let outputs = unsafe {
                std::slice::from_raw_parts(pp_output_connections, num_output_connections as usize)
            };
            if unsafe { child_cfg.LockForProcess(inputs, outputs) }.is_ok() {
                eapo_common::trace_log!("Success in LockForProcess of child apo");
            }
        }

        let hr = unsafe {
            self.base_lock_for_process(
                num_input_connections,
                pp_input_connections,
                num_output_connections,
                pp_output_connections,
                &in_format,
                &out_format,
            )
        };
        if hr.is_err() {
            eapo_common::error_log!("Error in CBaseAudioProcessingObject::LockForProcess");
            return hr;
        } else if hr == S_OK {
            eapo_common::trace_log!("LockForProcess successful");
        }

        let mut max_frame_count = max_input_frame_count;
        if max_frame_count == 0 {
            max_frame_count = max_output_frame_count;
        }

        let real_channel_count = if self.child_has_configuration() {
            out_format.dwSamplesPerFrame
        } else {
            in_format.dwSamplesPerFrame
        };

        let engine = unsafe { &mut *self.engine.get() };
        let channel_mask = if engine.is_capture() {
            let mut mask = in_format.dwChannelMask;
            if mask == 0 && in_format.dwSamplesPerFrame == out_format.dwSamplesPerFrame {
                mask = out_format.dwChannelMask;
            }
            mask
        } else {
            let mut mask = out_format.dwChannelMask;
            if mask == 0 && in_format.dwSamplesPerFrame == out_format.dwSamplesPerFrame {
                mask = in_format.dwChannelMask;
            }
            mask
        };

        engine.initialize(
            out_format.fFramesPerSecond,
            in_format.dwSamplesPerFrame,
            real_channel_count,
            out_format.dwSamplesPerFrame,
            channel_mask,
            max_frame_count,
            "",
        );
        hr
    }

    unsafe fn base_lock_for_process(
        &mut self,
        num_input_connections: u32,
        pp_input_connections: *const *const APO_CONNECTION_DESCRIPTOR,
        num_output_connections: u32,
        pp_output_connections: *const *const APO_CONNECTION_DESCRIPTOR,
        in_format: &UNCOMPRESSEDAUDIOFORMAT,
        out_format: &UNCOMPRESSEDAUDIOFORMAT,
    ) -> HRESULT {
        let cxx_base = self.cxx_base.get();
        if !cxx_base.is_null() {
            let hr = unsafe {
                eapo_cxx_base_lock_for_process(
                    cxx_base,
                    num_input_connections,
                    pp_input_connections,
                    num_output_connections,
                    pp_output_connections,
                )
            };
            if hr.is_ok() {
                self.input_channel_count.set(in_format.dwSamplesPerFrame);
                self.locked.set(true);
                self.initialized.set(true);
            }
            return hr;
        }
        if num_input_connections != 1 || num_output_connections != 1 {
            return APOERR_NUM_CONNECTIONS_INVALID;
        }
        if self.locked.get() {
            return APOERR_APO_LOCKED;
        }
        let in_is_float32 = in_format.guidFormatType == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
            && in_format.dwBytesPerSampleContainer == 4;
        let out_is_float32 = out_format.guidFormatType == KSDATAFORMAT_SUBTYPE_IEEE_FLOAT
            && out_format.dwBytesPerSampleContainer == 4;
        if !in_is_float32
            || !out_is_float32
            || in_format.fFramesPerSecond != out_format.fFramesPerSecond
            || in_format.dwBytesPerSampleContainer != out_format.dwBytesPerSampleContainer
            || in_format.dwValidBitsPerSample != out_format.dwValidBitsPerSample
        {
            return APOERR_FORMAT_NOT_SUPPORTED;
        }

        self.input_channel_count.set(in_format.dwSamplesPerFrame);
        self.locked.set(true);
        self.initialized.set(true);
        S_OK
    }

    unsafe fn base_unlock_for_process(&mut self) -> HRESULT {
        let cxx_base = self.cxx_base.get();
        if !cxx_base.is_null() {
            let hr = unsafe { eapo_cxx_base_unlock_for_process(cxx_base) };
            self.locked.set(false);
            return hr;
        }
        self.locked.set(false);
        S_OK
    }

    unsafe fn apo_process_impl(
        &mut self,
        num_input_connections: u32,
        pp_input_connections: *const *const APO_CONNECTION_PROPERTY,
        num_output_connections: u32,
        pp_output_connections: *mut *mut APO_CONNECTION_PROPERTY,
    ) {
        if pp_input_connections.is_null() || pp_output_connections.is_null() {
            return;
        }
        let Some(input_connection) = (unsafe { pp_input_connections.as_ref() }) else {
            return;
        };
        let Some(output_connection) = (unsafe { pp_output_connections.as_ref() }) else {
            return;
        };
        if input_connection.is_null() || output_connection.is_null() {
            return;
        }

        let input_connection = unsafe { &**input_connection };
        let output_connection = unsafe { &mut **output_connection };
        match input_connection.u32BufferFlags {
            flags if flags == BUFFER_VALID || flags == BUFFER_SILENT => {
                let input_frames = input_connection.pBuffer as *mut f32;
                let output_frames = output_connection.pBuffer as *mut f32;
                let frame_count = input_connection.u32ValidFrameCount;

                let engine = unsafe { &mut *self.engine.get() };

                if input_connection.u32BufferFlags == BUFFER_SILENT && !input_frames.is_null() {
                    unsafe {
                        ptr::write_bytes(
                            input_frames,
                            0,
                            frame_count as usize * engine.get_input_channel_count() as usize,
                        );
                    }
                }

                if let Some(child_rt) = unsafe { (*self.child.get()).rt.as_ref() } {
                    unsafe {
                        child_rt.APOProcess(
                            num_input_connections,
                            pp_input_connections,
                            num_output_connections,
                            pp_output_connections,
                        );
                    }
                    unsafe {
                        engine.process_f32_interleaved(output_frames, output_frames, frame_count);
                    }
                } else {
                    unsafe {
                        engine.process_f32_interleaved(output_frames, input_frames, frame_count);
                    }
                }

                output_connection.u32ValidFrameCount = input_connection.u32ValidFrameCount;

                if input_connection.u32BufferFlags == BUFFER_SILENT {
                    if self.allow_silent_buffer_modification.get() {
                        let output_frame_count = output_connection.u32ValidFrameCount as usize
                            * engine.get_output_channel_count() as usize;
                        let mut silent = true;
                        for i in 0..output_frame_count {
                            if unsafe { *output_frames.add(i) }.abs() > 1e-10f32 {
                                silent = false;
                                break;
                            }
                        }
                        // BUFFER_SILENT seems to be important for some sound card drivers,
                        // so only use BUFFER_VALID if there really is audio
                        output_connection.u32BufferFlags =
                            if silent { BUFFER_SILENT } else { BUFFER_VALID };
                    } else {
                        if !output_frames.is_null() {
                            unsafe {
                                ptr::write_bytes(
                                    output_frames,
                                    0,
                                    output_connection.u32ValidFrameCount as usize
                                        * engine.get_output_channel_count() as usize,
                                );
                            }
                        }
                        output_connection.u32BufferFlags = BUFFER_SILENT;
                    }
                } else {
                    output_connection.u32BufferFlags = BUFFER_VALID;
                }
            }
            _ => {}
        }
    }
}

impl Drop for EqualizerApo {
    fn drop(&mut self) {
        INST_COUNT.fetch_sub(1, Ordering::SeqCst);
        self.reset_child();
    }
}

unsafe fn base_from_apo(this: *mut c_void) -> *mut EqualizerApo {
    this as *mut EqualizerApo
}

unsafe fn base_from_rt(this: *mut c_void) -> *mut EqualizerApo {
    unsafe { (this as *mut u8).sub(offset_of!(EqualizerApo, rt_vtbl)) as *mut EqualizerApo }
}

unsafe fn base_from_cfg(this: *mut c_void) -> *mut EqualizerApo {
    unsafe { (this as *mut u8).sub(offset_of!(EqualizerApo, cfg_vtbl)) as *mut EqualizerApo }
}

unsafe fn base_from_se(this: *mut c_void) -> *mut EqualizerApo {
    unsafe { (this as *mut u8).sub(offset_of!(EqualizerApo, se_vtbl)) as *mut EqualizerApo }
}

unsafe fn base_from_notif(this: *mut c_void) -> *mut EqualizerApo {
    unsafe { (this as *mut u8).sub(offset_of!(EqualizerApo, notif_vtbl)) as *mut EqualizerApo }
}

unsafe fn base_from_nde(this: *mut c_void) -> *mut EqualizerApo {
    unsafe { (this as *mut u8).sub(offset_of!(EqualizerApo, nde_vtbl)) as *mut EqualizerApo }
}

unsafe fn query_interface_common(
    this: *mut c_void,
    base: *mut EqualizerApo,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    let _ = this;
    if ppv.is_null() {
        return E_POINTER;
    }
    unsafe {
        *ppv = ptr::null_mut();
    }
    let object = unsafe { &*base };
    if !object.outer.is_null() {
        let vtbl = unsafe { *(object.outer as *const *const IUnknownVtbl) };
        return unsafe { ((*vtbl).query_interface)(object.outer, iid, ppv) };
    }
    unsafe { EqualizerApo::non_delegating_query_interface(base, iid, ppv) }
}

unsafe fn add_ref_common(base: *mut EqualizerApo) -> u32 {
    let object = unsafe { &*base };
    if !object.outer.is_null() {
        let vtbl = unsafe { *(object.outer as *const *const IUnknownVtbl) };
        return unsafe { ((*vtbl).add_ref)(object.outer) };
    }
    object.ref_count.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe fn release_common(base: *mut EqualizerApo) -> u32 {
    let object = unsafe { &*base };
    if !object.outer.is_null() {
        let vtbl = unsafe { *(object.outer as *const *const IUnknownVtbl) };
        return unsafe { ((*vtbl).release)(object.outer) };
    }
    unsafe { EqualizerApo::non_delegating_release(base) }
}

unsafe extern "system" fn apo_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    let base = unsafe { base_from_apo(this) };
    unsafe { query_interface_common(this, base, iid, ppv) }
}

unsafe extern "system" fn rt_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    let base = unsafe { base_from_rt(this) };
    unsafe { query_interface_common(this, base, iid, ppv) }
}

unsafe extern "system" fn cfg_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    let base = unsafe { base_from_cfg(this) };
    unsafe { query_interface_common(this, base, iid, ppv) }
}

unsafe extern "system" fn se_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    let base = unsafe { base_from_se(this) };
    unsafe { query_interface_common(this, base, iid, ppv) }
}

unsafe extern "system" fn notif_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    let base = unsafe { base_from_notif(this) };
    unsafe { query_interface_common(this, base, iid, ppv) }
}

unsafe extern "system" fn notif_add_ref(this: *mut c_void) -> u32 {
    unsafe { add_ref_common(base_from_notif(this)) }
}

unsafe extern "system" fn notif_release(this: *mut c_void) -> u32 {
    unsafe { release_common(base_from_notif(this)) }
}

unsafe extern "system" fn notif_get_registration_info(
    _this: *mut c_void,
    descriptors: *mut *mut c_void,
    count: *mut u32,
) -> HRESULT {
    if !descriptors.is_null() {
        unsafe { *descriptors = ptr::null_mut() };
    }
    if !count.is_null() {
        unsafe { *count = 0 };
    }
    S_OK
}

unsafe extern "system" fn notif_handle_notification(
    _this: *mut c_void,
    _notification: *mut c_void,
) {
}

unsafe extern "system" fn notif_get_registration_info2(
    _this: *mut c_void,
    _max_type: i32,
    descriptors: *mut *mut c_void,
    count: *mut u32,
) -> HRESULT {
    if !descriptors.is_null() {
        unsafe { *descriptors = ptr::null_mut() };
    }
    if !count.is_null() {
        unsafe { *count = 0 };
    }
    S_OK
}

unsafe fn extra_base(this: *mut c_void) -> *mut EqualizerApo {
    let base_field =
        (this as *const u8).add(std::mem::size_of::<usize>()) as *const *mut EqualizerApo;
    unsafe { *base_field }
}

unsafe extern "system" fn extra_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    let base = unsafe { extra_base(this) };
    unsafe { query_interface_common(this, base, iid, ppv) }
}

unsafe extern "system" fn extra_add_ref(this: *mut c_void) -> u32 {
    unsafe { add_ref_common(extra_base(this)) }
}

unsafe extern "system" fn extra_release(this: *mut c_void) -> u32 {
    unsafe { release_common(extra_base(this)) }
}

unsafe extern "system" fn extra_calc_max_input(
    _this: *mut c_void,
    max_output_frames: u32,
    input_frames: *mut u32,
) -> HRESULT {
    if input_frames.is_null() {
        return E_POINTER;
    }
    unsafe { *input_frames = max_output_frames };
    S_OK
}

unsafe extern "system" fn extra_calc_max_output(
    _this: *mut c_void,
    max_input_frames: u32,
    output_frames: *mut u32,
) -> HRESULT {
    if output_frames.is_null() {
        return E_POINTER;
    }
    unsafe { *output_frames = max_input_frames };
    S_OK
}

unsafe extern "system" fn extra_get_format_count(
    _this: *mut c_void,
    count: *mut u32,
) -> HRESULT {
    if count.is_null() {
        return E_POINTER;
    }
    unsafe { *count = 0 };
    S_OK
}

unsafe extern "system" fn extra_get_format(
    _this: *mut c_void,
    _index: u32,
    format: *mut *mut c_void,
) -> HRESULT {
    if format.is_null() {
        return E_POINTER;
    }
    unsafe { *format = ptr::null_mut() };
    E_INVALIDARG
}

unsafe extern "system" fn extra_get_format_representation(
    _this: *mut c_void,
    _index: u32,
    representation: *mut *mut u16,
) -> HRESULT {
    if representation.is_null() {
        return E_POINTER;
    }
    unsafe { *representation = ptr::null_mut() };
    E_INVALIDARG
}

unsafe extern "system" fn extra_set_audio_device_modules_manager(
    _this: *mut c_void,
    _manager: *mut c_void,
) -> HRESULT {
    S_OK
}

unsafe extern "system" fn extra_get_real_time_work_queue(
    _this: *mut c_void,
    queue_id: *mut u32,
) -> HRESULT {
    if queue_id.is_null() {
        return E_POINTER;
    }
    unsafe { *queue_id = 0 };
    S_OK
}

unsafe extern "system" fn se_get_effects_list(
    _this: *mut c_void,
    effects: *mut *mut GUID,
    count: *mut u32,
    _event: *mut c_void,
) -> HRESULT {
    if !effects.is_null() {
        unsafe { *effects = ptr::null_mut() };
    }
    if !count.is_null() {
        unsafe { *count = 0 };
    }
    S_OK
}

unsafe extern "system" fn se_get_controllable_system_effects_list(
    _this: *mut c_void,
    effects: *mut *mut c_void,
    count: *mut u32,
    _event: *mut c_void,
) -> HRESULT {
    if !effects.is_null() {
        unsafe { *effects = ptr::null_mut() };
    }
    if !count.is_null() {
        unsafe { *count = 0 };
    }
    S_OK
}

unsafe extern "system" fn se_set_audio_system_effect_state(
    _this: *mut c_void,
    _effect: *const GUID,
    _state: i32,
) -> HRESULT {
    S_OK
}

unsafe extern "system" fn apo_add_ref(this: *mut c_void) -> u32 {
    unsafe { add_ref_common(base_from_apo(this)) }
}

unsafe extern "system" fn rt_add_ref(this: *mut c_void) -> u32 {
    unsafe { add_ref_common(base_from_rt(this)) }
}

unsafe extern "system" fn cfg_add_ref(this: *mut c_void) -> u32 {
    unsafe { add_ref_common(base_from_cfg(this)) }
}

unsafe extern "system" fn se_add_ref(this: *mut c_void) -> u32 {
    unsafe { add_ref_common(base_from_se(this)) }
}

unsafe extern "system" fn apo_release(this: *mut c_void) -> u32 {
    unsafe { release_common(base_from_apo(this)) }
}

unsafe extern "system" fn rt_release(this: *mut c_void) -> u32 {
    unsafe { release_common(base_from_rt(this)) }
}

unsafe extern "system" fn cfg_release(this: *mut c_void) -> u32 {
    unsafe { release_common(base_from_cfg(this)) }
}

unsafe extern "system" fn se_release(this: *mut c_void) -> u32 {
    unsafe { release_common(base_from_se(this)) }
}

unsafe extern "system" fn nde_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    let base = unsafe { base_from_nde(this) };
    unsafe { EqualizerApo::non_delegating_query_interface(base, iid, ppv) }
}

unsafe extern "system" fn nde_add_ref(this: *mut c_void) -> u32 {
    let base = unsafe { base_from_nde(this) };
    let object = unsafe { &*base };
    object.ref_count.fetch_add(1, Ordering::SeqCst) + 1
}

unsafe extern "system" fn nde_release(this: *mut c_void) -> u32 {
    let base = unsafe { base_from_nde(this) };
    unsafe { EqualizerApo::non_delegating_release(base) }
}

unsafe extern "system" fn apo_reset(this: *mut c_void) -> HRESULT {
    let _ = this;
    // CBaseAudioProcessingObject::Reset is a no-op in the WDK base class.
    S_OK
}

unsafe extern "system" fn apo_get_latency(this: *mut c_void, time: *mut i64) -> HRESULT {
    if time.is_null() {
        return E_POINTER;
    }
    let object = unsafe { &mut *base_from_apo(this) };
    if !object.locked.get() {
        return APOERR_ALREADY_UNLOCKED;
    }
    unsafe {
        *time = 0;
    }
    if let Some(child_apo) = unsafe { (*object.child.get()).apo.as_ref() } {
        if let Ok(latency) = unsafe { child_apo.GetLatency() } {
            unsafe {
                *time = latency;
            }
        }
    }
    S_OK
}

unsafe extern "system" fn apo_get_registration_properties(
    this: *mut c_void,
    pp_reg_props: *mut *mut APO_REG_PROPERTIES,
) -> HRESULT {
    if pp_reg_props.is_null() {
        return E_POINTER;
    }
    let object = unsafe { &*base_from_apo(this) };
    let properties = properties_for_clsid(&object.clsid);
    let size = std::mem::size_of::<APO_REG_PROPERTIES>();
    let memory = unsafe { CoTaskMemAlloc(size) } as *mut APO_REG_PROPERTIES;
    if memory.is_null() {
        return E_OUTOFMEMORY;
    }
    unsafe {
        ptr::write(memory, *properties);
        *pp_reg_props = memory;
    }
    S_OK
}

unsafe extern "system" fn apo_initialize(
    this: *mut c_void,
    cb_data_size: u32,
    pby_data: *const u8,
) -> HRESULT {
    let object = unsafe { &mut *base_from_apo(this) };
    let hr = unsafe { object.initialize_impl(cb_data_size, pby_data) };
    hr
}

unsafe extern "system" fn apo_is_input_format_supported(
    this: *mut c_void,
    output_format: *mut c_void,
    requested_input_format: *mut c_void,
    pp_supported_input_format: *mut *mut c_void,
) -> HRESULT {
    let object = unsafe { &mut *base_from_apo(this) };
    unsafe {
        object.is_input_format_supported_impl(
            output_format,
            requested_input_format,
            pp_supported_input_format,
        )
    }
}

unsafe extern "system" fn apo_is_output_format_supported(
    this: *mut c_void,
    input_format: *mut c_void,
    requested_output_format: *mut c_void,
    pp_supported_output_format: *mut *mut c_void,
) -> HRESULT {
    let object = unsafe { &mut *base_from_apo(this) };
    let hr = unsafe {
        object.is_output_format_supported_impl(
            input_format,
            requested_output_format,
            pp_supported_output_format,
        )
    };
    hr
}

unsafe extern "system" fn apo_get_input_channel_count(
    this: *mut c_void,
    channel_count: *mut u32,
) -> HRESULT {
    if channel_count.is_null() {
        return E_POINTER;
    }
    let object = unsafe { &*base_from_apo(this) };
    unsafe {
        *channel_count = object.input_channel_count.get();
    }
    S_OK
}

unsafe extern "system" fn rt_apo_process(
    this: *mut c_void,
    num_input_connections: u32,
    pp_input_connections: *const *const APO_CONNECTION_PROPERTY,
    num_output_connections: u32,
    pp_output_connections: *mut *mut APO_CONNECTION_PROPERTY,
) {
    let object = unsafe { &mut *base_from_rt(this) };
    unsafe {
        object.apo_process_impl(
            num_input_connections,
            pp_input_connections,
            num_output_connections,
            pp_output_connections,
        )
    }
}

unsafe extern "system" fn rt_calc_input_frames(this: *mut c_void, output_frame_count: u32) -> u32 {
    let _ = this;
    output_frame_count
}

unsafe extern "system" fn rt_calc_output_frames(this: *mut c_void, input_frame_count: u32) -> u32 {
    let _ = this;
    input_frame_count
}

unsafe extern "system" fn cfg_lock_for_process(
    this: *mut c_void,
    num_input_connections: u32,
    pp_input_connections: *const *const APO_CONNECTION_DESCRIPTOR,
    num_output_connections: u32,
    pp_output_connections: *const *const APO_CONNECTION_DESCRIPTOR,
) -> HRESULT {
    let object = unsafe { &mut *base_from_cfg(this) };
    unsafe {
        object.lock_for_process_impl(
            num_input_connections,
            pp_input_connections,
            num_output_connections,
            pp_output_connections,
        )
    }
}

unsafe extern "system" fn cfg_unlock_for_process(this: *mut c_void) -> HRESULT {
    let object = unsafe { &mut *base_from_cfg(this) };
    if let Some(child_cfg) = unsafe { (*object.child.get()).cfg.as_ref() } {
        if let Err(e) = unsafe { child_cfg.UnlockForProcess() } {
            eapo_common::error_log!("Error in UnlockForProcess");
            return e.code();
        }
    }
    object.base_unlock_for_process()
}

impl EqualizerApo {
    unsafe fn is_output_format_supported_impl(
        &mut self,
        input_format: *mut c_void,
        requested_output_format: *mut c_void,
        pp_supported_output_format: *mut *mut c_void,
    ) -> HRESULT {
        unsafe {
            self.base_is_format_supported(
                input_format,
                requested_output_format,
                pp_supported_output_format,
                false,
            )
        }
    }

    unsafe fn initialize_impl(&mut self, cb_data_size: u32, pby_data: *const u8) -> HRESULT {
        eapo_common::log_helper::reset();
        eapo_common::trace_log!("Initialize");

        if pby_data.is_null() && cb_data_size != 0 {
            return E_INVALIDARG;
        }
        if !pby_data.is_null() && cb_data_size == 0 {
            return E_POINTER;
        }
        // C++: `if (cbDataSize != sizeof(APOInitSystemEffects)) return E_INVALIDARG;`
        if cb_data_size != std::mem::size_of::<ApoInitSystemEffects>() as u32 {
            return E_INVALIDARG;
        }
        let init_struct = unsafe { ptr::read_unaligned(pby_data as *const ApoInitSystemEffects) };
        let apo_guid = init_struct.apo_init.clsid;

        self.reset_child();

        eapo_common::trace_log!("APO GUID: {}", guid_to_string(apo_guid));
        let engine = unsafe { &mut *self.engine.get() };
        engine.set_pre_mix(treats_as_pre_mix(apo_guid));

        let store_ptr = init_struct.p_apo_endpoint_properties;
        let store = if store_ptr.is_null() {
            None
        } else {
            unsafe { IPropertyStore::from_raw_borrowed(&store_ptr) }
        };
        let Some(store) = store else {
            eapo_common::error_log!("Can't read endpoint guid");
            return E_POINTER;
        };
        let variant = match unsafe { store.GetValue(&PKEY_AudioEndpoint_GUID) } {
            Ok(v) => v,
            Err(e) => {
                eapo_common::error_log!("Can't read endpoint guid");
                return e.code();
            }
        };
        let device_guid = unsafe {
            variant
                .Anonymous
                .Anonymous
                .Anonymous
                .pwszVal
                .to_string()
                .unwrap_or_default()
        };
        // `EqualizerAPO::Initialize` deliberately never clears this PROPVARIANT:
        // `pAPOEndpointProperties` is the engine's own store, so `pwszVal` may be
        // engine-owned memory and `PropVariantClear` would free a live buffer.
        // The port stays byte-faithful and leaves the variant uncleared.
        eapo_common::trace_log!("Endpoint GUID: {}", device_guid);

        let mut device_test_pipe_name = String::new();
        if value_exists(APP_REGPATH, "DeviceTestPipeName") {
            match read_value(APP_REGPATH, "DeviceTestPipeName") {
                Ok(value) => device_test_pipe_name = value,
                Err(e) => eapo_common::error_log!("{}", e),
            }
        }

        if !device_test_pipe_name.is_empty() {
            send_message(
                &mut device_test_pipe_name,
                &device_guid,
                apo_guid,
                "Initialize",
            );
        }

        let mut child_apo_guid = String::new();

        let apo_info = DeviceAPOInfo::new();
        match apo_info.load(&device_guid) {
            Ok(true) => {
                let state = apo_info.get_current_install_state();
                engine.set_device_info(
                    apo_info.is_input(),
                    state.install_post_mix,
                    &apo_info.get_device_name(),
                    &apo_info.get_connection_name(),
                    &apo_info.get_device_guid(),
                    &apo_info.get_device_string(),
                );

                if treats_as_pre_mix(apo_guid) {
                    child_apo_guid = apo_info.get_pre_mix_child_guid();
                } else {
                    child_apo_guid = apo_info.get_post_mix_child_guid();
                }

                self.allow_silent_buffer_modification
                    .set(state.allow_silent_buffer_modification);
            }
            Ok(false) => {}
            Err(e) => {
                eapo_common::error_log!("Could not read endpoint device info because of: {}", e);
            }
        }

        #[cfg(feature = "live-probe")]
        if self.clsid == EQUALIZERAPO_PROBE_GUID {
            // Children receive the parent's init block, so `APOInit.clsid` is
            // always the parent's class id; without this the child would re-read
            // the same endpoint child key and chain forever.
            child_apo_guid = String::new();
        }

        eapo_common::trace_log!("Child APO GUID: {}", child_apo_guid);

        if !child_apo_guid.is_empty()
            && child_apo_guid != APOGUID_NULL
            && child_apo_guid != APOGUID_NOKEY
            && child_apo_guid != APOGUID_NOVALUE
        {
            let child_guid = {
                let wide = HSTRING::from(child_apo_guid.clone());
                unsafe { windows::Win32::System::Com::CLSIDFromString(&wide) }
            };
            let Ok(child_guid) = child_guid else {
                eapo_common::error_log!("Can't convert child apo guid string to guid");
                return S_OK;
            };

            let child_apo: IAudioProcessingObject =
                match unsafe { CoCreateInstance(&child_guid, None, CLSCTX_INPROC_SERVER) } {
                    Ok(apo) => apo,
                    Err(_) => {
                        eapo_common::error_log!("Error in CoCreateInstance for child apo");
                        self.reset_child();
                        return S_OK;
                    }
                };

            let child_rt: IAudioProcessingObjectRT = match child_apo.cast() {
                Ok(rt) => rt,
                Err(_) => {
                    eapo_common::error_log!("Error in QueryInterface for child apo RT");
                    self.reset_child();
                    return S_OK;
                }
            };

            let child_cfg: IAudioProcessingObjectConfiguration = match child_apo.cast() {
                Ok(cfg) => cfg,
                Err(_) => {
                    eapo_common::error_log!("Error in QueryInterface for child apo configuration");
                    self.reset_child();
                    return S_OK;
                }
            };

            let data = unsafe { std::slice::from_raw_parts(pby_data, cb_data_size as usize) };
            if unsafe { child_apo.Initialize(data) }.is_err() {
                eapo_common::error_log!("Error in Initialize of child apo");
                self.reset_child();
                return S_OK;
            }

            eapo_common::trace_log!("Successfully created and initialized child APO");

            {
                let child = unsafe { &mut *self.child.get() };
                child.apo = Some(child_apo);
                child.rt = Some(child_rt);
                child.cfg = Some(child_cfg);
            }

            if !device_test_pipe_name.is_empty() {
                send_message(
                    &mut device_test_pipe_name,
                    &device_guid,
                    apo_guid,
                    "ChildAPO",
                );
            }
        }

        S_OK
    }
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ApoInitBaseStruct {
    cb_size: u32,
    clsid: GUID,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct ApoInitSystemEffects {
    apo_init: ApoInitBaseStruct,
    p_apo_endpoint_properties: *mut c_void,
    p_apo_system_effects_properties: *mut c_void,
    p_reserved: *mut c_void,
    p_device_collection: *mut c_void,
}

const _: () = {
    assert!(std::mem::size_of::<ApoInitSystemEffects>() == 56);
    assert!(std::mem::offset_of!(ApoInitSystemEffects, p_apo_endpoint_properties) == 24);
    assert!(std::mem::offset_of!(ApoInitSystemEffects, p_apo_system_effects_properties) == 32);
};

/// Port of `sendMessage` (DeviceTest named-pipe protocol).
fn send_message(
    device_test_pipe_name: &mut String,
    device_guid: &str,
    apo_guid: GUID,
    phase: &str,
) {
    let message = format!(
        "{{\"deviceGuid\":\"{}\", \"stage\":\"{}\", \"phase\":\"{}\"}}",
        device_guid,
        if treats_as_pre_mix(apo_guid) {
            "PreMix"
        } else {
            "PostMix"
        },
        phase
    );

    let pipe_name = HSTRING::from(format!("\\\\.\\pipe\\{}", device_test_pipe_name));
    let mut pipe: Result<HANDLE, windows::core::Error> = unsafe {
        CreateFileW(
            &pipe_name,
            GENERIC_WRITE.0,
            FILE_SHARE_NONE,
            None,
            OPEN_EXISTING,
            FILE_FLAGS_AND_ATTRIBUTES(0),
            None,
        )
    };
    if pipe.is_err() && unsafe { WaitNamedPipeW(&pipe_name, 1000) }.as_bool() {
        pipe = unsafe {
            CreateFileW(
                &pipe_name,
                GENERIC_WRITE.0,
                FILE_SHARE_NONE,
                None,
                OPEN_EXISTING,
                FILE_FLAGS_AND_ATTRIBUTES(0),
                None,
            )
        };
    }

    match pipe {
        Ok(handle) => {
            let mut bytes_written = 0u32;
            if unsafe {
                WriteFile(
                    handle,
                    Some(message.as_bytes()),
                    Some(&mut bytes_written),
                    None,
                )
            }
            .is_err()
            {
                let error = unsafe { GetLastError() };
                eapo_common::error_log!(
                    "Could not write to pipe: {}",
                    eapo_common::string_helper::get_system_error_string(error.0 as i32)
                );
            }
            unsafe {
                let _ = FlushFileBuffers(handle);
                let _ = CloseHandle(handle);
            }
        }
        Err(_) => {
            let error = unsafe { GetLastError() };
            eapo_common::error_log!(
                "Could not connect to named pipe: {}",
                eapo_common::string_helper::get_system_error_string(error.0 as i32)
            );
            *device_test_pipe_name = String::new();
        }
    }
}

fn fill_wide<const N: usize>(target: &mut [u16; N], value: &str) {
    let mut index = 0;
    for unit in value.encode_utf16() {
        if index + 1 >= N {
            break;
        }
        target[index] = unit;
        index += 1;
    }
    target[index] = 0;
}

fn make_reg_properties(clsid: GUID) -> APO_REG_PROPERTIES {
    let mut properties = APO_REG_PROPERTIES::default();
    properties.clsid = clsid;
    properties.Flags = APO_FLAG(
        APO_FLAG_FRAMESPERSECOND_MUST_MATCH.0
            | APO_FLAG_BITSPERSAMPLE_MUST_MATCH.0
            | APO_FLAG_INPLACE.0,
    );
    fill_wide(&mut properties.szFriendlyName, "EqualizerAPO");
    fill_wide(&mut properties.szCopyrightInfo, "Copyright (C) 2015");
    properties.u32MajorVersion = 1;
    properties.u32MinorVersion = 0;
    properties.u32MinInputConnections = 1;
    properties.u32MaxInputConnections = 1;
    properties.u32MinOutputConnections = 1;
    properties.u32MaxOutputConnections = 1;
    properties.u32MaxInstances = u32::MAX;
    properties.u32NumAPOInterfaces = 1;
    properties.iidAPOInterfaceList = [IID_IAUDIO_PROCESSING_OBJECT];
    properties
}

static POST_MIX_PROPERTIES: OnceLock<APO_REG_PROPERTIES> = OnceLock::new();
static PRE_MIX_PROPERTIES: OnceLock<APO_REG_PROPERTIES> = OnceLock::new();
#[cfg(feature = "live-probe")]
static PROBE_PROPERTIES: OnceLock<APO_REG_PROPERTIES> = OnceLock::new();

/// Verification-only class id, compiled in only with `--features live-probe`.
/// The audio engine caches a class object per CLSID inside `audiodg.exe`, so a
/// brand new CLSID is the only way to make a running process load our DLL
/// without restarting the audio service. It is inert unless someone registers it
/// under `HKLM\SOFTWARE\Classes\AudioEngine\AudioProcessingObjects`, which no
/// installer path does. Keep in sync with `CLSID_EqualizerApoProbe` in
/// `src/cxx/apo_shim.cpp`.
#[cfg(feature = "live-probe")]
pub(crate) const EQUALIZERAPO_PROBE_GUID: GUID =
    GUID::from_u128(0xd3a9e4b1_6c25_4f7a_9e83_1b5c7d2a4e60);

pub(crate) fn post_mix_properties() -> &'static APO_REG_PROPERTIES {
    POST_MIX_PROPERTIES.get_or_init(|| make_reg_properties(eapo_common::EQUALIZERAPO_POST_MIX_GUID))
}

pub(crate) fn pre_mix_properties() -> &'static APO_REG_PROPERTIES {
    PRE_MIX_PROPERTIES.get_or_init(|| make_reg_properties(eapo_common::EQUALIZERAPO_PRE_MIX_GUID))
}

#[cfg(feature = "live-probe")]
pub(crate) fn probe_properties() -> &'static APO_REG_PROPERTIES {
    PROBE_PROPERTIES.get_or_init(|| make_reg_properties(EQUALIZERAPO_PROBE_GUID))
}

/// The probe class id is only ever registered in the pre-mix role, so it takes
/// the same code path as `EQUALIZERAPO_PRE_MIX_GUID`.
#[cfg(feature = "live-probe")]
fn treats_as_pre_mix(guid: GUID) -> bool {
    guid == eapo_common::EQUALIZERAPO_PRE_MIX_GUID || guid == EQUALIZERAPO_PROBE_GUID
}

#[cfg(not(feature = "live-probe"))]
fn treats_as_pre_mix(guid: GUID) -> bool {
    guid == eapo_common::EQUALIZERAPO_PRE_MIX_GUID
}

#[cfg(feature = "live-probe")]
pub(crate) fn properties_for_clsid(clsid: &GUID) -> &'static APO_REG_PROPERTIES {
    if *clsid == eapo_common::EQUALIZERAPO_PRE_MIX_GUID {
        pre_mix_properties()
    } else if *clsid == EQUALIZERAPO_PROBE_GUID {
        probe_properties()
    } else {
        post_mix_properties()
    }
}

#[cfg(not(feature = "live-probe"))]
pub(crate) fn properties_for_clsid(clsid: &GUID) -> &'static APO_REG_PROPERTIES {
    if *clsid == eapo_common::EQUALIZERAPO_PRE_MIX_GUID {
        pre_mix_properties()
    } else {
        post_mix_properties()
    }
}

fn trace_format(label: &str, format: &UNCOMPRESSEDAUDIOFORMAT) {
    eapo_common::trace_log!(
        "{} = {{ {:08X}, {}, {}, {}, {:.6}, {:08X} }}",
        label,
        format.guidFormatType.data1,
        format.dwSamplesPerFrame,
        format.dwBytesPerSampleContainer,
        format.dwValidBitsPerSample,
        format.fFramesPerSecond,
        format.dwChannelMask
    );
}

fn trace_format_with_frames(label: &str, format: &UNCOMPRESSEDAUDIOFORMAT, frames: u32) {
    eapo_common::trace_log!(
        "{} = {{ {:08X}, {}, {}, {}, {:.6}, {:08X}, {} }}",
        label,
        format.guidFormatType.data1,
        format.dwSamplesPerFrame,
        format.dwBytesPerSampleContainer,
        format.dwValidBitsPerSample,
        format.fFramesPerSecond,
        format.dwChannelMask,
        frames
    );
}

#[cfg(test)]
mod format_tests {
    use super::*;
    use windows::Win32::Media::Audio::Apo::IAudioMediaType;

    #[test]
    fn registration_properties_follow_the_requested_clsid() {
        #[allow(unused_mut)] // the probe class id is appended only with `live-probe`
        let mut clsids = vec![
            eapo_common::EQUALIZERAPO_PRE_MIX_GUID,
            eapo_common::EQUALIZERAPO_POST_MIX_GUID,
        ];
        #[cfg(feature = "live-probe")]
        clsids.push(EQUALIZERAPO_PROBE_GUID);
        unsafe {
            for clsid in clsids {
                let apo = EqualizerApo::create_instance(std::ptr::null_mut(), clsid);
                assert!(!apo.is_null());
                let mut properties: *mut APO_REG_PROPERTIES = std::ptr::null_mut();
                let hr = apo_get_registration_properties(apo as *mut c_void, &mut properties);
                assert!(hr.is_ok());
                assert!(!properties.is_null());
                assert_eq!((*properties).clsid, clsid);
                windows::Win32::System::Com::CoTaskMemFree(Some(
                    properties as *const c_void,
                ));
                EqualizerApo::non_delegating_release(apo);
            }
        }
    }

    #[test]
    fn supported_format_is_returned_for_matching_formats() {
        const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID =
            GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
        let mut format = UNCOMPRESSEDAUDIOFORMAT::default();
        format.guidFormatType = KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
        format.dwSamplesPerFrame = 2;
        format.dwBytesPerSampleContainer = 4;
        format.dwValidBitsPerSample = 32;
        format.fFramesPerSecond = 48000.0;
        format.dwChannelMask = 3;

        unsafe {
            let requested = crate::media_type::create_audio_media_type_from_uncompressed_audio_format(
                &format,
            );
            let opposite = crate::media_type::create_audio_media_type_from_uncompressed_audio_format(
                &format,
            );
            assert!(!requested.is_null());
            assert!(!opposite.is_null());
            let mut supported: *mut c_void = std::ptr::null_mut();
            let hr = EqualizerApo::rust_base_is_format_supported(opposite, requested, &mut supported);
            assert_eq!(hr, S_OK);
            assert_eq!(
                supported, requested,
                "S_OK must return the engine's requested format object"
            );
            drop(IAudioMediaType::from_raw(supported));
            drop(IAudioMediaType::from_raw(requested));
            drop(IAudioMediaType::from_raw(opposite));
        }
    }
}
