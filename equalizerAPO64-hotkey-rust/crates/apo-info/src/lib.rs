//! Port of `AbstractAPOInfo`, `DeviceAPOInfo` and `VoicemeeterAPOInfo`.
//!
//! This crate is shared by the APO, the Editor, DeviceSelector and Setup,
//! mirroring the original "Common" project. All registry paths, APO GUIDs,
//! sentinel strings and install-state rules are identical to the C++ code.
//!
//! `eapo-common`'s `registry_helper` is still a stub in this workspace, so a
//! private (`pub(crate)`) copy of `RegistryHelper` lives in [`registry`] and
//! is used by both implementations. It reports failures through the shared
//! [`eapo_common::registry_helper::RegistryException`] type.

pub mod abstract_apo_info;
pub mod device_apo_info;
pub mod voicemeeter_apo_info;

mod registry;

pub use abstract_apo_info::{AbstractAPOInfo, DeviceException};
pub use device_apo_info::{
    DeviceAPOInfo, InstallMode, InstallState, APOGUID_NOKEY, APOGUID_NOKEY as APO_GUID_NOKEY,
    APOGUID_NOVALUE, APOGUID_NOVALUE as APO_GUID_NOVALUE, APOGUID_NULL,
    APOGUID_NULL as APO_GUID_NULL, INSTALL_LFX_GFX, INSTALL_SFX_EFX, INSTALL_SFX_MFX,
    INSTALL_VERSION,
};
pub use voicemeeter_apo_info::{VoicemeeterAPOInfo, VoicemeeterException};

/// GUID string formatter matching `StringFromCLSID` (`{XXXXXXXX-...}`,
/// uppercase).
pub fn guid_to_string(guid: windows::core::GUID) -> String {
    registry::get_guid_string(guid)
}

/// Port of `RegistryHelper::isWindowsVersionAtLeast`, used by consumers that
/// need to hide install modes on Windows < 8.1.
pub fn is_windows_version_at_least(major: u32, minor: u32) -> bool {
    registry::is_windows_version_at_least(major, minor)
}
