//! Port of `DeviceAPOInfo.h/.cpp` (789 lines).
//!
//! All registry paths, value names, ordering and sentinel strings are kept
//! identical to the C++ implementation. Registry access goes through the
//! crate-private [`crate::registry`] port of `RegistryHelper`.

use std::any::Any;
use std::sync::{Arc, RwLock, RwLockReadGuard, RwLockWriteGuard};

use eapo_common::registry_helper::RegistryException;
use eapo_common::string_helper::{get_system_error_string, replace_illegal_characters};
use eapo_common::{APP_REGPATH, EQUALIZERAPO_POST_MIX_GUID, EQUALIZERAPO_PRE_MIX_GUID};
use windows::core::{GUID, HSTRING};
use windows::Win32::Media::Audio::{
    eCapture, eRender, ERole, IAudioClient, IMMDeviceEnumerator, PKEY_AudioEndpoint_GUID,
    AUDCLNT_SHAREMODE_SHARED, DEVICE_STATE_DISABLED, DEVICE_STATE_NOTPRESENT,
    DEVICE_STATE_UNPLUGGED,
};
use windows::Win32::System::Com::StructuredStorage::PropVariantClear;
use windows::Win32::System::Com::{CoCreateInstance, CoTaskMemFree, CLSCTX_ALL, STGM_READ};

use crate::abstract_apo_info::{AbstractAPOInfo, DeviceException};
use crate::registry;
use crate::voicemeeter_apo_info::VoicemeeterAPOInfo;

pub const APOGUID_NULL: &str = "{00000000-0000-0000-0000-000000000000}";
pub const APOGUID_NOKEY: &str = "!KEY";
pub const APOGUID_NOVALUE: &str = "!VALUE";

pub const INSTALL_VERSION: &str = "2";

const PROTECTED_DG_KEY_PATH: &str =
    r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Audio";
const PROTECTED_DG_VALUE_NAME: &str = "DisableProtectedAudioDG";
const APO_REGISTRATION_KEY_PATH: &str = r"HKEY_CLASSES_ROOT\AudioEngine\AudioProcessingObjects";
const CLSID_KEY_PATH: &str = r"HKEY_CLASSES_ROOT\CLSID";
const RENDER_KEY_PATH: &str =
    r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Render";
const CAPTURE_KEY_PATH: &str =
    r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\MMDevices\Audio\Capture";

fn child_apo_path() -> String {
    format!("{APP_REGPATH}\\Child APOs")
}

const PRE_MIX_CHILD_GUID_VALUE_NAME: &str = "PreMixChild";
const POST_MIX_CHILD_GUID_VALUE_NAME: &str = "PostMixChild";
const ALLOW_SILENT_BUFFER_VALUE_NAME: &str = "AllowSilentBufferModification";
const DISABLE_AUTO_ADJUST_VALUE_NAME: &str = "DisableAutomaticAdjustment";
const VERSION_VALUE_NAME: &str = "Version";
const CONNECTION_VALUE_NAME: &str = "{a45c254e-df1c-4efd-8020-67d146a850e0},2";
const DEVICE_VALUE_NAME: &str = "{b3f8fa53-0004-438e-9003-51a46e139bfc},6";
const COMBINED_DEVICE_VALUE_NAME: &str = "{b3f8fa53-0004-438e-9003-51a46e139bfc},41";
const FORMAT_VALUE_NAME: &str = "{f19f064d-082c-4e27-bc73-6882a1bb8e4c},0";
const CHANNEL_MASK_VALUE_NAME: &str = "{1da5d803-d492-4edd-8c23-e0c0ffee7f0e},3";
const LFX_GUID_VALUE_NAME: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},1";
const GFX_GUID_VALUE_NAME: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},2";
const SFX_GUID_VALUE_NAME: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},5";
const MFX_GUID_VALUE_NAME: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},6";
const EFX_GUID_VALUE_NAME: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},7";
const MULTI_SFX_GUID_VALUE_NAME: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},13";
const MULTI_MFX_GUID_VALUE_NAME: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},14";
const MULTI_EFX_GUID_VALUE_NAME: &str = "{d04e05a6-594b-4fb6-a80d-01af5eed7d1d},15";
const ALL_GUID_VALUE_NAME_COUNT: usize = 5;
const ALL_GUID_VALUE_NAMES: [&str; ALL_GUID_VALUE_NAME_COUNT] = [
    LFX_GUID_VALUE_NAME,
    GFX_GUID_VALUE_NAME,
    SFX_GUID_VALUE_NAME,
    MFX_GUID_VALUE_NAME,
    EFX_GUID_VALUE_NAME,
];
const LFX_INDEX: usize = 0;
const GFX_INDEX: usize = 1;
const SFX_INDEX: usize = 2;
const MFX_INDEX: usize = 3;
const EFX_INDEX: usize = 4;

const FX_TITLE_VALUE_NAME: &str = "{b725f130-47ef-101a-a5f1-02608c9eebac},10";
const SFX_PROCESSING_MODES_VALUE_NAME: &str = "{d3993a3f-99c2-4402-b5ec-a92a0367664b},5";
const MFX_PROCESSING_MODES_VALUE_NAME: &str = "{d3993a3f-99c2-4402-b5ec-a92a0367664b},6";
const EFX_PROCESSING_MODES_VALUE_NAME: &str = "{d3993a3f-99c2-4402-b5ec-a92a0367664b},7";
const DEFAULT_PROCESSING_MODE_VALUE: &str = "{C18E2F7E-933D-4965-B7D1-1EEF228D2AF3}";
const DISABLE_ENHANCEMENTS_VALUE_NAME: &str = "{1da5d803-d492-4edd-8c23-e0c0ffee7f0e},5";

const WAVE_FORMAT_EXTENSIBLE: u16 = 0xFFFE;

const CLSID_MMDEVICE_ENUMERATOR: GUID = GUID::from_u128(0xbcde0395_e52f_467c_8e3d_c4579291692e);

/// Port of `DeviceAPOInfo::InstallMode`.
#[repr(i32)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum InstallMode {
    #[default]
    InstallLfxGfx = 0,
    InstallSfxMfx = 1,
    InstallSfxEfx = 2,
}

pub const INSTALL_LFX_GFX: InstallMode = InstallMode::InstallLfxGfx;
pub const INSTALL_SFX_MFX: InstallMode = InstallMode::InstallSfxMfx;
pub const INSTALL_SFX_EFX: InstallMode = InstallMode::InstallSfxEfx;

/// Port of `DeviceAPOInfo::InstallState`.
///
/// The C++ `operator!=` uses `memcmp` over the struct (including padding);
/// this port compares fields, which is the intended semantics and avoids
/// reading uninitialised padding bytes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InstallState {
    pub install_pre_mix: bool,
    pub install_post_mix: bool,
    pub use_original_apo_pre_mix: bool,
    pub use_original_apo_post_mix: bool,
    pub auto_adjust: bool,
    pub install_mode: InstallMode,
    pub allow_silent_buffer_modification: bool,
}

impl Default for InstallState {
    fn default() -> Self {
        Self {
            install_pre_mix: false,
            install_post_mix: false,
            use_original_apo_pre_mix: false,
            use_original_apo_post_mix: false,
            auto_adjust: true,
            install_mode: InstallMode::InstallLfxGfx,
            allow_silent_buffer_modification: false,
        }
    }
}

#[derive(Debug, Clone)]
struct Inner {
    device_name: String,
    connection_name: String,
    device_guid: String,
    channel_count: u32,
    sample_rate: u32,
    channel_mask: u32,
    default_device: bool,
    enhancements_disabled: bool,
    disabled: bool,
    unplugged: bool,

    // used for creating child APO
    pre_mix_child_guid: String,
    post_mix_child_guid: String,

    // used for uninstallation
    original_apo_guids: [String; ALL_GUID_VALUE_NAME_COUNT],

    input: bool,
    installed: bool,
    version: String,
    current_install_state: InstallState,
    // selection in GUI
    selected_install_state: InstallState,
}

impl Default for Inner {
    fn default() -> Self {
        Self {
            device_name: String::new(),
            connection_name: String::new(),
            device_guid: String::new(),
            channel_count: 0,
            sample_rate: 0,
            channel_mask: 0,
            default_device: false,
            enhancements_disabled: false,
            disabled: false,
            unplugged: false,
            pre_mix_child_guid: String::new(),
            post_mix_child_guid: String::new(),
            original_apo_guids: std::array::from_fn(|_| APOGUID_NOKEY.to_string()),
            input: false,
            installed: false,
            version: "0".to_string(),
            current_install_state: InstallState::default(),
            selected_install_state: InstallState::default(),
        }
    }
}

/// Port of `DeviceAPOInfo`.
pub struct DeviceAPOInfo {
    inner: RwLock<Inner>,
}

impl DeviceAPOInfo {
    pub const INSTALL_LFX_GFX: InstallMode = InstallMode::InstallLfxGfx;
    pub const INSTALL_SFX_MFX: InstallMode = InstallMode::InstallSfxMfx;
    pub const INSTALL_SFX_EFX: InstallMode = InstallMode::InstallSfxEfx;

    pub fn new() -> Self {
        Self {
            inner: RwLock::new(Inner::default()),
        }
    }

    fn read(&self) -> RwLockReadGuard<'_, Inner> {
        self.inner.read().unwrap_or_else(|e| e.into_inner())
    }

    fn write(&self) -> RwLockWriteGuard<'_, Inner> {
        self.inner.write().unwrap_or_else(|e| e.into_inner())
    }

    pub fn load(&self, device_guid: &str) -> Result<bool, RegistryException> {
        self.load_with_default(device_guid, "")
    }

    /// `defaultDeviceGuid` is the C++ optional argument; an empty string means
    /// "query the current default endpoint".
    pub fn load_with_default(
        &self,
        device_guid: &str,
        default_device_guid: &str,
    ) -> Result<bool, RegistryException> {
        let render_key = format!("{RENDER_KEY_PATH}\\{device_guid}");
        let capture_key = format!("{CAPTURE_KEY_PATH}\\{device_guid}");
        let (key_path, input) = if registry::key_exists(&render_key) {
            (render_key, false)
        } else {
            (capture_key, true)
        };

        let mut inner = self.write();

        let device_state = registry::read_dword_value(&key_path, "DeviceState")?;
        if device_state & DEVICE_STATE_NOTPRESENT.0 != 0 {
            return Ok(false);
        }
        // somehow, disabled devices do not actually use DEVICE_STATE_DISABLED in registry but 0x10000000
        inner.disabled =
            device_state & DEVICE_STATE_DISABLED.0 != 0 || device_state & 0x1000_0000 != 0;
        inner.unplugged = device_state & DEVICE_STATE_UNPLUGGED.0 != 0;

        inner.device_guid = device_guid.to_string();
        inner.input = input;

        let properties_path = format!("{key_path}\\Properties");
        inner.connection_name = registry::read_value(&properties_path, CONNECTION_VALUE_NAME)?;
        inner.device_name = registry::read_value(&properties_path, DEVICE_VALUE_NAME)?;

        inner.channel_count = 0;
        inner.sample_rate = 0;
        inner.channel_mask = 0;
        if registry::value_exists(&properties_path, FORMAT_VALUE_NAME) {
            let format = registry::read_binary_value(&properties_path, FORMAT_VALUE_NAME)?;
            if format.len() >= 18 + 8 {
                // The property blob starts with an 8 byte header; the C++ code
                // casts &format[8] to WAVEFORMATEX.
                let wave_format = &format[8..];
                let format_tag = u16::from_le_bytes([wave_format[0], wave_format[1]]);
                inner.channel_count = u16::from_le_bytes([wave_format[2], wave_format[3]]) as u32;
                inner.sample_rate = u32::from_le_bytes([
                    wave_format[4],
                    wave_format[5],
                    wave_format[6],
                    wave_format[7],
                ]);
                if format_tag == WAVE_FORMAT_EXTENSIBLE && wave_format.len() >= 24 {
                    inner.channel_mask = u32::from_le_bytes([
                        wave_format[20],
                        wave_format[21],
                        wave_format[22],
                        wave_format[23],
                    ]);
                }
            }
        }
        if inner.channel_mask == 0
            && registry::value_exists(&properties_path, CHANNEL_MASK_VALUE_NAME)
        {
            inner.channel_mask =
                registry::read_dword_value(&properties_path, CHANNEL_MASK_VALUE_NAME)?;
        }

        let default_device_guid = if default_device_guid.is_empty() {
            Self::get_default_device(input)
        } else {
            default_device_guid.to_string()
        };

        inner.default_device = match (
            clsid_from_string(&inner.device_guid),
            clsid_from_string(&default_device_guid),
        ) {
            (Some(guid1), Some(guid2)) => guid1 == guid2,
            _ => false,
        };

        inner.enhancements_disabled = false;
        let fx_properties_path = format!("{key_path}\\FxProperties");
        if registry::key_exists(&fx_properties_path)
            && registry::value_exists(&fx_properties_path, DISABLE_ENHANCEMENTS_VALUE_NAME)
        {
            inner.enhancements_disabled =
                registry::read_dword_value(&fx_properties_path, DISABLE_ENHANCEMENTS_VALUE_NAME)?
                    != 0;
        }

        inner.installed = false;
        inner.current_install_state = InstallState {
            install_mode: InstallMode::InstallLfxGfx,
            install_pre_mix: true,
            install_post_mix: !input,
            use_original_apo_pre_mix: true,
            use_original_apo_post_mix: !input,
            allow_silent_buffer_modification: false,
            auto_adjust: true,
        };
        inner.version = "0".to_string();
        inner.pre_mix_child_guid = String::new();
        inner.post_mix_child_guid = String::new();

        if !registry::key_exists(&fx_properties_path) {
            for i in 0..ALL_GUID_VALUE_NAME_COUNT {
                inner.original_apo_guids[i] = APOGUID_NOKEY.to_string();
            }
        } else {
            for i in 0..ALL_GUID_VALUE_NAME_COUNT {
                if registry::value_exists(&fx_properties_path, ALL_GUID_VALUE_NAMES[i]) {
                    let mut original_apo_guid =
                        registry::read_value(&fx_properties_path, ALL_GUID_VALUE_NAMES[i])?;
                    if original_apo_guid == registry::get_guid_string(EQUALIZERAPO_PRE_MIX_GUID)
                        || original_apo_guid
                            == registry::get_guid_string(EQUALIZERAPO_POST_MIX_GUID)
                    {
                        original_apo_guid = APOGUID_NOVALUE.to_string();
                    }
                    inner.original_apo_guids[i] = original_apo_guid;
                } else {
                    inner.original_apo_guids[i] = APOGUID_NOVALUE.to_string();
                }
            }

            let mut found = false;
            let mut found_at = [false; ALL_GUID_VALUE_NAME_COUNT];
            for i in 0..ALL_GUID_VALUE_NAME_COUNT {
                found_at[i] = false;

                if registry::value_exists(&fx_properties_path, ALL_GUID_VALUE_NAMES[i]) {
                    let apo_guid_string =
                        registry::read_value(&fx_properties_path, ALL_GUID_VALUE_NAMES[i])?;

                    if let Some(apo_guid) = clsid_from_string(&apo_guid_string) {
                        if apo_guid == EQUALIZERAPO_PRE_MIX_GUID
                            || apo_guid == EQUALIZERAPO_POST_MIX_GUID
                        {
                            found_at[i] = true;
                            found = true;
                        }
                    }
                }
            }

            if found {
                inner.installed = true;

                let child_key = format!("{}\\{device_guid}", child_apo_path());
                if registry::key_exists(&child_key) {
                    if registry::value_exists(&child_key, VERSION_VALUE_NAME) {
                        inner.version = registry::read_value(&child_key, VERSION_VALUE_NAME)?;
                        if inner.version != INSTALL_VERSION {
                            return Err(RegistryException(
                                "Unsupported version of APO installation detected! Please uninstall newer Equalizer APO before using this version of Device Selector."
                                    .to_string(),
                            ));
                        }
                    } else {
                        inner.version = "1".to_string();
                    }

                    for i in 0..ALL_GUID_VALUE_NAME_COUNT {
                        if registry::value_exists(&child_key, ALL_GUID_VALUE_NAMES[i]) {
                            inner.original_apo_guids[i] =
                                registry::read_value(&child_key, ALL_GUID_VALUE_NAMES[i])?;
                        }
                    }

                    if inner.version == INSTALL_VERSION {
                        if registry::value_exists(&child_key, PRE_MIX_CHILD_GUID_VALUE_NAME) {
                            inner.pre_mix_child_guid =
                                registry::read_value(&child_key, PRE_MIX_CHILD_GUID_VALUE_NAME)?;
                        }
                        if registry::value_exists(&child_key, POST_MIX_CHILD_GUID_VALUE_NAME) {
                            inner.post_mix_child_guid =
                                registry::read_value(&child_key, POST_MIX_CHILD_GUID_VALUE_NAME)?;
                        }

                        inner.current_install_state.install_pre_mix =
                            found_at[LFX_INDEX] || found_at[SFX_INDEX];
                        inner.current_install_state.install_post_mix =
                            found_at[GFX_INDEX] || found_at[MFX_INDEX] || found_at[EFX_INDEX];

                        if inner.pre_mix_child_guid.is_empty() {
                            inner.current_install_state.use_original_apo_pre_mix = false;
                        }
                        if inner.post_mix_child_guid.is_empty() {
                            inner.current_install_state.use_original_apo_post_mix = false;
                        }

                        if found_at[LFX_INDEX] || found_at[GFX_INDEX] {
                            inner.current_install_state.install_mode = InstallMode::InstallLfxGfx;
                        } else if found_at[EFX_INDEX] {
                            inner.current_install_state.install_mode = InstallMode::InstallSfxEfx;
                        } else if found_at[SFX_INDEX] || found_at[MFX_INDEX] {
                            inner.current_install_state.install_mode = InstallMode::InstallSfxMfx;
                        }

                        if registry::value_exists(&child_key, ALLOW_SILENT_BUFFER_VALUE_NAME) {
                            inner.current_install_state.allow_silent_buffer_modification =
                                registry::read_value(&child_key, ALLOW_SILENT_BUFFER_VALUE_NAME)?
                                    != "false";
                        }
                        if registry::value_exists(&child_key, DISABLE_AUTO_ADJUST_VALUE_NAME) {
                            inner.current_install_state.auto_adjust =
                                registry::read_value(&child_key, DISABLE_AUTO_ADJUST_VALUE_NAME)?
                                    == "false";
                        }
                    } else {
                        if registry::value_exists(&child_key, LFX_GUID_VALUE_NAME) {
                            inner.pre_mix_child_guid = inner.original_apo_guids[0].clone();
                            inner.post_mix_child_guid = inner.original_apo_guids[1].clone();
                        } else {
                            inner.pre_mix_child_guid = inner.original_apo_guids[2].clone();
                            inner.post_mix_child_guid = inner.original_apo_guids[3].clone();
                        }
                    }
                } else if registry::key_exists(&child_apo_path())
                    && registry::value_exists(&child_apo_path(), device_guid)
                {
                    for i in 0..ALL_GUID_VALUE_NAME_COUNT {
                        if found_at[i] {
                            inner.original_apo_guids[i] =
                                registry::read_value(&child_apo_path(), device_guid)?;
                            if i == LFX_INDEX || i == SFX_INDEX {
                                inner.pre_mix_child_guid = inner.original_apo_guids[i].clone();
                            } else {
                                inner.post_mix_child_guid = inner.original_apo_guids[i].clone();
                            }
                            break;
                        }
                    }
                }
            } else if registry::is_windows_version_at_least(6, 3) {
                // Windows 8.1
                // only use LFX/GFX if the audio driver supplied only those APOs
                if registry::key_exists(&fx_properties_path)
                    && (registry::value_exists(&fx_properties_path, LFX_GUID_VALUE_NAME)
                        || registry::value_exists(&fx_properties_path, GFX_GUID_VALUE_NAME))
                    && !registry::value_exists(&fx_properties_path, SFX_GUID_VALUE_NAME)
                    && !registry::value_exists(&fx_properties_path, MFX_GUID_VALUE_NAME)
                    && !registry::value_exists(&fx_properties_path, EFX_GUID_VALUE_NAME)
                    && !registry::value_exists(&fx_properties_path, MULTI_SFX_GUID_VALUE_NAME)
                    && !registry::value_exists(&fx_properties_path, MULTI_MFX_GUID_VALUE_NAME)
                    && !registry::value_exists(&fx_properties_path, MULTI_EFX_GUID_VALUE_NAME)
                {
                    inner.current_install_state.install_mode = InstallMode::InstallLfxGfx;
                }
                // bluetooth devices may be combined in Windows 11, EFX will not work then
                else if registry::value_exists(&properties_path, COMBINED_DEVICE_VALUE_NAME) {
                    inner.current_install_state.install_mode = InstallMode::InstallSfxMfx;
                } else {
                    inner.current_install_state.install_mode = InstallMode::InstallSfxEfx;
                }
            }
        }

        Ok(true)
    }

    fn uninstall_impl(&self, inner: &Inner) -> Result<(), RegistryException> {
        let key_path = if !inner.input {
            format!("{RENDER_KEY_PATH}\\{}", inner.device_guid)
        } else {
            format!("{CAPTURE_KEY_PATH}\\{}", inner.device_guid)
        };
        let fx_properties_path = format!("{key_path}\\FxProperties");

        if inner.original_apo_guids[0] == APOGUID_NOKEY {
            registry::delete_key(&fx_properties_path)?;
        } else {
            for i in 0..ALL_GUID_VALUE_NAME_COUNT {
                if inner.original_apo_guids[i] == APOGUID_NOVALUE {
                    if registry::value_exists(&fx_properties_path, ALL_GUID_VALUE_NAMES[i]) {
                        registry::delete_value(&fx_properties_path, ALL_GUID_VALUE_NAMES[i])?;
                    }
                } else if !inner.original_apo_guids[i].is_empty() {
                    registry::write_value(
                        &fx_properties_path,
                        ALL_GUID_VALUE_NAMES[i],
                        &inner.original_apo_guids[i],
                    )?;
                }
            }
        }

        let child_path = child_apo_path();
        if registry::key_exists(&child_path)
            && registry::value_exists(&child_path, &inner.device_guid)
        {
            registry::delete_value(&child_path, &inner.device_guid)?;
        }

        let child_device_path = format!("{child_path}\\{}", inner.device_guid);
        if registry::key_exists(&child_device_path) {
            registry::delete_key(&child_device_path)?;
        }

        if registry::key_exists(&child_path) && registry::key_empty(&child_path)? {
            registry::delete_key(&child_path)?;
        }
        Ok(())
    }

    fn get_original_apo_pre_mix_inner(inner: &Inner) -> String {
        let mut guid = match inner.selected_install_state.install_mode {
            InstallMode::InstallLfxGfx => {
                let mut guid = inner.original_apo_guids[LFX_INDEX].clone();
                if registry::is_windows_version_at_least(6, 3) {
                    if inner.original_apo_guids[LFX_INDEX] == APOGUID_NOVALUE
                        && inner.original_apo_guids[GFX_INDEX] == APOGUID_NOVALUE
                    {
                        guid = inner.original_apo_guids[SFX_INDEX].clone();
                    }
                }
                guid
            }
            InstallMode::InstallSfxMfx => {
                let mut guid = inner.original_apo_guids[SFX_INDEX].clone();
                if inner.original_apo_guids[SFX_INDEX] == APOGUID_NOVALUE
                    && inner.original_apo_guids[MFX_INDEX] == APOGUID_NOVALUE
                {
                    guid = inner.original_apo_guids[LFX_INDEX].clone();
                }
                guid
            }
            InstallMode::InstallSfxEfx => {
                let mut guid = inner.original_apo_guids[SFX_INDEX].clone();
                if inner.original_apo_guids[SFX_INDEX] == APOGUID_NOVALUE
                    && inner.original_apo_guids[EFX_INDEX] == APOGUID_NOVALUE
                {
                    guid = inner.original_apo_guids[LFX_INDEX].clone();
                }
                guid
            }
        };

        if guid == APOGUID_NOKEY || guid == APOGUID_NOVALUE {
            guid = String::new();
        }
        guid
    }

    fn get_original_apo_post_mix_inner(inner: &Inner) -> String {
        let mut guid = match inner.selected_install_state.install_mode {
            InstallMode::InstallLfxGfx => {
                let mut guid = inner.original_apo_guids[GFX_INDEX].clone();
                if registry::is_windows_version_at_least(6, 3) {
                    if inner.original_apo_guids[LFX_INDEX] == APOGUID_NOVALUE
                        && inner.original_apo_guids[GFX_INDEX] == APOGUID_NOVALUE
                    {
                        guid = inner.original_apo_guids[MFX_INDEX].clone();
                    }
                }
                guid
            }
            InstallMode::InstallSfxMfx => {
                let mut guid = inner.original_apo_guids[MFX_INDEX].clone();
                if inner.original_apo_guids[SFX_INDEX] == APOGUID_NOVALUE
                    && inner.original_apo_guids[MFX_INDEX] == APOGUID_NOVALUE
                {
                    guid = inner.original_apo_guids[GFX_INDEX].clone();
                }
                guid
            }
            InstallMode::InstallSfxEfx => {
                let mut guid = inner.original_apo_guids[EFX_INDEX].clone();
                if inner.original_apo_guids[SFX_INDEX] == APOGUID_NOVALUE
                    && inner.original_apo_guids[EFX_INDEX] == APOGUID_NOVALUE
                {
                    guid = inner.original_apo_guids[GFX_INDEX].clone();
                }
                guid
            }
        };

        if guid == APOGUID_NOKEY || guid == APOGUID_NOVALUE {
            guid = String::new();
        }
        guid
    }

    /// Port of `getOriginalAPOPreMix` (non-const in C++, reads the selection).
    pub fn get_original_apo_pre_mix(&self) -> String {
        Self::get_original_apo_pre_mix_inner(&self.read())
    }

    /// Port of `getOriginalAPOPostMix`.
    pub fn get_original_apo_post_mix(&self) -> String {
        Self::get_original_apo_post_mix_inner(&self.read())
    }

    fn install_impl_inner(inner: &Inner) -> Result<(), RegistryException> {
        let selected = inner.selected_install_state;
        if !selected.install_pre_mix && !selected.install_post_mix {
            return Ok(());
        }

        let child_path = child_apo_path();
        let child_device_path = format!("{child_path}\\{}", inner.device_guid);
        registry::create_key(&child_path)?;
        registry::create_key(&child_device_path)?;

        let key_path = if !inner.input {
            format!("{RENDER_KEY_PATH}\\{}", inner.device_guid)
        } else {
            format!("{CAPTURE_KEY_PATH}\\{}", inner.device_guid)
        };
        let fx_properties_path = format!("{key_path}\\FxProperties");

        if !registry::key_exists(&fx_properties_path) {
            if registry::create_key(&fx_properties_path).is_err() {
                // Permissions were not sufficient, so change them
                registry::take_ownership(&key_path)?;
                registry::make_writable(&key_path)?;
                registry::create_key(&fx_properties_path)?;
            }

            registry::write_value(&fx_properties_path, FX_TITLE_VALUE_NAME, "Equalizer APO")?;

            for i in 0..ALL_GUID_VALUE_NAME_COUNT {
                registry::write_value(&child_device_path, ALL_GUID_VALUE_NAMES[i], APOGUID_NOKEY)?;
            }
        } else {
            let mut valuenames = Vec::new();

            for i in 0..ALL_GUID_VALUE_NAME_COUNT {
                let mut apo_guid_string = APOGUID_NOVALUE.to_string();
                if registry::value_exists(&fx_properties_path, ALL_GUID_VALUE_NAMES[i]) {
                    apo_guid_string =
                        registry::read_value(&fx_properties_path, ALL_GUID_VALUE_NAMES[i])?;
                    valuenames.push(ALL_GUID_VALUE_NAMES[i].to_string());
                }

                registry::write_value(
                    &child_device_path,
                    ALL_GUID_VALUE_NAMES[i],
                    &apo_guid_string,
                )?;
            }

            if !valuenames.is_empty() {
                registry::save_to_file(
                    &fx_properties_path,
                    &valuenames,
                    &format!(
                        "backup_{}_{}.reg",
                        replace_illegal_characters(&inner.device_name),
                        replace_illegal_characters(&inner.connection_name)
                    ),
                )?;
            }
        }

        let mut pre_mix_value = String::new();
        let mut post_mix_value = String::new();
        if selected.use_original_apo_pre_mix {
            pre_mix_value = Self::get_original_apo_pre_mix_inner(inner);
        }
        if selected.use_original_apo_post_mix {
            post_mix_value = Self::get_original_apo_post_mix_inner(inner);
        }
        registry::write_value(
            &child_device_path,
            PRE_MIX_CHILD_GUID_VALUE_NAME,
            &pre_mix_value,
        )?;
        registry::write_value(
            &child_device_path,
            POST_MIX_CHILD_GUID_VALUE_NAME,
            &post_mix_value,
        )?;

        registry::write_value(
            &child_device_path,
            ALLOW_SILENT_BUFFER_VALUE_NAME,
            if selected.allow_silent_buffer_modification {
                "true"
            } else {
                "false"
            },
        )?;
        if selected.auto_adjust {
            if registry::value_exists(&child_device_path, DISABLE_AUTO_ADJUST_VALUE_NAME) {
                registry::delete_value(&child_device_path, DISABLE_AUTO_ADJUST_VALUE_NAME)?;
            }
        } else {
            registry::write_value(&child_device_path, DISABLE_AUTO_ADJUST_VALUE_NAME, "true")?;
        }
        registry::write_value(&child_device_path, VERSION_VALUE_NAME, INSTALL_VERSION)?;

        match selected.install_mode {
            InstallMode::InstallLfxGfx => {
                if selected.install_pre_mix {
                    registry::write_value(
                        &fx_properties_path,
                        LFX_GUID_VALUE_NAME,
                        &registry::get_guid_string(EQUALIZERAPO_PRE_MIX_GUID),
                    )?;
                }
                if selected.install_post_mix && !inner.input {
                    registry::write_value(
                        &fx_properties_path,
                        GFX_GUID_VALUE_NAME,
                        &registry::get_guid_string(EQUALIZERAPO_POST_MIX_GUID),
                    )?;
                }
                if registry::value_exists(&fx_properties_path, SFX_GUID_VALUE_NAME) {
                    registry::delete_value(&fx_properties_path, SFX_GUID_VALUE_NAME)?;
                }
                if registry::value_exists(&fx_properties_path, MFX_GUID_VALUE_NAME) {
                    registry::delete_value(&fx_properties_path, MFX_GUID_VALUE_NAME)?;
                }
                if registry::value_exists(&fx_properties_path, EFX_GUID_VALUE_NAME) {
                    registry::delete_value(&fx_properties_path, EFX_GUID_VALUE_NAME)?;
                }
            }
            InstallMode::InstallSfxMfx => {
                if registry::value_exists(&fx_properties_path, LFX_GUID_VALUE_NAME) {
                    registry::delete_value(&fx_properties_path, LFX_GUID_VALUE_NAME)?;
                }
                if registry::value_exists(&fx_properties_path, GFX_GUID_VALUE_NAME) {
                    registry::delete_value(&fx_properties_path, GFX_GUID_VALUE_NAME)?;
                }
                if selected.install_pre_mix {
                    registry::write_value(
                        &fx_properties_path,
                        SFX_GUID_VALUE_NAME,
                        &registry::get_guid_string(EQUALIZERAPO_PRE_MIX_GUID),
                    )?;
                    if !registry::value_exists(&fx_properties_path, SFX_PROCESSING_MODES_VALUE_NAME)
                    {
                        registry::write_multi_value(
                            &fx_properties_path,
                            SFX_PROCESSING_MODES_VALUE_NAME,
                            DEFAULT_PROCESSING_MODE_VALUE,
                        )?;
                    }
                }
                if selected.install_post_mix && !inner.input {
                    registry::write_value(
                        &fx_properties_path,
                        MFX_GUID_VALUE_NAME,
                        &registry::get_guid_string(EQUALIZERAPO_POST_MIX_GUID),
                    )?;
                    if !registry::value_exists(&fx_properties_path, MFX_PROCESSING_MODES_VALUE_NAME)
                    {
                        registry::write_multi_value(
                            &fx_properties_path,
                            MFX_PROCESSING_MODES_VALUE_NAME,
                            DEFAULT_PROCESSING_MODE_VALUE,
                        )?;
                    }
                }
                // don't change efx
            }
            InstallMode::InstallSfxEfx => {
                if registry::value_exists(&fx_properties_path, LFX_GUID_VALUE_NAME) {
                    registry::delete_value(&fx_properties_path, LFX_GUID_VALUE_NAME)?;
                }
                if registry::value_exists(&fx_properties_path, GFX_GUID_VALUE_NAME) {
                    registry::delete_value(&fx_properties_path, GFX_GUID_VALUE_NAME)?;
                }
                if selected.install_pre_mix {
                    registry::write_value(
                        &fx_properties_path,
                        SFX_GUID_VALUE_NAME,
                        &registry::get_guid_string(EQUALIZERAPO_PRE_MIX_GUID),
                    )?;
                    if !registry::value_exists(&fx_properties_path, SFX_PROCESSING_MODES_VALUE_NAME)
                    {
                        registry::write_multi_value(
                            &fx_properties_path,
                            SFX_PROCESSING_MODES_VALUE_NAME,
                            DEFAULT_PROCESSING_MODE_VALUE,
                        )?;
                    }
                }
                // don't change mfx
                if selected.install_post_mix && !inner.input {
                    registry::write_value(
                        &fx_properties_path,
                        EFX_GUID_VALUE_NAME,
                        &registry::get_guid_string(EQUALIZERAPO_POST_MIX_GUID),
                    )?;
                    if !registry::value_exists(&fx_properties_path, EFX_PROCESSING_MODES_VALUE_NAME)
                    {
                        registry::write_multi_value(
                            &fx_properties_path,
                            EFX_PROCESSING_MODES_VALUE_NAME,
                            DEFAULT_PROCESSING_MODE_VALUE,
                        )?;
                    }
                }
            }
        }

        // force-enable enhancements
        if registry::value_exists(&fx_properties_path, DISABLE_ENHANCEMENTS_VALUE_NAME) {
            registry::delete_value(&fx_properties_path, DISABLE_ENHANCEMENTS_VALUE_NAME)?;
        }

        Ok(())
    }

    /// Port of `DeviceAPOInfo::loadAllInfos`.
    pub fn load_all_infos(input: bool) -> Result<Vec<Arc<dyn AbstractAPOInfo>>, RegistryException> {
        let mut result: Vec<Arc<dyn AbstractAPOInfo>> = Vec::new();

        let key_path = if input {
            CAPTURE_KEY_PATH
        } else {
            RENDER_KEY_PATH
        };
        let device_guid_strings = registry::enum_sub_keys(key_path)?;
        let default_device_guid = Self::get_default_device(input);
        for device_guid_string in device_guid_strings {
            let info = DeviceAPOInfo::new();
            if info.load_with_default(&device_guid_string, &default_device_guid)? {
                let selected = info.get_current_install_state();
                info.set_selected_install_state(selected);
                result.push(Arc::new(info));
            }
        }

        if !input {
            VoicemeeterAPOInfo::prepend_infos(&mut result)?;
        }

        Ok(result)
    }

    /// Port of `DeviceAPOInfo::getDefaultDevice(input, role = 1)`.
    pub fn get_default_device(input: bool) -> String {
        Self::get_default_device_with_role(input, 1)
    }

    pub fn get_default_device_with_role(input: bool, role: i32) -> String {
        let enumerator: IMMDeviceEnumerator =
            match unsafe { CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL) } {
                Ok(e) => e,
                Err(_) => return String::new(),
            };

        let end_point = unsafe {
            enumerator.GetDefaultAudioEndpoint(if input { eCapture } else { eRender }, ERole(role))
        };
        let Ok(end_point) = end_point else {
            return String::new();
        };

        let store = unsafe { end_point.OpenPropertyStore(STGM_READ) };
        let Ok(store) = store else {
            return String::new();
        };

        let mut variant = match unsafe { store.GetValue(&PKEY_AudioEndpoint_GUID) } {
            Ok(v) => v,
            Err(_) => return String::new(),
        };
        let result = unsafe {
            let pwsz = variant.Anonymous.Anonymous.Anonymous.pwszVal;
            if pwsz.is_null() {
                String::new()
            } else {
                pwsz.to_string().unwrap_or_default()
            }
        };
        unsafe {
            let _ = PropVariantClear(&mut variant);
        }
        result
    }

    /// Port of `DeviceAPOInfo::checkProtectedAudioDG`.
    pub fn check_protected_audio_dg(fix: bool) -> Result<bool, RegistryException> {
        let mut result = true;
        if !registry::value_exists(PROTECTED_DG_KEY_PATH, PROTECTED_DG_VALUE_NAME)
            || registry::read_dword_value(PROTECTED_DG_KEY_PATH, PROTECTED_DG_VALUE_NAME)? != 1
        {
            result = false;
            if fix {
                registry::write_dword_value(PROTECTED_DG_KEY_PATH, PROTECTED_DG_VALUE_NAME, 1)?;
            }
        }
        Ok(result)
    }

    /// Port of `DeviceAPOInfo::checkAPORegistration`. The original repairs a
    /// missing registration by invoking `regsvr32.exe /s <dir>\EqualizerAPO.dll`.
    pub fn check_apo_registration(fix: bool) -> bool {
        let pre = registry::get_guid_string(EQUALIZERAPO_PRE_MIX_GUID);
        let post = registry::get_guid_string(EQUALIZERAPO_POST_MIX_GUID);
        if !registry::key_exists(&format!("{APO_REGISTRATION_KEY_PATH}\\{pre}"))
            || !registry::key_exists(&format!("{APO_REGISTRATION_KEY_PATH}\\{post}"))
            || !registry::key_exists(&format!("{CLSID_KEY_PATH}\\{pre}"))
            || !registry::key_exists(&format!("{CLSID_KEY_PATH}\\{post}"))
        {
            if fix {
                use windows::Win32::UI::Shell::ShellExecuteW;
                use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;
                if let Some(dir) = module_directory() {
                    let params = HSTRING::from(format!("/s \"{dir}\\EqualizerAPO.dll\""));
                    unsafe {
                        ShellExecuteW(
                            None,
                            windows::core::w!("open"),
                            windows::core::w!("regsvr32.exe"),
                            &params,
                            None,
                            SW_SHOWNORMAL,
                        );
                    }
                }
            }
            return false;
        }
        true
    }

    pub fn get_current_install_state(&self) -> InstallState {
        self.read().current_install_state
    }

    pub fn get_selected_install_state(&self) -> InstallState {
        self.read().selected_install_state
    }

    pub fn set_selected_install_state(&self, state: InstallState) {
        self.write().selected_install_state = state;
    }

    /// Convenience for GUI code that mutated the C++ reference in place.
    pub fn update_selected_install_state(&self, f: impl FnOnce(&mut InstallState)) {
        f(&mut self.write().selected_install_state);
    }

    pub fn get_pre_mix_child_guid(&self) -> String {
        self.read().pre_mix_child_guid.clone()
    }

    pub fn get_post_mix_child_guid(&self) -> String {
        self.read().post_mix_child_guid.clone()
    }

    pub fn set_pre_mix_child_guid(&self, guid: &str) {
        self.write().pre_mix_child_guid = guid.to_string();
    }

    pub fn set_post_mix_child_guid(&self, guid: &str) {
        self.write().post_mix_child_guid = guid.to_string();
    }

    pub fn test_apo_installation(&self) -> Result<(), DeviceException> {
        let enumerator: IMMDeviceEnumerator =
            unsafe { CoCreateInstance(&CLSID_MMDEVICE_ENUMERATOR, None, CLSCTX_ALL) }
                .map_err(|e| self.fail("CoCreateInstance for IMMDeviceEnumerator", e))?;

        let (input, device_guid) = {
            let inner = self.read();
            (inner.input, inner.device_guid.clone())
        };
        let device_id = HSTRING::from(format!(
            "{}{}",
            if input {
                "{0.0.1.00000000}."
            } else {
                "{0.0.0.00000000}."
            },
            device_guid
        ));
        let device =
            unsafe { enumerator.GetDevice(&device_id) }.map_err(|e| self.fail("GetDevice", e))?;

        let state = unsafe { device.GetState() }.map_err(|e| self.fail("GetState", e))?;
        if state.0 & DEVICE_STATE_DISABLED.0 != 0 || state.0 & DEVICE_STATE_UNPLUGGED.0 != 0 {
            return Ok(());
        }

        let audio_client: IAudioClient =
            unsafe { device.Activate(CLSCTX_ALL, None) }.map_err(|e| self.fail("Activate", e))?;

        let format =
            unsafe { audio_client.GetMixFormat() }.map_err(|e| self.fail("GetMixFormat", e))?;

        let init_result = unsafe {
            audio_client.Initialize(
                AUDCLNT_SHAREMODE_SHARED,
                0,
                1_000_000, /* 100 ms */
                0,
                format,
                None,
            )
        };
        unsafe {
            CoTaskMemFree(Some(format as *const core::ffi::c_void));
        }
        init_result.map_err(|e| self.fail("Initialize", e))
    }

    fn fail(&self, function_name: &str, hr: windows::core::Error) -> DeviceException {
        let message = get_system_error_string(hr.code().0);
        let device_name = self.read().device_name.clone();
        DeviceException::new(format!(
            "{function_name} failed for device \"{device_name}\" ({message})"
        ))
    }
}

impl Default for DeviceAPOInfo {
    fn default() -> Self {
        Self::new()
    }
}

impl AbstractAPOInfo for DeviceAPOInfo {
    fn get_connection_name(&self) -> String {
        self.read().connection_name.clone()
    }

    fn get_device_name(&self) -> String {
        self.read().device_name.clone()
    }

    fn get_device_guid(&self) -> String {
        self.read().device_guid.clone()
    }

    fn get_device_string(&self) -> String {
        format!(
            "{} {} {}",
            self.get_connection_name(),
            self.get_device_name(),
            self.get_device_guid()
        )
    }

    fn get_channel_count(&self) -> u32 {
        self.read().channel_count
    }

    fn get_sample_rate(&self) -> u32 {
        self.read().sample_rate
    }

    fn get_channel_mask(&self) -> u32 {
        self.read().channel_mask
    }

    fn is_input(&self) -> bool {
        self.read().input
    }

    fn is_installed(&self) -> bool {
        self.read().installed
    }

    fn can_be_upgraded(&self) -> bool {
        let inner = self.read();
        inner.installed && inner.version != INSTALL_VERSION
    }

    fn has_changes(&self) -> bool {
        let inner = self.read();
        inner.installed && inner.selected_install_state != inner.current_install_state
    }

    fn is_experimental(&self) -> bool {
        let inner = self.read();
        !inner.installed && inner.original_apo_guids[0] == APOGUID_NOKEY
    }

    fn is_enhancements_disabled(&self) -> bool {
        self.read().enhancements_disabled
    }

    fn is_default_device(&self) -> bool {
        self.read().default_device
    }

    fn is_disabled(&self) -> bool {
        self.read().disabled
    }

    fn is_unplugged(&self) -> bool {
        self.read().unplugged
    }

    fn install(&self) -> Result<(), RegistryException> {
        let inner = self.read();
        Self::install_impl_inner(&inner)
    }

    fn uninstall(&self) -> Result<(), RegistryException> {
        let inner = self.read();
        self.uninstall_impl(&inner)
    }

    fn reinstall(&self) -> Result<(), RegistryException> {
        self.uninstall()?;
        let device_guid = self.read().device_guid.clone();
        self.load(&device_guid)?;
        self.install()
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn as_any_arc(self: Arc<Self>) -> Arc<dyn Any + Send + Sync> {
        self
    }
}

fn clsid_from_string(value: &str) -> Option<GUID> {
    if value.is_empty() {
        return None;
    }
    let value = HSTRING::from(value);
    unsafe { windows::Win32::System::Com::CLSIDFromString(&value).ok() }
}

fn module_directory() -> Option<String> {
    use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
    let mut buf = [0u16; 260];
    let len = unsafe { GetModuleFileNameW(None, &mut buf) } as usize;
    if len == 0 {
        return None;
    }
    let path = String::from_utf16_lossy(&buf[..len]);
    Some(match path.rfind('\\') {
        Some(pos) => path[..pos].to_string(),
        None => path,
    })
}

#[cfg(test)]
mod live_tests {
    use super::*;

    /// Read-only enumeration of the real Core Audio endpoints. Requires an
    /// interactive audio session; run with `--ignored --nocapture`.
    #[test]
    #[ignore]
    fn enumerate_real_devices_read_only() {
        use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_APARTMENTTHREADED};
        unsafe {
            let _ = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
        }
        for input in [false, true] {
            match DeviceAPOInfo::load_all_infos(input) {
                Ok(infos) => {
                    println!(
                        "{} endpoints:",
                        if input { "capture" } else { "render" }
                    );
                    for info in &infos {
                        println!(
                            "  {} | {} | {} ch @ {} Hz | installed={} input={} default={} guid={}",
                            info.get_device_name(),
                            info.get_connection_name(),
                            info.get_channel_count(),
                            info.get_sample_rate(),
                            info.is_installed(),
                            info.is_input(),
                            info.is_default_device(),
                            info.get_device_guid()
                        );
                    }
                    if !infos.is_empty() {
                        assert!(infos.iter().all(|i| !i.get_device_guid().is_empty()));
                    }
                }
                Err(error) => panic!("load_all_infos({input}) failed: {error}"),
            }
        }
        unsafe {
            CoUninitialize();
        }
    }
}
