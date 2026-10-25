//! Equalizer APO Setup / Uninstall binary entry point.
//!
//! The implementation lives in `lib.rs`; this file is kept minimal so the
//! requireAdministrator manifest can be applied only to the Setup binary.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    eapo_wizard::run();
}
