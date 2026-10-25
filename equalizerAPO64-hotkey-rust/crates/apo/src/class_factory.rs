//! Port of `ClassFactory.h/.cpp`.

use std::ffi::c_void;
use std::ptr;
use std::sync::atomic::{AtomicU32, Ordering};

use windows::core::{GUID, HRESULT};
use windows::Win32::Foundation::{E_NOINTERFACE, E_OUTOFMEMORY, E_POINTER};

use crate::apo::{EqualizerApo, IID_IUNKNOWN};

const S_OK: HRESULT = HRESULT(0);
const IID_ICLASS_FACTORY: GUID = GUID::from_u128(0x00000001_0000_0000_c000_000000000046);

type QiFn = unsafe extern "system" fn(*mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type RefFn = unsafe extern "system" fn(*mut c_void) -> u32;
type ReleaseFn = unsafe extern "system" fn(*mut c_void) -> u32;
type CreateInstanceFn =
    unsafe extern "system" fn(*mut c_void, *mut c_void, *const GUID, *mut *mut c_void) -> HRESULT;
type LockServerFn = unsafe extern "system" fn(*mut c_void, i32) -> HRESULT;

#[repr(C)]
struct ClassFactoryVtbl {
    query_interface: QiFn,
    add_ref: RefFn,
    release: ReleaseFn,
    create_instance: CreateInstanceFn,
    lock_server: LockServerFn,
}

unsafe impl Sync for ClassFactoryVtbl {}

static CLASS_FACTORY_VTBL: ClassFactoryVtbl = ClassFactoryVtbl {
    query_interface: factory_query_interface,
    add_ref: factory_add_ref,
    release: factory_release,
    create_instance: factory_create_instance,
    lock_server: factory_lock_server,
};

#[repr(C)]
pub(crate) struct ClassFactory {
    vtbl: *const ClassFactoryVtbl,
    ref_count: AtomicU32,
    clsid: GUID,
}

/// C++ `new ClassFactory()`; the returned object has one reference.
pub(crate) unsafe fn create_factory(clsid: GUID) -> *mut ClassFactory {
    let factory = Box::new(ClassFactory {
        vtbl: &CLASS_FACTORY_VTBL,
        ref_count: AtomicU32::new(1),
        clsid,
    });
    Box::into_raw(factory)
}

pub(crate) unsafe fn query_interface_impl(
    this: *mut ClassFactory,
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
    if iid == IID_IUNKNOWN || iid == IID_ICLASS_FACTORY {
        unsafe {
            add_ref_impl(this);
            *ppv = this as *mut c_void;
        }
        S_OK
    } else {
        E_NOINTERFACE
    }
}

unsafe fn add_ref_impl(this: *mut ClassFactory) -> u32 {
    let factory = unsafe { &*(this as *const ClassFactory) };
    factory.ref_count.fetch_add(1, Ordering::SeqCst) + 1
}

pub(crate) unsafe fn release_impl(this: *mut ClassFactory) -> u32 {
    let factory = unsafe { &*(this as *const ClassFactory) };
    let old = factory.ref_count.fetch_sub(1, Ordering::SeqCst);
    if old == 1 {
        unsafe {
            drop(Box::from_raw(this as *mut ClassFactory));
        }
        0
    } else {
        old - 1
    }
}

pub(crate) unsafe fn create_instance(
    this: *mut ClassFactory,
    p_unknown_outer: *mut c_void,
    iid: &GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    create_instance_impl(this, p_unknown_outer, iid, ppv)
}

fn create_instance_impl(
    this: *mut ClassFactory,
    p_unknown_outer: *mut c_void,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    if ppv.is_null() || iid.is_null() {
        return E_POINTER;
    }
    let iid_value = unsafe { *iid };
    if !p_unknown_outer.is_null() && iid_value != IID_IUNKNOWN {
        return E_NOINTERFACE;
    }
    unsafe {
        *ppv = ptr::null_mut();
    }

    let apo = unsafe { EqualizerApo::create_instance(p_unknown_outer, (*this).clsid) };
    if apo.is_null() {
        return E_OUTOFMEMORY;
    }

    let hr = unsafe { EqualizerApo::non_delegating_query_interface(apo, iid, ppv) };
    unsafe {
        EqualizerApo::non_delegating_release(apo);
    }
    hr
}

fn lock_server_impl(_this: *mut ClassFactory, lock: i32) -> HRESULT {
    if lock != 0 {
        crate::registration::add_factory_lock();
    } else {
        crate::registration::release_factory_lock();
    }
    S_OK
}

unsafe extern "system" fn factory_query_interface(
    this: *mut c_void,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    unsafe { query_interface_impl(this as *mut ClassFactory, iid, ppv) }
}

unsafe extern "system" fn factory_add_ref(this: *mut c_void) -> u32 {
    unsafe { add_ref_impl(this as *mut ClassFactory) }
}

unsafe extern "system" fn factory_release(this: *mut c_void) -> u32 {
    unsafe { release_impl(this as *mut ClassFactory) }
}

unsafe extern "system" fn factory_create_instance(
    this: *mut c_void,
    p_unknown_outer: *mut c_void,
    iid: *const GUID,
    ppv: *mut *mut c_void,
) -> HRESULT {
    create_instance_impl(this as *mut ClassFactory, p_unknown_outer, iid, ppv)
}

unsafe extern "system" fn factory_lock_server(this: *mut c_void, lock: i32) -> HRESULT {
    lock_server_impl(this as *mut ClassFactory, lock)
}
