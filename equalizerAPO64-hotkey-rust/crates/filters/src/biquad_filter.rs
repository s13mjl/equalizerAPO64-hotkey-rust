//! Port of `filters/BiQuadFilter.{h,cpp}`.

use crate::biquad::{BiQuad, BiQuadType};
use crate::{channel, IFilter};

pub struct BiQuadFilter {
    type_: BiQuadType,
    db_gain: f64,
    freq: f64,
    bandwidth_or_q_or_s: f64,
    is_bandwidth_or_s: bool,
    is_corner_freq: bool,
    channel_count: usize,
    a0: f64,
    b1: f64,
    b2: f64,
    a1: f64,
    a2: f64,
    x1: Vec<f64>,
    x2: Vec<f64>,
    y1: Vec<f64>,
    y2: Vec<f64>,
}

impl BiQuadFilter {
    pub fn new(
        type_: BiQuadType,
        db_gain: f64,
        freq: f64,
        bandwidth_or_q_or_s: f64,
        is_bandwidth_or_s: bool,
        is_corner_freq: bool,
    ) -> Self {
        Self {
            type_,
            db_gain,
            freq,
            bandwidth_or_q_or_s,
            is_bandwidth_or_s,
            is_corner_freq,
            channel_count: 0,
            a0: 0.0,
            b1: 0.0,
            b2: 0.0,
            a1: 0.0,
            a2: 0.0,
            x1: Vec::new(),
            x2: Vec::new(),
            y1: Vec::new(),
            y2: Vec::new(),
        }
    }

    pub fn get_type(&self) -> BiQuadType {
        self.type_
    }

    pub fn get_db_gain(&self) -> f64 {
        self.db_gain
    }

    pub fn get_freq(&self) -> f64 {
        self.freq
    }

    pub fn get_bandwidth_or_q_or_s(&self) -> f64 {
        self.bandwidth_or_q_or_s
    }

    pub fn get_is_bandwidth_or_s(&self) -> bool {
        self.is_bandwidth_or_s
    }

    pub fn get_is_corner_freq(&self) -> bool {
        self.is_corner_freq
    }
}

impl IFilter for BiQuadFilter {
    fn initialize(
        &mut self,
        sample_rate: f32,
        _max_frame_count: usize,
        channel_names: Vec<String>,
    ) -> Vec<String> {
        self.channel_count = channel_names.len();

        let mut biquad_freq = self.freq;
        if self.is_corner_freq
            && matches!(self.type_, BiQuadType::LowShelf | BiQuadType::HighShelf)
        {
            let mut s = self.bandwidth_or_q_or_s;
            if !self.is_bandwidth_or_s {
                let q = self.bandwidth_or_q_or_s;
                let a = 10f64.powf(self.db_gain / 40.0);
                s = 1.0 / ((1.0 / (q * q) - 2.0) / (a + 1.0 / a) + 1.0);
            }
            let center_freq_factor = 10f64.powf(self.db_gain.abs() / 80.0 / s);
            if matches!(self.type_, BiQuadType::LowShelf) {
                biquad_freq *= center_freq_factor;
            } else {
                biquad_freq /= center_freq_factor;
            }
        }

        let master = BiQuad::new(
            self.type_,
            self.db_gain,
            biquad_freq,
            sample_rate as f64,
            self.bandwidth_or_q_or_s,
            self.is_bandwidth_or_s,
        );
        let (coeffs, a0) = master.get_coefficients();
        self.a0 = a0;
        self.b1 = coeffs[0];
        self.b2 = coeffs[1];
        self.a1 = coeffs[2];
        self.a2 = coeffs[3];

        self.x1 = vec![0.0; self.channel_count];
        self.x2 = vec![0.0; self.channel_count];
        self.y1 = vec![0.0; self.channel_count];
        self.y2 = vec![0.0; self.channel_count];

        channel_names
    }

    unsafe fn process(
        &mut self,
        output: &mut [*mut f64],
        input: &[*mut f64],
        frame_count: usize,
    ) {
        for i in 0..self.channel_count {
            let input_channel = channel(input[i], frame_count);
            let output_channel = channel(output[i], frame_count);
            let mut x1 = self.x1[i];
            let mut x2 = self.x2[i];
            let mut y1 = self.y1[i];
            let mut y2 = self.y2[i];

            for j in 0..frame_count {
                let sample = input_channel[j];
                let result = self.a0 * sample + self.b1 * x1 + self.b2 * x2
                    - self.a1 * y1
                    - self.a2 * y2;
                x2 = x1;
                x1 = sample;
                y2 = y1;
                y1 = result;
                output_channel[j] = result;
            }

            self.x1[i] = x1;
            self.x2[i] = x2;
            self.y1[i] = y1;
            self.y2[i] = y2;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn initialize_keeps_channel_names_and_zeroes_state() {
        let mut filter = BiQuadFilter::new(BiQuadType::Peaking, 6.0, 1000.0, 1.0, false, false);
        let names = vec!["L".to_string(), "R".to_string()];
        assert_eq!(filter.initialize(48000.0, 8, names.clone()), names);

        let mut left_in = vec![1.0, 0.0, 0.0, 0.0];
        let mut right_in = vec![1.0, 0.0, 0.0, 0.0];
        let mut left_out = vec![0.0; 4];
        let mut right_out = vec![0.0; 4];
        let mut output = [left_out.as_mut_ptr(), right_out.as_mut_ptr()];
        let input = [left_in.as_mut_ptr(), right_in.as_mut_ptr()];
        unsafe { filter.process(&mut output, &input, 4) };
        assert_eq!(left_out, right_out);
        assert_ne!(left_out[0], 0.0);
    }

    #[test]
    fn corner_frequency_does_not_panic_at_zero_gain() {
        let mut filter = BiQuadFilter::new(BiQuadType::LowShelf, 0.0, 100.0, 0.9, true, true);
        let names = vec!["L".to_string()];
        filter.initialize(48000.0, 4, names);
        let mut input = vec![1.0, 0.0, 0.0, 0.0];
        let mut output = vec![0.0; 4];
        unsafe { filter.process(&mut [output.as_mut_ptr()], &[input.as_mut_ptr()], 4) };
        assert!(output.iter().all(|sample| sample.is_finite()));
    }

    #[test]
    fn bandwidth_filter_is_stable() {
        let mut filter = BiQuadFilter::new(BiQuadType::LowPass, 0.0, 1000.0, 1.0, true, false);
        let names = vec!["L".to_string()];
        filter.initialize(48000.0, 4, names);
        let mut input = vec![1.0, 0.0, 0.0, 0.0];
        let mut output = vec![0.0; 4];
        unsafe { filter.process(&mut [output.as_mut_ptr()], &[input.as_mut_ptr()], 4) };
        assert!(output.iter().all(|sample| sample.is_finite()));
    }
}
