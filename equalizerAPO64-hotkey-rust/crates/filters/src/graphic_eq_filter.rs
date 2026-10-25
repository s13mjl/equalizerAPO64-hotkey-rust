//! Port of `filters/GraphicEQFilter.{h,cpp}`.

use eapo_common::gain_iterator::FilterNode;

use crate::convolution_filter::ConvolutionFilter;
use crate::IFilter;

pub struct GraphicEQFilter {
    filter: ConvolutionFilter,
}

impl GraphicEQFilter {
    pub fn new(nodes: Vec<FilterNode>, filter_length: usize) -> Self {
        Self {
            filter: ConvolutionFilter::new_graphic_eq(nodes, filter_length),
        }
    }

    pub fn get_nodes(&self) -> &[FilterNode] {
        self.filter.graphic_eq_nodes()
    }
}

impl IFilter for GraphicEQFilter {
    fn initialize(
        &mut self,
        sample_rate: f32,
        max_frame_count: usize,
        channel_names: Vec<String>,
    ) -> Vec<String> {
        self.filter
            .initialize(sample_rate, max_frame_count, channel_names)
    }

    unsafe fn process(
        &mut self,
        output: &mut [*mut f64],
        input: &[*mut f64],
        frame_count: usize,
    ) {
        self.filter.process(output, input, frame_count);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use eapo_common::gain_iterator::GainIterator;

    #[test]
    fn gain_iterator_interpolates_between_nodes() {
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
        let middle = (100.0f64 * 1000.0).sqrt();
        assert!((iterator.gain_at(middle) - 5.0).abs() < 1e-12);
    }

    #[test]
    fn graphic_eq_produces_finite_impulse_response() {
        let nodes = vec![
            FilterNode {
                freq: 100.0,
                db_gain: 6.0,
            },
            FilterNode {
                freq: 1000.0,
                db_gain: -6.0,
            },
        ];
        let mut filter = GraphicEQFilter::new(nodes, 128);
        assert_eq!(filter.get_nodes().len(), 2);
        filter.initialize(48000.0, 256, vec!["L".into()]);
        let mut input = vec![0.0; 256];
        let mut output = vec![0.0; 256];
        input[0] = 1.0;
        unsafe { filter.process(&mut [output.as_mut_ptr()], &[input.as_mut_ptr()], 256) };
        assert!(output.iter().all(|sample| sample.is_finite()));
        assert!(output.iter().any(|sample| sample.abs() > 1e-6));
    }
}
