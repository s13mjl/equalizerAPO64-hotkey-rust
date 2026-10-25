//! Port of `filters/CopyFilter.{h,cpp}`.

use eapo_common::channel_helper;
use eapo_common::string_helper;
use eapo_common::trace_log;

use crate::{channel, IFilter};

#[derive(Clone, Debug, PartialEq)]
pub struct Summand {
    pub factor: f64,
    pub is_decibel: bool,
    pub channel: String,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Assignment {
    pub target_channel: String,
    pub source_sum: Vec<Summand>,
}

struct InternalSummand {
    factor: f64,
    channel: i32,
}

struct InternalAssignment {
    target_channel: i32,
    source_sum: Vec<InternalSummand>,
}

pub struct CopyFilter {
    assignments: Vec<Assignment>,
    internal_assignments: Vec<InternalAssignment>,
}

impl CopyFilter {
    pub fn new(assignments: Vec<Assignment>) -> Self {
        Self {
            assignments,
            internal_assignments: Vec::new(),
        }
    }

    pub fn from_parameters(parameters: &str) -> Self {
        let mut assignments = Vec::new();

        for assignment_string in string_helper::split(parameters, ' ', true) {
            let parts = string_helper::split(&assignment_string, '=', true);
            if parts.len() != 2 {
                continue;
            }

            let target = parts[0].clone();
            let source = parts[1].clone();
            let mut source_sum = Vec::new();

            for summand_string in string_helper::split(&source, '+', true) {
                let factors = string_helper::split(&summand_string, '*', true);
                let (factor, channel) = match factors.len() {
                    2 => (factors[0].clone(), factors[1].clone()),
                    1 => {
                        if factors[0] == "0" || factors[0].contains('.') {
                            (factors[0].clone(), String::new())
                        } else {
                            (String::new(), factors[0].clone())
                        }
                    }
                    _ => (String::new(), String::new()),
                };

                let (factor_value, is_decibel) = if factor.is_empty() {
                    (1.0, false)
                } else {
                    let factor_value = leading_number(&factor);
                    let lower = factor.to_lowercase();
                    let is_decibel = factor.chars().count() > 2 && lower.ends_with("db");
                    (factor_value, is_decibel)
                };

                source_sum.push(Summand {
                    factor: factor_value,
                    is_decibel,
                    channel,
                });
            }

            let assignment = Assignment {
                target_channel: target,
                source_sum,
            };
            if !assignment.target_channel.is_empty() && !assignment.source_sum.is_empty() {
                assignments.push(assignment);
            }
        }

        Self::new(assignments)
    }

    pub fn get_assignments(&self) -> &[Assignment] {
        &self.assignments
    }
}

fn leading_number(s: &str) -> f64 {
    let trimmed = s.trim_start();
    let end = trimmed
        .char_indices()
        .find(|(_, c)| !matches!(c, '0'..='9' | '+' | '-' | '.' | 'e' | 'E'))
        .map_or(trimmed.len(), |(index, _)| index);
    trimmed[..end].parse().unwrap_or(0.0)
}

impl IFilter for CopyFilter {
    fn get_all_channels(&self) -> bool {
        true
    }

    fn get_in_place(&self) -> bool {
        false
    }

    fn initialize(
        &mut self,
        _sample_rate: f32,
        _max_frame_count: usize,
        channel_names: Vec<String>,
    ) -> Vec<String> {
        self.internal_assignments.clear();
        let mut out_channel_names: Vec<String> = Vec::new();

        for assignment in &self.assignments {
            let mut channel_name = assignment.target_channel.clone();
            let channel_index =
                channel_helper::get_channel_index(&assignment.target_channel, &channel_names, true);
            if channel_index != -1 {
                channel_name = channel_names[channel_index as usize].clone();
            }
            let target_channel = match out_channel_names.iter().position(|name| *name == channel_name)
            {
                Some(index) => index,
                None => {
                    out_channel_names.push(channel_name);
                    out_channel_names.len() - 1
                }
            };

            let mut source_sum = Vec::with_capacity(assignment.source_sum.len());
            for summand in &assignment.source_sum {
                let channel = if !summand.channel.is_empty() {
                    channel_helper::get_channel_index(&summand.channel, &channel_names, false)
                } else {
                    -1
                };
                let factor = if summand.is_decibel {
                    10f64.powf(summand.factor / 20.0)
                } else {
                    summand.factor
                };
                source_sum.push(InternalSummand { factor, channel });
            }

            self.internal_assignments.push(InternalAssignment {
                target_channel: target_channel as i32,
                source_sum,
            });
        }

        let mut message = String::from("Copying ");
        for (i, internal) in self.internal_assignments.iter().enumerate() {
            if i > 0 {
                message.push_str(", ");
            }
            message.push_str(&format!(
                "to channel {} ",
                out_channel_names[internal.target_channel as usize]
            ));
            for (j, summand) in internal.source_sum.iter().enumerate() {
                if j > 0 {
                    message.push_str(", ");
                }
                if summand.channel != -1 {
                    message.push_str(&format!(
                        "from channel {} with factor {}",
                        channel_names[summand.channel as usize], summand.factor
                    ));
                } else {
                    message.push_str(&format!("value {}", summand.factor));
                }
            }
        }
        trace_log!("{}", message);

        out_channel_names
    }

    unsafe fn process(
        &mut self,
        output: &mut [*mut f64],
        input: &[*mut f64],
        frame_count: usize,
    ) {
        for assignment in &self.internal_assignments {
            if assignment.target_channel == -1 || assignment.source_sum.is_empty() {
                continue;
            }
            let target = channel(output[assignment.target_channel as usize], frame_count);
            let first = &assignment.source_sum[0];
            if first.channel == -1 {
                target.fill(first.factor);
            } else {
                let source = channel(input[first.channel as usize], frame_count);
                if first.factor == 1.0 {
                    target.copy_from_slice(source);
                } else {
                    for i in 0..frame_count {
                        target[i] = first.factor * source[i];
                    }
                }
            }

            for summand in &assignment.source_sum[1..] {
                if summand.channel == -1 {
                    for sample in target[..frame_count].iter_mut() {
                        *sample += summand.factor;
                    }
                } else {
                    let source = channel(input[summand.channel as usize], frame_count);
                    if summand.factor == 1.0 {
                        for i in 0..frame_count {
                            target[i] += source[i];
                        }
                    } else {
                        for i in 0..frame_count {
                            target[i] += summand.factor * source[i];
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        vec!["L".to_string(), "R".to_string()]
    }

    fn run(filter: &mut CopyFilter, left: &mut [f64], right: &mut [f64]) -> (Vec<f64>, Vec<f64>) {
        let mut left_out = vec![0.0; left.len()];
        let mut right_out = vec![0.0; right.len()];
        unsafe {
            filter.process(
                &mut [left_out.as_mut_ptr(), right_out.as_mut_ptr()],
                &[left.as_mut_ptr(), right.as_mut_ptr()],
                left.len(),
            )
        };
        (left_out, right_out)
    }

    #[test]
    fn copies_and_sums_assignments() {
        let mut filter = CopyFilter::new(vec![
            Assignment {
                target_channel: "L".to_string(),
                source_sum: vec![Summand {
                    factor: 1.0,
                    is_decibel: false,
                    channel: "R".to_string(),
                }],
            },
            Assignment {
                target_channel: "R".to_string(),
                source_sum: vec![
                    Summand {
                        factor: 0.5,
                        is_decibel: false,
                        channel: "L".to_string(),
                    },
                    Summand {
                        factor: -3.0,
                        is_decibel: false,
                        channel: String::new(),
                    },
                ],
            },
        ]);
        filter.initialize(48000.0, 2, names());
        let mut left = vec![2.0, 4.0];
        let mut right = vec![4.0, 8.0];
        let (left_out, right_out) = run(&mut filter, &mut left, &mut right);
        assert_eq!(left_out, vec![4.0, 8.0]);
        assert_eq!(right_out, vec![-2.0, -1.0]);
    }

    #[test]
    fn decibel_factors_are_converted() {
        let mut filter = CopyFilter::new(vec![Assignment {
            target_channel: "L".to_string(),
            source_sum: vec![Summand {
                factor: 6.020599913279624,
                is_decibel: true,
                channel: "R".to_string(),
            }],
        }]);
        filter.initialize(48000.0, 2, names());
        let mut left = vec![0.0];
        let mut right = vec![0.25];
        let (left_out, _) = run(&mut filter, &mut left, &mut right);
        assert!((left_out[0] - 0.5).abs() < 1e-12);
    }

    #[test]
    fn parses_factory_parameter_syntax() {
        let filter = CopyFilter::from_parameters("L=R R=0.5*L+-3.0");
        assert_eq!(filter.get_assignments().len(), 2);
        let second = &filter.get_assignments()[1];
        assert_eq!(second.source_sum.len(), 2);
        assert_eq!(second.source_sum[0].factor, 0.5);
        assert_eq!(second.source_sum[1].factor, -3.0);
        assert!(second.source_sum[1].channel.is_empty());
    }
}
