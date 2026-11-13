//! Shared hotkey manager plus the eframe repaint hook.

pub use eapo_editor_core::hotkey::*;

/// Wakes an idle eframe event loop when a global hotkey fires.
pub trait RepaintContext {
    fn set_repaint_context(&self, ctx: egui::Context);
}

impl RepaintContext for HotkeyManager {
    fn set_repaint_context(&self, ctx: egui::Context) {
        self.set_wake_callback(std::sync::Arc::new(move || ctx.request_repaint()));
    }
}
