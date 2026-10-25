//! Port of the Equalizer APO `DeviceSelector` application.
//!
//! `DeviceSelector /u` uninstalls the APO from all playback and capture
//! devices; without arguments the device selection dialog is shown. The
//! dialog supports install/uninstall/reinstall, the troubleshooting install
//! options and the device test that validates the APO installation through a
//! named pipe.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod i18n;
mod receive_thread;
mod selector;
mod test_dialog;
mod test_thread;

use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::Arc;

use eapo_apo_info::{DeviceAPOInfo, VoicemeeterAPOInfo};
use windows::core::HSTRING;
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_OK};

use selector::SelectorApp;

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
    let args: Vec<String> = std::env::args().skip(1).collect();

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

    let exit_result = Arc::new(AtomicI32::new(0));
    let title = format!("Equalizer APO {} Device Selector", version_string());
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(&title)
            .with_inner_size([582.0, 434.0])
            .with_min_inner_size([520.0, 360.0])
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
