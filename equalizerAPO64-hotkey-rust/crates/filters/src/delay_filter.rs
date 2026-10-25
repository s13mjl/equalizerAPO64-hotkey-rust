//! Port of `filters/DelayFilter.{h,cpp}`.

use std::sync::OnceLock;

use regex::Regex;

use crate::{channel, IFilter};

pub struct DelayFilter {
    delay: f64,
    is_ms: bool,
    buffer_length: usize,
    channel_count: usize,
    buffers: Vec<Vec<f64>>,
    buffer_offset: usize,
}

impl DelayFilter {
    pub fn new(delay: f64, is_ms: bool) -> Self {
        Self {
            delay,
            is_ms,
            buffer_length: 0,
            channel_count: 0,
            buffers: Vec::new(),
            buffer_offset: 0,
        }
    }

    pub fn from_parameters(parameters: &str) -> Option<Self> {
        static DELAY: OnceLock<Regex> = OnceLock::new();
        let value = parameters.replace(',', ".");
        let re = DELAY.get_or_init(|| {
            Regex::new(
                r"^\s*([-+]?(?:\d+(?:\.\d*)?|\.\d+)(?:[eE][-+]?\d+)?)\s*(\S*)",
            )
            .unwrap()
        });
        let captures = re.captures(&value)?;
        let delay: f64 = captures[1].parse().ok()?;
        if delay < 0.0 {
            return None;
        }
        let unit = captures.get(2).map(|m| m.as_str()).unwrap_or("").to_lowercase();
        match unit.as_str() {
            "ms" => Some(Self::new(delay, true)),
            "samples" => Some(Self::new(delay, false)),
            _ => None,
        }
    }

    pub fn get_delay(&self) -> f64 {
        self.delay
    }

    pub fn get_is_ms(&self) -> bool {
        self.is_ms
    }
}

impl IFilter for DelayFilter {
    fn get_in_place(&self) -> bool {
        false
    }

    fn initialize(
        &mut self,
        sample_rate: f32,
        _max_frame_count: usize,
        channel_names: Vec<String>,
    ) -> Vec<String> {
        self.channel_count = channel_names.len();
        self.buffer_length = if self.is_ms {
            (sample_rate as f64 * self.delay / 1000.0 + 0.5) as usize
        } else {
            (self.delay + 0.5) as usize
        };
        self.buffers = vec![vec![0.0; self.buffer_length]; self.channel_count];
        self.buffer_offset = 0;

        channel_names
    }

    unsafe fn process(
        &mut self,
        output: &mut [*mut f64],
        input: &[*mut f64],
        frame_count: usize,
    ) {
        let buffer_length = self.buffer_length;
        let offset = self.buffer_offset;

        for i in 0..self.channel_count {
            let input_channel = channel(input[i], frame_count);
            let output_channel = channel(output[i], frame_count);
            let buffer_channel = &mut self.buffers[i];

            if buffer_length <= frame_count {
                output_channel[..buffer_length - offset]
                    .copy_from_slice(&buffer_channel[offset..buffer_length]);
                output_channel[buffer_length - offset..buffer_length]
                    .copy_from_slice(&buffer_channel[..offset]);
                output_channel[buffer_length..]
                    .copy_from_slice(&input_channel[..frame_count - buffer_length]);
                buffer_channel.copy_from_slice(&input_channel[frame_count - buffer_length..]);
            } else if buffer_length < offset + frame_count {
                output_channel[..buffer_length - offset]
                    .copy_from_slice(&buffer_channel[offset..buffer_length]);
                output_channel[buffer_length - offset..]
                    .copy_from_slice(&buffer_channel[..frame_count - (buffer_length - offset)]);
                buffer_channel[offset..]
                    .copy_from_slice(&input_channel[..buffer_length - offset]);
                buffer_channel[..frame_count - (buffer_length - offset)]
                    .copy_from_slice(&input_channel[buffer_length - offset..]);
            } else {
                output_channel[..frame_count]
                    .copy_from_slice(&buffer_channel[offset..offset + frame_count]);
                buffer_channel[offset..offset + frame_count]
                    .copy_from_slice(&input_channel[..frame_count]);
            }
        }

        if buffer_length <= frame_count {
            self.buffer_offset = 0;
        } else {
            self.buffer_offset = (offset + frame_count) % buffer_length;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run(filter: &mut DelayFilter, input: &[f64]) -> Vec<f64> {
        let mut input = input.to_vec();
        let mut output = vec![0.0; input.len()];
        unsafe {
            filter.process(
                &mut [output.as_mut_ptr()],
                &[input.as_mut_ptr()],
                input.len(),
            )
        };
        output
    }

    #[test]
    fn delays_by_samples() {
        let mut filter = DelayFilter::new(2.0, false);
        filter.initialize(48000.0, 4, vec!["L".into()]);
        assert_eq!(run(&mut filter, &[1.0, 2.0, 3.0, 4.0]), vec![0.0, 0.0, 1.0, 2.0]);
        assert_eq!(run(&mut filter, &[5.0, 6.0, 7.0, 8.0]), vec![3.0, 4.0, 5.0, 6.0]);
    }

    #[test]
    fn zero_delay_is_passthrough() {
        let mut filter = DelayFilter::new(0.0, false);
        filter.initialize(48000.0, 4, vec!["L".into()]);
        assert_eq!(run(&mut filter, &[1.0, 2.0, 3.0]), vec![1.0, 2.0, 3.0]);
    }

    #[test]
    fn long_delay_wraps_buffer() {
        let mut filter = DelayFilter::new(5.0, false);
        filter.initialize(48000.0, 3, vec!["L".into()]);
        assert_eq!(run(&mut filter, &[1.0, 2.0, 3.0]), vec![0.0, 0.0, 0.0]);
        assert_eq!(run(&mut filter, &[4.0, 5.0, 6.0]), vec![0.0, 0.0, 1.0]);
    }

    #[test]
    fn parses_ms_and_samples() {
        let filter = DelayFilter::from_parameters(" 12.5 ms").unwrap();
        assert!(filter.get_is_ms());
        assert_eq!(filter.get_delay(), 12.5);
        let filter = DelayFilter::from_parameters("3 Samples").unwrap();
        assert!(!filter.get_is_ms());
        assert!(DelayFilter::from_parameters("-1 ms").is_none());
        assert!(DelayFilter::from_parameters("1 second").is_none());
    }
}
