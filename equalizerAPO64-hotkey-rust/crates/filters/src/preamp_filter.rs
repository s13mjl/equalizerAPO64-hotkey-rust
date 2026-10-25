//! Port of `filters/PreampFilter.{h,cpp}`.

use std::sync::OnceLock;

use regex::Regex;

use crate::{channel, IFilter};

pub struct PreampFilter {
    db_gain: f64,
    gain: f64,
    channel_count: usize,
}

impl PreampFilter {
    pub fn new(db_gain: f64) -> Self {
        Self {
            db_gain,
            gain: 10f64.powf(db_gain / 20.0),
            channel_count: 0,
        }
    }

    pub fn from_parameters(parameters: &str) -> Option<Self> {
        static NUMBER: OnceLock<Regex> = OnceLock::new();
        let value = parameters.replace(',', ".");
        let re = NUMBER.get_or_init(|| {
            Regex::new(r"^\s*([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)").unwrap()
        });
        let captures = re.captures(&value)?;
        let db_gain: f64 = captures[1].parse().ok()?;
        Some(Self::new(db_gain))
    }

    pub fn get_db_gain(&self) -> f64 {
        self.db_gain
    }

    pub fn get_gain(&self) -> f64 {
        self.gain
    }
}

impl IFilter for PreampFilter {
    fn initialize(
        &mut self,
        _sample_rate: f32,
        _max_frame_count: usize,
        channel_names: Vec<String>,
    ) -> Vec<String> {
        self.channel_count = channel_names.len();
        channel_names
    }

    unsafe fn process(
        &mut self,
        output: &mut [*mut f64],
        input: &[*mut f64],
        frame_count: usize,
    ) {
        let gain = self.gain;
        for i in 0..self.channel_count {
            let input_channel = channel(input[i], frame_count);
            let output_channel = channel(output[i], frame_count);
            for j in 0..frame_count {
                output_channel[j] = input_channel[j] * gain;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn doubles_with_plus_six_db() {
        let mut filter = PreampFilter::new(20.0 * 2f64.log10());
        filter.initialize(48000.0, 2, vec!["L".into()]);
        let mut input = vec![0.5, -0.25];
        let mut output = vec![0.0; 2];
        unsafe { filter.process(&mut [output.as_mut_ptr()], &[input.as_mut_ptr()], 2) };
        assert!((output[0] - 1.0).abs() < 1e-12);
        assert!((output[1] + 0.5).abs() < 1e-12);
    }

    #[test]
    fn parses_comma_decimal_mark() {
        let filter = PreampFilter::from_parameters(" -6,5 dB").unwrap();
        assert!((filter.get_db_gain() + 6.5).abs() < 1e-12);
    }

    #[test]
    fn rejects_missing_number() {
        assert!(PreampFilter::from_parameters(" dB").is_none());
    }
}
