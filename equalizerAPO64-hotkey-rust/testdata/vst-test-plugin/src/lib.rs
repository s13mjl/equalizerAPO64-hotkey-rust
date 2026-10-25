//! Minimal VST 2.4 effect used to verify the Rust host.
//! Double-precision pass-through with a gain taken from the opaque chunk.

use std::ffi::c_void;
use std::sync::atomic::{AtomicU64, Ordering};

const VST_MAGIC: i32 = 0x5673_7450;
const FLAG_SUPPORTS_FLOAT: i32 = 1 << 4;
const FLAG_CHUNKS: i32 = 1 << 5;
const FLAG_SUPPORTS_DOUBLE: i32 = 1 << 12;

static GAIN_BITS: AtomicU64 = AtomicU64::new(0x3FE0_0000_0000_0000); // 0.5
static mut CHUNK: [u8; 8] = [0; 8];

type HostCallback = unsafe extern "C" fn(*mut VstEffect, i32, i32, isize, *const i8, f32) -> isize;
type Dispatcher = unsafe extern "C" fn(*mut VstEffect, i32, i32, isize, *mut c_void, f32) -> isize;
type ProcessF32 = unsafe extern "C" fn(*mut VstEffect, *const *const f32, *mut *mut f32, i32);
type ProcessF64 = unsafe extern "C" fn(*mut VstEffect, *const *const f64, *mut *mut f64, i32);

#[repr(C)]
struct VstEffect {
    magic: i32,
    dispatcher: Option<Dispatcher>,
    process: Option<unsafe extern "C" fn(*mut VstEffect, *const *const f32, *mut *mut f32, i32)>,
    set_parameter: Option<unsafe extern "C" fn(*mut VstEffect, u32, f32)>,
    get_parameter: Option<unsafe extern "C" fn(*mut VstEffect, u32) -> f32>,
    num_programs: i32,
    num_params: i32,
    num_inputs: i32,
    num_outputs: i32,
    flags: i32,
    reserved1: *mut c_void,
    reserved2: *mut c_void,
    delay: i32,
    reserved3: i32,
    reserved4: i32,
    input_output_ratio: f32,
    object: *mut c_void,
    user: *mut c_void,
    unique_id: i32,
    version: i32,
    process_float: Option<ProcessF32>,
    process_double: Option<ProcessF64>,
    reserved: [u8; 56],
}

unsafe extern "C" fn dispatcher(
    effect: *mut VstEffect,
    opcode: i32,
    _index: i32,
    value: isize,
    ptr: *mut c_void,
    _opt: f32,
) -> isize {
    match opcode {
        1 => {
            drop(unsafe { Box::from_raw(effect) });
            0
        }
        8 => {
            if !ptr.is_null() {
                let dst = unsafe { std::slice::from_raw_parts_mut(ptr as *mut u8, 5) };
                dst.copy_from_slice(b"Gain\0");
            }
            0
        }
        13 => {
            #[repr(C)]
            struct Rect {
                top: i16,
                left: i16,
                bottom: i16,
                right: i16,
            }
            static mut RECT: Rect = Rect { top: 0, left: 0, bottom: 100, right: 200 };
            if !ptr.is_null() {
                unsafe { *(ptr as *mut *mut Rect) = std::ptr::addr_of_mut!(RECT) };
            }
            0
        }
        23 => {
            let gain = f64::from_bits(GAIN_BITS.load(Ordering::Relaxed));
            let bytes = gain.to_le_bytes();
            unsafe {
                CHUNK = bytes;
                *(ptr as *mut *mut u8) = std::ptr::addr_of_mut!(CHUNK) as *mut u8;
            }
            8
        }
        24 => {
            if value >= 8 && !ptr.is_null() {
                let bytes = unsafe { std::slice::from_raw_parts(ptr as *const u8, 8) };
                let mut gain = [0u8; 8];
                gain.copy_from_slice(bytes);
                GAIN_BITS.store(f64::from_le_bytes(gain).to_bits(), Ordering::Relaxed);
            }
            0
        }
        45 => {
            if !ptr.is_null() {
                let name = b"Rust Test VST\0";
                unsafe {
                    std::ptr::copy_nonoverlapping(name.as_ptr(), ptr as *mut u8, name.len());
                }
            }
            0
        }
        47 => {
            if !ptr.is_null() {
                let vendor = b"EqualizerAPO Port\0";
                unsafe {
                    std::ptr::copy_nonoverlapping(vendor.as_ptr(), ptr as *mut u8, vendor.len());
                }
            }
            0
        }
        49 => 1,
        _ => 0,
    }
}

unsafe extern "C" fn process_f32(
    _effect: *mut VstEffect,
    inputs: *const *const f32,
    outputs: *mut *mut f32,
    frames: i32,
) {
    let gain = f64::from_bits(GAIN_BITS.load(Ordering::Relaxed)) as f32;
    for channel in 0..2 {
        let input = unsafe { *inputs.add(channel) };
        let output = unsafe { *outputs.add(channel) };
        for frame in 0..frames as usize {
            unsafe { *output.add(frame) = *input.add(frame) * gain };
        }
    }
}

unsafe extern "C" fn process_f64(
    _effect: *mut VstEffect,
    inputs: *const *const f64,
    outputs: *mut *mut f64,
    frames: i32,
) {
    let gain = f64::from_bits(GAIN_BITS.load(Ordering::Relaxed));
    for channel in 0..2 {
        let input = unsafe { *inputs.add(channel) };
        let output = unsafe { *outputs.add(channel) };
        for frame in 0..frames as usize {
            unsafe { *output.add(frame) = *input.add(frame) * gain };
        }
    }
}

#[no_mangle]
pub extern "C" fn VSTPluginMain(_callback: HostCallback) -> *mut VstEffect {
    let effect = VstEffect {
        magic: VST_MAGIC,
        dispatcher: Some(dispatcher),
        process: None,
        set_parameter: None,
        get_parameter: None,
        num_programs: 0,
        num_params: 1,
        num_inputs: 2,
        num_outputs: 2,
        flags: FLAG_SUPPORTS_FLOAT | FLAG_CHUNKS | FLAG_SUPPORTS_DOUBLE,
        reserved1: std::ptr::null_mut(),
        reserved2: std::ptr::null_mut(),
        delay: 0,
        reserved3: 0,
        reserved4: 0,
        input_output_ratio: 1.0,
        object: std::ptr::null_mut(),
        user: std::ptr::null_mut(),
        unique_id: 0x4541_504F,
        version: 1,
        process_float: Some(process_f32),
        process_double: Some(process_f64),
        reserved: [0; 56],
    };
    Box::into_raw(Box::new(effect))
}
