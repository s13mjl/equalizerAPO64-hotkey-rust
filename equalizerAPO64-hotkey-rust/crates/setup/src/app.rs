//! egui implementation of the NSIS Modern UI wizard pages.
//!
//! Installer: Welcome -> License -> Directory -> Start Menu -> Components ->
//! Installing -> Finish.
//! Uninstaller: Welcome -> Confirm -> Components -> Uninstalling -> Finish.

use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, Receiver};
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use eframe::egui;
use egui::{Align, Layout, RichText};
use regex::Regex;

use crate::files;
use crate::install::{self, InstallMessage, InstallOptions, InstallResult};
use crate::lang::Language;
use crate::registry::{RegistryStore, WinRegistry, HKLM_REGPATH};
use crate::uninstall::{self, UninstallMessage, UninstallOptions, UninstallResult};
use crate::util;
use crate::version::{display_version, full_name};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Page {
    Welcome,
    License,
    Directory,
    StartMenu,
    Components,
    Progress,
    Finish,
    Error,
    UninstallWelcome,
    UninstallConfirm,
    UninstallComponents,
    UninstallProgress,
    UninstallFinish,
}

impl Page {
    fn is_progress(self) -> bool {
        matches!(self, Page::Progress | Page::UninstallProgress)
    }

    fn is_finish(self) -> bool {
        matches!(self, Page::Finish | Page::UninstallFinish)
    }
}

struct InstallerData {
    language: Language,
    install_dir: String,
    available_space: u64,
    start_menu_folder: String,
    old_start_menu_folder: String,
    create_shortcuts: bool,
    check_updates: bool,
    license_accepted: bool,
    validation_error: Option<String>,
    staging: PathBuf,
    required_space: u64,
    current_exe: PathBuf,
}

impl InstallerData {
    fn load() -> Self {
        let language = Language::from_system();
        let registry = WinRegistry::new();
        let previous_install = registry
            .read_string(HKLM_REGPATH, "InstallPath")
            .unwrap_or_default();
        let install_dir = if previous_install.trim().is_empty() {
            install::default_install_dir()
        } else {
            PathBuf::from(previous_install)
        };

        let old_start_menu_folder = registry
            .read_string(HKLM_REGPATH, "Start Menu Folder")
            .unwrap_or_default();
        let start_menu_folder = version_replaced_folder(&old_start_menu_folder, &display_version());

        let staging = files::find_staging(&util::current_exe_dir());
        let required_space = files::payload_size(&staging);
        let available_space = util::free_space(&install_dir);
        let current_exe = std::env::current_exe().unwrap_or_default();

        Self {
            language,
            install_dir: install_dir.to_string_lossy().to_string(),
            available_space,
            start_menu_folder,
            old_start_menu_folder,
            create_shortcuts: true,
            check_updates: true,
            license_accepted: false,
            validation_error: None,
            staging,
            required_space,
            current_exe,
        }
    }
}

struct UninstallerData {
    language: Language,
    install_dir: PathBuf,
    start_menu_folder: String,
    remove_configs: bool,
    from_temp: bool,
}

enum Mode {
    Install(InstallerData),
    Uninstall(UninstallerData),
}

enum ReceiverKind {
    Install(Receiver<InstallMessage>),
    Uninstall(Receiver<UninstallMessage>),
}

enum WorkerEvent {
    Progress { fraction: f32, text: String },
    Log(String),
    InstallDone(InstallResult),
    UninstallDone(UninstallResult),
}

enum Outcome {
    Install(InstallResult),
    Uninstall(UninstallResult),
}

impl Outcome {
    fn reboot(&self) -> bool {
        match self {
            Outcome::Install(result) => result.reboot,
            Outcome::Uninstall(result) => result.reboot,
        }
    }

    fn error(&self) -> Option<&str> {
        match self {
            Outcome::Install(result) => result.error.as_deref(),
            Outcome::Uninstall(result) => result.error.as_deref(),
        }
    }
}

pub struct SetupApp {
    mode: Mode,
    page: Page,
    log: Vec<String>,
    progress: f32,
    status: String,
    show_details: bool,
    receiver: Option<ReceiverKind>,
    cancel: Arc<AtomicBool>,
    outcome: Option<Outcome>,
    reboot_now: bool,
    allow_close: bool,
    license: String,
}

impl SetupApp {
    fn new(mode: Mode) -> Self {
        let page = match mode {
            Mode::Install(_) => Page::Welcome,
            Mode::Uninstall(_) => Page::UninstallWelcome,
        };
        let license = load_license();
        Self {
            mode,
            page,
            log: Vec::new(),
            progress: 0.0,
            status: String::new(),
            show_details: false,
            receiver: None,
            cancel: Arc::new(AtomicBool::new(false)),
            outcome: None,
            reboot_now: false,
            allow_close: false,
            license,
        }
    }

    fn language(&self) -> Language {
        match &self.mode {
            Mode::Install(data) => data.language,
            Mode::Uninstall(data) => data.language,
        }
    }

    fn is_installer(&self) -> bool {
        matches!(self.mode, Mode::Install(_))
    }

    fn poll_worker(&mut self, ctx: &egui::Context) {
        let mut events: Vec<WorkerEvent> = Vec::new();
        let mut disconnected = false;
        match &self.receiver {
            Some(ReceiverKind::Install(receiver)) => loop {
                match receiver.try_recv() {
                    Ok(InstallMessage::Progress { fraction, text }) => {
                        events.push(WorkerEvent::Progress { fraction, text });
                    }
                    Ok(InstallMessage::Log(line)) => events.push(WorkerEvent::Log(line)),
                    Ok(InstallMessage::Finished(result)) => {
                        events.push(WorkerEvent::InstallDone(result));
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            },
            Some(ReceiverKind::Uninstall(receiver)) => loop {
                match receiver.try_recv() {
                    Ok(UninstallMessage::Progress { fraction, text }) => {
                        events.push(WorkerEvent::Progress { fraction, text });
                    }
                    Ok(UninstallMessage::Log(line)) => events.push(WorkerEvent::Log(line)),
                    Ok(UninstallMessage::Finished(result)) => {
                        events.push(WorkerEvent::UninstallDone(result));
                    }
                    Err(mpsc::TryRecvError::Empty) => break,
                    Err(mpsc::TryRecvError::Disconnected) => {
                        disconnected = true;
                        break;
                    }
                }
            },
            None => {}
        }

        let mut finished = false;
        for event in events {
            match event {
                WorkerEvent::Progress { fraction, text } => {
                    self.progress = fraction;
                    self.status = text;
                }
                WorkerEvent::Log(line) => self.log.push(line),
                WorkerEvent::InstallDone(result) => {
                    self.finish_install(result);
                    finished = true;
                }
                WorkerEvent::UninstallDone(result) => {
                    self.finish_uninstall(result);
                    finished = true;
                }
            }
        }
        if finished {
            self.receiver = None;
            ctx.request_repaint();
            return;
        }

        if disconnected && self.page.is_progress() {
            self.receiver = None;
            self.page = Page::Error;
            let message = if self.is_installer() {
                self.language().install_failed().to_string()
            } else {
                self.language().uninstall_failed().to_string()
            };
            self.log
                .push("The worker thread terminated unexpectedly.".to_string());
            self.outcome = Some(if self.is_installer() {
                Outcome::Install(InstallResult::failed(message))
            } else {
                Outcome::Uninstall(UninstallResult::failed(message))
            });
            ctx.request_repaint();
        }
    }

    fn finish_install(&mut self, result: InstallResult) {
        if let Some(error) = &result.error {
            self.log.push(error.clone());
        }
        if result.cancelled {
            self.status = self.language().installation_aborted().to_string();
        }
        self.page = if result.success {
            Page::Finish
        } else {
            Page::Error
        };
        self.progress = if result.success { 1.0 } else { self.progress };
        self.reboot_now = false;
        self.outcome = Some(Outcome::Install(result));
    }

    fn finish_uninstall(&mut self, result: UninstallResult) {
        if let Some(error) = &result.error {
            self.log.push(error.clone());
        }
        if result.cancelled {
            self.status = self.language().uninstallation_aborted().to_string();
        }
        self.page = if result.success {
            Page::UninstallFinish
        } else {
            Page::Error
        };
        self.progress = if result.success { 1.0 } else { self.progress };
        self.reboot_now = false;
        self.outcome = Some(Outcome::Uninstall(result));
    }

    fn start_install(&mut self) {
        let options = if let Mode::Install(data) = &self.mode {
            let install_dir = PathBuf::from(data.install_dir.trim());
            let start_menu_folder = if data.create_shortcuts {
                data.start_menu_folder.trim().to_string()
            } else {
                String::new()
            };
            Some(InstallOptions {
                install_dir,
                start_menu_folder,
                old_start_menu_folder: data.old_start_menu_folder.clone(),
                check_updates: data.check_updates,
                staging: data.staging.clone(),
                current_exe: data.current_exe.clone(),
                language: data.language,
            })
        } else {
            None
        };
        let Some(options) = options else {
            return;
        };

        self.cancel.store(false, Ordering::Relaxed);
        self.log.clear();
        self.progress = 0.0;
        self.status = self.language().preparing().to_string();
        self.outcome = None;
        self.show_details = false;
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::clone(&self.cancel);
        let spawned = std::thread::Builder::new()
            .name("eapo-setup-install".to_string())
            .spawn(move || install::run_install(options, sender, cancel));
        match spawned {
            Ok(_) => {
                self.receiver = Some(ReceiverKind::Install(receiver));
                self.page = Page::Progress;
            }
            Err(error) => {
                let message = format!("Could not start installer worker: {error}");
                self.log.push(message.clone());
                self.outcome = Some(Outcome::Install(InstallResult::failed(message)));
                self.page = Page::Error;
            }
        }
    }

    fn start_uninstall(&mut self) {
        let options = if let Mode::Uninstall(data) = &self.mode {
            Some(UninstallOptions {
                install_dir: data.install_dir.clone(),
                start_menu_folder: data.start_menu_folder.clone(),
                remove_configs: data.remove_configs,
                language: data.language,
                from_temp: data.from_temp,
            })
        } else {
            None
        };
        let Some(options) = options else {
            return;
        };

        self.cancel.store(false, Ordering::Relaxed);
        self.log.clear();
        self.progress = 0.0;
        self.status = self.language().preparing().to_string();
        self.outcome = None;
        self.show_details = false;
        let (sender, receiver) = mpsc::channel();
        let cancel = Arc::clone(&self.cancel);
        let spawned = std::thread::Builder::new()
            .name("eapo-setup-uninstall".to_string())
            .spawn(move || uninstall::run_uninstall(options, sender, cancel));
        match spawned {
            Ok(_) => {
                self.receiver = Some(ReceiverKind::Uninstall(receiver));
                self.page = Page::UninstallProgress;
            }
            Err(error) => {
                let message = format!("Could not start uninstaller worker: {error}");
                self.log.push(message.clone());
                self.outcome = Some(Outcome::Uninstall(UninstallResult::failed(message)));
                self.page = Page::Error;
            }
        }
    }

    fn close_app(&mut self, ctx: &egui::Context) {
        self.allow_close = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn request_abort(&mut self, ctx: &egui::Context) {
        if self.allow_close {
            return;
        }
        let language = self.language();
        let text = if self.is_installer() {
            language.abort_install_text()
        } else {
            language.abort_uninstall_text()
        };
        if !util::confirm_yes_no(&text, language.abort_title()) {
            return;
        }
        if self.page.is_progress() {
            self.cancel.store(true, Ordering::Relaxed);
            self.status = language.cancelling().to_string();
            self.log.push("Cancellation requested.".to_string());
        } else {
            self.close_app(ctx);
        }
    }

    fn page_title(&self) -> String {
        let language = self.language();
        match self.page {
            Page::Welcome => language.welcome_title(),
            Page::License => language.license_title().to_string(),
            Page::Directory => language.directory_title().to_string(),
            Page::StartMenu => language.start_menu_title().to_string(),
            Page::Components => language.components_title().to_string(),
            Page::Progress => language.installing_title().to_string(),
            Page::Finish => language.finish_install_title(),
            Page::Error => language.error_title().to_string(),
            Page::UninstallWelcome => language.uninstall_welcome_title(),
            Page::UninstallConfirm => language.confirm_title().to_string(),
            Page::UninstallComponents => language.components_title().to_string(),
            Page::UninstallProgress => language.uninstalling_title().to_string(),
            Page::UninstallFinish => language.finish_uninstall_title(),
        }
    }

    fn primary_visible(&self) -> bool {
        !self.page.is_progress()
    }

    fn primary_label(&self) -> String {
        let language = self.language();
        match self.page {
            Page::Components => language.install().to_string(),
            Page::UninstallConfirm | Page::UninstallComponents => language.uninstall().to_string(),
            Page::Finish | Page::UninstallFinish => language.finish().to_string(),
            Page::Error => language.close().to_string(),
            _ => language.next().to_string(),
        }
    }

    fn primary_enabled(&self) -> bool {
        match self.page {
            Page::License => match &self.mode {
                Mode::Install(data) => data.license_accepted,
                _ => true,
            },
            _ => true,
        }
    }

    fn can_go_back(&self) -> bool {
        matches!(
            self.page,
            Page::License
                | Page::Directory
                | Page::StartMenu
                | Page::Components
                | Page::UninstallConfirm
                | Page::UninstallComponents
        )
    }

    fn go_back(&mut self) {
        self.page = match self.page {
            Page::License => Page::Welcome,
            Page::Directory => Page::License,
            Page::StartMenu => Page::Directory,
            Page::Components => Page::StartMenu,
            Page::UninstallConfirm => Page::UninstallWelcome,
            Page::UninstallComponents => Page::UninstallConfirm,
            other => other,
        };
    }

    fn primary_clicked(&mut self, ctx: &egui::Context) {
        match self.page {
            Page::Welcome => self.page = Page::License,
            Page::License => self.page = Page::Directory,
            Page::Directory => {
                let valid = if let Mode::Install(data) = &mut self.mode {
                    let value = data.install_dir.trim().to_string();
                    let path = Path::new(&value);
                    if value.is_empty() || !path.is_absolute() {
                        data.validation_error = Some(data.language.invalid_directory().to_string());
                        false
                    } else {
                        let available = util::free_space(path);
                        data.install_dir = value;
                        data.validation_error = None;
                        data.available_space = available;
                        true
                    }
                } else {
                    false
                };
                if valid {
                    self.page = Page::StartMenu;
                }
            }
            Page::StartMenu => self.page = Page::Components,
            Page::Components => self.start_install(),
            Page::Finish | Page::UninstallFinish => {
                if self.reboot_now {
                    if let Err(error) = util::reboot() {
                        util::message_error(&error, self.language().error_title());
                    }
                }
                self.close_app(ctx);
            }
            Page::Error => self.close_app(ctx),
            Page::UninstallWelcome => self.page = Page::UninstallConfirm,
            Page::UninstallConfirm => self.page = Page::UninstallComponents,
            Page::UninstallComponents => self.start_uninstall(),
            Page::Progress | Page::UninstallProgress => {}
        }
    }

    fn header(&mut self, ui: &mut egui::Ui) {
        ui.add_space(4.0);
        ui.label(RichText::new(full_name()).size(12.0).weak());
        ui.label(RichText::new(self.page_title()).size(21.0).strong());
        ui.add_space(4.0);
    }

    fn content(&mut self, ui: &mut egui::Ui) {
        match self.page {
            Page::Welcome | Page::UninstallWelcome => self.page_welcome(ui),
            Page::License => self.page_license(ui),
            Page::Directory => self.page_directory(ui),
            Page::StartMenu => self.page_start_menu(ui),
            Page::Components => self.page_components(ui),
            Page::Progress | Page::UninstallProgress => self.page_progress(ui),
            Page::Finish | Page::UninstallFinish => self.page_finish(ui),
            Page::Error => self.page_error(ui),
            Page::UninstallConfirm => self.page_confirm(ui),
            Page::UninstallComponents => self.page_uninstall_components(ui),
        }
    }

    fn page_welcome(&mut self, ui: &mut egui::Ui) {
        let language = self.language();
        let text = if self.is_installer() {
            language.welcome_text()
        } else {
            language.uninstall_welcome_text()
        };
        ui.add_space(14.0);
        ui.add(egui::Label::new(RichText::new(text).size(14.0)).wrap());
        ui.add_space(16.0);
        if !self.is_installer() {
            ui.label(
                RichText::new(format!(
                    "{}: {}",
                    language.uninstall_window_title(),
                    install_dir_text(&self.mode)
                ))
                .weak(),
            );
        }
    }

    fn page_license(&mut self, ui: &mut egui::Ui) {
        let language = self.language();
        if self.license.is_empty() {
            ui.colored_label(egui::Color32::RED, language.license_missing());
            return;
        }
        ui.add(egui::Label::new(language.license_text()).wrap());
        ui.add_space(8.0);
        egui::ScrollArea::vertical()
            .max_height(300.0)
            .auto_shrink([false, true])
            .show(ui, |ui| {
                ui.add(
                    egui::Label::new(RichText::new(&self.license).monospace().size(11.0)).wrap(),
                );
            });
        ui.add_space(8.0);
        if let Mode::Install(data) = &mut self.mode {
            ui.checkbox(&mut data.license_accepted, language.license_accept());
        }
    }

    fn page_directory(&mut self, ui: &mut egui::Ui) {
        let mut browse_clicked = false;

        if let Mode::Install(data) = &mut self.mode {
            ui.add(egui::Label::new(data.language.directory_text()).wrap());
            ui.add_space(12.0);
            ui.label(data.language.destination_folder());
            ui.horizontal(|ui| {
                let browse_width = 110.0;
                let width = (ui.available_width() - browse_width - 8.0).max(120.0);
                let response = ui.add_sized(
                    [width, 24.0],
                    egui::TextEdit::singleline(&mut data.install_dir),
                );
                if response.changed() {
                    data.validation_error = None;
                }
                if ui.button(data.language.browse()).clicked() {
                    browse_clicked = true;
                }
            });
            ui.add_space(10.0);
            ui.label(format!(
                "{}: {}",
                data.language.space_required(),
                format_size(data.required_space)
            ));
            ui.label(format!(
                "{}: {}",
                data.language.space_available(),
                format_size(data.available_space)
            ));
            if let Some(validation) = &data.validation_error {
                ui.add_space(6.0);
                ui.colored_label(egui::Color32::RED, validation);
            }
        }

        if browse_clicked {
            if let Mode::Install(data) = &mut self.mode {
                if let Some(folder) = util::pick_folder(
                    data.language.directory_title(),
                    Path::new(&data.install_dir),
                ) {
                    data.install_dir = folder.to_string_lossy().to_string();
                    data.available_space = util::free_space(&folder);
                    data.validation_error = None;
                }
            }
        }
    }

    fn page_start_menu(&mut self, ui: &mut egui::Ui) {
        if let Mode::Install(data) = &mut self.mode {
            ui.add(egui::Label::new(data.language.start_menu_text()).wrap());
            ui.add_space(12.0);
            ui.checkbox(&mut data.create_shortcuts, data.language.no_shortcuts());
            ui.add_space(8.0);
            let width = ui.available_width().min(420.0);
            ui.add_enabled_ui(data.create_shortcuts, |ui| {
                ui.label(data.language.start_menu_folder_label());
                ui.add_sized(
                    [width, 24.0],
                    egui::TextEdit::singleline(&mut data.start_menu_folder),
                );
            });
        }
    }

    fn page_components(&mut self, ui: &mut egui::Ui) {
        if let Mode::Install(data) = &mut self.mode {
            ui.add(egui::Label::new(data.language.components_text()).wrap());
            ui.add_space(14.0);
            ui.checkbox(&mut data.check_updates, data.language.check_for_updates());
        }
    }

    fn page_uninstall_components(&mut self, ui: &mut egui::Ui) {
        if let Mode::Uninstall(data) = &mut self.mode {
            ui.add(egui::Label::new(data.language.uninstall_components_text()).wrap());
            ui.add_space(14.0);
            ui.checkbox(
                &mut data.remove_configs,
                data.language.remove_configs_and_backups(),
            );
        }
    }

    fn page_confirm(&mut self, ui: &mut egui::Ui) {
        let language = self.language();
        ui.add_space(12.0);
        ui.add(egui::Label::new(RichText::new(language.confirm_text()).size(14.0)).wrap());
        ui.add_space(10.0);
        ui.add(egui::Label::new(language.confirm_hint()).wrap());
    }

    fn page_progress(&mut self, ui: &mut egui::Ui) {
        let language = self.language();
        let text = if self.page == Page::Progress {
            language.installing_text()
        } else {
            language.uninstalling_text()
        };
        ui.add(egui::Label::new(text).wrap());
        ui.add_space(12.0);
        ui.add(
            egui::ProgressBar::new(self.progress)
                .show_percentage()
                .animate(true),
        );
        ui.add_space(8.0);
        ui.label(&self.status);
        ui.add_space(10.0);
        let toggle_label = if self.show_details {
            language.details_hide()
        } else {
            language.details()
        };
        if ui.button(toggle_label).clicked() {
            self.show_details = !self.show_details;
        }
        if self.show_details {
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .max_height(180.0)
                .auto_shrink([false, false])
                .stick_to_bottom(true)
                .show(ui, |ui| {
                    for line in &self.log {
                        ui.label(RichText::new(line).monospace().size(11.0));
                    }
                });
        }
    }

    fn page_finish(&mut self, ui: &mut egui::Ui) {
        let language = self.language();
        let text = if self.page == Page::Finish {
            language.finish_install_text()
        } else {
            language.finish_uninstall_text()
        };
        ui.add_space(16.0);
        ui.add(egui::Label::new(RichText::new(text).size(14.0)).wrap());
        ui.add_space(8.0);
        ui.add(egui::Label::new(language.finish_hint()).wrap());
        let reboot = self.outcome.as_ref().map(Outcome::reboot).unwrap_or(false);
        if reboot {
            ui.add_space(14.0);
            ui.checkbox(&mut self.reboot_now, language.reboot_now());
        }
    }

    fn page_error(&mut self, ui: &mut egui::Ui) {
        let language = self.language();
        let message = self
            .outcome
            .as_ref()
            .and_then(Outcome::error)
            .map(str::to_string)
            .unwrap_or_else(|| {
                if self.is_installer() {
                    language.install_failed().to_string()
                } else {
                    language.uninstall_failed().to_string()
                }
            });
        ui.add_space(10.0);
        ui.colored_label(egui::Color32::RED, RichText::new(message).size(14.0));
        ui.add_space(10.0);
        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .max_height(260.0)
            .stick_to_bottom(true)
            .show(ui, |ui| {
                for line in &self.log {
                    ui.label(RichText::new(line).monospace().size(11.0));
                }
            });
    }

    fn footer(&mut self, ctx: &egui::Context, ui: &mut egui::Ui) {
        let language = self.language();
        ui.add_space(8.0);
        ui.horizontal(|ui| {
            if ui
                .add_enabled(
                    self.can_go_back(),
                    egui::Button::new(language.back()).min_size(egui::vec2(90.0, 28.0)),
                )
                .clicked()
            {
                self.go_back();
            }
            ui.with_layout(Layout::right_to_left(Align::Center), |ui| {
                if self.primary_visible() {
                    if ui
                        .add_enabled(
                            self.primary_enabled(),
                            egui::Button::new(self.primary_label())
                                .min_size(egui::vec2(100.0, 28.0)),
                        )
                        .clicked()
                    {
                        self.primary_clicked(ctx);
                    }
                }
                if !self.page.is_finish() && self.page != Page::Error {
                    if ui
                        .add(egui::Button::new(language.cancel()).min_size(egui::vec2(90.0, 28.0)))
                        .clicked()
                    {
                        self.request_abort(ctx);
                    }
                }
            });
        });
    }
}

impl eframe::App for SetupApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if self.receiver.is_some() {
            self.poll_worker(ctx);
            if self.page.is_progress() {
                ctx.request_repaint_after(Duration::from_millis(80));
            }
        }

        if ctx.input(|input| input.viewport().close_requested()) && !self.allow_close {
            if self.page.is_finish() || self.page == Page::Error {
                self.close_app(ctx);
            } else {
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
                self.request_abort(ctx);
            }
            return;
        }

        egui::TopBottomPanel::bottom("setup_footer")
            .exact_height(52.0)
            .show(ctx, |ui| self.footer(ctx, ui));
        egui::CentralPanel::default().show(ctx, |ui| {
            self.header(ui);
            ui.separator();
            ui.add_space(6.0);
            self.content(ui);
        });
    }

    fn clear_color(&self, _visuals: &egui::Visuals) -> [f32; 4] {
        [0.96, 0.96, 0.96, 1.0]
    }
}

pub fn run_installer() -> eframe::Result<()> {
    let data = InstallerData::load();
    let title = data.language.window_title();
    let app = SetupApp::new(Mode::Install(data));
    run(title, app)
}

/// NSIS `/S` silent installation.  Uses the same defaults as the wizard's
/// first page (previous InstallPath or `%ProgramFiles%\EqualizerAPO`), or the
/// `/D=` override, and performs the selected default sections.
pub fn run_silent_install(install_dir_override: Option<PathBuf>) -> i32 {
    let data = InstallerData::load();
    let install_dir = install_dir_override
        .unwrap_or_else(|| PathBuf::from(&data.install_dir));
    let options = InstallOptions {
        install_dir,
        start_menu_folder: data.start_menu_folder.trim().to_string(),
        old_start_menu_folder: data.old_start_menu_folder.clone(),
        check_updates: data.check_updates,
        staging: data.staging.clone(),
        current_exe: data.current_exe.clone(),
        language: data.language,
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    install::run_install(options, sender, Arc::new(AtomicBool::new(false)));
    let success = receiver
        .into_iter()
        .find_map(|message| match message {
            InstallMessage::Finished(result) => Some(result.success),
            _ => None,
        })
        .unwrap_or(false);
    if success { 0 } else { 1 }
}

/// NSIS `/S` silent uninstallation.
pub fn run_silent_uninstall(
    install_dir: PathBuf,
    start_menu_folder: String,
    from_temp: bool,
) -> i32 {
    let options = UninstallOptions {
        install_dir,
        start_menu_folder,
        remove_configs: false,
        language: Language::from_system(),
        from_temp,
    };
    let (sender, receiver) = std::sync::mpsc::channel();
    uninstall::run_uninstall(options, sender, Arc::new(AtomicBool::new(false)));
    let success = receiver
        .into_iter()
        .find_map(|message| match message {
            UninstallMessage::Finished(result) => Some(result.success),
            _ => None,
        })
        .unwrap_or(false);
    if success { 0 } else { 1 }
}

pub fn run_uninstaller(
    install_dir: PathBuf,
    start_menu_folder: String,
    from_temp: bool,
) -> eframe::Result<()> {
    let language = Language::from_system();
    let title = language.uninstall_window_title();
    let app = SetupApp::new(Mode::Uninstall(UninstallerData {
        language,
        install_dir,
        start_menu_folder,
        remove_configs: false,
        from_temp,
    }));
    run(title, app)
}

fn run(title: String, app: SetupApp) -> eframe::Result<()> {
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title.clone())
            .with_inner_size([760.0, 560.0])
            .with_min_inner_size([680.0, 500.0])
            .with_resizable(false),
        ..Default::default()
    };
    eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            eapo_common::egui_fonts::install_cjk_font(&cc.egui_ctx);
            cc.egui_ctx.set_visuals(egui::Visuals::light());
            Ok(Box::new(app))
        }),
    )
}

fn load_license() -> String {
    let adjacent = util::current_exe_dir().join("License.txt");
    if let Ok(text) = std::fs::read_to_string(&adjacent) {
        if !text.is_empty() {
            return text;
        }
    }
    files::LICENSE_TEXT.to_string()
}

/// Port of the start-menu version replacement in `.onInit`.
pub fn version_replaced_folder(existing: &str, version: &str) -> String {
    let existing = existing.trim();
    if existing.is_empty() {
        return format!("Equalizer APO {version}");
    }
    static PATTERN: OnceLock<Regex> = OnceLock::new();
    let pattern = PATTERN.get_or_init(|| {
        Regex::new(r"Equalizer APO [0-9]+\.[0-9]+(?:\.[0-9]+)?").expect("valid regex")
    });
    pattern
        .replace_all(existing, format!("Equalizer APO {version}"))
        .into_owned()
}

fn format_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = 1024.0 * 1024.0;
    const GB: f64 = 1024.0 * 1024.0 * 1024.0;
    let value = bytes as f64;
    if value >= GB {
        format!("{:.1} GB", value / GB)
    } else if value >= MB {
        format!("{:.1} MB", value / MB)
    } else if value >= KB {
        format!("{:.1} KB", value / KB)
    } else {
        format!("{bytes} bytes")
    }
}

fn install_dir_text(mode: &Mode) -> String {
    match mode {
        Mode::Install(data) => data.install_dir.clone(),
        Mode::Uninstall(data) => data.install_dir.to_string_lossy().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_is_replaced_in_start_menu_folder_name() {
        assert_eq!(
            version_replaced_folder("Equalizer APO 1.4.1", "1.4.2"),
            "Equalizer APO 1.4.2"
        );
        assert_eq!(
            version_replaced_folder("Equalizer APO 1.4.1 (custom)", "1.4.2"),
            "Equalizer APO 1.4.2 (custom)"
        );
        assert_eq!(
            version_replaced_folder("Equalizer APO 2.10.3", "1.4.2"),
            "Equalizer APO 1.4.2"
        );
        assert_eq!(
            version_replaced_folder("My Equalizer APO shortcuts", "1.4.2"),
            "My Equalizer APO shortcuts"
        );
        assert_eq!(version_replaced_folder("", "1.4.2"), "Equalizer APO 1.4.2");
        assert_eq!(
            version_replaced_folder("Equalizer APO 1.4", "1.4.2"),
            "Equalizer APO 1.4.2"
        );
    }

    #[test]
    fn size_formatting_is_human_readable() {
        assert_eq!(format_size(512), "512 bytes");
        assert_eq!(format_size(2048), "2.0 KB");
        assert_eq!(format_size(3 * 1024 * 1024), "3.0 MB");
        assert_eq!(format_size(2 * 1024 * 1024 * 1024), "2.0 GB");
    }
}
