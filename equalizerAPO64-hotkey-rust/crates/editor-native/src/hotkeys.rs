//! Per-row hotkey persistence and registration for the native editor.

use super::*;

impl App {
    pub(super) fn reload_hotkeys(&mut self) {
        self.hotkeys.clear_all();
        for row in &mut self.rows {
            row.hotkey = None;
        }
        let Some(path) = self.doc_path.clone() else {
            return;
        };
        let text_path = path.to_string_lossy();
        let bindings = settings::load_hotkey_bindings(&text_path);
        if !bindings.is_empty() {
            let lines: Vec<String> = self.rows.iter().map(|row| row.line.clone()).collect();
            let keys = settings::hotkey_keys(&lines);
            for (row, key) in self.rows.iter_mut().zip(keys) {
                let Some(sequence) = bindings.get(&key).copied() else {
                    continue;
                };
                row.hotkey = Some(sequence);
                let filter_id = format!("{:x}", row.uid);
                let _ = self.hotkeys.register_hotkey(&filter_id, Some(sequence));
            }
            return;
        }
        // Files written before `hotkeyBindings` existed only have the row
        // number in `rowPrefs`.  Verify the command prefix so a shifted row
        // never receives another filter's hotkey.
        let hotkeys = settings::load_row_hotkeys(&text_path);
        for (index, (command, sequence)) in hotkeys {
            if index == 0 || index > self.rows.len() {
                continue;
            }
            let row = &mut self.rows[index - 1];
            let row_command = row.line.split(':').next().unwrap_or("").trim();
            if row_command != command {
                continue;
            }
            row.hotkey = Some(sequence);
            let filter_id = format!("{:x}", row.uid);
            let _ = self.hotkeys.register_hotkey(&filter_id, Some(sequence));
        }
    }

    pub(super) fn save_hotkeys(&self) {
        let Some(path) = self.doc_path.clone() else {
            return;
        };
        let path = path.to_string_lossy();
        let lines: Vec<String> = self.rows.iter().map(|row| row.line.clone()).collect();
        let keys = settings::hotkey_keys(&lines);
        let bindings: Vec<(String, KeySequence)> = self
            .rows
            .iter()
            .zip(keys)
            .filter_map(|(row, key)| row.hotkey.map(|sequence| (key, sequence)))
            .collect();
        settings::save_hotkey_bindings(&path, &bindings);
        let updates: Vec<(usize, String, serde_json::Map<String, serde_json::Value>)> = self
            .rows
            .iter()
            .enumerate()
            .map(|(index, row)| {
                let command = row
                    .line
                    .split(':')
                    .next()
                    .unwrap_or("")
                    .trim()
                    .to_string();
                let mut prefs = row.prefs.clone();
                prefs.remove("hotkey");
                if let Some(sequence) = row.hotkey {
                    prefs.insert(
                        "hotkey".to_string(),
                        serde_json::Value::String(sequence.to_string_qt()),
                    );
                }
                (index + 1, command, prefs)
            })
            .collect();
        settings::save_row_state(&path, &updates);
    }

    pub(super) unsafe fn set_hotkey_for_row(&mut self, hwnd: HWND, row: usize) {
        if row >= self.rows.len() {
            return;
        }
        self.ensure_item(row);
        if self.rows[row]
            .item
            .as_ref()
            .and_then(|item| item.editor.as_ref())
            .is_none()
        {
            let text = wide_z(&i18n::tr("This row does not have a filter editor."));
            let _ = MessageBoxW(
                Some(hwnd),
                PCWSTR(text.as_ptr()),
                w!("Equalizer APO"),
                MB_ICONWARNING,
            );
            return;
        }
        let Some(sequence) = prompt_hotkey(hwnd, &i18n::tr("Set filter hotkey"), self.rows[row].hotkey) else {
            return;
        };
        // A held Alt key would otherwise activate the editor menu bar (a
        // visible flash) right after the dialog closes.
        if GetKeyState(VK_MENU.0 as i32) < 0 {
            SUPPRESS_KEYMENU.store(true, Ordering::Relaxed);
        }
        let filter_id = format!("{:x}", self.rows[row].uid);
        self.hotkeys.unregister_hotkey(&filter_id);
        if let Some(sequence) = sequence {
            if !self.hotkeys.register_hotkey(&filter_id, Some(sequence)) {
                let message = wide_z(&i18n::tr("The hotkey could not be registered. It may already be in use."));
                let _ = MessageBoxW(
                    Some(hwnd),
                    PCWSTR(message.as_ptr()),
                    w!("Equalizer APO"),
                    MB_ICONWARNING,
                );
                self.reload_hotkeys();
                return;
            }
        }
        self.rows[row].hotkey = sequence;
        self.save_hotkeys();
        self.invalidate_row(hwnd, row);
    }

    pub(super) unsafe fn clear_hotkey_for_row(&mut self, hwnd: HWND, row: usize) {
        if row >= self.rows.len() {
            return;
        }
        let filter_id = format!("{:x}", self.rows[row].uid);
        self.hotkeys.unregister_hotkey(&filter_id);
        self.rows[row].hotkey = None;
        self.save_hotkeys();
        self.invalidate_row(hwnd, row);
    }

    pub(super) unsafe fn handle_hotkey(&mut self, hwnd: HWND, filter_id: &str) {
        if let Some(index) = self
            .rows
            .iter()
            .position(|row| format!("{:x}", row.uid) == filter_id)
        {
            self.toggle_row(hwnd, index);
            self.start_analysis();
        }
    }
}
