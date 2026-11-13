//! Row table construction and structural parsing.

use super::*;

impl App {
    pub(super) fn set_lines(&mut self, lines: Vec<String>) {
        let mut lines = lines;
        retain_preamp_only(&mut lines);
        self.rows.clear();
        self.rows.reserve(lines.len());
        let row_state = self
            .doc_path
            .as_ref()
            .map(|path| settings::load_row_state(&path.to_string_lossy()))
            .unwrap_or_default();
        for (position, line) in lines.into_iter().enumerate() {
            let structural = parse_line(&line);
            let kind = structural.kind;
            let enabled = structural.enabled;
            let height = row_height(kind, self.dpi);
            let uid = self.next_uid;
            self.next_uid += 1;
            let command = line.split(':').next().unwrap_or("").trim();
            let mut prefs = row_state
                .get(&(position + 1))
                .filter(|(stored, _)| stored == command)
                .map(|(_, prefs)| prefs.clone())
                .unwrap_or_default();
            prefs.remove("hotkey");
            self.rows.push(Row {
                uid,
                line,
                kind,
                height,
                enabled,
                item: None,
                hotkey: None,
                prefs,
                geq_table_top: 0,
            });
        }
        self.recompute_prefix();
        self.offset = 0;
    }

    pub(super) fn recompute_prefix(&mut self) {
        self.prefix.clear();
        self.prefix.reserve(self.rows.len() + 1);
        self.prefix.push(0);
        for row in &self.rows {
            let last = *self.prefix.last().unwrap();
            self.prefix.push(last + row.height);
        }
        self.total = *self.prefix.last().unwrap();
        if let Some(selected) = self.selected {
            self.selected = (selected < self.rows.len()).then_some(selected);
        }
    }

    pub(super) fn rebuild_rows(&mut self) {
        for row in &mut self.rows {
            row.height = row_height(row.kind, self.dpi);
        }
        self.recompute_prefix();
    }

    pub(super) fn ensure_item(&mut self, index: usize) {
        if index >= self.rows.len() || self.rows[index].item.is_some() {
            return;
        }
        let names = self.channel_names.clone();
        let sample_rate = self
            .selected_device
            .as_ref()
            .map(|device| device.get_sample_rate())
            .unwrap_or(48000);
        let context = FilterContext {
            config_path: "",
            channel_names: &names,
            channel_mask: self.channel_mask,
            device_sample_rate: sample_rate,
        };
        let uid = self.rows[index].uid;
        let kind = self.rows[index].kind;
        let line = self.rows[index].line.clone();
        let prefs = self.rows[index].prefs.clone();
        let mut item = FilterItem::new(uid, line, &context);
        item.prefs = prefs;
        item.rebuild(&context);
        if kind == FilterKind::Loudness {
            if let Some(volume) = self.master_volume {
                if let Some(FilterEditor::Loudness {
                    volume: shown, ..
                }) = item.editor.as_mut()
                {
                    *shown = volume;
                }
            }
        }
        self.rows[index].item = Some(Box::new(item));
    }

    /// Cached accessibility check for a VST plugin path.  The ACL query is
    /// expensive and used to run on every repaint of the row.
    pub(super) fn vst_readable(&mut self, path: &str) -> Option<bool> {
        if path.is_empty() {
            return None;
        }
        if let Some(value) = self.vst_readable_cache.get(path) {
            return *value;
        }
        let path_ref = std::path::Path::new(path);
        let absolute = if path_ref.is_absolute() {
            path_ref.to_path_buf()
        } else {
            std::path::Path::new(
                &eapo_filters::vst_plugin_library::VSTPluginLibrary::get_default_plugin_path(),
            )
            .join(path_ref)
        };
        let value = if absolute.exists() {
            vst_editor::audio_service_readable(&absolute)
        } else {
            None
        };
        self.vst_readable_cache.insert(path.to_string(), value);
        value
    }

    pub(super) fn sync_item(&mut self, hwnd: HWND, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        let Some((line, prefs)) = self.rows[index].item.as_mut().map(|item| {
            item.sync_from_editor();
            let line = item.line.clone();
            let mut prefs = item.collect_prefs();
            prefs.remove("hotkey");
            (line, prefs)
        }) else {
            return;
        };
        self.rows[index].prefs = prefs;
        self.apply_row_line(index, line);
        self.dirty = true;
        unsafe {
            self.update_title(hwnd);
        }
    }

    pub(super) unsafe fn prune_items(&mut self, hwnd: HWND) {
        if self.rows.len() < 512 {
            return;
        }
        let viewport = self.viewport_height(hwnd);
        let start = (self.offset.max(0) as u32).saturating_sub(200);
        let end = (self.offset.max(0) as u32)
            .saturating_add(viewport.max(0) as u32)
            .saturating_add(400);
        for (index, row) in self.rows.iter_mut().enumerate() {
            let row_start = self.prefix[index];
            let row_end = self.prefix[index + 1];
            let keep = self.edit.as_ref().map(|edit| edit.row) == Some(index);
            if !keep && (row_end < start || row_start > end) {
                row.item = None;
            }
        }
        let _ = hwnd;
    }

}
