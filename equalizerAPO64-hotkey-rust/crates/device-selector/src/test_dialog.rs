//! Port of `DeviceTestDialog.{h,cpp}`.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::Arc;

use eapo_apo_info::{AbstractAPOInfo, DeviceAPOInfo};
use eapo_common::registry_helper::RegistryException;
use egui::{Color32, RichText};

use crate::test_thread::{DeviceTestThread, ItemStatus, TestEvent};

#[derive(Clone)]
enum CellStatus {
    None,
    Waiting,
    Success,
    Warning,
    Error,
    Unavailable(String),
}

impl CellStatus {
    fn static_circle(tooltip: &str) -> Self {
        Self::Unavailable(tooltip.to_string())
    }
}

struct TestDeviceEntry {
    info: Arc<DeviceAPOInfo>,
    group: usize,
    pre_mix: CellStatus,
    post_mix: CellStatus,
}

struct LogLine {
    text: String,
    error: bool,
}

pub struct DeviceTestDialog {
    entries: Vec<TestDeviceEntry>,
    groups: Vec<&'static str>,
    item_map: HashMap<String, usize>,
    log_lines: Vec<LogLine>,
    thread_finished: bool,
    sender: Sender<TestEvent>,
    receiver: Receiver<TestEvent>,
    cancel: Arc<AtomicBool>,
    error_dialog: Option<String>,
    abort_dialog: Option<String>,
}

impl DeviceTestDialog {
    pub fn new() -> Self {
        let mut entries = Vec::new();
        let mut groups = Vec::new();
        let mut item_map = HashMap::new();
        let mut registry_error: Option<String> = None;

        match filter_devices(false) {
            Ok(devices) if !devices.is_empty() => {
                let group = groups.len();
                groups.push("Playback devices");
                add_devices(&mut entries, &mut item_map, &devices, group);
            }
            Ok(_) => {}
            Err(error) => registry_error = Some(error.to_string()),
        }
        if registry_error.is_none() {
            match filter_devices(true) {
                Ok(devices) if !devices.is_empty() => {
                    let group = groups.len();
                    groups.push("Capture devices");
                    add_devices(&mut entries, &mut item_map, &devices, group);
                }
                Ok(_) => {}
                Err(error) => registry_error = Some(error.to_string()),
            }
        }

        let (sender, receiver) = channel();
        let cancel = Arc::new(AtomicBool::new(false));
        let devices: Vec<Arc<DeviceAPOInfo>> =
            entries.iter().map(|entry| entry.info.clone()).collect();
        let thread = DeviceTestThread::new(devices, sender.clone(), cancel.clone());
        thread.start();

        Self {
            entries,
            groups,
            item_map,
            log_lines: Vec::new(),
            thread_finished: false,
            sender,
            receiver,
            cancel,
            error_dialog: registry_error,
            abort_dialog: None,
        }
    }

    /// Draws the dialog. Returns `Some(result)` when it should be closed:
    /// 1 = OK, 0 = window closed, -1 = aborted.
    pub fn show(&mut self, ctx: &egui::Context) -> Option<i32> {
        self.process_events();

        let mut close_result = None;
        let mut close_requested = false;
        let mut abort_acknowledged = false;

        egui::Modal::new(egui::Id::new("device_test_dialog")).show(ctx, |ui| {
            ui.set_min_width(580.0);
            ui.set_min_height(360.0);
            ui.horizontal(|ui| {
                ui.heading(crate::i18n::tr("Testing APO installation on devices..."));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if !self.thread_finished && self.abort_dialog.is_none() {
                        if ui
                            .button("X")
                            .on_hover_text(crate::i18n::tr("Close and stop testing"))
                            .clicked()
                        {
                            close_requested = true;
                        }
                    }
                });
            });
            ui.separator();

            egui::ScrollArea::vertical()
                .max_height(240.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("device_test_grid")
                        .num_columns(4)
                        .spacing([16.0, 4.0])
                        .show(ui, |ui| {
                            ui.label(RichText::new("Connector").strong());
                            ui.label(RichText::new("Device").strong());
                            ui.label(RichText::new("Pre-mix").strong());
                            ui.label(RichText::new("Post-mix").strong());
                            ui.end_row();

                            for (group_index, group_name) in self.groups.iter().enumerate() {
                                ui.label(RichText::new(*group_name).strong());
                                ui.label("");
                                ui.label("");
                                ui.label("");
                                ui.end_row();
                                for entry in self
                                    .entries
                                    .iter()
                                    .filter(|entry| entry.group == group_index)
                                {
                                    ui.label(RichText::new(&entry.info.get_connection_name()));
                                    ui.label(RichText::new(&entry.info.get_device_name()));
                                    draw_status_cell(ui, &entry.pre_mix);
                                    draw_status_cell(ui, &entry.post_mix);
                                    ui.end_row();
                                }
                            }
                        });
                });

            // Group headers cannot span grid columns; the groups are drawn
            // as bold rows above their devices, matching the tree layout.
            ui.separator();
            egui::ScrollArea::vertical()
                .id_salt("test_status_output")
                .max_height(110.0)
                .stick_to_bottom(true)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for line in &self.log_lines {
                        let mut text = line.text.replace("<b>", "").replace("</b>", "");
                        text = text
                            .replace("<span style=\"color:red\">", "")
                            .replace("</span>", "");
                        let mut rich = RichText::new(text);
                        if line.error {
                            rich = rich.color(Color32::RED);
                        }
                        if line.text.contains("<b>") {
                            rich = rich.strong();
                        }
                        ui.label(rich);
                    }
                });

            ui.add_space(4.0);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui
                    .add_enabled(self.thread_finished, egui::Button::new(crate::i18n::tr("OK")))
                    .clicked()
                {
                    close_result = Some(1);
                }
            });
        });

        if let Some(message) = self.abort_dialog.clone() {
            egui::Modal::new(egui::Id::new("device_test_abort")).show(ctx, |ui| {
                ui.heading(crate::i18n::tr("Error"));
                ui.label(&message);
                ui.add_space(4.0);
                if ui.button(crate::i18n::tr("OK")).clicked() {
                    abort_acknowledged = true;
                }
            });
            if abort_acknowledged {
                self.abort_dialog = None;
                return Some(-1);
            }
            return None;
        }

        if let Some(message) = self.error_dialog.clone() {
            let mut acknowledged = false;
            egui::Modal::new(egui::Id::new("device_test_error")).show(ctx, |ui| {
                ui.heading(crate::i18n::tr("Error"));
                ui.label(&message);
                ui.add_space(4.0);
                if ui.button(crate::i18n::tr("OK")).clicked() {
                    acknowledged = true;
                }
            });
            if acknowledged {
                self.error_dialog = None;
            }
            return None;
        }

        if close_requested {
            self.cancel.store(true, Ordering::Relaxed);
            return Some(0);
        }
        close_result
    }

    fn process_events(&mut self) {
        while let Ok(event) = self.receiver.try_recv() {
            match event {
                TestEvent::Log(message) => self.log_lines.push(LogLine {
                    text: message,
                    error: false,
                }),
                TestEvent::LogError(message) => self.log_lines.push(LogLine {
                    text: message,
                    error: true,
                }),
                TestEvent::ShowErrorDialog(message) => self.error_dialog = Some(message),
                TestEvent::Abort(message, _code) => self.abort_dialog = Some(message),
                TestEvent::SetItemStatus {
                    guid,
                    post_mix,
                    status,
                } => {
                    if let Some(index) = self.item_map.get(&guid.to_lowercase()) {
                        if let Some(entry) = self.entries.get_mut(*index) {
                            let cell = match status {
                                ItemStatus::Waiting => CellStatus::Waiting,
                                ItemStatus::Success => CellStatus::Success,
                                ItemStatus::Warning => CellStatus::Warning,
                                ItemStatus::Error => CellStatus::Error,
                            };
                            if post_mix {
                                entry.post_mix = cell;
                            } else {
                                entry.pre_mix = cell;
                            }
                        }
                    }
                }
                TestEvent::Finished => self.thread_finished = true,
            }
        }
    }
}

impl Drop for DeviceTestDialog {
    fn drop(&mut self) {
        self.cancel.store(true, Ordering::Relaxed);
        // The sender is kept alive by the thread; dropping the dialog leaves
        // the thread running until it notices the cancel flag. The original
        // called QThread::terminate() here, which has no safe Rust equivalent.
        let _ = &self.sender;
    }
}

fn filter_devices(input: bool) -> Result<Vec<Arc<DeviceAPOInfo>>, RegistryException> {
    let mut filtered = Vec::new();
    for info in DeviceAPOInfo::load_all_infos(input)? {
        if !info.is_installed() {
            continue;
        }
        if let Ok(device) = info.clone().as_any_arc().downcast::<DeviceAPOInfo>() {
            filtered.push(device);
        }
    }
    Ok(filtered)
}

fn add_devices(
    entries: &mut Vec<TestDeviceEntry>,
    item_map: &mut HashMap<String, usize>,
    devices: &[Arc<DeviceAPOInfo>],
    group: usize,
) {
    for info in devices {
        let selected = info.get_selected_install_state();
        let (pre_mix, post_mix) = if info.is_disabled() {
            (
                CellStatus::static_circle("Cannot test APO installation as device is disabled"),
                CellStatus::static_circle("Cannot test APO installation as device is disabled"),
            )
        } else if info.is_unplugged() {
            (
                CellStatus::static_circle("Cannot test APO installation as device is unplugged"),
                CellStatus::static_circle("Cannot test APO installation as device is unplugged"),
            )
        } else {
            (CellStatus::Waiting, CellStatus::Waiting)
        };

        let entry = TestDeviceEntry {
            info: info.clone(),
            group,
            pre_mix: if selected.install_pre_mix {
                pre_mix
            } else {
                CellStatus::None
            },
            post_mix: if selected.install_post_mix && !info.is_input() {
                post_mix
            } else {
                CellStatus::None
            },
        };
        let index = entries.len();
        item_map.insert(info.get_device_guid().to_lowercase(), index);
        entries.push(entry);
    }
}

fn draw_status_cell(ui: &mut egui::Ui, status: &CellStatus) {
    match status {
        CellStatus::None => {
            ui.label("");
        }
        CellStatus::Waiting => {
            ui.add(egui::Spinner::new().size(12.0));
        }
        CellStatus::Success => {
            ui.label(RichText::new("●").color(Color32::from_rgb(0, 160, 0)));
        }
        CellStatus::Warning => {
            ui.label(RichText::new("●").color(Color32::from_rgb(230, 140, 0))).on_hover_text(crate::i18n::tr("Equalizer APO works but original APO could not be initialized. Maybe unset \"Use original APO\" in troubleshooting options."));
        }
        CellStatus::Error => {
            ui.label(RichText::new("●").color(Color32::RED));
        }
        CellStatus::Unavailable(tooltip) => {
            ui.label(RichText::new("○").color(Color32::GRAY))
                .on_hover_text(tooltip);
        }
    }
}
