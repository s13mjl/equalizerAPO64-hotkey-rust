//! Editor-side model of a configuration file.
//!
//! The editor intentionally has its own line parser: it mirrors the order of
//! the C++ `IFilterGUIFactory` instances so the same lines get the same GUI,
//! but it never depends on the audio engine for parsing or serialization.

use serde_json::{Map as JsonMap, Value as JsonValue};

use crate::keyseq::KeySequence;

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
    pub fn label(self) -> &'static str {
        match self {
            BiquadKind::Peaking => "Peaking filter",
            BiquadKind::LowPass => "Low-pass filter",
            BiquadKind::HighPass => "High-pass filter",
            BiquadKind::BandPass => "Band-pass filter",
            BiquadKind::LowShelf => "Low-shelf filter",
            BiquadKind::HighShelf => "High-shelf filter",
            BiquadKind::Notch => "Notch filter",
            BiquadKind::AllPass => "All-pass filter",
        }
    }

    pub fn category(self) -> u8 {
        match self {
            BiquadKind::Peaking => 0,
            BiquadKind::LowPass | BiquadKind::HighPass | BiquadKind::BandPass => 1,
            BiquadKind::LowShelf | BiquadKind::HighShelf => 2,
            BiquadKind::Notch => 3,
            BiquadKind::AllPass => 4,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FreqMode {
    Center,
    Corner,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum QMode {
    Fixed,
    Q,
    Bandwidth,
    Slope,
}

#[derive(Clone, Debug)]
pub struct BiquadEditor {
    pub kind: BiquadKind,
    pub freq: f64,
    pub gain: f64,
    pub q: f64,
    pub freq_mode: FreqMode,
    pub q_mode: QMode,
}

impl Default for BiquadEditor {
    fn default() -> Self {
        Self {
            kind: BiquadKind::Peaking,
            freq: 100.0,
            gain: 0.0,
            q: 10.0,
            freq_mode: FreqMode::Center,
            q_mode: QMode::Q,
        }
    }
}

impl BiquadEditor {
    pub fn parse(parameters: &str) -> Option<Self> {
        let mut ed = BiquadEditor::default();
        let mut toks = parameters.split_whitespace().peekable();
        // The C++ factory only recognizes ON; the filter's enable state is
        // the leading '#'.
        let first = toks.next()?;
        if !first.eq_ignore_ascii_case("on") {
            return None;
        }
        let type_tok = toks.next()?;
        let (kind, corner_token, fixed_q) = match type_tok {
            "PK" | "PEQ" | "Modal" => (BiquadKind::Peaking, false, false),
            "LP" => (BiquadKind::LowPass, false, true),
            "HP" => (BiquadKind::HighPass, false, true),
            "LPQ" => (BiquadKind::LowPass, false, false),
            "HPQ" => (BiquadKind::HighPass, false, false),
            "BP" => (BiquadKind::BandPass, false, true),
            "LS" => (BiquadKind::LowShelf, false, true),
            "HS" => (BiquadKind::HighShelf, false, true),
            "LSC" => (BiquadKind::LowShelf, true, true),
            "HSC" => (BiquadKind::HighShelf, true, true),
            "NO" => (BiquadKind::Notch, false, true),
            "AP" => (BiquadKind::AllPass, false, true),
            _ => return None,
        };
        ed.kind = kind;
        ed.freq_mode = if corner_token { FreqMode::Corner } else { FreqMode::Center };
        ed.q_mode = if fixed_q { QMode::Fixed } else { QMode::Q };
        let mut have_freq = false;
        let mut have_gain = false;
        let mut have_q = false;
        while let Some(tok) = toks.next() {
            match tok.to_ascii_lowercase().as_str() {
                "fc" => {
                    ed.freq = parse_freq_token(toks.next()?).unwrap_or(100.0);
                    have_freq = true;
                    let _ = toks.next_if(|t| t.eq_ignore_ascii_case("hz"));
                }
                "gain" => {
                    ed.gain = parse_num(toks.next()?).unwrap_or(0.0);
                    have_gain = true;
                    let _ = toks.next_if(|t| t.eq_ignore_ascii_case("db"));
                }
                "q" => {
                    ed.q = parse_num(toks.next()?).unwrap_or(1.0);
                    ed.q_mode = QMode::Q;
                    have_q = true;
                }
                "bw" => {
                    ed.q = parse_num(toks.next()?).unwrap_or(1.0);
                    ed.q_mode = QMode::Bandwidth;
                    have_q = true;
                    let _ = toks.next_if(|t| t.eq_ignore_ascii_case("oct"));
                }
                "s" => {
                    ed.q = parse_num(toks.next()?).unwrap_or(0.9);
                    ed.q_mode = QMode::Slope;
                    have_q = true;
                    let _ = toks.next_if(|t| t.eq_ignore_ascii_case("db"));
                }
                _ => {}
            }
        }
        if !have_freq {
            return None;
        }
        if matches!(ed.kind, BiquadKind::Peaking | BiquadKind::LowShelf | BiquadKind::HighShelf) && !have_gain {
            return None;
        }
        if !have_q {
            // The C++ factory rejects peaking/all-pass filters without Q/BW.
            if matches!(ed.kind, BiquadKind::Peaking | BiquadKind::AllPass) {
                return None;
            }
            ed.q_mode = match ed.kind {
                BiquadKind::LowPass | BiquadKind::HighPass | BiquadKind::BandPass => QMode::Fixed,
                BiquadKind::LowShelf | BiquadKind::HighShelf => QMode::Fixed,
                BiquadKind::Notch => QMode::Fixed,
                _ => QMode::Q,
            };
            ed.q = match ed.kind {
                BiquadKind::LowShelf | BiquadKind::HighShelf => 0.9,
                BiquadKind::Notch => 30.0,
                _ => std::f64::consts::FRAC_1_SQRT_2,
            };
        }
        Some(ed)
    }

    pub fn store(&self) -> (String, String) {
        let mut params = String::from("ON ");
        let qmode = self.q_mode;
        let type_token = match self.kind {
            BiquadKind::Peaking => "PK",
            BiquadKind::LowPass => {
                if qmode == QMode::Fixed { "LP" } else { "LPQ" }
            }
            BiquadKind::HighPass => {
                if qmode == QMode::Fixed { "HP" } else { "HPQ" }
            }
            BiquadKind::BandPass => "BP",
            BiquadKind::LowShelf => {
                if self.freq_mode == FreqMode::Corner && qmode != QMode::Fixed { "LSC" } else { "LS" }
            }
            BiquadKind::HighShelf => {
                if self.freq_mode == FreqMode::Corner && qmode != QMode::Fixed { "HSC" } else { "HS" }
            }
            BiquadKind::Notch => "NO",
            BiquadKind::AllPass => "AP",
        };
        params.push_str(type_token);
        if matches!(self.kind, BiquadKind::LowShelf | BiquadKind::HighShelf) && qmode == QMode::Slope {
            params.push_str(&format!(" {} dB", qt_num(self.q)));
        }
        params.push_str(&format!(" Fc {} Hz", qt_num(self.freq)));
        if matches!(
            self.kind,
            BiquadKind::Peaking | BiquadKind::LowShelf | BiquadKind::HighShelf
        ) {
            params.push_str(&format!(" Gain {} dB", qt_num(self.gain)));
        }
        match qmode {
            QMode::Q => params.push_str(&format!(" Q {}", qt_num(self.q))),
            QMode::Bandwidth => params.push_str(&format!(" BW Oct {}", qt_num(self.q))),
            _ => {}
        }
        ("Filter".to_string(), params)
    }

    pub fn gain(&self) -> f64 {
        self.gain
    }
}

#[derive(Clone, Debug)]
pub struct Summand {
    pub channel: String,
    pub factor: f64,
    pub is_decibel: bool,
}

impl Default for Summand {
    fn default() -> Self {
        Self { channel: " ".to_string(), factor: 1.0, is_decibel: false }
    }
}

#[derive(Clone, Debug, Default)]
pub struct Assignment {
    pub target: String,
    pub summands: Vec<Summand>,
}

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct FilterNode {
    pub freq: f64,
    pub db_gain: f64,
}

#[derive(Clone, Debug)]
pub struct GraphicEqEditor {
    pub nodes: Vec<FilterNode>,
    pub band_count: i32,
    pub selected: Vec<usize>,
    pub table_width: f32,
    pub view_height: f32,
    pub zoom_x: f64,
    pub zoom_y: f64,
    pub scroll: egui::Vec2,
}

impl Default for GraphicEqEditor {
    fn default() -> Self {
        Self {
            nodes: Vec::new(),
            band_count: -1,
            selected: Vec::new(),
            table_width: 119.0,
            view_height: 150.0,
            zoom_x: 1.0,
            zoom_y: 1.0,
            scroll: egui::vec2(290.5952109946698, 195.0),
        }
    }
}

impl GraphicEqEditor {
    pub fn bands15() -> &'static [f64] {
        &[25.0, 40.0, 63.0, 100.0, 160.0, 250.0, 400.0, 630.0, 1000.0, 1600.0, 2500.0, 4000.0, 6300.0, 10000.0, 16000.0]
    }

    pub fn bands31() -> &'static [f64] {
        &[
            20.0, 25.0, 31.5, 40.0, 50.0, 63.0, 80.0, 100.0, 125.0, 160.0, 200.0, 250.0, 315.0, 400.0,
            500.0, 630.0, 800.0, 1000.0, 1250.0, 1600.0, 2000.0, 2500.0, 3150.0, 4000.0, 5000.0, 6300.0,
            8000.0, 10000.0, 12500.0, 16000.0, 20000.0,
        ]
    }

    pub fn bands(count: i32) -> &'static [f64] {
        match count {
            15 => Self::bands15(),
            31 => Self::bands31(),
            _ => &[],
        }
    }

    pub fn parse(parameters: &str) -> Option<Self> {
        let mut ed = GraphicEqEditor::default();
        let value = if !parameters.contains('.') { parameters.replace(',', ".") } else { parameters.to_string() };
        let re = regex::Regex::new(r"[-+]?[0-9]*\.?[0-9]+([eE][-+]?[0-9]+)?").ok()?;
        let mut numbers = re.find_iter(&value).filter_map(|m| m.as_str().parse::<f64>().ok());
        while let (Some(freq), Some(db_gain)) = (numbers.next(), numbers.next()) {
            ed.nodes.push(FilterNode { freq, db_gain });
        }
        ed.sort_nodes();
        ed.band_count = ed.verify_bands();
        Some(ed)
    }

    pub fn sort_nodes(&mut self) {
        self.nodes.sort_by(|a, b| a.freq.partial_cmp(&b.freq).unwrap_or(std::cmp::Ordering::Equal));
    }

    pub fn verify_bands(&self) -> i32 {
        for n in [15usize, 31usize] {
            let bands = Self::bands(n as i32);
            if self.nodes.len() == n && self.nodes.iter().zip(bands).all(|(a, b)| (a.freq - b).abs() <= 0.1) {
                return n as i32;
            }
        }
        -1
    }

    pub fn set_band_count(&mut self, count: i32) {
        if count == self.band_count {
            return;
        }
        let bands = Self::bands(count);
        if bands.is_empty() {
            self.band_count = count;
            return;
        }
        let mut iter = eaph::GainIterator::new(
            self.nodes.iter().map(|n| eaph::FilterNode { freq: n.freq, db_gain: n.db_gain }).collect(),
        );
        self.nodes = bands
            .iter()
            .map(|&band| FilterNode {
                freq: band,
                db_gain: (iter.gain_at(band) * 100.0).round() / 100.0,
            })
            .collect();
        self.selected.clear();
        self.band_count = count;
    }

    pub fn store(&self) -> (String, String) {
        let parameters = self
            .nodes
            .iter()
            .map(|n| format!("{} {}", qt_num(n.freq), qt_num(n.db_gain)))
            .collect::<Vec<_>>()
            .join("; ");
        ("GraphicEQ".to_string(), parameters)
    }
}

// Alias module to keep the intuitive name at call sites.
mod eaph {
    pub use eapo_common::gain_iterator::{FilterNode, GainIterator};
}

#[derive(Clone, Debug, Default)]
pub struct ChannelDialogState {
    pub selected: Vec<String>,
    pub extras: Vec<(String, bool)>,
    pub all: bool,
}

#[derive(Clone, Debug, Default)]
pub struct DeviceDialogState {
    pub all: bool,
    pub show_installed: bool,
    pub checked: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct VstEditor {
    pub path: String,
    pub chunk_data: String,
    pub params: Vec<(String, f64)>,
    pub embed: bool,
    pub auto_apply: bool,
    pub status: String,
}

#[derive(Clone, Debug)]
pub enum FilterEditor {
    Preamp { db: f64 },
    Biquad(BiquadEditor),
    Delay { delay: f64, is_ms: bool },
    Channel { selected: Vec<String>, channel_names: Vec<String>, channel_mask: u32, dialog: Option<ChannelDialogState> },
    Copy { assignments: Vec<Assignment>, channel_names: Vec<String>, tab_index: usize, height: f32 },
    GraphicEq(GraphicEqEditor),
    Convolution { path: String, device_sample_rate: u32 },
    Vst(VstEditor),
    Loudness { state: bool, ref_level: i32, ref_offset: i32, att: f64, volume: f64, calibrating: bool, measured: i32 },
    Include { path: String },
    Device { pattern: String, dialog: Option<DeviceDialogState> },
    Stage { pre_mix: bool, post_mix: bool, capture: bool },
}

impl FilterEditor {
    pub fn title(&self) -> &'static str {
        match self {
            FilterEditor::Preamp { .. } => "Preamp",
            FilterEditor::Biquad(_) => "Filter",
            FilterEditor::Delay { .. } => "Delay",
            FilterEditor::Channel { .. } => "Channel",
            FilterEditor::Copy { .. } => "Copy",
            FilterEditor::GraphicEq(_) => "GraphicEQ",
            FilterEditor::Convolution { .. } => "Convolution",
            FilterEditor::Vst(_) => "VSTPlugin",
            FilterEditor::Loudness { .. } => "LoudnessCorrection",
            FilterEditor::Include { .. } => "Include",
            FilterEditor::Device { .. } => "Device",
            FilterEditor::Stage { .. } => "Stage",
        }
    }

    pub fn store(&self) -> (String, String) {
        match self {
            FilterEditor::Preamp { db } => ("Preamp".into(), format!("{} dB", qt_num(*db))),
            FilterEditor::Biquad(b) => b.store(),
            FilterEditor::Delay { delay, is_ms } => (
                "Delay".into(),
                format!("{} {}", qt_num(*delay), if *is_ms { "ms" } else { "samples" }),
            ),
            FilterEditor::Channel { selected, channel_names, .. } => {
                let mut ordered: Vec<String> = Vec::new();
                for name in channel_names {
                    if let Some(s) = selected.iter().find(|s| s.eq_ignore_ascii_case(name)) {
                        ordered.push(s.to_uppercase());
                    }
                }
                if selected.iter().any(|s| s == "ALL") {
                    ordered.push("ALL".into());
                }
                for s in selected {
                    if !ordered.iter().any(|o| o.eq_ignore_ascii_case(s)) {
                        ordered.push(s.to_uppercase());
                    }
                }
                ("Channel".into(), ordered.join(" "))
            }
            FilterEditor::Copy { assignments, .. } => ("Copy".into(), store_assignments(assignments)),
            FilterEditor::GraphicEq(g) => g.store(),
            FilterEditor::Convolution { path, .. } => ("Convolution".into(), path.clone()),
            FilterEditor::Vst(v) => {
                let mut params = format!("Library {}", quote_if_space(&vst_config_path(&v.path)));
                if !v.chunk_data.is_empty() {
                    params.push_str(&format!(" ChunkData \"{}\"", v.chunk_data));
                } else {
                    for (name, value) in &v.params {
                        let name = if name.contains(' ') || name.contains('"') {
                            format!("\"{}\"", name.replace('"', "\"\""))
                        } else {
                            name.clone()
                        };
                        params.push_str(&format!(" {} {}", name, qt_num(*value)));
                    }
                }
                ("VSTPlugin".into(), params)
            }
            FilterEditor::Loudness { state, ref_level, ref_offset, att, .. } => (
                "LoudnessCorrection".into(),
                format!(
                    "State {} ReferenceLevel {} ReferenceOffset {} Attenuation {}",
                    if *state { 1 } else { 0 },
                    ref_level,
                    ref_offset,
                    if *att == 0.0 || *att == 1.0 { format!("{:.1}", att) } else { qt_num(*att) }
                ),
            ),
            FilterEditor::Include { path } => ("Include".into(), path.clone()),
            FilterEditor::Device { pattern, .. } => ("Device".into(), pattern.clone()),
            FilterEditor::Stage { pre_mix, post_mix, capture } => {
                let mut list = Vec::new();
                if *pre_mix { list.push("pre-mix"); }
                if *post_mix { list.push("post-mix"); }
                if *capture { list.push("capture"); }
                ("Stage".into(), list.join(" "))
            }
        }
    }

    pub fn configure_channels(&mut self, names: &[String], mask: u32) {
        match self {
            FilterEditor::Channel { channel_names, channel_mask, selected, .. } => {
                *channel_names = names.to_vec();
                *channel_mask = mask;
                let _ = selected;
            }
            FilterEditor::Copy { channel_names, assignments, .. } => {
                *channel_names = names.to_vec();
                for a in assignments {
                    for s in &mut a.summands {
                        // Channels may be free-form names; nothing to clamp.
                        let _ = &s.channel;
                    }
                }
            }
            _ => {}
        }
    }

    pub fn preferences(&self) -> JsonMap<String, JsonValue> {
        let mut m = JsonMap::new();
        match self {
            FilterEditor::Copy { tab_index, height, .. } => {
                if *height != 88.0 { m.insert("height".into(), JsonValue::from(*height as f64)); }
                if *tab_index != 0 { m.insert("tabIndex".into(), JsonValue::from(*tab_index as i64)); }
            }
            FilterEditor::GraphicEq(g) => {
                if g.table_width != 119.0 { m.insert("tableWidth".into(), JsonValue::from(g.table_width as f64)); }
                if g.view_height != 150.0 { m.insert("viewHeight".into(), JsonValue::from(g.view_height as f64)); }
                if g.zoom_x != 1.0 { m.insert("zoomX".into(), JsonValue::from(g.zoom_x)); }
                if g.zoom_y != 1.0 { m.insert("zoomY".into(), JsonValue::from(g.zoom_y)); }
                if g.scroll.x != 0.0 { m.insert("scrollX".into(), JsonValue::from(g.scroll.x as f64)); }
                if g.scroll.y != 0.0 { m.insert("scrollY".into(), JsonValue::from(g.scroll.y as f64)); }
            }
            FilterEditor::Vst(v) => {
                m.insert("embed".into(), JsonValue::Bool(v.embed));
                m.insert("autoApplyDialog".into(), JsonValue::Bool(v.auto_apply));
            }
            _ => {}
        }
        m
    }

    pub fn apply_preferences(&mut self, prefs: &JsonMap<String, JsonValue>) {
        match self {
            FilterEditor::Copy { tab_index, height, .. } => {
                if let Some(v) = prefs.get("height").and_then(|v| v.as_f64()) { *height = v as f32; }
                if let Some(v) = prefs.get("tabIndex").and_then(|v| v.as_i64()) { *tab_index = v as usize; }
            }
            FilterEditor::GraphicEq(g) => {
                if let Some(v) = prefs.get("tableWidth").and_then(|v| v.as_f64()) { g.table_width = v as f32; }
                if let Some(v) = prefs.get("viewHeight").and_then(|v| v.as_f64()) { g.view_height = v as f32; }
                if let Some(v) = prefs.get("zoomX").and_then(|v| v.as_f64()) { g.zoom_x = v; }
                if let Some(v) = prefs.get("zoomY").and_then(|v| v.as_f64()) { g.zoom_y = v; }
                let sx = prefs
                    .get("scrollX")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(290.5952109946698 * g.zoom_x) as f32;
                let sy = prefs
                    .get("scrollY")
                    .and_then(|v| v.as_f64())
                    .unwrap_or(195.0 * g.zoom_y) as f32;
                g.scroll = egui::vec2(sx, sy);
            }
            FilterEditor::Vst(v) => {
                v.embed = prefs.get("embed").and_then(|v| v.as_bool()).unwrap_or(false);
                v.auto_apply = prefs.get("autoApplyDialog").and_then(|v| v.as_bool()).unwrap_or(false);
            }
            _ => {}
        }
    }
}

pub struct FilterContext<'a> {
    pub config_path: &'a str,
    pub channel_names: &'a [String],
    pub channel_mask: u32,
    pub device_sample_rate: u32,
}

pub fn parse_editor(parameters: &str, ctx: &FilterContext<'_>) -> Option<FilterEditor> {
    parse_editor_keyed("", parameters, ctx)
}

fn parse_editor_keyed(command: &str, parameters: &str, ctx: &FilterContext<'_>) -> Option<FilterEditor> {
    // ExpressionFilterGUIFactory: any expression disables the GUI entirely.
    if parameters.contains('`') {
        return None;
    }
    match command {
        "Include" => Some(FilterEditor::Include { path: parameters.trim().to_string() }),
        "Device" => Some(FilterEditor::Device { pattern: parameters.to_string(), dialog: None }),
        "Channel" => {
            let selected: Vec<String> = parameters
                .split_whitespace()
                .filter(|s| !s.is_empty())
                .map(|s| {
                    let upper = s.to_uppercase();
                    if upper == "SUB" {
                        return "LFE".to_string();
                    }
                    if let Ok(index) = upper.parse::<usize>() {
                        if index >= 1 && index <= ctx.channel_names.len() {
                            return ctx.channel_names[index - 1].to_uppercase();
                        }
                    }
                    upper
                })
                .collect();
            Some(FilterEditor::Channel {
                selected,
                channel_names: ctx.channel_names.to_vec(),
                channel_mask: ctx.channel_mask,
                dialog: None,
            })
        }
        "Stage" => {
            let parts: Vec<String> = parameters.split_whitespace().map(|s| s.to_lowercase()).collect();
            Some(FilterEditor::Stage {
                pre_mix: parts.iter().any(|s| s == "pre-mix"),
                post_mix: parts.iter().any(|s| s == "post-mix"),
                capture: parts.iter().any(|s| s == "capture"),
            })
        }
        "Preamp" => {
            let db = parameters.split_whitespace().next().and_then(parse_num)?;
            Some(FilterEditor::Preamp { db })
        }
        _ if command.starts_with("Filter") => BiquadEditor::parse(parameters).map(FilterEditor::Biquad),
        "Delay" => {
            let mut it = parameters.split_whitespace();
            let delay = parse_num(it.next()?)?;
            if delay < 0.0 {
                return None;
            }
            let unit = it.next()?;
            let is_ms = if unit.eq_ignore_ascii_case("ms") {
                true
            } else if unit.eq_ignore_ascii_case("samples") {
                false
            } else {
                return None;
            };
            Some(FilterEditor::Delay { delay, is_ms })
        }
        "Copy" => Some(FilterEditor::Copy {
            assignments: parse_assignments(parameters),
            channel_names: ctx.channel_names.to_vec(),
            tab_index: 0,
            height: 88.0,
        }),
        "GraphicEQ" => GraphicEqEditor::parse(parameters).map(FilterEditor::GraphicEq),
        "Convolution" => Some(FilterEditor::Convolution {
            path: parameters.trim().to_string(),
            device_sample_rate: ctx.device_sample_rate,
        }),
        "VSTPlugin" => Some(FilterEditor::Vst(parse_vst(parameters))),
        "LoudnessCorrection" => Some(parse_loudness(parameters)),
        _ => None,
    }
}

fn parse_loudness(parameters: &str) -> FilterEditor {
    let mut state = true;
    let mut ref_level = 0;
    let mut ref_offset = 0;
    let mut att = 1.0;
    let toks: Vec<&str> = parameters.split_whitespace().collect();
    let mut i = 0;
    while i + 1 < toks.len() {
        match toks[i].to_ascii_lowercase().as_str() {
            "state" => state = toks[i + 1] != "0",
            "referencelevel" => ref_level = parse_num(toks[i + 1]).unwrap_or(0.0) as i32,
            "referenceoffset" => ref_offset = parse_num(toks[i + 1]).unwrap_or(0.0) as i32,
            "attenuation" => att = parse_num(toks[i + 1]).unwrap_or(1.0),
            _ => {}
        }
        i += 2;
    }
    FilterEditor::Loudness { state, ref_level, ref_offset, att, volume: 0.0, calibrating: false, measured: 75 }
}

fn parse_vst(parameters: &str) -> VstEditor {
    let mut v = VstEditor::default();
    let toks = tokenize_quoted(parameters);
    let mut i = 0;
    while i < toks.len() {
        match toks[i].as_str() {
            "Library" if i + 1 < toks.len() => {
                v.path = toks[i + 1].clone();
                i += 2;
                continue;
            }
            "ChunkData" if i + 1 < toks.len() => {
                v.chunk_data = toks[i + 1].clone();
                i += 2;
                continue;
            }
            name => {
                if i + 1 < toks.len() {
                    if let Some(value) = parse_num(&toks[i + 1]) {
                        v.params.push((name.to_string(), value));
                        i += 2;
                        continue;
                    }
                }
            }
        }
        i += 1;
    }
    v.status = if v.path.is_empty() { "No file selected.".into() } else { String::new() };
    v
}

fn tokenize_quoted(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut quoted = false;
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if quoted {
            if c == '"' {
                if chars.peek() == Some(&'"') {
                    chars.next();
                    cur.push('"');
                } else {
                    quoted = false;
                    out.push(std::mem::take(&mut cur));
                }
            } else {
                cur.push(c);
            }
        } else if c == '"' {
            quoted = true;
        } else if c.is_whitespace() {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(c);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

pub fn vst_config_path(path: &str) -> String {
    let p = std::path::Path::new(path);
    if !p.is_absolute() {
        return path.to_string();
    }
    let default = eapo_filters::vst_plugin_library::VSTPluginLibrary::get_default_plugin_path();
    if default.is_empty() {
        return path.to_string();
    }
    let base = std::path::Path::new(&default);
    let base_components: Vec<std::path::Component> = base.components().collect();
    let target_components: Vec<std::path::Component> = p.components().collect();
    let mut common = 0;
    while common < base_components.len()
        && common < target_components.len()
        && base_components[common] == target_components[common]
    {
        common += 1;
    }
    let mut parts: Vec<String> = Vec::new();
    for _ in common..base_components.len() {
        parts.push("..".into());
    }
    for c in &target_components[common..] {
        parts.push(c.as_os_str().to_string_lossy().into_owned());
    }
    let rel = parts.join("\\");
    if rel.starts_with("..\\..\\") {
        path.to_string()
    } else {
        rel
    }
}

fn quote_if_space(path: &str) -> String {
    if path.contains(' ') {
        format!("\"{path}\"")
    } else {
        path.to_string()
    }
}

pub fn parse_assignments(parameters: &str) -> Vec<Assignment> {
    let mut out = Vec::new();
    for tok in parameters.split_whitespace() {
        let Some((target, rhs)) = tok.split_once('=') else { continue };
        let target = target.trim();
        if target.is_empty() {
            continue;
        }
        let mut a = Assignment { target: target.to_string(), summands: Vec::new() };
        for part in rhs.split('+') {
            if part.trim().is_empty() {
                continue;
            }
            a.summands.push(parse_summand(part));
        }
        if a.summands.is_empty() {
            a.summands.push(Summand::default());
        }
        out.push(a);
    }
    out
}

fn parse_summand(text: &str) -> Summand {
    let mut s = Summand { channel: String::new(), factor: 1.0, is_decibel: false };
    let text = text.trim();
    if let Some((left, right)) = text.split_once('*') {
        s.channel = right.trim().to_string();
        let mut left = left.trim().to_string();
        if left.to_ascii_lowercase().ends_with("db") {
            left.truncate(left.len() - 2);
            s.is_decibel = true;
        }
        s.factor = parse_num(&left).unwrap_or(1.0);
    } else if text == "0" || text.contains('.') || text.contains('e') || text.contains('E') {
        s.factor = parse_num(text).unwrap_or(0.0);
        s.channel = String::new();
    } else {
        s.channel = text.to_string();
    }
    s
}

pub fn store_assignments(assignments: &[Assignment]) -> String {
    let mut parameters = String::new();
    let mut first_assignment = true;
    for assignment in assignments {
        let target = assignment.target.trim();
        if target.is_empty() {
            continue;
        }
        let mut first_summand = true;
        for summand in &assignment.summands {
            if summand.channel == " " {
                continue;
            }
            if first_summand {
                first_summand = false;
                if first_assignment {
                    first_assignment = false;
                } else {
                    parameters.push(' ');
                }
                parameters.push_str(target);
                parameters.push('=');
            } else {
                parameters.push('+');
            }
            let channel = summand.channel.trim();
            let has_channel = !channel.is_empty();
            let has_factor = !has_channel || summand.factor != 1.0 || summand.is_decibel;
            if has_factor {
                let mut f = qt_num(summand.factor);
                if f != "0" && !f.contains('.') && !f.contains('e') && !f.contains('E') {
                    f.push_str(".0");
                }
                parameters.push_str(&f);
                if summand.is_decibel {
                    parameters.push_str("dB");
                }
            }
            if has_factor && has_channel {
                parameters.push('*');
            }
            if has_channel {
                parameters.push_str(channel);
            }
        }
    }
    parameters
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FilterKind {
    Include,
    Device,
    Channel,
    Stage,
    Preamp,
    Biquad,
    Delay,
    Copy,
    GraphicEq,
    Convolution,
    Vst,
    Loudness,
    Unknown,
}

#[derive(Clone)]
pub struct FilterItem {
    pub uid: u64,
    pub filter_id: String,
    pub line: String,
    pub indent: String,
    pub enabled: bool,
    pub prefs: JsonMap<String, JsonValue>,
    pub hotkey: Option<KeySequence>,
    pub editor: Option<FilterEditor>,
    pub kind: FilterKind,
}

impl FilterItem {
    pub fn new(uid: u64, line: impl Into<String>, ctx: &FilterContext<'_>) -> Self {
        let mut item = Self {
            uid,
            filter_id: format!("{uid:x}"),
            line: line.into(),
            indent: String::new(),
            enabled: true,
            prefs: JsonMap::new(),
            hotkey: None,
            editor: None,
            kind: FilterKind::Unknown,
        };
        item.rebuild(ctx);
        item
    }

    pub fn command_prefix(&self) -> String {
        match self.line.find(':') {
            Some(pos) => self.line[..pos].trim().to_string(),
            None => self.line.trim().to_string(),
        }
    }

    pub fn structural(&self) -> StructuralLine<'_> {
        parse_line(&self.line)
    }

    pub fn rebuild(&mut self, ctx: &FilterContext<'_>) {
        let s = parse_line(&self.line);
        self.indent = s.indent.to_string();
        self.enabled = s.enabled;
        self.kind = s.kind;
        self.editor = if s.key.is_empty() {
            None
        } else {
            parse_editor_keyed(&s.key, s.parameters, ctx)
        };
        if let Some(ed) = &mut self.editor {
            ed.apply_preferences(&self.prefs);
            ed.configure_channels(ctx.channel_names, ctx.channel_mask);
        }
    }

    /// C++ `FilterTableRow::updateModel` plus the `CommentFilterGUI` prefix.
    pub fn sync_from_editor(&mut self) -> bool {
        let Some(ed) = &self.editor else { return false };
        let (command, parameters) = ed.store();
        let line = compose_line(&self.indent, self.enabled, &command, &parameters);
        if line == self.line {
            return false;
        }
        self.line = line;
        true
    }

    pub fn set_enabled(&mut self, enabled: bool) {
        if self.enabled == enabled {
            return;
        }
        self.enabled = enabled;
        let indent_len = self.indent.len();
        let body = &self.line[indent_len..];
        if enabled {
            if let Some(rest) = body.strip_prefix('#') {
                let rest = rest.strip_prefix(' ').unwrap_or(rest);
                self.line = format!("{}{}", self.indent, rest);
            }
        } else {
            self.line = format!("{}# {}", self.indent, body);
        }
    }

    pub fn toggle_enabled(&mut self) {
        self.set_enabled(!self.enabled);
    }

    pub fn collect_prefs(&self) -> JsonMap<String, JsonValue> {
        let mut prefs = self.prefs.clone();
        if let Some(ed) = &self.editor {
            for (k, v) in ed.preferences() {
                prefs.insert(k, v);
            }
        }
        prefs.remove("hotkey");
        if let Some(hk) = self.hotkey {
            prefs.insert("hotkey".into(), JsonValue::String(hk.to_string_qt()));
        }
        prefs
    }

    pub fn load_hotkey_from_prefs(&mut self) {
        self.hotkey = self
            .prefs
            .get("hotkey")
            .and_then(|v| v.as_str())
            .and_then(KeySequence::parse);
    }
}

#[derive(Clone, Debug)]
pub struct StructuralLine<'a> {
    pub indent: &'a str,
    pub enabled: bool,
    pub key: String,
    pub parameters: &'a str,
    pub kind: FilterKind,
}

pub fn parse_line(line: &str) -> StructuralLine<'_> {
    let indent_len = line.len() - line.trim_start_matches([' ', '\t']).len();
    let indent = &line[..indent_len];
    let mut body = &line[indent_len..];
    let mut enabled = true;
    if let Some(rest) = body.strip_prefix('#') {
        enabled = false;
        body = rest.strip_prefix(' ').unwrap_or(rest);
    }
    if let Some(colon) = body.find(':') {
        let key = body[..colon].trim().to_string();
        let parameters = &body[colon + 1..];
        let kind = classify(&key, parameters);
        StructuralLine { indent, enabled, key, parameters, kind }
    } else {
        StructuralLine { indent, enabled, key: String::new(), parameters: "", kind: FilterKind::Unknown }
    }
}

fn classify(key: &str, parameters: &str) -> FilterKind {
    if parameters.contains('`') || key.is_empty() {
        return FilterKind::Unknown;
    }
    match key {
        "Include" => FilterKind::Include,
        "Device" => FilterKind::Device,
        "Channel" => FilterKind::Channel,
        "Stage" => FilterKind::Stage,
        "Preamp" => FilterKind::Preamp,
        k if k.starts_with("Filter") => {
            if BiquadEditor::parse(parameters).is_some() { FilterKind::Biquad } else { FilterKind::Unknown }
        }
        "Delay" => FilterKind::Delay,
        "Copy" => FilterKind::Copy,
        "GraphicEQ" => FilterKind::GraphicEq,
        "Convolution" => FilterKind::Convolution,
        "VSTPlugin" => FilterKind::Vst,
        "LoudnessCorrection" => FilterKind::Loudness,
        _ => FilterKind::Unknown,
    }
}

pub fn compose_line(indent: &str, enabled: bool, command: &str, parameters: &str) -> String {
    format!(
        "{}{}{}: {}",
        indent,
        if enabled { "" } else { "# " },
        command,
        parameters
    )
}

/// C++ `QString::arg(double)` default formatting is `%g` with 6 significant
/// digits.  Reproducing it keeps rewritten lines close to the original text.
pub fn qt_num(value: f64) -> String {
    if value.is_nan() {
        return "nan".into();
    }
    if value.is_infinite() {
        return if value < 0.0 { "-inf".into() } else { "inf".into() };
    }
    if value == 0.0 {
        return "0".into();
    }
    let abs = value.abs();
    let exp = abs.log10().floor() as i32;
    let mut s = if !(-4..6).contains(&exp) {
        let mantissa = value / 10f64.powi(exp);
        let mut m = format!("{mantissa:.5}");
        trim_zeros(&mut m);
        let sign = if exp < 0 { '-' } else { '+' };
        format!("{m}e{}{:02}", sign, exp.abs())
    } else {
        let decimals = (5 - exp).max(0) as usize;
        let mut s = format!("{value:.decimals$}");
        trim_zeros(&mut s);
        s
    };
    if s == "-0" {
        s = "0".into();
    }
    s
}

fn trim_zeros(s: &mut String) {
    if s.contains('.') {
        while s.ends_with('0') {
            s.pop();
        }
        if s.ends_with('.') {
            s.pop();
        }
    }
}

fn parse_freq_token(s: &str) -> Option<f64> {
    let cleaned: String = s
        .trim()
        .replace('\u{a0}', "")
        .replace(',', ".")
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'))
        .collect();
    let mut value: f64 = cleaned.parse().ok()?;
    if cleaned.len() >= 5 && !cleaned.contains(['e', 'E']) && cleaned.as_bytes()[cleaned.len() - 4] == b'.' {
        // Room EQ Wizard writes e.g. "1.000" for 1000 Hz.
        value *= 1000.0;
    }
    Some(value)
}

pub fn parse_num(s: &str) -> Option<f64> {
    let cleaned: String = s
        .trim()
        .replace('\u{a0}', "")
        .replace(',', ".")
        .chars()
        .take_while(|c| c.is_ascii_digit() || matches!(c, '+' | '-' | '.' | 'e' | 'E'))
        .collect();
    if cleaned.is_empty() {
        return None;
    }
    cleaned.parse().ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx() -> (Vec<String>, FilterContext<'static>) {
        let names = vec!["L".to_string(), "R".to_string()];
        (names, FilterContext { config_path: "C:\\config.txt", channel_names: &[], channel_mask: 3, device_sample_rate: 48000 })
    }

    fn item(line: &str) -> FilterItem {
        let (names, _) = ctx();
        let c = FilterContext { config_path: "", channel_names: &names, channel_mask: 3, device_sample_rate: 48000 };
        FilterItem::new(1, line, &c)
    }

    /// A leading `#` is the Editor's "row is switched off" marker
    /// (`CommentFilterGUI::store` prefixes it while the power button is off), not
    /// a lossy comment: the row keeps its real filter type, so switching the
    /// power on writes the active line back without retyping anything.
    #[test]
    fn commented_rows_keep_their_type_and_enable_cleanly() {
        let mut preamp = item("# Preamp: 50 dB");
        assert_eq!(preamp.kind, FilterKind::Preamp);
        assert!(!preamp.enabled);
        assert_eq!(preamp.line, "# Preamp: 50 dB");

        preamp.set_enabled(true);
        assert_eq!(preamp.line, "Preamp: 50 dB");

        preamp.set_enabled(false);
        assert_eq!(preamp.line, "# Preamp: 50 dB");

        let mut active = item("Preamp: -6 dB");
        assert_eq!(active.kind, FilterKind::Preamp);
        assert!(active.enabled);
        active.set_enabled(false);
        assert_eq!(active.line, "# Preamp: -6 dB");
    }

    #[test]
    fn classifies_filter_guis() {
        assert_eq!(item("Preamp: -6 dB").kind, FilterKind::Preamp);
        assert_eq!(item("Filter: ON PK Fc 100 Hz Gain 3 dB Q 1").kind, FilterKind::Biquad);
        assert_eq!(item("Delay: 1.5 ms").kind, FilterKind::Delay);
        assert_eq!(item("GraphicEQ: 25 0; 40 -1").kind, FilterKind::GraphicEq);
        assert_eq!(item("Copy: L=R R=L").kind, FilterKind::Copy);
        assert_eq!(item("Stage: post-mix").kind, FilterKind::Stage);
        assert_eq!(item("Channel: L R").kind, FilterKind::Channel);
        assert_eq!(item("Device: all").kind, FilterKind::Device);
        assert_eq!(item("Include: other.txt").kind, FilterKind::Include);
        assert_eq!(item("Convolution: ir.wav").kind, FilterKind::Convolution);
        assert_eq!(item("VSTPlugin: Library C:\\x.dll").kind, FilterKind::Vst);
        assert_eq!(item("LoudnessCorrection: State 1 ReferenceLevel 0 ReferenceOffset 0 Attenuation 1.0").kind, FilterKind::Loudness);
        assert_eq!(item("Unknown: x").kind, FilterKind::Unknown);
        assert_eq!(item("# a comment").kind, FilterKind::Unknown);
        // Inline expressions keep the raw text editor.
        assert_eq!(item("Preamp: `-6 * 2` dB").kind, FilterKind::Unknown);
    }

    #[test]
    fn enabled_toggle_preserves_indent_and_text() {
        let mut it = item("  # Filter: ON PK Fc 100 Hz Gain 3 dB Q 1");
        assert!(!it.enabled);
        it.toggle_enabled();
        assert!(it.enabled);
        assert_eq!(it.line, "  Filter: ON PK Fc 100 Hz Gain 3 dB Q 1");
        it.toggle_enabled();
        assert_eq!(it.line, "  # Filter: ON PK Fc 100 Hz Gain 3 dB Q 1");
    }

    #[test]
    fn biquad_round_trip() {
        let ed = BiquadEditor::parse("ON PK Fc 100 Hz Gain 3 dB Q 1.5").unwrap();
        assert_eq!(ed.store(), ("Filter".into(), "ON PK Fc 100 Hz Gain 3 dB Q 1.5".into()));
        let ed = BiquadEditor::parse("ON LP Fc 1200 Hz").unwrap();
        assert_eq!(ed.store(), ("Filter".into(), "ON LP Fc 1200 Hz".into()));
        let ed = BiquadEditor::parse("ON LS Fc 100 Hz Gain -2 dB S 6 dB").unwrap();
        assert_eq!(ed.store(), ("Filter".into(), "ON LS 6 dB Fc 100 Hz Gain -2 dB".into()));
    }

    #[test]
    fn copy_round_trip() {
        let p = "L=R L=0.5*R+0.5*L R=2.0dB*L";
        let (cmd, out) = FilterEditor::Copy {
            assignments: parse_assignments(p),
            channel_names: vec![],
            tab_index: 0,
            height: 88.0,
        }
        .store();
        assert_eq!(cmd, "Copy");
        assert_eq!(out, "L=R L=0.5*R+0.5*L R=2.0dB*L");
    }

    #[test]
    fn graphic_eq_round_trip() {
        let ed = GraphicEqEditor::parse("25 0; 40 -1.5; 63 2").unwrap();
        assert_eq!(ed.store().1, "25 0; 40 -1.5; 63 2");
        assert_eq!(ed.band_count, -1);
    }

    #[test]
    fn compose_matches_cpp_spacing() {
        assert_eq!(compose_line("  ", false, "Preamp", "-6 dB"), "  # Preamp: -6 dB");
        assert_eq!(compose_line("", true, "Copy", ""), "Copy: ");
    }

    #[test]
    fn qt_number_formatting() {
        assert_eq!(qt_num(0.0), "0");
        assert_eq!(qt_num(-6.0), "-6");
        assert_eq!(qt_num(1.25), "1.25");
        assert_eq!(qt_num(0.3333), "0.3333");
        assert_eq!(qt_num(1000000.0), "1e+06");
        assert_eq!(qt_num(0.00001234), "1.234e-05");
    }

    #[test]
    fn prefs_hotkey_round_trip() {
        let mut it = item("Preamp: -6 dB");
        it.hotkey = Some(KeySequence::parse("Ctrl+Alt+X").unwrap());
        let prefs = it.collect_prefs();
        assert_eq!(prefs.get("hotkey").unwrap().as_str().unwrap(), "Ctrl+Alt+X");
        let mut it2 = item("Preamp: -6 dB");
        it2.prefs = prefs;
        it2.load_hotkey_from_prefs();
        assert_eq!(it2.hotkey.unwrap().to_string_qt(), "Ctrl+Alt+X");
    }
}
