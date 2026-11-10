//! Keyboard, mouse, drag and reorder handling for the native editor.

use super::*;

impl App {
    pub(super) unsafe fn on_key(&mut self, hwnd: HWND, wparam: WPARAM) {
        let key = wparam.0 as u16;
        let control = GetKeyState(VK_CONTROL.0 as i32) < 0;
        if control && key == VK_S.0 {
            self.save(hwnd);
            return;
        }
        if control && key == VK_N.0 {
            self.new_tab(hwnd);
            return;
        }
        if control && key == VK_O.0 {
            self.open_dialog(hwnd);
            return;
        }
        if control && key == VK_W.0 {
            self.close_tab(hwnd, self.active_tab);
            return;
        }
        if control && key == VK_C.0 {
            self.copy_selection();
            return;
        }
        if control && key == VK_X.0 {
            self.cut_selection(hwnd);
            return;
        }
        if control && key == VK_V.0 {
            self.paste_clipboard(hwnd);
            return;
        }
        if control && key == VK_A.0 {
            self.selection = (0..self.rows.len()).collect();
            if !self.rows.is_empty() {
                self.selected = Some(0);
                self.selection_anchor = Some(0);
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            return;
        }
        if self.rows.is_empty() {
            return;
        }
        if key == VK_DELETE.0 {
            self.remove_selected(hwnd);
            return;
        }
        if key == VK_F2.0 {
            if let Some(index) = self.selected {
                self.begin_edit(hwnd, index);
            }
            return;
        }
        if key == VK_SPACE.0 {
            let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
            if ctrl {
                // Ctrl+Space toggles the multi-selection, like Ctrl+click.
                if let Some(index) = self.selected {
                    if !self.selection.remove(&index) {
                        self.selection.insert(index);
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            } else {
                // Space toggles the enable state of the current or selected
                // filters, which is what users expect from the ON/OFF prefix.
                let mut targets: Vec<usize> = if self.selection.is_empty() {
                    self.selected.into_iter().collect()
                } else {
                    self.selection.iter().copied().collect()
                };
                targets.sort_unstable();
                let toggled = !targets.is_empty();
                for index in targets {
                    self.toggle_row(hwnd, index);
                }
                if toggled {
                    self.start_analysis();
                }
            }
            return;
        }
        let viewport = self.viewport_height(hwnd);
        let page_rows = (viewport / scale(24, self.dpi)).max(1) as usize;
        let current = self.selected.unwrap_or(0);
        let last = self.rows.len() - 1;
        let target = if key == VK_UP.0 {
            current.saturating_sub(1)
        } else if key == VK_DOWN.0 {
            (current + 1).min(last)
        } else if key == VK_PRIOR.0 {
            current.saturating_sub(page_rows)
        } else if key == VK_NEXT.0 {
            (current + page_rows).min(last)
        } else if key == VK_HOME.0 {
            0
        } else if key == VK_END.0 {
            last
        } else {
            return;
        };
        self.commit_edit(hwnd, true);
        let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
        let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
        if shift {
            self.select_row(target, (false, true));
        } else if ctrl {
            self.selected = Some(target);
        } else {
            self.select_row(target, (false, false));
        }
        self.ensure_visible(hwnd, target);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
    pub(super) fn tab_hit(&self, x: i32) -> Option<(usize, bool)> {
        self.tab_layout().into_iter().find_map(|(index, rect)| {
            if x >= rect.left && x < rect.right {
                Some((index, x >= rect.right - scale(22, self.dpi)))
            } else {
                None
            }
        })
    }

    pub(super) unsafe fn on_click(&mut self, hwnd: HWND, lparam: LPARAM) {
        let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
        let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        let chrome = self.chrome_height();
        let table_bottom = self.table_bottom(hwnd);
        if y >= table_bottom {
            let hit = self
                .analysis_hits
                .iter()
                .find(|(_, rect)| point_in(rect, x, y))
                .copied();
            if let Some((kind, rect)) = hit {
                let guard = match kind {
                    AnalysisSetting::StartFrom => 3,
                    AnalysisSetting::Channel => 4,
                    AnalysisSetting::Resolution => 5,
                };
                if self.menu_toggle_allowed(guard) {
                    self.open_analysis_menu(hwnd, rect.left, rect.bottom, kind);
                }
            }
            return;
        }
        if self.show_toolbar && y < scale(30, self.dpi) {
            let new_left = scale(8, self.dpi);
            let new_right = scale(62, self.dpi);
            let open_right = scale(120, self.dpi);
            let save_right = scale(178, self.dpi);
            if x >= new_left && x < new_right {
                self.new_tab(hwnd);
            } else if x >= scale(66, self.dpi) && x < open_right {
                self.open_dialog(hwnd);
            } else if x >= scale(124, self.dpi) && x < save_right {
                self.save(hwnd);
            } else {
                let layout = self.toolbar_layout();
                if point_in(&layout[0], x, y) {
                    self.instant_mode = !self.instant_mode;
                    let _ = InvalidateRect(Some(hwnd), None, false);
                } else if point_in(&layout[1], x, y) {
                    if self.menu_toggle_allowed(1) {
                        self.open_device_menu(hwnd, layout[1].left, scale(30, self.dpi));
                    }
                } else if point_in(&layout[2], x, y) && self.menu_toggle_allowed(2) {
                    self.open_channel_menu(hwnd, layout[2].left, scale(30, self.dpi));
                }
            }
            return;
        }
        if y >= self.tab_top() && y < chrome {
            if let Some((index, close)) = self.tab_hit(x) {
                if close {
                    self.close_tab(hwnd, index);
                } else {
                    self.switch_tab(hwnd, index);
                }
            }
            return;
        }
        if y < chrome || y >= table_bottom {
            return;
        }
        let content = y - chrome + self.offset;
        if content < 0 || content as u32 >= self.total || self.rows.is_empty() {
            return;
        }
        let row = self
            .prefix
            .partition_point(|&start| start <= content as u32)
            .saturating_sub(1)
            .min(self.rows.len() - 1);
        self.commit_edit(hwnd, true);
        let ctrl = GetKeyState(VK_CONTROL.0 as i32) < 0;
        let shift = GetKeyState(VK_SHIFT.0 as i32) < 0;
        let power_left = scale(18, self.dpi);
        let power_right = scale(36, self.dpi);
        if x < power_left {
            self.select_row(row, (false, false));
            self.reorder = Some(ReorderState {
                start_y: y,
                active: false,
            });
            self.reorder_drop = Some(row);
            let _ = SetCapture(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
            return;
        }
        if x >= power_left && x <= power_right {
            self.select_row(row, (false, false));
            self.toggle_row(hwnd, row);
            return;
        }
        self.select_row(row, (ctrl, shift));
        self.ensure_item(row);
        let row_y = self.chrome_height() + self.prefix[row] as i32 - self.offset;
        if self.rows[row].enabled {
            self.handle_editor_click(hwnd, row, x, y, row_y);
        }
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    pub(super) fn select_row(&mut self, row: usize, modifiers: (bool, bool)) {
        let (ctrl, shift) = modifiers;
        if shift {
            let anchor = self.selection_anchor.unwrap_or(row);
            let start = anchor.min(row);
            let end = anchor.max(row);
            self.selection = (start..=end).collect();
            self.selected = Some(row);
        } else if ctrl {
            if !self.selection.remove(&row) {
                self.selection.insert(row);
            }
            self.selected = Some(row);
            self.selection_anchor = Some(row);
        } else {
            self.selection.clear();
            self.selection.insert(row);
            self.selected = Some(row);
            self.selection_anchor = Some(row);
        }
    }

    pub(super) unsafe fn handle_editor_click(
        &mut self,
        hwnd: HWND,
        row: usize,
        x: i32,
        mouse_y: i32,
        row_y: i32,
    ) {
        let dpi = self.dpi;
        let row_height_px = self.rows[row].height as i32;
        let geq_table_top = self.rows[row].geq_table_top;
        let mut sync = false;
        let mut analyze = false;
        let mut drag = None;
        let mut calibrate = None;
        let mut vst_options = None;
        let mut edit_value: Option<ValueField> = None;
        let mut pan_geq = false;
        let mut scroll_drag: Option<usize> = None;
        let mut vst_path_changed = false;
        let mut biquad_menu: Option<(BiquadMenu, RECT)> = None;
        if let Some(item) = self.rows[row].item.as_mut() {
            match &mut item.editor {
                Some(FilterEditor::Preamp { db }) => {
                    let track = preamp_slider_rect(dpi, row_y);
                    let right = RECT { right: scale(470, dpi), ..track };
                    if point_in(&preamp_value_rect(dpi, row_y), x, mouse_y) {
                        edit_value = Some(ValueField::PreampDb);
                    } else if point_in(&right, x, mouse_y) {
                        let t = ((x - right.left) as f64 / (right.right - right.left).max(1) as f64)
                            .clamp(0.0, 1.0);
                        *db = ((-20.0 + t * 40.0) * 10.0).round() / 10.0;
                        drag = Some(DragState {
                            row,
                            field: DragField::PreampDb,
                            start_x: x,
                            start_value: *db,
                            node: 0,
                        });
                        sync = true;
                        analyze = true;
                    }
                }
                Some(FilterEditor::Biquad(biquad)) => {
                    let rects = biquad_field_rects(dpi, row_y);
                    if point_in(&rects[0], x, mouse_y) {
                        biquad_menu = Some((BiquadMenu::Kind, rects[0]));
                    } else if point_in(&rects[4], x, mouse_y) {
                        biquad_menu = Some((BiquadMenu::QMode, rects[4]));
                    } else if point_in(&rects[1], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::BiquadFreq,
                            start_x: x,
                            start_value: biquad.freq,
                            node: 0,
                        });
                    } else if point_in(&rects[2], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::BiquadGain,
                            start_x: x,
                            start_value: biquad.gain,
                            node: 0,
                        });
                    } else if point_in(&rects[3], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::BiquadQ,
                            start_x: x,
                            start_value: biquad.q,
                            node: 0,
                        });
                    }
                }
                Some(FilterEditor::Delay { delay, is_ms }) => {
                    let rects = delay_field_rects(dpi, row_y);
                    if point_in(&rects[0], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::DelayValue,
                            start_x: x,
                            start_value: *delay,
                            node: 0,
                        });
                    } else if point_in(&rects[1], x, mouse_y) {
                        *is_ms = !*is_ms;
                        sync = true;
                        analyze = true;
                    }
                }
                Some(FilterEditor::Stage {
                    pre_mix,
                    post_mix,
                    capture,
                }) => {
                    let rects = stage_field_rects(dpi, row_y);
                    if point_in(&rects[0], x, mouse_y) {
                        *pre_mix = !*pre_mix;
                        sync = true;
                        analyze = true;
                    } else if point_in(&rects[1], x, mouse_y) {
                        *post_mix = !*post_mix;
                        sync = true;
                        analyze = true;
                    } else if point_in(&rects[2], x, mouse_y) {
                        *capture = !*capture;
                        sync = true;
                        analyze = true;
                    }
                }
                Some(FilterEditor::Include { path }) => {
                    let rects = path_row_rects(dpi, row_y);
                    if point_in(&rects[1], x, mouse_y) {
                        if let Some(chosen) = FileDialog::new()
                            .set_title(&i18n::tr("Include configuration file"))
                            .add_filter("Configuration files", &["txt"])
                            .pick_file()
                        {
                            *path = chosen.to_string_lossy().into_owned();
                            sync = true;
                            analyze = true;
                        }
                    }
                }
                Some(FilterEditor::Convolution { path, .. }) => {
                    let rects = path_row_rects(dpi, row_y);
                    if point_in(&rects[1], x, mouse_y) {
                        if let Some(chosen) = FileDialog::new()
                            .set_title(&i18n::tr("Select impulse response"))
                            .add_filter("Impulse responses", &["wav", "flac", "ogg"])
                            .pick_file()
                        {
                            *path = chosen.to_string_lossy().into_owned();
                            sync = true;
                            analyze = true;
                        }
                    }
                }
                Some(FilterEditor::Vst(vst)) => {
                    let rects = path_row_rects(dpi, row_y);
                    let actions = vst_action_rects(dpi, row_y);
                    if point_in(&rects[0], x, mouse_y) {
                        let current = vst.path.clone();
                        if let Some(text) = prompt_text(
                            hwnd,
                            &i18n::tr("VST plugin path"),
                            &i18n::tr(
                                "Enter the VST plugin path (file name or absolute path):",
                            ),
                            &current,
                        ) {
                            vst.path = text.trim().to_string();
                            vst_editor::refresh_plugin_status(vst);
                            vst_path_changed = true;
                            sync = true;
                            analyze = true;
                        }
                    } else if point_in(&rects[1], x, mouse_y) {
                        if let Some(chosen) = FileDialog::new()
                            .set_title(&i18n::tr("Select VST plugin"))
                            .add_filter("VST plugins", &["dll"])
                            .pick_file()
                        {
                            vst.path =
                                eapo_editor_core::model::vst_config_path(&chosen.to_string_lossy());
                            vst_editor::refresh_plugin_status(vst);
                            vst_path_changed = true;
                            sync = true;
                            analyze = true;
                        }
                    } else if point_in(&actions[0], x, mouse_y) {
                        if vst_editor::open_plugin_editor(vst) {
                            sync = true;
                            analyze = true;
                        }
                    } else if point_in(&actions[1], x, mouse_y) {
                        vst_options = Some((actions[1].left, actions[1].bottom));
                    }
                }
                Some(FilterEditor::Loudness {
                    state,
                    ref_level,
                    ref_offset,
                    att,
                    volume,
                    ..
                }) => {
                    let rects = loudness_field_rects(dpi, row_y);
                    if point_in(&rects[0], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::LoudnessRef,
                            start_x: x,
                            start_value: *ref_level as f64,
                            node: 0,
                        });
                    } else if point_in(&rects[1], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::LoudnessOffset,
                            start_x: x,
                            start_value: *ref_offset as f64,
                            node: 0,
                        });
                    } else if point_in(&rects[2], x, mouse_y) {
                        drag = Some(DragState {
                            row,
                            field: DragField::LoudnessAtt,
                            start_x: x,
                            start_value: *att,
                            node: 0,
                        });
                    } else if point_in(&rects[3], x, mouse_y) {
                        *state = false;
                        calibrate = Some(*volume);
                        sync = true;
                        analyze = true;
                    }
                }
                Some(FilterEditor::Copy {
                    assignments,
                    channel_names,
                    ..
                }) => {
                    let button = change_button_rect(dpi, row_y);
                    if point_in(&button, x, mouse_y) {
                        if let Some(result) =
                            copy_dialog::edit_copy(hwnd, assignments, channel_names)
                        {
                            *assignments = result;
                            for assignment in assignments.iter() {
                                if !assignment.target.is_empty()
                                    && !channel_names
                                        .iter()
                                        .any(|name| name.eq_ignore_ascii_case(&assignment.target))
                                {
                                    channel_names.push(assignment.target.clone());
                                }
                                for summand in &assignment.summands {
                                    let name = summand.channel.trim();
                                    if !name.is_empty()
                                        && summand.channel != " "
                                        && !channel_names
                                            .iter()
                                            .any(|existing| existing.eq_ignore_ascii_case(name))
                                    {
                                        channel_names.push(name.to_string());
                                    }
                                }
                            }
                            sync = true;
                            analyze = true;
                        }
                    }
                }
                Some(FilterEditor::Channel {
                    selected,
                    channel_names,
                    channel_mask,
                    ..
                }) => {
                    let button = change_button_rect(dpi, row_y);
                    if point_in(&button, x, mouse_y) {
                        let mut names: Vec<String> = Vec::new();
                        let mut labels: Vec<String> = Vec::new();
                        for (name, label) in speaker_positions(*channel_mask) {
                            names.push(name.to_string());
                            labels.push(format!("{name} ({label})"));
                        }
                        for name in channel_names.iter() {
                            if !names.iter().any(|n| n.eq_ignore_ascii_case(name)) {
                                names.push(name.clone());
                                labels.push(name.clone());
                            }
                        }
                        for name in selected.iter() {
                            if name != "ALL"
                                && !names.iter().any(|n| n.eq_ignore_ascii_case(name))
                            {
                                names.push(name.clone());
                                labels.push(name.clone());
                            }
                        }
                        let checked: Vec<usize> = names
                            .iter()
                            .enumerate()
                            .filter(|(_, name)| {
                                selected.iter().any(|s| s.eq_ignore_ascii_case(name))
                            })
                            .map(|(index, _)| index)
                            .collect();
                        let all = selected.iter().any(|s| s == "ALL");
                        let options = chooser::CheckListOptions {
                            all_label: Some(i18n::tr("Select all channels")),
                            all_checked: all,
                            filter: None,
                        };
                        if let Some(result) = chooser::check_list(
                            hwnd,
                            &i18n::tr("Channel selection"),
                            &labels,
                            &checked,
                            options,
                        ) {
                            *selected = if result.all {
                                vec!["ALL".to_string()]
                            } else {
                                result
                                    .checked
                                    .iter()
                                    .filter_map(|index| names.get(*index))
                                    .map(|name| name.to_uppercase())
                                    .collect()
                            };
                            sync = true;
                            analyze = true;
                        }
                    }
                }
                Some(FilterEditor::Device { pattern, .. }) => {
                    let button = change_button_rect(dpi, row_y);
                    if point_in(&button, x, mouse_y) {
                        let all = pattern.trim().eq_ignore_ascii_case("all");
                        let mut labels = Vec::with_capacity(self.devices.len());
                        let mut keys = Vec::with_capacity(self.devices.len());
                        let mut keep = Vec::with_capacity(self.devices.len());
                        for device in &self.devices {
                            let kind = if device.is_input() {
                                "Capture"
                            } else {
                                "Playback"
                            };
                            let apo = if device.is_installed() {
                                "APO installed"
                            } else {
                                "APO not installed"
                            };
                            labels.push(format!(
                                "{kind} | {} | {} | {apo}",
                                device.get_connection_name(),
                                device.get_device_name()
                            ));
                            keys.push(device.get_device_string());
                            keep.push(device.is_installed());
                        }
                        let checked: Vec<usize> = if all {
                            Vec::new()
                        } else {
                            keys.iter()
                                .enumerate()
                                .filter(|(_, key)| device_matches(key, pattern))
                                .map(|(index, _)| index)
                                .collect()
                        };
                        let options = chooser::CheckListOptions {
                            all_label: Some(i18n::tr("Select all devices")),
                            all_checked: all,
                            filter: Some((
                                i18n::tr("Show only devices with APO installed"),
                                true,
                                keep,
                            )),
                        };
                        if let Some(result) = chooser::check_list(
                            hwnd,
                            &i18n::tr("Device selection"),
                            &labels,
                            &checked,
                            options,
                        ) {
                            *pattern = if result.all {
                                "all".to_string()
                            } else {
                                let selected: Vec<String> = result
                                    .checked
                                    .iter()
                                    .filter_map(|index| keys.get(*index))
                                    .cloned()
                                    .collect();
                                selected.join("; ")
                            };
                            sync = true;
                            analyze = true;
                        }
                    }
                }
                Some(FilterEditor::GraphicEq(graphic)) => {
                    let (buttons, _) = geq_button_rects(dpi, row_y, graphic.band_count);
                    let mut client = RECT::default();
                    let _ = GetClientRect(hwnd, &mut client);
                    let height = row_height_px;
                    let plot = geq_plot_rect(dpi, row_y, height, client.right);
                    let table = geq_table_rect(dpi, row_y, height, client.right);
                    let visible = geq_table_visible_rows(dpi, &table);
                    let table_top =
                        geq_table_top_clamped(geq_table_top, graphic.nodes.len(), visible);
                    if point_in(&buttons[0], x, mouse_y) {
                        graphic.set_band_count(15);
                        sync = true;
                        analyze = true;
                    } else if point_in(&buttons[1], x, mouse_y) {
                        graphic.set_band_count(31);
                        sync = true;
                        analyze = true;
                    } else if point_in(&buttons[2], x, mouse_y) {
                        graphic.set_band_count(-1);
                        sync = true;
                        analyze = true;
                    } else if point_in(&buttons[3], x, mouse_y) {
                        for node in &mut graphic.nodes {
                            node.db_gain = -node.db_gain;
                        }
                        sync = true;
                        analyze = true;
                    } else if point_in(&buttons[4], x, mouse_y) {
                        let max = graphic
                            .nodes
                            .iter()
                            .map(|node| node.db_gain)
                            .fold(f64::NEG_INFINITY, f64::max);
                        if max.is_finite() {
                            for node in &mut graphic.nodes {
                                node.db_gain -= max;
                            }
                        }
                        sync = true;
                        analyze = true;
                    } else if point_in(&buttons[5], x, mouse_y) {
                        for node in &mut graphic.nodes {
                            node.db_gain = 0.0;
                        }
                        sync = true;
                        analyze = true;
                    } else if graphic.nodes.len() > visible
                        && point_in(&geq_table_scrollbar_rect(dpi, &table), x, mouse_y)
                    {
                        scroll_drag = Some(geq_table_top_at_y(
                            dpi,
                            &table,
                            visible,
                            graphic.nodes.len(),
                            mouse_y,
                        ));
                    } else if let Some((index, column)) = geq_table_cell_at(
                        dpi,
                        &table,
                        table_top,
                        visible,
                        graphic.nodes.len(),
                        x,
                        mouse_y,
                    ) {
                        graphic.selected = vec![index];
                        edit_value = Some(if column == 0 {
                            ValueField::GeqFreq(index)
                        } else {
                            ValueField::GeqGain(index)
                        });
                    } else if point_in(&plot, x, mouse_y) {
                        let view = GeqView::new(graphic);
                        let margin = scale(10, dpi);
                        let mut best: Option<(usize, i32)> = None;
                        for (index, node) in graphic.nodes.iter().enumerate() {
                            let nx = view.x_of(node.freq, &plot).round() as i32;
                            let ny = view
                                .y_of(node.db_gain.clamp(GEQ_WORLD_MIN_DB, GEQ_WORLD_MAX_DB), &plot)
                                .round() as i32;
                            if nx < plot.left - margin
                                || nx > plot.right + margin
                                || ny < plot.top - margin
                                || ny > plot.bottom + margin
                            {
                                continue;
                            }
                            let distance = (nx - x).abs() + (ny - mouse_y).abs();
                            if distance <= scale(10, dpi)
                                && best.map(|(_, best_distance)| distance < best_distance).unwrap_or(true)
                            {
                                best = Some((index, distance));
                            }
                        }
                        if let Some((index, _)) = best {
                            graphic.selected = vec![index];
                            let value = graphic.nodes[index].db_gain;
                            drag = Some(DragState {
                                row,
                                field: DragField::GeqNode,
                                start_x: x,
                                start_value: value,
                                node: index,
                            });
                        } else {
                            graphic.selected.clear();
                            pan_geq = true;
                        }
                    }
                }
                _ => {}
            }
        }
        if vst_path_changed {
            self.vst_readable_cache.clear();
        }
        if let Some(top) = scroll_drag {
            self.rows[row].geq_table_top = top;
            self.geq_table_scroll_drag = Some(row);
            let _ = SetCapture(hwnd);
            self.invalidate_row(hwnd, row);
            return;
        }
        if pan_geq {
            self.geq_pan = Some((row, x, mouse_y));
            let _ = SetCapture(hwnd);
        }
        if let Some(field) = edit_value {
            self.begin_value_edit(hwnd, row, field);
            return;
        }
        if let Some((menu_kind, rect)) = biquad_menu {
            let guard = if matches!(menu_kind, BiquadMenu::Kind) { 7 } else { 8 };
            if self.menu_toggle_allowed(guard) {
                self.biquad_type_menu(hwnd, row, rect.left, rect.bottom, menu_kind);
            }
        }
        if let Some((menu_x, menu_y)) = vst_options {
            if !self.menu_toggle_allowed(6) {
                return;
            }
            if let Some(item) = self.rows[row].item.as_mut() {
                if let Some(FilterEditor::Vst(vst)) = item.editor.as_mut() {
                    let menu = match CreatePopupMenu() {
                        Ok(menu) => menu,
                        Err(_) => return,
                    };
                    let mut menu_texts: Vec<Vec<u16>> = Vec::new();
                    let _ = AppendMenuW(
                        menu,
                        if vst.embed {
                            MF_STRING | MF_CHECKED
                        } else {
                            MF_STRING
                        },
                        MENU_VST_EMBED,
                        menu_text(&mut menu_texts, "Embed"),
                    );
                    let _ = AppendMenuW(
                        menu,
                        if vst.auto_apply {
                            MF_STRING | MF_CHECKED
                        } else {
                            MF_STRING
                        },
                        MENU_VST_AUTO_APPLY,
                        menu_text(&mut menu_texts, "Auto apply"),
                    );
                    let mut point = POINT { x: menu_x, y: menu_y };
                    let _ = ClientToScreen(hwnd, &mut point);
                    let command =
                        TrackPopupMenuEx(menu, TPM_RETURNCMD.0, point.x, point.y, hwnd, None).0
                            as usize;
                    let _ = DestroyMenu(menu);
                    if command == 0 {
                        self.menu_guard = Some((6, std::time::Instant::now()));
                    }
                    match command {
                        MENU_VST_EMBED => vst.embed = !vst.embed,
                        MENU_VST_AUTO_APPLY => vst.auto_apply = !vst.auto_apply,
                        _ => {}
                    }
                    if command == MENU_VST_EMBED || command == MENU_VST_AUTO_APPLY {
                        self.dirty = true;
                        self.save_hotkeys();
                        self.autosave(hwnd);
                    }
                    let _ = InvalidateRect(Some(hwnd), None, false);
                }
            }
            return;
        }
        if let Some(volume) = calibrate {
            self.sync_item(hwnd, row);
            self.autosave(hwnd);
            self.start_analysis();
            calibration::stop();
            let measured_level = calibration_dialog::calibrate(hwnd, volume);
            if let Some(item) = self.rows[row].item.as_mut() {
                if let Some(FilterEditor::Loudness {
                    state,
                    ref_level,
                    measured,
                    ..
                }) = item.editor.as_mut()
                {
                    *state = true;
                    if let Some(value) = measured_level {
                        *ref_level = ((75 - value) as f64 + volume) as i32;
                        *measured = value;
                    }
                }
            }
            sync = true;
            analyze = true;
        }
        if let Some(drag) = drag {
            self.drag = Some(drag);
            self.drag_moved = false;
            let _ = SetCapture(hwnd);
        }
        if sync {
            self.sync_item(hwnd, row);
            if drag.is_none() {
                self.save_hotkeys();
                self.autosave(hwnd);
            }
        }
        if analyze {
            self.start_analysis();
        }
        if matches!(
            self.rows[row].item.as_ref().and_then(|item| item.editor.as_ref()),
            Some(FilterEditor::GraphicEq(_))
        ) {
            self.ensure_geq_table_visible(hwnd, row);
        }
    }

    pub(super) unsafe fn handle_editor_double_click(
        &mut self,
        hwnd: HWND,
        row: usize,
        x: i32,
        mouse_y: i32,
        row_y: i32,
    ) -> bool {
        self.ensure_item(row);
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let height = self.rows[row].height as i32;
        let plot = geq_plot_rect(self.dpi, row_y, height, client.right);
        if !point_in(&plot, x, mouse_y) {
            return false;
        }
        let mut added = false;
        if let Some(item) = self.rows[row].item.as_mut() {
            if let Some(FilterEditor::GraphicEq(graphic)) = &mut item.editor {
                if graphic.band_count < 0 {
                    let view = GeqView::new(graphic);
                    let frequency = view.frequency_at(x as f32, &plot);
                    let db_gain = (view.db_at(mouse_y as f32, &plot) * 10.0).round() / 10.0;
                    graphic.nodes.push(FilterNode {
                        freq: frequency,
                        db_gain,
                    });
                    graphic.sort_nodes();
                    graphic.selected = graphic
                        .nodes
                        .iter()
                        .position(|node| {
                            (node.freq - frequency).abs() < 1e-9
                                && (node.db_gain - db_gain).abs() < 1e-9
                        })
                        .map(|index| vec![index])
                        .unwrap_or_default();
                    added = true;
                }
            }
        }
        if added {
            self.sync_item(hwnd, row);
            self.save_hotkeys();
            self.autosave(hwnd);
            self.start_analysis();
            self.ensure_geq_table_visible(hwnd, row);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
        added
    }
    pub(super) unsafe fn on_mouse_move(&mut self, hwnd: HWND, lparam: LPARAM) {
        let move_x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
        let move_y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        self.hover_pos = Some((move_x, move_y));
        let hot = self.hot_control_at(hwnd, move_x, move_y);
        if hot != self.hot_rect {
            if let Some(old) = self.hot_rect {
                let _ = InvalidateRect(Some(hwnd), Some(&old), false);
            }
            if let Some(new) = hot {
                let _ = InvalidateRect(Some(hwnd), Some(&new), false);
            }
            self.hot_rect = hot;
        }
        if let Some(last) = self.plot_pan {
            if self.show_analysis {
                let view = (
                    (self.plot_rect.right - self.plot_rect.left) as f32,
                    (self.plot_rect.bottom - self.plot_rect.top) as f32,
                );
                self.plot
                    .pan((move_x - last.0) as f32, (move_y - last.1) as f32, view.0, view.1);
                self.plot_pan = Some((move_x, move_y));
                self.invalidate_plot(hwnd);
            } else {
                self.plot_pan = None;
            }
            return;
        }
        if let Some(row) = self.geq_table_scroll_drag {
            if row >= self.rows.len() {
                self.geq_table_scroll_drag = None;
                let _ = ReleaseCapture();
                return;
            }
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let row_y = self.chrome_height() + self.prefix[row] as i32 - self.offset;
            let table = geq_table_rect(self.dpi, row_y, self.rows[row].height as i32, client.right);
            let visible = geq_table_visible_rows(self.dpi, &table);
            let nodes = match self.rows[row]
                .item
                .as_ref()
                .and_then(|item| item.editor.as_ref())
            {
                Some(FilterEditor::GraphicEq(graphic)) => graphic.nodes.len(),
                _ => 0,
            };
            let top = geq_table_top_at_y(self.dpi, &table, visible, nodes, move_y);
            if top != self.rows[row].geq_table_top {
                self.rows[row].geq_table_top = top;
                self.invalidate_row(hwnd, row);
            }
            return;
        }
        if let Some((row, last_x, last_y)) = self.geq_pan {
            if row >= self.rows.len() {
                self.geq_pan = None;
                let _ = ReleaseCapture();
                return;
            }
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let row_y = self.chrome_height() + self.prefix[row] as i32 - self.offset;
            let height = self.rows[row].height as i32;
            let plot = geq_plot_rect(self.dpi, row_y, height, client.right);
            let mut moved = false;
            if let Some(item) = self.rows[row].item.as_mut() {
                if let Some(FilterEditor::GraphicEq(graphic)) = &mut item.editor {
                    let mut view = GeqView::new(graphic);
                    view.pan(
                        (move_x - last_x) as f32,
                        (move_y - last_y) as f32,
                        &plot,
                    );
                    view.store(graphic);
                    moved = true;
                }
            }
            self.geq_pan = Some((row, move_x, move_y));
            if moved {
                self.invalidate_row(hwnd, row);
            }
            return;
        }
        let over_plot = self.show_analysis && point_in(&self.plot_rect, move_x, move_y);
        let hover_changed = match (self.plot_hover, over_plot) {
            (None, false) => false,
            (Some(_), true) | (None, true) | (Some(_), false) => true,
        };
        if hover_changed {
            self.plot_hover = if over_plot {
                Some((move_x, move_y))
            } else {
                None
            };
            self.invalidate_plot(hwnd);
        } else if over_plot {
            self.plot_hover = Some((move_x, move_y));
            self.invalidate_plot(hwnd);
        }
        if over_plot {
            return;
        }
        let reorder_chrome = self.chrome_height();
        let reorder_dpi = self.dpi;
        if let Some(reorder) = self.reorder.as_mut() {
            let y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
            let was_active = reorder.active;
            if (y - reorder.start_y).abs() > scale(4, reorder_dpi) {
                reorder.active = true;
            }
            let active_now = reorder.active;
            let content = (y - reorder_chrome + self.offset).max(0) as u32;
            let drop = self
                .prefix
                .partition_point(|&start| start <= content)
                .saturating_sub(1)
                .min(self.rows.len());
            let drop_changed = self.reorder_drop != Some(drop);
            self.reorder_drop = Some(drop);
            if drop_changed || (!was_active && active_now) {
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
            return;
        }
        let Some(drag) = self.drag else {
            return;
        };
        if drag.row >= self.rows.len() {
            self.drag = None;
            let _ = ReleaseCapture();
            return;
        }
        let x = (lparam.0 & 0xFFFF) as u16 as i16 as i32;
        let mouse_y = ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        // Treat a press without horizontal movement as a click (which opens
        // the inline number editor) rather than a value drag.
        if drag_value_field(drag.field).is_some()
            && !self.drag_moved
            && (x - drag.start_x).abs() < scale(3, self.dpi)
        {
            return;
        }
        self.drag_moved = true;
        let delta = (x - drag.start_x) as f64;
        let geq_plot = if drag.field == DragField::GeqNode {
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let row_y = self.chrome_height()
                + self.prefix[drag.row] as i32
                - self.offset;
            let height = self.rows[drag.row].height as i32;
            Some(geq_plot_rect(self.dpi, row_y, height, client.right))
        } else {
            None
        };
        let mut changed = false;
        if let Some(item) = self.rows[drag.row].item.as_mut() {
            match (&mut item.editor, drag.field) {
                (Some(FilterEditor::Preamp { db }), DragField::PreampDb) => {
                    *db = (((drag.start_value + delta * 0.1) * 10.0).round() / 10.0)
                        .clamp(-20.0, 20.0);
                    changed = true;
                }
                (Some(FilterEditor::Biquad(biquad)), DragField::BiquadFreq) => {
                    biquad.freq = (drag.start_value * 2.0f64.powf(delta / 50.0))
                        .clamp(1.0, 30000.0);
                    changed = true;
                }
                (Some(FilterEditor::Biquad(biquad)), DragField::BiquadGain) => {
                    biquad.gain = (drag.start_value + delta * 0.1).clamp(-30.0, 30.0);
                    changed = true;
                }
                (Some(FilterEditor::Biquad(biquad)), DragField::BiquadQ) => {
                    biquad.q = (drag.start_value * 2.0f64.powf(delta / 60.0))
                        .clamp(0.1, 100.0);
                    changed = true;
                }
                (Some(FilterEditor::Delay { delay, .. }), DragField::DelayValue) => {
                    *delay = (drag.start_value + delta * 0.1).clamp(0.0, 10000.0);
                    changed = true;
                }
                (Some(FilterEditor::Loudness { ref_level, .. }), DragField::LoudnessRef) => {
                    *ref_level = ((drag.start_value + delta / 8.0).round() as i32)
                        .clamp(-999, 999);
                    changed = true;
                }
                (Some(FilterEditor::Loudness { ref_offset, .. }), DragField::LoudnessOffset) => {
                    *ref_offset = ((drag.start_value + delta / 8.0).round() as i32)
                        .clamp(-999, 999);
                    changed = true;
                }
                (Some(FilterEditor::Loudness { att, .. }), DragField::LoudnessAtt) => {
                    *att = ((drag.start_value + delta * 0.01).clamp(0.0, 1.0) * 100.0)
                        .round()
                        / 100.0;
                    changed = true;
                }
                (Some(FilterEditor::GraphicEq(graphic)), DragField::GeqNode) => {
                    if let Some(plot) = geq_plot {
                        if drag.node < graphic.nodes.len() {
                            let view = GeqView::new(graphic);
                            let db = view.db_at(mouse_y as f32, &plot);
                            graphic.nodes[drag.node].db_gain = (db * 10.0).round() / 10.0;
                            changed = true;
                        }
                    }
                }
                _ => {}
            }
        }
        if changed {
            self.sync_item(hwnd, drag.row);
            // Only the dragged row changed; repainting the whole window on
            // every mouse move caused the visible flicker.
            let mut client = RECT::default();
            let _ = GetClientRect(hwnd, &mut client);
            let row_top =
                self.chrome_height() + self.prefix[drag.row] as i32 - self.offset;
            let row_rect = RECT {
                left: 0,
                top: row_top,
                right: client.right,
                bottom: row_top + self.rows[drag.row].height as i32,
            };
            let _ = InvalidateRect(Some(hwnd), Some(&row_rect), false);
        }
    }

    pub(super) unsafe fn on_mouse_up(&mut self, hwnd: HWND) {
        if self.geq_table_scroll_drag.take().is_some() {
            let _ = ReleaseCapture();
            return;
        }
        if self.plot_pan.take().is_some() {
            let _ = ReleaseCapture();
            self.invalidate_plot(hwnd);
            return;
        }
        if let Some((row, _, _)) = self.geq_pan.take() {
            let _ = ReleaseCapture();
            self.invalidate_row(hwnd, row);
            return;
        }
        if self.reorder.is_some() {
            self.finish_reorder(hwnd);
            return;
        }
        if let Some(drag) = self.drag.take() {
            let _ = ReleaseCapture();
            let open_editor = if self.drag_moved {
                None
            } else {
                drag_value_field(drag.field)
            };
            let moved = self.drag_moved;
            self.drag_moved = false;
            if self.instant_mode && self.doc_path.is_some() {
                self.save(hwnd);
            } else {
                self.start_analysis();
            }
            if moved {
                self.save_hotkeys();
            }
            let _ = InvalidateRect(Some(hwnd), None, false);
            if let Some(field) = open_editor {
                self.begin_value_edit(hwnd, drag.row, field);
            }
        }
    }

    pub(super) unsafe fn finish_reorder(&mut self, hwnd: HWND) {
        let Some(reorder) = self.reorder.take() else {
            return;
        };
        let drop = self.reorder_drop.take();
        let _ = ReleaseCapture();
        if !reorder.active {
            let _ = InvalidateRect(Some(hwnd), None, false);
            return;
        }
        let Some(drop) = drop else {
            return;
        };
        let mut indices: Vec<usize> = self.selection.iter().copied().collect();
        if indices.is_empty() {
            if let Some(index) = self.selected {
                indices.push(index);
            }
        }
        indices.sort_unstable();
        indices.dedup();
        if indices.is_empty() {
            return;
        }
        let selected: HashSet<usize> = indices.into_iter().collect();
        let mut moving = Vec::new();
        let mut kept = Vec::with_capacity(self.rows.len());
        let mut removed_before = 0;
        for (index, row) in self.rows.drain(..).enumerate() {
            if selected.contains(&index) {
                if index < drop {
                    removed_before += 1;
                }
                moving.push(row);
            } else {
                kept.push(row);
            }
        }
        let count = moving.len();
        if count == 0 {
            self.rows = kept;
            return;
        }
        let insert = drop.saturating_sub(removed_before).min(kept.len());
        kept.splice(insert..insert, moving);
        self.rows = kept;
        self.selection = (insert..insert + count).collect();
        self.selected = self.selection.iter().next().copied();
        self.selection_anchor = self.selected;
        self.dirty = true;
        self.recompute_prefix();
        self.update_title(hwnd);
        self.save_hotkeys();
        self.start_analysis();
        self.autosave(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }
}
