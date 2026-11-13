//! Tab and document lifecycle: opening, switching, saving and closing.

use super::*;

impl App {
    /// The document of every tab. The frontmost tab comes from the live state
    /// because `tabs` is only refreshed when a tab is captured.
    pub(super) fn tab_paths(&self) -> Vec<Option<PathBuf>> {
        let mut paths: Vec<Option<PathBuf>> =
            self.tabs.iter().map(|tab| tab.path.clone()).collect();
        if let Some(slot) = paths.get_mut(self.active_tab) {
            *slot = self.doc_path.clone();
        }
        paths
    }
    pub(super) fn doc_title(&self) -> String {
        self.doc_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| i18n::tr("Unsaved"))
    }

    pub(super) fn capture_active_tab(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        let index = self.active_tab.min(self.tabs.len() - 1);
        let lines: Vec<String> = self.rows.iter().map(|row| row.line.clone()).collect();
        let title = self.doc_title();
        let tab = &mut self.tabs[index];
        tab.title = title;
        tab.path = self.doc_path.clone();
        tab.lines = lines;
        tab.offset = self.offset;
        tab.selected = self.selected;
        tab.selection = self.selection.clone();
        tab.selection_anchor = self.selection_anchor;
        tab.dirty = self.dirty;
    }

    pub(super) unsafe fn load_tab(&mut self, hwnd: HWND, index: usize) {
        let Some(tab) = self.tabs.get(index) else {
            return;
        };
        let lines = tab.lines.clone();
        let path = tab.path.clone();
        let offset = tab.offset;
        let selected = tab.selected;
        let selection = tab.selection.clone();
        let selection_anchor = tab.selection_anchor;
        let dirty = tab.dirty;
        self.active_tab = index;
        self.doc_path = path;
        self.set_lines(lines);
        self.reload_hotkeys();
        self.offset = offset;
        self.selected = selected;
        self.selection = if selection.is_empty() {
            selected.into_iter().collect()
        } else {
            selection
        };
        self.selection_anchor = selection_anchor.or(selected);
        self.dirty = dirty;
        self.edit = None;
        self.drag = None;
        self.analysis_result = None;
        self.update_scrollbar(hwnd);
        self.update_title(hwnd);
        self.start_analysis();
        self.publish_live_files(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    pub(super) unsafe fn switch_tab(&mut self, hwnd: HWND, index: usize) {
        if index == self.active_tab || index >= self.tabs.len() {
            return;
        }
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        self.flush_autosave(hwnd);
        self.capture_active_tab();
        self.load_tab(hwnd, index);
    }

    pub(super) unsafe fn new_tab(&mut self, hwnd: HWND) {
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        self.flush_autosave(hwnd);
        self.capture_active_tab();
        let index = self.tabs.len();
        self.tabs.push(TabInfo {
            title: i18n::tr("Unsaved"),
            path: None,
            lines: vec![String::new()],
            offset: 0,
            selected: None,
            selection: HashSet::new(),
            selection_anchor: None,
            dirty: false,
        });
        self.load_tab(hwnd, index);
    }

    pub(super) unsafe fn open_paths(&mut self, hwnd: HWND, paths: Vec<PathBuf>) {
        if paths.is_empty() {
            return;
        }
        self.flush_autosave(hwnd);
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        self.capture_active_tab();
        let mut last = None;
        for path in paths {
            if let Some(index) = self
                .tabs
                .iter()
                .position(|tab| tab.path.as_ref().is_some_and(|open| same_file(open, &path)))
            {
                settings::push_recent(
                    &mut self.recent_files,
                    &path.to_string_lossy(),
                );
                self.rebuild_file_menu(hwnd);
                last = Some(index);
                continue;
            }
            let path_string = path.to_string_lossy().into_owned();
            let lines = match load_config_lines(&path_string) {
                Ok(lines) => lines,
                Err(error) => {
                    let message = wide_z(&i18n::tr_arg(
                        "Could not read %0: %1",
                        &[&path.display().to_string(), &error.to_string()],
                    ));
                    let _ = MessageBoxW(
                        Some(hwnd),
                        PCWSTR(message.as_ptr()),
                        w!("Equalizer APO"),
                        MB_ICONWARNING,
                    );
                    continue;
                }
            };
            settings::push_recent(&mut self.recent_files, &path_string);
            self.rebuild_file_menu(hwnd);
            let title = path
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_else(|| path.to_string_lossy().into_owned());
            self.tabs.push(TabInfo {
                title,
                path: Some(path),
                lines,
                offset: 0,
                selected: None,
                selection: HashSet::new(),
                selection_anchor: None,
                dirty: false,
            });
            last = Some(self.tabs.len() - 1);
        }
        if let Some(index) = last {
            self.load_tab(hwnd, index);
        }
    }

    pub(super) unsafe fn open_dialog(&mut self, hwnd: HWND) {
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        let mut dialog = FileDialog::new()
            .set_title(&i18n::tr("Open configuration"))
            .add_filter("E-APO configurations (*.txt)", &["txt"]);
        if let Some(parent) = self.doc_path.as_ref().and_then(|path| path.parent()) {
            dialog = dialog.set_directory(parent);
        }
        if let Some(paths) = dialog.pick_files() {
            self.open_paths(hwnd, paths);
        }
    }

    pub(super) unsafe fn save_as(&mut self, hwnd: HWND) -> bool {
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        let mut dialog = FileDialog::new()
            .set_title(&i18n::tr("Save configuration as"))
            .add_filter("E-APO configurations (*.txt)", &["txt"]);
        if let Some(path) = &self.doc_path {
            if let Some(parent) = path.parent() {
                dialog = dialog.set_directory(parent);
            }
            if let Some(name) = path.file_name() {
                dialog = dialog.set_file_name(name.to_string_lossy());
            }
        } else {
            dialog = dialog.set_file_name("config.txt");
        }
        let Some(path) = dialog.save_file() else {
            return false;
        };
        let path = if path.extension().is_none() {
            path.with_extension("txt")
        } else {
            path
        };
        let previous_path = self.doc_path.clone();
        self.doc_path = Some(path);
        let saved = self.save(hwnd);
        if saved {
            let path_string = self
                .doc_path
                .as_ref()
                .map(|path| path.to_string_lossy().into_owned())
                .unwrap_or_default();
            settings::push_recent(&mut self.recent_files, &path_string);
            self.rebuild_file_menu(hwnd);
            self.capture_active_tab();
            self.save_hotkeys();
            self.update_title(hwnd);
            let _ = InvalidateRect(Some(hwnd), None, false);
        } else {
            self.doc_path = previous_path;
        }
        saved
    }

    pub(super) unsafe fn rebuild_file_menu(&mut self, hwnd: HWND) {
        let file = GetSubMenu(GetMenu(hwnd), 0);
        if file.0.is_null() {
            return;
        }
        build_file_menu(file, &self.recent_files, &mut self.menu_labels);
        let _ = DrawMenuBar(hwnd);
    }

    pub(super) unsafe fn update_title(&self, hwnd: HWND) {
        let name = self
            .doc_path
            .as_ref()
            .and_then(|path| path.file_name())
            .map(|name| name.to_string_lossy().into_owned())
            .unwrap_or_else(|| i18n::tr("Unsaved"));
        let suffix = if self.dirty { " *" } else { "" };
        // Mark the document the audio engine currently follows for this device.
        let live = self.live_mode != LiveMode::Off
            && self
                .doc_path
                .as_ref()
                .is_some_and(|path| self.live_published.as_ref().is_some_and(|(_, files)| files.contains(path)));
        let live_mark = if live { " [live]" } else { "" };
        let title = format!("Equalizer APO Configuration Editor - {name}{suffix}{live_mark}");
        let mut current = [0u16; 512];
        let length = GetWindowTextW(hwnd, &mut current).max(0) as usize;
        if length > 0
            && String::from_utf16_lossy(&current[..length.min(current.len())]) == title
        {
            return;
        }
        let title = wide_z(&title);
        let _ = SetWindowTextW(hwnd, PCWSTR(title.as_ptr()));
    }

    pub(super) unsafe fn save(&mut self, hwnd: HWND) -> bool {
        self.autosave_due = None;
        // Finish any queued background write first so an older snapshot cannot
        // land after this synchronous one.
        self.wait_instant_save(hwnd);
        if self.doc_path.is_none() {
            return self.save_as(hwnd);
        }
        let Some(path) = self.doc_path.clone() else {
            return false;
        };
        let lines: Vec<String> = self.rows.iter().map(|row| row.line.clone()).collect();
        match save_config_file(&path.to_string_lossy(), &lines) {
            Ok(()) => {
                self.dirty = false;
                self.capture_active_tab();
                self.update_title(hwnd);
                self.start_analysis();
                self.publish_live_files(hwnd);
                true
            }
            Err(error) => {
                let text = wide_z(&i18n::tr_arg(
                    "Could not save %0: %1",
                    &[&path.display().to_string(), &error.to_string()],
                ));
                let _ = MessageBoxW(
                    Some(hwnd),
                    PCWSTR(text.as_ptr()),
                    w!("Equalizer APO"),
                    MB_ICONWARNING,
                );
                false
            }
        }
    }

    pub(super) unsafe fn autosave(&mut self, _hwnd: HWND) {
        if self.instant_mode && self.doc_path.is_some() {
            // Debounce: a slider drag produces dozens of edits per second and
            // every write can block while the engine has the file open.
            self.autosave_due = Some(
                std::time::Instant::now() + std::time::Duration::from_millis(300),
            );
        }
    }

    /// Writes a pending instant-mode change before the document changes
    /// underneath it (tab switch, open, close).
    pub(super) unsafe fn flush_autosave(&mut self, hwnd: HWND) {
        if self.autosave_due.take().is_some() && self.instant_mode && self.doc_path.is_some() {
            self.save(hwnd);
        }
    }

    /// Hands the current snapshot to the background writer.  Used by the
    /// debounced instant-mode path, where waiting for the engine to release
    /// the file would freeze dragging.
    pub(super) fn begin_instant_save(&mut self) {
        self.autosave_due = None;
        if !self.instant_mode {
            return;
        }
        let Some(path) = self.doc_path.clone() else {
            return;
        };
        let Some(sender) = self.instant_save_tx.as_ref() else {
            return;
        };
        self.instant_save_serial += 1;
        let serial = self.instant_save_serial;
        let lines: Vec<String> = self.rows.iter().map(|row| row.line.clone()).collect();
        if sender.send((serial, path, lines)).is_ok() {
            self.instant_save_in_flight += 1;
        }
    }

    /// Collects background write results and clears the dirty flag only when
    /// no newer snapshot was queued in the meantime.
    pub(super) unsafe fn poll_instant_save(&mut self, hwnd: HWND) {
        loop {
            let outcome = self.instant_save_rx.try_recv();
            let Ok((serial, result)) = outcome else {
                break;
            };
            self.instant_save_in_flight = self.instant_save_in_flight.saturating_sub(1);
            match result {
                Ok(()) => {
                    // The snapshot is on disk now, so the analysis panel can
                    // reflect it.
                    self.start_analysis();
                    // A newer edit may already be scheduled but not sent yet.
                    if serial == self.instant_save_serial && self.autosave_due.is_none() {
                        self.dirty = false;
                        self.update_title(hwnd);
                    }
                }
                Err(error) => {
                    self.report_save_error(hwnd, &error);
                }
            }
        }
    }

    /// Blocks until every queued background write has finished.  Called by
    /// synchronous saves so a stale snapshot can never land after the newer
    /// one.
    pub(super) unsafe fn wait_instant_save(&mut self, hwnd: HWND) {
        while self.instant_save_in_flight > 0 {
            match self.instant_save_rx.recv() {
                Ok((serial, result)) => {
                    self.instant_save_in_flight -= 1;
                    if result.is_ok() && serial == self.instant_save_serial {
                        self.dirty = false;
                    } else if let Err(error) = result {
                        self.report_save_error(hwnd, &error);
                    }
                }
                Err(_) => {
                    self.instant_save_in_flight = 0;
                }
            }
        }
    }

    unsafe fn report_save_error(&self, hwnd: HWND, error: &std::io::Error) {
        let Some(path) = self.doc_path.clone() else {
            return;
        };
        let text = wide_z(&i18n::tr_arg(
            "Could not save %0: %1",
            &[&path.display().to_string(), &error.to_string()],
        ));
        let _ = MessageBoxW(
            Some(hwnd),
            PCWSTR(text.as_ptr()),
            w!("Equalizer APO"),
            MB_ICONWARNING,
        );
    }

    /// Asks for every dirty tab before the window closes.  Saving a tab that is
    /// not in front requires loading it, so the original tab is restored when
    /// the user cancels.
    pub(super) unsafe fn confirm_close(&mut self, hwnd: HWND) -> bool {
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        self.flush_autosave(hwnd);
        self.capture_active_tab();
        let original = self.active_tab;
        for index in 0..self.tabs.len() {
            if !self.tabs[index].dirty {
                continue;
            }
            let title = wide_z(&i18n::tr_arg(
                "Save changes to %0 before closing?",
                &[&self.tabs[index].title],
            ));
            let answer = MessageBoxW(
                Some(hwnd),
                PCWSTR(title.as_ptr()),
                w!("Equalizer APO"),
                MB_YESNOCANCEL | MB_ICONWARNING,
            );
            if answer.0 == IDCANCEL.0 {
                self.load_tab(hwnd, original);
                return false;
            }
            if answer.0 == IDYES.0 {
                self.load_tab(hwnd, index);
                if !self.save(hwnd) {
                    self.load_tab(hwnd, original);
                    return false;
                }
            }
        }
        true
    }

    pub(super) unsafe fn close_tab(&mut self, hwnd: HWND, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if index != self.active_tab {
            self.switch_tab(hwnd, index);
        }
        self.commit_value_edit(hwnd, true);
        self.commit_edit(hwnd, true);
        self.flush_autosave(hwnd);
        self.capture_active_tab();
        if self.tabs[index].dirty {
            let title = wide_z(&i18n::tr_arg(
                "Save changes to %0 before closing?",
                &[&self.tabs[index].title],
            ));
            let answer = MessageBoxW(
                Some(hwnd),
                PCWSTR(title.as_ptr()),
                w!("Equalizer APO"),
                MB_YESNOCANCEL | MB_ICONWARNING,
            );
            if answer.0 == IDCANCEL.0 {
                return;
            }
            if answer.0 == IDYES.0 && !self.save(hwnd) {
                return;
            }
        }
        self.tabs.remove(index);
        let no_saved_tabs = self.tabs.iter().all(|tab| tab.path.is_none());
        if self.tabs.is_empty() {
            self.tabs.push(TabInfo {
                title: i18n::tr("Unsaved"),
                path: None,
                lines: vec![String::new()],
                offset: 0,
                selected: None,
                selection: HashSet::new(),
                selection_anchor: None,
                dirty: false,
            });
        }
        let next = next_tab_after_close(self.tabs.len(), index);
        self.active_tab = usize::MAX;
        self.load_tab(hwnd, next);
        // Closing the last saved document means the editor no longer follows
        // any file for this device, so remove the live manifest and let the
        // engine fall back to its default configuration.
        if no_saved_tabs && self.live_mode != LiveMode::Off && self.live_published.is_some() {
            self.clear_live_files();
            self.update_title(hwnd);
        }
    }


}
