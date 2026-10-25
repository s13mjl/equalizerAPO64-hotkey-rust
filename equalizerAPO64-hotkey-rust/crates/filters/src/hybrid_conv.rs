//! Port of the single-segment partitioned convolution from `libHybridConv`.

use std::sync::Arc;

use rustfft::num_complex::Complex;
use rustfft::{Fft, FftPlanner};

pub struct HConvSingle {
    step: usize,
    maxstep: usize,
    mixpos: usize,
    framelength: usize,
    steptask: Vec<usize>,
    spectrum: Vec<Complex<f64>>,
    scratch: Vec<Complex<f64>>,
    in_freq_real: Vec<f64>,
    in_freq_imag: Vec<f64>,
    filterbuf_freq_real: Vec<Vec<f64>>,
    filterbuf_freq_imag: Vec<Vec<f64>>,
    mixbuf_freq_real: Vec<Vec<f64>>,
    mixbuf_freq_imag: Vec<Vec<f64>>,
    history_time: Vec<f64>,
    fft: Arc<dyn Fft<f64>>,
    ifft: Arc<dyn Fft<f64>>,
}

impl HConvSingle {
    pub fn new(h: &[f64], flen: usize, steps: usize) -> Self {
        assert!(flen > 0);
        let steps = steps.max(1);
        let dft_len = 2 * flen;
        let mut planner = FftPlanner::<f64>::new();
        let fft = planner.plan_fft_forward(dft_len);
        let ifft = planner.plan_fft_inverse(dft_len);
        let scratch_len = fft
            .get_inplace_scratch_len()
            .max(ifft.get_inplace_scratch_len());

        let num_filterbuf = (h.len() + flen - 1) / flen;
        let mut steptask = vec![0usize; steps + 1];
        let num = num_filterbuf / steps;
        for (i, task) in steptask.iter_mut().enumerate() {
            *task = i * num;
        }
        let pos = if steptask[1] == 0 { 1 } else { 2 };
        let num = num_filterbuf % steps;
        for j in pos..pos + num {
            for task in steptask.iter_mut().skip(j) {
                *task += 1;
            }
        }

        let num_mixbuf = num_filterbuf + 1;
        let mut filter = Self {
            step: 0,
            maxstep: steps,
            mixpos: 0,
            framelength: flen,
            steptask,
            spectrum: vec![Complex::ZERO; dft_len],
            scratch: vec![Complex::ZERO; scratch_len],
            in_freq_real: vec![0.0; flen + 1],
            in_freq_imag: vec![0.0; flen + 1],
            filterbuf_freq_real: vec![vec![0.0; flen + 1]; num_filterbuf],
            filterbuf_freq_imag: vec![vec![0.0; flen + 1]; num_filterbuf],
            mixbuf_freq_real: vec![vec![0.0; flen + 1]; num_mixbuf],
            mixbuf_freq_imag: vec![vec![0.0; flen + 1]; num_mixbuf],
            history_time: vec![0.0; flen],
            fft,
            ifft,
        };

        let gain = 0.5 / flen as f64;
        for i in 0..num_filterbuf {
            let start = i * flen;
            let segment_len = (h.len() - start).min(flen);
            filter.spectrum.fill(Complex::ZERO);
            for j in 0..segment_len {
                filter.spectrum[j] = Complex::new(gain * h[start + j], 0.0);
            }
            let (spectrum, scratch) = (&mut filter.spectrum, &mut filter.scratch);
            filter.fft.process_with_scratch(spectrum, scratch);
            for j in 0..=flen {
                filter.filterbuf_freq_real[i][j] = filter.spectrum[j].re;
                filter.filterbuf_freq_imag[i][j] = filter.spectrum[j].im;
            }
        }

        filter
    }

    #[allow(dead_code)]
    pub fn framelength(&self) -> usize {
        self.framelength
    }

    pub fn put(&mut self, x: &[f64]) {
        let flen = self.framelength;
        for i in 0..self.spectrum.len() {
            self.spectrum[i] = if i < flen && i < x.len() {
                Complex::new(x[i], 0.0)
            } else {
                // A short final block is zero-padded; the original C++ read
                // past the input buffer in this case.
                Complex::ZERO
            };
        }
        self.fft.process_with_scratch(&mut self.spectrum, &mut self.scratch);
        for j in 0..=flen {
            self.in_freq_real[j] = self.spectrum[j].re;
            self.in_freq_imag[j] = self.spectrum[j].im;
        }
    }

    pub fn process(&mut self) {
        let flen = self.framelength;
        let start = self.steptask[self.step];
        let stop = self.steptask[self.step + 1];
        for s in start..stop {
            let mix_idx = (s + self.mixpos) % self.mixbuf_freq_real.len();
            let y_real = &mut self.mixbuf_freq_real[mix_idx];
            let y_imag = &mut self.mixbuf_freq_imag[mix_idx];
            let h_real = &self.filterbuf_freq_real[s];
            let h_imag = &self.filterbuf_freq_imag[s];

            for n in 0..=flen {
                let xr = self.in_freq_real[n];
                let xi = self.in_freq_imag[n];
                y_real[n] += xr * h_real[n] - xi * h_imag[n];
                y_imag[n] += xr * h_imag[n] + xi * h_real[n];
            }
        }
        self.step = (self.step + 1) % self.maxstep;
    }

    pub fn get(&mut self, y: &mut [f64]) {
        self.get_internal(y, false);
    }

    #[allow(dead_code)]
    pub fn get_add(&mut self, y: &mut [f64]) {
        self.get_internal(y, true);
    }

    fn get_internal(&mut self, y: &mut [f64], add: bool) {
        let flen = self.framelength;
        let mpos = self.mixpos;
        for j in 0..=flen {
            self.spectrum[j] = Complex::new(
                self.mixbuf_freq_real[mpos][j],
                self.mixbuf_freq_imag[mpos][j],
            );
            self.mixbuf_freq_real[mpos][j] = 0.0;
            self.mixbuf_freq_imag[mpos][j] = 0.0;
        }
        self.spectrum[0].im = 0.0;
        self.spectrum[flen].im = 0.0;
        for j in 1..flen {
            self.spectrum[2 * flen - j] = self.spectrum[j].conj();
        }

        self.ifft.process_with_scratch(&mut self.spectrum, &mut self.scratch);

        for n in 0..flen.min(y.len()) {
            let value = self.spectrum[n].re + self.history_time[n];
            if add {
                y[n] += value;
            } else {
                y[n] = value;
            }
        }
        for n in 0..flen {
            self.history_time[n] = self.spectrum[flen + n].re;
        }
        self.mixpos = (mpos + 1) % self.mixbuf_freq_real.len();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn delta_impulse_reproduces_filter() {
        let h = [0.5, -0.25, 0.125];
        let mut filter = HConvSingle::new(&h, 4, 1);
        let mut output = [0.0; 4];
        filter.put(&[1.0, 0.0, 0.0, 0.0]);
        filter.process();
        filter.get(&mut output);
        for (actual, expected) in output.iter().zip([0.5, -0.25, 0.125, 0.0]) {
            assert!((actual - expected).abs() < 1e-14);
        }

        filter.put(&[0.0; 4]);
        filter.process();
        filter.get(&mut output);
        assert!(output.iter().all(|sample| sample.abs() < 1e-14));
    }

    #[test]
    fn partitioned_tail_arrives_in_next_block() {
        let h = [1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
        let mut filter = HConvSingle::new(&h, 4, 1);
        let mut output = [0.0; 4];
        filter.put(&[1.0, 0.0, 0.0, 0.0]);
        filter.process();
        filter.get(&mut output);
        for (actual, expected) in output.iter().zip([1.0, 2.0, 3.0, 4.0]) {
            assert!((actual - expected).abs() < 1e-12);
        }

        filter.put(&[0.0; 4]);
        filter.process();
        filter.get(&mut output);
        for (actual, expected) in output.iter().zip([5.0, 6.0, 0.0, 0.0]) {
            assert!((actual - expected).abs() < 1e-12);
        }
    }

    #[test]
    fn get_add_accumulates() {
        let mut filter = HConvSingle::new(&[1.0], 2, 1);
        let mut output = [2.0, 3.0];
        filter.put(&[1.0, 0.0]);
        filter.process();
        filter.get_add(&mut output);
        assert_eq!(output, [3.0, 3.0]);
    }
}
