//! Port of `AnalysisThread.cpp` using `eapo_engine::FilterEngine` and rustfft.
//!
//! The algorithm is unchanged: an impulse is pushed through the engine, the
//! first non-zero sample is located, the remaining response is transformed
//! with a real-input FFT and the maximum bin magnitude is reported as peak
//! gain.  Timing values are measured the same way as the C++ implementation.

use std::sync::mpsc::{self, Receiver, Sender};
use std::thread::{self, JoinHandle};
use std::time::Instant;

use eapo_engine::FilterEngine;
use rustfft::num_complex::Complex;
use rustfft::FftPlanner;

#[derive(Clone, Debug)]
pub struct DeviceSnapshot {
    pub capture: bool,
    pub device_name: String,
    pub connection_name: String,
    pub device_guid: String,
    pub device_string: String,
    pub channel_count: u32,
    pub sample_rate: u32,
    pub channel_mask: u32,
}

#[derive(Clone, Debug)]
pub struct AnalysisRequest {
    pub device: DeviceSnapshot,
    pub channel_mask: u32,
    pub channel_index: usize,
    pub config_path: String,
    pub frame_count: usize,
    pub generation: u64,
}

#[derive(Clone, Debug)]
pub struct AnalysisResult {
    pub freq_data: Vec<Complex<f64>>,
    pub freq_data_length: usize,
    pub freq_data_sample_rate: u32,
    pub peak_gain: f64,
    pub latency: i32,
    pub initialization_time: f64,
    pub processing_time: f64,
    pub processed_frames: u32,
    pub generation: u64,
}

impl AnalysisResult {
    pub fn nodes(&self) -> Vec<eapo_common::gain_iterator::FilterNode> {
        let mut nodes = Vec::with_capacity(self.freq_data_length / 2);
        for i in 0..self.freq_data_length / 2 {
            let mut freq = (i as f64 / self.freq_data_length as f64) * self.freq_data_sample_rate as f64;
            if freq == 0.0 {
                freq = 0.001;
            }
            let magnitude = (self.freq_data[i].re * self.freq_data[i].re
                + self.freq_data[i].im * self.freq_data[i].im)
                .sqrt();
            nodes.push(eapo_common::gain_iterator::FilterNode {
                freq,
                db_gain: magnitude.log10() * 20.0,
            });
        }
        nodes
    }

    pub fn cpu_usage(&self) -> f64 {
        if self.processed_frames == 0 || self.freq_data_sample_rate == 0 {
            return 0.0;
        }
        self.processing_time * 100.0
            / (self.processed_frames as f64 * 1000.0 / self.freq_data_sample_rate as f64)
    }
}

pub struct AnalysisThread {
    request_tx: Option<Sender<AnalysisRequest>>,
    result_rx: Receiver<AnalysisResult>,
    join: Option<JoinHandle<()>>,
}

impl AnalysisThread {
    pub fn new() -> Self {
        let (request_tx, request_rx) = mpsc::channel::<AnalysisRequest>();
        let (result_tx, result_rx) = mpsc::channel::<AnalysisResult>();
        let join = thread::spawn(move || analysis_main(request_rx, result_tx));
        Self { request_tx: Some(request_tx), result_rx, join: Some(join) }
    }

    pub fn set_parameters(&self, request: AnalysisRequest) {
        if let Some(tx) = &self.request_tx {
            let _ = tx.send(request);
        }
    }

    pub fn try_recv(&self) -> Option<AnalysisResult> {
        self.result_rx.try_recv().ok()
    }
}

impl Default for AnalysisThread {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for AnalysisThread {
    fn drop(&mut self) {
        self.request_tx.take();
        if let Some(t) = self.join.take() {
            let _ = t.join();
        }
    }
}

fn analysis_main(request_rx: Receiver<AnalysisRequest>, result_tx: Sender<AnalysisResult>) {
    while let Ok(mut request) = request_rx.recv() {
        // Supersede queued requests; the newest parameters win, like the C++
        // `frameCount = 0` handshake.
        while let Ok(newer) = request_rx.try_recv() {
            request = newer;
        }
        let result = run_analysis(&request);
        if result_tx.send(result).is_err() {
            break;
        }
    }
}

pub fn run_analysis(request: &AnalysisRequest) -> AnalysisResult {
    let device = &request.device;

    let mut channel_count = device.channel_count;
    if request.channel_mask != 0 && request.channel_mask != device.channel_mask {
        channel_count = (0..31).filter(|i| request.channel_mask & (1 << i) != 0).count() as u32;
    }
    let mut channel_mask = request.channel_mask;
    if channel_count == 0 {
        channel_count = 8;
        channel_mask = eapo_common::channel_helper::KSAUDIO_SPEAKER_7POINT1_SURROUND;
    }
    let sample_rate = if device.sample_rate == 0 { 48000 } else { device.sample_rate };
    let frame_count = request.frame_count.max(1);
    let channel_index = request.channel_index.min(channel_count.saturating_sub(1) as usize);

    let init_start = Instant::now();
    let mut engine = FilterEngine::new();
    engine.set_device_info(
        device.capture,
        true,
        &device.device_name,
        &device.connection_name,
        &device.device_guid,
        &device.device_string,
    );
    engine.initialize(
        sample_rate as f32,
        channel_count,
        channel_count,
        channel_count,
        channel_mask,
        frame_count as u32,
        &request.config_path,
    );
    let initialization_time = init_start.elapsed().as_secs_f64() * 1000.0;

    let sample_count = frame_count * channel_count as usize;
    let mut input = vec![0.0f64; sample_count];
    let mut output = vec![0.0f64; sample_count];
    // C++: for (i = 0; i < channelCount; i++) buf[i] = 1.0f;
    for i in 0..channel_count as usize {
        input[i] = 1.0;
    }

    let mut time_data = vec![0.0f64; frame_count];
    let mut freq_data = vec![Complex::new(0.0f64, 0.0); frame_count / 2 + 1];
    let mut latency: i32 = 0;
    let mut start_frame: i64 = -1;
    let mut processing_time = 0.0;
    let mut processed_frames: u32 = 0;

    // Stop searching for startFrame after 10 seconds of audio data.
    while processed_frames < 10 * sample_rate {
        let pstart = Instant::now();
        unsafe {
            engine.process_f64_interleaved(
                output.as_mut_ptr(),
                input.as_ptr(),
                frame_count as u32,
            );
        }
        processing_time += pstart.elapsed().as_secs_f64() * 1000.0;
        processed_frames += frame_count as u32;

        if start_frame != -1 {
            let sf = start_frame as usize;
            for i in 0..sf {
                time_data[frame_count - sf + i] = output[i * channel_count as usize + channel_index];
            }
            break;
        }

        for i in 0..frame_count {
            if output[i * channel_count as usize + channel_index].abs() > 1e-5 {
                start_frame = i as i64;
                break;
            }
        }

        if start_frame != -1 {
            let sf = start_frame as usize;
            for i in 0..frame_count - sf {
                time_data[i] = output[(sf + i) * channel_count as usize + channel_index];
            }
            if start_frame == 0 {
                break;
            }
        }

        if latency == 0 {
            input.iter_mut().for_each(|s| *s = 0.0);
        }

        if start_frame == -1 {
            latency += frame_count as i32;
        }
    }

    let peak_gain;
    if start_frame != -1 {
        latency += start_frame as i32;
        let mut buffer: Vec<Complex<f64>> =
            time_data.iter().map(|&v| Complex::new(v, 0.0)).collect();
        let mut planner = FftPlanner::<f64>::new();
        let fft = planner.plan_fft_forward(frame_count);
        fft.process(&mut buffer);

        let mut peak: f64 = f64::NEG_INFINITY;
        for c in buffer.iter().take(frame_count / 2) {
            let sqr = c.re * c.re + c.im * c.im;
            if sqr > peak {
                peak = sqr;
            }
        }
        freq_data = buffer[..frame_count / 2 + 1].to_vec();
        peak_gain = if peak > 0.0 { peak.sqrt().log10() * 20.0 } else { f64::NEG_INFINITY };
    } else {
        latency = 0;
        peak_gain = f64::NEG_INFINITY;
    }

    AnalysisResult {
        freq_data,
        freq_data_length: frame_count,
        freq_data_sample_rate: sample_rate,
        peak_gain,
        latency,
        initialization_time,
        processing_time,
        processed_frames,
        generation: request.generation,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cpu_usage_matches_cpp_formula() {
        let r = AnalysisResult {
            freq_data: vec![],
            freq_data_length: 65536,
            freq_data_sample_rate: 48000,
            peak_gain: 0.0,
            latency: 0,
            initialization_time: 0.0,
            processing_time: 1.0,
            processed_frames: 48000,
            generation: 0,
        };
        // processingTime * 100 / (processedFrames * 1000 / sampleRate)
        assert!((r.cpu_usage() - 0.1).abs() < 1e-9);
    }

    #[test]
    fn positive_preamp_is_reflected_in_peak_gain() {
        let dir = std::env::temp_dir().join(format!(
            "eapo_analysis_test_{}",
            std::process::id()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let config = dir.join("config.txt");
        std::fs::write(&config, "Preamp: 20 dB\n").unwrap();

        let request = AnalysisRequest {
            device: DeviceSnapshot {
                capture: false,
                device_name: "Test Device".into(),
                connection_name: "Test Connection".into(),
                device_guid: "{00000000-0000-0000-0000-000000000000}".into(),
                device_string: "Test Device (Test Connection)".into(),
                channel_count: 2,
                sample_rate: 48000,
                channel_mask: 3,
            },
            channel_mask: 3,
            channel_index: 0,
            config_path: config.to_string_lossy().into_owned(),
            frame_count: 65536,
            generation: 1,
        };
        let result = run_analysis(&request);
        assert!(
            (result.peak_gain - 20.0).abs() < 0.01,
            "peak gain was {}",
            result.peak_gain
        );
        let max_node = result
            .nodes()
            .iter()
            .map(|node| node.db_gain)
            .fold(f64::NEG_INFINITY, f64::max);
        assert!((max_node - 20.0).abs() < 0.01, "max node gain was {max_node}");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
