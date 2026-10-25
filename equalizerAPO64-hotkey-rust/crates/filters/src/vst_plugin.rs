//! Port of `filters/VSTPluginFilter.{h,cpp}`.

use std::collections::HashMap;
use std::panic::{self, AssertUnwindSafe};
use std::ptr;
use std::sync::Arc;

use eapo_common::error_log;

use crate::vst_plugin_instance::VstPluginInstance;
use crate::vst_plugin_library::VSTPluginLibrary;
use crate::{channel, IFilter};

pub struct VSTPluginFilter {
    library: Arc<VSTPluginLibrary>,
    lib_path: String,
    chunk_data: String,
    param_map: HashMap<String, f32>,
    channel_count: usize,
    effect_channel_count: usize,
    effect_count: usize,
    effects: Vec<VstPluginInstance>,
    empty_channels: Vec<Vec<f64>>,
    input_array: Vec<*mut f64>,
    output_array: Vec<*mut f64>,
    float_inputs: Vec<Vec<f32>>,
    float_outputs: Vec<Vec<f32>>,
    float_input_ptrs: Vec<*mut f32>,
    float_output_ptrs: Vec<*mut f32>,
    delay_buffer_length: usize,
    delay_buffers: Vec<Vec<f64>>,
    delay_buffer_offset: usize,
    delay_scratch: Vec<f64>,
    skip_processing: bool,
    report_crash: bool,
    max_frame_count: usize,
}

unsafe impl Send for VSTPluginFilter {}

impl VSTPluginFilter {
    pub fn new(
        library: Arc<VSTPluginLibrary>,
        chunk_data: String,
        param_map: HashMap<String, f32>,
    ) -> Self {
        let lib_path = library.lib_path().to_string();
        Self {
            library,
            lib_path,
            chunk_data,
            param_map,
            channel_count: 0,
            effect_channel_count: 0,
            effect_count: 0,
            effects: Vec::new(),
            empty_channels: Vec::new(),
            input_array: Vec::new(),
            output_array: Vec::new(),
            float_inputs: Vec::new(),
            float_outputs: Vec::new(),
            float_input_ptrs: Vec::new(),
            float_output_ptrs: Vec::new(),
            delay_buffer_length: 0,
            delay_buffers: Vec::new(),
            delay_buffer_offset: 0,
            delay_scratch: Vec::new(),
            skip_processing: false,
            report_crash: true,
            max_frame_count: 0,
        }
    }

    pub fn get_library(&self) -> Arc<VSTPluginLibrary> {
        Arc::clone(&self.library)
    }

    pub fn get_chunk_data(&self) -> String {
        self.chunk_data.clone()
    }

    pub fn get_param_map(&self) -> HashMap<String, f32> {
        self.param_map.clone()
    }

    pub fn prepare_for_processing(&mut self, sample_rate: f32, max_frame_count: usize) {
        self.max_frame_count = max_frame_count;

        for i in 0..self.effect_count {
            let used_channel_count = if i == self.effect_count - 1
                && self.channel_count % self.effect_channel_count != 0
            {
                self.channel_count % self.effect_channel_count
            } else {
                self.effect_channel_count
            };
            let effect = &mut self.effects[i];
            effect.set_used_channel_count(used_channel_count as i32);
            effect.prepare_for_processing(sample_rate, max_frame_count as i32);
            effect.write_to_effect(&self.chunk_data, &self.param_map);
            effect.start_processing();
        }
    }

    fn cleanup(&mut self) {
        for effect in &mut self.effects {
            effect.stop_processing();
        }
        self.effects.clear();
        self.effect_count = 0;
        self.empty_channels.clear();
        self.input_array.clear();
        self.output_array.clear();
        self.float_inputs.clear();
        self.float_outputs.clear();
        self.float_input_ptrs.clear();
        self.float_output_ptrs.clear();
        self.delay_buffers.clear();
        self.delay_buffer_length = 0;
        self.delay_buffer_offset = 0;
        self.delay_scratch.clear();
    }

    unsafe fn copy_input_to_output(
        &self,
        output: &mut [*mut f64],
        input: &[*mut f64],
        frame_count: usize,
    ) {
        for i in 0..self.channel_count {
            let input_channel = channel(input[i], frame_count);
            let output_channel = channel(output[i], frame_count);
            output_channel.copy_from_slice(input_channel);
        }
    }

    unsafe fn process_inner(
        &mut self,
        output: &mut [*mut f64],
        input: &[*mut f64],
        frame_count: usize,
    ) {
        if self.skip_processing {
            self.copy_input_to_output(output, input, frame_count);
            return;
        }

        let mut channel_offset = 0usize;
        let mut empty_channel_index = 0usize;

        for i in 0..self.effect_count {
            let num_inputs = self.effects[i].num_inputs().max(0) as usize;
            let num_outputs = self.effects[i].num_outputs().max(0) as usize;
            let can_double_replacing = self.effects[i].can_double_replacing();
            let can_replacing = self.effects[i].can_replacing();

            for j in 0..num_inputs.min(self.input_array.len()) {
                self.input_array[j] = if channel_offset + j < self.channel_count {
                    input[channel_offset + j]
                } else {
                    let pointer = self
                        .empty_channels
                        .get(empty_channel_index)
                        .map_or(ptr::null_mut(), |channel| channel.as_ptr() as *mut f64);
                    empty_channel_index += 1;
                    pointer
                };
            }
            for j in 0..num_outputs.min(self.output_array.len()) {
                self.output_array[j] = if channel_offset + j < self.channel_count {
                    output[channel_offset + j]
                } else {
                    let pointer = self
                        .empty_channels
                        .get(empty_channel_index)
                        .map_or(ptr::null_mut(), |channel| channel.as_ptr() as *mut f64);
                    empty_channel_index += 1;
                    pointer
                };
            }

            if can_double_replacing {
                self.effects[i].process_double_replacing(
                    &self.input_array[..num_inputs.min(self.input_array.len())],
                    &self.output_array[..num_outputs.min(self.output_array.len())],
                    frame_count as i32,
                );
            } else {
                for j in 0..num_inputs.min(self.float_inputs.len()) {
                    let source =
                        std::slice::from_raw_parts(self.input_array[j] as *const f64, frame_count);
                    let destination = &mut self.float_inputs[j][..frame_count];
                    for sample in 0..frame_count {
                        destination[sample] = source[sample] as f32;
                    }
                }

                if can_replacing {
                    self.effects[i].process_replacing(
                        &self.float_input_ptrs[..num_inputs.min(self.float_input_ptrs.len())],
                        &self.float_output_ptrs[..num_outputs.min(self.float_output_ptrs.len())],
                        frame_count as i32,
                    );
                } else {
                    for j in 0..num_outputs.min(self.float_outputs.len()) {
                        self.float_outputs[j][..frame_count].fill(0.0);
                    }
                    self.effects[i].process(
                        &self.float_input_ptrs[..num_inputs.min(self.float_input_ptrs.len())],
                        &self.float_output_ptrs[..num_outputs.min(self.float_output_ptrs.len())],
                        frame_count as i32,
                    );
                }

                for j in 0..num_outputs.min(self.float_outputs.len()) {
                    let source = &self.float_outputs[j][..frame_count];
                    let destination =
                        std::slice::from_raw_parts_mut(self.output_array[j], frame_count);
                    for sample in 0..frame_count {
                        destination[sample] = source[sample] as f64;
                    }
                }
            }

            if num_outputs < num_inputs {
                for j in num_outputs..num_inputs {
                    if channel_offset + j < self.channel_count {
                        let output_channel = channel(output[channel_offset + j], frame_count);
                        output_channel.fill(0.0);
                    }
                }
            }

            channel_offset += self.effect_channel_count;
        }

        if self.delay_buffer_length > 0 {
            for i in 0..self.channel_count {
                let output_channel = channel(output[i], frame_count);
                let delay_buffer = &mut self.delay_buffers[i];
                let delay_length = self.delay_buffer_length;
                let offset = self.delay_buffer_offset;

                if delay_length <= frame_count {
                    self.delay_scratch[..frame_count - delay_length]
                        .copy_from_slice(&output_channel[..frame_count - delay_length]);
                    output_channel[..delay_length - offset]
                        .copy_from_slice(&delay_buffer[offset..delay_length]);
                    output_channel[delay_length - offset..delay_length]
                        .copy_from_slice(&delay_buffer[..offset]);
                    output_channel[delay_length..]
                        .copy_from_slice(&self.delay_scratch[..frame_count - delay_length]);
                    delay_buffer.copy_from_slice(&output_channel[frame_count - delay_length..]);
                } else {
                    self.delay_scratch[..frame_count].copy_from_slice(&output_channel[..frame_count]);
                    if delay_length < offset + frame_count {
                        output_channel[..delay_length - offset]
                            .copy_from_slice(&delay_buffer[offset..delay_length]);
                        output_channel[delay_length - offset..]
                            .copy_from_slice(&delay_buffer[..frame_count - (delay_length - offset)]);
                        delay_buffer[offset..]
                            .copy_from_slice(&self.delay_scratch[..delay_length - offset]);
                        delay_buffer[..frame_count - (delay_length - offset)].copy_from_slice(
                            &self.delay_scratch[delay_length - offset..frame_count],
                        );
                    } else {
                        output_channel[..frame_count]
                            .copy_from_slice(&delay_buffer[offset..offset + frame_count]);
                        delay_buffer[offset..offset + frame_count]
                            .copy_from_slice(&self.delay_scratch[..frame_count]);
                    }
                }
            }

            if self.delay_buffer_length <= frame_count {
                self.delay_buffer_offset = 0;
            } else {
                self.delay_buffer_offset =
                    (self.delay_buffer_offset + frame_count) % self.delay_buffer_length;
            }
        }
    }
}

impl IFilter for VSTPluginFilter {
    fn get_in_place(&self) -> bool {
        false
    }

    fn initialize(
        &mut self,
        sample_rate: f32,
        max_frame_count: usize,
        channel_names: Vec<String>,
    ) -> Vec<String> {
        self.cleanup();

        self.channel_count = channel_names.len();
        if self.channel_count == 0 {
            return channel_names;
        }

        self.skip_processing = false;
        self.report_crash = true;

        let mut first_effect = VstPluginInstance::new(Arc::clone(&self.library), 2);
        if !first_effect.initialize() {
            error_log!(
                "The VST plugin {} crashed during initialization.",
                self.lib_path
            );
            self.skip_processing = true;
        }

        self.effect_channel_count =
            first_effect.num_inputs().max(first_effect.num_outputs()).max(0) as usize;
        if self.effect_channel_count == 0 {
            self.skip_processing = true;
            self.effect_count = 0;
            return channel_names;
        }
        self.effect_count = (self.channel_count + self.effect_channel_count - 1)
            / self.effect_channel_count;
        self.effects.push(first_effect);
        for _ in 1..self.effect_count {
            let mut effect = VstPluginInstance::new(Arc::clone(&self.library), 2);
            if !effect.initialize() && !self.skip_processing {
                error_log!(
                    "The VST plugin {} crashed during initialization.",
                    self.lib_path
                );
                self.skip_processing = true;
            }
            self.effects.push(effect);
        }

        self.prepare_for_processing(sample_rate, max_frame_count);

        let empty_channel_count = 2 * (self.effect_count * self.effect_channel_count
            - self.channel_count);
        self.empty_channels = vec![vec![0.0; max_frame_count]; empty_channel_count];

        let max_inputs = self
            .effects
            .iter()
            .map(|effect| effect.num_inputs().max(0) as usize)
            .max()
            .unwrap_or(0);
        let max_outputs = self
            .effects
            .iter()
            .map(|effect| effect.num_outputs().max(0) as usize)
            .max()
            .unwrap_or(0);
        self.input_array = vec![ptr::null_mut(); max_inputs];
        self.output_array = vec![ptr::null_mut(); max_outputs];

        self.float_inputs = vec![vec![0.0; max_frame_count]; max_inputs];
        self.float_outputs = vec![vec![0.0; max_frame_count]; max_outputs];
        self.float_input_ptrs = self
            .float_inputs
            .iter_mut()
            .map(|channel| channel.as_mut_ptr())
            .collect();
        self.float_output_ptrs = self
            .float_outputs
            .iter_mut()
            .map(|channel| channel.as_mut_ptr())
            .collect();

        self.delay_buffer_length = if let Some(effect) = self.effects.first() {
            effect.get_initial_delay().max(0) as usize
        } else {
            0
        };
        if self.delay_buffer_length > 0 {
            self.delay_buffers = vec![vec![0.0; self.delay_buffer_length]; self.channel_count];
            self.delay_buffer_offset = 0;
        }
        self.delay_scratch = vec![0.0; max_frame_count];

        channel_names
    }

    unsafe fn process(
        &mut self,
        output: &mut [*mut f64],
        input: &[*mut f64],
        frame_count: usize,
    ) {
        let result =
            panic::catch_unwind(AssertUnwindSafe(|| self.process_inner(output, input, frame_count)));
        if result.is_err() {
            if self.report_crash {
                error_log!(
                    "The VST plugin {} crashed during audio processing.",
                    self.lib_path
                );
                self.report_crash = false;
            }
            self.copy_input_to_output(output, input, frame_count);
        }
    }
}

impl Drop for VSTPluginFilter {
    fn drop(&mut self) {
        self.cleanup();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    fn test_plugin_path() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("..")
            .join("testdata")
            .join("vst-test-plugin")
            .join("target")
            .join("release")
            .join("TestVST.dll")
    }

    /// Run after building testdata/vst-test-plugin:
    /// `cargo test -p eapo-filters test_vst_plugin_end_to_end -- --ignored`
    #[test]
    #[ignore]
    fn test_vst_plugin_end_to_end() {
        let path = test_plugin_path();
        assert!(path.exists(), "build the test plugin first: {}", path.display());
        let library = VSTPluginLibrary::get_instance(&path.to_string_lossy());
        assert!(
            library.initialize() >= 0,
            "test plugin library failed to load"
        );

        // Chunk contains a little-endian f64 gain of 0.5.
        let chunk = "AAAAAAAA4D8=".to_string();
        let mut filter = VSTPluginFilter::new(library, chunk, HashMap::new());
        assert_eq!(
            filter.initialize(48000.0, 64, vec!["L".into(), "R".into()]),
            vec!["L".to_string(), "R".to_string()]
        );

        let mut left = vec![1.0f64; 8];
        let mut right = vec![-2.0f64; 8];
        let mut out_left = vec![0.0f64; 8];
        let mut out_right = vec![0.0f64; 8];
        unsafe {
            filter.process(
                &mut [out_left.as_mut_ptr(), out_right.as_mut_ptr()],
                &[left.as_mut_ptr(), right.as_mut_ptr()],
                8,
            );
        }
        assert!(out_left.iter().all(|sample| (*sample - 0.5).abs() < 1e-12));
        assert!(out_right.iter().all(|sample| (*sample + 1.0).abs() < 1e-12));
        let _ = (&mut left, &mut right);
    }
}
