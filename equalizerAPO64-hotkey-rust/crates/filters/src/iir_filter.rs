//! Port of `filters/IIRFilter.{h,cpp}`.

use crate::{channel, IFilter};

pub struct IIRFilter {
    order: usize,
    b0: f64,
    a: Vec<f64>,
    b: Vec<f64>,
    channel_count: usize,
    x: Vec<f64>,
    y: Vec<f64>,
}

impl IIRFilter {
    pub fn new(coefficients: Vec<f64>) -> Self {
        let order = (coefficients.len() / 2).saturating_sub(1);
        let a0 = coefficients.get(order + 1).copied().unwrap_or(1.0);
        let mut a = vec![0.0; order];
        let mut b = vec![0.0; order];
        let b0 = coefficients.first().copied().unwrap_or(0.0) / a0;
        for i in 0..order {
            b[i] = coefficients.get(i + 1).copied().unwrap_or(0.0) / a0;
            a[i] = -coefficients.get(i + order + 2).copied().unwrap_or(0.0) / a0;
        }

        Self {
            order,
            b0,
            a,
            b,
            channel_count: 0,
            x: Vec::new(),
            y: Vec::new(),
        }
    }

    pub fn get_coefficients(&self) -> (f64, &[f64], &[f64]) {
        (self.b0, &self.b, &self.a)
    }
}

impl IFilter for IIRFilter {
    fn initialize(
        &mut self,
        _sample_rate: f32,
        _max_frame_count: usize,
        channel_names: Vec<String>,
    ) -> Vec<String> {
        self.channel_count = channel_names.len();
        self.x = vec![0.0; self.order * self.channel_count];
        self.y = vec![0.0; self.order * self.channel_count];
        channel_names
    }

    unsafe fn process(
        &mut self,
        output: &mut [*mut f64],
        input: &[*mut f64],
        frame_count: usize,
    ) {
        if self.order == 0 {
            for i in 0..self.channel_count {
                let input_channel = channel(input[i], frame_count);
                let output_channel = channel(output[i], frame_count);
                for j in 0..frame_count {
                    output_channel[j] = self.b0 * input_channel[j];
                }
            }
            return;
        }

        for i in 0..self.channel_count {
            let input_channel = channel(input[i], frame_count);
            let output_channel = channel(output[i], frame_count);
            let offset = i * self.order;
            let xo = &mut self.x[offset..offset + self.order];
            let yo = &mut self.y[offset..offset + self.order];

            for j in 0..frame_count {
                let sample = input_channel[j];
                let mut sum = self.b0 * sample;

                for k in (1..self.order).rev() {
                    sum += self.b[k] * xo[k];
                    xo[k] = xo[k - 1];
                }

                sum += self.b[0] * xo[0];

                for k in (1..self.order).rev() {
                    sum += self.a[k] * yo[k];
                    yo[k] = yo[k - 1];
                }

                sum += self.a[0] * yo[0];

                xo[0] = sample;
                yo[0] = sum;

                output_channel[j] = sum;
            }
        }

        for value in self.x.iter_mut().chain(self.y.iter_mut()) {
            if value.abs() < f64::MIN_POSITIVE {
                *value = 0.0;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn first_order_matches_difference_equation() {
        let mut filter = IIRFilter::new(vec![0.5, 0.0, 1.0, -0.5]);
        let names = vec!["L".to_string()];
        filter.initialize(48000.0, 4, names);
        let mut input = vec![1.0, 0.0, 0.0, 0.0];
        let mut output = vec![0.0; 4];
        unsafe { filter.process(&mut [output.as_mut_ptr()], &[input.as_mut_ptr()], 4) };
        let expected = [0.5, 0.25, 0.125, 0.0625];
        for (actual, expected) in output.iter().zip(expected) {
            assert!((actual - expected).abs() < 1e-15);
        }
    }

    #[test]
    fn channels_keep_independent_state() {
        let mut filter = IIRFilter::new(vec![1.0, 0.0, 1.0, 0.0]);
        filter.initialize(48000.0, 2, vec!["L".into(), "R".into()]);
        let mut left = vec![1.0, 2.0];
        let mut right = vec![3.0, 4.0];
        let mut left_out = vec![0.0; 2];
        let mut right_out = vec![0.0; 2];
        unsafe {
            filter.process(
                &mut [left_out.as_mut_ptr(), right_out.as_mut_ptr()],
                &[left.as_mut_ptr(), right.as_mut_ptr()],
                2,
            )
        };
        assert_eq!(left_out, vec![1.0, 2.0]);
        assert_eq!(right_out, vec![3.0, 4.0]);
    }
}
