//! Port of `FilterConfiguration.{h,cpp}`.
//!
//! Channel routing and buffer swapping mirror the original pointer-based
//! implementation: channel buffers are `Box<[f64]>` so swapping two entries
//! exchanges the heap buffers exactly like the C++ `double*` swap.

use std::f64::consts::PI;

use eapo_filters::IFilter;

pub struct FilterInfo {
    pub filter: Box<dyn IFilter>,
    pub in_place: bool,
    /// Empty means the C++ `inChannelCount == 0` optimization: keep the
    /// pointer array from the previous filter.
    pub in_channels: Vec<usize>,
    pub out_channels: Vec<usize>,
}

pub struct FilterConfiguration {
    real_channel_count: usize,
    output_channel_count: usize,
    all_channel_count: usize,
    max_frame_count: usize,
    all_samples: Vec<Box<[f64]>>,
    all_samples2: Vec<Box<[f64]>>,
    current_samples: Vec<*mut f64>,
    current_samples2: Vec<*mut f64>,
    filter_infos: Vec<FilterInfo>,
}

// The raw pointers only reference buffers owned by this configuration and are
// never dereferenced across threads except by the single thread that owns the
// configuration at that moment (audio thread or loading thread before publish).
unsafe impl Send for FilterConfiguration {}

impl FilterConfiguration {
    pub fn new(
        real_channel_count: usize,
        output_channel_count: usize,
        all_channel_count: usize,
        max_frame_count: usize,
        filter_infos: Vec<FilterInfo>,
    ) -> Self {
        let mut all_samples = Vec::with_capacity(all_channel_count);
        let mut all_samples2 = Vec::with_capacity(all_channel_count);
        for _ in 0..all_channel_count {
            all_samples.push(vec![0.0f64; max_frame_count].into_boxed_slice());
            all_samples2.push(vec![0.0f64; max_frame_count].into_boxed_slice());
        }
        Self {
            real_channel_count,
            output_channel_count,
            all_channel_count,
            max_frame_count,
            all_samples,
            all_samples2,
            current_samples: vec![std::ptr::null_mut(); all_channel_count],
            current_samples2: vec![std::ptr::null_mut(); all_channel_count],
            filter_infos,
        }
    }

    pub fn is_empty(&self) -> bool {
        self.filter_infos.is_empty()
    }

    pub fn filter_count(&self) -> usize {
        self.filter_infos.len()
    }

    pub fn all_channel_count(&self) -> usize {
        self.all_channel_count
    }

    pub fn real_channel_count(&self) -> usize {
        self.real_channel_count
    }

    pub fn output_channel_count(&self) -> usize {
        self.output_channel_count
    }

    /// Deinterleaves `frame_count` frames from a `real_channel_count`-channel
    /// interleaved buffer.
    ///
    /// # Safety
    /// `input` must point to `frame_count * real_channel_count` doubles.
    pub unsafe fn read(&mut self, input: *const f64, frame_count: usize) {
        let frames = frame_count.min(self.max_frame_count);
        match self.real_channel_count {
            1 => {
                let src = std::slice::from_raw_parts(input, frames);
                self.all_samples[0][..frames].copy_from_slice(src);
            }
            2 => {
                let src = std::slice::from_raw_parts(input, frames * 2);
                let (left, right) = self.all_samples.split_at_mut(1);
                for i in 0..frames {
                    left[0][i] = src[i * 2];
                    right[0][i] = src[i * 2 + 1];
                }
            }
            6 => self.read_deinterleave::<6>(input, frames),
            8 => self.read_deinterleave::<8>(input, frames),
            _ => {
                let channels = self.real_channel_count;
                let src = std::slice::from_raw_parts(input, frames * channels);
                for c in 0..channels {
                    for i in 0..frames {
                        self.all_samples[c][i] = src[i * channels + c];
                    }
                }
            }
        }
    }

    unsafe fn read_deinterleave<const C: usize>(&mut self, input: *const f64, frames: usize) {
        let src = std::slice::from_raw_parts(input, frames * C);
        for c in 0..C {
            for i in 0..frames {
                self.all_samples[c][i] = src[i * C + c];
            }
        }
    }

    /// Copies non-interleaved input channels.
    ///
    /// # Safety
    /// Every pointer in `input` must be valid for `frame_count` doubles.
    pub unsafe fn read_channels(&mut self, input: &[*const f64], frame_count: usize) {
        let frames = frame_count.min(self.max_frame_count);
        for c in 0..self.real_channel_count {
            let src = std::slice::from_raw_parts(input[c], frames);
            self.all_samples[c][..frames].copy_from_slice(src);
        }
    }

    pub fn process(&mut self, frame_count: usize) {
        let frames = frame_count.min(self.max_frame_count);
        for c in self.real_channel_count..self.all_channel_count {
            self.all_samples[c][..frames].fill(0.0);
        }
        // Mono input upmixed to stereo as the Windows audio system would do.
        if self.real_channel_count == 1 && self.output_channel_count >= 2 {
            let (first, rest) = self.all_samples.split_at_mut(1);
            rest[0][..frames].copy_from_slice(&first[0][..frames]);
        }

        let Self {
            all_samples,
            all_samples2,
            current_samples,
            current_samples2,
            filter_infos,
            ..
        } = self;

        for info in filter_infos.iter_mut() {
            for (j, &index) in info.in_channels.iter().enumerate() {
                current_samples[j] = all_samples[index].as_mut_ptr();
            }
            if info.in_place {
                for (j, &index) in info.out_channels.iter().enumerate() {
                    current_samples2[j] = all_samples[index].as_mut_ptr();
                }
            } else {
                for (j, &index) in info.out_channels.iter().enumerate() {
                    current_samples2[j] = all_samples2[index].as_mut_ptr();
                }
            }

            unsafe {
                info.filter
                    .process(current_samples2, current_samples, frame_count);
            }

            if !info.in_place {
                for &index in info.out_channels.iter() {
                    std::mem::swap(&mut all_samples[index], &mut all_samples2[index]);
                }
                std::mem::swap(current_samples, current_samples2);
            }
        }
    }

    /// Crossfades this configuration towards `next_config`.
    ///
    /// Returns the updated transition counter.
    pub fn do_transition(
        &mut self,
        next_config: &mut FilterConfiguration,
        frame_count: usize,
        transition_counter: u32,
        transition_length: u32,
    ) -> u32 {
        let frames = frame_count.min(self.max_frame_count);
        let mut counter = transition_counter;
        for f in 0..frames {
            let factor = if transition_length == 0 || counter >= transition_length {
                1.0
            } else {
                0.5 * (1.0 - (counter as f64 * PI / transition_length as f64).cos())
            };
            for c in 0..self.output_channel_count {
                if c < next_config.all_samples.len() {
                    self.all_samples[c][f] = self.all_samples[c][f] * (1.0 - factor)
                        + next_config.all_samples[c][f] * factor;
                }
            }
            counter += 1;
        }
        counter
    }

    /// Interleaves the first `output_channel_count` channels.
    ///
    /// # Safety
    /// `output` must have room for `frame_count * output_channel_count` doubles.
    pub unsafe fn write(&self, output: *mut f64, frame_count: usize) {
        let frames = frame_count.min(self.max_frame_count);
        match self.output_channel_count {
            1 => {
                let dst = std::slice::from_raw_parts_mut(output, frames);
                dst.copy_from_slice(&self.all_samples[0][..frames]);
            }
            2 => {
                let dst = std::slice::from_raw_parts_mut(output, frames * 2);
                for i in 0..frames {
                    dst[i * 2] = self.all_samples[0][i];
                    dst[i * 2 + 1] = self.all_samples[1][i];
                }
            }
            6 => self.write_interleave::<6>(output, frames),
            8 => self.write_interleave::<8>(output, frames),
            _ => {
                let channels = self.output_channel_count;
                let dst = std::slice::from_raw_parts_mut(output, frames * channels);
                for c in 0..channels {
                    for i in 0..frames {
                        dst[i * channels + c] = self.all_samples[c][i];
                    }
                }
            }
        }
    }

    unsafe fn write_interleave<const C: usize>(&self, output: *mut f64, frames: usize) {
        let dst = std::slice::from_raw_parts_mut(output, frames * C);
        for c in 0..C {
            for i in 0..frames {
                dst[i * C + c] = self.all_samples[c][i];
            }
        }
    }

    /// Copies non-interleaved output channels.
    ///
    /// # Safety
    /// Every pointer in `output` must be valid for `frame_count` doubles.
    pub unsafe fn write_channels(&self, output: &mut [*mut f64], frame_count: usize) {
        let frames = frame_count.min(self.max_frame_count);
        for c in 0..self.output_channel_count {
            let dst = std::slice::from_raw_parts_mut(output[c], frames);
            dst.copy_from_slice(&self.all_samples[c][..frames]);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestFilter {
        in_channels: Vec<usize>,
        out_channels: Vec<usize>,
        in_place: bool,
        mode: TestMode,
    }

    enum TestMode {
        Ones,
        Double,
    }

    impl IFilter for TestFilter {
        fn get_in_place(&self) -> bool {
            self.in_place
        }

        fn initialize(
            &mut self,
            _sample_rate: f32,
            _max_frame_count: usize,
            channel_names: Vec<String>,
        ) -> Vec<String> {
            channel_names
        }

        unsafe fn process(
            &mut self,
            output: &mut [*mut f64],
            input: &[*mut f64],
            frame_count: usize,
        ) {
            match self.mode {
                TestMode::Ones => {
                    for slot in output.iter().take(self.out_channels.len()) {
                        let dst = std::slice::from_raw_parts_mut(*slot, frame_count);
                        dst.fill(1.0);
                    }
                }
                TestMode::Double => {
                    for i in 0..self.out_channels.len() {
                        let src = std::slice::from_raw_parts(input[i], frame_count);
                        let dst = std::slice::from_raw_parts_mut(output[i], frame_count);
                        for j in 0..frame_count {
                            dst[j] = src[j] * 2.0;
                        }
                    }
                }
            }
        }
    }

    fn info(filter: TestFilter) -> FilterInfo {
        FilterInfo {
            in_place: filter.in_place,
            in_channels: filter.in_channels.clone(),
            out_channels: filter.out_channels.clone(),
            filter: Box::new(filter),
        }
    }

    #[test]
    fn stereo_roundtrip_without_filters() {
        let mut config = FilterConfiguration::new(2, 2, 2, 8, Vec::new());
        let input = [1.0f64, 2.0, 3.0, 4.0];
        unsafe {
            config.read(input.as_ptr(), 2);
        }
        config.process(2);
        let mut output = [0.0f64; 4];
        unsafe {
            config.write(output.as_mut_ptr(), 2);
        }
        assert_eq!(output, [1.0, 2.0, 3.0, 4.0]);
    }

    #[test]
    fn mono_is_upmixed_to_stereo() {
        let mut config = FilterConfiguration::new(1, 2, 2, 8, Vec::new());
        let input = [0.5f64, 0.25];
        unsafe {
            config.read(input.as_ptr(), 2);
        }
        config.process(2);
        let mut output = [0.0f64; 4];
        unsafe {
            config.write(output.as_mut_ptr(), 2);
        }
        assert_eq!(output, [0.5, 0.5, 0.25, 0.25]);
    }

    #[test]
    fn routing_reads_selected_channel_and_writes_target() {
        let filter = TestFilter {
            in_channels: vec![1],
            out_channels: vec![0],
            in_place: false,
            mode: TestMode::Double,
        };
        let mut config = FilterConfiguration::new(2, 2, 2, 8, vec![info(filter)]);
        let input = [1.0f64, 2.0, 3.0, 4.0];
        unsafe {
            config.read(input.as_ptr(), 2);
        }
        config.process(2);
        let mut output = [0.0f64; 4];
        unsafe {
            config.write(output.as_mut_ptr(), 2);
        }
        assert_eq!(output, [4.0, 2.0, 8.0, 4.0]);
    }

    #[test]
    fn crossfade_matches_cosine_curve() {
        let mut current = FilterConfiguration::new(2, 2, 2, 8, Vec::new());
        let filter2 = TestFilter {
            in_channels: vec![0, 1],
            out_channels: vec![0, 1],
            in_place: true,
            mode: TestMode::Ones,
        };
        let mut next = FilterConfiguration::new(2, 2, 2, 8, vec![info(filter2)]);
        let input = [0.0f64; 8];
        unsafe {
            current.read(input.as_ptr(), 4);
            next.read(input.as_ptr(), 4);
        }
        current.process(4);
        next.process(4);
        let counter = current.do_transition(&mut next, 4, 0, 4);
        assert_eq!(counter, 4);
        let mut output = [0.0f64; 8];
        unsafe {
            current.write(output.as_mut_ptr(), 4);
        }
        let expected = [0.0, 0.5 * (1.0 - (1.0f64 * PI / 4.0).cos()), 0.5, 0.5 * (1.0 - (3.0f64 * PI / 4.0).cos())];
        for (i, exp) in expected.iter().enumerate() {
            assert!((output[i * 2] - exp).abs() < 1e-12, "frame {}: {} != {}", i, output[i * 2], exp);
        }
    }
}
