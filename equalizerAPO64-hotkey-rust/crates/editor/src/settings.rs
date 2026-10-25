//! Registry-backed editor settings (QSettings equivalents).

use std::path::Path;

use serde_json::Value as JsonValue;

use crate::model::FilterItem;

pub const EDITOR_REGPATH: &str = r"HKEY_CURRENT_USER\SOFTWARE\EqualizerAPO\Configuration Editor";
pub const EDITOR_PER_FILE_REGPATH: &str =
    r"HKEY_CURRENT_USER\SOFTWARE\EqualizerAPO\Configuration Editor\file-specific";
pub const ANALYSIS_REGPATH: &str =
    r"HKEY_CURRENT_USER\SOFTWARE\EqualizerAPO\Configuration Editor\analysis";

fn read_int(key: &str, name: &str, default: i32) -> i32 {
    if let Ok(v) = eapo_common::registry_helper::read_dword_value(key, name) {
        return v as i32;
    }
    if let Ok(v) = eapo_common::registry_helper::read_value(key, name) {
        return v.parse().unwrap_or(default);
    }
    default
}

fn read_float(key: &str, name: &str, default: f64) -> f64 {
    if let Ok(v) = eapo_common::registry_helper::read_value(key, name) {
        return v.parse().unwrap_or(default);
    }
    if let Ok(v) = eapo_common::registry_helper::read_dword_value(key, name) {
        return v as f64;
    }
    default
}

fn read_bool(key: &str, name: &str, default: bool) -> bool {
    if let Ok(v) = eapo_common::registry_helper::read_value(key, name) {
        let l = v.to_ascii_lowercase();
        return l == "true" || l == "1";
    }
    if let Ok(v) = eapo_common::registry_helper::read_dword_value(key, name) {
        return v != 0;
    }
    default
}

fn write_bool(key: &str, name: &str, value: bool) {
    let _ = eapo_common::registry_helper::write_value(key, name, if value { "true" } else { "false" });
}

fn write_int(key: &str, name: &str, value: i32) {
    let _ = eapo_common::registry_helper::write_dword_value(key, name, value as u32);
}

#[derive(Clone, Debug)]
pub struct GlobalPreferences {
    pub instant_mode: bool,
    pub selected_device: String,
    pub selected_channel_mask: i32,
    pub analysis_start_from: i32,
    pub analysis_channel: String,
    pub analysis_resolution: i32,
    pub analysis_zoom_x: f64,
    pub analysis_zoom_y: f64,
    pub analysis_scroll_x: i32,
    pub analysis_scroll_y: i32,
    pub open_files: Vec<String>,
    pub tab_index: i32,
    pub recent_files: Vec<String>,
    pub window_size: Option<(f32, f32)>,
    pub window_pos: Option<(f32, f32)>,
    pub language: Option<String>,
    pub show_toolbar: bool,
    pub show_analysis: bool,
}

impl Default for GlobalPreferences {
    fn default() -> Self {
        Self {
            instant_mode: true,
            selected_device: String::new(),
            selected_channel_mask: 0,
            analysis_start_from: 0,
            analysis_channel: String::new(),
            analysis_resolution: 65536,
            analysis_zoom_x: 1.0,
            analysis_zoom_y: 1.0,
            analysis_scroll_x: 0,
            analysis_scroll_y: 0,
            open_files: Vec::new(),
            tab_index: 0,
            recent_files: Vec::new(),
            window_size: None,
            window_pos: None,
            language: None,
            show_toolbar: true,
            show_analysis: true,
        }
    }
}

impl GlobalPreferences {
    pub fn load() -> Self {
        let mut p = Self::default();
        p.instant_mode = read_bool(EDITOR_REGPATH, "instantMode", true);
        p.selected_device = eapo_common::registry_helper::read_value(EDITOR_REGPATH, "selectedDevice").unwrap_or_default();
        p.selected_channel_mask = read_int(EDITOR_REGPATH, "selectedChannelMask", 0);
        p.analysis_start_from = read_int(ANALYSIS_REGPATH, "startFrom", 0);
        p.analysis_channel = eapo_common::registry_helper::read_value(ANALYSIS_REGPATH, "channel").unwrap_or_default();
        p.analysis_resolution = read_int(ANALYSIS_REGPATH, "resolution", 65536);
        p.analysis_zoom_x = read_float(ANALYSIS_REGPATH, "zoomX", 1.0);
        p.analysis_zoom_y = read_float(ANALYSIS_REGPATH, "zoomY", 1.0);
        p.analysis_scroll_x = read_int(ANALYSIS_REGPATH, "scrollX", i32::MIN);
        p.analysis_scroll_y = read_int(ANALYSIS_REGPATH, "scrollY", i32::MIN);
        p.open_files = eapo_common::registry_helper::read_multi_value(EDITOR_REGPATH, "openFiles").unwrap_or_default();
        p.tab_index = read_int(EDITOR_REGPATH, "tabIndex", 0);
        p.recent_files = eapo_common::registry_helper::read_multi_value(EDITOR_REGPATH, "recentFiles").unwrap_or_default();
        p.language = eapo_common::registry_helper::read_value(EDITOR_REGPATH, "language").ok();
        p.show_toolbar = read_bool(EDITOR_REGPATH, "showToolbar", true);
        p.show_analysis = read_bool(EDITOR_REGPATH, "showAnalysisPanel", true);
        if let Ok(text) = eapo_common::registry_helper::read_value(EDITOR_REGPATH, "windowSize") {
            if let Some((w, h)) = text.split_once('x') {
                if let (Ok(w), Ok(h)) = (w.parse(), h.parse()) {
                    p.window_size = Some((w, h));
                }
            }
        }
        if let Ok(text) = eapo_common::registry_helper::read_value(EDITOR_REGPATH, "windowPos") {
            if let Some((x, y)) = text.split_once(',') {
                if let (Ok(x), Ok(y)) = (x.parse(), y.parse()) {
                    p.window_pos = Some((x, y));
                }
            }
        }
        p
    }

    pub fn save(&self) {
        let _ = eapo_common::registry_helper::create_key(EDITOR_REGPATH);
        write_bool(EDITOR_REGPATH, "instantMode", self.instant_mode);
        let _ = eapo_common::registry_helper::write_value(EDITOR_REGPATH, "selectedDevice", &self.selected_device);
        write_int(EDITOR_REGPATH, "selectedChannelMask", self.selected_channel_mask);
        let _ = eapo_common::registry_helper::create_key(ANALYSIS_REGPATH);
        write_int(ANALYSIS_REGPATH, "startFrom", self.analysis_start_from);
        let _ = eapo_common::registry_helper::write_value(ANALYSIS_REGPATH, "channel", &self.analysis_channel);
        write_int(ANALYSIS_REGPATH, "resolution", self.analysis_resolution);
        let _ = eapo_common::registry_helper::write_value(ANALYSIS_REGPATH, "zoomX", &self.analysis_zoom_x.to_string());
        let _ = eapo_common::registry_helper::write_value(ANALYSIS_REGPATH, "zoomY", &self.analysis_zoom_y.to_string());
        write_int(ANALYSIS_REGPATH, "scrollX", self.analysis_scroll_x);
        write_int(ANALYSIS_REGPATH, "scrollY", self.analysis_scroll_y);
        let _ = eapo_common::registry_helper::write_multi_value(EDITOR_REGPATH, "openFiles", &self.open_files);
        write_int(EDITOR_REGPATH, "tabIndex", self.tab_index);
        let _ = eapo_common::registry_helper::write_multi_value(EDITOR_REGPATH, "recentFiles", &self.recent_files);
        if let Some((w, h)) = self.window_size {
            let _ = eapo_common::registry_helper::write_value(EDITOR_REGPATH, "windowSize", &format!("{w}x{h}"));
        }
        if let Some((x, y)) = self.window_pos {
            let _ = eapo_common::registry_helper::write_value(EDITOR_REGPATH, "windowPos", &format!("{x},{y}"));
        }
        write_bool(EDITOR_REGPATH, "showToolbar", self.show_toolbar);
        write_bool(EDITOR_REGPATH, "showAnalysisPanel", self.show_analysis);
        if let Some(lang) = &self.language {
            let _ = eapo_common::registry_helper::write_value(EDITOR_REGPATH, "language", lang);
        } else {
            let _ = eapo_common::registry_helper::delete_value(EDITOR_REGPATH, "language");
        }
    }

    pub fn reset_global() {
        if let Ok(groups) = eapo_common::registry_helper::enum_sub_keys(EDITOR_REGPATH) {
            for g in groups {
                if !g.eq_ignore_ascii_case("file-specific") {
                    delete_tree(&format!("{EDITOR_REGPATH}\\{g}"));
                }
            }
        }
        // Also remove child values of the root key, keeping file-specific.
        for name in ["instantMode", "selectedDevice", "selectedChannelMask", "openFiles", "tabIndex", "recentFiles", "windowSize", "windowPos", "language", "geometry", "windowState", "showToolbar", "showAnalysisPanel"] {
            let _ = eapo_common::registry_helper::delete_value(EDITOR_REGPATH, name);
        }
    }

    pub fn reset_file_specific() {
        if let Ok(groups) = eapo_common::registry_helper::enum_sub_keys(EDITOR_PER_FILE_REGPATH) {
            for g in groups {
                delete_tree(&format!("{EDITOR_PER_FILE_REGPATH}\\{g}"));
            }
        }
        for name in ["rowPrefs", "scrollX", "scrollY"] {
            let _ = eapo_common::registry_helper::delete_value(EDITOR_PER_FILE_REGPATH, name);
        }
    }
}

fn delete_tree(key: &str) {
    let Some(sub) = key.strip_prefix(r"HKEY_CURRENT_USER\") else { return };
    let wide: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = windows::Win32::System::Registry::RegDeleteTreeW(
            windows::Win32::System::Registry::HKEY_CURRENT_USER,
            windows::core::PCWSTR(wide.as_ptr()),
        );
    }
}

fn file_pref_key(config_path: &str) -> String {
    format!("{EDITOR_PER_FILE_REGPATH}\\{}", config_path.replace('\\', "|"))
}

/// C++ `FilterTable::setLines`: apply row preferences matched by line number
/// and the command text in front of the colon.
pub fn load_row_preferences(config_path: &str, items: &mut [FilterItem]) {
    if config_path.is_empty() {
        return;
    }
    let key = file_pref_key(config_path);
    let Ok(lines) = eapo_common::registry_helper::read_multi_value(&key, "rowPrefs") else {
        return;
    };
    for line in lines {
        let Some((number, rest)) = line.split_once(':') else { continue };
        let Some((command, json)) = rest.split_once(':') else { continue };
        let Ok(index) = number.parse::<usize>() else { continue };
        if index == 0 || index > items.len() {
            continue;
        }
        let item = &mut items[index - 1];
        if item.command_prefix() != command {
            continue;
        }
        if let Ok(JsonValue::Object(map)) = serde_json::from_str::<JsonValue>(json) {
            item.prefs = map;
            item.load_hotkey_from_prefs();
        }
    }
}

/// C++ `FilterTable::savePreferences` plus the CommentFilterGUI hotkey pref.
pub fn save_row_preferences(config_path: &str, items: &[FilterItem]) {
    if config_path.is_empty() {
        return;
    }
    let mut lines: Vec<String> = Vec::new();
    for (i, item) in items.iter().enumerate() {
        let prefs = item.collect_prefs();
        if prefs.is_empty() {
            continue;
        }
        let json = serde_json::to_string(&JsonValue::Object(prefs)).unwrap_or_else(|_| "{}".into());
        lines.push(format!("{}:{}:{}", i + 1, item.command_prefix(), json));
    }
    let key = file_pref_key(config_path);
    let _ = eapo_common::registry_helper::create_key(&key);
    let _ = eapo_common::registry_helper::write_multi_value(&key, "rowPrefs", &lines);
}

pub fn load_scroll_offsets(config_path: &str) -> (i32, i32) {
    if config_path.is_empty() {
        return (0, 0);
    }
    let key = file_pref_key(config_path);
    (read_int(&key, "scrollX", 0), read_int(&key, "scrollY", 0))
}

pub fn save_scroll_offsets(config_path: &str, x: i32, y: i32) {
    if config_path.is_empty() {
        return;
    }
    let key = file_pref_key(config_path);
    write_int(&key, "scrollX", x);
    write_int(&key, "scrollY", y);
}

/// Adds a file to the MRU list with the same 10-entry cap as MainWindow.
pub fn push_recent(recent: &mut Vec<String>, path: &str) {
    recent.retain(|p| p != path);
    recent.insert(0, path.to_string());
    if recent.len() > 10 {
        recent.truncate(10);
    }
}

pub fn config_dir_from_registry() -> std::path::PathBuf {
    if let Ok(path) = eapo_common::registry_helper::read_value(eapo_common::APP_REGPATH, "ConfigPath") {
        if !path.is_empty() {
            return std::path::PathBuf::from(path);
        }
    }
    std::env::current_dir().unwrap_or_else(|_| Path::new(".").to_path_buf())
}

pub fn native_path(path: &Path) -> String {
    path.to_string_lossy().replace('/', "\\")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::*;

    #[test]
    fn row_pref_format_matches_cpp() {
        let mut it = FilterItem::new(1, "Preamp: -6 dB", &FilterContext {
            config_path: "",
            channel_names: &[],
            channel_mask: 3,
            device_sample_rate: 48000,
        });
        it.hotkey = Some(crate::keyseq::KeySequence::parse("Ctrl+Alt+X").unwrap());
        let prefs = it.collect_prefs();
        let json = serde_json::to_string(&JsonValue::Object(prefs)).unwrap();
        assert!(json.contains("\"hotkey\":\"Ctrl+Alt+X\""));
        let line = format!("1:{}:{}", it.command_prefix(), json);
        let (num, rest) = line.split_once(':').unwrap();
        assert_eq!(num, "1");
        let (command, js) = rest.split_once(':').unwrap();
        assert_eq!(command, "Preamp");
        assert!(js.contains("Ctrl+Alt+X"));
    }

    #[test]
    fn recent_files_cap() {
        let mut r = Vec::new();
        for i in 0..15 {
            push_recent(&mut r, &format!("{i}.txt"));
        }
        assert_eq!(r.len(), 10);
        assert_eq!(r[0], "14.txt");
        push_recent(&mut r, "5.txt");
        assert_eq!(r[0], "5.txt");
        assert_eq!(r.len(), 10);
    }
}
