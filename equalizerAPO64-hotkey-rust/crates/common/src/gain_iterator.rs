//! Port of `helpers/GainIterator.{h,cpp}`.

/// A point of the gain curve (`FilterNode` in the C++ code).
#[derive(Clone, Copy, Debug)]
pub struct FilterNode {
    pub freq: f64,
    pub db_gain: f64,
}

impl FilterNode {
    /// Mirrors the C++ `FilterNode(double freq, double dbGain)` constructor.
    pub fn new(freq: f64, db_gain: f64) -> Self {
        Self { freq, db_gain }
    }
}

/// Interpolates gain values between sorted [`FilterNode`]s on a log
/// frequency axis, caching the current bracket like the C++ class.
pub struct GainIterator {
    nodes: Vec<FilterNode>,
    left: Option<usize>,
    right: Option<usize>,
    log_left: f64,
    log_right_minus_left: f64,
}

impl GainIterator {
    /// Creates an iterator over `nodes` (must be sorted by frequency).
    pub fn new(nodes: Vec<FilterNode>) -> Self {
        Self {
            nodes,
            left: None,
            right: None,
            log_left: 0.0,
            log_right_minus_left: 0.0,
        }
    }

    /// Returns the interpolated gain at `freq` in dB.
    pub fn gain_at(&mut self, freq: f64) -> f64 {
        let needs_recompute = (self.left.is_none() && self.right.is_none())
            || (self.left.is_some() && freq < self.nodes[self.left.unwrap()].freq);
        if needs_recompute {
            let it = self.nodes.partition_point(|n| n.freq < freq);
            self.left = if it != 0 { Some(it - 1) } else { None };
            self.right = if it != self.nodes.len() {
                Some(it)
            } else {
                None
            };
            if let (Some(l), Some(r)) = (self.left, self.right) {
                self.log_left = self.nodes[l].freq.ln();
                self.log_right_minus_left = self.nodes[r].freq.ln() - self.log_left;
            }
        } else if let Some(r) = self.right {
            if freq > self.nodes[r].freq {
                let mut it = r;
                while it != self.nodes.len() && freq > self.nodes[it].freq {
                    it += 1;
                }
                self.right = if it == self.nodes.len() {
                    None
                } else {
                    Some(it)
                };
                self.left = if it != 0 { Some(it - 1) } else { None };
                if let (Some(l), Some(rr)) = (self.left, self.right) {
                    self.log_left = self.nodes[l].freq.ln();
                    self.log_right_minus_left = self.nodes[rr].freq.ln() - self.log_left;
                }
            }
        }

        match (self.left, self.right) {
            (None, None) => 0.0,
            (None, Some(r)) => self.nodes[r].db_gain,
            (Some(l), None) => self.nodes[l].db_gain,
            (Some(l), Some(r)) => {
                let left = self.nodes[l];
                let right = self.nodes[r];
                if left.db_gain == right.db_gain {
                    left.db_gain
                } else {
                    let t = (freq.ln() - self.log_left) / self.log_right_minus_left;
                    left.db_gain + t * (right.db_gain - left.db_gain)
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_curve_returns_zero() {
        let mut iterator = GainIterator::new(Vec::new());
        assert_eq!(iterator.gain_at(100.0), 0.0);
    }

    #[test]
    fn single_node_returns_its_gain_everywhere() {
        let mut iterator = GainIterator::new(vec![FilterNode {
            freq: 1000.0,
            db_gain: -6.0,
        }]);
        assert_eq!(iterator.gain_at(10.0), -6.0);
        assert_eq!(iterator.gain_at(1000.0), -6.0);
        assert_eq!(iterator.gain_at(20000.0), -6.0);
    }

    #[test]
    fn interpolates_logarithmically() {
        let nodes = vec![
            FilterNode {
                freq: 100.0,
                db_gain: 0.0,
            },
            FilterNode {
                freq: 1000.0,
                db_gain: 10.0,
            },
        ];
        let mut iterator = GainIterator::new(nodes);
        assert_eq!(iterator.gain_at(100.0), 0.0);
        let middle = (100.0f64 * 1000.0).sqrt();
        assert!((iterator.gain_at(middle) - 5.0).abs() < 1e-12);
        assert_eq!(iterator.gain_at(1000.0), 10.0);
        // Outside the node range the nearest node gain is used.
        assert_eq!(iterator.gain_at(50.0), 0.0);
        assert_eq!(iterator.gain_at(2000.0), 10.0);
    }

    #[test]
    fn cache_handles_moving_backwards() {
        let mut iterator = GainIterator::new(vec![
            FilterNode {
                freq: 100.0,
                db_gain: 0.0,
            },
            FilterNode {
                freq: 1000.0,
                db_gain: 10.0,
            },
        ]);
        assert_eq!(iterator.gain_at(50.0), 0.0);
        assert_eq!(iterator.gain_at(2000.0), 10.0);
        assert_eq!(iterator.gain_at(100.0), 0.0);
        assert!((iterator.gain_at(316.22776601683796) - 5.0).abs() < 1e-9);
    }

    #[test]
    fn equal_gains_do_not_produce_nan() {
        let mut iterator = GainIterator::new(vec![
            FilterNode {
                freq: 100.0,
                db_gain: f64::NEG_INFINITY,
            },
            FilterNode {
                freq: 1000.0,
                db_gain: f64::NEG_INFINITY,
            },
        ]);
        assert_eq!(iterator.gain_at(300.0), f64::NEG_INFINITY);
    }
}
