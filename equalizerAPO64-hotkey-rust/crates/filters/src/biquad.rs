//! Port of `filters/BiQuad.{h,cpp}`.

use std::f64::consts::{LN_2, PI};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BiQuadType {
    LowPass,
    HighPass,
    BandPass,
    Notch,
    AllPass,
    Peaking,
    LowShelf,
    HighShelf,
}

#[derive(Clone, Copy, Debug)]
pub struct BiQuad {
    a: [f64; 4],
    a0: f64,
    x1: f64,
    x2: f64,
    y1: f64,
    y2: f64,
}

impl Default for BiQuad {
    fn default() -> Self {
        Self {
            a: [0.0; 4],
            a0: 0.0,
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }
}

impl BiQuad {
    pub fn new(
        type_: BiQuadType,
        db_gain: f64,
        freq: f64,
        srate: f64,
        bandwidth_or_q_or_s: f64,
        is_bandwidth_or_s: bool,
    ) -> Self {
        let a = if matches!(
            type_,
            BiQuadType::Peaking | BiQuadType::LowShelf | BiQuadType::HighShelf
        ) {
            10f64.powf(db_gain / 40.0)
        } else {
            10f64.powf(db_gain / 20.0)
        };
        let omega = 2.0 * PI * freq / srate;
        let sn = omega.sin();
        let cs = omega.cos();

        let alpha = if !is_bandwidth_or_s {
            sn / (2.0 * bandwidth_or_q_or_s)
        } else if matches!(type_, BiQuadType::LowShelf | BiQuadType::HighShelf) {
            sn / 2.0 * ((a + 1.0 / a) * (1.0 / bandwidth_or_q_or_s - 1.0) + 2.0).sqrt()
        } else {
            sn * (LN_2 / 2.0 * bandwidth_or_q_or_s * omega / sn).sinh()
        };

        let beta = 2.0 * a.sqrt() * alpha;

        let (b0, b1, b2, a0, a1, a2) = match type_ {
            BiQuadType::LowPass => (
                (1.0 - cs) / 2.0,
                1.0 - cs,
                (1.0 - cs) / 2.0,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
            BiQuadType::HighPass => (
                (1.0 + cs) / 2.0,
                -(1.0 + cs),
                (1.0 + cs) / 2.0,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
            BiQuadType::BandPass => (alpha, 0.0, -alpha, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            BiQuadType::Notch => (1.0, -2.0 * cs, 1.0, 1.0 + alpha, -2.0 * cs, 1.0 - alpha),
            BiQuadType::AllPass => (
                1.0 - alpha,
                -2.0 * cs,
                1.0 + alpha,
                1.0 + alpha,
                -2.0 * cs,
                1.0 - alpha,
            ),
            BiQuadType::Peaking => (
                1.0 + alpha * a,
                -2.0 * cs,
                1.0 - alpha * a,
                1.0 + alpha / a,
                -2.0 * cs,
                1.0 - alpha / a,
            ),
            BiQuadType::LowShelf => (
                a * ((a + 1.0) - (a - 1.0) * cs + beta),
                2.0 * a * ((a - 1.0) - (a + 1.0) * cs),
                a * ((a + 1.0) - (a - 1.0) * cs - beta),
                (a + 1.0) + (a - 1.0) * cs + beta,
                -2.0 * ((a - 1.0) + (a + 1.0) * cs),
                (a + 1.0) + (a - 1.0) * cs - beta,
            ),
            BiQuadType::HighShelf => (
                a * ((a + 1.0) + (a - 1.0) * cs + beta),
                -2.0 * a * ((a - 1.0) + (a + 1.0) * cs),
                a * ((a + 1.0) + (a - 1.0) * cs - beta),
                (a + 1.0) - (a - 1.0) * cs + beta,
                2.0 * ((a - 1.0) - (a + 1.0) * cs),
                (a + 1.0) - (a - 1.0) * cs - beta,
            ),
        };

        Self {
            a0: b0 / a0,
            a: [b1 / a0, b2 / a0, a1 / a0, a2 / a0],
            x1: 0.0,
            x2: 0.0,
            y1: 0.0,
            y2: 0.0,
        }
    }

    #[inline]
    pub fn process(&mut self, sample: f64) -> f64 {
        let result = self.a0 * sample + self.a[1] * self.x2 + self.a[0] * self.x1
            - self.a[3] * self.y2
            - self.a[2] * self.y1;

        self.x2 = self.x1;
        self.x1 = sample;
        self.y2 = self.y1;
        self.y1 = result;

        result
    }

    #[inline]
    pub fn remove_denormals(&mut self) {
        if self.x1.abs() < f64::MIN_POSITIVE {
            self.x1 = 0.0;
        }
        if self.x2.abs() < f64::MIN_POSITIVE {
            self.x2 = 0.0;
        }
        if self.y1.abs() < f64::MIN_POSITIVE {
            self.y1 = 0.0;
        }
        if self.y2.abs() < f64::MIN_POSITIVE {
            self.y2 = 0.0;
        }
    }

    pub fn set_coefficients(&mut self, a_in: &[f64; 4], a0_in: f64) {
        self.a = *a_in;
        self.a0 = a0_in;
    }

    pub fn gain_at(&self, freq: f64, srate: f64) -> f64 {
        let omega = 2.0 * PI * freq / srate;
        let sn = (omega / 2.0).sin();
        let phi = sn * sn;
        let b0 = self.a0;
        let b1 = self.a[0];
        let b2 = self.a[1];
        let a0 = 1.0;
        let a1 = self.a[2];
        let a2 = self.a[3];

        let numerator =
            (b0 + b1 + b2).powi(2) - 4.0 * (b0 * b1 + 4.0 * b0 * b2 + b1 * b2) * phi
                + 16.0 * b0 * b2 * phi * phi;
        let denominator =
            (a0 + a1 + a2).powi(2) - 4.0 * (a0 * a1 + 4.0 * a0 * a2 + a1 * a2) * phi
                + 16.0 * a0 * a2 * phi * phi;

        10.0 * numerator.log10() - 10.0 * denominator.log10()
    }

    pub fn get_coefficients(&self) -> ([f64; 4], f64) {
        (self.a, self.a0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SRATE: f64 = 48000.0;

    #[test]
    fn peaking_has_expected_center_gain() {
        let biquad = BiQuad::new(BiQuadType::Peaking, 6.0, 1000.0, SRATE, 1.0, false);
        assert!((biquad.gain_at(1000.0, SRATE) - 6.0).abs() < 1e-9);
        assert!(biquad.gain_at(20.0, SRATE).abs() < 0.1);
        assert!(biquad.gain_at(20000.0, SRATE).abs() < 0.1);
    }

    #[test]
    fn low_pass_rejects_high_frequencies() {
        let biquad = BiQuad::new(
            BiQuadType::LowPass,
            0.0,
            1000.0,
            SRATE,
            std::f64::consts::FRAC_1_SQRT_2,
            false,
        );
        assert!(biquad.gain_at(100.0, SRATE).abs() < 0.01);
        assert!((biquad.gain_at(1000.0, SRATE) + 3.0103).abs() < 0.01);
        assert!(biquad.gain_at(20000.0, SRATE) < -40.0);
    }

    #[test]
    fn difference_equation_matches_processing() {
        let mut biquad = BiQuad::new(BiQuadType::LowShelf, -3.0, 200.0, SRATE, 0.9, true);
        let (coeffs, a0) = biquad.get_coefficients();
        let input = [1.0, 0.25, -0.5, 0.0, 0.75];
        let mut x1 = 0.0;
        let mut x2 = 0.0;
        let mut y1 = 0.0;
        let mut y2 = 0.0;
        for &sample in &input {
            let expected =
                a0 * sample + coeffs[1] * x2 + coeffs[0] * x1 - coeffs[3] * y2 - coeffs[2] * y1;
            let actual = biquad.process(sample);
            assert_eq!(actual, expected);
            x2 = x1;
            x1 = sample;
            y2 = y1;
            y1 = expected;
        }
    }
}
