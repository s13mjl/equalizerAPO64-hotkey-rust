//! Thin adapter between the configuration parser and the filter crate.
//!
//! The factory logic itself lives in `factories.rs` and is a direct port of
//! each C++ `*FilterFactory::createFilter`. Only object construction is
//! wrapped here so the configuration code stays free of filter internals.

use std::collections::HashMap;

use eapo_common::gain_iterator::FilterNode;
use eapo_filters::biquad::BiQuadType;
use eapo_filters::biquad_filter::BiQuadFilter;
use eapo_filters::channel_filter::ChannelFilter;
use eapo_filters::convolution_filter::ConvolutionFilter;
use eapo_filters::copy_filter::{Assignment as FilterAssignment, CopyFilter, Summand};
use eapo_filters::delay_filter::DelayFilter;
use eapo_filters::graphic_eq_filter::GraphicEQFilter;
use eapo_filters::iir_filter::IIRFilter;
use eapo_filters::loudness_correction::LoudnessCorrectionFilter;
use eapo_filters::preamp_filter::PreampFilter;
use eapo_filters::vst_plugin::VSTPluginFilter;
use eapo_filters::vst_plugin_library::VSTPluginLibrary;
use eapo_filters::IFilter;

use crate::factories::{BiquadKind, CopyAssignment};

pub type VstLibrary = VSTPluginLibrary;

pub fn make_biquad(
    kind: BiquadKind,
    db_gain: f64,
    freq: f64,
    bandwidth_or_q_or_s: f64,
    is_bandwidth_or_s: bool,
    is_corner_freq: bool,
) -> Box<dyn IFilter> {
    let type_ = match kind {
        BiquadKind::Peaking => BiQuadType::Peaking,
        BiquadKind::LowPass => BiQuadType::LowPass,
        BiquadKind::HighPass => BiQuadType::HighPass,
        BiquadKind::BandPass => BiQuadType::BandPass,
        BiquadKind::LowShelf => BiQuadType::LowShelf,
        BiquadKind::HighShelf => BiQuadType::HighShelf,
        BiquadKind::Notch => BiQuadType::Notch,
        BiquadKind::AllPass => BiQuadType::AllPass,
    };
    Box::new(BiQuadFilter::new(
        type_,
        db_gain,
        freq,
        bandwidth_or_q_or_s,
        is_bandwidth_or_s,
        is_corner_freq,
    ))
}

pub fn make_iir(coefficients: Vec<f64>) -> Box<dyn IFilter> {
    Box::new(IIRFilter::new(coefficients))
}

pub fn make_graphic_eq(nodes: Vec<FilterNode>, filter_length: u32) -> Box<dyn IFilter> {
    Box::new(GraphicEQFilter::new(nodes, filter_length as usize))
}

pub fn make_convolution(filename: String) -> Box<dyn IFilter> {
    Box::new(ConvolutionFilter::new(filename))
}

pub fn make_channel(words: Vec<String>) -> Box<dyn IFilter> {
    Box::new(ChannelFilter::new(words))
}

pub fn make_copy(assignments: Vec<CopyAssignment>) -> Box<dyn IFilter> {
    let assignments = assignments
        .into_iter()
        .map(|assignment| FilterAssignment {
            target_channel: assignment.target_channel,
            source_sum: assignment
                .source_sum
                .into_iter()
                .map(|summand| Summand {
                    factor: summand.factor,
                    is_decibel: summand.is_decibel,
                    channel: summand.channel,
                })
                .collect(),
        })
        .collect();
    Box::new(CopyFilter::new(assignments))
}

pub fn make_delay(delay: f64, is_ms: bool) -> Box<dyn IFilter> {
    Box::new(DelayFilter::new(delay, is_ms))
}

pub fn make_preamp(db_gain: f64) -> Box<dyn IFilter> {
    Box::new(PreampFilter::new(db_gain))
}

pub fn make_loudness(parameters: &str) -> Option<Box<dyn IFilter>> {
    LoudnessCorrectionFilter::from_parameters(parameters)
        .map(|filter| Box::new(filter) as Box<dyn IFilter>)
}

pub fn get_vst_library(path: &str) -> Option<std::sync::Arc<VstLibrary>> {
    Some(VSTPluginLibrary::get_instance(path))
}

pub fn initialize_vst_library(library: &VstLibrary) -> i32 {
    library.initialize()
}

pub fn vst_library_path(library: &VstLibrary) -> String {
    library.lib_path().to_string()
}

pub fn make_vst(
    library: std::sync::Arc<VstLibrary>,
    chunk_data: String,
    param_map: HashMap<String, f32>,
) -> Box<dyn IFilter> {
    Box::new(VSTPluginFilter::new(library, chunk_data, param_map))
}
