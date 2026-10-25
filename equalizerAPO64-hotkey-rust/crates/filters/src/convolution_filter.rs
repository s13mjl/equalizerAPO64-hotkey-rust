//! Port of `filters/ConvolutionFilter.{h,cpp}` and the impulse response
//! loading behavior of libsndfile for WAV files.

use std::sync::Arc;

use eapo_common::gain_iterator::{FilterNode, GainIterator};
use eapo_common::{error_log, trace_log};
use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

use crate::audio_file::read_audio_file;
use crate::hybrid_conv::HConvSingle;
use crate::{channel, IFilter};

enum IrSource {
    File(String),
    GraphicEq {
        nodes: Vec<FilterNode>,
        filter_length: usize,
    },
}

pub struct ConvolutionFilter {
    filters: Vec<HConvSingle>,
    sample_rate: f32,
    channel_count: usize,
    max_frame_count: usize,
    before_first_process: bool,
    filename: String,
    source: IrSource,
}

impl ConvolutionFilter {
    pub fn new(filename: impl Into<String>) -> Self {
        let filename = filename.into();
        Self {
            filters: Vec::new(),
            sample_rate: 0.0,
            channel_count: 0,
            max_frame_count: 0,
            before_first_process: true,
            filename: filename.clone(),
            source: IrSource::File(filename),
        }
    }

    pub(crate) fn new_graphic_eq(nodes: Vec<FilterNode>, filter_length: usize) -> Self {
        Self {
            filters: Vec::new(),
            sample_rate: 0.0,
            channel_count: 0,
            max_frame_count: 0,
            before_first_process: true,
            filename: String::new(),
            source: IrSource::GraphicEq {
                nodes,
                filter_length,
            },
        }
    }

    pub fn filename(&self) -> &str {
        &self.filename
    }

    pub(crate) fn graphic_eq_nodes(&self) -> &[FilterNode] {
        match &self.source {
            IrSource::GraphicEq { nodes, .. } => nodes,
            IrSource::File(_) => &[],
        }
    }

    fn cleanup(&mut self) {
        self.filters.clear();
    }

    fn initialize_filters(&mut self, frame_count: usize) {
        if self.channel_count == 0 || frame_count == 0 {
            return;
        }

        let mut impulse_channels: Vec<Vec<f64>> = Vec::new();
        match &self.source {
            IrSource::File(filename) => match read_audio_file(filename) {
                Ok(audio) => {
                    if (self.sample_rate as f64 - audio.sample_rate as f64).abs() > 1.0 {
                        error_log!(
                            "Impulse response sample rate ({} Hz) does not match device sample rate ({:.6} Hz)",
                            audio.sample_rate,
                            self.sample_rate
                        );
                        return;
                    }
                    if audio.channels.is_empty() || audio.channels[0].is_empty() {
                        error_log!("Impulse response file {} contains no audio frames", filename);
                        return;
                    }
                    trace_log!("Convolving using impulse response file {}", filename);
                    impulse_channels = audio.channels;
                }
                Err(message) => {
                    error_log!("Error while reading impulse response file: {}", message);
                    return;
                }
            },
            IrSource::GraphicEq {
                nodes,
                filter_length,
            } => {
                if *filter_length == 0 {
                    error_log!("Graphic equalizer filter length must not be zero");
                    return;
                }
                impulse_channels.push(build_graphic_eq_impulse(
                    nodes,
                    *filter_length,
                    self.sample_rate,
                ));
            }
        }

        let file_channel_count = impulse_channels.len();
        for i in 0..self.channel_count {
            let impulse = &impulse_channels[i % file_channel_count];
            self.filters
                .push(HConvSingle::new(impulse, frame_count, 1));
        }
    }
}

impl IFilter for ConvolutionFilter {
    fn initialize(
        &mut self,
        sample_rate: f32,
        max_frame_count: usize,
        channel_names: Vec<String>,
    ) -> Vec<String> {
        self.cleanup();

        self.sample_rate = sample_rate;
        self.max_frame_count = max_frame_count;
        self.channel_count = channel_names.len();
        self.before_first_process = true;

        self.initialize_filters(max_frame_count);

        channel_names
    }

    unsafe fn process(
        &mut self,
        output: &mut [*mut f64],
        input: &[*mut f64],
        frame_count: usize,
    ) {
        if self.filters.is_empty() {
            return;
        }

        if self.before_first_process && frame_count != self.max_frame_count {
            self.cleanup();
            self.initialize_filters(frame_count);
            if self.filters.len() != self.channel_count {
                return;
            }
        }

        self.before_first_process = false;

        for i in 0..self.channel_count {
            let input_channel = channel(input[i], frame_count);
            let output_channel = channel(output[i], frame_count);
            let filter = &mut self.filters[i];
            filter.put(input_channel);
            filter.process();
            filter.get(output_channel);
        }
    }
}

fn build_graphic_eq_impulse(
    nodes: &[FilterNode],
    filter_length: usize,
    sample_rate: f32,
) -> Vec<f64> {
    let dft_length = filter_length * 2;
    let mut planner = FftPlanner::<f64>::new();
    let forward: Arc<dyn Fft<f64>> = planner.plan_fft_forward(dft_length);
    let reverse: Arc<dyn Fft<f64>> = planner.plan_fft_inverse(dft_length);
    let mut time_data = vec![Complex::ZERO; dft_length];
    let mut freq_data = vec![Complex::ZERO; dft_length];

    let mut gain_iterator = GainIterator::new(nodes.to_vec());
    for i in 0..filter_length {
        let freq = i as f64 * sample_rate as f64 / dft_length as f64;
        let db_gain = gain_iterator.gain_at(freq);
        let gain = 10f64.powf(db_gain / 20.0);
        freq_data[i] = Complex::new(gain, 0.0);
        freq_data[dft_length - i - 1] = Complex::new(gain, 0.0);
    }

    let threshold = 10f64.powf(-100.0 / 20.0);
    let log_threshold = threshold.ln();
    for value in freq_data.iter_mut() {
        value.re = if value.re < threshold {
            log_threshold
        } else {
            value.re.ln()
        };
        value.im = 0.0;
    }

    reverse.process(&mut freq_data);
    let scale = 1.0 / dft_length as f64;
    for value in time_data.iter_mut() {
        *value *= scale;
    }

    for i in 1..filter_length {
        time_data[i].re += time_data[dft_length - i].re;
        time_data[i].im -= time_data[dft_length - i].im;
        time_data[dft_length - i] = Complex::ZERO;
    }
    time_data[filter_length].im = -time_data[filter_length].im;

    forward.process(&mut time_data);
    for value in time_data.iter_mut() {
        let magnitude = value.re.exp();
        *value = Complex::new(magnitude * value.im.cos(), magnitude * value.im.sin());
    }

    reverse.process(&mut time_data);
    for value in time_data.iter_mut() {
        *value *= scale;
    }
    for i in 0..filter_length {
        let factor = 0.5 * (1.0 + (2.0 * std::f64::consts::PI * i as f64 / dft_length as f64).cos());
        time_data[i].re *= factor;
        time_data[i].im *= factor;
    }

    time_data[..filter_length]
        .iter()
        .map(|value| value.re)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{SampleFormat, WavWriter};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn load_wav_test_spec(
        channels: u16,
        sample_rate: u32,
        bits_per_sample: u16,
        sample_format: SampleFormat,
    ) -> hound::WavSpec {
        hound::WavSpec {
            channels,
            sample_rate,
            bits_per_sample,
            sample_format,
        }
    }

    fn temp_wav_path() -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("eapo_conv_{}_{}.wav", std::process::id(), nanos))
    }

    #[test]
    fn delta_impulse_equals_loaded_impulse_response() {
        let path = temp_wav_path();
        let spec = load_wav_test_spec(1, 48000, 32, SampleFormat::Float);
        {
            let mut writer = WavWriter::create(&path, spec).unwrap();
            for sample in [0.5f32, -0.25, 0.125] {
                writer.write_sample(sample).unwrap();
            }
            writer.finalize().unwrap();
        }

        let mut filter = ConvolutionFilter::new(path.to_string_lossy().into_owned());
        filter.initialize(48000.0, 8, vec!["L".into(), "R".into()]);
        let mut left = vec![0.0; 8];
        let mut right = vec![0.0; 8];
        let mut input_left = vec![0.0; 8];
        let mut input_right = vec![0.0; 8];
        input_left[0] = 1.0;
        input_right[0] = 1.0;
        let mut output = [left.as_mut_ptr(), right.as_mut_ptr()];
        let input = [input_left.as_mut_ptr(), input_right.as_mut_ptr()];
        unsafe { filter.process(&mut output, &input, 8) };

        for channel in [&left, &right] {
            assert!((channel[0] - 0.5).abs() < 1e-12);
            assert!((channel[1] + 0.25).abs() < 1e-12);
            assert!((channel[2] - 0.125).abs() < 1e-12);
            assert!(channel[3..].iter().all(|sample| sample.abs() < 1e-12));
        }

        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn pcm16_loading_normalizes_samples() {
        let path = temp_wav_path();
        let spec = load_wav_test_spec(1, 8000, 16, SampleFormat::Int);
        {
            let mut writer = WavWriter::create(&path, spec).unwrap();
            writer.write_sample(16384i16).unwrap();
            writer.write_sample(-8192i16).unwrap();
            writer.finalize().unwrap();
        }
        let loaded = read_audio_file(&path).unwrap();
        assert_eq!(loaded.sample_rate, 8000);
        assert_eq!(loaded.channels[0], vec![0.5, -0.25]);
        let _ = std::fs::remove_file(path);
    }
}
