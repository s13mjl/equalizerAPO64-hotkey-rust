//! Port of `filters/loudnessCorrection/LoudnessCorrectionFilter.{h,cpp}`.

pub mod parameter_archive;
pub mod volume_controller;

use std::f64::consts::{LN_2, PI};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use regex::Regex;
use windows::Win32::Foundation::S_OK;

use crate::biquad::{BiQuad, BiQuadType};
use crate::{channel, IFilter};

use self::parameter_archive::ParameterArchive;
use self::volume_controller::VolumeController;

#[derive(Clone, Copy, Debug)]
pub struct FilterParameters {
    pub state: bool,
    pub reference_level: f32,
    pub reference_offset: f32,
    pub attenuation: f32,
    initialized: bool,
}

impl Default for FilterParameters {
    fn default() -> Self {
        Self {
            state: false,
            reference_level: 0.0,
            reference_offset: 0.0,
            attenuation: 1.0,
            initialized: false,
        }
    }
}

impl FilterParameters {
    pub fn from_parameters(parameters: &str) -> Self {
        static STATE: OnceLock<Regex> = OnceLock::new();
        static REFERENCE_LEVEL: OnceLock<Regex> = OnceLock::new();
        static REFERENCE_OFFSET: OnceLock<Regex> = OnceLock::new();
        static ATTENUATION: OnceLock<Regex> = OnceLock::new();

        let archive = ParameterArchive::from_string(parameters);
        let mut output = Self::default();
        let mut error = 0;

        match archive.get::<bool>(
            STATE.get_or_init(|| Regex::new(r"\s*State\s+(0|1)").unwrap()),
        ) {
            Some(value) => output.state = value,
            None => error += 1,
        }
        match archive.get::<f32>(
            REFERENCE_LEVEL
                .get_or_init(|| Regex::new(r"\s*ReferenceLevel\s+([-+0-9]+)").unwrap()),
        ) {
            Some(value) => output.reference_level = value,
            None => error += 1,
        }
        match archive.get::<f32>(
            REFERENCE_OFFSET
                .get_or_init(|| Regex::new(r"\s*ReferenceOffset\s+([-+0-9]+)").unwrap()),
        ) {
            Some(value) => output.reference_offset = value,
            None => error += 1,
        }
        let attenuation = archive.get::<f32>(ATTENUATION.get_or_init(|| {
            Regex::new(r"\s*Attenuation\s+((1((\.|,)0+)?)|(0((\.|,)[0-9]+)?))").unwrap()
        }));
        output.attenuation = attenuation.unwrap_or(1.0);

        output.initialized = error == 0;
        output
    }

    pub fn is_initialized(&self) -> bool {
        self.initialized
    }

    pub fn serialize_string(&self) -> String {
        let mut archive = ParameterArchive::new();
        archive.add(self.state, "State");
        archive.add(self.reference_level, "ReferenceLevel");
        archive.add(self.reference_offset, "ReferenceOffset");
        archive.add(self.attenuation, "Attenuation");
        archive.serialized_parameters().to_string()
    }

    pub fn serialize(&self) -> Vec<u8> {
        let mut archive = ParameterArchive::new();
        archive.add(self.state, "State");
        archive.add(self.reference_level, "ReferenceLevel");
        archive.add(self.reference_offset, "ReferenceOffset");
        archive.add(self.attenuation, "Attenuation");
        archive.get_serialized_parameters()
    }
}

#[derive(Clone, Copy, Default)]
struct ShelfCoefficients {
    a_low_shelf: [f64; 4],
    a0_low_shelf: f64,
    a_high_shelf: [f64; 4],
    a0_high_shelf: f64,
}

struct SharedParameters {
    coefficients: Mutex<ShelfCoefficients>,
    neutral_updated: AtomicBool,
    parameter_changed: AtomicBool,
    att_factor_bits: AtomicU64,
    stop: AtomicBool,
}

pub struct LoudnessCorrectionFilter {
    parameters: FilterParameters,
    channel_count: usize,
    sample_rate: f32,
    att_factor: f64,
    low_shelf_biquads: Vec<BiQuad>,
    high_shelf_biquads: Vec<BiQuad>,
    neutral: bool,
    shared: Arc<SharedParameters>,
    update_thread: Option<JoinHandle<()>>,
}

impl LoudnessCorrectionFilter {
    pub fn new(mut parameters: FilterParameters) -> Self {
        if parameters.attenuation > 1.0 {
            parameters.attenuation = 1.0;
        }
        if parameters.attenuation < 0.0 {
            parameters.attenuation = 0.0;
        }

        Self {
            parameters,
            channel_count: 0,
            sample_rate: 0.0,
            att_factor: 1.0,
            low_shelf_biquads: Vec::new(),
            high_shelf_biquads: Vec::new(),
            neutral: true,
            shared: Arc::new(SharedParameters {
                coefficients: Mutex::new(ShelfCoefficients::default()),
                neutral_updated: AtomicBool::new(false),
                parameter_changed: AtomicBool::new(false),
                att_factor_bits: AtomicU64::new(1.0f64.to_bits()),
                stop: AtomicBool::new(false),
            }),
            update_thread: None,
        }
    }

    pub fn from_parameters(parameters: &str) -> Option<Self> {
        let filter_parameters = FilterParameters::from_parameters(parameters);
        if filter_parameters.is_initialized() {
            Some(Self::new(filter_parameters))
        } else {
            None
        }
    }

    pub fn parameters(&self) -> &FilterParameters {
        &self.parameters
    }

    fn get_l_shelf_parameter(
        &self,
        volume: f64,
        frequency: &mut f64,
        q: &mut f64,
        gain: &mut f64,
        pre_amp: &mut f64,
    ) {
        *frequency = 75.0;
        *q = 0.52;
        let vol_diff = (self.parameters.reference_level - self.parameters.reference_offset) as f64 - volume;
        if vol_diff > 0.0 {
            *gain = vol_diff * 0.55 / (1.0 - 0.55) * self.parameters.attenuation as f64;
            *pre_amp = -*gain;
        } else if vol_diff < 0.0 {
            *pre_amp = 0.0;
            *gain = vol_diff
                * 0.55
                * (vol_diff / 90.0).exp()
                * self.parameters.attenuation as f64;
        } else {
            *gain = 0.0;
        }
    }

    fn get_h_shelf_parameter(
        &self,
        volume: f64,
        frequency: &mut f64,
        q: &mut f64,
        gain: &mut f64,
    ) {
        *frequency = 10000.0;
        *q = 0.9;
        let vol_diff = (self.parameters.reference_level - self.parameters.reference_offset) as f64 - volume;
        if vol_diff > 0.0 {
            *gain = vol_diff
                * 0.225
                * (-vol_diff / 100.0).exp()
                * self.parameters.attenuation as f64;
        } else if vol_diff < 0.0 {
            *gain = vol_diff
                * 0.175
                * (vol_diff / 80.0).exp()
                * self.parameters.attenuation as f64;
        } else {
            *gain = 0.0;
        }
    }

    fn stop_update_thread(&mut self) {
        if let Some(handle) = self.update_thread.take() {
            self.shared.stop.store(true, Ordering::Release);
            let _ = handle.join();
        }
        self.shared.stop.store(false, Ordering::Release);
    }
}

impl IFilter for LoudnessCorrectionFilter {
    fn initialize(
        &mut self,
        sample_rate: f32,
        _max_frame_count: usize,
        channel_names: Vec<String>,
    ) -> Vec<String> {
        self.stop_update_thread();

        self.channel_count = channel_names.len();
        self.low_shelf_biquads = vec![BiQuad::default(); self.channel_count];
        self.high_shelf_biquads = vec![BiQuad::default(); self.channel_count];
        self.sample_rate = sample_rate;
        self.att_factor = 1.0;
        self.neutral = true;

        let mut freq_ls = 75.0;
        let mut q_ls = 1.0;
        let mut gain_ls = 0.0;
        let mut freq_hs = 10000.0;
        let mut q_hs = 1.0;
        let mut gain_hs = 0.0;

        let volume_controller = VolumeController::new();
        let mut volume = 0.0;
        if volume_controller.get_volume(&mut volume) == S_OK {
            let mut pre_amp = 0.0;
            self.get_l_shelf_parameter(volume, &mut freq_ls, &mut q_ls, &mut gain_ls, &mut pre_amp);
            self.att_factor = (pre_amp / 6.0 * LN_2).exp();
            self.get_h_shelf_parameter(
                volume + pre_amp,
                &mut freq_hs,
                &mut q_hs,
                &mut gain_hs,
            );
            self.neutral = gain_ls.abs().max(gain_hs.abs()) < 0.2;
        }

        for i in 0..self.channel_count {
            self.low_shelf_biquads[i] = BiQuad::new(
                BiQuadType::LowShelf,
                gain_ls,
                freq_ls,
                sample_rate as f64,
                q_ls,
                false,
            );
            self.high_shelf_biquads[i] = BiQuad::new(
                BiQuadType::HighShelf,
                gain_hs,
                freq_hs,
                sample_rate as f64,
                q_hs,
                false,
            );
        }

        let mut coefficients = ShelfCoefficients::default();
        if let Some(biquad) = self.low_shelf_biquads.first() {
            let (a, a0) = biquad.get_coefficients();
            coefficients.a_low_shelf = a;
            coefficients.a0_low_shelf = a0;
        }
        if let Some(biquad) = self.high_shelf_biquads.first() {
            let (a, a0) = biquad.get_coefficients();
            coefficients.a_high_shelf = a;
            coefficients.a0_high_shelf = a0;
        }
        if let Ok(mut shared) = self.shared.coefficients.lock() {
            *shared = coefficients;
        }
        self.shared
            .att_factor_bits
            .store(self.att_factor.to_bits(), Ordering::Release);
        self.shared
            .neutral_updated
            .store(self.neutral, Ordering::Release);
        self.shared
            .parameter_changed
            .store(false, Ordering::Release);

        let shared = Arc::clone(&self.shared);
        let parameters = self.parameters;
        let update_sample_rate = self.sample_rate;
        self.update_thread = thread::Builder::new()
            .name("loudness-parameter-update".to_string())
            .spawn(move || parameter_update_thread(shared, parameters, update_sample_rate))
            .ok();

        channel_names
    }

    unsafe fn process(
        &mut self,
        output: &mut [*mut f64],
        input: &[*mut f64],
        frame_count: usize,
    ) {
        if !self.parameters.state {
            for i in 0..self.channel_count {
                let input_channel = channel(input[i], frame_count);
                let output_channel = channel(output[i], frame_count);
                output_channel.copy_from_slice(input_channel);
            }
            return;
        }

        if self.shared.parameter_changed.load(Ordering::Acquire) {
            if let Ok(coefficients) = self.shared.coefficients.try_lock() {
                for i in 0..self.channel_count {
                    self.low_shelf_biquads[i]
                        .set_coefficients(&coefficients.a_low_shelf, coefficients.a0_low_shelf);
                    self.high_shelf_biquads[i]
                        .set_coefficients(&coefficients.a_high_shelf, coefficients.a0_high_shelf);
                }
                self.neutral = self.shared.neutral_updated.load(Ordering::Acquire);
                self.shared
                    .parameter_changed
                    .store(false, Ordering::Release);
            }
        }

        for i in 0..self.channel_count {
            let input_channel = channel(input[i], frame_count);
            let output_channel = channel(output[i], frame_count);
            let low = &mut self.low_shelf_biquads[i];
            let high = &mut self.high_shelf_biquads[i];
            for j in 0..frame_count {
                let mut temp = low.process(input_channel[j]);
                low.remove_denormals();
                temp *= f64::from_bits(self.shared.att_factor_bits.load(Ordering::Relaxed));
                let mut result = high.process(temp);
                high.remove_denormals();
                if self.neutral {
                    result = input_channel[j];
                }
                output_channel[j] = result;
            }
        }
    }
}

impl Drop for LoudnessCorrectionFilter {
    fn drop(&mut self) {
        self.stop_update_thread();
    }
}

fn parameter_update_thread(
    shared: Arc<SharedParameters>,
    parameters: FilterParameters,
    sample_rate: f32,
) {
    let volume_controller = VolumeController::new();
    let mut volume_old = parameters.reference_level as f64;
    let mut volume = volume_old;

    while !shared.stop.load(Ordering::Acquire) {
        if !shared.parameter_changed.load(Ordering::Acquire) {
            let mut current_volume = volume;
            if volume_controller.get_volume(&mut current_volume) == S_OK {
                volume = current_volume;
                if volume != volume_old {
                    let vol_diff =
                        (parameters.reference_level - parameters.reference_offset) as f64 - volume;

                    let freq_ls = 75.0;
                    let q_ls = 0.52;
                    let mut gain_ls = 0.0;
                    let mut pre_amp = 0.0;
                    if vol_diff > 0.0 {
                        gain_ls =
                            vol_diff * 0.55 / (1.0 - 0.55) * parameters.attenuation as f64;
                        pre_amp = -gain_ls;
                    } else if vol_diff < 0.0 {
                        gain_ls = vol_diff
                            * 0.55
                            * (vol_diff / 90.0).exp()
                            * parameters.attenuation as f64;
                    }

                    let freq_hs = 10000.0;
                    let q_hs = 0.9;
                    let mut gain_hs = 0.0;
                    let hs_volume = volume + pre_amp;
                    let hs_vol_diff =
                        (parameters.reference_level - parameters.reference_offset) as f64
                            - hs_volume;
                    if hs_vol_diff > 0.0 {
                        gain_hs = hs_vol_diff
                            * 0.225
                            * (-hs_vol_diff / 100.0).exp()
                            * parameters.attenuation as f64;
                    } else if hs_vol_diff < 0.0 {
                        gain_hs = hs_vol_diff
                            * 0.175
                            * (hs_vol_diff / 80.0).exp()
                            * parameters.attenuation as f64;
                    }

                    let (a_hs, a0_hs) =
                        shelf_coefficients(freq_hs, q_hs, gain_hs, true, sample_rate);
                    let (a_ls, a0_ls) =
                        shelf_coefficients(freq_ls, q_ls, gain_ls, false, sample_rate);
                    if let Ok(mut coefficients) = shared.coefficients.lock() {
                        *coefficients = ShelfCoefficients {
                            a_low_shelf: a_ls,
                            a0_low_shelf: a0_ls,
                            a_high_shelf: a_hs,
                            a0_high_shelf: a0_hs,
                        };
                    }
                    let att_factor = (pre_amp / 6.0 * LN_2).exp();
                    shared
                        .att_factor_bits
                        .store(att_factor.to_bits(), Ordering::Release);
                    shared
                        .neutral_updated
                        .store(gain_ls.abs().max(gain_hs.abs()) < 0.2, Ordering::Release);
                    shared.parameter_changed.store(true, Ordering::Release);
                    volume_old = volume;
                }
            }
        }
        thread::sleep(Duration::from_millis(10));
    }
}

fn shelf_coefficients(
    freq: f64,
    bandwidth_or_q_or_s: f64,
    db_gain: f64,
    high_shelf: bool,
    sample_rate: f32,
) -> ([f64; 4], f64) {
    let a = 10f64.powf(db_gain / 40.0);
    let omega = 2.0 * PI * freq / sample_rate as f64;
    let sn = omega.sin();
    let cs = omega.cos();
    let alpha = sn / 2.0
        * ((a + 1.0 / a) * (1.0 / bandwidth_or_q_or_s - 1.0) + 2.0).sqrt();
    let beta = 2.0 * a.sqrt() * alpha;

    let mut coefficients = [0.0; 4];
    let a0;
    let a0_field;
    if high_shelf {
        a0 = (a + 1.0) - (a - 1.0) * cs + beta;
        a0_field = a * ((a + 1.0) + (a - 1.0) * cs + beta) / a0;
        coefficients[0] = -2.0 * a * ((a - 1.0) + (a + 1.0) * cs) / a0;
        coefficients[1] = a * ((a + 1.0) + (a - 1.0) * cs - beta) / a0;
        coefficients[2] = 2.0 * ((a - 1.0) - (a + 1.0) * cs) / a0;
        coefficients[3] = ((a + 1.0) - (a - 1.0) * cs - beta) / a0;
    } else {
        a0 = (a + 1.0) + (a - 1.0) * cs + beta;
        a0_field = a * ((a + 1.0) - (a - 1.0) * cs + beta) / a0;
        coefficients[0] = 2.0 * a * ((a - 1.0) - (a + 1.0) * cs) / a0;
        coefficients[1] = a * ((a + 1.0) - (a - 1.0) * cs - beta) / a0;
        coefficients[2] = -2.0 * ((a - 1.0) + (a + 1.0) * cs) / a0;
        coefficients[3] = ((a + 1.0) + (a - 1.0) * cs - beta) / a0;
    }
    (coefficients, a0_field)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_all_parameters() {
        let parameters =
            FilterParameters::from_parameters("State 1 ReferenceLevel -20 ReferenceOffset 5 Attenuation 0.5");
        assert!(parameters.is_initialized());
        assert!(parameters.state);
        assert_eq!(parameters.reference_level, -20.0);
        assert_eq!(parameters.reference_offset, 5.0);
        assert_eq!(parameters.attenuation, 0.5);
    }

    #[test]
    fn attenuation_defaults_when_missing_or_out_of_range() {
        let parameters = FilterParameters::from_parameters("State 0 ReferenceLevel 0 ReferenceOffset 0");
        assert!(parameters.is_initialized());
        assert_eq!(parameters.attenuation, 1.0);

        let parameters = FilterParameters::from_parameters(
            "State 0 ReferenceLevel 0 ReferenceOffset 0 Attenuation 1.5",
        );
        assert!(parameters.is_initialized());
        assert_eq!(parameters.attenuation, 1.0);
    }

    #[test]
    fn missing_required_parameter_is_not_initialized() {
        assert!(!FilterParameters::from_parameters("ReferenceLevel 0 ReferenceOffset 0")
            .is_initialized());
        assert!(LoudnessCorrectionFilter::from_parameters("State 1").is_none());
    }

    #[test]
    fn serialization_round_trips() {
        let parameters = FilterParameters::from_parameters(
            "State 1 ReferenceLevel -20 ReferenceOffset 5 Attenuation 0.5",
        );
        let serialized = parameters.serialize_string();
        let reparsed = FilterParameters::from_parameters(&serialized);
        assert!(reparsed.is_initialized());
        assert_eq!(reparsed.state, parameters.state);
        assert_eq!(reparsed.reference_level, parameters.reference_level);
        assert_eq!(reparsed.reference_offset, parameters.reference_offset);
        assert_eq!(reparsed.attenuation, parameters.attenuation);
        assert!(parameters.serialize().len() > serialized.len());
    }

    #[test]
    fn shelf_coefficients_match_biquad() {
        let (coefficients, a0) = shelf_coefficients(100.0, 0.7, 5.0, false, 48000.0);
        let biquad = BiQuad::new(BiQuadType::LowShelf, 5.0, 100.0, 48000.0, 0.7, true);
        let (expected, expected_a0) = biquad.get_coefficients();
        assert_eq!(a0, expected_a0);
        assert_eq!(coefficients, expected);
    }
}
