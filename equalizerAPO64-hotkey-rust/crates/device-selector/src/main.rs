//! Port of the Equalizer APO `DeviceSelector` application.
//!
//! `DeviceSelector /u` uninstalls the APO from all playback and capture
//! devices; without arguments the device selection dialog is shown. The
//! dialog supports install/uninstall/reinstall, the troubleshooting install
//! options and the device test that validates the APO installation through a
//! named pipe.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod elevation;
mod i18n;
mod receive_thread;
mod selector;
mod test_dialog;
mod test_thread;

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use eapo_apo_info::{DeviceAPOInfo, VoicemeeterAPOInfo};
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::{
    MessageBoxW, IDYES, MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_YESNO, SW_SHOWNORMAL,
};

use selector::SelectorApp;

fn wide_z(text: &str) -> Vec<u16> {
    text.encode_utf16().chain(std::iter::once(0)).collect()
}

fn launch_setup() {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(directory) = exe.parent() else {
        return;
    };
    let mut candidates = vec![directory.join("Setup.exe")];
    if let Some(parent) = directory.parent() {
        candidates.push(parent.join("Setup.exe"));
    }
    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }
        let file = wide_z(&candidate.to_string_lossy());
        let result = unsafe {
            ShellExecuteW(
                None,
                w!("open"),
                PCWSTR(file.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            )
        };
        if result.0 as isize > 32 {
            return;
        }
    }
}

fn version_string() -> String {
    let version = env!("CARGO_PKG_VERSION");
    if let Some(stripped) = version.strip_suffix(".0") {
        stripped.to_string()
    } else {
        version.to_string()
    }
}

fn main() {
    i18n::set_language(i18n::system_language());

    // Registering the APO and restarting the audio service require
    // administrator rights. The C++ project ships this executable with a
    // `requireAdministrator` manifest; build.rs embeds the same manifest and
    // this is the fallback for launch paths that ignore it.
    const ELEVATED_MARKER: &str = "--elevated";
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    let relaunched = args.iter().any(|argument| argument == ELEVATED_MARKER);
    args.retain(|argument| argument != ELEVATED_MARKER);
    if !relaunched && !elevation::is_elevated() {
        if let Ok(executable) = std::env::current_exe() {
            let mut relaunch_args = args.clone();
            relaunch_args.push(ELEVATED_MARKER.to_string());
            if elevation::relaunch_elevated(&executable, &relaunch_args).is_ok() {
                return;
            }
        }
        // Elevation was declined; without it the device configuration cannot
        // be changed, so do not start the UI.
        return;
    }

    if args.iter().any(|argument| argument == "/u") {
        let mut result = 0;
        for index in 0..=1 {
            match DeviceAPOInfo::load_all_infos(index == 1) {
                Ok(infos) => {
                    for info in infos {
                        if info.is_installed() {
                            if let Err(error) = info.uninstall() {
                                show_error_message(&error.to_string());
                                result = -1;
                            }
                        }
                    }
                }
                Err(error) => {
                    show_error_message(&error.to_string());
                    result = -1;
                }
            }
        }

        let _ = VoicemeeterAPOInfo::ensure_voicemeeter_client_running();
        std::process::exit(result);
    }

    // DeviceSelector only attaches the APO to audio endpoints. The program
    // itself (files + registered ConfigPath) is installed by Setup.exe; without
    // it nothing can process audio, so point the user there instead of showing a
    // selection UI that cannot have any effect.
    let config_registered = eapo_common::registry_helper::read_value(
        eapo_common::APP_REGPATH,
        "ConfigPath",
    )
    .map(|value| !value.trim().is_empty())
    .unwrap_or(false);
    if !config_registered {
        let text = wide_z(
            "Equalizer APO is not installed or its configuration path is not registered.
Please run Setup.exe first.

Do you want to start Setup.exe now?",
        );
        let title = wide_z("Equalizer APO Device Selector");
        let answer = unsafe {
            MessageBoxW(
                None,
                PCWSTR(text.as_ptr()),
                PCWSTR(title.as_ptr()),
                MB_YESNO | MB_ICONWARNING,
            )
        };
        if answer.0 == IDYES.0 {
            launch_setup();
        }
        std::process::exit(1);
    }

    let exit_result = Arc::new(AtomicI32::new(0));
    let title = format!("Equalizer APO {} Device Selector", version_string());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size([880.0, 620.0])
            .with_min_inner_size([720.0, 520.0])
            .with_resizable(true),
        ..Default::default()
    };

    let app_exit = exit_result.clone();
    let app_args = args.clone();
    let _ = eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            eapo_common::egui_fonts::install_cjk_font(&cc.egui_ctx);
            Ok(Box::new(SelectorApp::new(app_args, app_exit)))
        }),
    );

    std::process::exit(exit_result.load(Ordering::SeqCst));
}

impl eframe::App for SelectorApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        SelectorApp::update(self, ctx);
    }
}

fn show_error_message(message: &str) {
    let text = HSTRING::from(message);
    let title = HSTRING::from(crate::i18n::tr("Error while accessing the registry"));
    unsafe {
        MessageBoxW(None, &text, &title, MB_OK | MB_ICONERROR);
    }
}
