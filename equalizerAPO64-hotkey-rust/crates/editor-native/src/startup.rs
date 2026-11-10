//! Startup checks and sibling application launches.

use super::*;

pub(crate) unsafe fn launch_sibling(name: &str) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(directory) = exe.parent() else {
        return;
    };
    let mut candidates = vec![directory.join(name)];
    if let Some(parent) = directory.parent() {
        candidates.push(parent.join(name));
    }
    for candidate in candidates {
        if !candidate.exists() {
            continue;
        }
        let path = wide_z(&candidate.to_string_lossy());
        for operation in [w!("open"), w!("runas")] {
            let result = ShellExecuteW(
                None,
                operation,
                PCWSTR(path.as_ptr()),
                None,
                None,
                SW_SHOWNORMAL,
            );
            if result.0 as isize > 32 {
                return;
            }
        }
    }
}

pub(crate) unsafe fn startup_checks(hwnd: HWND) {
    if SKIP_CHECKS.load(Ordering::Relaxed) || scroll_test_enabled() {
        return;
    }
    // Without the registered configuration path the APO engine never loads a
    // configuration, so gain/EQ edits cannot affect audio.
    let config_registered = eapo_common::registry_helper::read_value(
        eapo_common::APP_REGPATH,
        "ConfigPath",
    )
    .map(|value| !value.trim().is_empty())
    .unwrap_or(false);
    if !config_registered {
        let text = wide_z(&i18n::tr(
            "Equalizer APO is not installed correctly: the configuration path is not registered, so gain and filter changes will not affect audio.\nDo you want to start the installer (Setup.exe) to repair the installation?",
        ));
        let answer = MessageBoxW(
            Some(hwnd),
            PCWSTR(text.as_ptr()),
            w!("Equalizer APO"),
            MB_YESNO | MB_ICONWARNING,
        );
        if answer.0 == IDYES.0 {
            launch_sibling("Setup.exe");
        }
        return;
    }
    let mut message: Option<String> = None;
    if !DeviceAPOInfo::check_apo_registration(false)
        || !DeviceAPOInfo::check_protected_audio_dg(false).unwrap_or(false)
    {
        message = Some(
            "A registry value required by Equalizer APO is not set correctly.\nDo you want to open the Device Selector to fix it?"
                .to_string(),
        );
    } else if let Ok(devices) = DeviceAPOInfo::load_all_infos(false) {
        if let Some(default) = devices.iter().find(|device| device.is_default_device()) {
            if !default.is_installed() {
                message = Some(
                    "Equalizer APO is not installed to the default playback device.\nDo you want to open the Device Selector?"
                        .to_string(),
                );
            } else if default.is_enhancements_disabled() {
                message = Some(
                    "Audio enhancements are disabled for the default playback device.\nDo you want to open the Device Selector?"
                        .to_string(),
                );
            }
        }
    }
    if let Some(message) = message {
        let text = wide_z(&i18n::tr(&message));
        let answer = MessageBoxW(
            Some(hwnd),
            PCWSTR(text.as_ptr()),
            w!("Equalizer APO"),
            MB_YESNO | MB_ICONWARNING,
        );
        if answer.0 == IDYES.0 {
            launch_sibling("DeviceSelector.exe");
        }
    }
}
