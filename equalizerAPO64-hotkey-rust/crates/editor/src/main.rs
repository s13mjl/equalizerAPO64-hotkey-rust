//! Equalizer APO Configuration Editor — pure Rust / egui port.
// Some helper APIs mirror the C++ classes but are not exercised by the egui UI yet.
#![allow(dead_code)]
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod analysis;
mod calibration;
mod file_dialog;
mod filter_table;
mod guis;
mod hotkey;
mod i18n;
mod hotkey_dialog;
mod keyseq;
mod main_window;
mod model;
mod plot;
mod settings;
mod vst_editor;

use std::sync::atomic::AtomicBool;
use std::sync::Arc;

fn main() -> eframe::Result<()> {
    // main.cpp creates the per-user editor keys before constructing MainWindow.
    let _ = eapo_common::registry_helper::create_key(eapo_common::USER_REGPATH);
    let _ = eapo_common::registry_helper::create_key(settings::EDITOR_REGPATH);
    let _ = eapo_common::registry_helper::create_key(settings::EDITOR_PER_FILE_REGPATH);

    let args: Vec<String> = std::env::args().skip(1).collect();
    let prefs = settings::GlobalPreferences::load();
    let size = prefs.window_size.unwrap_or((1024.0, 768.0));
    let mut viewport = eframe::egui::ViewportBuilder::default()
        .with_title(format!(
            "Equalizer APO {} Configuration Editor",
            main_window::VERSION
        ))
        .with_inner_size([size.0, size.1])
        .with_min_inner_size([640.0, 480.0]);
    if let Some((x, y)) = prefs.window_pos {
        viewport = viewport.with_position([x, y]);
    }
    let options = eframe::NativeOptions {
        viewport,
        ..Default::default()
    };

    loop {
        let restart = Arc::new(AtomicBool::new(false));
        let restart_for_app = restart.clone();
        let prefs_for_app = prefs.clone();
        let args_for_app = args.clone();
        let result = eframe::run_native(
            "Equalizer APO Configuration Editor",
            options.clone(),
            Box::new(move |cc| {
                Ok(Box::new(main_window::EditorApp::new(
                    cc,
                    args_for_app,
                    prefs_for_app,
                    restart_for_app,
                )))
            }),
        );
        if restart.load(std::sync::atomic::Ordering::SeqCst) {
            if let Ok(exe) = std::env::current_exe() {
                let _ = std::process::Command::new(exe).args(&args).spawn();
            }
            return Ok(());
        }
        return result;
    }
}
