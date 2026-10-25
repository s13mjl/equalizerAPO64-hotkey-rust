//! Helpers ported from the original `helpers/` directory.
//!
//! Windows-specific functionality is implemented through the `windows` crate;
//! everything else is plain Rust. Wide strings of the C++ implementation map
//! to Rust `String` (UTF-8) at crate boundaries.

pub mod abstract_library;
pub mod channel_helper;
#[cfg(feature = "gui")]
pub mod egui_fonts;
pub mod gain_iterator;
pub mod log_helper;
pub mod memory_helper;
pub mod precision_timer;
pub mod registry_helper;
pub mod scope_guard;
pub mod service_helper;
pub mod string_helper;
pub mod task_scheduler_helper;
pub mod vst;

pub use windows::core::GUID;

pub const APP_REGPATH: &str = r"HKEY_LOCAL_MACHINE\SOFTWARE\EqualizerAPO";
pub const USER_REGPATH: &str = r"HKEY_CURRENT_USER\SOFTWARE\EqualizerAPO";

// {EACD2258-FCAC-4FF4-B36D-419E924A6D79}
pub const EQUALIZERAPO_PRE_MIX_GUID: GUID = GUID::from_u128(0xeacd2258_fcac_4ff4_b36d_419e924a6d79);
// {EC1CC9CE-FAED-4822-828A-82A81A6F018F}
pub const EQUALIZERAPO_POST_MIX_GUID: GUID =
    GUID::from_u128(0xec1cc9ce_faed_4822_828a_82a81a6f018f);
