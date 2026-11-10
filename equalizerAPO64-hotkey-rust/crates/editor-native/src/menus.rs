//! Context, device, channel and filter popup menus.

use super::*;

impl App {
    pub(super) unsafe fn on_context_menu(&mut self, hwnd: HWND, lparam: LPARAM) {
        let x = ((lparam.0 & 0xFFFF) as u16 as i16) as i32;
        let y = (((lparam.0 >> 16) & 0xFFFF) as u16 as i16) as i32;
        let chrome = self.chrome_height();
        let table_bottom = self.table_bottom(hwnd);
        if y >= table_bottom {
            return;
        }
        if y >= chrome && self.total > 0 {
            let content = y - chrome + self.offset;
            if content >= 0 && (content as u32) < self.total {
                let row = self
                    .prefix
                    .partition_point(|&start| start <= content as u32)
                    .saturating_sub(1)
                    .min(self.rows.len().saturating_sub(1));
                self.selected = Some(row);
                let _ = InvalidateRect(Some(hwnd), None, false);
            }
        }
        let menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => return,
        };
        let mut menu_texts: Vec<Vec<u16>> = Vec::new();
        let selected_row = self.selected.and_then(|index| self.rows.get(index));
        let kind = selected_row.map(|row| row.kind);
        let has_editor = kind.is_some_and(|kind| kind != FilterKind::Unknown);
        let has_hotkey = selected_row.and_then(|row| row.hotkey).is_some();
        let flag = |enabled: bool| {
            if enabled {
                MF_STRING
            } else {
                MF_STRING | MF_GRAYED
            }
        };
        let _ = AppendMenuW(menu, MF_STRING, MENU_TOGGLE, menu_text(&mut menu_texts, "Enable / disable"));
        let _ = AppendMenuW(menu, flag(has_editor), MENU_SET_HOTKEY, menu_text(&mut menu_texts, "Set hotkey..."));
        let _ = AppendMenuW(menu, flag(has_hotkey), MENU_CLEAR_HOTKEY, menu_text(&mut menu_texts, "Clear hotkey"));
        let _ = AppendMenuW(menu, MF_STRING, MENU_EDIT, menu_text(&mut menu_texts, "Edit text"));
        let _ = AppendMenuW(
            menu,
            flag(kind == Some(FilterKind::Vst)),
            MENU_OPEN_VST,
            menu_text(&mut menu_texts, "Open VST panel"),
        );
        let _ = AppendMenuW(menu, MF_STRING, MENU_REMOVE, menu_text(&mut menu_texts, "Remove"));
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, w!(""));
        let add_menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => {
                let _ = DestroyMenu(menu);
                return;
            }
        };
        let mut add_commands: Vec<(usize, String)> = Vec::new();
        for (group, entries) in ADD_GROUPS {
            let submenu = match CreatePopupMenu() {
                Ok(menu) => menu,
                Err(_) => continue,
            };
            for (label, line) in *entries {
                let id = 600 + add_commands.len();
                let _ = AppendMenuW(submenu, MF_STRING, id, menu_text(&mut menu_texts, label));
                add_commands.push((id, line.to_string()));
            }
            let _ = AppendMenuW(
                add_menu,
                MF_POPUP,
                submenu.0 as usize,
                menu_text(&mut menu_texts, group),
            );
        }
        let _ = AppendMenuW(menu, MF_POPUP, add_menu.0 as usize, menu_text(&mut menu_texts, "Add filter"));
        let point = Self::screen_point(hwnd, x, y);
        let command = TrackPopupMenuEx(menu, TPM_RETURNCMD.0, point.x, point.y, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        let before = self.selected;
        match command {
            MENU_TOGGLE => {
                if let Some(index) = self.selected {
                    self.toggle_row(hwnd, index);
                    self.start_analysis();
                }
            }
            MENU_EDIT => {
                if let Some(index) = self.selected {
                    self.begin_edit(hwnd, index);
                }
            }
            MENU_SET_HOTKEY => {
                if let Some(index) = self.selected {
                    self.set_hotkey_for_row(hwnd, index);
                }
            }
            MENU_CLEAR_HOTKEY => {
                if let Some(index) = self.selected {
                    self.clear_hotkey_for_row(hwnd, index);
                }
            }
            MENU_OPEN_VST => {
                if let Some(index) = self.selected {
                    self.ensure_item(index);
                    let mut opened = false;
                    if let Some(item) = self.rows[index].item.as_mut() {
                        if let Some(FilterEditor::Vst(vst)) = item.editor.as_mut() {
                            vst_editor::open_plugin_editor(vst);
                            opened = true;
                        }
                    }
                    if opened {
                        self.sync_item(hwnd, index);
                        self.save_hotkeys();
                        self.start_analysis();
                        self.autosave(hwnd);
                        let _ = InvalidateRect(Some(hwnd), None, false);
                    }
                }
            }
            MENU_REMOVE => self.remove_selected(hwnd),
            _ => {
                if let Some((_, line)) = add_commands.iter().find(|(id, _)| *id == command) {
                    let line = line.clone();
                    self.add_row(hwnd, &line, before);
                }
            }
        }
    }

    pub(super) unsafe fn open_device_menu(&mut self, hwnd: HWND, x: i32, y: i32) {
        let menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => return,
        };
        let mut commands = Vec::new();
        let mut menu_texts: Vec<Vec<u16>> = Vec::new();
        for (index, device) in self.devices.iter().enumerate() {
            let mut label = format!(
                "{}: {} - {}",
                if device.is_input() { "Capture" } else { "Playback" },
                device.get_connection_name(),
                device.get_device_name()
            );
            // Live configuration only has an audible effect where the APO is
            // actually installed, so say which devices are not.
            if !device.is_installed() {
                label.push_str(&format!(" ({})", i18n::tr("APO not installed")));
            }
            let id = 800 + index;
            let _ = AppendMenuW(menu, MF_STRING, id, menu_raw(&mut menu_texts, &label));
            commands.push((id, index));
        }
        let _ = AppendMenuW(menu, MF_SEPARATOR, 0, PCWSTR::null());
        let radio = |on: bool| MF_STRING | if on { MF_CHECKED } else { MF_UNCHECKED };
        let _ = AppendMenuW(
            menu,
            radio(self.live_mode == LiveMode::Off),
            130,
            menu_text(&mut menu_texts, "Live configuration: none (use config.txt)"),
        );
        let _ = AppendMenuW(
            menu,
            radio(self.live_mode == LiveMode::ActiveTab),
            131,
            menu_text(&mut menu_texts, "Live configuration: active tab"),
        );
        let _ = AppendMenuW(
            menu,
            radio(self.live_mode == LiveMode::AllTabs),
            132,
            menu_text(&mut menu_texts, "Live configuration: all open tabs"),
        );
        let point = Self::screen_point(hwnd, x, y);
        let command = TrackPopupMenuEx(menu, TPM_RETURNCMD.0, point.x, point.y, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        if command == 0 {
            self.menu_guard = Some((1, std::time::Instant::now()));
        }
        if (130..=132).contains(&command) {
            let mode = match command {
                130 => LiveMode::Off,
                131 => LiveMode::ActiveTab,
                _ => LiveMode::AllTabs,
            };
            self.set_live_mode(hwnd, mode);
            return;
        }
        let Some((_, index)) = commands.into_iter().find(|(id, _)| *id == command) else {
            return;
        };
        let Some(device) = self.devices.get(index).cloned() else {
            return;
        };
        self.selected_device = Some(device.clone());
        if self.channel_mask_from_device {
            self.channel_mask = device.get_channel_mask().max(1);
        }
        self.channel_names = channel_helper::get_channel_names(
            device.get_channel_count() as i32,
            self.channel_mask,
        );
        self.analysis_channel_index = self
            .analysis_channel_index
            .min(self.channel_names.len().saturating_sub(1));
        for row in &mut self.rows {
            row.item = None;
        }
        self.analysis_result = None;
        self.start_analysis();
        self.publish_live_files(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    pub(super) unsafe fn open_channel_menu(&mut self, hwnd: HWND, x: i32, y: i32) {
        let options = [
            ("From device", 0u32),
            ("Mono", channel_helper::KSAUDIO_SPEAKER_MONO),
            ("Stereo", channel_helper::KSAUDIO_SPEAKER_STEREO),
            ("Quadraphonic", channel_helper::KSAUDIO_SPEAKER_QUAD),
            (
                "5.1 Surround",
                channel_helper::KSAUDIO_SPEAKER_5POINT1_SURROUND,
            ),
            (
                "7.1 Surround",
                channel_helper::KSAUDIO_SPEAKER_7POINT1_SURROUND,
            ),
        ];
        let menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => return,
        };
        let mut menu_texts: Vec<Vec<u16>> = Vec::new();
        for (index, (label, _)) in options.iter().enumerate() {
            let _ = AppendMenuW(menu, MF_STRING, 830 + index, menu_text(&mut menu_texts, label));
        }
        let point = Self::screen_point(hwnd, x, y);
        let command = TrackPopupMenuEx(menu, TPM_RETURNCMD.0, point.x, point.y, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        if command == 0 {
            self.menu_guard = Some((2, std::time::Instant::now()));
        }
        if !(830..830 + options.len()).contains(&command) {
            return;
        }
        let mask = options[command - 830].1;
        if mask == 0 {
            self.channel_mask_from_device = true;
            self.channel_mask = self
                .selected_device
                .as_ref()
                .map(|device| device.get_channel_mask())
                .unwrap_or(3);
        } else {
            self.channel_mask_from_device = false;
            self.channel_mask = mask;
        }
        let channel_count = self
            .selected_device
            .as_ref()
            .map(|device| device.get_channel_count() as i32)
            .unwrap_or(16);
        self.channel_names = channel_helper::get_channel_names(channel_count, self.channel_mask);
        self.analysis_channel_index = self
            .analysis_channel_index
            .min(self.channel_names.len().saturating_sub(1));
        for row in &mut self.rows {
            row.item = None;
        }
        self.analysis_result = None;
        self.start_analysis();
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    pub(super) unsafe fn open_analysis_menu(&mut self, hwnd: HWND, x: i32, y: i32, kind: AnalysisSetting) {
        let menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => return,
        };
        let mut menu_texts: Vec<Vec<u16>> = Vec::new();
        match kind {
            AnalysisSetting::StartFrom => {
                let current = if self.analysis_start_from == 1 { 701 } else { 700 };
                for (id, label) in [(700, "config.txt"), (701, "Current file")] {
                    let flags = if id == current {
                        MF_STRING | MF_CHECKED
                    } else {
                        MF_STRING
                    };
                    let _ = AppendMenuW(menu, flags, id, menu_text(&mut menu_texts, label));
                }
            }
            AnalysisSetting::Channel => {
                for (index, name) in self.channel_names.iter().enumerate() {
                    let flags = if index == self.analysis_channel_index {
                        MF_STRING | MF_CHECKED
                    } else {
                        MF_STRING
                    };
                    let _ = AppendMenuW(menu, flags, 720 + index, menu_raw(&mut menu_texts, name));
                }
            }
            AnalysisSetting::Resolution => {
                for (index, resolution) in ANALYSIS_RESOLUTIONS.iter().enumerate() {
                    let flags = if *resolution == self.analysis_resolution {
                        MF_STRING | MF_CHECKED
                    } else {
                        MF_STRING
                    };
                    let _ = AppendMenuW(
                        menu,
                        flags,
                        740 + index,
                        menu_raw(&mut menu_texts, &resolution.to_string()),
                    );
                }
            }
        }
        let point = Self::screen_point(hwnd, x, y);
        let command = TrackPopupMenuEx(menu, TPM_RETURNCMD.0, point.x, point.y, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        if command == 0 {
            let guard = match kind {
                AnalysisSetting::StartFrom => 3,
                AnalysisSetting::Channel => 4,
                AnalysisSetting::Resolution => 5,
            };
            self.menu_guard = Some((guard, std::time::Instant::now()));
        }
        let mut changed = false;
        match kind {
            AnalysisSetting::StartFrom => {
                if command == 700 {
                    self.analysis_start_from = 0;
                    changed = true;
                } else if command == 701 {
                    self.analysis_start_from = 1;
                    changed = true;
                }
            }
            AnalysisSetting::Channel => {
                if (720..720 + self.channel_names.len()).contains(&command) {
                    self.analysis_channel_index = command - 720;
                    changed = true;
                }
            }
            AnalysisSetting::Resolution => {
                if (740..740 + ANALYSIS_RESOLUTIONS.len()).contains(&command) {
                    self.analysis_resolution = ANALYSIS_RESOLUTIONS[command - 740];
                    changed = true;
                }
            }
        }
        if changed {
            self.analysis_result = None;
            self.start_analysis();
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    pub(super) unsafe fn biquad_type_menu(
        &mut self,
        hwnd: HWND,
        row: usize,
        x: i32,
        y: i32,
        menu_kind: BiquadMenu,
    ) {
        let menu = match CreatePopupMenu() {
            Ok(menu) => menu,
            Err(_) => return,
        };
        let mut menu_texts: Vec<Vec<u16>> = Vec::new();
        match menu_kind {
            BiquadMenu::Kind => {
                for (offset, label) in [
                    "Peaking filter",
                    "Low-pass filter",
                    "High-pass filter",
                    "Band-pass filter",
                    "Low-shelf filter",
                    "High-shelf filter",
                    "Notch filter",
                    "All-pass filter",
                ]
                .iter()
                .enumerate()
                {
                    let _ = AppendMenuW(
                        menu,
                        MF_STRING,
                        400 + offset,
                        menu_text(&mut menu_texts, label),
                    );
                }
            }
            BiquadMenu::QMode => {
                for (offset, label) in ["Fixed", "Q", "Bandwidth", "Slope"].iter().enumerate() {
                    let _ = AppendMenuW(
                        menu,
                        MF_STRING,
                        420 + offset,
                        menu_text(&mut menu_texts, label),
                    );
                }
            }
        }
        let point = Self::screen_point(hwnd, x, y);
        let command = TrackPopupMenuEx(menu, TPM_RETURNCMD.0, point.x, point.y, hwnd, None).0 as usize;
        let _ = DestroyMenu(menu);
        if command == 0 {
            let guard = if matches!(menu_kind, BiquadMenu::Kind) { 7 } else { 8 };
            self.menu_guard = Some((guard, std::time::Instant::now()));
        }
        let mut changed = false;
        if let Some(item) = self.rows[row].item.as_mut() {
            if let Some(FilterEditor::Biquad(biquad)) = &mut item.editor {
                match menu_kind {
                    BiquadMenu::Kind => {
                        let kinds = [
                            BiquadKind::Peaking,
                            BiquadKind::LowPass,
                            BiquadKind::HighPass,
                            BiquadKind::BandPass,
                            BiquadKind::LowShelf,
                            BiquadKind::HighShelf,
                            BiquadKind::Notch,
                            BiquadKind::AllPass,
                        ];
                        if (400..408).contains(&command) {
                            biquad.kind = kinds[command - 400];
                            changed = true;
                        }
                    }
                    BiquadMenu::QMode => {
                        let modes = [QMode::Fixed, QMode::Q, QMode::Bandwidth, QMode::Slope];
                        if (420..424).contains(&command) {
                            biquad.q_mode = modes[command - 420];
                            changed = true;
                        }
                    }
                }
            }
        }
        if changed {
            self.sync_item(hwnd, row);
            self.save_hotkeys();
            self.start_analysis();
            self.autosave(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    /// Converts client coordinates to screen coordinates for popup menus.
    pub(super) unsafe fn screen_point(hwnd: HWND, x: i32, y: i32) -> POINT {
        let mut point = POINT { x, y };
        let _ = ClientToScreen(hwnd, &mut point);
        point
    }

    /// Returns false when this click only dismissed a menu that was just
    /// closed, so dropdown buttons behave as toggles instead of reopening.
    pub(super) fn menu_toggle_allowed(&mut self, kind: u8) -> bool {
        if let Some((previous, at)) = self.menu_guard {
            if previous == kind
                && at.elapsed() < std::time::Duration::from_millis(400)
            {
                self.menu_guard = None;
                return false;
            }
        }
        self.menu_guard = None;
        true
    }

}
