//! Preferences, view menu synchronisation and restart handling.

use super::*;

impl App {
    /// Clears the editor preferences (like the original Settings menu) and
    /// restarts the editor so the defaults take effect.
    pub(super) unsafe fn reset_preferences(&mut self, hwnd: HWND, file_specific: bool) {
        let message = wide_z(&i18n::tr(
            "Configuration Editor will be restarted to apply the changed settings. Proceed?",
        ));
        let title = wide_z(&i18n::tr("Restart required"));
        let answer = MessageBoxW(
            Some(hwnd),
            PCWSTR(message.as_ptr()),
            PCWSTR(title.as_ptr()),
            MB_YESNO | MB_ICONQUESTION,
        );
        if answer.0 != IDYES.0 {
            return;
        }
        use eapo_common::registry_helper;
        if file_specific {
            let _ = registry_helper::delete_key_tree(settings::EDITOR_PER_FILE_REGPATH);
        } else {
            let root = settings::EDITOR_REGPATH;
            for name in registry_helper::enum_value_names(root).unwrap_or_default() {
                let _ = registry_helper::delete_value(root, &name);
            }
            for child in registry_helper::enum_sub_keys(root).unwrap_or_default() {
                if child.eq_ignore_ascii_case("file-specific") {
                    continue;
                }
                let _ = registry_helper::delete_key_tree(&format!("{root}\\{child}"));
            }
        }
        self.skip_save_preferences = true;
        self.restart_after_close = true;
        if !file_specific {
            // The restart comes back with live mode off, so drop this device's
            // manifest entry instead of leaving it applied behind the scenes.
            self.clear_live_files();
        }
        let _ = PostMessageW(Some(hwnd), WM_CLOSE, WPARAM(0), LPARAM(0));
    }

    /// Keeps the View menu check marks in sync with the current layout.
    pub(super) unsafe fn sync_view_menu(&self, hwnd: HWND) {
        let view = GetSubMenu(GetMenu(hwnd), 2);
        if view.0.is_null() {
            return;
        }
        let checked = |on: bool| if on { MF_CHECKED.0 } else { MF_UNCHECKED.0 };
        let _ = CheckMenuItem(view, 123, MF_BYCOMMAND.0 | checked(self.show_toolbar));
        let _ = CheckMenuItem(view, 122, MF_BYCOMMAND.0 | checked(self.show_analysis));
    }

    /// Keeps the live-configuration radio in the Settings menu in sync.
    pub(super) unsafe fn sync_live_menu(&self, hwnd: HWND) {
        let settings = GetSubMenu(GetMenu(hwnd), 3);
        if settings.0.is_null() {
            return;
        }
        let checked = |mode: LiveMode| {
            if self.live_mode == mode {
                MF_CHECKED.0
            } else {
                MF_UNCHECKED.0
            }
        };
        let _ = CheckMenuItem(settings, 130, MF_BYCOMMAND.0 | checked(LiveMode::Off));
        let _ = CheckMenuItem(settings, 131, MF_BYCOMMAND.0 | checked(LiveMode::ActiveTab));
        let _ = CheckMenuItem(settings, 132, MF_BYCOMMAND.0 | checked(LiveMode::AllTabs));
    }

    pub(super) unsafe fn save_preferences(&mut self, hwnd: HWND) {
        let mut prefs = self.prefs.clone();
        prefs.instant_mode = self.instant_mode;
        prefs.live_mode = self.live_mode;
        prefs.show_toolbar = self.show_toolbar;
        prefs.show_analysis = self.show_analysis;
        prefs.recent_files = self.recent_files.clone();
        prefs.analysis_resolution = self.analysis_resolution;
        prefs.analysis_start_from = self.analysis_start_from;
        // Keep the previous view when the analysis panel was never laid out in
        // this session, otherwise closing the editor would reset it.
        if self.plot_fitted_for != (0, 0) {
            prefs.analysis_zoom_x = self.plot.zoom_x;
            prefs.analysis_zoom_y = self.plot.zoom_y;
            prefs.analysis_scroll_x = self.plot.scroll_x.round() as i32;
            prefs.analysis_scroll_y = self.plot.scroll_y.round() as i32;
        }
        prefs.analysis_channel = self
            .channel_names
            .get(self.analysis_channel_index)
            .cloned()
            .unwrap_or_default();
        prefs.selected_device = self
            .selected_device
            .as_ref()
            .map(|device| device.get_device_string())
            .unwrap_or_default();
        prefs.selected_channel_mask = if self.channel_mask_from_device {
            0
        } else {
            self.channel_mask as i32
        };
        // Use the real screen rectangle so the saved position matches the
        // coordinates CreateWindowExW expects.  Minimized and maximized
        // windows have no meaningful normal rectangle; keep the last values.
        if !IsIconic(hwnd).as_bool() {
            prefs.window_maximized = IsZoomed(hwnd).as_bool();
        }
        if !IsIconic(hwnd).as_bool() && !IsZoomed(hwnd).as_bool() {
            let mut rect = RECT::default();
            if GetWindowRect(hwnd, &mut rect).is_ok() {
                let width = rect.right - rect.left;
                let height = rect.bottom - rect.top;
                if width >= 320 && height >= 240 && rect.left > -30000 && rect.top > -30000 {
                    prefs.window_size = Some((width as f32, height as f32));
                    prefs.window_pos = Some((rect.left as f32, rect.top as f32));
                }
            }
        }
        prefs.save();
        self.prefs = prefs;
        // Persist the per-row GUI preferences together with the hotkeys so
        // VST embedding, GEQ zoom and similar state survives a restart.
        self.save_hotkeys();
    }

}
