//! Port of `MainWindow` (menus, toolbar, tabs, device/channel selection,
//! analysis dock, preferences, close behavior).

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use eapo_apo_info::{AbstractAPOInfo, DeviceAPOInfo};
use eapo_common::channel_helper;
use egui::{Color32, Vec2};

use crate::analysis::{AnalysisRequest, AnalysisResult, AnalysisThread, DeviceSnapshot};
use crate::filter_table::{self, file_title, FilterTab, TabUiReport, TableClipboard};
use crate::guis::GuiServices;
use crate::hotkey_dialog::HotkeyDialog;
use crate::model::FilterContext;
use crate::plot::{self, PlotState};
use crate::settings::{self, GlobalPreferences};

pub const VERSION: &str = "1.4.2";

const KSAUDIO_SPEAKER_SURROUND: u32 =
    channel_helper::SPEAKER_FRONT_LEFT | channel_helper::SPEAKER_FRONT_RIGHT
        | channel_helper::SPEAKER_FRONT_CENTER | channel_helper::SPEAKER_LOW_FREQUENCY;
const KSAUDIO_SPEAKER_5POINT1: u32 = KSAUDIO_SPEAKER_SURROUND
    | channel_helper::SPEAKER_BACK_LEFT
    | channel_helper::SPEAKER_BACK_RIGHT;

#[derive(Clone)]
struct ChannelInfo {
    mask: u32,
    name: &'static str,
}

fn channel_infos() -> Vec<ChannelInfo> {
    vec![
        ChannelInfo { mask: channel_helper::KSAUDIO_SPEAKER_MONO, name: "Mono" },
        ChannelInfo { mask: channel_helper::KSAUDIO_SPEAKER_STEREO, name: "Stereo" },
        ChannelInfo { mask: channel_helper::KSAUDIO_SPEAKER_QUAD, name: "Quadraphonic" },
        ChannelInfo { mask: KSAUDIO_SPEAKER_SURROUND, name: "Surround" },
        ChannelInfo { mask: KSAUDIO_SPEAKER_5POINT1, name: "5.1" },
        ChannelInfo { mask: channel_helper::KSAUDIO_SPEAKER_5POINT1_SURROUND, name: "5.1 Surround" },
        ChannelInfo { mask: channel_helper::KSAUDIO_SPEAKER_7POINT1_SURROUND, name: "7.1 Surround" },
    ]
}

fn device_label(dev: &Arc<dyn AbstractAPOInfo>) -> String {
    format!("{} - {}", dev.get_connection_name(), dev.get_device_name())
}

fn shell_open(path: &str, run_as: bool) {
    use windows::Win32::UI::Shell::ShellExecuteW;
    use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
    let file: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let op: Vec<u16> = (if run_as { "runas" } else { "open" })
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect();
    unsafe {
        let result = ShellExecuteW(
            None,
            windows::core::PCWSTR(op.as_ptr()),
            windows::core::PCWSTR(file.as_ptr()),
            None,
            None,
            SW_SHOWNORMAL,
        );
        if (result.0 as isize) <= 32 && !run_as {
            shell_open(path, true);
        }
    }
}

fn launch_next_to_editor(name: &str) {
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            let path = dir.join(name);
            shell_open(&path.to_string_lossy(), false);
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PromptTarget {
    App,
    Tab,
}

struct ClosePrompt {
    target: PromptTarget,
    tab_index: usize,
}

struct InfoDialog {
    title: String,
    text: String,
}

struct CheckDialog {
    title: String,
    text: String,
}

#[derive(Clone, Copy)]
enum RestartAction {
    Language(Option<&'static str>),
    ResetGlobal,
    ResetFileSpecific,
}

struct RestartPrompt {
    action: RestartAction,
}

pub struct EditorApp {
    config_dir: PathBuf,
    output_devices: Vec<Arc<dyn AbstractAPOInfo>>,
    input_devices: Vec<Arc<dyn AbstractAPOInfo>>,
    all_devices: Vec<Arc<dyn AbstractAPOInfo>>,
    default_output_device: Option<Arc<dyn AbstractAPOInfo>>,
    selected_device: Option<Arc<dyn AbstractAPOInfo>>,
    channel_infos: Vec<ChannelInfo>,
    channel_combo_index: usize,
    channel_names: Vec<String>,
    analysis_channel_names: Vec<String>,
    analysis_channel: String,
    analysis_start_from: i32,
    analysis_resolution: i32,
    instant_mode: bool,
    show_toolbar: bool,
    show_analysis: bool,
    tabs: Vec<FilterTab>,
    active_tab: usize,
    clipboard: TableClipboard,
    hotkey_dialog: HotkeyDialog,
    recent_files: Vec<String>,
    analysis: AnalysisThread,
    analysis_result: Option<AnalysisResult>,
    analysis_state: PlotState,
    analysis_generation: u64,
    volume_controller: Option<eapo_filters::loudness_correction::volume_controller::VolumeController>,
    master_volume: Option<f64>,
    last_volume_check: Instant,
    check_queue: Vec<CheckDialog>,
    info_dialog: Option<InfoDialog>,
    about_open: bool,
    close_prompt: Option<ClosePrompt>,
    close_discarded: HashSet<usize>,
    restart_prompt: Option<RestartPrompt>,
    no_save_preferences: bool,
    no_save_file_preferences: bool,
    closing: bool,
    restart_flag: Arc<AtomicBool>,
    ui_ctx: egui::Context,
    window_size: (f32, f32),
}

impl EditorApp {
    pub fn new(
        cc: &eframe::CreationContext<'_>,
        args: Vec<String>,
        prefs: GlobalPreferences,
        restart_flag: Arc<AtomicBool>,
    ) -> Self {
        eapo_common::egui_fonts::install_cjk_font(&cc.egui_ctx);
        let language = prefs
            .language
            .as_deref()
            .map(crate::i18n::from_code)
            .unwrap_or_else(crate::i18n::system_language);
        crate::i18n::set_language(language);
        let output_devices = DeviceAPOInfo::load_all_infos(false).unwrap_or_default();
        let input_devices = DeviceAPOInfo::load_all_infos(true).unwrap_or_default();
        let default_output_device = output_devices.iter().find(|d| d.is_default_device()).cloned();
        let mut all_devices = output_devices.clone();
        all_devices.extend(input_devices.iter().cloned());
        let config_dir = settings::config_dir_from_registry();
        let channel_infos = channel_infos();
        let window_size = prefs.window_size.unwrap_or((1024.0, 768.0));
        let ui_ctx = cc.egui_ctx.clone();

        let mut app = Self {
            config_dir,
            output_devices,
            input_devices,
            all_devices,
            default_output_device,
            selected_device: None,
            channel_infos,
            channel_combo_index: 0,
            channel_names: Vec::new(),
            analysis_channel_names: Vec::new(),
            analysis_channel: prefs.analysis_channel.clone(),
            analysis_start_from: prefs.analysis_start_from,
            analysis_resolution: round_resolution(prefs.analysis_resolution),
            instant_mode: prefs.instant_mode,
            show_toolbar: prefs.show_toolbar,
            show_analysis: prefs.show_analysis,
            tabs: Vec::new(),
            active_tab: 0,
            clipboard: TableClipboard::default(),
            hotkey_dialog: HotkeyDialog::default(),
            recent_files: prefs.recent_files.clone(),
            analysis: AnalysisThread::new(),
            analysis_result: None,
            analysis_state: PlotState::default(),
            analysis_generation: 0,
            volume_controller: None,
            master_volume: None,
            last_volume_check: Instant::now(),
            check_queue: Vec::new(),
            info_dialog: None,
            about_open: false,
            close_prompt: None,
            close_discarded: HashSet::new(),
            restart_prompt: None,
            no_save_preferences: false,
            no_save_file_preferences: false,
            closing: false,
            restart_flag,
            ui_ctx,
            window_size,
        };
        app.analysis_state.zoom_x = prefs.analysis_zoom_x;
        app.analysis_state.zoom_y = prefs.analysis_zoom_y;
        app.analysis_state.scroll = Vec2::new(
            if prefs.analysis_scroll_x == i32::MIN {
                app.analysis_state.hz_to_x(20.0) as f32
            } else {
                prefs.analysis_scroll_x.max(0) as f32
            },
            if prefs.analysis_scroll_y == i32::MIN {
                app.analysis_state.db_to_y(22.0) as f32
            } else {
                prefs.analysis_scroll_y.max(0) as f32
            },
        );

        // Restore selected device.
        if !prefs.selected_device.is_empty() {
            for dev in app.all_devices.iter().filter(|d| d.is_installed()) {
                if dev.get_device_string().eq_ignore_ascii_case(&prefs.selected_device) {
                    app.selected_device = Some(dev.clone());
                    break;
                }
            }
        }
        app.rebuild_channel_infos();
        if prefs.selected_channel_mask != 0 {
            if let Some(i) = app.channel_infos.iter().position(|c| c.mask as i32 == prefs.selected_channel_mask) {
                app.channel_combo_index = i + 1;
            }
        }
        app.refresh_channel_state(false);

        // Command line arguments (absolute paths as in main.cpp).
        let mut any_arg = false;
        for arg in args {
            any_arg = true;
            let p = {
                let p = Path::new(&arg);
                if p.is_absolute() { p.to_path_buf() } else { app.config_dir.join(p) }
            };
            app.load_path(&p);
        }
        if !any_arg && app.tabs.is_empty() {
            app.load_path(&app.config_dir.join("config.txt"));
        }
        if let Some(active) = app.tabs.get(app.active_tab) {
            let channel = &prefs.analysis_channel;
            if !channel.is_empty() && app.analysis_channel_names.iter().any(|c| c == channel) {
                app.analysis_channel = channel.clone();
            }
            let _ = active;
        }
        app.build_checks();
        app.start_analysis();
        let _ = cc;
        app
    }

    fn build_checks(&mut self) {
        let registry_problem = !DeviceAPOInfo::check_protected_audio_dg(false).unwrap_or(false)
            || !DeviceAPOInfo::check_apo_registration(false);
        if registry_problem {
            self.check_queue.push(CheckDialog {
                title: crate::i18n::tr("Registry problem"),
                text: crate::i18n::tr("A registry value that is required for the operation of Equalizer APO is not set correctly.\nDo you want to run the Device Selector application to fix the problem?"),
            });
        }
        if let Some(default) = &self.default_output_device {
            if !default.is_installed() {
                self.check_queue.push(CheckDialog {
                    title: crate::i18n::tr("APO not installed to device"),
                    text: crate::i18n::tr("Equalizer APO has not been installed to the selected device.\nDo you want to run the Device Selector application to fix the problem?"),
                });
            }
        }
        let disabled = self
            .output_devices
            .iter()
            .chain(self.input_devices.iter())
            .find(|d| d.is_installed() && d.is_enhancements_disabled())
            .cloned();
        if let Some(dev) = disabled {
            self.check_queue.push(CheckDialog {
                title: crate::i18n::tr("Audio enhancements disabled"),
                text: crate::i18n::tr_arg(
                    "Audio enhancements are not enabled for the device\n%0 %1.\nDo you want to run the Device Selector application to fix the problem?",
                    &[&dev.get_connection_name(), &dev.get_device_name()],
                ),
            });
        }
    }

    fn selected_device(&self) -> Option<Arc<dyn AbstractAPOInfo>> {
        self.selected_device.clone().or_else(|| self.default_output_device.clone())
    }

    fn rebuild_channel_infos(&mut self) {
        self.channel_infos = channel_infos();
        self.channel_combo_index = self.channel_combo_index.min(self.channel_infos.len());
    }

    fn from_device_label(&self) -> String {
        if let Some(dev) = self.selected_device() {
            if let Some(info) = self.channel_infos.iter().find(|i| i.mask == dev.get_channel_mask()) {
                format!("{} ({})", crate::i18n::tr("From device"), info.name)
            } else if dev.get_channel_count() != 0 {
                format!("{} ({} channels)", crate::i18n::tr("From device"), dev.get_channel_count())
            } else {
                format!("{} (? channels)", crate::i18n::tr("From device"))
            }
        } else {
            format!("{} (?)", crate::i18n::tr("From device"))
        }
    }

    fn channel_mask(&self) -> u32 {
        if let Some(dev) = self.selected_device() {
            if self.channel_combo_index == 0 {
                let mask = dev.get_channel_mask();
                if mask != 0 { mask } else { channel_helper::get_default_channel_mask(dev.get_channel_count() as i32) }
            } else {
                self.channel_infos.get(self.channel_combo_index - 1).map(|c| c.mask).unwrap_or(0)
            }
        } else {
            0
        }
    }

    fn effective_channel_mask_and_names(&self) -> (u32, Vec<String>) {
        let Some(dev) = self.selected_device() else { return (0, Vec::new()) };
        let mut count = dev.get_channel_count();
        let mut mask = self.channel_mask();
        if mask != 0 && mask != dev.get_channel_mask() {
            count = mask.count_ones();
        }
        if count == 0 {
            count = 8;
            mask = channel_helper::KSAUDIO_SPEAKER_7POINT1_SURROUND;
        }
        let names = channel_helper::get_channel_names(count as i32, mask);
        (mask, names)
    }

    fn refresh_channel_state(&mut self, update_tabs: bool) {
        let (mask, names) = self.effective_channel_mask_and_names();
        self.channel_names = names;
        self.analysis_channel_names = self.channel_names.clone();
        if !self.analysis_channel_names.is_empty()
            && !self.analysis_channel_names.iter().any(|c| c == &self.analysis_channel)
        {
            self.analysis_channel = self.analysis_channel_names[0].clone();
        }
        if update_tabs {
            let sample_rate = self.selected_device().map(|d| d.get_sample_rate()).unwrap_or(0);
            for tab in &mut self.tabs {
                let path = tab.config_path.clone();
                tab.configure_channels(&self.channel_names, mask, sample_rate, &path);
            }
        }
    }

    fn channel_configuration_selected(&mut self) {
        self.refresh_channel_state(true);
        self.start_analysis();
    }

    fn device_selected(&mut self) {
        self.channel_combo_index = 0;
        self.rebuild_channel_infos();
        self.channel_configuration_selected();
    }

    fn build_ctx<'a>(&'a self, config_path: &'a str) -> FilterContext<'a> {
        FilterContext {
            config_path,
            channel_names: &self.channel_names,
            channel_mask: self.channel_mask(),
            device_sample_rate: self.selected_device().map(|d| d.get_sample_rate()).unwrap_or(0),
        }
    }

    fn load_path(&mut self, path: &Path) -> bool {
        let path = path.to_path_buf();
        let native = settings::native_path(&path);
        if let Some(i) = self.tabs.iter().position(|t| t.config_path == native) {
            self.active_tab = i;
            self.start_analysis();
            return true;
        }
        let lines = match filter_table::load_config_lines(&native) {
            Ok(lines) => lines,
            Err(e) => {
                self.info_dialog = Some(InfoDialog {
                    title: crate::i18n::tr("Error"),
                    text: crate::i18n::tr_arg("Error while reading configuration file: %0", &[&e.to_string()]),
                });
                return false;
            }
        };
        let title = file_title(&native);
        let ctx = self.build_ctx(&native);
        let tab = FilterTab::new(title, native.clone(), native.clone(), lines, &ctx);
        tab.hotkeys.set_repaint_context(self.ui_ctx.clone());
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        settings::push_recent(&mut self.recent_files, &native);
        self.start_analysis();
        true
    }

    fn new_tab(&mut self) {
        let ctx = self.build_ctx("");
        let tab = FilterTab::new("Unsaved".into(), String::new(), String::new(), Vec::new(), &ctx);
        tab.hotkeys.set_repaint_context(self.ui_ctx.clone());
        self.tabs.push(tab);
        self.active_tab = self.tabs.len() - 1;
        self.start_analysis();
    }

    fn save_tab(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        if self.tabs[index].config_path.is_empty() {
            return self.save_tab_as(index);
        }
        let path = self.tabs[index].config_path.clone();
        let lines = self.tabs[index].lines();
        if let Err(e) = filter_table::save_config_file(&path, &lines) {
            self.info_dialog = Some(InfoDialog {
                title: crate::i18n::tr("Error"),
                text: crate::i18n::tr_arg("Error while writing configuration file: %0", &[&e.to_string()]),
            });
            return false;
        }
        self.tabs[index].mark_saved();
        self.start_analysis();
        true
    }

    fn save_tab_as(&mut self, index: usize) -> bool {
        if index >= self.tabs.len() {
            return false;
        }
        let tab = &self.tabs[index];
        let mut dialog = crate::file_dialog::FileDialog::new()
            .add_filter(&crate::i18n::tr("E-APO configurations (*.txt)"), &["txt"])
            .set_title(&crate::i18n::tr("Save file as"));
        let path = if tab.config_path.is_empty() {
            dialog = dialog.set_directory(&self.config_dir);
            dialog = dialog.set_file_name("config.txt");
            dialog.save_file()
        } else {
            let p = Path::new(&tab.config_path);
            if let Some(parent) = p.parent() {
                dialog = dialog.set_directory(parent);
            }
            if let Some(name) = p.file_name() {
                dialog = dialog.set_file_name(name.to_string_lossy());
            }
            dialog.save_file()
        };
        let Some(mut path) = path else { return false };
        if path.extension().is_none() {
            path.set_extension("txt");
        }
        let native = settings::native_path(&path);
        let lines = self.tabs[index].lines();
        if let Err(e) = filter_table::save_config_file(&native, &lines) {
            self.info_dialog = Some(InfoDialog {
                title: crate::i18n::tr("Error"),
                text: crate::i18n::tr_arg("Error while writing configuration file: %0", &[&e.to_string()]),
            });
            return false;
        }
        let tab = &mut self.tabs[index];
        tab.config_path = native.clone();
        tab.tooltip = native.clone();
        tab.title = file_title(&native);
        tab.mark_saved();
        self.start_analysis();
        true
    }

    fn close_tab_now(&mut self, index: usize, save_prefs: bool) {
        if index >= self.tabs.len() {
            return;
        }
        if save_prefs && !self.no_save_file_preferences {
            self.tabs[index].save_preferences();
        }
        let path = self.tabs[index].config_path.clone();
        if !path.is_empty() {
            settings::push_recent(&mut self.recent_files, &path);
        }
        self.tabs.remove(index);
        if self.active_tab >= self.tabs.len() {
            self.active_tab = self.tabs.len().saturating_sub(1);
        }
        self.start_analysis();
    }

    fn request_tab_close(&mut self, index: usize) {
        if index >= self.tabs.len() {
            return;
        }
        if self.tabs[index].dirty {
            self.close_prompt = Some(ClosePrompt { target: PromptTarget::Tab, tab_index: index });
        } else {
            self.close_tab_now(index, true);
        }
    }

    fn request_app_close(&mut self, ctx: &egui::Context) {
        if self.closing {
            return;
        }
        if self.close_prompt.is_some() {
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            return;
        }
        self.close_discarded.clear();
        self.continue_app_close(ctx);
    }

    fn first_dirty_tab(&self) -> Option<usize> {
        self.tabs
            .iter()
            .enumerate()
            .find(|(i, t)| t.dirty && !self.close_discarded.contains(i))
            .map(|(i, _)| i)
    }

    fn continue_app_close(&mut self, ctx: &egui::Context) {
        if let Some(index) = self.first_dirty_tab() {
            self.close_prompt = Some(ClosePrompt { target: PromptTarget::App, tab_index: index });
            // The close request is cancelled until the prompt is resolved.
            ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
        } else {
            self.finalize_close(ctx);
        }
    }

    fn finalize_close(&mut self, ctx: &egui::Context) {
        self.save_preferences();
        self.closing = true;
        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
    }

    fn save_preferences(&mut self) {
        if !self.no_save_preferences {
            let size = self.window_size;
            let mut prefs = GlobalPreferences::default();
            prefs.instant_mode = self.instant_mode;
            prefs.selected_device = self
                .selected_device
                .as_ref()
                .map(|d| d.get_device_string())
                .unwrap_or_default();
            prefs.selected_channel_mask = if self.channel_combo_index == 0 {
                0
            } else {
                self.channel_infos
                    .get(self.channel_combo_index - 1)
                    .map(|c| c.mask as i32)
                    .unwrap_or(0)
            };
            prefs.analysis_start_from = self.analysis_start_from;
            prefs.analysis_channel = self.analysis_channel.clone();
            prefs.analysis_resolution = self.analysis_resolution;
            prefs.analysis_zoom_x = self.analysis_state.zoom_x;
            prefs.analysis_zoom_y = self.analysis_state.zoom_y;
            prefs.analysis_scroll_x = self.analysis_state.scroll.x as i32;
            prefs.analysis_scroll_y = self.analysis_state.scroll.y as i32;
            prefs.recent_files = self.recent_files.clone();
            prefs.window_size = Some(size);
            prefs.show_toolbar = self.show_toolbar;
            prefs.show_analysis = self.show_analysis;
            prefs.language = eapo_common::registry_helper::read_value(settings::EDITOR_REGPATH, "language").ok();
            prefs.save();
        }
        if !self.no_save_file_preferences {
            for (i, tab) in self.tabs.iter().enumerate() {
                if !self.close_discarded.contains(&i) {
                    tab.save_preferences();
                }
            }
        }
    }

    fn start_analysis(&mut self) {
        if !self.show_analysis {
            return;
        }
        let Some(device) = self.selected_device() else { return };
        let config_path = if self.analysis_start_from == 1 {
            self.tabs
                .get(self.active_tab)
                .filter(|t| !t.config_path.is_empty())
                .map(|t| t.config_path.clone())
                .unwrap_or_else(|| settings::native_path(&self.config_dir.join("config.txt")))
        } else {
            settings::native_path(&self.config_dir.join("config.txt"))
        };
        let mask = self.channel_mask();
        let channel_index = self
            .analysis_channel_names
            .iter()
            .position(|c| c == &self.analysis_channel)
            .unwrap_or(0);
        self.analysis_generation += 1;
        self.analysis.set_parameters(AnalysisRequest {
            device: DeviceSnapshot {
                capture: device.is_input(),
                device_name: device.get_device_name(),
                connection_name: device.get_connection_name(),
                device_guid: device.get_device_guid(),
                device_string: device.get_device_string(),
                channel_count: device.get_channel_count(),
                sample_rate: device.get_sample_rate(),
                channel_mask: device.get_channel_mask(),
            },
            channel_mask: mask,
            channel_index,
            config_path,
            frame_count: self.analysis_resolution.max(128) as usize,
            generation: self.analysis_generation,
        });
    }

    fn poll_analysis(&mut self, ctx: &egui::Context) {
        while let Some(result) = self.analysis.try_recv() {
            if result.generation == self.analysis_generation {
                self.analysis_result = Some(result);
                ctx.request_repaint();
            }
        }
    }

    fn poll_hotkeys(&mut self, ctx: &egui::Context) {
        for index in 0..self.tabs.len() {
            let ids = self.tabs[index].poll_hotkeys();
            let mut changed = false;
            for id in ids {
                if self.tabs[index].toggle_by_filter_id(&id) {
                    changed = true;
                }
            }
            if changed {
                self.tabs[index].mark_changed();
                if self.instant_mode && !self.tabs[index].config_path.is_empty() {
                    self.save_tab(index);
                }
                ctx.request_repaint();
            }
        }
    }

    fn poll_volume(&mut self) {
        let has_loudness = self.tabs.iter().any(|t| {
            t.items.iter().any(|i| matches!(i.editor, Some(crate::model::FilterEditor::Loudness { .. })))
        });
        if !has_loudness {
            return;
        }
        if self.last_volume_check.elapsed() < Duration::from_millis(10) {
            return;
        }
        self.last_volume_check = Instant::now();
        if self.volume_controller.is_none() {
            self.volume_controller = Some(eapo_filters::loudness_correction::volume_controller::VolumeController::new());
        }
        if let Some(vc) = &self.volume_controller {
            let mut volume = 0.0;
            if vc.get_volume(&mut volume).ok().is_ok() {
                if self.master_volume != Some(volume) {
                    self.master_volume = Some(volume);
                    self.start_analysis();
                }
            }
        }
    }

    fn active_clipboard_text(&self, ctx: &egui::Context) -> Option<String> {
        let _ = ctx;
        None
    }

    fn menu_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("menu_bar").show(ctx, |ui| {
            egui::menu::bar(ui, |ui| {
                ui.menu_button(crate::i18n::tr("File"), |ui| {
                    if menu_button(ui, &crate::i18n::tr("New"), "Ctrl+N") {
                        self.new_tab();
                    }
                    if menu_button(ui, &crate::i18n::tr("Open"), "Ctrl+O") {
                        self.open_dialog();
                    }
                    if menu_button(ui, &crate::i18n::tr("Save"), "Ctrl+S") {
                        self.save_current();
                    }
                    if menu_button(ui, &crate::i18n::tr("Save as..."), "") {
                        self.save_current_as();
                    }
                    ui.separator();
                    for recent in self.recent_files.clone() {
                        if ui.button(&recent).clicked() {
                            self.load_path(Path::new(&recent));
                            ui.close_menu();
                        }
                    }
                    if !self.recent_files.is_empty() {
                        ui.separator();
                    }
                    if menu_button(ui, &crate::i18n::tr("Exit"), "Alt+F4") {
                        ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                    }
                });
                ui.menu_button(crate::i18n::tr("Edit"), |ui| {
                    if menu_button(ui, &crate::i18n::tr("Cut"), "Ctrl+X") {
                        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                            if tab.cut(&mut self.clipboard, ctx) {
                                tab.mark_changed();
                                self.maybe_save_active();
                            }
                        }
                    }
                    if menu_button(ui, &crate::i18n::tr("Copy"), "Ctrl+C") {
                        if let Some(tab) = self.tabs.get(self.active_tab) {
                            tab.copy(&mut self.clipboard, ctx);
                        }
                    }
                    if menu_button(ui, &crate::i18n::tr("Paste"), "Ctrl+V") {
                        self.paste_active(ctx);
                    }
                    if menu_button(ui, &crate::i18n::tr("Delete"), "") {
                        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                            if tab.delete_selected() {
                                self.maybe_save_active();
                            }
                        }
                    }
                    ui.separator();
                    if menu_button(ui, &crate::i18n::tr("Select all"), "Ctrl+A") {
                        if let Some(tab) = self.tabs.get_mut(self.active_tab) {
                            tab.select_all();
                        }
                    }
                });
                ui.menu_button(crate::i18n::tr("View"), |ui| {
                    ui.checkbox(&mut self.show_toolbar, crate::i18n::tr("Toolbar"));
                    if ui.checkbox(&mut self.show_analysis, crate::i18n::tr("Analysis panel")).changed() && self.show_analysis {
                        self.start_analysis();
                    }
                });
                ui.menu_button(crate::i18n::tr("Settings"), |ui| {
                    ui.menu_button(crate::i18n::tr("Language"), |ui| {
                        let current = eapo_common::registry_helper::read_value(settings::EDITOR_REGPATH, "language").unwrap_or_default();
                        let current_language = if current.is_empty() { None } else { Some(crate::i18n::from_code(&current)) };
                        let system = crate::i18n::system_language();
                        let automatic = crate::i18n::tr_arg(
                            "Automatic (%0)",
                            &[crate::i18n::native_name(system)],
                        );
                        let choices = [
                            (None, automatic),
                            (Some(crate::i18n::Language::English), "English".to_string()),
                            (Some(crate::i18n::Language::German), "Deutsch".to_string()),
                            (Some(crate::i18n::Language::Chinese), "中文".to_string()),
                            (Some(crate::i18n::Language::French), "Français".to_string()),
                        ];
                        for (choice, label) in choices {
                            let selected = current_language == choice;
                            if ui.radio(selected, label).clicked() {
                                let code = choice.map(crate::i18n::language_code);
                                self.restart_prompt = Some(RestartPrompt { action: RestartAction::Language(code) });
                                ui.close_menu();
                            }
                        }
                    });
                    ui.separator();
                    if menu_button(ui, &crate::i18n::tr("Reset all global preferences"), "") {
                        self.restart_prompt = Some(RestartPrompt { action: RestartAction::ResetGlobal });
                    }
                    if menu_button(ui, &crate::i18n::tr("Reset all file-specific preferences"), "") {
                        self.restart_prompt = Some(RestartPrompt { action: RestartAction::ResetFileSpecific });
                    }
                });
            });
        });
    }

    fn toolbar(&mut self, ctx: &egui::Context) {
        if !self.show_toolbar {
            return;
        }
        egui::TopBottomPanel::top("main_toolbar").show(ctx, |ui| {
            ui.horizontal(|ui| {
                if ui.button(crate::i18n::tr("New")).clicked() {
                    self.new_tab();
                }
                if ui.button(crate::i18n::tr("Open")).clicked() {
                    self.open_dialog();
                }
                if ui.button(crate::i18n::tr("Save")).clicked() {
                    self.save_current();
                }
                ui.add_space(10.0);
                let was = self.instant_mode;
                if ui.checkbox(&mut self.instant_mode, crate::i18n::tr("Instant mode")).on_hover_text(crate::i18n::tr("Changes are saved immediately")).changed()
                    && !was
                    && self.instant_mode
                {
                    self.instant_mode_enabled();
                }
                ui.separator();
                ui.label(crate::i18n::tr("Device: "));
                let mut selected_changed = false;
                let selected_text = match &self.selected_device {
                    Some(d) => device_label(d),
                    None => self
                        .default_output_device
                        .as_ref()
                        .map(|d| format!("{} ({})", crate::i18n::tr("Default"), device_label(d)))
                        .unwrap_or_else(|| "Default".into()),
                };
                egui::ComboBox::from_id_salt("device_combo")
                    .selected_text(selected_text)
                    .width(320.0)
                    .show_ui(ui, |ui| {
                        if let Some(default) = self.default_output_device.clone() {
                            let label = format!("{} ({})", crate::i18n::tr("Default"), device_label(&default));
                            if ui.selectable_label(self.selected_device.is_none(), label).clicked() {
                                self.selected_device = None;
                                selected_changed = true;
                                ui.close_menu();
                            }
                        }
                        ui.label(egui::RichText::new(crate::i18n::tr("Playback devices:")).strong());
                        for dev in self.output_devices.iter().filter(|d| d.is_installed()) {
                            let selected = self
                                .selected_device
                                .as_ref()
                                .map(|s| s.get_device_string() == dev.get_device_string())
                                .unwrap_or(false);
                            if ui.selectable_label(selected, device_label(dev)).clicked() {
                                self.selected_device = Some(dev.clone());
                                selected_changed = true;
                                ui.close_menu();
                            }
                        }
                        ui.label(egui::RichText::new(crate::i18n::tr("Capture devices:")).strong());
                        for dev in self.input_devices.iter().filter(|d| d.is_installed()) {
                            let selected = self
                                .selected_device
                                .as_ref()
                                .map(|s| s.get_device_string() == dev.get_device_string())
                                .unwrap_or(false);
                            if ui.selectable_label(selected, device_label(dev)).clicked() {
                                self.selected_device = Some(dev.clone());
                                selected_changed = true;
                                ui.close_menu();
                            }
                        }
                    });
                if selected_changed {
                    self.device_selected();
                }
                ui.add_space(10.0);
                ui.label(crate::i18n::tr("Channel configuration: "));
                let mut channel_changed = false;
                let selected_channel = if self.channel_combo_index == 0 {
                    self.from_device_label()
                } else {
                    self.channel_infos
                        .get(self.channel_combo_index - 1)
                        .map(|c| c.name.to_string())
                        .unwrap_or_default()
                };
                egui::ComboBox::from_id_salt("channel_combo")
                    .selected_text(selected_channel)
                    .show_ui(ui, |ui| {
                        if ui
                            .selectable_label(self.channel_combo_index == 0, self.from_device_label())
                            .clicked()
                        {
                            self.channel_combo_index = 0;
                            channel_changed = true;
                            ui.close_menu();
                        }
                        for (i, info) in self.channel_infos.iter().enumerate() {
                            if ui
                                .selectable_label(self.channel_combo_index == i + 1, info.name)
                                .clicked()
                            {
                                self.channel_combo_index = i + 1;
                                channel_changed = true;
                                ui.close_menu();
                            }
                        }
                    });
                if channel_changed {
                    self.channel_configuration_selected();
                }
            });
        });
    }

    fn analysis_panel(&mut self, ctx: &egui::Context) {
        if !self.show_analysis {
            return;
        }
        egui::TopBottomPanel::bottom("analysis_dock")
            .resizable(true)
            .default_height(330.0)
            .min_height(180.0)
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    ui.vertical(|ui| {
                        ui.set_width(300.0);
                        ui.group(|ui| {
                            ui.label(egui::RichText::new(crate::i18n::tr("Settings")).strong());
                            egui::Grid::new("analysis_settings").num_columns(2).show(ui, |ui| {
                                ui.label(crate::i18n::tr("Start from:"));
                                let before = self.analysis_start_from;
                                egui::ComboBox::from_id_salt("start_from")
                                    .selected_text(if self.analysis_start_from == 1 {
                                        crate::i18n::tr("Current file")
                                    } else {
                                        crate::i18n::tr("config.txt")
                                    })
                                    .show_ui(ui, |ui| {
                                        ui.selectable_value(&mut self.analysis_start_from, 0, crate::i18n::tr("config.txt"));
                                        ui.selectable_value(&mut self.analysis_start_from, 1, crate::i18n::tr("Current file"));
                                    });
                                if before != self.analysis_start_from {
                                    self.start_analysis();
                                }
                                ui.end_row();
                                ui.label(crate::i18n::tr("Channel:"));
                                let mut channel_changed = false;
                                egui::ComboBox::from_id_salt("analysis_channel")
                                    .selected_text(&self.analysis_channel)
                                    .show_ui(ui, |ui| {
                                        for name in &self.analysis_channel_names {
                                            if ui.selectable_label(&self.analysis_channel == name, name).clicked() {
                                                self.analysis_channel = name.clone();
                                                channel_changed = true;
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                if channel_changed {
                                    self.start_analysis();
                                }
                                ui.end_row();
                                ui.label(crate::i18n::tr("Resolution:"));
                                let mut resolution_changed = false;
                                egui::ComboBox::from_id_salt("analysis_resolution")
                                    .selected_text(self.analysis_resolution.to_string())
                                    .show_ui(ui, |ui| {
                                        for v in [128, 256, 512, 1024, 2048, 4096, 8192, 16384, 32768, 65536, 131072, 262144, 524288, 1048576, 2097152, 4194304, 8388608] {
                                            if ui.selectable_label(self.analysis_resolution == v, v.to_string()).clicked() {
                                                self.analysis_resolution = v;
                                                resolution_changed = true;
                                                ui.close_menu();
                                            }
                                        }
                                    });
                                if resolution_changed {
                                    self.start_analysis();
                                }
                                ui.end_row();
                            });
                        });
                        ui.group(|ui| {
                            ui.label(egui::RichText::new(crate::i18n::tr("Estimated properties")).strong());
                            egui::Grid::new("analysis_values").num_columns(2).show(ui, |ui| {
                                ui.label(crate::i18n::tr("Peak gain:"));
                                let (peak, color) = match &self.analysis_result {
                                    Some(r) => (format!("{:.1} dB", r.peak_gain), if r.peak_gain > 0.0 { Color32::LIGHT_RED } else { ui.visuals().text_color() }),
                                    None => (String::new(), ui.visuals().text_color()),
                                };
                                ui.colored_label(color, peak);
                                ui.end_row();
                                ui.label(crate::i18n::tr("Latency: "));
                                let latency = match &self.analysis_result {
                                    Some(r) if r.freq_data_sample_rate > 0 => format!(
                                        "{:.1} ms ({} s.)",
                                        r.latency as f64 * 1000.0 / r.freq_data_sample_rate as f64,
                                        r.latency
                                    ),
                                    _ => String::new(),
                                };
                                ui.label(latency);
                                ui.end_row();
                                ui.label(crate::i18n::tr("Init. time:"));
                                let init = self.analysis_result.as_ref().map(|r| format!("{:.1} ms", r.initialization_time)).unwrap_or_default();
                                ui.label(init);
                                ui.end_row();
                                ui.label(crate::i18n::tr("CPU usage:"));
                                let (cpu, color) = match &self.analysis_result {
                                    Some(r) => {
                                        let usage = r.cpu_usage();
                                        let color = if usage >= 50.0 { Color32::LIGHT_RED } else if usage >= 20.0 { Color32::from_rgb(255, 160, 0) } else { ui.visuals().text_color() };
                                        (format!("{:.1} % (one core)", usage), color)
                                    }
                                    None => (String::new(), ui.visuals().text_color()),
                                };
                                ui.colored_label(color, cpu);
                                ui.end_row();
                            });
                        });
                    });
                    ui.separator();
                    let nodes = self
                        .analysis_result
                        .as_ref()
                        .map(|r| r.nodes())
                        .unwrap_or_default();
                    let height = (ui.available_height() - 4.0).max(80.0);
                    plot::show_analysis(ui, &mut self.analysis_state, height, &nodes);
                });
            });
    }

    fn open_dialog(&mut self) {
        let mut dialog = crate::file_dialog::FileDialog::new()
            .add_filter(&crate::i18n::tr("E-APO configurations (*.txt)"), &["txt"])
            .set_title(&crate::i18n::tr("Open file"));
        if let Some(tab) = self.tabs.get(self.active_tab) {
            if !tab.config_path.is_empty() {
                if let Some(parent) = Path::new(&tab.config_path).parent() {
                    dialog = dialog.set_directory(parent);
                }
            } else {
                dialog = dialog.set_directory(&self.config_dir);
            }
        } else {
            dialog = dialog.set_directory(&self.config_dir);
        }
        if let Some(files) = dialog.pick_files() {
            for file in files {
                self.load_path(&file);
            }
        }
    }

    fn save_current(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        self.save_tab(self.active_tab);
    }

    fn save_current_as(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        self.save_tab_as(self.active_tab);
    }

    fn reload_current(&mut self) {
        let Some(tab) = self.tabs.get(self.active_tab) else { return };
        if tab.config_path.is_empty() {
            return;
        }
        let path = tab.config_path.clone();
        let title = tab.title.clone();
        let tooltip = tab.tooltip.clone();
        match filter_table::load_config_lines(&path) {
            Ok(lines) => {
                let ctx = self.build_ctx(&path);
                let new_tab = FilterTab::new(title, tooltip, path.clone(), lines, &ctx);
                new_tab.hotkeys.set_repaint_context(self.ui_ctx.clone());
                let active = self.active_tab;
                self.tabs[active] = new_tab;
                self.start_analysis();
            }
            Err(e) => {
                self.info_dialog = Some(InfoDialog {
                    title: crate::i18n::tr("Error"),
                    text: crate::i18n::tr_arg("Error while reading configuration file: %0", &[&e.to_string()]),
                });
            }
        }
    }

    fn paste_active(&mut self, ctx: &egui::Context) {
        let sys = ctx.input(|i| i.events.iter().find_map(|e| match e {
            egui::Event::Paste(t) => Some(t.clone()),
            _ => None,
        }));
        let clipboard = self.clipboard.clone();
        let path = self.tabs.get(self.active_tab).map(|t| t.config_path.clone()).unwrap_or_default();
        let names = self.channel_names.clone();
        let mask = self.channel_mask();
        let rate = self.selected_device().map(|d| d.get_sample_rate()).unwrap_or(0);
        let fctx = FilterContext { config_path: &path, channel_names: &names, channel_mask: mask, device_sample_rate: rate };
        let pasted = self
            .tabs
            .get_mut(self.active_tab)
            .map(|tab| tab.paste(&clipboard, &fctx, sys))
            .unwrap_or(false);
        if pasted {
            self.maybe_save_active();
        }
    }

    fn maybe_save_active(&mut self) {
        if self.tabs.is_empty() {
            return;
        }
        self.tabs[self.active_tab].mark_changed();
        if self.instant_mode && !self.tabs[self.active_tab].config_path.is_empty() {
            self.save_tab(self.active_tab);
        }
    }

    fn toggle_selected_filters(&mut self) {
        let Some(tab) = self.tabs.get_mut(self.active_tab) else { return };
        if tab.selected.is_empty() {
            return;
        }
        let targets: Vec<u64> = tab.items.iter().filter(|i| tab.selected.contains(&i.uid)).map(|i| i.uid).collect();
        for uid in targets {
            if let Some(i) = tab.index_of_uid(uid) {
                let new_enabled = !tab.items[i].enabled;
                tab.items[i].set_enabled(new_enabled);
            }
        }
        tab.mark_changed();
        self.maybe_save_active();
    }

    fn instant_mode_enabled(&mut self) {
        for i in 0..self.tabs.len() {
            if !self.tabs[i].config_path.is_empty() {
                self.save_tab(i);
            }
        }
    }

    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        if self.hotkey_dialog.open || self.close_prompt.is_some() || self.restart_prompt.is_some() || self.info_dialog.is_some() {
            return;
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::N)) {
            self.new_tab();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::O)) {
            self.open_dialog();
        }
        if ctx.input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::S)) {
            self.save_current();
        }
    }

    fn tab_bar(&mut self, ui: &mut egui::Ui) {
        let mut select = None;
        let mut close = None;
        ui.horizontal_wrapped(|ui| {
            for (i, tab) in self.tabs.iter().enumerate() {
                let title = if tab.dirty { format!("{}*", tab.title) } else { tab.title.clone() };
                let resp = ui.selectable_label(i == self.active_tab, title);
                if resp.clicked() {
                    select = Some(i);
                }
                if resp.middle_clicked() {
                    close = Some(i);
                }
                resp.on_hover_text(&tab.tooltip);
                if ui.small_button("x").clicked() {
                    close = Some(i);
                }
            }
        });
        if let Some(i) = select {
            if i != self.active_tab {
                self.active_tab = i;
                self.start_analysis();
            }
        }
        if let Some(i) = close {
            self.request_tab_close(i);
        }
    }

    fn central(&mut self, ctx: &egui::Context) {
        egui::CentralPanel::default().show(ctx, |ui| {
            self.tab_bar(ui);
            ui.separator();
            if self.tabs.is_empty() {
                ui.centered_and_justified(|ui| {
                    ui.label(crate::i18n::tr("No configuration open. Use File > Open or File > New."));
                });
                return;
            }
            let active = self.active_tab.min(self.tabs.len() - 1);
            self.active_tab = active;
            let _ = self.poll_analysis(ctx);
            let config_path = self.tabs[active].config_path.clone();
            let names = self.channel_names.clone();
            let mask = self.channel_mask();
            let rate = self.selected_device().map(|d| d.get_sample_rate()).unwrap_or(0);
            let fctx = FilterContext { config_path: &config_path, channel_names: &names, channel_mask: mask, device_sample_rate: rate };
            let all_devices = self.all_devices.clone();
            let services = GuiServices {
                all_devices: &all_devices,
                config_path: &config_path,
                master_volume: self.master_volume,
            };
            let keyboard_enabled = !self.hotkey_dialog.open
                && self.close_prompt.is_none()
                && self.restart_prompt.is_none()
                && self.info_dialog.is_none()
                && !self.about_open
                && self.check_queue.is_empty();
            let mut report = TabUiReport::default();
            self.tabs[active].ui(
                ui,
                &services,
                &fctx,
                &mut self.clipboard,
                &mut self.hotkey_dialog,
                &mut report,
                keyboard_enabled,
            );
            if report.changed {
                self.tabs[active].mark_changed();
                if report.hotkey_changed {
                    self.tabs[active].save_preferences();
                }
                self.maybe_save_active();
            }
            if let Some(path) = report.open_config {
                self.load_path(Path::new(&path));
            }
        });
    }

    fn show_hotkey_dialog(&mut self, ctx: &egui::Context) {
        if !self.hotkey_dialog.open {
            return;
        }
        let target = self.hotkey_dialog.target_filter_id.clone();
        if let Some(result) = self.hotkey_dialog.show(ctx) {
            if let Some(filter_id) = target {
                let mut ok = false;
                for tab in &mut self.tabs {
                    if tab.item_by_filter_id(&filter_id).is_some() {
                        ok = tab.set_hotkey(&filter_id, result);
                        tab.save_preferences();
                        tab.mark_changed();
                        break;
                    }
                }
                if !ok && result.is_some() {
                    self.info_dialog = Some(InfoDialog {
                        title: "Hotkey".into(),
                        text: "The hotkey could not be registered. It may already be in use by another application.".into(),
                    });
                }
                self.maybe_save_active();
            }
        }
    }

    fn show_prompts(&mut self, ctx: &egui::Context) {
        // doChecks dialogs.
        if let Some(check) = self.check_queue.first() {
            let mut answer = None;
            egui::Window::new(&check.title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(&check.text);
                    ui.horizontal(|ui| {
                        if ui.button(crate::i18n::tr("Yes")).clicked() {
                            answer = Some(true);
                        }
                        if ui.button(crate::i18n::tr("No")).clicked() {
                            answer = Some(false);
                        }
                    });
                });
            if let Some(yes) = answer {
                self.check_queue.remove(0);
                if yes {
                    launch_next_to_editor("DeviceSelector.exe");
                    self.check_queue.clear();
                }
            }
        }

        if let Some(info) = &self.info_dialog {
            let mut close = false;
            egui::Window::new(&info.title)
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(&info.text);
                    if ui.button(crate::i18n::tr("OK")).clicked() {
                        close = true;
                    }
                });
            if close {
                self.info_dialog = None;
            }
        }

        if self.about_open {
            let mut open = true;
            egui::Window::new(crate::i18n::tr("About"))
                .collapsible(false)
                .resizable(false)
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.heading(format!("Equalizer APO {VERSION} Configuration Editor"));
                    ui.label(crate::i18n::tr("Global per-filter hotkey fork, pure Rust port."));
                    ui.label(crate::i18n::tr("Copyright (C) EqualizerAPO Contributors. GPL-2.0-or-later."));
                });
            self.about_open = open;
        }

        if let Some(prompt) = &self.close_prompt {
            let index = prompt.tab_index;
            let target = prompt.target;
            let path = self
                .tabs
                .get(index)
                .map(|t| if t.tooltip.is_empty() { t.config_path.clone() } else { t.tooltip.clone() })
                .unwrap_or_default();
            let mut action = 0;
            egui::Window::new(crate::i18n::tr("Unsaved changes"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(crate::i18n::tr_arg("The configuration file %0 has unsaved changes.", &[&path]));
                    ui.label(crate::i18n::tr("Do you want to save the changes before closing the file?"));
                    ui.horizontal(|ui| {
                        if ui.button(crate::i18n::tr("Save")).clicked() {
                            action = 1;
                        }
                        if ui.button(crate::i18n::tr("Discard")).clicked() {
                            action = 2;
                        }
                        if ui.button(crate::i18n::tr("Cancel")).clicked() {
                            action = 3;
                        }
                    });
                });
            if action == 1 {
                if self.save_tab(index) {
                    match target {
                        PromptTarget::Tab => {
                            self.close_prompt = None;
                            self.close_tab_now(index, true);
                        }
                        PromptTarget::App => {
                            self.close_prompt = None;
                            self.continue_app_close(ctx);
                        }
                    }
                }
            } else if action == 2 {
                self.close_prompt = None;
                match target {
                    PromptTarget::Tab => self.close_tab_now(index, false),
                    PromptTarget::App => {
                        self.close_discarded.insert(index);
                        self.continue_app_close(ctx);
                    }
                }
            } else if action == 3 {
                self.close_prompt = None;
                self.close_discarded.clear();
                self.no_save_preferences = false;
                self.no_save_file_preferences = false;
                if target == PromptTarget::App {
                    self.restart_flag.store(false, Ordering::SeqCst);
                }
                ctx.send_viewport_cmd(egui::ViewportCommand::CancelClose);
            }
        }

        if let Some(prompt) = &self.restart_prompt {
            let action = prompt.action;
            let mut answer = None;
            egui::Window::new(crate::i18n::tr("Restart required"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
                .show(ctx, |ui| {
                    ui.label(crate::i18n::tr("Configuration Editor will be restarted to apply the changed settings. Proceed?"));
                    ui.horizontal(|ui| {
                        if ui.button(crate::i18n::tr("Yes")).clicked() {
                            answer = Some(true);
                        }
                        if ui.button(crate::i18n::tr("No")).clicked() {
                            answer = Some(false);
                        }
                    });
                });
            if let Some(true) = answer {
                self.restart_prompt = None;
                match action {
                    RestartAction::Language(code) => {
                        match code {
                            Some(c) => {
                                let _ = eapo_common::registry_helper::write_value(settings::EDITOR_REGPATH, "language", c);
                            }
                            None => {
                                let _ = eapo_common::registry_helper::delete_value(settings::EDITOR_REGPATH, "language");
                            }
                        }
                    }
                    RestartAction::ResetGlobal => {
                        GlobalPreferences::reset_global();
                        self.no_save_preferences = true;
                    }
                    RestartAction::ResetFileSpecific => {
                        GlobalPreferences::reset_file_specific();
                        self.no_save_file_preferences = true;
                    }
                }
                self.restart_flag.store(true, Ordering::SeqCst);
                // Close through the normal path so unsaved tabs are prompted,
                // exactly like MainWindow::closeEvent.
                ctx.send_viewport_cmd(egui::ViewportCommand::Close);
            } else if let Some(false) = answer {
                self.restart_prompt = None;
            }
        }
    }
}

fn menu_button(ui: &mut egui::Ui, label: &str, shortcut: &str) -> bool {
    let button = if shortcut.is_empty() {
        egui::Button::new(label)
    } else {
        egui::Button::new(label).shortcut_text(shortcut)
    };
    let clicked = ui.add(button).clicked();
    if clicked {
        ui.close_menu();
    }
    clicked
}

fn round_resolution(value: i32) -> i32 {
    let v = value.clamp(128, 8388608) as f64;
    let rounded = 2f64.powf(v.log2().round()) as i32;
    rounded.clamp(128, 8388608)
}

impl eframe::App for EditorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let size = ctx.screen_rect().size();
        if size.x > 0.0 && size.y > 0.0 {
            self.window_size = (size.x, size.y);
        }
        let modal_open = self.hotkey_dialog.open
            || self.close_prompt.is_some()
            || self.restart_prompt.is_some()
            || self.info_dialog.is_some()
            || self.about_open
            || !self.check_queue.is_empty();
        ctx.data_mut(|d| d.insert_temp(egui::Id::new("eapo_modal"), modal_open));
        if ctx.input(|i| i.viewport().close_requested()) && !self.closing {
            self.request_app_close(ctx);
        }
        self.handle_shortcuts(ctx);
        self.poll_analysis(ctx);
        self.poll_hotkeys(ctx);
        self.poll_volume();
        self.menu_bar(ctx);
        self.toolbar(ctx);
        self.analysis_panel(ctx);
        self.central(ctx);
        self.show_hotkey_dialog(ctx);
        self.show_prompts(ctx);
        if self.show_analysis {
            ctx.request_repaint_after(Duration::from_millis(100));
        }
    }
}
