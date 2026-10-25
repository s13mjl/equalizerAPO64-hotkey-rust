//! Port of `helpers/VSTPluginInstance.{h,cpp}`.

use std::cell::UnsafeCell;
use std::collections::HashMap;
use std::ffi::{c_char, c_void, CStr};
use std::ptr;
use std::sync::atomic::{AtomicI32, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};

use eapo_common::string_helper::utf8_to_wstring;
use windows::Win32::Foundation::HWND;

use crate::vst_abi::*;
use crate::vst_plugin_library::VSTPluginLibrary;

struct HostContext {
    sample_rate_bits: AtomicU32,
    used_channel_count: AtomicI32,
    process_level: AtomicI32,
    language: AtomicI32,
    automate_func: Mutex<Option<Arc<dyn Fn() + Send + Sync>>>,
    size_window_func: Mutex<Option<Arc<dyn Fn(i32, i32) + Send + Sync>>>,
    time_info: UnsafeCell<VstTimeInfo>,
}

impl HostContext {
    fn new(process_level: i32) -> Self {
        Self {
            sample_rate_bits: AtomicU32::new(0.0f32.to_bits()),
            used_channel_count: AtomicI32::new(-1),
            process_level: AtomicI32::new(process_level),
            language: AtomicI32::new(1),
            automate_func: Mutex::new(None),
            size_window_func: Mutex::new(None),
            time_info: UnsafeCell::new(VstTimeInfo::default()),
        }
    }
}

pub struct VstPluginInstance {
    library: Arc<VSTPluginLibrary>,
    effect: *mut VstEffect,
    context: Box<HostContext>,
    sample_rate: f32,
    used_channel_count: i32,
}

unsafe impl Send for VstPluginInstance {}

impl VstPluginInstance {
    pub fn new(library: Arc<VSTPluginLibrary>, process_level: i32) -> Self {
        Self {
            library,
            effect: ptr::null_mut(),
            context: Box::new(HostContext::new(process_level)),
            sample_rate: 0.0,
            used_channel_count: -1,
        }
    }

    pub fn initialize(&mut self) -> bool {
        let Some(vst_plugin_main) = self.library.vst_plugin_main() else {
            return false;
        };
        let effect = unsafe { vst_plugin_main(host_callback) };
        if effect.is_null() {
            return false;
        }

        unsafe {
            (*effect).host_internal = &*self.context as *const HostContext as *mut HostContext
                as *mut c_void;
        }

        if unsafe { (*effect).magic_number } == VST_MAGICNUMBER {
            self.effect = effect;
            self.control(VST_EFFECT_OPCODE_INITIALIZE, 0, 0, ptr::null_mut(), 0.0);
            self.used_channel_count = self.num_inputs().max(self.num_outputs());
            self.context
                .used_channel_count
                .store(self.used_channel_count, Ordering::Release);
            true
        } else {
            false
        }
    }

    pub fn num_inputs(&self) -> i32 {
        if self.effect.is_null() {
            0
        } else {
            unsafe { (*self.effect).num_inputs }
        }
    }

    pub fn num_outputs(&self) -> i32 {
        if self.effect.is_null() {
            0
        } else {
            unsafe { (*self.effect).num_outputs }
        }
    }

    pub fn can_replacing(&self) -> bool {
        if self.effect.is_null() {
            true
        } else {
            unsafe { (*self.effect).flags & VST_EFFECT_FLAG_SUPPORTS_FLOAT != 0 }
        }
    }

    pub fn unique_id(&self) -> i32 {
        if self.effect.is_null() {
            0
        } else {
            unsafe { (*self.effect).unique_id }
        }
    }

    pub fn get_name(&self) -> String {
        if self.effect.is_null() {
            return String::new();
        }
        let mut buffer = [0u8; 256];
        self.control(
            VST_EFFECT_OPCODE_EFFECT_NAME,
            0,
            0,
            buffer.as_mut_ptr().cast(),
            0.0,
        );
        buffer[255] = 0;
        utf8_to_wstring(&buffer)
    }

    pub fn get_used_channel_count(&self) -> i32 {
        self.used_channel_count
    }

    pub fn set_used_channel_count(&mut self, count: i32) {
        self.used_channel_count = count;
        self.context
            .used_channel_count
            .store(count, Ordering::Release);
    }

    pub fn get_sample_rate(&self) -> f32 {
        self.sample_rate
    }

    pub fn get_process_level(&self) -> i32 {
        self.context.process_level.load(Ordering::Acquire)
    }

    pub fn set_process_level(&self, value: i32) {
        self.context.process_level.store(value, Ordering::Release);
    }

    pub fn get_language(&self) -> i32 {
        self.context.language.load(Ordering::Acquire)
    }

    pub fn set_language(&self, value: i32) {
        self.context.language.store(value, Ordering::Release);
    }

    pub fn can_double_replacing(&self) -> bool {
        !self.effect.is_null()
            && unsafe { (*self.effect).flags & VST_EFFECT_FLAG_SUPPORTS_DOUBLE != 0 }
    }

    pub fn get_initial_delay(&self) -> i32 {
        if self.effect.is_null() {
            0
        } else {
            unsafe { (*self.effect).delay }
        }
    }

    pub fn prepare_for_processing(&mut self, sample_rate: f32, block_size: i32) {
        if self.effect.is_null() {
            return;
        }
        self.sample_rate = sample_rate;
        self.context
            .sample_rate_bits
            .store(sample_rate.to_bits(), Ordering::Release);
        self.control(
            VST_EFFECT_OPCODE_SET_SAMPLE_RATE,
            0,
            0,
            ptr::null_mut(),
            sample_rate,
        );
        self.control(
            VST_EFFECT_OPCODE_SET_BLOCK_SIZE,
            0,
            block_size as isize,
            ptr::null_mut(),
            0.0,
        );
    }

    pub fn write_to_effect(
        &self,
        chunk_data: &str,
        param_map: &HashMap<String, f32>,
    ) {
        if self.effect.is_null() {
            return;
        }
        let flags = unsafe { (*self.effect).flags };
        if flags & VST_EFFECT_FLAG_CHUNKS != 0 {
            if !chunk_data.is_empty() {
                if let Some(buffer) = base64_decode(chunk_data) {
                    self.control(
                        VST_EFFECT_OPCODE_SET_CHUNK_DATA,
                        1,
                        buffer.len() as isize,
                        buffer.as_ptr() as *mut c_void,
                        0.0,
                    );
                }
            }
        } else {
            let num_params = unsafe { (*self.effect).num_params };
            for i in 0..num_params {
                let mut buffer = [0u8; 256];
                self.control(
                    VST_EFFECT_OPCODE_PARAM_GET_NAME,
                    i,
                    0,
                    buffer.as_mut_ptr().cast(),
                    0.0,
                );
                buffer[255] = 0;
                let name = utf8_to_wstring(&buffer);
                if let Some(value) = param_map.get(&name) {
                    if let Some(set_parameter) = unsafe { (*self.effect).set_parameter } {
                        unsafe { set_parameter(self.effect, i as u32, *value) };
                    }
                }
            }
        }
    }

    pub fn read_from_effect(
        &self,
        chunk_data: &mut String,
        param_map: &mut HashMap<String, f32>,
    ) {
        if self.effect.is_null() {
            return;
        }
        chunk_data.clear();
        param_map.clear();

        let flags = unsafe { (*self.effect).flags };
        if flags & VST_EFFECT_FLAG_CHUNKS != 0 {
            let mut chunk: *mut u8 = ptr::null_mut();
            let size = self.control(
                VST_EFFECT_OPCODE_GET_CHUNK_DATA,
                1,
                0,
                (&mut chunk as *mut *mut u8).cast(),
                0.0,
            ) as i32;
            if !chunk.is_null() && size > 0 {
                let bytes = unsafe { std::slice::from_raw_parts(chunk, size as usize) };
                *chunk_data = base64_encode(bytes);
            }
        } else {
            let num_params = unsafe { (*self.effect).num_params };
            for i in 0..num_params {
                let mut buffer = [0u8; 256];
                self.control(
                    VST_EFFECT_OPCODE_PARAM_GET_NAME,
                    i,
                    0,
                    buffer.as_mut_ptr().cast(),
                    0.0,
                );
                buffer[255] = 0;
                let value = unsafe {
                    (*self.effect)
                        .get_parameter
                        .map(|get_parameter| get_parameter(self.effect, i as u32))
                        .unwrap_or(0.0)
                };
                param_map.insert(utf8_to_wstring(&buffer), value);
            }
        }
    }

    pub fn start_processing(&self) {
        if self.effect.is_null() {
            return;
        }
        self.control(VST_EFFECT_OPCODE_SUSPEND, 0, 1, ptr::null_mut(), 0.0);
        self.control(
            VST_EFFECT_OPCODE_PROCESS_BEGIN,
            0,
            0,
            ptr::null_mut(),
            0.0,
        );
    }

    pub fn process_double_replacing(
        &mut self,
        input_array: &[*mut f64],
        output_array: &[*mut f64],
        frame_count: i32,
    ) {
        if self.effect.is_null() {
            return;
        }
        if let Some(process) = unsafe { (*self.effect).process_double } {
            unsafe {
                process(
                    self.effect,
                    input_array.as_ptr() as *const *const f64,
                    output_array.as_ptr() as *mut *mut f64,
                    frame_count,
                )
            };
        }
    }

    pub fn process_replacing(
        &mut self,
        input_array: &[*mut f32],
        output_array: &[*mut f32],
        frame_count: i32,
    ) {
        if self.effect.is_null() {
            return;
        }
        if let Some(process) = unsafe { (*self.effect).process_float } {
            unsafe {
                process(
                    self.effect,
                    input_array.as_ptr() as *const *const f32,
                    output_array.as_ptr() as *mut *mut f32,
                    frame_count,
                )
            };
        }
    }

    pub fn process(
        &mut self,
        input_array: &[*mut f32],
        output_array: &[*mut f32],
        frame_count: i32,
    ) {
        if self.effect.is_null() {
            return;
        }
        if let Some(process) = unsafe { (*self.effect).process } {
            unsafe {
                process(
                    self.effect,
                    input_array.as_ptr() as *const *const f32,
                    output_array.as_ptr() as *mut *mut f32,
                    frame_count,
                )
            };
        }
    }

    pub fn stop_processing(&self) {
        if self.effect.is_null() {
            return;
        }
        self.control(VST_EFFECT_OPCODE_PROCESS_END, 0, 0, ptr::null_mut(), 0.0);
        self.control(VST_EFFECT_OPCODE_SUSPEND, 0, 0, ptr::null_mut(), 0.0);
    }

    pub fn start_editing(&self, hwnd: HWND, width: &mut i16, height: &mut i16) {
        if self.effect.is_null() {
            return;
        }
        let mut rect: *mut VstRect = ptr::null_mut();
        self.control(
            VST_EFFECT_OPCODE_EDITOR_GET_RECT,
            0,
            0,
            (&mut rect as *mut *mut VstRect).cast(),
            0.0,
        );
        self.control(
            VST_EFFECT_OPCODE_EDITOR_OPEN,
            0,
            0,
            hwnd.0,
            0.0,
        );
        self.control(
            VST_EFFECT_OPCODE_EDITOR_GET_RECT,
            0,
            0,
            (&mut rect as *mut *mut VstRect).cast(),
            0.0,
        );

        if rect.is_null() {
            *width = 0;
            *height = 0;
        } else {
            *width = unsafe { (*rect).right - (*rect).left };
            *height = unsafe { (*rect).bottom - (*rect).top };
        }
    }

    pub fn do_idle(&self) {
        if self.effect.is_null() {
            return;
        }
        self.control(
            VST_EFFECT_OPCODE_EDITOR_KEEP_ALIVE,
            0,
            0,
            ptr::null_mut(),
            0.0,
        );
    }

    pub fn stop_editing(&self) {
        if self.effect.is_null() {
            return;
        }
        self.control(VST_EFFECT_OPCODE_EDITOR_CLOSE, 0, 0, ptr::null_mut(), 0.0);
    }

    pub fn set_automate_func<F>(&self, func: F)
    where
        F: Fn() + Send + Sync + 'static,
    {
        if let Ok(mut automate) = self.context.automate_func.lock() {
            *automate = Some(Arc::new(func));
        }
    }

    pub fn clear_automate_func(&self) {
        if let Ok(mut automate) = self.context.automate_func.lock() {
            *automate = None;
        }
    }

    pub fn on_automate(&self) {
        let func = self
            .context
            .automate_func
            .lock()
            .ok()
            .and_then(|automate| automate.clone());
        if let Some(func) = func {
            func();
        }
    }

    pub fn set_size_window_func<F>(&self, func: F)
    where
        F: Fn(i32, i32) + Send + Sync + 'static,
    {
        if let Ok(mut size_window) = self.context.size_window_func.lock() {
            *size_window = Some(Arc::new(func));
        }
    }

    pub fn clear_size_window_func(&self) {
        if let Ok(mut size_window) = self.context.size_window_func.lock() {
            *size_window = None;
        }
    }

    pub fn on_size_window(&self, width: i32, height: i32) {
        let func = self
            .context
            .size_window_func
            .lock()
            .ok()
            .and_then(|size_window| size_window.clone());
        if let Some(func) = func {
            func(width, height);
        }
    }

    fn control(
        &self,
        opcode: i32,
        index: i32,
        value: isize,
        ptr: *mut c_void,
        opt: f32,
    ) -> isize {
        if self.effect.is_null() {
            return 0;
        }
        unsafe { effect_control(self.effect, opcode, index, value, ptr, opt) }
    }
}

impl Drop for VstPluginInstance {
    fn drop(&mut self) {
        if !self.effect.is_null() {
            self.control(VST_EFFECT_OPCODE_DESTROY, 0, 0, ptr::null_mut(), 0.0);
            self.effect = ptr::null_mut();
        }
    }
}

unsafe fn effect_control(
    effect: *mut VstEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr: *mut c_void,
    opt: f32,
) -> isize {
    if effect.is_null() {
        return 0;
    }
    match (*effect).control {
        Some(control) => control(effect, opcode, index, value, ptr, opt),
        None => 0,
    }
}

unsafe extern "C" fn host_callback(
    effect: *mut VstEffect,
    opcode: i32,
    index: i32,
    value: isize,
    ptr: *const c_char,
    _opt: f32,
) -> isize {
    let context = if effect.is_null() || (*effect).host_internal.is_null() {
        None
    } else {
        Some(&*((*effect).host_internal as *const HostContext))
    };

    match opcode {
        VST_HOST_OPCODE_VST_VERSION => VST_VERSION_2_4_0_0,
        VST_HOST_OPCODE_CURRENT_EFFECT_ID => VST_EQUALIZER_APO_ID,
        VST_EFFECT_OPCODE_PRODUCT_NAME => {
            if !ptr.is_null() {
                let product = b"Equalizer APO\0";
                let destination = ptr as *mut u8;
                std::slice::from_raw_parts_mut(destination, 64).fill(0);
                std::ptr::copy_nonoverlapping(
                    product.as_ptr(),
                    destination,
                    product.len(),
                );
            }
            1
        }
        VST_EFFECT_OPCODE_VENDOR_VERSION => VST_VENDOR_VERSION,
        VST_HOST_OPCODE_PIN_CONNECTED => match context {
            Some(context) => {
                if index < context.used_channel_count.load(Ordering::Acquire) {
                    0
                } else {
                    1
                }
            }
            None => 0,
        },
        VST_HOST_OPCODE_IO_NEED_IDLE => {
            if effect.is_null() {
                0
            } else {
                effect_control(
                    effect,
                    VST_HOST_OPCODE_KEEPALIVE_OR_IDLE,
                    0,
                    0,
                    ptr::null_mut(),
                    0.0,
                )
            }
        }
        VST_HOST_OPCODE_EDITOR_UPDATE => {
            if effect.is_null() {
                0
            } else {
                effect_control(
                    effect,
                    VST_EFFECT_OPCODE_EDITOR_KEEP_ALIVE,
                    0,
                    0,
                    ptr::null_mut(),
                    0.0,
                )
            }
        }
        VST_HOST_OPCODE_GET_TIME => match context {
            Some(context) => {
                let time = &mut *context.time_info.get();
                time.sample_rate =
                    f32::from_bits(context.sample_rate_bits.load(Ordering::Acquire)) as f64;
                time as *mut VstTimeInfo as isize
            }
            None => 0,
        },
        VST_HOST_OPCODE_GET_SAMPLE_RATE => match context {
            Some(context) => {
                f32::from_bits(context.sample_rate_bits.load(Ordering::Acquire)) as isize
            }
            None => 0,
        },
        VST_HOST_OPCODE_GET_ACTIVE_THREAD => match context {
            Some(context) => context.process_level.load(Ordering::Acquire) as isize,
            None => 0,
        },
        VST_HOST_OPCODE_LANGUAGE => match context {
            Some(context) => context.language.load(Ordering::Acquire) as isize,
            None => 0,
        },
        VST_HOST_OPCODE_GET_REPLACE_OR_ACCUMULATE => 1,
        VST_HOST_OPCODE_EDITOR_RESIZE => match context {
            Some(context) => {
                let func = context
                    .size_window_func
                    .lock()
                    .ok()
                    .and_then(|size_window| size_window.clone());
                if let Some(func) = func {
                    func(index, value as i32);
                }
                0
            }
            None => 1,
        },
        VST_HOST_OPCODE_SUPPORTS => {
            if ptr.is_null() {
                0
            } else {
                let feature = CStr::from_ptr(ptr).to_bytes();
                if feature == b"startStopProcess" || feature == b"sizeWindow" {
                    1
                } else {
                    0
                }
            }
        }
        VST_HOST_OPCODE_AUTOMATE => {
            if let Some(context) = context {
                let func = context
                    .automate_func
                    .lock()
                    .ok()
                    .and_then(|automate| automate.clone());
                if let Some(func) = func {
                    func();
                }
            }
            0
        }
        VST_HOST_OPCODE_PARAM_START_EDIT
        | VST_HOST_OPCODE_PARAM_STOP_EDIT
        | VST_HOST_OPCODE_KEEPALIVE_OR_IDLE
        | VST_HOST_OPCODE_WANT_MIDI => 0,
        _ => 0,
    }
}

const BASE64_ALPHABET: &[u8; 64] =
    b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

pub(crate) fn base64_encode(input: &[u8]) -> String {
    let mut output = String::with_capacity((input.len() + 2) / 3 * 4);
    for chunk in input.chunks(3) {
        let first = chunk[0] as u32;
        let second = chunk.get(1).copied().unwrap_or(0) as u32;
        let third = chunk.get(2).copied().unwrap_or(0) as u32;
        let value = (first << 16) | (second << 8) | third;
        output.push(BASE64_ALPHABET[(value >> 18) as usize & 0x3F] as char);
        output.push(BASE64_ALPHABET[(value >> 12) as usize & 0x3F] as char);
        if chunk.len() > 1 {
            output.push(BASE64_ALPHABET[(value >> 6) as usize & 0x3F] as char);
        } else {
            output.push('=');
        }
        if chunk.len() > 2 {
            output.push(BASE64_ALPHABET[value as usize & 0x3F] as char);
        } else {
            output.push('=');
        }
    }
    output
}

pub(crate) fn base64_decode(input: &str) -> Option<Vec<u8>> {
    let mut output = Vec::with_capacity(input.len() / 4 * 3);
    let mut accumulator = 0u32;
    let mut bits = 0u32;
    for byte in input.bytes() {
        if byte.is_ascii_whitespace() {
            continue;
        }
        if byte == b'=' {
            break;
        }
        let value = match byte {
            b'A'..=b'Z' => byte - b'A',
            b'a'..=b'z' => byte - b'a' + 26,
            b'0'..=b'9' => byte - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32;
        accumulator = (accumulator << 6) | value;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            output.push((accumulator >> bits) as u8);
        }
    }
    Some(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_round_trip() {
        for input in [
            &b""[..],
            &b"a"[..],
            &b"ab"[..],
            &b"abc"[..],
            &[0u8, 1, 2, 3, 250, 251, 252, 253][..],
        ] {
            let encoded = base64_encode(input);
            assert_eq!(base64_decode(&encoded).unwrap(), input);
        }
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
        assert_eq!(base64_decode("aGVs bG8=").unwrap(), b"hello");
        assert!(base64_decode("!!!!").is_none());
    }
}

#[cfg(test)]
mod integration_tests {
    use super::*;
    use std::path::Path;

    /// Run after building testdata/vst-test-plugin.
    #[test]
    #[ignore]
    fn instance_edits_and_chunks_with_test_plugin() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("testdata")
            .join("vst-test-plugin")
            .join("target")
            .join("release")
            .join("TestVST.dll");
        let library = VSTPluginLibrary::get_instance(&path.to_string_lossy());
        assert!(
            library.initialize() >= 0,
            "test plugin library failed to load"
        );
        let mut instance = VstPluginInstance::new(library, 1);
        assert!(instance.initialize());
        assert_eq!(instance.num_inputs(), 2);
        assert_eq!(instance.num_outputs(), 2);
        assert!(instance.can_double_replacing());
        assert_eq!(instance.get_name(), "Rust Test VST");

        instance.write_to_effect("AAAAAAAA4D8=", &HashMap::new());
        let mut chunk = String::new();
        let mut params = HashMap::new();
        instance.read_from_effect(&mut chunk, &mut params);
        assert_eq!(chunk, "AAAAAAAA4D8=");

        let mut width: i16 = 0;
        let mut height: i16 = 0;
        instance.start_editing(HWND::default(), &mut width, &mut height);
        assert_eq!((width, height), (200, 100));
        instance.do_idle();
        instance.stop_editing();
    }
}
