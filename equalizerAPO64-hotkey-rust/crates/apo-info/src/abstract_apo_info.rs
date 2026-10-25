//! Port of `AbstractAPOInfo.h/.cpp`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use eapo_common::registry_helper::RegistryException;

/// Port of the `AbstractAPOInfo` interface.
///
/// The C++ interface uses non-const `install`/`uninstall`/`reinstall` because
/// the device implementation reloads its registry state. The Rust port keeps
/// instances shareable (`Arc<dyn AbstractAPOInfo>`) and therefore takes
/// `&self`; implementations use interior mutability. Install-state selection
/// is exposed by [`crate::DeviceAPOInfo`] through copy/set accessors.
pub trait AbstractAPOInfo: Send + Sync + Any {
    fn get_connection_name(&self) -> String;
    fn get_device_name(&self) -> String;
    fn get_device_guid(&self) -> String;
    fn get_device_string(&self) -> String;
    fn get_channel_count(&self) -> u32;
    fn get_sample_rate(&self) -> u32;
    fn get_channel_mask(&self) -> u32;
    fn is_input(&self) -> bool;
    fn is_installed(&self) -> bool;
    fn can_be_upgraded(&self) -> bool;
    fn has_changes(&self) -> bool;
    fn is_experimental(&self) -> bool;
    fn is_enhancements_disabled(&self) -> bool;
    fn is_default_device(&self) -> bool;
    fn is_disabled(&self) -> bool;
    fn is_unplugged(&self) -> bool;

    fn install(&self) -> Result<(), RegistryException>;
    fn uninstall(&self) -> Result<(), RegistryException>;
    fn reinstall(&self) -> Result<(), RegistryException>;

    /// Downcast support replacing C++ `dynamic_cast`.
    fn as_any(&self) -> &dyn Any;

    /// Downcast support for `Arc<dyn AbstractAPOInfo>` values.
    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync>;
}

/// Port of `DeviceException`, thrown by `testAPOInstallation`.
#[derive(Debug, Clone)]
pub struct DeviceException {
    message: String,
}

impl DeviceException {
    pub fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }

    /// C++ accessor name, kept for call-site parity.
    pub fn get_message(&self) -> String {
        self.message.clone()
    }

    pub fn message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for DeviceException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for DeviceException {}
