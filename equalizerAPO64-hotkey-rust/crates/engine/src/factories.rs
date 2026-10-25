//! Direct port of every `IFilterFactory` implementation.
//!
//! Dispatch order is identical to `FilterEngine::loadConfigFile`:
//! Device, If, Expression/inline, Include, Stage, Channel, IIR, BiQuad,
//! Preamp, Delay, Copy, Convolution, GraphicEQ, VSTPlugin, LoudnessCorrection.

use std::collections::{HashMap, HashSet};
use std::path::Path;
use std::sync::OnceLock;

use eapo_common::gain_iterator::FilterNode;
use eapo_common::{channel_helper, registry_helper, string_helper, APP_REGPATH};
use eapo_expression::{ExpressionHost, Value};
use eapo_filters::IFilter;
use regex::Regex;

use crate::adapters;
use crate::filter_engine::Loader;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BiquadKind {
    Peaking,
    LowPass,
    HighPass,
    BandPass,
    LowShelf,
    HighShelf,
    Notch,
    AllPass,
}

impl BiquadKind {
    fn description(self) -> &'static str {
        match self {
            BiquadKind::Peaking => "peaking",
            BiquadKind::LowPass => "low-pass",
            BiquadKind::HighPass => "high-pass",
            BiquadKind::BandPass => "band-pass",
            BiquadKind::LowShelf => "low-shelf",
            BiquadKind::HighShelf => "high-shelf",
            BiquadKind::Notch => "notch",
            BiquadKind::AllPass => "all-pass",
        }
    }
}

#[derive(Clone, Debug)]
pub struct CopySummand {
    pub factor: f64,
    pub is_decibel: bool,
    pub channel: String,
}

#[derive(Clone, Debug)]
pub struct CopyAssignment {
    pub target_channel: String,
    pub source_sum: Vec<CopySummand>,
}

pub(crate) struct ExpressionHostImpl<'a> {
    pub watched: &'a mut HashSet<String>,
}

impl<'a> ExpressionHost for ExpressionHostImpl<'a> {
    fn read_reg_string(&mut self, key: &str, value_name: &str) -> Result<String, String> {
        registry_helper::read_value(key, value_name).map_err(|e| e.to_string())
    }

    fn read_reg_dword(&mut self, key: &str, value_name: &str) -> Result<u32, String> {
        registry_helper::read_dword_value(key, value_name).map_err(|e| e.to_string())
    }

    fn watch_registry_key(&mut self, key: &str) {
        self.watched.insert(key.to_string());
    }
}

pub(crate) fn initialize_factories(loader: &mut Loader) {
    loader
        .parser
        .define_const("deviceName", Value::String(loader.device_name.clone()));
    loader.parser.define_const(
        "connectionName",
        Value::String(loader.connection_name.clone()),
    );
    loader
        .parser
        .define_const("deviceGuid", Value::String(loader.device_guid.clone()));
    loader.parser.define_const(
        "inputChannelCount",
        Value::Int(loader.settings.input_channel_count as i64),
    );
    loader.parser.define_const(
        "outputChannelCount",
        Value::Int(loader.settings.output_channel_count as i64),
    );
    loader
        .parser
        .define_const("sampleRate", Value::Float(loader.settings.sample_rate as f64));
    let stage = if loader.capture {
        "capture"
    } else if loader.pre_mix {
        "pre-mix"
    } else {
        "post-mix"
    };
    loader
        .parser
        .define_const("stage", Value::String(stage.to_string()));
}

pub(crate) fn start_of_configuration(loader: &mut Loader) {
    loader.device_matches = true;
    loader.if_true_count = 0;
    loader.if_false_count = 0;
    loader.if_true_count_stack.clear();
    loader.if_execute_else = false;
    loader.include_recursion_depth = -1;
    loader.stage_matches = loader.capture || !loader.pre_mix || !loader.post_mix_installed;
    loader.stage_matches_stack.clear();
}

pub(crate) fn start_of_file(loader: &mut Loader, _config_path: &str) {
    loader.if_true_count_stack.push(loader.if_true_count);
    loader.if_true_count = 0;
    loader.if_execute_else = false;
    if loader.if_false_count != 0 {
        eapo_common::error_log!("File was included inside If that evaluated to false!");
        loader.if_false_count = 0;
    }
    loader.include_recursion_depth += 1;
    loader.stage_matches_stack.push(loader.stage_matches);
}

pub(crate) fn end_of_file(loader: &mut Loader, _config_path: &str) {
    loader.device_matches = true;
    if loader.if_true_count != 0 || loader.if_false_count != 0 {
        eapo_common::error_log!("If was not closed by EndIf!");
        loader.if_false_count = 0;
    }
    loader.if_true_count = loader.if_true_count_stack.pop().unwrap_or(0);
    loader.include_recursion_depth -= 1;
    loader.stage_matches = loader.stage_matches_stack.pop().unwrap_or(true);
}

pub(crate) fn end_of_configuration(_loader: &mut Loader) {}

type FactoryFn = fn(&mut Loader, &str, &mut String, &mut String) -> Vec<Box<dyn IFilter>>;

const FACTORIES: [FactoryFn; 15] = [
    device_factory,
    if_factory,
    expression_factory,
    include_factory,
    stage_factory,
    channel_factory,
    iir_factory,
    biquad_factory,
    preamp_factory,
    delay_factory,
    copy_factory,
    convolution_factory,
    graphic_eq_factory,
    vst_plugin_factory,
    loudness_correction_factory,
];

pub(crate) fn dispatch_line(
    loader: &mut Loader,
    config_path: &str,
    command: &str,
    parameters: &str,
) {
    let mut command = command.to_string();
    let mut parameters = parameters.to_string();
    for factory in FACTORIES {
        let filters = factory(loader, config_path, &mut command, &mut parameters);
        if command.is_empty() {
            break;
        }
        if !filters.is_empty() {
            loader.add_filters(filters);
            break;
        }
    }
}

fn regex(cell: &'static OnceLock<Regex>, pattern: &str) -> &'static Regex {
    cell.get_or_init(|| Regex::new(pattern).unwrap())
}

fn parse_leading_f64(s: &str) -> Option<(f64, usize)> {
    static NUMBER: OnceLock<Regex> = OnceLock::new();
    let re = regex(&NUMBER, r"^\s*[-+]?(\d+(\.\d*)?|\.\d+)([eE][-+]?\d+)?");
    let m = re.find(s)?;
    let value: f64 = m.as_str().trim().parse().ok()?;
    Some((value, m.end()))
}

fn parse_number(s: &str) -> f64 {
    parse_leading_f64(s).map(|(v, _)| v).unwrap_or(0.0)
}

// ---------------------------------------------------------------------------
// Device
// ---------------------------------------------------------------------------

fn device_factory(
    loader: &mut Loader,
    _config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    if command == "Device" {
        let matches = match_device(&loader.device_string, parameters);
        eapo_common::trace_log!(
            "{}atching pattern \"{}\" with device \"{}\"",
            if matches { "M" } else { "Not m" },
            string_helper::trim(parameters),
            loader.device_string
        );
        loader.device_matches = matches;
    }
    if !loader.device_matches {
        command.clear();
    }
    Vec::new()
}

fn match_device(device_string: &str, pattern: &str) -> bool {
    static GUID: OnceLock<Regex> = OnceLock::new();
    let value = format!("{};", string_helper::trim(pattern));
    let mut full_list: Vec<Vec<String>> = Vec::new();
    let mut current_list: Vec<String> = Vec::new();
    let mut current_word = String::new();

    for c in value.chars() {
        if c == ' ' || c == ';' {
            if !current_word.is_empty() {
                current_list.push(std::mem::take(&mut current_word));
            }
            if c == ';' && !current_list.is_empty() {
                full_list.push(std::mem::take(&mut current_list));
            }
        } else {
            current_word.push(c);
        }
    }

    let guid_re = regex(&GUID, r"\{[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-[0-9a-fA-F]{12}\}");
    let device_string_no_guid = guid_re.replace_all(device_string, "").into_owned();
    let device_lower = device_string.to_lowercase();
    let device_no_guid_lower = device_string_no_guid.to_lowercase();

    let mut matches = false;
    for list in &full_list {
        matches = true;
        if list.len() == 1 && list[0].to_lowercase() == "all" {
            break;
        }
        for word in list {
            let word = word.to_lowercase();
            let match_string = if word.contains('{') {
                &device_lower
            } else {
                &device_no_guid_lower
            };
            if !match_string.contains(&word) {
                matches = false;
                break;
            }
        }
        if matches {
            break;
        }
    }
    matches
}

// ---------------------------------------------------------------------------
// If / ElseIf / Else / EndIf
// ---------------------------------------------------------------------------

fn if_factory(
    loader: &mut Loader,
    _config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    let expression = string_helper::trim(parameters);

    if command == "If" {
        if loader.if_false_count == 0 {
            match loader.eval_expression(&expression) {
                Ok(result) => {
                    let is_true = result.as_bool();
                    if result.type_char() == 'b' {
                        eapo_common::trace_log!(
                            "If({}) evaluated to {}",
                            expression,
                            result.to_string_repr()
                        );
                    } else {
                        eapo_common::trace_log!(
                            "If({}) evaluated to {} ({})",
                            expression,
                            result.to_string_repr(),
                            if is_true { "true" } else { "false" }
                        );
                    }
                    if is_true {
                        loader.if_true_count += 1;
                    } else {
                        loader.if_false_count += 1;
                        loader.if_execute_else = true;
                    }
                }
                Err(e) => {
                    eapo_common::error_log!("Error while evaluating If({}): {}", expression, e);
                    loader.if_false_count += 1;
                }
            }
        } else {
            loader.if_false_count += 1;
        }
    } else if command == "ElseIf" {
        if loader.if_false_count == 0 {
            if loader.if_true_count == 0 {
                eapo_common::error_log!("ElseIf without If!");
            } else {
                loader.if_false_count += 1;
                loader.if_true_count -= 1;
            }
        } else if loader.if_false_count == 1 && loader.if_execute_else {
            match loader.eval_expression(&expression) {
                Ok(result) => {
                    let is_true = result.as_bool();
                    if result.type_char() == 'b' {
                        eapo_common::trace_log!(
                            "ElseIf({}) evaluated to {}",
                            expression,
                            result.to_string_repr()
                        );
                    } else {
                        eapo_common::trace_log!(
                            "ElseIf({}) evaluated to {} ({})",
                            expression,
                            result.to_string_repr(),
                            if is_true { "true" } else { "false" }
                        );
                    }
                    if is_true {
                        loader.if_false_count -= 1;
                        loader.if_true_count += 1;
                        loader.if_execute_else = false;
                    }
                }
                Err(e) => {
                    eapo_common::error_log!(
                        "Error while evaluating ElseIf({}): {}",
                        expression,
                        e
                    );
                }
            }
        }
    } else if command == "Else" {
        if loader.if_false_count == 0 {
            if loader.if_true_count == 0 {
                eapo_common::error_log!("Else without If!");
            } else {
                loader.if_false_count += 1;
                loader.if_true_count -= 1;
            }
        } else if loader.if_false_count == 1 && loader.if_execute_else {
            loader.if_false_count -= 1;
            loader.if_true_count += 1;
            loader.if_execute_else = false;
        }
    } else if command == "EndIf" {
        if loader.if_false_count == 0 {
            if loader.if_true_count == 0 {
                eapo_common::error_log!("EndIf without If!");
            } else {
                loader.if_true_count -= 1;
            }
        } else {
            loader.if_false_count -= 1;
        }
        if loader.if_false_count == 0 {
            loader.if_execute_else = false;
        }
    }

    if loader.if_false_count > 0 {
        command.clear();
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// Expression / Eval / inline expressions
// ---------------------------------------------------------------------------

fn expression_factory(
    loader: &mut Loader,
    _config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    if command.starts_with('#') {
        return Vec::new();
    }

    let chars: Vec<char> = parameters.chars().collect();
    let mut in_expression = false;
    let mut last_was_backslash = false;
    let mut output = String::new();
    let mut expression = String::new();

    for i in 0..chars.len() {
        let c = chars[i];
        if c == '`' {
            if !in_expression {
                if last_was_backslash {
                    output.push(c);
                } else {
                    in_expression = true;
                }
            } else if last_was_backslash {
                expression.push(c);
            } else {
                in_expression = false;
                match loader.eval_expression(&expression) {
                    Ok(result) => {
                        let result_string = result.to_string_repr();
                        output.push_str(&result_string);
                        eapo_common::trace_log!(
                            "Inline expression {} evaluated to {}",
                            expression,
                            result_string
                        );
                    }
                    Err(e) => {
                        eapo_common::error_log!(
                            "Error while evaluating inline expression {}: {}",
                            expression,
                            e
                        );
                    }
                }
                expression.clear();
            }
            last_was_backslash = false;
        } else if c == '\\' {
            if i >= chars.len() - 1 || chars[i + 1] != '`' {
                if in_expression {
                    expression.push(c);
                } else {
                    output.push(c);
                }
            }
            last_was_backslash = true;
        } else {
            if in_expression {
                expression.push(c);
            } else {
                output.push(c);
            }
            last_was_backslash = false;
        }
    }

    *parameters = output;

    if command == "Eval" {
        let expression = string_helper::trim(parameters);
        match loader.eval_expression(&expression) {
            Ok(result) => {
                eapo_common::trace_log!(
                    "Expression {} evaluated to {}",
                    expression,
                    result.to_string_repr()
                );
            }
            Err(e) => {
                eapo_common::error_log!("Error while evaluating expression {}: {}", expression, e);
            }
        }
        command.clear();
    }

    Vec::new()
}

// ---------------------------------------------------------------------------
// Include
// ---------------------------------------------------------------------------

const RECURSION_LIMIT: i32 = 100;

fn include_factory(
    loader: &mut Loader,
    config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    if command == "Include" {
        let value = parameters.trim_start().to_string();
        let include_path = if Path::new(&value).is_relative() {
            resolve_relative(config_path, &value)
        } else {
            value.clone()
        };
        if loader.include_recursion_depth >= RECURSION_LIMIT {
            eapo_common::error_log!(
                "Skipping include of {} as recursion limit of {} has been reached",
                value,
                RECURSION_LIMIT
            );
        } else {
            loader.load_config_file(&include_path);
        }
        command.clear();
    }
    Vec::new()
}

fn resolve_relative(config_path: &str, value: &str) -> String {
    let path = Path::new(config_path);
    match path.parent() {
        Some(parent) => parent.join(value).to_string_lossy().into_owned(),
        None => value.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Stage
// ---------------------------------------------------------------------------

fn stage_factory(
    loader: &mut Loader,
    _config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    if command == "Stage" {
        loader.stage_matches = false;
        let stage_string = string_helper::to_lower_case(&string_helper::trim(parameters));
        let parts: Vec<&str> = stage_string.split(' ').filter(|s| !s.is_empty()).collect();
        let mut matching_part = "";
        for part in &parts {
            match *part {
                "pre-mix" => {
                    if !loader.capture && loader.pre_mix {
                        loader.stage_matches = true;
                        matching_part = part;
                    }
                }
                "post-mix" => {
                    if !loader.capture && !loader.pre_mix {
                        loader.stage_matches = true;
                        matching_part = part;
                    }
                }
                "capture" => {
                    if loader.capture {
                        loader.stage_matches = true;
                        matching_part = part;
                    }
                }
                _ => {
                    eapo_common::error_log!(
                        "Unknown stage \"{}\"! Only pre-mix, post-mix and capture are supported.",
                        part
                    );
                }
            }
        }
        if loader.stage_matches {
            eapo_common::trace_log!("Matching stage \"{}\"", matching_part);
        } else {
            eapo_common::trace_log!("Not matching stage set \"{}\"", stage_string);
        }
    }

    if !loader.stage_matches {
        command.clear();
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// Channel
// ---------------------------------------------------------------------------

fn channel_factory(
    _loader: &mut Loader,
    _config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    if command == "Channel" {
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
        return vec![adapters::make_channel(words)];
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// IIR
// ---------------------------------------------------------------------------

fn iir_factory(
    _loader: &mut Loader,
    _config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    static TYPE_RE: OnceLock<Regex> = OnceLock::new();
    static ORDER_RE: OnceLock<Regex> = OnceLock::new();
    static COEFF_RE: OnceLock<Regex> = OnceLock::new();
    if !command.starts_with("Filter") {
        return Vec::new();
    }
    let type_re = regex(&TYPE_RE, r"^\s*ON\s+([A-Za-z]+)");
    let caps = match type_re.captures(parameters) {
        Some(c) => c,
        None => return Vec::new(),
    };
    if &caps[1] != "IIR" {
        return Vec::new();
    }
    let order_re = regex(&ORDER_RE, r"\s*Order\s+([0-9]+)");
    let order = match order_re.captures(parameters) {
        Some(c) => c[1].parse::<u32>().unwrap_or(0),
        None => return Vec::new(),
    };
    if order < 1 {
        eapo_common::error_log!("Order {} not supported, must at least be 1", order);
        return Vec::new();
    }
    let coeff_re = regex(&COEFF_RE, r"\s+Coefficients((?: [-+0-9.eE]+)+)");
    let caps = match coeff_re.captures(parameters) {
        Some(c) => c,
        None => return Vec::new(),
    };
    let coefficient_strings: Vec<&str> = caps[1].split(' ').filter(|s| !s.is_empty()).collect();
    let expected = ((order + 1) * 2) as usize;
    if coefficient_strings.len() != expected {
        eapo_common::error_log!(
            "Invalid number of coefficients. Expected {} coefficients instead of {}",
            expected,
            coefficient_strings.len()
        );
        return Vec::new();
    }
    let coefficients: Vec<f64> = coefficient_strings
        .iter()
        .map(|s| parse_number(s))
        .collect();
    let mut stream = format!("Adding IIR filter of order {} with coefficients", order);
    for i in 0..=order as usize {
        stream.push_str(&format!(" b{}={}", i, coefficients[i]));
    }
    for i in 0..=order as usize {
        stream.push_str(&format!(" a{}={}", i, coefficients[i + order as usize + 1]));
    }
    eapo_common::trace_log!("{}", stream);
    vec![adapters::make_iir(coefficients)]
}

// ---------------------------------------------------------------------------
// BiQuad ("Filter: ON ...")
// ---------------------------------------------------------------------------

fn biquad_factory(
    _loader: &mut Loader,
    _config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    static TYPE_RE: OnceLock<Regex> = OnceLock::new();
    static FREQ_RE: OnceLock<Regex> = OnceLock::new();
    static GAIN_RE: OnceLock<Regex> = OnceLock::new();
    static Q_RE: OnceLock<Regex> = OnceLock::new();
    static BW_RE: OnceLock<Regex> = OnceLock::new();
    static SLOPE_RE: OnceLock<Regex> = OnceLock::new();

    if !command.starts_with("Filter") {
        return Vec::new();
    }

    *parameters = parameters.replace(',', ".");

    let type_re = regex(&TYPE_RE, r"^\s*ON\s+([A-Za-z]+)");
    let caps = match type_re.captures(parameters) {
        Some(c) => c,
        None => return Vec::new(),
    };
    let type_string = caps[1].to_string();
    let kind = match type_string.as_str() {
        "PK" | "PEQ" | "Modal" => BiquadKind::Peaking,
        "LP" | "LPQ" => BiquadKind::LowPass,
        "HP" | "HPQ" => BiquadKind::HighPass,
        "BP" => BiquadKind::BandPass,
        "LS" | "LSC" => BiquadKind::LowShelf,
        "HS" | "HSC" => BiquadKind::HighShelf,
        "NO" => BiquadKind::Notch,
        "AP" => BiquadKind::AllPass,
        _ => {
            if type_string != "None" {
                eapo_common::error_log!("Invalid filter type {}", type_string);
            }
            return Vec::new();
        }
    };
    let type_description = kind.description();
    let type_end = caps.get(0).unwrap().end();
    let params = &parameters[type_end..];
    let mut stream = format!("Adding {} filter", type_description);

    let mut freq = 0.0;
    let mut gain = 0.0;
    let mut bandwidth_or_q_or_s = 0.0;
    let mut is_bandwidth_or_s = false;
    let mut is_corner_freq = false;
    let mut error = false;

    let freq_re = regex(&FREQ_RE, r"\s+Fc\s*([-+0-9.eE\u{00A0}]+)\s*H\s*z");
    if let Some(c) = freq_re.captures(params) {
        freq = get_freq(&c[1]);
        stream.push_str(&format!(" with frequency {} Hz", freq));
    } else {
        eapo_common::error_log!(
            "No frequency given in filter string {}{}",
            type_string,
            params
        );
        error = true;
    }

    let gain_re = regex(&GAIN_RE, r"\s+Gain\s*([-+0-9.eE]+)\s*dB");
    if let Some(c) = gain_re.captures(params) {
        if matches!(
            kind,
            BiquadKind::LowPass
                | BiquadKind::HighPass
                | BiquadKind::Notch
                | BiquadKind::AllPass
        ) {
            eapo_common::trace_log!("Ignoring gain for filter of type {}", type_description);
        } else {
            gain = parse_number(&c[1]);
            if kind == BiquadKind::Peaking {
                stream.push_str(&format!(", gain {} dB", gain));
            } else {
                stream.push_str(&format!(" and gain {} dB", gain));
            }
        }
    } else if matches!(
        kind,
        BiquadKind::Peaking | BiquadKind::LowShelf | BiquadKind::HighShelf
    ) {
        eapo_common::error_log!(
            "No gain given in filter string {}{}",
            type_string,
            params
        );
        error = true;
    }

    let q_re = regex(&Q_RE, r"\s+Q\s*([-+0-9.eE]+)");
    if let Some(c) = q_re.captures(params) {
        bandwidth_or_q_or_s = parse_number(&c[1]);
        stream.push_str(&format!(" and Q {}", bandwidth_or_q_or_s));
    }

    let bw_re = regex(&BW_RE, r"\s+BW\s+Oct\s*([-+0-9.eE]+)");
    if let Some(c) = bw_re.captures(params) {
        if matches!(kind, BiquadKind::LowShelf | BiquadKind::HighShelf) {
            eapo_common::trace_log!(
                "Ignoring bandwidth for filter of type {}",
                type_description
            );
        } else {
            bandwidth_or_q_or_s = parse_number(&c[1]);
            is_bandwidth_or_s = true;
            stream.push_str(&format!(
                " and bandwidth {} octaves",
                bandwidth_or_q_or_s
            ));
        }
    }

    let slope_re = regex(&SLOPE_RE, r"^\s*([-+0-9.eE]+)\s*dB");
    if let Some(c) = slope_re.captures(params) {
        if !matches!(kind, BiquadKind::LowShelf | BiquadKind::HighShelf) {
            eapo_common::trace_log!("Ignoring slope for filter of type {}", type_description);
        } else {
            bandwidth_or_q_or_s = parse_number(&c[1]);
            is_bandwidth_or_s = true;
            stream.push_str(&format!(" and slope {} dB", bandwidth_or_q_or_s));
        }
    }

    if bandwidth_or_q_or_s == 0.0 {
        match kind {
            BiquadKind::Peaking | BiquadKind::AllPass => {
                eapo_common::error_log!(
                    "No Q or bandwidth given in filter string {}{}",
                    type_string,
                    params
                );
                error = true;
            }
            BiquadKind::LowPass | BiquadKind::HighPass | BiquadKind::BandPass => {
                bandwidth_or_q_or_s = std::f64::consts::FRAC_1_SQRT_2;
            }
            BiquadKind::LowShelf | BiquadKind::HighShelf => {
                // found out by experimentation with RoomEQWizard
                bandwidth_or_q_or_s = 0.9;
                is_bandwidth_or_s = true;
            }
            BiquadKind::Notch => {
                // found out by experimentation with RoomEQWizard
                bandwidth_or_q_or_s = 30.0;
            }
        }
    } else if matches!(kind, BiquadKind::LowShelf | BiquadKind::HighShelf) {
        if is_bandwidth_or_s {
            // Maximum S is 1 for 12 dB
            bandwidth_or_q_or_s /= 12.0;
        }
        if !type_string.ends_with('C') {
            is_corner_freq = true;
        }
    }

    if error {
        return Vec::new();
    }
    eapo_common::trace_log!("{}", stream);
    vec![adapters::make_biquad(
        kind,
        gain,
        freq,
        bandwidth_or_q_or_s,
        is_bandwidth_or_s,
        is_corner_freq,
    )]
}

fn get_freq(freq_string: &str) -> f64 {
    let s = freq_string.replace('\u{00A0}', "");
    let mut result = match parse_leading_f64(&s) {
        Some((v, _)) => v,
        None => return -1.0,
    };
    if s.len() >= 5 && !s.contains('e') && !s.contains('E') {
        let chars: Vec<char> = s.chars().collect();
        if chars.len() >= 4 && chars[chars.len() - 4] == '.' {
            // Interpret as thousands separator because of Room EQ Wizard
            result *= 1000.0;
        }
    }
    result
}

// ---------------------------------------------------------------------------
// Preamp
// ---------------------------------------------------------------------------

fn preamp_factory(
    _loader: &mut Loader,
    _config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    if command == "Preamp" {
        let value = parameters.replace(',', ".");
        if let Some((preamp_db, end)) = parse_leading_f64(&value) {
            let rest = value[end..].trim_start();
            if rest.starts_with("dB") {
                eapo_common::trace_log!("Adjusting preamp by {} dB", preamp_db);
                return vec![adapters::make_preamp(preamp_db)];
            }
        }
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// Delay
// ---------------------------------------------------------------------------

fn delay_factory(
    _loader: &mut Loader,
    _config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    if command == "Delay" {
        let value = parameters.replace(',', ".");
        if let Some((delay, end)) = parse_leading_f64(&value) {
            let rest = value[end..].trim_start();
            let unit = rest.split_whitespace().next().unwrap_or("");
            if delay >= 0.0 {
                if unit.eq_ignore_ascii_case("ms") {
                    eapo_common::trace_log!("Delaying by {} ms", delay);
                    return vec![adapters::make_delay(delay, true)];
                } else if unit.eq_ignore_ascii_case("samples") {
                    eapo_common::trace_log!("Delaying by {} samples", delay);
                    return vec![adapters::make_delay(delay, false)];
                }
            }
        }
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// Copy
// ---------------------------------------------------------------------------

fn copy_factory(
    _loader: &mut Loader,
    _config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    if command != "Copy" {
        return Vec::new();
    }
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
            let (factor, channel) = if factors.len() == 2 {
                (factors[0].clone(), factors[1].clone())
            } else if factors.len() == 1 {
                if factors[0] == "0" || factors[0].contains('.') {
                    (factors[0].clone(), String::new())
                } else {
                    (String::new(), factors[0].clone())
                }
            } else {
                continue;
            };
            let (factor_value, is_decibel) = if factor.is_empty() {
                (1.0, false)
            } else {
                let lower = factor.to_lowercase();
                let is_db = factor.chars().count() > 2 && lower.ends_with("db");
                (parse_number(&factor), is_db)
            };
            source_sum.push(CopySummand {
                factor: factor_value,
                is_decibel,
                channel,
            });
        }
        if !target.is_empty() && !source_sum.is_empty() {
            assignments.push(CopyAssignment {
                target_channel: target,
                source_sum,
            });
        }
    }
    vec![adapters::make_copy(assignments)]
}

// ---------------------------------------------------------------------------
// Convolution
// ---------------------------------------------------------------------------

fn convolution_factory(
    _loader: &mut Loader,
    config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    if command == "Convolution" {
        let value = parameters.trim_start().to_string();
        let absolute_path = if Path::new(&value).is_relative() {
            resolve_relative(config_path, &value)
        } else {
            value
        };
        return vec![adapters::make_convolution(absolute_path)];
    }
    Vec::new()
}

// ---------------------------------------------------------------------------
// GraphicEQ
// ---------------------------------------------------------------------------

fn graphic_eq_factory(
    _loader: &mut Loader,
    _config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    static NUMBER_RE: OnceLock<Regex> = OnceLock::new();
    if command != "GraphicEQ" {
        return Vec::new();
    }
    let mut value = parameters.clone();
    if !value.contains('.') {
        value = value.replace(',', ".");
    }
    let number_re = regex(&NUMBER_RE, r"[-+0-9.eE]+");
    let mut matches = number_re.find_iter(&value);
    let mut nodes = Vec::new();
    while let Some(freq_match) = matches.next() {
        if let Some(gain_match) = matches.next() {
            let freq = parse_number(freq_match.as_str());
            let gain = parse_number(gain_match.as_str());
            nodes.push(FilterNode { freq, db_gain: gain });
        }
    }
    nodes.sort_by(|a, b| a.freq.partial_cmp(&b.freq).unwrap_or(std::cmp::Ordering::Equal));
    eapo_common::trace_log!("Graphic equalizer with {} nodes", nodes.len());
    vec![adapters::make_graphic_eq(nodes, 16384)]
}

// ---------------------------------------------------------------------------
// VSTPlugin
// ---------------------------------------------------------------------------

fn vst_plugin_factory(
    _loader: &mut Loader,
    config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    if command != "VSTPlugin" {
        return Vec::new();
    }
    let mut library = None;
    let mut chunk_data = String::new();
    let mut param_map: HashMap<String, f32> = HashMap::new();
    let parts = string_helper::split_quoted(parameters, ' ', '"');

    let mut i = 0;
    while i + 1 < parts.len() {
        let key = parts[i].clone();
        let value = parts[i + 1].clone();

        if key == "Library" {
            let lib_path = if Path::new(&value).is_relative() {
                let plugin_path = default_vst_plugin_path();
                Path::new(&plugin_path)
                    .join(&value)
                    .to_string_lossy()
                    .into_owned()
            } else {
                value
            };
            library = adapters::get_vst_library(&lib_path);
        } else if key == "ChunkData" {
            chunk_data = value;
        } else {
            let first = value.chars().next().unwrap_or('\0');
            if !first.is_ascii_digit() {
                let x = i + 2;
                if x <= parts.len() {
                    if let Some(next) = parts.get(x) {
                        let f: f32 = parse_number(next) as f32;
                        param_map.insert(value.clone(), f);
                    }
                }
            } else {
                let f: f32 = parse_number(&value) as f32;
                param_map.insert(key.clone(), f);
            }
        }
        i += 2;
    }

    let Some(library) = library else {
        return Vec::new();
    };

    let mut create = true;
    if !config_path.is_empty() {
        create = false;
        eapo_common::trace_log!("Adding VST plugin {}", adapters::vst_library_path(&library));
        let res = adapters::initialize_vst_library(&library);
        if res < 0 {
            const FILE_NOT_FOUND: i32 = -1;
            const LOADING_FAILED: i32 = -2;
            const FUNCTIONS_MISSING: i32 = -3;
            const WRONG_ARCHITECTURE: i32 = -4;
            let lib_path = adapters::vst_library_path(&library);
            if res == FILE_NOT_FOUND {
                eapo_common::error_log!("File {} not found", lib_path);
            } else if res == LOADING_FAILED {
                eapo_common::error_log!("Library {} could not be loaded", lib_path);
            } else if res == FUNCTIONS_MISSING {
                eapo_common::error_log!("Library {} does not contain needed functions", lib_path);
            } else if res == WRONG_ARCHITECTURE {
                eapo_common::error_log!(
                    "Library {} has wrong architecture, must be 64-bit",
                    lib_path
                );
            }
        } else {
            create = true;
        }
    }

    if create {
        vec![adapters::make_vst(library, chunk_data, param_map)]
    } else {
        Vec::new()
    }
}

fn default_vst_plugin_path() -> String {
    static PATH: OnceLock<String> = OnceLock::new();
    PATH.get_or_init(|| {
        let install_path = registry_helper::read_value(APP_REGPATH, "InstallPath").unwrap_or_default();
        format!("{}\\VSTPlugins", install_path)
    })
    .clone()
}

// ---------------------------------------------------------------------------
// LoudnessCorrection
// ---------------------------------------------------------------------------

fn loudness_correction_factory(
    _loader: &mut Loader,
    _config_path: &str,
    command: &mut String,
    parameters: &mut String,
) -> Vec<Box<dyn IFilter>> {
    if command != "LoudnessCorrection" {
        return Vec::new();
    }
    match adapters::make_loudness(parameters) {
        Some(filter) => {
            eapo_common::trace_log!("Adding loudness correction filter");
            vec![filter]
        }
        None => Vec::new(),
    }
}

#[allow(dead_code)]
fn _unused_channel_helper() {
    let _ = channel_helper::get_default_channel_mask(2);
}
