//! Port of `FilterTable`, `FilterTableRow`, `FilterTableMimeData`,
//! `FilterTemplate` and `FilterTable::updateGuis` including hotkey wiring.

use std::collections::HashSet;
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

use egui::{Color32, Vec2};
use serde_json::Map as JsonMap;

use crate::guis::{self, EditorActions, GuiServices};
use crate::hotkey::HotkeyManager;
use crate::keyseq::KeySequence;
use crate::model::*;
use crate::settings;

static NEXT_UID: AtomicU64 = AtomicU64::new(0x5f00_0000);

fn next_uid() -> u64 {
    NEXT_UID.fetch_add(1, Ordering::Relaxed)
}

#[derive(Clone, Debug)]
pub struct FilterTemplate {
    pub name: String,
    pub line: String,
    pub path: Vec<String>,
}

/// Same catalogue and ordering as the factory list in `FilterTable.cpp`.
pub fn filter_templates() -> Vec<FilterTemplate> {
    let mut t = Vec::new();
    let mut add = |name: &str, line: &str, path: &[&str]| {
        t.push(FilterTemplate {
            name: crate::i18n::tr(name),
            line: line.into(),
            path: path
                .iter()
                .map(|s| crate::i18n::tr(s))
                .collect(),
        });
    };
    add("Comment", "# ", &[]);
    add("Include (Include configuration file)", "Include:", &["Control"]);
    add("Device (Select device)", "Device: all", &["Control"]);
    add("Channel (Select channels)", "Channel: all", &["Control"]);
    add("Stage (Select processing stage)", "Stage: post-mix", &["Control"]);
    add("Preamp (Preamplification)", "Preamp: 0 dB", &["Basic filters"]);
    add("Peaking filter", "Filter: ON PK Fc 100 Hz Gain 0 dB Q 10", &["Parametric filters"]);
    add("Low-pass filter", "Filter: ON LP Fc 100 Hz", &["Parametric filters"]);
    add("High-pass filter", "Filter: ON HP Fc 100 Hz", &["Parametric filters"]);
    add("Band-pass filter", "Filter: ON BP Fc 100 Hz Q 10", &["Parametric filters"]);
    add("Low-shelf filter", "Filter: ON LS Fc 100 Hz Gain 0 dB", &["Parametric filters"]);
    add("High-shelf filter", "Filter: ON HS Fc 100 Hz Gain 0 dB", &["Parametric filters"]);
    add("Notch filter", "Filter: ON NO Fc 100 Hz", &["Parametric filters"]);
    add("All-pass filter", "Filter: ON AP Fc 100 Hz Q 10", &["Parametric filters"]);
    add("Delay", "Delay: 0 ms", &["Basic filters"]);
    add("Copy (Copy between channels)", "Copy: ", &["Basic filters"]);
    add(
        "15-band graphic equalizer",
        "GraphicEQ: 25 0; 40 0; 63 0; 100 0; 160 0; 250 0; 400 0; 630 0; 1000 0; 1600 0; 2500 0; 4000 0; 6300 0; 10000 0; 16000 0",
        &["Graphic equalizers"],
    );
    add(
        "31-band graphic equalizer",
        "GraphicEQ: 20 0; 25 0; 31.5 0; 40 0; 50 0; 63 0; 80 0; 100 0; 125 0; 160 0; 200 0; 250 0; 315 0; 400 0; 500 0; 630 0; 800 0; 1000 0; 1250 0; 1600 0; 2000 0; 2500 0; 3150 0; 4000 0; 5000 0; 6300 0; 8000 0; 10000 0; 12500 0; 16000 0; 20000 0",
        &["Graphic equalizers"],
    );
    add("Graphic equalizer with variable bands", "GraphicEQ: ", &["Graphic equalizers"]);
    add("Convolution (Convolution with impulse response)", "Convolution:", &["Advanced filters"]);
    add("VST plugin", "VSTPlugin:", &["Plugins"]);
    add(
        "Loudness correction",
        "LoudnessCorrection: State 1 ReferenceLevel 0 ReferenceOffset 0 Attenuation 1.0",
        &["Advanced filters"],
    );
    t
}

pub fn add_menu(ui: &mut egui::Ui, templates: &[FilterTemplate]) -> Option<String> {
    let mut selected = None;
    // Root entries first (Comment), then each path group in factory order.
    for t in templates.iter().filter(|t| t.path.is_empty()) {
        if ui.button(&t.name).clicked() {
            selected = Some(t.line.clone());
            ui.close_menu();
        }
    }
    let mut groups: Vec<String> = Vec::new();
    for t in templates {
        if let Some(p) = t.path.first() {
            if !groups.contains(p) {
                groups.push(p.clone());
            }
        }
    }
    for group in groups {
        ui.menu_button(&group, |ui| {
            for t in templates.iter().filter(|t| t.path.first() == Some(&group)) {
                if ui.button(&t.name).clicked() {
                    selected = Some(t.line.clone());
                    ui.close_menu();
                }
            }
        });
    }
    selected
}

#[derive(Clone, Default)]
pub struct TableClipboard {
    pub text: String,
    pub prefs: Vec<JsonMap<String, serde_json::Value>>,
}

pub struct FilterTab {
    pub tab_id: u64,
    pub title: String,
    pub tooltip: String,
    pub config_path: String,
    pub items: Vec<FilterItem>,
    pub selected: HashSet<u64>,
    pub focused: Option<u64>,
    pub selection_start: Option<u64>,
    pub dirty: bool,
    pub hotkeys: HotkeyManager,
    pub last_scroll: Vec2,
    pending_scroll: Option<Vec2>,
    editing: Option<u64>,
    editing_buffer: String,
    editing_started: bool,
}

impl FilterTab {
    pub fn new(
        title: String,
        tooltip: String,
        config_path: String,
        lines: Vec<String>,
        ctx: &FilterContext<'_>,
    ) -> Self {
        let items: Vec<FilterItem> = lines
            .into_iter()
            .map(|line| FilterItem::new(next_uid(), line, ctx))
            .collect();
        let mut tab = Self {
            tab_id: next_uid(),
            title,
            tooltip,
            config_path: config_path.clone(),
            items,
            selected: HashSet::new(),
            focused: None,
            selection_start: None,
            dirty: false,
            hotkeys: HotkeyManager::new(),
            last_scroll: Vec2::ZERO,
            pending_scroll: None,
            editing: None,
            editing_buffer: String::new(),
            editing_started: false,
        };
        if !config_path.is_empty() {
            settings::load_row_preferences(&config_path, &mut tab.items);
            for item in &mut tab.items {
                item.rebuild(ctx);
            }
            let (sx, sy) = settings::load_scroll_offsets(&config_path);
            if sx != 0 || sy != 0 {
                tab.pending_scroll = Some(Vec2::new(sx as f32, sy as f32));
            }
        }
        if let Some(first) = tab.items.first() {
            tab.focused = Some(first.uid);
            tab.selection_start = Some(first.uid);
        }
        tab.re_register_hotkeys();
        tab
    }

    pub fn lines(&self) -> Vec<String> {
        self.items.iter().map(|i| i.line.clone()).collect()
    }

    pub fn mark_saved(&mut self) {
        self.dirty = false;
    }

    pub fn mark_changed(&mut self) {
        self.dirty = true;
    }

    pub fn index_of_uid(&self, uid: u64) -> Option<usize> {
        self.items.iter().position(|i| i.uid == uid)
    }

    pub fn item_by_filter_id(&self, filter_id: &str) -> Option<usize> {
        self.items.iter().position(|i| i.filter_id == filter_id)
    }

    pub fn configure_channels(&mut self, names: &[String], mask: u32, device_sample_rate: u32, config_path: &str) {
        for item in &mut self.items {
            if let Some(ed) = &mut item.editor {
                ed.configure_channels(names, mask);
                if let FilterEditor::Convolution { device_sample_rate: rate, .. } = ed {
                    *rate = device_sample_rate;
                }
                let _ = config_path;
            }
        }
        self.re_register_hotkeys();
    }

    pub fn update_context(&mut self, ctx: &FilterContext<'_>) {
        for item in &mut self.items {
            item.rebuild(ctx);
        }
        for item in &mut self.items {
            if let Some(ed) = &mut item.editor {
                ed.apply_preferences(&item.prefs);
                ed.configure_channels(ctx.channel_names, ctx.channel_mask);
            }
        }
        self.re_register_hotkeys();
    }

    /// C++ `FilterTable::updateGuis` re-registers all hotkeys after clearing.
    pub fn re_register_hotkeys(&mut self) {
        self.hotkeys.clear_all();
        for item in &self.items {
            if item.editor.is_some() && item.hotkey.is_some() {
                self.hotkeys.register_hotkey(&item.filter_id, item.hotkey);
            }
        }
    }

    pub fn register_filter_hotkey(&mut self, filter_id: &str) {
        if let Some(i) = self.item_by_filter_id(filter_id) {
            let hk = self.items[i].hotkey;
            self.hotkeys.register_hotkey(filter_id, hk);
        }
    }

    pub fn set_hotkey(&mut self, filter_id: &str, seq: Option<KeySequence>) -> bool {
        let Some(i) = self.item_by_filter_id(filter_id) else { return false };
        self.items[i].hotkey = seq;
        self.items[i].prefs.remove("hotkey");
        self.register_filter_hotkey(filter_id);
        self.dirty = true;
        match seq {
            Some(_) => self.hotkeys.has_hotkey(filter_id),
            None => true,
        }
    }

    pub fn poll_hotkeys(&mut self) -> Vec<String> {
        self.hotkeys.poll()
    }

    pub fn toggle_by_filter_id(&mut self, filter_id: &str) -> bool {
        let Some(i) = self.item_by_filter_id(filter_id) else { return false };
        if self.items[i].editor.is_none() {
            return false;
        }
        self.items[i].toggle_enabled();
        self.dirty = true;
        true
    }

    pub fn save_preferences(&self) {
        settings::save_row_preferences(&self.config_path, &self.items);
        settings::save_scroll_offsets(&self.config_path, self.last_scroll.x as i32, self.last_scroll.y as i32);
    }

    pub fn select_only(&mut self, uid: u64) {
        self.selected.clear();
        self.selected.insert(uid);
        self.focused = Some(uid);
        self.selection_start = Some(uid);
    }

    pub fn select_all(&mut self) {
        self.selected = self.items.iter().map(|i| i.uid).collect();
    }

    pub fn delete_selected(&mut self) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        let removed_focus = self.focused.filter(|u| self.selected.contains(u)).is_some();
        let removed_start = self.selection_start.filter(|u| self.selected.contains(u)).is_some();
        self.items.retain(|i| !self.selected.contains(&i.uid));
        self.selected.clear();
        if removed_focus {
            self.focused = self.items.first().map(|i| i.uid);
        }
        if removed_start {
            self.selection_start = self.focused;
        }
        self.dirty = true;
        self.re_register_hotkeys();
        true
    }

    pub fn add_line(&mut self, line: String, before: Option<u64>, ctx: &FilterContext<'_>) -> u64 {
        let uid = next_uid();
        let item = FilterItem::new(uid, line, ctx);
        let index = before.and_then(|b| self.index_of_uid(b)).unwrap_or(self.items.len());
        self.items.insert(index, item);
        self.selected.clear();
        self.selected.insert(uid);
        self.focused = Some(uid);
        self.selection_start = Some(uid);
        self.dirty = true;
        self.re_register_hotkeys();
        uid
    }

    pub fn remove_uid(&mut self, uid: u64) -> bool {
        let Some(i) = self.index_of_uid(uid) else { return false };
        self.items.remove(i);
        self.selected.remove(&uid);
        if self.focused == Some(uid) {
            self.focused = self.items.get(i).or_else(|| self.items.last()).map(|i| i.uid);
        }
        if self.selection_start == Some(uid) {
            self.selection_start = self.focused;
        }
        self.dirty = true;
        self.re_register_hotkeys();
        true
    }

    pub fn copy(&self, clipboard: &mut TableClipboard, ctx: &egui::Context) {
        let mut text = String::new();
        let mut prefs = Vec::new();
        let mut first = true;
        for item in &self.items {
            if self.selected.contains(&item.uid) {
                if first { first = false; } else { text.push('\n'); }
                text.push_str(&item.line);
                prefs.push(item.collect_prefs());
            }
        }
        if !self.selected.is_empty() {
            ctx.copy_text(text.clone());
            *clipboard = TableClipboard { text, prefs };
        }
    }

    pub fn cut(&mut self, clipboard: &mut TableClipboard, ctx: &egui::Context) -> bool {
        if self.selected.is_empty() {
            return false;
        }
        self.copy(clipboard, ctx);
        self.delete_selected()
    }

    pub fn paste(&mut self, clipboard: &TableClipboard, ctx: &FilterContext<'_>, system_text: Option<String>) -> bool {
        let text = match system_text {
            Some(t) if !t.is_empty() && t != clipboard.text => t,
            _ => clipboard.text.clone(),
        };
        if text.is_empty() {
            return false;
        }
        let use_prefs = text == clipboard.text && !clipboard.prefs.is_empty();
        let drop_row = self.items.iter().position(|i| self.selected.contains(&i.uid)).unwrap_or(self.items.len());
        let mut lines: Vec<&str> = text.split('\n').collect();
        if lines.last() == Some(&"") {
            lines.pop();
        }
        if lines.is_empty() {
            return false;
        }
        self.selected.clear();
        self.focused = None;
        self.selection_start = None;
        let mut insert = drop_row;
        for (i, line) in lines.iter().enumerate() {
            let uid = next_uid();
            let mut item = FilterItem::new(uid, *line, ctx);
            if use_prefs {
                if let Some(p) = clipboard.prefs.get(i) {
                    item.prefs = p.clone();
                    item.load_hotkey_from_prefs();
                    item.rebuild(ctx);
                }
            }
            self.items.insert(insert, item);
            self.selected.insert(uid);
            if self.focused.is_none() {
                self.focused = Some(uid);
                self.selection_start = Some(uid);
            }
            insert += 1;
        }
        self.dirty = true;
        self.re_register_hotkeys();
        true
    }

    fn select_click(&mut self, uid: u64, modifiers: egui::Modifiers) {
        if modifiers.ctrl {
            if !self.selected.remove(&uid) {
                self.selected.insert(uid);
            }
            self.selection_start = Some(uid);
        } else if modifiers.shift {
            if let Some(start) = self.selection_start.and_then(|s| self.index_of_uid(s)) {
                if let Some(row) = self.index_of_uid(uid) {
                    self.selected.clear();
                    for i in start.min(row)..=start.max(row) {
                        self.selected.insert(self.items[i].uid);
                    }
                }
            }
        } else if !self.selected.contains(&uid) {
            self.selected.clear();
            self.selected.insert(uid);
            self.selection_start = Some(uid);
        } else if self.selected.len() > 1 && self.selection_start == Some(uid) {
            self.selected.clear();
            self.selected.insert(uid);
        } else {
            self.selection_start = Some(uid);
        }
        self.focused = Some(uid);
    }

    fn move_focus(&mut self, delta: isize, shift: bool, ctrl: bool) {
        let Some(focus) = self.focused else { return };
        let Some(row) = self.index_of_uid(focus) else { return };
        let new_row = (row as isize + delta).clamp(0, self.items.len() as isize - 1) as usize;
        let new_uid = self.items[new_row].uid;
        self.focused = Some(new_uid);
        if ctrl {
            // C++ moves the focus without changing the selection.
        } else if shift {
            if let Some(start) = self.selection_start.and_then(|s| self.index_of_uid(s)) {
                self.selected.clear();
                for i in start.min(new_row)..=start.max(new_row) {
                    self.selected.insert(self.items[i].uid);
                }
            }
        } else {
            self.selected.clear();
            self.selected.insert(new_uid);
            self.selection_start = Some(new_uid);
        }
    }

    /// Drag-reorder helper, tested separately.
    fn apply_drop(&mut self, payload: &DragPayload, drop_index: usize, copy: bool, ctx: &FilterContext<'_>) -> bool {
        let selected: HashSet<u64> = payload.uids.iter().copied().collect();
        if selected.is_empty() {
            return false;
        }
        if copy {
            let mut insert = drop_index;
            let source: Vec<FilterItem> = self.items.iter().filter(|i| selected.contains(&i.uid)).cloned().collect();
            self.selected.clear();
            for mut item in source {
                let uid = next_uid();
                item.uid = uid;
                item.filter_id = format!("{uid:x}");
                self.items.insert(insert, item);
                self.selected.insert(uid);
                insert += 1;
            }
        } else {
            self.move_selected(&selected, drop_index);
        }
        self.dirty = true;
        self.re_register_hotkeys();
        let _ = ctx;
        true
    }

    fn move_selected(&mut self, selected: &HashSet<u64>, drop_index: usize) {
        let mut removed_before = 0;
        let mut moving: Vec<FilterItem> = Vec::new();
        let mut kept: Vec<FilterItem> = Vec::with_capacity(self.items.len());
        for (i, item) in self.items.drain(..).enumerate() {
            if selected.contains(&item.uid) {
                if i < drop_index {
                    removed_before += 1;
                }
                moving.push(item);
            } else {
                kept.push(item);
            }
        }
        let insert = drop_index.saturating_sub(removed_before).min(kept.len());
        for (offset, item) in moving.into_iter().enumerate() {
            kept.insert(insert + offset, item);
        }
        self.items = kept;
    }

    pub fn ui(
        &mut self,
        ui: &mut egui::Ui,
        services: &GuiServices<'_>,
        ctx: &FilterContext<'_>,
        clipboard: &mut TableClipboard,
        dialog: &mut crate::hotkey_dialog::HotkeyDialog,
        report: &mut TabUiReport,
        keyboard_enabled: bool,
    ) {
        let mut changed = false;
        let over_plot = ui.ctx().data(|d| d.get_temp::<bool>(egui::Id::new("eapo_plot_hover")).unwrap_or(false));
        let no_text_focus = !ui.ctx().wants_keyboard_input();
        if keyboard_enabled && self.editing.is_none() && no_text_focus {
            if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::C)) {
                self.copy(clipboard, ui.ctx());
            }
            if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::X)) {
                changed |= self.cut(clipboard, ui.ctx());
            }
            if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::V)) {
                let sys = ui.ctx().input(|i| i.events.iter().find_map(|e| match e {
                    egui::Event::Paste(t) => Some(t.clone()),
                    _ => None,
                }));
                let sys = sys.or_else(|| ui.ctx().input(|i| i.events.iter().find_map(|e| match e {
                    egui::Event::Text(t) => Some(t.clone()),
                    _ => None,
                })));
                changed |= self.paste(clipboard, ctx, sys);
            }
            if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::A)) {
                self.select_all();
            }
            if !over_plot && ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete)) {
                changed |= self.delete_selected();
            }
            if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::F2)) {
                if let Some(uid) = self.focused {
                    if let Some(i) = self.index_of_uid(uid) {
                        self.editing = Some(uid);
                        self.editing_buffer = self.items[i].line.clone();
                        self.editing_started = true;
                    }
                }
            }
            if !over_plot {
                let (down, up) = ui.ctx().input_mut(|i| {
                    (i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown), i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp))
                });
                let shift = ui.ctx().input(|i| i.modifiers.shift);
                let ctrl = ui.ctx().input(|i| i.modifiers.ctrl);
                if down { self.move_focus(1, shift, ctrl); }
                if up { self.move_focus(-1, shift, ctrl); }
            }
            if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Space)) {
                if let Some(focus) = self.focused {
                    let ctrl = ui.ctx().input(|i| i.modifiers.ctrl);
                    if !ctrl || !self.selected.remove(&focus) {
                        self.selected.insert(focus);
                    }
                }
            }
        }

        let mut remove_uid = None;
        let mut add_before: Option<(String, Option<u64>)> = None;
        let mut begin_hotkey: Option<String> = None;
        let mut clear_hotkey: Option<String> = None;
        let mut toggle_hotkey: Option<String> = None;
        let mut edit_uid: Option<u64> = None;
        let mut row_rects: Vec<egui::Rect> = Vec::new();
        let mut rebuild_uid: Option<u64> = None;

        let scroll = egui::ScrollArea::vertical().id_salt(("filter_table", self.tab_id));
        let scroll = if let Some(off) = self.pending_scroll.take() {
            scroll.scroll_offset(off)
        } else {
            scroll
        };
        let output = scroll.show(ui, |ui| {
            for index in 0..self.items.len() {
                let uid = self.items[index].uid;
                let is_selected = self.selected.contains(&uid);
                let is_focused = self.focused == Some(uid);
                let bg = if is_selected {
                    if is_focused { Color32::from_rgb(64, 136, 255) } else { Color32::from_rgb(97, 143, 219) }
                } else if is_focused {
                    Color32::from_rgb(160, 160, 160).gamma_multiply(0.35)
                } else {
                    Color32::TRANSPARENT
                };
                let frame = egui::Frame::new()
                    .fill(bg)
                    .inner_margin(egui::Margin::symmetric(2, 3))
                    .corner_radius(5.0);
                let row = frame.show(ui, |ui| {
                    ui.horizontal(|ui| {
                        let number = format!("{}", index + 1);
                        let payload = DragPayload {
                            uids: if is_selected {
                                self.items.iter().filter(|i| self.selected.contains(&i.uid)).map(|i| i.uid).collect()
                            } else {
                                vec![uid]
                            },
                        };
                        let num_resp = ui
                            .dnd_drag_source(egui::Id::new(("row_drag", uid)), payload, |ui| {
                                ui.add_sized([25.0, 18.0], egui::Label::new(egui::RichText::new(number).strong()).sense(egui::Sense::click()))
                            })
                            .inner;
                        if num_resp.clicked() {
                            self.select_click(uid, ui.input(|i| i.modifiers));
                        }
                        num_resp.context_menu(|ui| {
                            self.row_context_menu(ui, uid, index, &mut toggle_hotkey, &mut begin_hotkey, &mut clear_hotkey, &mut edit_uid, &mut remove_uid, &mut add_before);
                        });
                        ui.menu_button(crate::i18n::tr("Add"), |ui| {
                            if let Some(line) = add_menu(ui, &filter_templates()) {
                                add_before = Some((line, Some(uid)));
                                ui.close_menu();
                            }
                        });
                        if ui.small_button(crate::i18n::tr("Remove")).clicked() {
                            remove_uid = Some(uid);
                        }
                        if ui.small_button(crate::i18n::tr("Edit text")).clicked() {
                            edit_uid = Some(uid);
                        }
                        let item = &mut self.items[index];
                        if item.editor.is_some() {
                            let mut power = item.enabled;
                            if ui.selectable_label(power, if power { "On" } else { "Off" }).on_hover_text(crate::i18n::tr("Power on")).clicked() {
                                power = !power;
                                item.set_enabled(power);
                                changed = true;
                            }
                            let tip = match item.hotkey {
                                Some(hk) => format!("Hotkey: {} (click to change)", hk.to_string_qt()),
                                None => "Click to set hotkey".to_string(),
                            };
                            if ui.small_button(crate::i18n::tr("Hotkey")).on_hover_text(tip).clicked() {
                                begin_hotkey = Some(item.filter_id.clone());
                            }
                        }

                        if self.editing == Some(uid) {
                            let resp = ui.add(egui::TextEdit::singleline(&mut self.editing_buffer).desired_width(f32::INFINITY));
                            if self.editing_started {
                                resp.request_focus();
                                self.editing_started = false;
                            }
                            let enter = resp.lost_focus() && ui.input(|i| i.key_pressed(egui::Key::Enter));
                            let lost = resp.lost_focus();
                            let escape = ui.input(|i| i.key_pressed(egui::Key::Escape));
                            if escape {
                                self.editing = None;
                            } else if enter || lost {
                                commit_row_edit(item, &self.editing_buffer);
                                self.editing = None;
                                rebuild_uid = Some(uid);
                                changed = true;
                            }
                        } else if let Some(ed) = &mut item.editor {
                            let mut ed_actions = EditorActions::default();
                            ui.add_enabled_ui(item.enabled, |ui| {
                                guis::show_editor(ui, ed, services, &mut ed_actions);
                            });
                            if ed_actions.open_config.is_some() {
                                report.open_config = ed_actions.open_config.take();
                            }
                            if ed_actions.changed && item.sync_from_editor() {
                                changed = true;
                            }
                        } else {
                            let label = egui::Label::new(&item.line).sense(egui::Sense::click());
                            let resp = ui.add(label);
                            if resp.clicked() {
                                self.select_click(uid, ui.input(|i| i.modifiers));
                            }
                            if resp.double_clicked() {
                                edit_uid = Some(uid);
                            }
                        }
                    });
                });
                row_rects.push(row.response.rect);
            }

            ui.horizontal(|ui| {
                ui.menu_button(crate::i18n::tr("Add filter"), |ui| {
                    if let Some(line) = add_menu(ui, &filter_templates()) {
                        add_before = Some((line, None));
                        ui.close_menu();
                    }
                });
            });
        });
        self.last_scroll = output.state.offset;

        // Global drag-and-drop release over the table.
        if ui.ctx().input(|i| i.pointer.any_released()) {
            if let Some(payload) = egui::DragAndDrop::take_payload::<DragPayload>(ui.ctx()) {
                if let Some(pos) = ui.ctx().input(|i| i.pointer.interact_pos()) {
                    let table_rect = row_rects.first().map(|r| {
                        let mut rr = *r;
                        for r in &row_rects { rr = rr.union(*r); }
                        rr
                    });
                    if table_rect.map(|r| r.expand(10.0).contains(pos)).unwrap_or(false) {
                        let mut drop_index = self.items.len();
                        for (i, r) in row_rects.iter().enumerate() {
                            if pos.y <= r.center().y {
                                drop_index = i;
                                break;
                            }
                        }
                        let copy = ui.ctx().input(|i| i.modifiers.ctrl);
                        if self.apply_drop(&payload, drop_index, copy, ctx) {
                            changed = true;
                        }
                        ui.ctx().request_repaint();
                    }
                }
            }
        } else if let Some(pos) = ui.ctx().input(|i| i.pointer.hover_pos()) {
            // Insertion marker.
            if egui::DragAndDrop::has_any_payload(ui.ctx()) {
                if let Some(last) = row_rects.last() {
                    let mut y = last.bottom();
                    let mut x = last.left();
                    for r in &row_rects {
                        if pos.y <= r.center().y {
                            y = r.top();
                            x = r.left();
                            break;
                        }
                    }
                    ui.painter().line_segment(
                        [egui::pos2(x, y), egui::pos2(last.right(), y)],
                        egui::Stroke::new(2.0, Color32::LIGHT_BLUE),
                    );
                }
            }
        }

        if let Some(uid) = edit_uid {
            if let Some(i) = self.index_of_uid(uid) {
                self.editing = Some(uid);
                self.editing_buffer = self.items[i].line.clone();
                self.editing_started = true;
            }
        }
        if let Some((line, before)) = add_before {
            let uid = self.add_line(line, before, ctx);
            self.select_only(uid);
            changed = true;
        }
        if let Some(uid) = rebuild_uid {
            if let Some(i) = self.index_of_uid(uid) {
                self.items[i].rebuild(ctx);
                self.re_register_hotkeys();
            }
        }
        if let Some(uid) = remove_uid {
            changed |= self.remove_uid(uid);
        }
        if let Some(filter_id) = toggle_hotkey {
            if self.toggle_by_filter_id(&filter_id) {
                changed = true;
            }
        }
        if let Some(filter_id) = begin_hotkey {
            let hk = self
                .item_by_filter_id(&filter_id)
                .and_then(|i| self.items[i].hotkey);
            dialog.begin(filter_id, hk);
        }
        if let Some(filter_id) = clear_hotkey {
            if self.set_hotkey(&filter_id, None) {
                changed = true;
                report.hotkey_changed = true;
            }
        }

        report.changed |= changed;
    }

    fn row_context_menu(
        &mut self,
        ui: &mut egui::Ui,
        uid: u64,
        index: usize,
        toggle_hotkey: &mut Option<String>,
        begin_hotkey: &mut Option<String>,
        clear_hotkey: &mut Option<String>,
        edit_uid: &mut Option<u64>,
        remove_uid: &mut Option<u64>,
        add_before: &mut Option<(String, Option<u64>)>,
    ) {
        if !self.selected.contains(&uid) {
            self.select_only(uid);
        }
        let enabled = self.items[index].enabled;
        if self.items[index].editor.is_some()
            && ui.button(if enabled { "Disable filter" } else { "Enable filter" }).clicked()
        {
            let filter_id = self.items[index].filter_id.clone();
            *toggle_hotkey = Some(filter_id);
            ui.close_menu();
        }
        if self.items[index].editor.is_some() {
            let has = self.items[index].hotkey.is_some();
            if ui.button(crate::i18n::tr("Set Hotkey...")).clicked() {
                *begin_hotkey = Some(self.items[index].filter_id.clone());
                ui.close_menu();
            }
            if has && ui.button(crate::i18n::tr("Clear Hotkey")).clicked() {
                *clear_hotkey = Some(self.items[index].filter_id.clone());
                ui.close_menu();
            }
            let text = match self.items[index].hotkey {
                Some(hk) => format!("Hotkey: {}", hk.to_string_qt()),
                None => "Hotkey: None".into(),
            };
            ui.label(text);
        }
        if ui.button(crate::i18n::tr("Edit text")).clicked() {
            *edit_uid = Some(uid);
            ui.close_menu();
        }
        if ui.button(crate::i18n::tr("Add filter")).clicked() {
            if let Some(line) = add_menu(ui, &filter_templates()) {
                *add_before = Some((line, Some(uid)));
            }
            ui.close_menu();
        }
        if ui.button(crate::i18n::tr("Remove")).clicked() {
            *remove_uid = Some(uid);
            ui.close_menu();
        }
    }
}

fn commit_row_edit(item: &mut FilterItem, text: &str) {
    if text == item.line {
        return;
    }
    item.line = text.to_string();
    let s = parse_line(text);
    item.indent = s.indent.to_string();
    item.enabled = s.enabled;
    // Rebuilding the GUI is done by the caller through update_context on the
    // next frame; here we can at least drop a stale editor.
    item.editor = None;
    item.kind = s.kind;
}

#[derive(Default)]
pub struct TabUiReport {
    pub changed: bool,
    pub hotkey_changed: bool,
    pub open_config: Option<String>,
}

#[derive(Clone, Debug)]
struct DragPayload {
    uids: Vec<u64>,
}

/// Pure helper mirroring `FilterTable::dropEvent` move semantics.
pub fn reorder_uids(ids: &mut Vec<u64>, selected: &HashSet<u64>, drop_row: usize, copy: bool) -> Option<Vec<u64>> {
    if copy {
        return Some(selected.iter().copied().collect());
    }
    let mut removed_before = 0;
    let mut moving = Vec::new();
    let mut kept = Vec::new();
    for (i, id) in ids.drain(..).enumerate() {
        if selected.contains(&id) {
            if i < drop_row {
                removed_before += 1;
            }
            moving.push(id);
        } else {
            kept.push(id);
        }
    }
    let insert = drop_row.saturating_sub(removed_before).min(kept.len());
    for (o, id) in moving.iter().enumerate() {
        kept.insert(insert + o, *id);
    }
    *ids = kept;
    None
}

pub fn save_config_file(path: &str, lines: &[String]) -> std::io::Result<()> {
    let mut text = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i != 0 {
            text.push_str("\r\n");
        }
        text.push_str(line);
    }
    let bytes = text.into_bytes();
    let mut attempts = 0;
    loop {
        match std::fs::write(path, &bytes) {
            Ok(()) => return Ok(()),
            Err(e) => {
                let sharing = e.raw_os_error() == Some(32);
                attempts += 1;
                if !sharing || attempts > 2000 {
                    return Err(e);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    }
}

pub fn load_config_lines(path: &str) -> std::io::Result<Vec<String>> {
    let mut attempts = 0;
    let bytes = loop {
        match std::fs::read(path) {
            Ok(b) => break b,
            Err(e) => {
                let sharing = e.raw_os_error() == Some(32);
                attempts += 1;
                if !sharing || attempts > 2000 {
                    return Err(e);
                }
                std::thread::sleep(std::time::Duration::from_millis(1));
            }
        }
    };
    let text = match String::from_utf8(bytes) {
        Ok(t) => t,
        Err(e) => String::from_utf8_lossy(e.as_bytes()).into_owned(),
    };
    let mut lines = Vec::new();
    for line in text.split('\n') {
        lines.push(line.strip_suffix('\r').unwrap_or(line).to_string());
    }
    if lines.is_empty() {
        lines.push(String::new());
    }
    Ok(lines)
}

pub fn file_title(path: &str) -> String {
    Path::new(path)
        .file_name()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "Unsaved".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drag_reorder_before_row() {
        let mut ids = vec![1, 2, 3, 4, 5];
        let selected: HashSet<u64> = [2u64, 4].into_iter().collect();
        reorder_uids(&mut ids, &selected, 0, false);
        assert_eq!(ids, vec![2, 4, 1, 3, 5]);
    }

    #[test]
    fn drag_reorder_to_end() {
        let mut ids = vec![1, 2, 3, 4];
        let selected: HashSet<u64> = [1u64, 3].into_iter().collect();
        reorder_uids(&mut ids, &selected, 4, false);
        assert_eq!(ids, vec![2, 4, 1, 3]);
    }

    #[test]
    fn drag_reorder_down_adjusts_insert() {
        let mut ids = vec![1, 2, 3, 4, 5];
        let selected: HashSet<u64> = [1u64].into_iter().collect();
        reorder_uids(&mut ids, &selected, 4, false);
        assert_eq!(ids, vec![2, 3, 4, 1, 5]);
    }

    #[test]
    fn save_uses_crlf_and_preserves_trailing_line() {
        let dir = std::env::temp_dir().join("eapo_editor_test");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("config.txt");
        let path_str = path.to_string_lossy().into_owned();
        let lines = vec!["Preamp: -6 dB".to_string(), "".to_string()];
        save_config_file(&path_str, &lines).unwrap();
        let bytes = std::fs::read(&path).unwrap();
        assert_eq!(bytes, b"Preamp: -6 dB\r\n");
        let loaded = load_config_lines(&path_str).unwrap();
        assert_eq!(loaded, lines);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
