//! Row editing, selection changes and clipboard operations.

use super::*;

impl App {
    pub(super) fn apply_row_line(&mut self, index: usize, line: String) {
        if !is_preamp_only_line(&line) {
            return;
        }
        let structural = parse_line(&line);
        let kind = structural.kind;
        let enabled = structural.enabled;
        let dpi = self.dpi;
        let row = &mut self.rows[index];
        row.line = line;
        row.kind = kind;
        row.enabled = enabled;
        row.height = row_height(kind, dpi);
        self.recompute_prefix();
    }

    pub(super) unsafe fn toggle_row(&mut self, hwnd: HWND, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        let names = vec!["L".to_string(), "R".to_string()];
        let ctx = FilterContext {
            config_path: "",
            channel_names: &names,
            channel_mask: 3,
            device_sample_rate: 48000,
        };
        let mut item = FilterItem::new(0, self.rows[index].line.clone(), &ctx);
        item.toggle_enabled();
        // Keep the materialized editor in sync: otherwise a later slider drag
        // recomposes the line from a stale `enabled` flag and comments the row
        // out again ("adjusting the slider disables the filter").
        if let Some(existing) = self.rows[index].item.as_mut() {
            existing.line = item.line.clone();
            existing.enabled = item.enabled;
        }
        self.apply_row_line(index, item.line);
        self.dirty = true;
        self.update_title(hwnd);
        self.autosave(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    pub(super) unsafe fn ensure_visible(&mut self, hwnd: HWND, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        let viewport = self.viewport_height(hwnd);
        let top = self.prefix[index] as i32;
        let bottom = self.prefix[index + 1] as i32;
        let mut offset = self.offset;
        if top < offset {
            offset = top;
        }
        if bottom > offset + viewport {
            offset = bottom - viewport;
        }
        if offset != self.offset {
            self.set_offset(hwnd, offset);
        }
    }

    pub(super) unsafe fn begin_edit(&mut self, hwnd: HWND, index: usize) {
        if index >= self.rows.len() {
            return;
        }
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        self.ensure_visible(hwnd, index);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let chrome = self.chrome_height();
        let top = chrome + self.prefix[index] as i32 - self.offset;
        let height = (self.rows[index].height as i32).max(scale(22, self.dpi));
        let mut point = POINT {
            x: scale(40, self.dpi),
            y: top,
        };
        let _ = ClientToScreen(hwnd, &mut point);
        let width = (client.right - scale(48, self.dpi)).max(scale(80, self.dpi));
        let text = wide_z(&self.rows[index].line);
        let style = WINDOW_STYLE(
            WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32,
        );
        let edit = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            PCWSTR(text.as_ptr()),
            style,
            point.x,
            point.y,
            width,
            height,
            Some(hwnd),
            None,
            None,
            None,
        ) {
            Ok(edit) => edit,
            Err(_) => return,
        };
        SendMessageW(
            edit,
            WM_SETFONT,
            Some(WPARAM(self.body.0 as usize)),
            Some(LPARAM(1)),
        );
        let previous = SetWindowLongPtrW(edit, GWLP_WNDPROC, edit_proc as *const () as isize);
        SetWindowLongPtrW(edit, GWLP_USERDATA, previous);
        self.edit = Some(EditState { row: index, hwnd: edit });
        let _ = SetFocus(Some(edit));
    }

    pub(super) unsafe fn commit_edit(&mut self, hwnd: HWND, apply: bool) {
        let Some(edit) = self.edit.take() else {
            return;
        };
        if apply {
            let capacity = GetWindowTextLengthW(edit.hwnd).max(0) as usize + 1;
            let mut buffer = vec![0u16; capacity];
            let length = GetWindowTextW(edit.hwnd, &mut buffer).max(0) as usize;
            let text = String::from_utf16_lossy(&buffer[..length.min(buffer.len())]);
            if edit.row < self.rows.len() && self.rows[edit.row].line != text {
                self.apply_row_line(edit.row, text);
                // A raw text edit is authoritative; force a re-parse so the
                // next slider edit does not overwrite it with stale state.
                self.rows[edit.row].item = None;
                self.dirty = true;
                self.update_title(hwnd);
                self.update_scrollbar(hwnd);
                self.save_hotkeys();
                self.autosave(hwnd);
            }
        }
        let _ = DestroyWindow(edit.hwnd);
        let _ = SetFocus(Some(hwnd));
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    pub(super) unsafe fn begin_value_edit(&mut self, hwnd: HWND, row: usize, field: ValueField) {
        self.commit_value_edit(hwnd, true);
        if row >= self.rows.len() {
            return;
        }
        self.ensure_item(row);
        let Some(item) = self.rows[row].item.as_ref() else {
            return;
        };
        let Some(editor) = item.editor.as_ref() else {
            return;
        };
        let dpi = self.dpi;
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let top = self.chrome_height() + self.prefix[row] as i32 - self.offset;
        let (rect, text) = match (field, editor) {
            (ValueField::PreampDb, FilterEditor::Preamp { db }) => {
                (preamp_value_rect(dpi, top), format!("{db}"))
            }
            (ValueField::BiquadFreq, FilterEditor::Biquad(biquad)) => {
                (biquad_field_rects(dpi, top)[1], format!("{:.1}", biquad.freq))
            }
            (ValueField::BiquadGain, FilterEditor::Biquad(biquad)) => {
                (biquad_field_rects(dpi, top)[2], format!("{:.1}", biquad.gain))
            }
            (ValueField::BiquadQ, FilterEditor::Biquad(biquad)) => {
                (biquad_field_rects(dpi, top)[3], format!("{:.2}", biquad.q))
            }
            (ValueField::Delay, FilterEditor::Delay { delay, .. }) => {
                (delay_field_rects(dpi, top)[0], format!("{delay:.2}"))
            }
            (ValueField::LoudnessRef, FilterEditor::Loudness { ref_level, .. }) => {
                (loudness_field_rects(dpi, top)[0], format!("{ref_level}"))
            }
            (ValueField::LoudnessOffset, FilterEditor::Loudness { ref_offset, .. }) => {
                (loudness_field_rects(dpi, top)[1], format!("{ref_offset}"))
            }
            (ValueField::LoudnessAtt, FilterEditor::Loudness { att, .. }) => {
                (loudness_field_rects(dpi, top)[2], format!("{att:.2}"))
            }
            (ValueField::GeqFreq(index), FilterEditor::GraphicEq(graphic)) => {
                let table = geq_table_rect(dpi, top, self.rows[row].height as i32, client.right);
                let visible = geq_table_visible_rows(dpi, &table);
                let table_top = geq_table_top_clamped(
                    self.rows[row].geq_table_top,
                    graphic.nodes.len(),
                    visible,
                );
                let Some(node) = graphic.nodes.get(index) else {
                    return;
                };
                let Some(cell) = geq_table_cell_rect(
                    dpi,
                    &table,
                    table_top,
                    visible,
                    graphic.nodes.len(),
                    index,
                    0,
                ) else {
                    return;
                };
                (cell, format!("{}", node.freq))
            }
            (ValueField::GeqGain(index), FilterEditor::GraphicEq(graphic)) => {
                let table = geq_table_rect(dpi, top, self.rows[row].height as i32, client.right);
                let visible = geq_table_visible_rows(dpi, &table);
                let table_top = geq_table_top_clamped(
                    self.rows[row].geq_table_top,
                    graphic.nodes.len(),
                    visible,
                );
                let Some(node) = graphic.nodes.get(index) else {
                    return;
                };
                let Some(cell) = geq_table_cell_rect(
                    dpi,
                    &table,
                    table_top,
                    visible,
                    graphic.nodes.len(),
                    index,
                    1,
                ) else {
                    return;
                };
                (cell, format!("{}", node.db_gain))
            }
            _ => return,
        };
        let text = wide_z(&text);
        let edit = match CreateWindowExW(
            WINDOW_EX_STYLE(0),
            w!("EDIT"),
            PCWSTR(text.as_ptr()),
            WINDOW_STYLE(WS_CHILD.0 | WS_VISIBLE.0 | WS_BORDER.0 | ES_AUTOHSCROLL as u32),
            rect.left,
            rect.top,
            rect.right - rect.left,
            rect.bottom - rect.top,
            Some(hwnd),
            None,
            None,
            None,
        ) {
            Ok(edit) => edit,
            Err(_) => return,
        };
        SendMessageW(
            edit,
            WM_SETFONT,
            Some(WPARAM(self.body.0 as usize)),
            Some(LPARAM(1)),
        );
        let previous = SetWindowLongPtrW(edit, GWLP_WNDPROC, value_edit_proc as *const () as isize);
        SetWindowLongPtrW(edit, GWLP_USERDATA, previous);
        SendMessageW(edit, 0x00B1, Some(WPARAM(0)), Some(LPARAM(-1)));
        self.value_edit = Some(ValueEdit {
            row,
            hwnd: edit,
            field,
        });
        let _ = SetFocus(Some(edit));
    }

    pub(super) unsafe fn commit_value_edit(&mut self, hwnd: HWND, apply: bool) {
        let Some(edit) = self.value_edit.take() else {
            return;
        };
        if apply {
            let mut buffer = [0u16; 64];
            let length = GetWindowTextW(edit.hwnd, &mut buffer).max(0) as usize;
            let text = String::from_utf16_lossy(&buffer[..length.min(buffer.len())]);
            if let Some(value) = eapo_editor_core::model::parse_num(&text) {
                let mut changed = false;
                if let Some(item) = self
                    .rows
                    .get_mut(edit.row)
                    .and_then(|row| row.item.as_mut())
                {
                    if let Some(editor) = item.editor.as_mut() {
                        match (edit.field, editor) {
                            (ValueField::PreampDb, FilterEditor::Preamp { db }) => {
                                let value = value.clamp(-100.0, 100.0);
                                if *db != value {
                                    *db = value;
                                    changed = true;
                                }
                            }
                            (ValueField::BiquadFreq, FilterEditor::Biquad(biquad)) => {
                                let value = value.clamp(1.0, 30000.0);
                                if biquad.freq != value {
                                    biquad.freq = value;
                                    changed = true;
                                }
                            }
                            (ValueField::BiquadGain, FilterEditor::Biquad(biquad)) => {
                                let value = value.clamp(-30.0, 30.0);
                                if biquad.gain != value {
                                    biquad.gain = value;
                                    changed = true;
                                }
                            }
                            (ValueField::BiquadQ, FilterEditor::Biquad(biquad)) => {
                                let value = value.clamp(0.1, 100.0);
                                if biquad.q != value {
                                    biquad.q = value;
                                    changed = true;
                                }
                            }
                            (ValueField::Delay, FilterEditor::Delay { delay, .. }) => {
                                // Samples can legitimately exceed the drag range
                                // (e.g. 4800 samples = 100 ms), so only guard
                                // against negative/huge values here.
                                let value = value.clamp(0.0, 1_000_000.0);
                                if *delay != value {
                                    *delay = value;
                                    changed = true;
                                }
                            }
                            (ValueField::LoudnessRef, FilterEditor::Loudness { ref_level, .. }) => {
                                let value = value.clamp(-999.0, 999.0).round() as i32;
                                if *ref_level != value {
                                    *ref_level = value;
                                    changed = true;
                                }
                            }
                            (
                                ValueField::LoudnessOffset,
                                FilterEditor::Loudness { ref_offset, .. },
                            ) => {
                                let value = value.clamp(-999.0, 999.0).round() as i32;
                                if *ref_offset != value {
                                    *ref_offset = value;
                                    changed = true;
                                }
                            }
                            (ValueField::LoudnessAtt, FilterEditor::Loudness { att, .. }) => {
                                let value = (value.clamp(0.0, 1.0) * 100.0).round() / 100.0;
                                if *att != value {
                                    *att = value;
                                    changed = true;
                                }
                            }
                            (ValueField::GeqFreq(index), FilterEditor::GraphicEq(graphic)) => {
                                if let Some(node) = graphic.nodes.get_mut(index) {
                                    let value = value.clamp(1.0, 30000.0);
                                    if (node.freq - value).abs() > 1e-9 {
                                        let db_gain = node.db_gain;
                                        node.freq = value;
                                        graphic.sort_nodes();
                                        graphic.selected = graphic
                                            .nodes
                                            .iter()
                                            .position(|candidate| {
                                                (candidate.freq - value).abs() < 1e-9
                                                    && (candidate.db_gain - db_gain).abs() < 1e-9
                                            })
                                            .map(|position| vec![position])
                                            .unwrap_or_default();
                                        changed = true;
                                    }
                                }
                            }
                            (ValueField::GeqGain(index), FilterEditor::GraphicEq(graphic)) => {
                                if let Some(node) = graphic.nodes.get_mut(index) {
                                    let value =
                                        value.clamp(GEQ_WORLD_MIN_DB, GEQ_WORLD_MAX_DB);
                                    if (node.db_gain - value).abs() > 1e-9 {
                                        node.db_gain = value;
                                        changed = true;
                                    }
                                }
                            }
                            _ => {}
                        }
                    }
                }
                if changed {
                    self.sync_item(hwnd, edit.row);
                    self.save_hotkeys();
                    self.start_analysis();
                    self.autosave(hwnd);
                }
            }
        }
        let _ = DestroyWindow(edit.hwnd);
        let _ = SetFocus(Some(hwnd));
        self.invalidate_row(hwnd, edit.row);
        if matches!(
            edit.field,
            ValueField::GeqFreq(_) | ValueField::GeqGain(_)
        ) {
            self.ensure_geq_table_visible(hwnd, edit.row);
        }
    }

    /// Scrolls the GraphicEQ node table so the selected node stays visible.
    pub(super) unsafe fn ensure_geq_table_visible(&mut self, hwnd: HWND, row: usize) {
        if row >= self.rows.len() {
            return;
        }
        self.ensure_item(row);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let row_y = self.chrome_height() + self.prefix[row] as i32 - self.offset;
        let table = geq_table_rect(self.dpi, row_y, self.rows[row].height as i32, client.right);
        let visible = geq_table_visible_rows(self.dpi, &table);
        let (nodes, selected) = match self.rows[row].item.as_ref().and_then(|item| item.editor.as_ref()) {
            Some(FilterEditor::GraphicEq(graphic)) => {
                (graphic.nodes.len(), graphic.selected.first().copied())
            }
            _ => return,
        };
        let Some(selected) = selected else {
            return;
        };
        let current = geq_table_top_clamped(self.rows[row].geq_table_top, nodes, visible);
        let mut top = current;
        if selected < current {
            top = selected;
        } else if visible > 0 && selected >= current + visible {
            top = selected + 1 - visible;
        }
        top = geq_table_top_clamped(top, nodes, visible);
        if top != self.rows[row].geq_table_top {
            self.rows[row].geq_table_top = top;
            self.invalidate_row(hwnd, row);
        }
    }

    pub(super) unsafe fn add_row(&mut self, hwnd: HWND, line: &str, before: Option<usize>) {
        if !is_preamp_only_line(line) {
            return;
        }
        self.commit_edit(hwnd, true);
        let structural = parse_line(line);
        let height = row_height(structural.kind, self.dpi);
        let index = before.unwrap_or(self.rows.len()).min(self.rows.len());
        let uid = self.next_uid;
        self.next_uid += 1;
        self.rows.insert(
            index,
            Row {
                uid,
                line: line.to_string(),
                kind: structural.kind,
                height,
                enabled: structural.enabled,
                item: None,
                hotkey: None,
                prefs: serde_json::Map::new(),
                geq_table_top: 0,
            },
        );
        self.selected = Some(index);
        self.dirty = true;
        self.recompute_prefix();
        self.update_scrollbar(hwnd);
        self.ensure_visible(hwnd, index);
        self.update_title(hwnd);
        self.save_hotkeys();
        self.autosave(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    pub(super) unsafe fn remove_selected(&mut self, hwnd: HWND) {
        let mut indices: Vec<usize> = self.selection.iter().copied().collect();
        if indices.is_empty() {
            if let Some(index) = self.selected {
                indices.push(index);
            }
        }
        if indices.is_empty() {
            return;
        }
        self.commit_edit(hwnd, true);
        indices.sort_unstable();
        indices.dedup();
        let first = indices[0].min(self.rows.len().saturating_sub(1));
        for index in indices.into_iter().rev() {
            if index < self.rows.len() {
                self.rows.remove(index);
            }
        }
        self.selected = if self.rows.is_empty() {
            None
        } else {
            Some(first.min(self.rows.len() - 1))
        };
        self.selection = self.selected.into_iter().collect();
        self.selection_anchor = self.selected;
        self.dirty = true;
        self.recompute_prefix();
        self.update_scrollbar(hwnd);
        self.save_hotkeys();
        self.reload_hotkeys();
        self.update_title(hwnd);
        self.start_analysis();
        self.autosave(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    pub(super) fn copy_selection(&self) -> bool {
        let mut indices: Vec<usize> = self.selection.iter().copied().collect();
        if indices.is_empty() {
            if let Some(index) = self.selected {
                indices.push(index);
            }
        }
        if indices.is_empty() {
            return false;
        }
        indices.sort_unstable();
        let text = indices
            .iter()
            .filter_map(|index| self.rows.get(*index))
            .map(|row| row.line.as_str())
            .collect::<Vec<_>>()
            .join("\n");
        unsafe { clipboard::set_text(&text) }
    }

    pub(super) unsafe fn cut_selection(&mut self, hwnd: HWND) -> bool {
        if self.copy_selection() {
            self.remove_selected(hwnd);
            true
        } else {
            false
        }
    }

    pub(super) unsafe fn paste_clipboard(&mut self, hwnd: HWND) -> bool {
        let Some(text) = clipboard::get_text() else {
            return false;
        };
        let mut lines: Vec<String> = text
            .split('\n')
            .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
            .collect();
        lines.retain(|line| is_preamp_only_line(line));
        if lines.is_empty() {
            return false;
        }
        self.commit_edit(hwnd, true);
        let insert = self
            .selected
            .unwrap_or(self.rows.len())
            .min(self.rows.len());
        let mut inserted = Vec::with_capacity(lines.len());
        for (offset, line) in lines.drain(..).enumerate() {
            let (kind, enabled) = {
                let structural = parse_line(&line);
                (structural.kind, structural.enabled)
            };
            let height = row_height(kind, self.dpi);
            let uid = self.next_uid;
            self.next_uid += 1;
            self.rows.insert(
                insert + offset,
                Row {
                    uid,
                    line,
                    kind,
                    height,
                    enabled,
                    item: None,
                    hotkey: None,
                    prefs: serde_json::Map::new(),
                    geq_table_top: 0,
                },
            );
            inserted.push(insert + offset);
        }
        self.selection = inserted.iter().copied().collect();
        self.selected = inserted.last().copied();
        self.selection_anchor = inserted.first().copied();
        self.dirty = true;
        self.recompute_prefix();
        self.update_scrollbar(hwnd);
        self.update_title(hwnd);
        self.save_hotkeys();
        self.start_analysis();
        self.autosave(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
        true
    }

}
