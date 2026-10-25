//! Filter GUIs, ported from `Editor/guis/*`.  All controls, labels and
//! defaults are present; the layout is egui-native instead of pixel-identical
//! to the Qt forms.

use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

use eapo_apo_info::{AbstractAPOInfo, VoicemeeterAPOInfo};
use eapo_common::channel_helper;
use egui::{Color32, Sense, Stroke, Vec2};

use crate::model::*;
use crate::plot;

pub struct GuiServices<'a> {
    pub all_devices: &'a [Arc<dyn AbstractAPOInfo>],
    pub config_path: &'a str,
    pub master_volume: Option<f64>,
}

#[derive(Default)]
pub struct EditorActions {
    pub changed: bool,
    pub open_config: Option<String>,
    pub update_analysis: bool,
}

fn audio_service_readable(path: &Path) -> Option<bool> {
    // SECURITY_LOCAL_SERVICE_RID == 19; the APO runs as LOCAL SERVICE.
    let mask =
        eapo_common::registry_helper::get_file_access_for_user(&path.to_string_lossy(), 19).ok()?;
    let generic_read = windows::Win32::Foundation::GENERIC_READ.0;
    let file_generic_read = windows::Win32::Storage::FileSystem::FILE_GENERIC_READ.0;
    Some(
        mask & generic_read == generic_read || mask & file_generic_read == file_generic_read,
    )
}

fn probe_cached(path: &Path) -> Result<(u32, u64), String> {
    use std::collections::HashMap;
    use std::sync::{Mutex, OnceLock};
    type ProbeResult = Result<(u32, u64), String>;
    static CACHE: OnceLock<Mutex<HashMap<(std::path::PathBuf, u64, u64), ProbeResult>>> =
        OnceLock::new();
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    let modified = metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
        .map(|duration| duration.as_nanos() as u64)
        .unwrap_or(0);
    let key = (path.to_path_buf(), modified, metadata.len());
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    let mut cache = cache
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner());
    if let Some(result) = cache.get(&key) {
        return result.clone();
    }
    let result = eapo_filters::audio_file::probe_audio_file(path);
    cache.insert(key, result.clone());
    result
}

pub fn show_editor(
    ui: &mut egui::Ui,
    ed: &mut FilterEditor,
    services: &GuiServices<'_>,
    actions: &mut EditorActions,
) {
    match ed {
        FilterEditor::Preamp { db } => {
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("Preamplification:"));
                // The dial in the Qt GUI is limited to ±20 dB; values outside
                // that range must be preserved, not clamped in the model.
                let mut dial = db.clamp(-20.0, 20.0);
                let r = ui.add(
                    egui::Slider::new(&mut dial, -20.0..=20.0)
                        .suffix(" dB")
                        .fixed_decimals(1),
                );
                if r.changed() {
                    *db = dial;
                }
                ui.label(crate::i18n::tr("Gain:"));
                let r2 = ui.add(
                    egui::DragValue::new(db)
                        .speed(0.1)
                        .range(-100.0..=100.0)
                        .suffix(" dB")
                        .fixed_decimals(1),
                );
                actions.changed |= r.changed() || r2.changed();
            });
        }
        FilterEditor::Biquad(b) => biquad_ui(ui, b, actions),
        FilterEditor::Delay { delay, is_ms } => {
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("Delay:"));
                let r = ui.add(
                    egui::DragValue::new(delay)
                        .speed(if *is_ms { 0.1 } else { 1.0 })
                        .range(0.0..=1000000.0)
                        .suffix(if *is_ms { " ms" } else { " samples" })
                        .fixed_decimals(if *is_ms { 2 } else { 0 }),
                );
                let old = *is_ms;
                let combo = egui::ComboBox::from_id_salt("delay_unit")
                    .selected_text(if *is_ms { "Time" } else { "Samples" })
                    .show_ui(ui, |ui| {
                        ui.selectable_value(is_ms, true, crate::i18n::tr("Time"));
                        ui.selectable_value(is_ms, false, crate::i18n::tr("Samples"));
                    });
                let _ = combo;
                actions.changed |= r.changed() || old != *is_ms;
            });
        }
        FilterEditor::Channel { selected, channel_names, channel_mask, dialog } => {
            channel_ui(ui, selected, channel_names, *channel_mask, dialog, actions);
        }
        FilterEditor::Copy {
            assignments,
            channel_names,
            tab_index,
            height,
        } => {
            copy_ui(ui, assignments, channel_names, tab_index, height, actions);
        }
        FilterEditor::GraphicEq(g) => graphic_eq_ui(ui, g, services, actions),
        FilterEditor::Convolution { path, device_sample_rate } => {
            convolution_ui(ui, path, *device_sample_rate, services.config_path, actions);
        }
        FilterEditor::Vst(v) => vst_ui(ui, v, actions),
        FilterEditor::Loudness {
            state,
            ref_level,
            ref_offset,
            att,
            volume,
            calibrating,
            measured,
        } => {
            let vol = services.master_volume.unwrap_or(*volume);
            *volume = vol;
            loudness_ui(
                ui,
                state,
                ref_level,
                ref_offset,
                att,
                vol,
                calibrating,
                measured,
                actions,
            );
        }
        FilterEditor::Include { path } => {
            include_ui(ui, path, services.config_path, actions);
        }
        FilterEditor::Device { pattern, dialog } => {
            device_ui(ui, pattern, dialog, services.all_devices, actions);
        }
        FilterEditor::Stage { pre_mix, post_mix, capture } => {
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("Selected processing stage:"));
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.label(crate::i18n::tr("Playback:"));
                        actions.changed |= ui.checkbox(pre_mix, crate::i18n::tr("Pre-mix")).changed();
                        actions.changed |= ui.checkbox(post_mix, crate::i18n::tr("Post-mix")).changed();
                    });
                    ui.horizontal(|ui| {
                        ui.label(crate::i18n::tr("Capture:"));
                        actions.changed |= ui.checkbox(capture, crate::i18n::tr("Capture")).changed();
                    });
                });
            });
        }
    }
}

fn biquad_ui(ui: &mut egui::Ui, b: &mut BiquadEditor, actions: &mut EditorActions) {
    const TYPES: [BiquadKind; 8] = [
        BiquadKind::Peaking,
        BiquadKind::LowPass,
        BiquadKind::HighPass,
        BiquadKind::BandPass,
        BiquadKind::LowShelf,
        BiquadKind::HighShelf,
        BiquadKind::Notch,
        BiquadKind::AllPass,
    ];
    ui.horizontal(|ui| {
        let before_kind = b.kind;
        egui::ComboBox::from_id_salt("biquad_type")
            .selected_text(crate::i18n::tr(b.kind.label()))
            .show_ui(ui, |ui| {
                for t in TYPES {
                    ui.selectable_value(&mut b.kind, t, crate::i18n::tr(t.label()));
                }
            });
        if before_kind != b.kind {
            b.q_mode = match b.kind {
                BiquadKind::Peaking => QMode::Q,
                BiquadKind::LowPass | BiquadKind::HighPass | BiquadKind::BandPass => QMode::Fixed,
                BiquadKind::LowShelf | BiquadKind::HighShelf => QMode::Fixed,
                BiquadKind::Notch => QMode::Fixed,
                BiquadKind::AllPass => QMode::Q,
            };
            b.freq_mode = if matches!(b.kind, BiquadKind::LowShelf | BiquadKind::HighShelf | BiquadKind::LowPass | BiquadKind::HighPass) {
                FreqMode::Center
            } else {
                FreqMode::Center
            };
            if b.kind == BiquadKind::Peaking && b.q == 0.9 {
                b.q = 10.0;
            }
            if b.kind == BiquadKind::AllPass && b.q == 0.9 {
                b.q = 10.0;
            }
            if matches!(b.kind, BiquadKind::LowShelf | BiquadKind::HighShelf) && b.q == std::f64::consts::FRAC_1_SQRT_2 {
                b.q = 0.9;
            }
            actions.changed = true;
        }

        // Frequency mode: only meaningful for shelves.
        if matches!(b.kind, BiquadKind::LowShelf | BiquadKind::HighShelf | BiquadKind::LowPass | BiquadKind::HighPass) {
            let has_corner = matches!(b.kind, BiquadKind::LowShelf | BiquadKind::HighShelf);
            let old = b.freq_mode;
            egui::ComboBox::from_id_salt("biquad_freq_mode")
                .selected_text(match b.freq_mode {
                    FreqMode::Center => "Center frequency",
                    FreqMode::Corner => "Corner frequency",
                })
                .show_ui(ui, |ui| {
                    ui.selectable_value(&mut b.freq_mode, FreqMode::Center, crate::i18n::tr("Center frequency"));
                    if has_corner {
                        ui.selectable_value(&mut b.freq_mode, FreqMode::Corner, crate::i18n::tr("Corner frequency"));
                    }
                });
            if old != b.freq_mode && has_corner {
                // Frequency conversion from the C++ corner-frequency handler.
                let mode = b.q_mode;
                let mut s = if mode == QMode::Slope { b.q / 12.0 } else { b.q };
                if mode == QMode::Q {
                    let a = 10f64.powf(b.gain / 40.0);
                    s = 1.0 / ((1.0 / (b.q * b.q) - 2.0) / (a + 1.0 / a) + 1.0);
                }
                if s.abs() > 1e-12 {
                    let factor = 10f64.powf(b.gain.abs() / 80.0 / s);
                    let to_center = matches!(b.freq_mode, FreqMode::Center) == (b.kind == BiquadKind::LowShelf);
                    if to_center {
                        b.freq *= factor;
                    } else {
                        b.freq /= factor;
                    }
                }
                actions.changed = true;
            }
        }

        let r = ui.add(
            egui::DragValue::new(&mut b.freq)
                .speed(1.0)
                .range(0.01..=99999.99)
                .suffix(" Hz"),
        );
        actions.changed |= r.changed();
        ui.label(crate::i18n::tr("Fc"));

        if matches!(b.kind, BiquadKind::Peaking | BiquadKind::LowShelf | BiquadKind::HighShelf) {
            ui.label(crate::i18n::tr("Gain:"));
            let r = ui.add(
                egui::DragValue::new(&mut b.gain)
                    .speed(0.1)
                    .range(-199.99..=199.99)
                    .suffix(" dB"),
            );
            actions.changed |= r.changed();
        }

        let shelf = matches!(b.kind, BiquadKind::LowShelf | BiquadKind::HighShelf);
        let show_fixed = !shelf || b.freq_mode == FreqMode::Center;
        if shelf && b.freq_mode == FreqMode::Corner && b.q_mode == QMode::Fixed {
            b.q_mode = QMode::Slope;
        }
        let old_mode = b.q_mode;
        egui::ComboBox::from_id_salt("biquad_q_mode")
            .selected_text(match b.q_mode {
                QMode::Fixed => if shelf { "Fixed S" } else { "Fixed Q" },
                QMode::Q => "Q factor",
                QMode::Bandwidth => "Bandwidth",
                QMode::Slope => "Slope",
            })
            .show_ui(ui, |ui| {
                if show_fixed {
                    ui.selectable_value(&mut b.q_mode, QMode::Fixed, crate::i18n::tr(if shelf { "Fixed S" } else { "Fixed Q" }));
                }
                if matches!(b.kind, BiquadKind::Peaking | BiquadKind::LowShelf | BiquadKind::HighShelf) || matches!(b.kind, BiquadKind::Notch) || matches!(b.kind, BiquadKind::AllPass) || matches!(b.kind, BiquadKind::BandPass) {
                    ui.selectable_value(&mut b.q_mode, QMode::Q, crate::i18n::tr("Q factor"));
                }
                if b.kind == BiquadKind::Peaking {
                    ui.selectable_value(&mut b.q_mode, QMode::Bandwidth, crate::i18n::tr("Bandwidth"));
                }
                if matches!(b.kind, BiquadKind::LowShelf | BiquadKind::HighShelf) {
                    ui.selectable_value(&mut b.q_mode, QMode::Slope, crate::i18n::tr("Slope"));
                }
            });
        if old_mode != b.q_mode {
            match b.q_mode {
                QMode::Fixed => {
                    b.q = match b.kind {
                        BiquadKind::LowShelf | BiquadKind::HighShelf => 0.9,
                        BiquadKind::Notch => 30.0,
                        _ => std::f64::consts::FRAC_1_SQRT_2,
                    };
                }
                QMode::Bandwidth => {
                    let n = 2.0 / std::f64::consts::LN_2 * (1.0 / (2.0 * b.q)).asinh();
                    b.q = n;
                }
                QMode::Q => {
                    if old_mode == QMode::Bandwidth {
                        let p2n = 2f64.powf(b.q);
                        b.q = p2n.sqrt() / (p2n - 1.0);
                    } else if old_mode == QMode::Slope {
                        let s = if b.q != 0.9 { b.q / 12.0 } else { 0.9 };
                        let a = 10f64.powf(b.gain / 40.0);
                        b.q = 1.0 / ((a + 1.0 / a) * (1.0 / s - 1.0) + 2.0).sqrt();
                    }
                }
                QMode::Slope => {
                    if b.q_mode == QMode::Slope {
                        if old_mode == QMode::Q {
                            let s = 1.0 / ((1.0 / (b.q * b.q) - 2.0) / (10f64.powf(b.gain / 40.0) + 10f64.powf(-b.gain / 40.0)) + 1.0);
                            b.q = s * 12.0;
                        } else {
                            b.q = 12.0;
                        }
                    }
                }
            }
            actions.changed = true;
        }

        if b.q_mode != QMode::Fixed {
            let suffix = match b.q_mode {
                QMode::Bandwidth => " Oct",
                QMode::Slope => " dB/Oct",
                _ => "",
            };
            let decimals = 4;
            let r = ui.add(
                egui::DragValue::new(&mut b.q)
                    .speed(if b.q_mode == QMode::Q { 0.1 } else { 0.5 })
                    .range(0.0001..=999.9999)
                    .suffix(suffix)
                    .max_decimals(decimals),
            );
            actions.changed |= r.changed();
        }
    });
}

fn include_ui(ui: &mut egui::Ui, path: &mut String, config_path: &str, actions: &mut EditorActions) {
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr("Include:"));
        let r = ui.add(egui::TextEdit::singleline(path).desired_width(320.0));
        actions.changed |= r.lost_focus();
        if ui.button(crate::i18n::tr("Select file")).clicked() {
            let base = Path::new(config_path).parent().map(Path::to_path_buf).unwrap_or_default();
            let mut dialog = crate::file_dialog::FileDialog::new()
                .add_filter(&crate::i18n::tr("E-APO configurations (*.txt)"), &["txt"])
                .set_title(&crate::i18n::tr("Include file"));
            if !base.as_os_str().is_empty() {
                dialog = dialog.set_directory(&base);
            }
            if let Some(p) = dialog.pick_file() {
                *path = relative_config_path(&base, &p);
                actions.changed = true;
            }
        }
        if ui.button(crate::i18n::tr("Open file in editor")).clicked() {
            if let Some(p) = resolve_config_path(config_path, path) {
                if p.exists() {
                    actions.open_config = Some(p.to_string_lossy().into_owned());
                }
            }
        }
        let error = file_error(config_path, path, false);
        if !error.is_empty() {
            ui.colored_label(Color32::RED, error);
        }
    });
}

fn convolution_ui(
    ui: &mut egui::Ui,
    path: &mut String,
    device_sample_rate: u32,
    config_path: &str,
    actions: &mut EditorActions,
) {
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr("Convolution with impulse response:"));
        let r = ui.add(egui::TextEdit::singleline(path).desired_width(280.0));
        actions.changed |= r.lost_focus();
        if ui.button(crate::i18n::tr("Select file")).clicked() {
            let base = Path::new(config_path).parent().map(Path::to_path_buf).unwrap_or_default();
            let mut dialog = crate::file_dialog::FileDialog::new()
                .add_filter(&crate::i18n::tr("Impulse response (*.wav *.flac *.ogg)"), &["wav", "flac", "ogg"])
                .set_title(&crate::i18n::tr("Select impulse response file"));
            if !base.as_os_str().is_empty() {
                dialog = dialog.set_directory(&base);
            }
            if let Some(p) = dialog.pick_file() {
                *path = relative_config_path(&base, &p);
                actions.changed = true;
            }
        }
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(crate::i18n::tr("File information"));
            let mut length = "No file selected".to_string();
            let mut sample_rate = "No file selected".to_string();
            let mut error = String::new();
            if path.is_empty() {
                error = "No file selected".into();
            } else if let Some(abs) = resolve_config_path(config_path, path) {
                if !abs.exists() {
                    error = "File not found".into();
                } else if audio_service_readable(&abs) == Some(false) {
                    error = "The file is not readable for the audio service.\nChange the file permissions or copy the file to the config directory.".into();
                } else {
                    match probe_cached(&abs) {
                        Ok((rate, frames)) => {
                            let ms = frames as f64 * 1000.0 / rate as f64;
                            length = format!("{} ms ({} samples)", crate::model::qt_num(ms), frames);
                            sample_rate = format!("{rate} Hz");
                            if device_sample_rate != 0 && rate != device_sample_rate {
                                error = format!("The file sample rate does not match the device sample rate ({device_sample_rate} Hz)!\nSelect a different file or change the device configuration.");
                            }
                        }
                        Err(_) => {
                            length = "Unsupported file format".into();
                            sample_rate = String::new();
                            error = "Unsupported file format".into();
                        }
                    }
                }
            }
            ui.horizontal(|ui| {
                ui.label(crate::i18n::tr("Length:"));
                ui.label(length);
                ui.label(crate::i18n::tr("Sample rate:"));
                ui.label(sample_rate);
            });
            if !error.is_empty() {
                ui.colored_label(Color32::RED, error);
            }
        });
    });
}

fn file_error(config_path: &str, path: &str, _convolution: bool) -> String {
    if path.is_empty() {
        return "No file selected".into();
    }
    match resolve_config_path(config_path, path) {
        Some(p) if p.exists() => String::new(),
        _ => "File not found".into(),
    }
}

pub fn resolve_config_path(config_path: &str, path: &str) -> Option<PathBuf> {
    if path.is_empty() {
        return None;
    }
    let p = Path::new(path);
    if p.is_absolute() {
        Some(p.to_path_buf())
    } else {
        let base = Path::new(config_path).parent()?;
        Some(base.join(p))
    }
}

pub fn relative_config_path(base: &Path, abs: &Path) -> String {
    let rel = path_relative(base, abs);
    let text = if rel.starts_with("..\\..\\") || rel.starts_with("../") {
        abs.to_string_lossy().replace('/', "\\")
    } else {
        rel
    };
    text
}

fn path_relative(base: &Path, target: &Path) -> String {
    let base_components: Vec<Component> = base.components().collect();
    let target_components: Vec<Component> = target.components().collect();
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
    parts.join("\\")
}

fn vst_ui(ui: &mut egui::Ui, v: &mut VstEditor, actions: &mut EditorActions) {
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr("VST plugin:"));
        let r = ui.add(egui::TextEdit::singleline(&mut v.path).desired_width(320.0));
        if r.lost_focus() {
            crate::vst_editor::refresh_plugin_status(v);
            actions.changed = true;
        }
        if ui.button(crate::i18n::tr("Select file")).clicked() {
            let default = eapo_filters::vst_plugin_library::VSTPluginLibrary::get_default_plugin_path();
            let mut dialog = crate::file_dialog::FileDialog::new()
                .add_filter(&crate::i18n::tr("VST plugins (*.dll)"), &["dll"])
                .set_title(&crate::i18n::tr("Select VST plugin"));
            if !default.is_empty() && Path::new(&default).exists() {
                dialog = dialog.set_directory(&default);
            }
            if let Some(p) = dialog.pick_file() {
                v.path = crate::model::vst_config_path(&p.to_string_lossy());
                crate::vst_editor::refresh_plugin_status(v);
                actions.changed = true;
            }
        }
        let options = ui.menu_button(crate::i18n::tr("Options"), |ui| {
            if ui.checkbox(&mut v.embed, crate::i18n::tr("Embed")).changed() {
                actions.changed = true;
            }
            if ui.checkbox(&mut v.auto_apply, crate::i18n::tr("Auto apply")).changed() {
                actions.changed = true;
            }
        });
        options.response.on_hover_text(crate::i18n::tr("Options"));
    });
    ui.horizontal(|ui| {
        if v.path.is_empty() {
            ui.colored_label(Color32::RED, crate::i18n::tr("No file selected."));
        } else {
            ui.label(if v.status.is_empty() { "VST plugin editor" } else { &v.status });
        }
        if ui.button(crate::i18n::tr("Open panel")).clicked() {
            if crate::vst_editor::open_plugin_editor(v) {
                actions.changed = true;
            }
        }
    });
    if !v.path.is_empty() {
        let path = Path::new(&v.path);
        let absolute = if path.is_absolute() {
            path.to_path_buf()
        } else {
            Path::new(
                &eapo_filters::vst_plugin_library::VSTPluginLibrary::get_default_plugin_path(),
            )
            .join(path)
        };
        if absolute.exists() && audio_service_readable(&absolute) == Some(false) {
            ui.colored_label(
                Color32::RED, crate::i18n::tr("The library is not readable by the audio service.\nChange the file permissions or copy the file to the VSTPlugins directory."),
            );
        }
    }
}

fn loudness_ui(
    ui: &mut egui::Ui,
    state: &mut bool,
    ref_level: &mut i32,
    ref_offset: &mut i32,
    att: &mut f64,
    volume: f64,
    calibrating: &mut bool,
    measured: &mut i32,
    actions: &mut EditorActions,
) {
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr("Loudness correction:"));
        ui.label(crate::i18n::tr("Reference level:"));
        actions.changed |= ui
            .add(egui::DragValue::new(ref_level).speed(1.0).range(-999..=999).suffix(" dB"))
            .changed();
        ui.label(crate::i18n::tr("Reference offset:"));
        actions.changed |= ui
            .add(egui::DragValue::new(ref_offset).speed(1.0).range(-999..=999).suffix(" dB"))
            .changed();
        ui.label(crate::i18n::tr("Attenuation:"));
        actions.changed |= ui
            .add(egui::DragValue::new(att).speed(0.01).range(0.0..=1.0).max_decimals(4))
            .changed();
        if ui.button(crate::i18n::tr("Calibrate")).clicked() {
            // The C++ dialog disables loudness correction while measuring.
            *state = false;
            *calibrating = true;
            crate::calibration::stop();
            actions.changed = true;
        }
        ui.label(crate::i18n::tr("Master volume:"));
        let mut shown_volume = volume;
        ui.add_enabled(false, egui::DragValue::new(&mut shown_volume).suffix(" dB").fixed_decimals(1));
    });

    if *calibrating {
        let mut open = true;
        egui::Window::new(crate::i18n::tr("Calibration"))
            .collapsible(false)
            .resizable(false)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.label(crate::i18n::tr("For a correct calibration, a sound level meter is required."));
                ui.label(crate::i18n::tr("Disable loudness correction during measurement (done automatically while this dialog is open)."));
                ui.label(crate::i18n::tr("Enable only one speaker and play pink noise at full application volume."));
                ui.label(crate::i18n::tr("Channel"));
                ui.horizontal(|ui| {
                    let selected = crate::calibration::selected_channel();
                    let mut new_channel = selected;
                    if ui.radio(selected == crate::calibration::CHANNEL_LEFT, crate::i18n::tr("Left")).clicked() {
                        new_channel = crate::calibration::CHANNEL_LEFT;
                    }
                    if ui.radio(selected == crate::calibration::CHANNEL_RIGHT, crate::i18n::tr("Right")).clicked() {
                        new_channel = crate::calibration::CHANNEL_RIGHT;
                    }
                    if ui.radio(selected == crate::calibration::CHANNEL_BOTH, crate::i18n::tr("Both")).clicked() {
                        new_channel = crate::calibration::CHANNEL_BOTH;
                    }
                    if new_channel != selected {
                        crate::calibration::set_selected_channel(new_channel);
                    }
                    if ui
                        .button(if crate::calibration::is_playing() { "Stop" } else { "Play" })
                        .clicked()
                    {
                        if crate::calibration::is_playing() {
                            crate::calibration::stop();
                        } else {
                            crate::calibration::play(crate::calibration::selected_channel());
                        }
                    }
                });
                ui.label(crate::i18n::tr("Adjust volume via Windows volume control so that the sound level meter shows 75 dB at listening position."));
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::tr("Level at listening position:"));
                    ui.add(egui::DragValue::new(measured).speed(1.0).range(0..=200).suffix(" dB"));
                });
                ui.horizontal(|ui| {
                    if ui.button(crate::i18n::tr("OK")).clicked() {
                        *ref_level = ((75 - *measured) as f64 + volume) as i32;
                        *state = true;
                        *calibrating = false;
                        crate::calibration::stop();
                        actions.changed = true;
                    }
                    if ui.button(crate::i18n::tr("Cancel")).clicked() {
                        *state = true;
                        *calibrating = false;
                        crate::calibration::stop();
                        actions.changed = true;
                    }
                });
            });
        if !open {
            *state = true;
            *calibrating = false;
            crate::calibration::stop();
            actions.changed = true;
        }
    }
}

fn channel_ui(
    ui: &mut egui::Ui,
    selected: &mut Vec<String>,
    channel_names: &[String],
    channel_mask: u32,
    dialog: &mut Option<ChannelDialogState>,
    actions: &mut EditorActions,
) {
    ui.horizontal_wrapped(|ui| {
        ui.label(crate::i18n::tr("Selected channels:"));
        let all = selected.iter().any(|s| s == "ALL");
        let mut names: Vec<String> = channel_names.to_vec();
        names.push("ALL".into());
        for s in selected.iter() {
            if s != "ALL" && !names.iter().any(|n| n.eq_ignore_ascii_case(s)) {
                names.push(s.clone());
            }
        }
        for name in names {
            let mut on = if name == "ALL" { all } else { selected.iter().any(|s| s.eq_ignore_ascii_case(&name)) };
            if ui.selectable_label(on, &name).clicked() {
                on = !on;
                if name == "ALL" {
                    if on { selected.clear(); selected.push("ALL".into()); }
                    else { selected.retain(|s| s != "ALL"); }
                } else {
                    selected.retain(|s| s != "ALL");
                    if on {
                        selected.push(name.clone());
                    } else {
                        selected.retain(|s| !s.eq_ignore_ascii_case(&name));
                    }
                }
                actions.changed = true;
            }
        }
        if selected.is_empty() {
            ui.weak("(none)");
        }
    });
    if ui.button(crate::i18n::tr("Change...")).clicked() {
        let mut d = ChannelDialogState::default();
        d.all = selected.iter().any(|s| s == "ALL");
        d.selected = selected.clone();
        let positions = speaker_positions(channel_mask);
        for (name, _) in &positions {
            let present = channel_names.iter().any(|n| n == name);
            if present && !d.extras.iter().any(|(n, _)| n == name) {
                d.extras.push((name.clone(), selected.iter().any(|s| s.eq_ignore_ascii_case(name))));
            }
        }
        for n in channel_names {
            if !positions.iter().any(|(p, _)| p == n) && !d.extras.iter().any(|(e, _)| e == n) {
                d.extras.push((n.clone(), selected.iter().any(|s| s.eq_ignore_ascii_case(n))));
            }
        }
        for s in selected.iter() {
            if s != "ALL" && !channel_names.iter().any(|n| n.eq_ignore_ascii_case(s)) && !d.extras.iter().any(|(n, _)| n.eq_ignore_ascii_case(s)) {
                d.extras.push((s.clone(), true));
            }
        }
        *dialog = Some(d);
    }

    if let Some(d) = dialog {
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new(crate::i18n::tr("Channel selection"))
            .collapsible(false)
            .resizable(true)
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.checkbox(&mut d.all, crate::i18n::tr("Select all channels"));
                ui.add_enabled_ui(!d.all, |ui| {
                    ui.group(|ui| {
                        ui.label(crate::i18n::tr("Speaker positions"));
                        let positions = speaker_positions(channel_mask);
                        egui::Grid::new("channel_positions").show(ui, |ui| {
                            for (name, label) in positions {
                                let mut checked = d.selected.iter().any(|s| s.eq_ignore_ascii_case(&name));
                                if ui.checkbox(&mut checked, format!("{name} ({label})")).changed() {
                                    d.selected.retain(|s| !s.eq_ignore_ascii_case(&name) && !(name == "LFE" && s == "SUB"));
                                    if checked {
                                        d.selected.push(name.clone());
                                    }
                                }
                            }
                        });
                    });
                    ui.group(|ui| {
                        ui.label(crate::i18n::tr("Additional channels"));
                        egui::ScrollArea::vertical().max_height(140.0).show(ui, |ui| {
                            let mut remove = None;
                            for (i, (name, checked)) in d.extras.iter_mut().enumerate() {
                                ui.horizontal(|ui| {
                                    ui.checkbox(checked, "");
                                    ui.add(egui::TextEdit::singleline(name).desired_width(100.0));
                                    if ui.small_button(crate::i18n::tr("Remove channel")).clicked() {
                                        remove = Some(i);
                                    }
                                });
                            }
                            if let Some(i) = remove {
                                d.extras.remove(i);
                            }
                        });
                        if ui.button(crate::i18n::tr("Add channel")).clicked() {
                            d.extras.push((String::new(), true));
                        }
                    });
                });
                ui.horizontal(|ui| {
                    if ui.button(crate::i18n::tr("OK")).clicked() {
                        apply = true;
                    }
                    if ui.button(crate::i18n::tr("Cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if apply {
            if d.all {
                selected.clear();
                selected.push("ALL".into());
            } else {
                selected.clear();
                for s in &d.selected {
                    if s != "ALL" {
                        selected.push(s.to_uppercase());
                    }
                }
                for (name, checked) in &d.extras {
                    let trimmed = name.trim();
                    if *checked && !trimmed.is_empty() {
                        selected.push(trimmed.to_uppercase());
                    }
                }
            }
            *dialog = None;
            actions.changed = true;
        } else if cancel || !open {
            *dialog = None;
        }
    }
}

fn speaker_positions(mask: u32) -> Vec<(String, &'static str)> {
    // Order mirrors ChannelFilterGUIDialog: C, L, R, SL, SR, RL, RR, RC, LFE.
    let defs: &[(u32, &str, &str)] = &[
        (channel_helper::SPEAKER_FRONT_CENTER, "C", "Center"),
        (channel_helper::SPEAKER_FRONT_LEFT, "L", "Left"),
        (channel_helper::SPEAKER_FRONT_RIGHT, "R", "Right"),
        (channel_helper::SPEAKER_SIDE_LEFT, "SL", "Side left"),
        (channel_helper::SPEAKER_SIDE_RIGHT, "SR", "Side right"),
        (channel_helper::SPEAKER_BACK_LEFT, "RL", "Rear left"),
        (channel_helper::SPEAKER_BACK_RIGHT, "RR", "Rear right"),
        (channel_helper::SPEAKER_BACK_CENTER, "RC", "Rear center"),
        (channel_helper::SPEAKER_LOW_FREQUENCY, "LFE", "Subwoofer"),
    ];
    defs.iter()
        .filter(|(bit, _, _)| mask & bit != 0)
        .map(|(_, name, label)| (name.to_string(), *label))
        .collect()
}

fn copy_ui(
    ui: &mut egui::Ui,
    assignments: &mut Vec<Assignment>,
    channel_names: &mut Vec<String>,
    tab_index: &mut usize,
    height: &mut f32,
    actions: &mut EditorActions,
) {
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr("Copy channels:"));
        ui.selectable_value(tab_index, 0, crate::i18n::tr("Graph")).on_hover_text(crate::i18n::tr("Graph"));
        ui.selectable_value(tab_index, 1, crate::i18n::tr("Assignments")).on_hover_text(crate::i18n::tr("Assignments"));
    });
    if *tab_index == 0 {
        copy_graph(ui, assignments, channel_names);
    } else {
        let mut working = assignments.clone();
        egui::ScrollArea::vertical().max_height((*height).max(40.0)).show(ui, |ui| {
            let mut remove_assignment = None;
            for ai in 0..working.len() {
                let mut add_summand = false;
                let mut remove_summand = None;
                ui.horizontal(|ui| {
                    ui.label(crate::i18n::tr("Channel"));
                    let r = ui.add(egui::TextEdit::singleline(&mut working[ai].target).desired_width(50.0));
                    actions.changed |= r.changed();
                    ui.label("=");
                    let summand_count = working[ai].summands.len();
                    for si in 0..summand_count {
                        if si > 0 {
                            ui.label("+");
                        }
                        let summand = &mut working[ai].summands[si];
                        let mode_index = if summand.channel.is_empty() && summand.channel != " " {
                            3
                        } else if summand.is_decibel {
                            2
                        } else if summand.factor != 1.0 {
                            1
                        } else {
                            0
                        };
                        let mut mode = mode_index;
                        egui::ComboBox::from_id_salt(("copy_mode", ai, si))
                            .selected_text(["Channel", "Factor * channel", "dB factor * channel", "Constant value"][mode_index])
                            .show_ui(ui, |ui| {
                                ui.selectable_value(&mut mode, 0, crate::i18n::tr("Channel"));
                                ui.selectable_value(&mut mode, 1, crate::i18n::tr("Factor * channel"));
                                ui.selectable_value(&mut mode, 2, crate::i18n::tr("dB factor * channel"));
                                ui.selectable_value(&mut mode, 3, crate::i18n::tr("Constant value"));
                            });
                        if mode != mode_index {
                            summand.is_decibel = mode == 2;
                            match mode {
                                0 => summand.factor = 1.0,
                                3 => {
                                    if summand.channel.is_empty() {
                                        summand.channel = " ".into();
                                    }
                                }
                                1 => {
                                    if summand.factor == 1.0 {
                                        summand.factor = 0.5;
                                    }
                                }
                                _ => {}
                            }
                            if mode != 3 && summand.channel == " " {
                                summand.channel = String::new();
                            }
                            actions.changed = true;
                        }
                        if mode != 0 {
                            let r = ui.add(
                                egui::DragValue::new(&mut summand.factor)
                                    .speed(0.1)
                                    .range(-99.99..=999.99)
                                    .suffix(if summand.is_decibel { " dB" } else { "" })
                                    .max_decimals(4),
                            );
                            actions.changed |= r.changed();
                        }
                        if mode == 1 || mode == 2 {
                            ui.label("*");
                        }
                        if mode != 3 {
                            let r = ui.add(egui::TextEdit::singleline(&mut summand.channel).desired_width(50.0));
                            actions.changed |= r.changed();
                        }
                        if ui.small_button(crate::i18n::tr("Add summand")).clicked() {
                            add_summand = true;
                            actions.changed = true;
                        }
                        let label = if summand_count == 1 { "Remove assignment" } else { "Remove summand" };
                        if ui.small_button(label).clicked() {
                            remove_summand = Some(si);
                        }
                    }
                    if add_summand {
                        let mut new = Summand::default();
                        new.channel = String::new();
                        working[ai].summands.push(new);
                    }
                    if let Some(si) = remove_summand {
                        if working[ai].summands.len() == 1 {
                            remove_assignment = Some(ai);
                        } else {
                            working[ai].summands.remove(si);
                            actions.changed = true;
                        }
                    }
                });
            }
            if let Some(ai) = remove_assignment {
                if ai < working.len() {
                    working.remove(ai);
                    actions.changed = true;
                }
            }
            if ui.button(crate::i18n::tr("Add assignment")).clicked() {
                let mut summand = Summand::default();
                summand.channel = String::new();
                working.push(Assignment { target: String::new(), summands: vec![summand] });
                actions.changed = true;
            }
        });
        *assignments = working;
        let handle = ui.allocate_response(egui::vec2(ui.available_width(), 7.0), Sense::drag());
        handle.clone().on_hover_cursor(egui::CursorIcon::ResizeVertical);
        if handle.dragged() {
            *height = (*height + handle.drag_delta().y).clamp(40.0, 600.0);
        }
        // Keep the graph source/target list in sync with typed targets.
        for a in assignments.iter() {
            if !a.target.is_empty() && !channel_names.iter().any(|n| n.eq_ignore_ascii_case(&a.target)) {
                channel_names.push(a.target.clone());
            }
            for s in &a.summands {
                if !s.channel.trim().is_empty() && s.channel != " " && !channel_names.iter().any(|n| n.eq_ignore_ascii_case(&s.channel)) {
                    channel_names.push(s.channel.clone());
                }
            }
        }
    }
}

fn copy_graph(ui: &mut egui::Ui, assignments: &[Assignment], channel_names: &[String]) {
    let height = 88.0;
    let (response, painter) = ui.allocate_painter(
        Vec2::new(ui.available_width().max(200.0), height),
        Sense::hover(),
    );
    let rect = response.rect;
    painter.rect_filled(rect, 2.0, ui.visuals().extreme_bg_color);
    let mut inputs: Vec<String> = channel_names.to_vec();
    inputs.push(String::new());
    for a in assignments {
        for s in &a.summands {
            let c = s.channel.trim();
            if !c.is_empty() && !inputs.iter().any(|n| n.eq_ignore_ascii_case(c)) {
                inputs.push(c.to_string());
            }
        }
    }
    let mut outputs: Vec<String> = channel_names.to_vec();
    for a in assignments {
        if !a.target.is_empty() && !outputs.iter().any(|n| n.eq_ignore_ascii_case(&a.target)) {
            outputs.push(a.target.clone());
        }
    }
    let box_w = 46.0;
    let row_h = 18.0;
    let input_x = rect.left() + 8.0;
    let output_x = rect.right() - box_w - 8.0;
    let mut input_pos = std::collections::HashMap::new();
    for (i, name) in inputs.iter().enumerate() {
        let y = rect.top() + 4.0 + i as f32 * row_h;
        if y + 14.0 > rect.bottom() {
            break;
        }
        let r = egui::Rect::from_min_size(egui::pos2(input_x, y), Vec2::new(box_w, 14.0));
        painter.rect_filled(r, 3.0, Color32::from_rgb(176, 255, 176));
        painter.text(r.center(), egui::Align2::CENTER_CENTER, if name.is_empty() { "0" } else { name }, egui::FontId::proportional(9.0), Color32::BLACK);
        input_pos.insert(name.to_uppercase(), r.center());
    }
    for (i, name) in outputs.iter().enumerate() {
        let y = rect.top() + 4.0 + i as f32 * row_h;
        if y + 14.0 > rect.bottom() {
            break;
        }
        let r = egui::Rect::from_min_size(egui::pos2(output_x, y), Vec2::new(box_w, 14.0));
        painter.rect_filled(r, 3.0, Color32::from_rgb(255, 194, 194));
        painter.text(r.center(), egui::Align2::CENTER_CENTER, name, egui::FontId::proportional(9.0), Color32::BLACK);
        if let Some(ap) = input_pos.get(&name.to_uppercase()) {
            let _ = ap;
        }
        for a in assignments.iter().filter(|a| a.target.eq_ignore_ascii_case(name)) {
            for s in &a.summands {
                let key = s.channel.trim().to_uppercase();
                let Some(ip) = input_pos.get(&key) else { continue };
                let color = if s.is_decibel { Color32::from_rgb(160, 120, 255) } else { Color32::from_gray(130) };
                painter.line_segment([*ip, r.center()], Stroke::new(1.0, color));
                if s.factor != 1.0 || s.is_decibel || s.channel.trim().is_empty() {
                    let label = if s.channel.trim().is_empty() {
                        format!("{}", s.factor)
                    } else if s.is_decibel {
                        format!("{} dB", s.factor)
                    } else {
                        format!("x{}", s.factor)
                    };
                    painter.text(
                        egui::pos2((ip.x + r.center().x) * 0.5, (ip.y + r.center().y) * 0.5),
                        egui::Align2::CENTER_CENTER,
                        label,
                        egui::FontId::proportional(8.0),
                        color,
                    );
                }
            }
        }
    }
}

fn graphic_eq_ui(
    ui: &mut egui::Ui,
    g: &mut GraphicEqEditor,
    services: &GuiServices<'_>,
    actions: &mut EditorActions,
) {
    ui.horizontal(|ui| {
        ui.vertical(|ui| {
            ui.label(crate::i18n::tr("Graphic EQ:"));
            for (count, label) in [(15, "15-band"), (31, "31-band"), (-1, "variable")] {
            let label = crate::i18n::tr(label);
                if ui.radio(g.band_count == count, label).clicked() {
                    g.set_band_count(count);
                    actions.changed = true;
                }
            }
        });
        let result = plot::show_graphic_eq(ui, g);
        actions.changed |= result.changed;
        let split = ui.allocate_response(egui::vec2(7.0, g.view_height.max(60.0)), Sense::drag());
        split.clone().on_hover_cursor(egui::CursorIcon::ResizeHorizontal);
        if split.dragged() {
            g.table_width = (g.table_width - split.drag_delta().x).clamp(50.0, 400.0);
        }
        let table_width = g.table_width.clamp(60.0, 400.0);
        ui.allocate_ui_with_layout(
            Vec2::new(table_width, g.view_height),
            egui::Layout::top_down(egui::Align::Min),
            |ui| {
                egui::ScrollArea::vertical().id_salt("geq_table").show(ui, |ui| {
                    egui::Grid::new("geq_grid").num_columns(3).striped(true).show(ui, |ui| {
                        ui.strong("Freq.");
                        ui.strong("Gain");
                        ui.label("");
                        ui.end_row();
                        let mut remove = None;
                        for i in 0..g.nodes.len() {
                            let selected = g.selected.contains(&i);
                            let mut changed_row = false;
                            ui.horizontal(|ui| {
                                if ui.selectable_label(selected, format!("{}", crate::model::qt_num(g.nodes[i].freq))).clicked() {
                                    g.selected = vec![i];
                                }
                            });
                            if g.band_count == -1 {
                                let mut freq = g.nodes[i].freq;
                                if ui.add(egui::DragValue::new(&mut freq).speed(0.5).range(0.001..=30000.0)).changed() {
                                    g.nodes[i].freq = freq;
                                    changed_row = true;
                                }
                            } else {
                                ui.label(crate::model::qt_num(g.nodes[i].freq));
                            }
                            let mut db = g.nodes[i].db_gain;
                            if ui.add(egui::DragValue::new(&mut db).speed(0.1).range(-100.0..=100.0)).changed() {
                                g.nodes[i].db_gain = db;
                                changed_row = true;
                            }
                            if ui.small_button("x").clicked() {
                                remove = Some(i);
                            }
                            ui.end_row();
                            if changed_row {
                                g.sort_nodes();
                                actions.changed = true;
                            }
                        }
                        if let Some(i) = remove {
                            if g.band_count == -1 {
                                g.nodes.remove(i);
                                g.selected.clear();
                                actions.changed = true;
                            }
                        }
                    });
                });
                ui.horizontal(|ui| {
                    if ui.button(crate::i18n::tr("Import")).clicked() {
                        import_graphic_eq(g);
                        actions.changed = true;
                    }
                    if ui.button(crate::i18n::tr("Export")).clicked() {
                        export_graphic_eq(g, services.config_path);
                    }
                    if ui.button(crate::i18n::tr("Invert response")).clicked() {
                        for n in &mut g.nodes { n.db_gain = -n.db_gain; }
                        actions.changed = true;
                    }
                    if ui.button(crate::i18n::tr("Normalize response")).clicked() {
                        let max = g.nodes.iter().map(|n| n.db_gain).fold(f64::NEG_INFINITY, f64::max);
                        if max != 0.0 && max.is_finite() {
                            for n in &mut g.nodes { n.db_gain -= max; }
                            actions.changed = true;
                        }
                    }
                    if ui.button(crate::i18n::tr("Reset response")).clicked() {
                        if g.selected.is_empty() {
                            for n in &mut g.nodes { n.db_gain = 0.0; }
                        } else {
                            for &i in &g.selected { if i < g.nodes.len() { g.nodes[i].db_gain = 0.0; } }
                        }
                        actions.changed = true;
                    }
                });
            },
        );
    });
    let handle = ui.allocate_response(egui::vec2(ui.available_width(), 7.0), Sense::drag());
    handle.clone().on_hover_cursor(egui::CursorIcon::ResizeVertical);
    if handle.dragged() {
        g.view_height = (g.view_height + handle.drag_delta().y).clamp(60.0, 600.0);
        actions.changed = true;
    }
}

fn import_graphic_eq(g: &mut GraphicEqEditor) {
    let Some(files) = crate::file_dialog::FileDialog::new()
        .add_filter(&crate::i18n::tr("Frequency response (*.csv)"), &["csv"])
        .add_filter(&crate::i18n::tr("All files (*.*)"), &["*"])
        .set_title(&crate::i18n::tr("Import frequency response"))
        .pick_files()
    else {
        return;
    };
    let re = regex::Regex::new(r"[-+]?[0-9]*\.?[0-9]+([eE][-+]?[0-9]+)?").unwrap();
    let mut nodes = Vec::new();
    for file in files {
        let Ok(text) = std::fs::read_to_string(&file) else { continue };
        for line in text.lines() {
            if line.starts_with('*') {
                continue;
            }
            let line = if !line.contains('.') { line.replace(',', ".") } else { line.to_string() };
            let mut it = re.find_iter(&line).filter_map(|m| m.as_str().parse::<f64>().ok());
            while let (Some(freq), Some(gain)) = (it.next(), it.next()) {
                nodes.push(FilterNode { freq, db_gain: gain });
            }
        }
    }
    nodes.sort_by(|a, b| a.freq.partial_cmp(&b.freq).unwrap_or(std::cmp::Ordering::Equal));
    g.nodes = nodes;
    g.band_count = g.verify_bands();
    g.selected.clear();
}

fn export_graphic_eq(g: &GraphicEqEditor, config_path: &str) {
    let mut dialog = crate::file_dialog::FileDialog::new()
        .add_filter(&crate::i18n::tr("Frequency response (*.csv)"), &["csv"])
        .add_filter(&crate::i18n::tr("All files (*.*)"), &["*"])
        .set_title(&crate::i18n::tr("Export frequency response"));
    if let Some(base) = Path::new(config_path).parent() {
        dialog = dialog.set_directory(base);
    }
    if let Some(mut path) = dialog.save_file() {
        if path.extension().is_none() {
            path.set_extension("csv");
        }
        let mut text = String::new();
        for n in &g.nodes {
            text.push_str(&format!("{}\t{}\n", crate::model::qt_num(n.freq), crate::model::qt_num(n.db_gain)));
        }
        let _ = std::fs::write(path, text);
    }
}

fn device_ui(
    ui: &mut egui::Ui,
    pattern: &mut String,
    dialog: &mut Option<DeviceDialogState>,
    devices: &[Arc<dyn AbstractAPOInfo>],
    actions: &mut EditorActions,
) {
    ui.horizontal(|ui| {
        ui.label(crate::i18n::tr("Selected devices:"));
        if ui.button(crate::i18n::tr("Change...")).clicked() {
            let all = pattern.trim().eq_ignore_ascii_case("all");
            let d = DeviceDialogState {
                all,
                show_installed: true,
                checked: if all {
                    Vec::new()
                } else {
                    devices
                        .iter()
                        .filter(|dev| device_matches(&dev.get_device_string(), pattern))
                        .map(|dev| dev.get_device_string())
                        .collect()
                },
            };
            *dialog = Some(d);
        }
    });
    egui::Frame::group(ui.style()).show(ui, |ui| {
        egui::Grid::new("device_list").num_columns(4).striped(true).show(ui, |ui| {
            ui.strong("Type");
            ui.strong("Connection");
            ui.strong("Device");
            ui.strong("State");
            ui.end_row();
            if pattern.trim().eq_ignore_ascii_case("all") {
                ui.label("");
                ui.label(crate::i18n::tr("All devices"));
                ui.label("");
                ui.label("");
                ui.end_row();
            } else {
                let mut any = false;
                let any_installed = devices.iter().any(|d| d.is_installed() && device_matches(&d.get_device_string(), pattern));
                for dev in devices {
                    if !device_matches(&dev.get_device_string(), pattern) || (any_installed && !dev.is_installed()) {
                        continue;
                    }
                    any = true;
                    ui.label(if dev.is_input() { "Capture" } else { "Playback" });
                    ui.label(dev.get_connection_name());
                    ui.label(dev.get_device_name());
                    let mut state = if dev.is_installed() { "APO installed".to_string() } else { "APO not installed".to_string() };
                    if let Some(vm) = dev.as_any().downcast_ref::<VoicemeeterAPOInfo>() {
                        if !vm.is_voicemeeter_installed() {
                            state.push_str(", Voicemeeter was uninstalled");
                        }
                    }
                    ui.label(state);
                    ui.end_row();
                }
                if !any && !devices.is_empty() {
                    ui.label("");
                    ui.colored_label(Color32::RED, format!("No device matched \"{}\"", pattern.trim()));
                    ui.label("");
                    ui.label("");
                    ui.end_row();
                }
            }
        });
    });

    if let Some(d) = dialog {
        let mut open = true;
        let mut apply = false;
        let mut cancel = false;
        egui::Window::new(crate::i18n::tr("Device selection"))
            .collapsible(false)
            .resizable(true)
            .default_size(Vec2::new(520.0, 360.0))
            .open(&mut open)
            .show(ui.ctx(), |ui| {
                ui.checkbox(&mut d.all, crate::i18n::tr("Select all devices"));
                ui.checkbox(&mut d.show_installed, crate::i18n::tr("Show only devices with APO installed"));
                ui.separator();
                egui::ScrollArea::vertical().max_height(280.0).show(ui, |ui| {
                    for input in [false, true] {
                        ui.strong(if input { "Capture devices" } else { "Playback devices" });
                        egui::Grid::new(if input { "input_devices" } else { "output_devices" })
                            .num_columns(3)
                            .striped(true)
                            .show(ui, |ui| {
                                for dev in devices.iter().filter(|d| d.is_input() == input) {
                                    let key = dev.get_device_string();
                                    let mut checked = d.checked.iter().any(|s| s.eq_ignore_ascii_case(&key));
                                    if d.show_installed && !checked && !dev.is_installed() {
                                        continue;
                                    }
                                    if ui.add_enabled(!d.all, egui::Checkbox::new(&mut checked, dev.get_connection_name())).changed() {
                                        d.checked.retain(|s| !s.eq_ignore_ascii_case(&key));
                                        if checked {
                                            d.checked.push(key.clone());
                                        }
                                    }
                                    ui.label(dev.get_device_name());
                                    let mut state = if dev.is_installed() { "APO installed".to_string() } else { "APO not installed".to_string() };
                                    if let Some(vm) = dev.as_any().downcast_ref::<VoicemeeterAPOInfo>() {
                                        if !vm.is_voicemeeter_installed() {
                                            state.push_str(", Voicemeeter was uninstalled");
                                        }
                                    }
                                    ui.label(state);
                                    ui.end_row();
                                }
                            });
                    }
                });
                ui.horizontal(|ui| {
                    if ui.button(crate::i18n::tr("OK")).clicked() {
                        apply = true;
                    }
                    if ui.button(crate::i18n::tr("Cancel")).clicked() {
                        cancel = true;
                    }
                });
            });
        if apply {
            if d.all {
                *pattern = "all".into();
            } else {
                *pattern = d.checked.join("; ");
            }
            *dialog = None;
            actions.changed = true;
        } else if cancel || !open {
            *dialog = None;
        }
    }
}

/// Port of `DeviceFilterFactory::matchDevice`: semicolon separated groups,
/// spaces within a group are AND terms, any matching group wins.
pub fn device_matches(device_string: &str, pattern: &str) -> bool {
    let no_guid = regex::Regex::new(r"\{[0-9A-Fa-f-]+\}").unwrap().replace_all(device_string, "").to_string();
    let pattern = format!("{};", pattern.trim());
    let mut group: Vec<String> = Vec::new();
    let mut groups: Vec<Vec<String>> = Vec::new();
    let mut word = String::new();
    for c in pattern.chars() {
        if c == ' ' || c == ';' {
            if !word.is_empty() {
                group.push(std::mem::take(&mut word));
            }
            if c == ';' && !group.is_empty() {
                groups.push(std::mem::take(&mut group));
            }
        } else {
            word.push(c);
        }
    }
    for group in groups {
        if group.len() == 1 && group[0].eq_ignore_ascii_case("all") {
            return true;
        }
        let mut matches = true;
        for term in group {
            let hay = if term.contains('{') { device_string } else { &no_guid };
            if !hay.to_lowercase().contains(&term.to_lowercase()) {
                matches = false;
                break;
            }
        }
        if matches {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_pattern_matching_matches_cpp_rules() {
        let dev = "Speakers {aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee}";
        assert!(device_matches(dev, "all"));
        assert!(device_matches(dev, "speakers"));
        assert!(device_matches(dev, "Speakers {aaaaaaaa-bbbb-cccc-dddd-eeeeeeeeeeee}"));
        assert!(!device_matches(dev, "headphones; microphone"));
        assert!(device_matches(dev, "headphones; speakers"));
        assert!(!device_matches(dev, "aaa bbb"));
        assert!(device_matches(dev, "{aaaaaaaa-bbbb"));
        assert!(!device_matches(dev, "speakers zzz"));
    }

    #[test]
    fn relative_paths_match_cpp_choice() {
        let base = Path::new("C:\\Config");
        let abs = Path::new("C:\\Config\\sub\\ir.wav");
        assert_eq!(path_relative(base, abs), "sub\\ir.wav");
    }
}
