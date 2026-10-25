//! VST 2.4 host ABI subset translated from `helpers/aeffectx.h`.

use std::ffi::{c_char, c_void};

pub const VST_MAGICNUMBER: i32 = 0x5673_7450;
pub const VST_EQUALIZER_APO_ID: isize = 0x4541_504F;
pub const VST_VERSION_2_4_0_0: isize = 2400;
pub const VST_VENDOR_VERSION: isize = 0x0104_0200;

pub const VST_EFFECT_FLAG_SUPPORTS_FLOAT: i32 = 1 << 4;
pub const VST_EFFECT_FLAG_CHUNKS: i32 = 1 << 5;
pub const VST_EFFECT_FLAG_SUPPORTS_DOUBLE: i32 = 1 << 12;

pub const VST_HOST_OPCODE_AUTOMATE: i32 = 0x00;
pub const VST_HOST_OPCODE_VST_VERSION: i32 = 0x01;
pub const VST_HOST_OPCODE_CURRENT_EFFECT_ID: i32 = 0x02;
pub const VST_HOST_OPCODE_KEEPALIVE_OR_IDLE: i32 = 0x03;
pub const VST_HOST_OPCODE_PIN_CONNECTED: i32 = 0x04;
pub const VST_HOST_OPCODE_WANT_MIDI: i32 = 0x06;
pub const VST_HOST_OPCODE_GET_TIME: i32 = 0x07;
pub const VST_HOST_OPCODE_IO_NEED_IDLE: i32 = 0x0D;
pub const VST_HOST_OPCODE_EDITOR_RESIZE: i32 = 0x0F;
pub const VST_HOST_OPCODE_GET_SAMPLE_RATE: i32 = 0x10;
#[allow(dead_code)]
pub const VST_HOST_OPCODE_GET_BLOCK_SIZE: i32 = 0x11;
pub const VST_HOST_OPCODE_GET_REPLACE_OR_ACCUMULATE: i32 = 0x16;
pub const VST_HOST_OPCODE_GET_ACTIVE_THREAD: i32 = 0x17;
pub const VST_HOST_OPCODE_SUPPORTS: i32 = 0x25;
pub const VST_HOST_OPCODE_LANGUAGE: i32 = 0x26;
pub const VST_HOST_OPCODE_EDITOR_UPDATE: i32 = 0x2A;
pub const VST_HOST_OPCODE_PARAM_START_EDIT: i32 = 0x2B;
pub const VST_HOST_OPCODE_PARAM_STOP_EDIT: i32 = 0x2C;

pub const VST_EFFECT_OPCODE_INITIALIZE: i32 = 0x00;
pub const VST_EFFECT_OPCODE_DESTROY: i32 = 0x01;
pub const VST_EFFECT_OPCODE_PARAM_GET_NAME: i32 = 0x08;
pub const VST_EFFECT_OPCODE_SET_SAMPLE_RATE: i32 = 0x0A;
pub const VST_EFFECT_OPCODE_SET_BLOCK_SIZE: i32 = 0x0B;
pub const VST_EFFECT_OPCODE_SUSPEND: i32 = 0x0C;
pub const VST_EFFECT_OPCODE_EDITOR_GET_RECT: i32 = 0x0D;
pub const VST_EFFECT_OPCODE_EDITOR_OPEN: i32 = 0x0E;
pub const VST_EFFECT_OPCODE_EDITOR_CLOSE: i32 = 0x0F;
pub const VST_EFFECT_OPCODE_EDITOR_KEEP_ALIVE: i32 = 0x13;
pub const VST_EFFECT_OPCODE_GET_CHUNK_DATA: i32 = 0x17;
pub const VST_EFFECT_OPCODE_SET_CHUNK_DATA: i32 = 0x18;
pub const VST_EFFECT_OPCODE_EFFECT_NAME: i32 = 0x2D;
pub const VST_EFFECT_OPCODE_PRODUCT_NAME: i32 = 0x30;
pub const VST_EFFECT_OPCODE_VENDOR_VERSION: i32 = 0x31;
pub const VST_EFFECT_OPCODE_PROCESS_BEGIN: i32 = 0x47;
pub const VST_EFFECT_OPCODE_PROCESS_END: i32 = 0x48;

#[repr(C)]
#[derive(Clone, Copy, Debug, Default)]
pub struct VstRect {
    pub top: i16,
    pub left: i16,
    pub bottom: i16,
    pub right: i16,
}

#[repr(C)]
#[derive(Clone, Copy, Debug)]
pub struct VstTimeInfo {
    pub sample_pos: f64,
    pub sample_rate: f64,
    pub nano_seconds: f64,
    pub ppq_pos: f64,
    pub tempo: f64,
    pub bar_start_pos: f64,
    pub cycle_start_pos: f64,
    pub cycle_end_pos: f64,
    pub time_sig_numerator: i32,
    pub time_sig_denominator: i32,
    pub empty3: [c_char; 12],
    pub flags: i32,
}

impl Default for VstTimeInfo {
    fn default() -> Self {
        Self {
            sample_pos: 0.0,
            sample_rate: 0.0,
            nano_seconds: 0.0,
            ppq_pos: 0.0,
            tempo: 0.0,
            bar_start_pos: 0.0,
            cycle_start_pos: 0.0,
            cycle_end_pos: 0.0,
            time_sig_numerator: 0,
            time_sig_denominator: 0,
            empty3: [0; 12],
            flags: 0xFFFF,
        }
    }
}

pub type VstHostCallback =
    unsafe extern "C" fn(*mut VstEffect, i32, i32, isize, *const c_char, f32) -> isize;
pub type VstEffectControl =
    unsafe extern "C" fn(*mut VstEffect, i32, i32, isize, *mut c_void, f32) -> isize;
pub type VstEffectProcess = unsafe extern "C" fn(
    *mut VstEffect,
    *const *const f32,
    *mut *mut f32,
    i32,
);
pub type VstEffectSetParameter = unsafe extern "C" fn(*mut VstEffect, u32, f32);
pub type VstEffectGetParameter = unsafe extern "C" fn(*mut VstEffect, u32) -> f32;
pub type VstEffectProcessFloat = unsafe extern "C" fn(
    *mut VstEffect,
    *const *const f32,
    *mut *mut f32,
    i32,
);
pub type VstEffectProcessDouble = unsafe extern "C" fn(
    *mut VstEffect,
    *const *const f64,
    *mut *mut f64,
    i32,
);
pub type VstPluginMain = unsafe extern "C" fn(VstHostCallback) -> *mut VstEffect;

#[repr(C)]
pub struct VstEffect {
    pub magic_number: i32,
    pub control: Option<VstEffectControl>,
    pub process: Option<VstEffectProcess>,
    pub set_parameter: Option<VstEffectSetParameter>,
    pub get_parameter: Option<VstEffectGetParameter>,
    pub num_programs: i32,
    pub num_params: i32,
    pub num_inputs: i32,
    pub num_outputs: i32,
    pub flags: i32,
    pub _unknown_00: *mut c_void,
    pub _unknown_01: *mut c_void,
    pub delay: i32,
    pub _unknown_02: i32,
    pub _unknown_03: i32,
    pub input_output_ratio: f32,
    pub effect_internal: *mut c_void,
    pub host_internal: *mut c_void,
    pub unique_id: i32,
    pub version: i32,
    pub process_float: Option<VstEffectProcessFloat>,
    pub process_double: Option<VstEffectProcessDouble>,
    pub _unknown: [u8; 56],
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::mem::{offset_of, size_of};

    #[test]
    fn effect_layout_matches_vst_2_4() {
        assert_eq!(size_of::<VstEffect>(), 192);
        assert_eq!(offset_of!(VstEffect, control), 8);
        assert_eq!(offset_of!(VstEffect, num_programs), 40);
        assert_eq!(offset_of!(VstEffect, flags), 56);
        assert_eq!(offset_of!(VstEffect, delay), 80);
        assert_eq!(offset_of!(VstEffect, host_internal), 104);
        assert_eq!(offset_of!(VstEffect, process_float), 120);
        assert_eq!(offset_of!(VstEffect, process_double), 128);
        assert_eq!(size_of::<VstTimeInfo>(), 88);
    }
}
