//! Pure-Rust EqualizerAPO COM audio processing object.
//!
//! Drop-in replacement for `EqualizerAPO.dll`: exports `DllGetClassObject`,
//! `DllCanUnloadNow`, `DllRegisterServer` and `DllUnregisterServer` under the
//! same names as the original `.def` file. The APO GUIDs and registration
//! properties are those of the C++ build.
//!
//! The original `DllMain` only cached the module handle. The Rust port has no
//! custom entry point (keeping the export table identical to the original
//! `.def`) and resolves the module handle lazily from a code address.

#![allow(non_snake_case)]

mod apo;
mod class_factory;
mod media_type;
mod registration;

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::Ordering;
use std::sync::OnceLock;

use windows::core::{GUID, HRESULT, PCWSTR};
use windows::Win32::Foundation::{E_POINTER, HMODULE};
use windows::Win32::System::LibraryLoader::{
    GetModuleHandleExW, GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS,
    GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT,
};

use apo::INST_COUNT;

const S_OK: HRESULT = HRESULT(0);
const S_FALSE: HRESULT = HRESULT(1);

static MODULE_HANDLE: OnceLock<usize> = OnceLock::new();

pub(crate) fn module_handle() -> Option<HMODULE> {
    let handle = *MODULE_HANDLE.get_or_init(|| unsafe {
        let mut module = HMODULE::default();
        let flags = GET_MODULE_HANDLE_EX_FLAG_FROM_ADDRESS
            | GET_MODULE_HANDLE_EX_FLAG_UNCHANGED_REFCOUNT;
        let address = DllGetClassObject as *const () as *const u16;
        if GetModuleHandleExW(flags, PCWSTR(address), &mut module).is_ok() {
            module.0 as usize
        } else {
            0
        }
    });
    if handle == 0 {
        None
    } else {
        Some(HMODULE(handle as *mut c_void))
    }
}

extern "C" {
    fn eapo_cxx_get_class_object(
        clsid: *const GUID,
        iid: *const GUID,
        ppv: *mut *mut c_void,
    ) -> HRESULT;
    fn eapo_cxx_can_unload_now() -> HRESULT;
}

/// `DllCanUnloadNow` (STDAPI, exported by name on x64).
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DllCanUnloadNow() -> HRESULT {
    let cxx = unsafe { eapo_cxx_can_unload_now() };
    let instances = INST_COUNT.load(Ordering::SeqCst);
    let factories = registration::CLASS_FACTORY_LOCK_COUNT.load(Ordering::SeqCst);
    let result = if cxx == S_OK && instances == 0 && factories == 0 {
        S_OK
    } else {
        S_FALSE
    };
    result
}

/// `DllGetClassObject` (STDAPI, exported by name on x64).
#[no_mangle]
#[allow(non_snake_case)]
pub unsafe extern "system" fn DllGetClassObject(
    clsid: *const GUID,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if clsid.is_null() || iid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    let clsid = unsafe { *clsid };
    let hr = unsafe { eapo_cxx_get_class_object(clsid_ptr(&clsid), iid, ppv) };
    hr
}

fn clsid_ptr(clsid: &GUID) -> *const GUID {
    clsid as *const GUID
}

/// Creates the pure-Rust COM implementation used by the C++ CBase shim.
#[no_mangle]
pub unsafe extern "system" fn eapo_rust_create_inner(
    clsid: *const GUID,
    cxx_base: *mut c_void,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if clsid.is_null() || ppv.is_null() {
        return E_POINTER;
    }
    let factory = unsafe { class_factory::create_factory(*clsid) };
    if factory.is_null() {
        return windows::Win32::Foundation::E_OUTOFMEMORY;
    }
    let hr = unsafe {
        class_factory::create_instance(
            factory,
            ptr::null_mut(),
            &apo::IID_IAUDIO_PROCESSING_OBJECT,
            ppv,
        )
    };
    unsafe {
        class_factory::release_impl(factory);
    }
    if hr.is_ok() {
        apo::EqualizerApo::attach_cxx_base(*ppv, cxx_base);
    }
    hr
}

/// `DllRegisterServer` (STDAPI).
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DllRegisterServer() -> HRESULT {
    unsafe { registration::dll_register_server() }
}

/// `DllUnregisterServer` (STDAPI).
#[no_mangle]
#[allow(non_snake_case)]
pub extern "system" fn DllUnregisterServer() -> HRESULT {
    unsafe { registration::dll_unregister_server() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[repr(C)]
    struct QiRefVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
    }

    #[repr(C)]
    struct FactoryVtbl {
        query_interface:
            unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
        add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
        release: unsafe extern "system" fn(*mut c_void) -> u32,
        create_instance: unsafe extern "system" fn(
            *mut c_void,
            *mut c_void,
            *const GUID,
            *mut *mut c_void,
        ) -> HRESULT,
        lock_server: unsafe extern "system" fn(*mut c_void, i32) -> HRESULT,
    }

    const IID_ICLASSFACTORY: GUID = GUID::from_u128(0x00000001_0000_0000_c000_000000000046);

    #[test]
    fn class_factory_creates_all_com_interfaces() {
        unsafe {
            let mut factory: *mut c_void = std::ptr::null_mut();
            let hr = DllGetClassObject(
                &eapo_common::EQUALIZERAPO_POST_MIX_GUID,
                &IID_ICLASSFACTORY,
                &mut factory,
            );
            assert!(hr.is_ok());
            assert!(!factory.is_null());
            let factory_vtbl = *(factory as *const *const FactoryVtbl);

            let mut apo: *mut c_void = std::ptr::null_mut();
            let hr = ((*factory_vtbl).create_instance)(
                factory,
                std::ptr::null_mut(),
                &apo::IID_IAUDIO_PROCESSING_OBJECT,
                &mut apo,
            );
            assert!(hr.is_ok());
            assert!(!apo.is_null());
            let apo_vtbl = *(apo as *const *const QiRefVtbl);

            let mut rt: *mut c_void = std::ptr::null_mut();
            let hr = ((*apo_vtbl).query_interface)(
                apo,
                &apo::IID_IAUDIO_PROCESSING_OBJECT_RT,
                &mut rt,
            );
            assert!(hr.is_ok());
            assert!(!rt.is_null());
            let rt_vtbl = *(rt as *const *const QiRefVtbl);

            let mut cfg: *mut c_void = std::ptr::null_mut();
            let hr = ((*apo_vtbl).query_interface)(
                apo,
                &apo::IID_IAUDIO_PROCESSING_OBJECT_CONFIGURATION,
                &mut cfg,
            );
            assert!(hr.is_ok());
            assert!(!cfg.is_null());
            let cfg_vtbl = *(cfg as *const *const QiRefVtbl);

            let mut se: *mut c_void = std::ptr::null_mut();
            let hr = ((*apo_vtbl).query_interface)(
                apo,
                &apo::IID_IAUDIO_SYSTEM_EFFECTS,
                &mut se,
            );
            assert!(hr.is_ok());
            assert!(!se.is_null());
            let se_vtbl = *(se as *const *const QiRefVtbl);

            let mut unknown: *mut c_void = std::ptr::null_mut();
            let hr = ((*apo_vtbl).query_interface)(apo, &apo::IID_IUNKNOWN, &mut unknown);
            assert!(hr.is_ok());
            assert!(!unknown.is_null());
            let unknown_vtbl = *(unknown as *const *const QiRefVtbl);

            assert_eq!(((*rt_vtbl).release)(rt), 4);
            assert_eq!(((*cfg_vtbl).release)(cfg), 3);
            assert_eq!(((*se_vtbl).release)(se), 2);
            assert_eq!(((*unknown_vtbl).release)(unknown), 1);
            assert_eq!(((*apo_vtbl).release)(apo), 0);
            assert_eq!(INST_COUNT.load(Ordering::SeqCst), 0);
            assert_eq!(((*factory_vtbl).release)(factory), 0);
        }
    }

    /// The audio engine aggregates APOs: `CSystemEffectWrapper` passes itself as
    /// `pUnkOuter`, and `EqualizerAPO::AddRef`/`Release` are
    /// `pUnkOuter->AddRef()`/`Release()` rather than a private counter. Every
    /// reference taken through the primary interface must therefore land on the
    /// outer object; otherwise the engine frees the wrapper while its APO is
    /// still alive and later faults while adding a reference to freed memory.
    #[test]
    fn references_on_the_primary_interface_are_forwarded_to_the_outer() {
        use std::sync::atomic::AtomicU32;
        use windows::Win32::Foundation::E_NOINTERFACE;

        #[repr(C)]
        struct Outer {
            vtbl: *const QiRefVtbl,
            add_refs: AtomicU32,
            releases: AtomicU32,
        }

        static OUTER_ADDS: AtomicU32 = AtomicU32::new(0);
        static OUTER_RELEASES: AtomicU32 = AtomicU32::new(0);

        unsafe extern "system" fn outer_query_interface(
            this: *mut c_void,
            iid: *const GUID,
            ppv: *mut *mut c_void,
        ) -> HRESULT {
            unsafe {
                if *iid == apo::IID_IUNKNOWN {
                    let vtbl = (*(this as *const Outer)).vtbl;
                    ((*vtbl).add_ref)(this);
                    *ppv = this;
                    S_OK
                } else {
                    *ppv = std::ptr::null_mut();
                    E_NOINTERFACE
                }
            }
        }

        unsafe extern "system" fn outer_add_ref(_this: *mut c_void) -> u32 {
            OUTER_ADDS.fetch_add(1, Ordering::SeqCst) + 1
        }

        unsafe extern "system" fn outer_release(_this: *mut c_void) -> u32 {
            OUTER_RELEASES.fetch_add(1, Ordering::SeqCst) + 1
        }

        static OUTER_VTBL: QiRefVtbl = QiRefVtbl {
            query_interface: outer_query_interface,
            add_ref: outer_add_ref,
            release: outer_release,
        };

        unsafe {
            let mut outer = Box::new(Outer {
                vtbl: &OUTER_VTBL,
                add_refs: AtomicU32::new(0),
                releases: AtomicU32::new(0),
            });
            let outer_ptr = &mut *outer as *mut Outer as *mut c_void;

            let mut factory: *mut c_void = std::ptr::null_mut();
            assert!(DllGetClassObject(
                &eapo_common::EQUALIZERAPO_POST_MIX_GUID,
                &IID_ICLASSFACTORY,
                &mut factory
            )
            .is_ok());
            let factory_vtbl = *(factory as *const *const FactoryVtbl);

            // Standard aggregation asks the factory for IUnknown.
            let mut unknown: *mut c_void = std::ptr::null_mut();
            let hr = ((*factory_vtbl).create_instance)(
                factory,
                outer_ptr,
                &apo::IID_IUNKNOWN,
                &mut unknown,
            );
            assert!(hr.is_ok(), "aggregated CreateInstance -> {hr:?}");
            assert!(!unknown.is_null());
            assert_eq!(
                OUTER_ADDS.load(Ordering::SeqCst),
                0,
                "creating the inner must not add a reference to the outer yet"
            );

            let unknown_vtbl = *(unknown as *const *const QiRefVtbl);

            // References taken through the primary interface belong to the outer.
            let mut apo_ptr: *mut c_void = std::ptr::null_mut();
            let hr = ((*unknown_vtbl).query_interface)(unknown, &apo::IID_IAUDIO_PROCESSING_OBJECT, &mut apo_ptr);
            assert!(hr.is_ok());
            assert_eq!(
                OUTER_ADDS.load(Ordering::SeqCst),
                1,
                "QueryInterface on the aggregated object must reach the outer"
            );
            let apo_vtbl = *(apo_ptr as *const *const QiRefVtbl);
            ((*apo_vtbl).add_ref)(apo_ptr);
            assert_eq!(
                OUTER_ADDS.load(Ordering::SeqCst),
                2,
                "AddRef through the primary interface must reach the outer"
            );
            ((*apo_vtbl).release)(apo_ptr);
            assert_eq!(OUTER_RELEASES.load(Ordering::SeqCst), 1);

            // Releasing the inner through IUnknown goes to the delegating counter.
            ((*unknown_vtbl).release)(unknown);
            ((*factory_vtbl).release)(factory);
            assert_eq!(INST_COUNT.load(Ordering::SeqCst), 0);
            drop(&mut outer);
        }
    }

    #[test]
    fn registration_properties_match_the_cpp_build() {
        let properties = apo::post_mix_properties();
        assert_eq!(
            properties.clsid,
            eapo_common::EQUALIZERAPO_POST_MIX_GUID
        );
        assert_eq!(properties.u32MajorVersion, 1);
        assert_eq!(properties.u32MinorVersion, 0);
        assert_eq!(properties.u32MinInputConnections, 1);
        assert_eq!(properties.u32MaxInputConnections, 1);
        assert_eq!(properties.u32MinOutputConnections, 1);
        assert_eq!(properties.u32MaxOutputConnections, 1);
        assert_eq!(properties.u32NumAPOInterfaces, 1);
        assert_eq!(properties.u32MaxInstances, u32::MAX);
        let flags = properties.Flags.0;
        assert!(flags & windows::Win32::Media::Audio::Apo::APO_FLAG_FRAMESPERSECOND_MUST_MATCH.0 != 0);
        assert!(flags & windows::Win32::Media::Audio::Apo::APO_FLAG_BITSPERSAMPLE_MUST_MATCH.0 != 0);
        assert!(flags & windows::Win32::Media::Audio::Apo::APO_FLAG_INPLACE.0 != 0);
        let name_len = properties
            .szFriendlyName
            .iter()
            .position(|&c| c == 0)
            .unwrap_or(properties.szFriendlyName.len());
        assert_eq!(
            String::from_utf16_lossy(&properties.szFriendlyName[..name_len]),
            "EqualizerAPO"
        );

        // The lazy module handle replaces the original DllMain cache.
        assert!(module_handle().is_some());
    }

    /// End-to-end COM test: LockForProcess and APOProcess with the machine's
    /// configured EqualizerAPO config. Opt-in only: the WDK base class rejects
    /// hand-built connection descriptors (`E_INVALIDARG`), and the reference
    /// C++ DLL returns the same code for them, so a real `LockForProcess` needs
    /// a live audio graph rather than a synthetic one.
    #[test]
    #[ignore]
    #[allow(unused_assignments)]
    fn lock_and_process_audio_through_com() {
        use std::mem::ManuallyDrop;
        use windows::core::Interface;
        use windows::Win32::Media::Audio::Apo::{
            IAudioMediaType, IAudioProcessingObjectConfiguration, IAudioProcessingObjectRT,
            APO_CONNECTION_BUFFER_TYPE_DEPENDANT, APO_CONNECTION_DESCRIPTOR,
            APO_CONNECTION_PROPERTY, BUFFER_SILENT, BUFFER_VALID, UNCOMPRESSEDAUDIOFORMAT,
        };
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};

        const FRAMES: usize = 1024;
        const CHANNELS: usize = 2;
        const KSDATAFORMAT_SUBTYPE_IEEE_FLOAT: GUID =
            GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            let mut factory: *mut c_void = std::ptr::null_mut();
            let hr = DllGetClassObject(
                &eapo_common::EQUALIZERAPO_POST_MIX_GUID,
                &IID_ICLASSFACTORY,
                &mut factory,
            );
            assert!(hr.is_ok());
            let factory_vtbl = *(factory as *const *const FactoryVtbl);
            let mut apo: *mut c_void = std::ptr::null_mut();
            let hr = ((*factory_vtbl).create_instance)(
                factory,
                std::ptr::null_mut(),
                &apo::IID_IAUDIO_PROCESSING_OBJECT,
                &mut apo,
            );
            assert!(hr.is_ok());
            let apo_vtbl = *(apo as *const *const QiRefVtbl);

            let mut cfg_ptr: *mut c_void = std::ptr::null_mut();
            assert!(((*apo_vtbl).query_interface)(
                apo,
                &apo::IID_IAUDIO_PROCESSING_OBJECT_CONFIGURATION,
                &mut cfg_ptr,
            )
            .is_ok());
            let cfg = IAudioProcessingObjectConfiguration::from_raw(cfg_ptr);

            let mut rt_ptr: *mut c_void = std::ptr::null_mut();
            assert!(((*apo_vtbl).query_interface)(
                apo,
                &apo::IID_IAUDIO_PROCESSING_OBJECT_RT,
                &mut rt_ptr,
            )
            .is_ok());
            let rt = IAudioProcessingObjectRT::from_raw(rt_ptr);

            let mut format = UNCOMPRESSEDAUDIOFORMAT::default();
            format.guidFormatType = KSDATAFORMAT_SUBTYPE_IEEE_FLOAT;
            format.dwSamplesPerFrame = CHANNELS as u32;
            format.dwBytesPerSampleContainer = 4;
            format.dwValidBitsPerSample = 32;
            format.fFramesPerSecond = 48000.0;
            format.dwChannelMask = 3;

            let in_media = IAudioMediaType::from_raw(
                crate::media_type::create_audio_media_type_from_uncompressed_audio_format(&format),
            );
            let out_media = IAudioMediaType::from_raw(
                crate::media_type::create_audio_media_type_from_uncompressed_audio_format(&format),
            );
            let mut in_descriptor = APO_CONNECTION_DESCRIPTOR {
                Type: APO_CONNECTION_BUFFER_TYPE_DEPENDANT,
                pBuffer: 0,
                u32MaxFrameCount: FRAMES as u32,
                pFormat: ManuallyDrop::new(Some(in_media)),
                u32Signature: 0,
            };
            let mut out_descriptor = APO_CONNECTION_DESCRIPTOR {
                Type: APO_CONNECTION_BUFFER_TYPE_DEPENDANT,
                pBuffer: 0,
                u32MaxFrameCount: FRAMES as u32,
                pFormat: ManuallyDrop::new(Some(out_media)),
                u32Signature: 0,
            };
            let inputs = [&in_descriptor as *const APO_CONNECTION_DESCRIPTOR];
            let outputs = [&out_descriptor as *const APO_CONNECTION_DESCRIPTOR];
            cfg.LockForProcess(&inputs, &outputs)
                .expect("LockForProcess");
            ManuallyDrop::drop(&mut in_descriptor.pFormat);
            ManuallyDrop::drop(&mut out_descriptor.pFormat);

            let mut input = vec![0.0f32; FRAMES * CHANNELS];
            for frame in 0..FRAMES {
                let sample = ((frame as f32) * 440.0 * 2.0 * std::f32::consts::PI / 48000.0).sin() * 0.25;
                input[frame * CHANNELS] = sample;
                input[frame * CHANNELS + 1] = sample;
            }
            let mut output = vec![0.0f32; FRAMES * CHANNELS];
            let mut input_property = APO_CONNECTION_PROPERTY {
                pBuffer: input.as_ptr() as usize,
                u32ValidFrameCount: FRAMES as u32,
                u32BufferFlags: BUFFER_VALID,
                u32Signature: 0,
            };
            let mut output_property = APO_CONNECTION_PROPERTY {
                pBuffer: output.as_mut_ptr() as usize,
                u32ValidFrameCount: 0,
                u32BufferFlags: BUFFER_VALID,
                u32Signature: 0,
            };
            let input_ptrs = [&input_property as *const APO_CONNECTION_PROPERTY];
            let mut output_ptrs = [&mut output_property as *mut APO_CONNECTION_PROPERTY];
            rt.APOProcess(1, input_ptrs.as_ptr(), 1, output_ptrs.as_mut_ptr());
            assert_eq!(output_property.u32ValidFrameCount, FRAMES as u32);
            assert_eq!(output_property.u32BufferFlags, BUFFER_VALID);
            assert!(output.iter().all(|sample| sample.is_finite()));

            // Silent input must produce silent output and the SILENT flag.
            input.fill(0.0);
            output.fill(123.0);
            input_property.u32BufferFlags = BUFFER_SILENT;
            output_property.u32ValidFrameCount = 0;
            rt.APOProcess(1, input_ptrs.as_ptr(), 1, output_ptrs.as_mut_ptr());
            assert_eq!(output_property.u32BufferFlags, BUFFER_SILENT);
            assert!(output.iter().all(|sample| *sample == 0.0));

            cfg.UnlockForProcess().expect("UnlockForProcess");
            drop(rt);
            drop(cfg);
            ((*apo_vtbl).release)(apo);
            ((*factory_vtbl).release)(factory);
            // No CoUninitialize: the references above are deliberately leaked.
        }
    }

    /// The wrapper must forward lock bookkeeping to the WDK base class:
    /// unlocking a never-locked object only reports `APOERR_ALREADY_UNLOCKED`
    /// if the real `CBaseAudioProcessingObject::UnlockForProcess` answered,
    /// because the standalone Rust fallback returns `S_OK`.
    #[test]
    fn wrapper_reaches_the_wdk_base_class() {
        use windows::Win32::Media::Audio::Apo::{
            APO_CONNECTION_DESCRIPTOR, APOERR_ALREADY_UNLOCKED,
        };

        #[repr(C)]
        struct CfgVtbl {
            query_interface:
                unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
            add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
            release: unsafe extern "system" fn(*mut c_void) -> u32,
            lock_for_process: unsafe extern "system" fn(
                *mut c_void,
                u32,
                *const *const APO_CONNECTION_DESCRIPTOR,
                u32,
                *const *const APO_CONNECTION_DESCRIPTOR,
            ) -> HRESULT,
            unlock_for_process: unsafe extern "system" fn(*mut c_void) -> HRESULT,
        }

        unsafe {
            let mut factory: *mut c_void = std::ptr::null_mut();
            assert!(DllGetClassObject(
                &eapo_common::EQUALIZERAPO_POST_MIX_GUID,
                &IID_ICLASSFACTORY,
                &mut factory,
            )
            .is_ok());
            let factory_vtbl = *(factory as *const *const FactoryVtbl);
            let mut apo: *mut c_void = std::ptr::null_mut();
            assert!(((*factory_vtbl).create_instance)(
                factory,
                std::ptr::null_mut(),
                &apo::IID_IAUDIO_PROCESSING_OBJECT,
                &mut apo,
            )
            .is_ok());
            let apo_vtbl = *(apo as *const *const QiRefVtbl);
            let mut cfg_ptr: *mut c_void = std::ptr::null_mut();
            assert!(((*apo_vtbl).query_interface)(
                apo,
                &apo::IID_IAUDIO_PROCESSING_OBJECT_CONFIGURATION,
                &mut cfg_ptr,
            )
            .is_ok());
            let cfg_vtbl = *(cfg_ptr as *const *const CfgVtbl);
            assert_eq!(
                ((*cfg_vtbl).unlock_for_process)(cfg_ptr),
                APOERR_ALREADY_UNLOCKED
            );
            ((*cfg_vtbl).release)(cfg_ptr);
            ((*apo_vtbl).release)(apo);
            ((*factory_vtbl).release)(factory);
        }
    }

    /// `EqualizerAPO::EqualizerAPO` builds the WDK base class with
    /// `regPostMixProperties` regardless of which class id it was created for,
    /// so *every* instance reports the post-mix CLSID from
    /// `GetRegistrationProperties` — including the verification probe, which has
    /// no reference counterpart. The audio engine reads exactly this blob, so the
    /// port must reproduce the quirk instead of "fixing" it.
    #[test]
    fn instances_report_the_post_mix_properties_like_the_reference() {
        use windows::core::Interface;
        use windows::Win32::Media::Audio::Apo::IAudioProcessingObject;
        use windows::Win32::System::Com::CoTaskMemFree;

        #[allow(unused_mut)] // the probe class id is appended only with `live-probe`
        let mut clsids = vec![
            eapo_common::EQUALIZERAPO_PRE_MIX_GUID,
            eapo_common::EQUALIZERAPO_POST_MIX_GUID,
        ];
        #[cfg(feature = "live-probe")]
        clsids.push(apo::EQUALIZERAPO_PROBE_GUID);
        for probe in clsids {
            unsafe {
                let mut factory: *mut c_void = std::ptr::null_mut();
                assert!(
                    DllGetClassObject(&probe, &IID_ICLASSFACTORY, &mut factory).is_ok(),
                    "the class id must be creatable"
                );
                let factory_vtbl = *(factory as *const *const FactoryVtbl);
                let mut apo: *mut c_void = std::ptr::null_mut();
                assert!(((*factory_vtbl).create_instance)(
                    factory,
                    std::ptr::null_mut(),
                    &apo::IID_IAUDIO_PROCESSING_OBJECT,
                    &mut apo,
                )
                .is_ok());
                let object = IAudioProcessingObject::from_raw(apo);
                let properties = object
                    .GetRegistrationProperties()
                    .expect("GetRegistrationProperties");
                assert!(!properties.is_null());
                assert_eq!(
                    (*properties).clsid,
                    eapo_common::EQUALIZERAPO_POST_MIX_GUID,
                    "the base class must be built with the post-mix blob for class id {probe:?}"
                );
                CoTaskMemFree(Some(properties as *const c_void));
                // Dropping `object` releases the wrapper, which releases the
                // Rust inner object; no instance may outlive this iteration.
                drop(object);
                ((*factory_vtbl).release)(factory);
            }
        }
        assert_eq!(
            unsafe { eapo_cxx_can_unload_now() },
            S_OK,
            "the wrapper must not leak instances"
        );
    }

    #[test]
    fn rejects_unknown_class_id() {
        use windows::Win32::Foundation::CLASS_E_CLASSNOTAVAILABLE;
        unsafe {
            let unknown_clsid = GUID::from_u128(0x11111111_2222_3333_4444_555555555555);
            let mut factory: *mut c_void = std::ptr::null_mut();
            let hr = DllGetClassObject(&unknown_clsid, &IID_ICLASSFACTORY, &mut factory);
            assert_eq!(hr, CLASS_E_CLASSNOTAVAILABLE);
            assert!(factory.is_null());
        }
    }

    /// Without the `live-probe` feature the shipped binary must not answer for
    /// the verification class id at all, in the Rust object and in the C++ shim.
    #[cfg(not(feature = "live-probe"))]
    #[test]
    fn rejects_the_verification_class_id_unless_the_feature_is_on() {
        use windows::Win32::Foundation::CLASS_E_CLASSNOTAVAILABLE;
        let probe = GUID::from_u128(0xd3a9e4b1_6c25_4f7a_9e83_1b5c7d2a4e60);
        unsafe {
            let mut factory: *mut c_void = std::ptr::null_mut();
            assert_eq!(
                DllGetClassObject(&probe, &IID_ICLASSFACTORY, &mut factory),
                CLASS_E_CLASSNOTAVAILABLE
            );
            assert!(factory.is_null());
        }
        assert_eq!(
            apo::properties_for_clsid(&probe).clsid,
            eapo_common::EQUALIZERAPO_POST_MIX_GUID,
            "an unknown class id must fall back to the post-mix properties"
        );
    }

    /// Differential oracle. Probes our COM object and the reference C++
    /// `EqualizerAPO.dll` with one identical call sequence and asserts the two
    /// answer identically — the interface surface, the registration properties,
    /// the unlocked-state results and every input-format negotiation outcome.
    ///
    /// Opt-in because it needs the reference DLL installed
    /// (`CPP_APO_PATH`, default `D:\zmq13\EqualizerAPO.dll`). It touches no
    /// audio device, no endpoint binding and no audio service.
    #[test]
    #[ignore]
    fn com_surface_matches_the_reference_cpp_binary() {
        use windows::core::Interface;
        use windows::Win32::Media::Audio::Apo::{
            IAudioMediaType, APO_REG_PROPERTIES, UNCOMPRESSEDAUDIOFORMAT,
        };
        use windows::Win32::System::LibraryLoader::{
            GetProcAddress, LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH,
        };

        /// `IAudioProcessingObject` slot order straight from
        /// `audioenginebaseapo.h`: Reset, GetLatency, GetRegistrationProperties,
        /// Initialize, IsInputFormatSupported, IsOutputFormatSupported,
        /// GetInputChannelCount.
        #[repr(C)]
        struct ProbeVtbl {
            query_interface:
                unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT,
            add_ref: unsafe extern "system" fn(*mut c_void) -> u32,
            release: unsafe extern "system" fn(*mut c_void) -> u32,
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

        type GetClassObject =
            unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;

        const FLOAT32: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);
        const PCM16: GUID = GUID::from_u128(0x00000001_0000_0010_8000_00aa00389b71);

        fn format(subtype: GUID, channels: u32, rate: f32, container: u32, valid: u32) -> UNCOMPRESSEDAUDIOFORMAT {
            UNCOMPRESSEDAUDIOFORMAT {
                guidFormatType: subtype,
                dwSamplesPerFrame: channels,
                dwBytesPerSampleContainer: container,
                dwValidBitsPerSample: valid,
                fFramesPerSecond: rate,
                dwChannelMask: if channels == 6 { 0x3F } else { 3 },
                ..Default::default()
            }
        }

        fn wide_to_string(value: &[u16]) -> String {
            let end = value.iter().position(|&c| c == 0).unwrap_or(value.len());
            String::from_utf16_lossy(&value[..end])
        }

        /// A GUID in full. Comparing only `data1` would let an interface id that
        /// differs in any other field pass unnoticed.
        fn format_guid(value: &GUID) -> String {
            format!(
                "{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}",
                value.data1,
                value.data2,
                value.data3,
                value.data4[0],
                value.data4[1],
                value.data4[2],
                value.data4[3],
                value.data4[4],
                value.data4[5],
                value.data4[6],
                value.data4[7]
            )
        }

        fn describe_format(format: &UNCOMPRESSEDAUDIOFORMAT) -> String {
            format!(
                "type={:08X} channels={} container={} valid={} rate={} mask={:08X}",
                format.guidFormatType.data1,
                format.dwSamplesPerFrame,
                format.dwBytesPerSampleContainer,
                format.dwValidBitsPerSample,
                format.fFramesPerSecond,
                format.dwChannelMask
            )
        }

        /// Produce the canonical "answer sheet" for one object pointer.
        unsafe fn probe(object: *mut c_void) -> Vec<String> {
            let vtbl = *(object as *const *const ProbeVtbl);
            let mut sheet = Vec::new();

            let iids: [(&str, GUID); 14] = [
                ("IUnknown", apo::IID_IUNKNOWN),
                ("IAudioProcessingObject", apo::IID_IAUDIO_PROCESSING_OBJECT),
                ("IAudioProcessingObjectRT", apo::IID_IAUDIO_PROCESSING_OBJECT_RT),
                (
                    "IAudioProcessingObjectConfiguration",
                    apo::IID_IAUDIO_PROCESSING_OBJECT_CONFIGURATION,
                ),
                ("IAudioSystemEffects", apo::IID_IAUDIO_SYSTEM_EFFECTS),
                ("IAudioSystemEffects2", apo::IID_IAUDIO_SYSTEM_EFFECTS2),
                ("IAudioSystemEffects3", apo::IID_IAUDIO_SYSTEM_EFFECTS3),
                (
                    "IAudioProcessingObjectNotifications",
                    apo::IID_IAUDIO_PROCESSING_OBJECT_NOTIFICATIONS,
                ),
                (
                    "IAudioProcessingObjectNotifications2",
                    apo::IID_IAUDIO_PROCESSING_OBJECT_NOTIFICATIONS2,
                ),
                ("IAudioProcessingObjectVBR", apo::IID_IAUDIO_PROCESSING_OBJECT_VBR),
                (
                    "IAudioSystemEffectsCustomFormats",
                    apo::IID_IAUDIO_SYSTEM_EFFECTS_CUSTOM_FORMATS,
                ),
                (
                    "IAudioDeviceModulesClient",
                    apo::IID_IAUDIO_DEVICE_MODULES_CLIENT,
                ),
                (
                    "IAudioProcessingObjectRTQueueService",
                    apo::IID_IAUDIO_PROCESSING_OBJECT_RT_QUEUE_SERVICE,
                ),
                ("IID_PPV_ARGS-sytle-unknown", GUID::from_u128(0x11111111_2222_3333_4444_555555555555)),
            ];
            for (name, iid) in iids {
                let mut ptr: *mut c_void = std::ptr::null_mut();
                let hr = ((*vtbl).query_interface)(object, &iid, &mut ptr);
                // The engine stores whatever pointer a successful QI returns and
                // later dispatches through it, so the *sub-object offset* is part
                // of the contract. Comparing only "null / object" would hide a
                // layout difference such as a shared vtable.
                sheet.push(format!(
                    "QI {name} -> {:08X} {}",
                    hr.0 as u32,
                    if ptr.is_null() {
                        "null".to_string()
                    } else {
                        format!("offset={:+}", ptr as isize - object as isize)
                    }
                ));
                if !ptr.is_null() {
                    let sub = *(ptr as *const *const ProbeVtbl);
                    ((*sub).release)(ptr);
                }
            }

            let mut properties: *mut APO_REG_PROPERTIES = std::ptr::null_mut();
            let hr = ((*vtbl).get_registration_properties)(object, &mut properties);
            sheet.push(format!("GetRegistrationProperties -> {:08X}", hr.0 as u32));
            if !properties.is_null() {
                let p = &*properties;
                sheet.push(format!(
                    "  clsid={}-flags={:08X} version={}.{} connections={}/{}/{}/{} instances={:08X} interfaces={} iid0={} name=\"{}\" copyright=\"{}\"",
                    format_guid(&p.clsid),
                    p.Flags.0,
                    p.u32MajorVersion,
                    p.u32MinorVersion,
                    p.u32MinInputConnections,
                    p.u32MaxInputConnections,
                    p.u32MinOutputConnections,
                    p.u32MaxOutputConnections,
                    p.u32MaxInstances,
                    p.u32NumAPOInterfaces,
                    format_guid(&p.iidAPOInterfaceList[0]),
                    wide_to_string(&p.szFriendlyName),
                    wide_to_string(&p.szCopyrightInfo)
                ));
                windows::Win32::System::Com::CoTaskMemFree(Some(properties as *const c_void));
            }

            let mut latency = -1i64;
            let hr = ((*vtbl).get_latency)(object, &mut latency);
            sheet.push(format!("GetLatency(unlocked) -> {:08X} value={}", hr.0 as u32, latency));

            let hr = ((*vtbl).reset)(object);
            sheet.push(format!("Reset -> {:08X}", hr.0 as u32));

            let mut channels = 0u32;
            let hr = ((*vtbl).get_input_channel_count)(object, &mut channels);
            sheet.push(format!(
                "GetInputChannelCount -> {:08X} value={}",
                hr.0 as u32, channels
            ));

            // Initialize argument validation, restricted to the cases that
            // return before the init block is dereferenced (a real block would
            // make both implementations read the endpoint property store).
            let zeroed = vec![0u8; 80];
            let init_cases: [(&str, u32, *const u8); 5] = [
                ("(0,null)", 0, std::ptr::null()),
                ("(56,null)", 56, std::ptr::null()),
                ("(80,null)", 80, std::ptr::null()),
                ("(0,block)", 0, zeroed.as_ptr()),
                ("(40,block)", 40, zeroed.as_ptr()),
            ];
            for (label, size, data) in init_cases {
                let hr = ((*vtbl).initialize)(object, size, data);
                sheet.push(format!("Initialize{label} -> {:08X}", hr.0 as u32));
            }

            let cases = [
                ("48k/2ch float32 into 48k/2ch", format(FLOAT32, 2, 48000.0, 4, 32), format(FLOAT32, 2, 48000.0, 4, 32)),
                ("44.1k/2ch float32 into 48k/2ch", format(FLOAT32, 2, 44100.0, 4, 32), format(FLOAT32, 2, 48000.0, 4, 32)),
                ("2ch float32 into 6ch", format(FLOAT32, 2, 48000.0, 4, 32), format(FLOAT32, 6, 48000.0, 4, 32)),
                ("6ch float32 into 2ch", format(FLOAT32, 6, 48000.0, 4, 32), format(FLOAT32, 2, 48000.0, 4, 32)),
                ("16-bit PCM into 48k/2ch", format(PCM16, 2, 48000.0, 2, 16), format(FLOAT32, 2, 48000.0, 4, 32)),
            ];
            for (label, requested, opposite) in cases {
                let requested_media =
                    IAudioMediaType::from_raw(crate::media_type::create_audio_media_type_from_uncompressed_audio_format(&requested));
                let opposite_media =
                    IAudioMediaType::from_raw(crate::media_type::create_audio_media_type_from_uncompressed_audio_format(&opposite));
                let mut supported: *mut c_void = std::ptr::null_mut();
                let hr = ((*vtbl).is_input_format_supported)(
                    object,
                    Interface::as_raw(&opposite_media),
                    Interface::as_raw(&requested_media),
                    &mut supported,
                );
                let mut line = format!(
                    "IsInputFormatSupported({}) -> {:08X}",
                    label,
                    hr.0 as u32
                );
                if !supported.is_null() {
                    // Whether the APO hands back the caller's own media object or
                    // a freshly created one is observable by the engine.
                    line.push_str(if supported == Interface::as_raw(&requested_media) {
                        " supported=is_requested"
                    } else {
                        " supported=is_new_object"
                    });
                    let media = IAudioMediaType::from_raw(supported);
                    let mut got = UNCOMPRESSEDAUDIOFORMAT::default();
                    match media.GetUncompressedAudioFormat(&mut got) {
                        Ok(()) => line.push_str(&format!(" supported={{ {} }}", describe_format(&got))),
                        Err(e) => line.push_str(&format!(" read_error={:08X}", e.code().0 as u32)),
                    }
                } else {
                    line.push_str(" supported=null");
                }
                sheet.push(line);

                let mut out_supported: *mut c_void = std::ptr::null_mut();
                let hr = ((*vtbl).is_output_format_supported)(
                    object,
                    Interface::as_raw(&opposite_media),
                    Interface::as_raw(&requested_media),
                    &mut out_supported,
                );
                let mut out_line = format!(
                    "IsOutputFormatSupported({}) -> {:08X}",
                    label,
                    hr.0 as u32
                );
                if !out_supported.is_null() {
                    let media = IAudioMediaType::from_raw(out_supported);
                    let mut got = UNCOMPRESSEDAUDIOFORMAT::default();
                    match media.GetUncompressedAudioFormat(&mut got) {
                        Ok(()) => out_line.push_str(&format!(" supported={{ {} }}", describe_format(&got))),
                        Err(e) => out_line.push_str(&format!(" read_error={:08X}", e.code().0 as u32)),
                    }
                } else {
                    out_line.push_str(" supported=null");
                }
                sheet.push(out_line);
            }

            sheet
        }

        let reference_path = std::env::var("CPP_APO_PATH")
            .unwrap_or_else(|_| r"D:\zmq13\EqualizerAPO.dll".to_string());
        let mut wide: Vec<u16> = reference_path.encode_utf16().collect();
        wide.push(0);

        let module = unsafe {
            LoadLibraryExW(
                windows::core::PCWSTR::from_raw(wide.as_ptr()),
                None,
                LOAD_WITH_ALTERED_SEARCH_PATH,
            )
        };
        let Ok(module) = module else {
            eprintln!("skipped: reference DLL {reference_path} cannot be loaded");
            return;
        };
        let address = unsafe { GetProcAddress(module, windows::core::s!("DllGetClassObject")) };
        let Some(address) = address else {
            eprintln!("skipped: reference DLL has no DllGetClassObject");
            return;
        };
        let reference_get_class_object: GetClassObject = unsafe {
            std::mem::transmute::<*const (), GetClassObject>(address as *const ())
        };

        unsafe {
            let _ = windows::Win32::System::Com::CoInitializeEx(
                None,
                windows::Win32::System::Com::COINIT_MULTITHREADED,
            );
        }

        unsafe fn create(get: GetClassObject, clsid: &GUID) -> *mut c_void {
            let mut factory: *mut c_void = std::ptr::null_mut();
            let hr = get(clsid, &IID_ICLASSFACTORY, &mut factory);
            assert!(hr.is_ok(), "DllGetClassObject -> {:08X}", hr.0 as u32);
            let factory_vtbl = *(factory as *const *const FactoryVtbl);
            let mut object: *mut c_void = std::ptr::null_mut();
            let hr = ((*factory_vtbl).create_instance)(
                factory,
                std::ptr::null_mut(),
                &apo::IID_IAUDIO_PROCESSING_OBJECT,
                &mut object,
            );
            assert!(hr.is_ok(), "CreateInstance -> {:08X}", hr.0 as u32);
            ((*factory_vtbl).release)(factory);
            object
        }

        let ours = unsafe { create(DllGetClassObject, &eapo_common::EQUALIZERAPO_POST_MIX_GUID) };
        let theirs =
            unsafe { create(reference_get_class_object, &eapo_common::EQUALIZERAPO_POST_MIX_GUID) };
        assert!(!ours.is_null() && !theirs.is_null());

        let mut our_sheet = unsafe { probe(ours) };
        let mut their_sheet = unsafe { probe(theirs) };

        // The engine also binds the *pre-mix* class id, and that is where the
        // reference's `CBaseAudioProcessingObject(regPostMixProperties)` quirk
        // shows up: a pre-mix instance reports the post-mix identity. Probe one
        // from both binaries as well, otherwise the whole pre-mix path stays
        // un-compared.
        for (label, clsid) in [
            ("premix", &eapo_common::EQUALIZERAPO_PRE_MIX_GUID),
            ("postmix2", &eapo_common::EQUALIZERAPO_POST_MIX_GUID),
        ] {
            let our_object = unsafe { create(DllGetClassObject, clsid) };
            let their_object = unsafe { create(reference_get_class_object, clsid) };
            let ours_vtbl = unsafe { *(our_object as *const *const ProbeVtbl) };
            let theirs_vtbl = unsafe { *(their_object as *const *const ProbeVtbl) };
            let prefix = format!("{label} | ");
            our_sheet.extend(
                unsafe { probe(our_object) }
                    .into_iter()
                    .map(|line| format!("{prefix}{line}")),
            );
            their_sheet.extend(
                unsafe { probe(their_object) }
                    .into_iter()
                    .map(|line| format!("{prefix}{line}")),
            );
            unsafe {
                ((*ours_vtbl).release)(our_object);
                ((*theirs_vtbl).release)(their_object);
            }
        }

        // Phase 2: the engine decides whether to lock *after* Initialize, so
        // initialize both objects with a genuine endpoint property store and
        // probe the whole sequence again in that state.
        #[repr(C)]
        struct InitBase {
            cb_size: u32,
            clsid: GUID,
        }
        #[repr(C)]
        struct InitSystemEffects {
            apo_init: InitBase,
            endpoint_properties: *mut c_void,
            system_effects_properties: *mut c_void,
            reserved: *mut c_void,
            device_collection: *mut c_void,
        }
        const _: () = assert!(std::mem::size_of::<InitSystemEffects>() == 56);

        let store = unsafe {
            use windows::Win32::Media::Audio::{eRender, IMMDeviceEnumerator, ERole};
            use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, STGM_READ};
            const CLSID_MMDEVICE_ENUMERATOR: GUID =
                GUID::from_u128(0xbcde0395_e52f_467c_8e3d_c4579291692e);
            let enumerator: Result<IMMDeviceEnumerator, _> =
                CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL);
            match enumerator
                .and_then(|enumerator| enumerator.GetDefaultAudioEndpoint(eRender, ERole(0)))
                .and_then(|device| device.OpenPropertyStore(STGM_READ))
            {
                Ok(property_store) => Some(property_store.into_raw()),
                Err(e) => {
                    eprintln!("phase 2 skipped, no endpoint property store: {e}");
                    None
                }
            }
        };

        let (our_sheet, their_sheet) = match store {
            Some(store) => unsafe {
                let init = InitSystemEffects {
                    apo_init: InitBase {
                        cb_size: std::mem::size_of::<InitSystemEffects>() as u32,
                        clsid: eapo_common::EQUALIZERAPO_POST_MIX_GUID,
                    },
                    endpoint_properties: store,
                    system_effects_properties: store,
                    reserved: std::ptr::null_mut(),
                    device_collection: std::ptr::null_mut(),
                };
                let ours_vtbl = *(ours as *const *const ProbeVtbl);
                let theirs_vtbl = *(theirs as *const *const ProbeVtbl);
                let our_hr = ((*ours_vtbl).initialize)(
                    ours,
                    std::mem::size_of::<InitSystemEffects>() as u32,
                    &init as *const InitSystemEffects as *const u8,
                );
                let their_hr = ((*theirs_vtbl).initialize)(
                    theirs,
                    std::mem::size_of::<InitSystemEffects>() as u32,
                    &init as *const InitSystemEffects as *const u8,
                );
                assert_eq!(
                    our_hr.0 as u32, their_hr.0 as u32,
                    "Initialize itself diverges (ours {our_hr:?}, cpp {their_hr:?})"
                );
                let mut ours_after: Vec<String> = probe(ours)
                    .into_iter()
                    .map(|line| format!("post | {line}"))
                    .collect();
                let mut theirs_after: Vec<String> = probe(theirs)
                    .into_iter()
                    .map(|line| format!("post | {line}"))
                    .collect();
                ours_after.push(format!("post | Initialize hr={:08X}", our_hr.0 as u32));
                theirs_after.push(format!("post | Initialize hr={:08X}", their_hr.0 as u32));
                let mut combined_ours = our_sheet;
                let mut combined_theirs = their_sheet;
                for line in combined_ours.iter_mut() {
                    *line = format!("pre  | {line}");
                }
                for line in combined_theirs.iter_mut() {
                    *line = format!("pre  | {line}");
                }
                combined_ours.extend(ours_after);
                combined_theirs.extend(theirs_after);
                (combined_ours, combined_theirs)
            },
            None => {
                let mut ours = our_sheet;
                let mut theirs = their_sheet;
                for line in ours.iter_mut() {
                    *line = format!("pre  | {line}");
                }
                for line in theirs.iter_mut() {
                    *line = format!("pre  | {line}");
                }
                (ours, theirs)
            }
        };

        unsafe {
            let ours_vtbl = *(ours as *const *const ProbeVtbl);
            let theirs_vtbl = *(theirs as *const *const ProbeVtbl);
            ((*ours_vtbl).release)(ours);
            ((*theirs_vtbl).release)(theirs);
            windows::Win32::System::Com::CoUninitialize();
        }

        let mismatches: Vec<String> = our_sheet
            .iter()
            .zip(their_sheet.iter())
            .enumerate()
            .filter(|(_, (a, b))| a != b)
            .map(|(index, (a, b))| format!("line {index}\n  ours:    {a}\n  cpp:     {b}"))
            .collect();
        if !mismatches.is_empty() {
            panic!(
                "{} probe(s) diverge from the reference C++ binary:\n{}",
                mismatches.len(),
                mismatches.join("\n")
            );
        }
        eprintln!(
            "all {} probes match the reference C++ binary",
            our_sheet.len()
        );
    }
    /// Opt-in differential: drive the full engine sequence — real endpoint
    /// property store for `Initialize`, an in-place connection descriptor for
    /// `LockForProcess`, then `APOProcess` on a 480-frame block — against our
    /// object and the reference C++ binary inside one process, and compare the
    /// outcomes sample by sample. No audio stream, endpoint change or service
    /// restart is involved.
    #[test]
    #[ignore]
    fn lock_and_process_match_the_reference_cpp_binary() {
        use std::mem::ManuallyDrop;
        use windows::core::Interface;
        use windows::Win32::Media::Audio::Apo::{
            IAudioMediaType, IAudioProcessingObject, IAudioProcessingObjectConfiguration,
            IAudioProcessingObjectRT, APO_CONNECTION_BUFFER_TYPE_DEPENDANT,
            APO_CONNECTION_DESCRIPTOR, APO_CONNECTION_PROPERTY, BUFFER_VALID,
            UNCOMPRESSEDAUDIOFORMAT,
        };
        use windows::Win32::Media::Audio::{eRender, IMMDeviceEnumerator, ERole};
        use windows::Win32::System::Com::{
            CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_ALL,
            COINIT_MULTITHREADED, STGM_READ,
        };
        use windows::Win32::System::LibraryLoader::{
            GetProcAddress, LoadLibraryExW, LOAD_WITH_ALTERED_SEARCH_PATH,
        };

        const FRAMES: usize = 480;
        const CHANNELS: usize = 2;
        const CLSID_MMDEVICE_ENUMERATOR: GUID =
            GUID::from_u128(0xbcde0395_e52f_467c_8e3d_c4579291692e);
        const IEEE_FLOAT: GUID = GUID::from_u128(0x00000003_0000_0010_8000_00aa00389b71);

        #[repr(C)]
        struct InitBase {
            cb_size: u32,
            clsid: GUID,
        }
        #[repr(C)]
        struct InitSystemEffects {
            apo_init: InitBase,
            endpoint_properties: *mut c_void,
            system_effects_properties: *mut c_void,
            reserved: *mut c_void,
            device_collection: *mut c_void,
        }

        let reference_path = std::env::var("CPP_APO_PATH")
            .unwrap_or_else(|_| r"D:\zmq13\EqualizerAPO.dll".to_string());
        let mut wide_path: Vec<u16> = reference_path.encode_utf16().collect();
        wide_path.push(0);
        let Ok(module) = (unsafe {
            LoadLibraryExW(
                windows::core::PCWSTR::from_raw(wide_path.as_ptr()),
                None,
                LOAD_WITH_ALTERED_SEARCH_PATH,
            )
        }) else {
            eprintln!("skipped: cannot load {reference_path}");
            return;
        };
        let Some(address) =
            (unsafe { GetProcAddress(module, windows::core::s!("DllGetClassObject")) })
        else {
            eprintln!("skipped: reference DLL has no DllGetClassObject");
            return;
        };
        type GetClassObject =
            unsafe extern "system" fn(*const GUID, *const GUID, *mut *mut c_void) -> HRESULT;
        let reference_get_class_object: GetClassObject =
            unsafe { std::mem::transmute::<*const (), GetClassObject>(address as *const ()) };

        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
            let enumerator: IMMDeviceEnumerator =
                CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL)
                    .expect("no device enumerator");
            let device = enumerator
                .GetDefaultAudioEndpoint(eRender, ERole(0))
                .expect("no render endpoint");
            let store = device
                .OpenPropertyStore(STGM_READ)
                .expect("OpenPropertyStore")
                .into_raw();

            let create = |get: GetClassObject| -> IAudioProcessingObject {
                let mut factory: *mut c_void = std::ptr::null_mut();
                let hr = get(
                    &eapo_common::EQUALIZERAPO_POST_MIX_GUID,
                    &IID_ICLASSFACTORY,
                    &mut factory,
                );
                assert!(hr.is_ok(), "DllGetClassObject -> {hr:?}");
                let factory_vtbl = *(factory as *const *const FactoryVtbl);
                let mut object: *mut c_void = std::ptr::null_mut();
                let hr = ((*factory_vtbl).create_instance)(
                    factory,
                    std::ptr::null_mut(),
                    &apo::IID_IAUDIO_PROCESSING_OBJECT,
                    &mut object,
                );
                assert!(hr.is_ok(), "CreateInstance -> {hr:?}");
                ((*factory_vtbl).release)(factory);
                IAudioProcessingObject::from_raw(object)
            };

            let mut format = UNCOMPRESSEDAUDIOFORMAT::default();
            format.guidFormatType = IEEE_FLOAT;
            format.dwSamplesPerFrame = CHANNELS as u32;
            format.dwBytesPerSampleContainer = 4;
            format.dwValidBitsPerSample = 32;
            format.fFramesPerSecond = 48000.0;
            format.dwChannelMask = 3;
            let media = IAudioMediaType::from_raw(
                crate::media_type::create_audio_media_type_from_uncompressed_audio_format(&format),
            );
            let mut descriptor = APO_CONNECTION_DESCRIPTOR {
                Type: APO_CONNECTION_BUFFER_TYPE_DEPENDANT,
                pBuffer: 0,
                u32MaxFrameCount: FRAMES as u32,
                pFormat: ManuallyDrop::new(Some(media.clone())),
                u32Signature: 0,
            };
            let connections = [&descriptor as *const APO_CONNECTION_DESCRIPTOR];

            let init = InitSystemEffects {
                apo_init: InitBase {
                    cb_size: std::mem::size_of::<InitSystemEffects>() as u32,
                    clsid: eapo_common::EQUALIZERAPO_POST_MIX_GUID,
                },
                endpoint_properties: store,
                system_effects_properties: store,
                reserved: std::ptr::null_mut(),
                device_collection: std::ptr::null_mut(),
            };
            let init_bytes = std::slice::from_raw_parts(
                &init as *const InitSystemEffects as *const u8,
                std::mem::size_of::<InitSystemEffects>(),
            );

            let mut run = |label: &str, object: IAudioProcessingObject| -> (String, Vec<f32>) {
                let init_hr = object.Initialize(init_bytes);
                let cfg: IAudioProcessingObjectConfiguration =
                    object.cast().expect("configuration interface");
                let lock_hr = cfg.LockForProcess(&connections, &connections);
                let mut summary = format!(
                    "{label}: Initialize={init_hr:?} LockForProcess={lock_hr:?}"
                );
                let mut samples = vec![0.0f32; FRAMES * CHANNELS];
                for frame in 0..FRAMES {
                    let value =
                        (frame as f32 * 440.0 * std::f32::consts::PI * 2.0 / 48000.0).sin() * 0.25;
                    samples[frame * CHANNELS] = value;
                    samples[frame * CHANNELS + 1] = value;
                }
                let before = samples.clone();
                if lock_hr.is_ok() {
                    let rt: IAudioProcessingObjectRT = object.cast().expect("rt interface");
                    let mut property = APO_CONNECTION_PROPERTY {
                        pBuffer: samples.as_mut_ptr() as usize,
                        u32ValidFrameCount: FRAMES as u32,
                        u32BufferFlags: BUFFER_VALID,
                        u32Signature: 0,
                    };
                    let in_props = [&property as *const APO_CONNECTION_PROPERTY];
                    let mut out_props = [&mut property as *mut APO_CONNECTION_PROPERTY];
                    rt.APOProcess(1, in_props.as_ptr(), 1, out_props.as_mut_ptr());
                    let peak = samples.iter().fold(0.0f32, |m, s| m.max(s.abs()));
                    summary.push_str(&format!(
                        " processed peak={peak:.6} frames={}",
                        property.u32ValidFrameCount
                    ));
                }
                if lock_hr.is_ok() {
                    let _ = cfg.UnlockForProcess();
                }
                // Leak the interface references: Rust's drop order would mask
                // the teardown behaviour this test exists to observe.
                std::mem::forget(cfg);
                std::mem::forget(object);
                let _ = before;
                (summary, samples)
            };

            let (our_summary, our_samples) = run("ours", create(DllGetClassObject));
            let (their_summary, their_samples) = run("cpp", create(reference_get_class_object));
            eprintln!("{our_summary}");
            eprintln!("{their_summary}");

            let our_locked = our_summary.contains("LockForProcess=Ok(");
            let their_locked = their_summary.contains("LockForProcess=Ok(");
            assert_eq!(
                our_locked, their_locked,
                "lock outcome diverges:\n  ours: {our_summary}\n  cpp:  {their_summary}"
            );
            if our_locked {
                let mut worst = 0.0f32;
                for (a, b) in our_samples.iter().zip(their_samples.iter()) {
                    worst = worst.max((a - b).abs());
                }
                eprintln!("max processed sample difference: {worst:.3e}");
                assert!(worst < 1e-6, "processed audio diverges by {worst}");
            }
            // No CoUninitialize: the references above are deliberately leaked.
        }
    }
}
