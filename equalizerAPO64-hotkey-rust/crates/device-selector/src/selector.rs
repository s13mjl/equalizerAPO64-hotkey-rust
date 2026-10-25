//! Port of `DeviceSelector.{h,cpp}` and the relevant parts of
//! `DeviceSelector.ui`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use eapo_apo_info::{
    AbstractAPOInfo, DeviceAPOInfo, InstallMode, InstallState, VoicemeeterAPOInfo,
};
use eapo_common::registry_helper::RegistryException;
use egui::RichText;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, LUID};
use windows::Win32::Security::{
    AdjustTokenPrivileges, LookupPrivilegeValueW, SE_PRIVILEGE_ENABLED, SE_SHUTDOWN_NAME,
    TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::System::Shutdown::{
    InitiateShutdownW, SHUTDOWN_GRACE_OVERRIDE, SHUTDOWN_RESTART,
    SHTDN_REASON_MAJOR_APPLICATION, SHTDN_REASON_MINOR_MAINTENANCE,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::test_dialog::DeviceTestDialog;

struct DeviceEntry {
    info: Arc<dyn AbstractAPOInfo>,
    checked: bool,
    group: usize,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum MessageKind {
    Info,
    Warning,
    Question,
    Critical,
}

#[derive(Clone)]
enum MessageAction {
    None,
    CancelUpgrade,
    Reboot,
    InstallDone { return_code: i32 },
}

#[derive(Clone)]
struct DialogMessage {
    title: String,
    text: String,
    kind: MessageKind,
    action: MessageAction,
}

pub struct SelectorApp {
    args: Vec<String>,
    devices: Vec<DeviceEntry>,
    selected: Vec<usize>,
    troubleshooting_open: bool,
    available_install_modes: Vec<InstallMode>,
    messages: VecDeque<DialogMessage>,
    active_message: Option<DialogMessage>,
    ask_for_reboot: bool,
    test_dialog: Option<DeviceTestDialog>,
    test_ran: bool,
    test_result: i32,
    pending_apply_complete: Option<bool>,
    closing: bool,
    exit_result: Arc<AtomicI32>,
}

impl SelectorApp {
    pub fn new(args: Vec<String>, exit_result: Arc<AtomicI32>) -> Self {
        let mut devices = Vec::new();
        let mut messages = VecDeque::new();

        let mut load = |input: bool, group: usize, devices: &mut Vec<DeviceEntry>| {
            match DeviceAPOInfo::load_all_infos(input) {
                Ok(infos) => {
                    for info in infos {
                        let voicemeeter =
                            info.as_any().downcast_ref::<VoicemeeterAPOInfo>();
                        let checked = if info.is_installed() {
                            match voicemeeter {
                                Some(voicemeeter) if !voicemeeter.is_voicemeeter_installed() => {
                                    false
                                }
                                _ => true,
                            }
                        } else {
                            false
                        };
                        devices.push(DeviceEntry {
                            info,
                            checked,
                            group,
                        });
                    }
                    true
                }
                Err(error) => {
                    messages.push_back(DialogMessage {
                        title: crate::i18n::tr("Error while accessing the registry"),
                        text: error.to_string(),
                        kind: MessageKind::Critical,
                        action: MessageAction::None,
                    });
                    false
                }
            }
        };

        if load(false, 0, &mut devices) {
            load(true, 1, &mut devices);
        }

        let mut app = Self {
            args,
            devices,
            selected: Vec::new(),
            troubleshooting_open: false,
            available_install_modes: if eapo_apo_info::is_windows_version_at_least(6, 3) {
                vec![
                    InstallMode::InstallLfxGfx,
                    InstallMode::InstallSfxMfx,
                    InstallMode::InstallSfxEfx,
                ]
            } else {
                vec![InstallMode::InstallLfxGfx]
            },
            messages,
            active_message: None,
            ask_for_reboot: false,
            test_dialog: None,
            test_ran: false,
            test_result: 0,
            pending_apply_complete: None,
            closing: false,
            exit_result,
        };

        let fixed_audio_dg = DeviceAPOInfo::check_protected_audio_dg(true)
            .map(|ok| !ok)
            .unwrap_or(false);
        let fixed_registration = !DeviceAPOInfo::check_apo_registration(true);
        if fixed_audio_dg || fixed_registration {
            app.messages.push_back(DialogMessage {
                title: crate::i18n::tr("Info"),
                text: crate::i18n::tr("A registry value that is required for the operation of Equalizer APO was not set correctly. This might have been caused by a driver installation or uninstallation. The value has been corrected. A reboot may be required so that the changes can take effect."),
                kind: MessageKind::Info,
                action: MessageAction::None,
            });
            app.ask_for_reboot = true;
        }

        app
    }

    pub fn update(&mut self, ctx: &egui::Context) {
        if self.closing {
            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            return;
        }

        let close_requested = ctx.input(|input| input.viewport().close_requested());
        if close_requested {
            if self.active_message.is_some() || self.test_dialog.is_some() {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.on_reject();
            }
        }

        if self.active_message.is_none() {
            self.active_message = self.messages.pop_front();
        }

        let mut copy_requested = false;
        let mut cancel_clicked = false;
        let mut ok_clicked = false;

        egui::CentralPanel::default().show(ctx, |ui| {
            ui.label(crate::i18n::tr("Please select the devices for which Equalizer APO is to be installed:"));
            ui.add_space(4.0);

            egui::ScrollArea::vertical()
                .id_salt("device_tree")
                .max_height(230.0)
                .min_scrolled_height(190.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    egui::Grid::new("device_tree_grid")
                        .num_columns(3)
                        .spacing([18.0, 4.0])
                        .striped(false)
                        .show(ui, |ui| {
                            ui.label(RichText::new("Connector").strong());
                            ui.label(RichText::new("Device").strong());
                            ui.label(RichText::new("Status").strong());
                            ui.end_row();

                            for (group_index, group_name) in
                                ["Playback devices", "Capture devices"].iter().enumerate()
                            {
                                let header = ui.label(RichText::new(*group_name).strong());
                                if header.clicked() {
                                    self.selected.clear();
                                }
                                ui.label("");
                                ui.label("");
                                ui.end_row();

                                for index in 0..self.devices.len() {
                                    if self.devices[index].group != group_index {
                                        continue;
                                    }

                                    let checkbox = ui.checkbox(&mut self.devices[index].checked, "");
                                    let _ = checkbox;

                                    let is_selected = self.selected.contains(&index);
                                    let connection = self.devices[index]
                                        .info
                                        .get_connection_name();
                                    let connection_response =
                                        ui.selectable_label(is_selected, connection);
                                    if connection_response.clicked() {
                                        self.select_row(ui, index);
                                    }
                                    if connection_response.secondary_clicked()
                                        && !self.selected.contains(&index)
                                    {
                                        self.selected = vec![index];
                                    }
                                    connection_response.context_menu(|ui| {
                                        if ui
                                            .button("Copy Device command to clipboard")
                                            .clicked()
                                        {
                                            copy_requested = true;
                                            ui.close_menu();
                                        }
                                    });

                                    let name = self.devices[index].info.get_device_name();
                                    let name_response = ui.selectable_label(is_selected, name);
                                    if name_response.clicked() {
                                        self.select_row(ui, index);
                                    }
                                    if name_response.secondary_clicked()
                                        && !self.selected.contains(&index)
                                    {
                                        self.selected = vec![index];
                                    }
                                    name_response.context_menu(|ui| {
                                        if ui
                                            .button("Copy Device command to clipboard")
                                            .clicked()
                                        {
                                            copy_requested = true;
                                            ui.close_menu();
                                        }
                                    });

                                    let status = RichText::new(self.state_text(index));
                                    let status_response = ui.add(
                                        egui::Label::new(status).sense(egui::Sense::click()),
                                    );
                                    if status_response.clicked() {
                                        self.select_row(ui, index);
                                    }
                                    if status_response.secondary_clicked()
                                        && !self.selected.contains(&index)
                                    {
                                        self.selected = vec![index];
                                    }
                                    status_response.context_menu(|ui| {
                                        if ui
                                            .button("Copy Device command to clipboard")
                                            .clicked()
                                        {
                                            copy_requested = true;
                                            ui.close_menu();
                                        }
                                    });
                                    ui.end_row();
                                }
                            }
                        });
                });

            if copy_requested {
                let mut command = String::from("Device: ");
                let mut first = true;
                for index in &self.selected {
                    if let Some(entry) = self.devices.get(*index) {
                        if !first {
                            command.push_str("; ");
                        }
                        first = false;
                        command.push_str(
                            &entry.info.get_device_string().replace(';', " "),
                        );
                    }
                }
                ctx.copy_text(command);
            }

            ui.add_space(4.0);
            ui.checkbox(
                &mut self.troubleshooting_open, crate::i18n::tr("Troubleshooting options (only use in case of problems)"),
            );
            if self.troubleshooting_open {
                egui::Frame::group(ui.style()).show(ui, |ui| {
                    self.draw_troubleshooting(ui);
                });
            }

            ui.add_space(4.0);
            ui.separator();

            let changed = self.is_changed();
            let any_checked = self.devices.iter().any(|entry| entry.checked);
            let show_ok = changed || !any_checked;
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                let cancel_text = if show_ok { "Cancel" } else { "Close" };
                if ui.button(cancel_text).clicked() {
                    cancel_clicked = true;
                }
                if show_ok
                    && ui
                        .add_enabled(changed, egui::Button::new(crate::i18n::tr("OK")))
                        .clicked()
                {
                    ok_clicked = true;
                }
            });
        });

        if cancel_clicked {
            self.on_reject();
        } else if ok_clicked {
            self.apply();
        }
        if self.active_message.is_none() {
            self.active_message = self.messages.pop_front();
        }

        if let Some(message) = self.active_message.clone() {
            let mut result: Option<bool> = None;
            egui::Modal::new(egui::Id::new("device_selector_message")).show(ctx, |ui| {
                ui.set_min_width(360.0);
                ui.heading(&message.title);
                ui.add_space(6.0);
                ui.label(&message.text);
                ui.add_space(8.0);
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    match message.kind {
                        MessageKind::Warning | MessageKind::Question => {
                            if ui.button(crate::i18n::tr("No")).clicked() {
                                result = Some(false);
                            }
                            if ui.button(crate::i18n::tr("Yes")).clicked() {
                                result = Some(true);
                            }
                        }
                        _ => {
                            if ui.button(crate::i18n::tr("OK")).clicked() {
                                result = Some(true);
                            }
                        }
                    }
                });
            });
            if let Some(answer) = result {
                self.active_message = None;
                self.handle_message_action(&message.action, answer);
            }
        }

        if let Some(dialog) = &mut self.test_dialog {
            if let Some(result) = dialog.show(ctx) {
                self.test_dialog = None;
                self.test_ran = true;
                self.test_result = result;
                let device_updated = false;
                self.complete(device_updated);
            }
        }

        if self.active_message.is_none() && self.messages.is_empty() {
            if let Some(device_updated) = self.pending_apply_complete.take() {
                self.complete(device_updated);
            }
        }
    }

    fn select_row(&mut self, ui: &egui::Ui, index: usize) {
        if ui.input(|input| input.modifiers.ctrl) {
            if let Some(position) = self.selected.iter().position(|value| *value == index) {
                self.selected.remove(position);
            } else {
                self.selected.push(index);
            }
        } else {
            self.selected = vec![index];
        }
    }

    fn state_text(&self, index: usize) -> String {
        let entry = &self.devices[index];
        let info = &entry.info;
        let checked = entry.checked;
        let mut state;
        if checked && !info.is_installed() {
            state = crate::i18n::tr("APO will be installed");
        } else if !checked && info.is_installed() {
            state = crate::i18n::tr("APO will be uninstalled");
        } else if info.is_installed() && info.can_be_upgraded() {
            state = crate::i18n::tr("APO will be upgraded");
        } else if info.is_installed() && info.has_changes() {
            state = crate::i18n::tr("APO installation will be changed");
        } else if info.is_installed() && info.is_enhancements_disabled() {
            state = crate::i18n::tr("Audio enhancements will be enabled");
        } else if info.is_installed() {
            state = crate::i18n::tr("APO is already installed");
        } else if info.is_experimental() {
            state = crate::i18n::tr("APO can be installed (experimental)");
        } else {
            state = crate::i18n::tr("APO can be installed");
        }

        if let Some(voicemeeter) = info.as_any().downcast_ref::<VoicemeeterAPOInfo>() {
            if !voicemeeter.is_voicemeeter_installed() {
                state.push_str(&crate::i18n::tr(", Voicemeeter was uninstalled"));
            } else if info.is_default_device() {
                state.push_str(&crate::i18n::tr(", Default device"));
            }
        } else if info.is_default_device() {
            state.push_str(&crate::i18n::tr(", Default device"));
        }

        if info.is_disabled() {
            state.push_str(&crate::i18n::tr(", Disabled"));
        }
        if info.is_unplugged() {
            state.push_str(&crate::i18n::tr(", Unplugged"));
        }
        state
    }

    fn draw_troubleshooting(&mut self, ui: &mut egui::Ui) {
        let mut enable = false;
        let mut is_input = false;
        let mut has_original_pre = true;
        let mut has_original_post = true;
        let mut state = InstallState::default();

        if self.selected.len() == 1 {
            let index = self.selected[0];
            if let Some(entry) = self.devices.get(index) {
                enable = entry.checked;
                if let Some(device) = entry.info.as_any().downcast_ref::<DeviceAPOInfo>() {
                    is_input = device.is_input();
                    has_original_pre = !device.get_original_apo_pre_mix().is_empty();
                    has_original_post = !device.get_original_apo_post_mix().is_empty();
                    state = device.get_selected_install_state();
                }
            }
        }

        if !enable {
            ui.vertical_centered(|ui| {
                ui.label(crate::i18n::tr("Please select exactly one activated device by clicking on its name"));
            });
            return;
        }

        let mut use_original_pre = state.use_original_apo_pre_mix && has_original_pre;
        let mut use_original_post = state.use_original_apo_post_mix && has_original_post;
        let mut changes: Vec<u8> = Vec::new();

        egui::Grid::new("troubleshooting_grid")
            .num_columns(6)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(crate::i18n::tr("Pre-mix:"));
                });
                if ui
                    .checkbox(&mut state.install_pre_mix, "Install APO")
                    .changed()
                {
                    changes.push(0);
                }
                if ui
                    .add_enabled(
                        has_original_pre && state.install_pre_mix,
                        egui::Checkbox::new(&mut use_original_pre, "Use original APO"),
                    )
                    .on_disabled_hover_text("No original APO detected")
                    .changed()
                {
                    state.use_original_apo_pre_mix = use_original_pre;
                    changes.push(2);
                }
                if ui.checkbox(&mut state.auto_adjust, crate::i18n::tr("Auto:")).changed() {
                    changes.push(6);
                }
                let current_name = install_mode_name(state.install_mode);
                egui::ComboBox::from_id_salt("install_mode")
                    .selected_text(current_name)
                    .show_ui(ui, |ui| {
                        for mode in &self.available_install_modes {
                            if ui
                                .selectable_label(
                                    state.install_mode == *mode,
                                    install_mode_name(*mode),
                                )
                                .clicked()
                                && state.install_mode != *mode
                            {
                                state.install_mode = *mode;
                                changes.push(4);
                            }
                        }
                    });
                ui.label("");
                ui.end_row();

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(crate::i18n::tr("Post-mix:"));
                });
                if ui
                    .add_enabled(
                        !is_input,
                        egui::Checkbox::new(&mut state.install_post_mix, "Install APO"),
                    )
                    .changed()
                {
                    changes.push(1);
                }
                if ui
                    .add_enabled(
                        !is_input && has_original_post && state.install_post_mix,
                        egui::Checkbox::new(&mut use_original_post, "Use original APO"),
                    )
                    .changed()
                {
                    state.use_original_apo_post_mix = use_original_post;
                    changes.push(3);
                }
                if ui
                    .checkbox(
                        &mut state.allow_silent_buffer_modification,
                        "Allow silent buffer modification",
                    )
                    .on_hover_text(crate::i18n::tr("Allows to change silent input buffers to audible output buffers, so that delayed audio can be played.

CAUTION: Some sound card drivers can produce unwanted sounds when this is enabled."),
                    )
                    .changed()
                {
                    changes.push(5);
                }
                ui.label("");
                ui.label("");
                ui.end_row();
            });

        for change in changes {
            for index in self.selected.clone() {
                if let Some(entry) = self.devices.get(index) {
                    if let Some(device) = entry.info.as_any().downcast_ref::<DeviceAPOInfo>() {
                        device.update_selected_install_state(|selected| match change {
                            0 => selected.install_pre_mix = state.install_pre_mix,
                            1 => selected.install_post_mix = state.install_post_mix,
                            2 => {
                                selected.use_original_apo_pre_mix = state.use_original_apo_pre_mix
                            }
                            3 => {
                                selected.use_original_apo_post_mix = state.use_original_apo_post_mix
                            }
                            4 => selected.install_mode = state.install_mode,
                            5 => {
                                selected.allow_silent_buffer_modification =
                                    state.allow_silent_buffer_modification
                            }
                            6 => selected.auto_adjust = state.auto_adjust,
                            _ => {}
                        });
                    }
                }
            }
        }
    }

    fn on_reject(&mut self) {
        if self.has_upgrades() {
            self.messages.push_back(DialogMessage {
                title: crate::i18n::tr("Upgrades available"),
                text: crate::i18n::tr("The APO installation of some devices should be upgraded. Do you really want to cancel?"),
                kind: MessageKind::Warning,
                action: MessageAction::CancelUpgrade,
            });
        } else {
            self.complete(false);
        }
    }

    fn apply(&mut self) {
        let mut device_updated = false;
        let mut errors: Vec<String> = Vec::new();

        for entry in &self.devices {
            let info = &entry.info;
            let checked = entry.checked;
            let result: Result<bool, RegistryException> =
                if checked && !info.is_installed() {
                    info.install().map(|_| true)
                } else if !checked && info.is_installed() {
                    info.uninstall().map(|_| true)
                } else if checked
                    && (info.can_be_upgraded()
                        || info.has_changes()
                        || info.is_enhancements_disabled())
                {
                    info.reinstall().map(|_| true)
                } else {
                    Ok(false)
                };

            match result {
                Ok(performed) => {
                    if performed && info.as_any().downcast_ref::<DeviceAPOInfo>().is_some() {
                        device_updated = true;
                    }
                }
                Err(error) => errors.push(error.to_string()),
            }
        }

        let _ = VoicemeeterAPOInfo::ensure_voicemeeter_client_running();

        if errors.is_empty() {
            self.complete(device_updated);
        } else {
            for error in errors {
                self.messages.push_back(DialogMessage {
                    title: crate::i18n::tr("Error while accessing the registry"),
                    text: error,
                    kind: MessageKind::Critical,
                    action: MessageAction::None,
                });
            }
            self.pending_apply_complete = Some(device_updated);
        }
    }

    fn complete(&mut self, device_updated: bool) {
        let install_mode = self.args.iter().any(|argument| argument == "/i");
        let need_test = install_mode || device_updated || self.ask_for_reboot;

        if need_test && !self.test_ran {
            self.test_dialog = Some(DeviceTestDialog::new());
            return;
        }

        if install_mode {
            self.messages.push_back(DialogMessage {
                title: crate::i18n::tr("Info"),
                text: crate::i18n::tr("This dialog can be reopened anytime by launching Device Selector from the start menu."),
                kind: MessageKind::Info,
                action: MessageAction::InstallDone {
                    return_code: if self.test_result == -1 { 1 } else { 0 },
                },
            });
            self.active_message = self.messages.pop_front();
        } else if self.test_result == -1 {
            self.messages.push_back(DialogMessage {
                title: crate::i18n::tr("Reboot"),
                text: crate::i18n::tr("To apply the changes, Windows should be rebooted. Reboot now?"),
                kind: MessageKind::Question,
                action: MessageAction::Reboot,
            });
            self.active_message = self.messages.pop_front();
        } else {
            self.exit(0);
        }
    }

    fn handle_message_action(&mut self, action: &MessageAction, answer: bool) {
        match action {
            MessageAction::None => {}
            MessageAction::CancelUpgrade => {
                if answer {
                    self.complete(false);
                }
            }
            MessageAction::Reboot => {
                if answer {
                    initiate_shutdown();
                }
                self.exit(0);
            }
            MessageAction::InstallDone { return_code } => {
                self.exit(*return_code);
            }
        }
    }

    fn exit(&mut self, return_code: i32) {
        self.exit_result.store(return_code, Ordering::SeqCst);
        self.closing = true;
        // The viewport close command is issued in `update`.
    }

    fn is_changed(&self) -> bool {
        self.devices.iter().any(|entry| {
            let installed = entry.info.is_installed();
            entry.checked != installed
                || (entry.checked
                    && installed
                    && (entry.info.can_be_upgraded()
                        || entry.info.has_changes()
                        || entry.info.is_enhancements_disabled()))
        })
    }

    fn has_upgrades(&self) -> bool {
        self.devices.iter().any(|entry| {
            entry.checked
                && entry.info.is_installed()
                && (entry.info.can_be_upgraded() || entry.info.is_enhancements_disabled())
        })
    }
}

fn install_mode_name(mode: InstallMode) -> &'static str {
    match mode {
        InstallMode::InstallLfxGfx => "Install as LFX/GFX",
        InstallMode::InstallSfxMfx => "Install as SFX/MFX",
        InstallMode::InstallSfxEfx => "Install as SFX/EFX",
    }
}

fn initiate_shutdown() {
    unsafe {
        let mut token = HANDLE::default();
        if OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .is_ok()
        {
            let mut luid = LUID::default();
            if LookupPrivilegeValueW(PCWSTR::null(), SE_SHUTDOWN_NAME, &mut luid).is_ok() {
                let privileges = TOKEN_PRIVILEGES {
                    PrivilegeCount: 1,
                    Privileges: [windows::Win32::Security::LUID_AND_ATTRIBUTES {
                        Luid: luid,
                        Attributes: SE_PRIVILEGE_ENABLED,
                    }],
                };
                if AdjustTokenPrivileges(token, false, Some(&privileges), 0, None, None).is_ok() {
                    let _ = InitiateShutdownW(
                        PCWSTR::null(),
                        PCWSTR::null(),
                        0,
                        SHUTDOWN_RESTART | SHUTDOWN_GRACE_OVERRIDE,
                        SHTDN_REASON_MAJOR_APPLICATION | SHTDN_REASON_MINOR_MAINTENANCE,
                    );
                }
            }
            let _ = CloseHandle(token);
        }
    }
}
