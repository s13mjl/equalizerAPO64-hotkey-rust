//! Port of `filters/ChannelFilter.{h,cpp}`.

use eapo_common::channel_helper;
use eapo_common::trace_log;

use crate::IFilter;

pub struct ChannelFilter {
    words: Vec<String>,
}

impl ChannelFilter {
    pub fn new(words: Vec<String>) -> Self {
        Self { words }
    }

    pub fn from_parameters(parameters: &str) -> Self {
        let value = format!("{} ", parameters);
        let mut words = Vec::new();
        let mut current = String::new();
        for c in value.chars() {
            if c == ' ' {
                if !current.is_empty() {
                    words.push(std::mem::take(&mut current));
                }
            } else {
                current.extend(c.to_uppercase());
            }
        }
        Self::new(words)
    }

    pub fn get_words(&self) -> &[String] {
        &self.words
    }
}

impl IFilter for ChannelFilter {
    fn get_all_channels(&self) -> bool {
        true
    }

    fn get_select_channels(&self) -> bool {
        true
    }

    fn initialize(
        &mut self,
        _sample_rate: f32,
        _max_frame_count: usize,
        channel_names: Vec<String>,
    ) -> Vec<String> {
        let channel_count = channel_names.len();
        let mut selected_channels = vec![false; channel_count];

        for word in &self.words {
            if word == "ALL" {
                selected_channels = vec![true; channel_count];
            } else {
                let channel_nr = channel_helper::get_channel_index(word, &channel_names, false);
                if channel_nr != -1 && (channel_nr as usize) < channel_count {
                    selected_channels[channel_nr as usize] = true;
                }
            }
        }

        let selected_channel_names: Vec<String> = channel_names
            .iter()
            .zip(selected_channels.iter())
            .filter(|(_, selected)| **selected)
            .map(|(name, _)| name.clone())
            .collect();

        let channel_numbers = selected_channels
            .iter()
            .enumerate()
            .filter(|(_, selected)| **selected)
            .map(|(index, _)| (index + 1).to_string())
            .collect::<Vec<_>>()
            .join(", ");
        trace_log!("Selecting channel(s) number {}", channel_numbers);

        selected_channel_names
    }

    unsafe fn process(
        &mut self,
        _output: &mut [*mut f64],
        _input: &[*mut f64],
        _frame_count: usize,
    ) {
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names() -> Vec<String> {
        vec![
            "L".to_string(),
            "R".to_string(),
            "C".to_string(),
            "LFE".to_string(),
        ]
    }

    #[test]
    fn selects_named_and_numbered_channels() {
        let mut filter = ChannelFilter::new(vec!["L".to_string(), "2".to_string()]);
        assert_eq!(
            filter.initialize(48000.0, 4, names()),
            vec!["L".to_string(), "R".to_string()]
        );
    }

    #[test]
    fn all_resets_the_selection() {
        let mut filter = ChannelFilter::new(vec!["C".to_string(), "ALL".to_string()]);
        assert_eq!(filter.initialize(48000.0, 4, names()), names());
    }

    #[test]
    fn parameters_are_upper_cased_and_split() {
        let filter = ChannelFilter::from_parameters(" l  r ");
        assert_eq!(filter.get_words(), &["L".to_string(), "R".to_string()]);
    }
}
