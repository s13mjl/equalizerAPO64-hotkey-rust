//! Port of `DeviceTestThread.{h,cpp}`.
//!
//! Restarts the audio service, asks every installed APO to test itself via the
//! `EqualizerAPODeviceTest` named pipe and, on failure, tries the remaining
//! install modes until one works (honouring the "Auto" troubleshooting flag).

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;
use std::time::{Duration, Instant};

use eapo_apo_info::{AbstractAPOInfo, DeviceAPOInfo, InstallMode};
use eapo_common::registry_helper;
use eapo_common::service_helper;
use windows::Win32::System::Com::{CoInitializeEx, CoUninitialize, COINIT_MULTITHREADED};

use crate::receive_thread::ReceiveThread;

pub const DEVICE_TEST_PIPE_NAME: &str = "EqualizerAPODeviceTest";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ItemStatus {
    Waiting,
    Success,
    Warning,
    Error,
}

pub enum TestEvent {
    Log(String),
    LogError(String),
    ShowErrorDialog(String),
    Abort(String, i32),
    SetItemStatus {
        guid: String,
        post_mix: bool,
        status: ItemStatus,
    },
    Finished,
}

#[derive(Clone, Copy)]
struct TestResult {
    pre_mix_ok: bool,
    post_mix_ok: bool,
    child_apo_pre_mix_ok: bool,
    child_apo_post_mix_ok: bool,
}

impl Default for TestResult {
    fn default() -> Self {
        Self {
            pre_mix_ok: false,
            post_mix_ok: false,
            child_apo_pre_mix_ok: true,
            child_apo_post_mix_ok: true,
        }
    }
}

impl TestResult {
    fn score(&self) -> i32 {
        let mut score = 0;
        if self.pre_mix_ok {
            score += 20;
        }
        if self.post_mix_ok {
            score += 10;
        }
        if !self.child_apo_pre_mix_ok {
            score -= 2;
        }
        if !self.child_apo_post_mix_ok {
            score -= 1;
        }
        score
    }
}

struct DeviceTestInfo {
    device_info: Arc<DeviceAPOInfo>,
    current_result: TestResult,
    remaining_install_modes: Vec<InstallMode>,
    best_install_mode: InstallMode,
    best_result: TestResult,
    wants_original_apo_pre_mix: bool,
    wants_original_apo_post_mix: bool,
}

impl DeviceTestInfo {
    fn new(device_info: Arc<DeviceAPOInfo>) -> Self {
        let selected = device_info.get_selected_install_state();
        let is_newer_windows = eapo_apo_info::is_windows_version_at_least(6, 3);

        let mut remaining_install_modes = Vec::new();
        if selected.auto_adjust {
            if is_newer_windows {
                remaining_install_modes.push(InstallMode::InstallSfxEfx);
                remaining_install_modes.push(InstallMode::InstallSfxMfx);
            }
            remaining_install_modes.push(InstallMode::InstallLfxGfx);
        } else {
            remaining_install_modes.push(selected.install_mode);
        }

        Self {
            wants_original_apo_pre_mix: selected.use_original_apo_pre_mix
                || device_info.get_original_apo_pre_mix().is_empty(),
            wants_original_apo_post_mix: selected.use_original_apo_post_mix
                || device_info.get_original_apo_post_mix().is_empty(),
            device_info,
            current_result: TestResult::default(),
            remaining_install_modes,
            best_install_mode: selected.install_mode,
            best_result: TestResult::default(),
        }
    }
}

pub struct DeviceTestThread {
    info_map: Vec<(String, DeviceTestInfo)>,
    sender: Sender<TestEvent>,
    cancel: Arc<AtomicBool>,
}

impl DeviceTestThread {
    pub fn new(
        devices: Vec<Arc<DeviceAPOInfo>>,
        sender: Sender<TestEvent>,
        cancel: Arc<AtomicBool>,
    ) -> Self {
        let mut info_map = Vec::new();
        for device in devices {
            if device.is_disabled() || device.is_unplugged() {
                continue;
            }
            let guid = device.get_device_guid().to_lowercase();
            info_map.push((guid, DeviceTestInfo::new(device)));
        }
        Self {
            info_map,
            sender,
            cancel,
        }
    }

    pub fn start(mut self) {
        std::thread::spawn(move || {
            self.run();
            let _ = self.sender.send(TestEvent::Finished);
        });
    }

    fn index_of(&self, guid: &str) -> Option<usize> {
        self.info_map.iter().position(|(key, _)| key == guid)
    }

    fn run(&mut self) {
        unsafe {
            let _ = CoInitializeEx(None, COINIT_MULTITHREADED);
        }

        let result =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| self.run_inner()));
        unsafe {
            CoUninitialize();
        }
        if result.is_err() {
            let _ = self
                .sender
                .send(TestEvent::ShowErrorDialog("Device test failed".to_string()));
        }
    }

    fn run_inner(&mut self) {
        let _ = self
            .sender
            .send(TestEvent::Log("Restarting audio service...".to_string()));
        if let Err(error) = service_helper::restart_service("AudioSrv") {
            let _ = self
                .sender
                .send(TestEvent::LogError("Restart failed.".to_string()));
            let _ = self.sender.send(TestEvent::Abort(error.to_string(), -1));
            return;
        }

        let mut remaining_devices: Vec<String> =
            self.info_map.iter().map(|(guid, _)| guid.clone()).collect();
        let mut non_working_devices = 0u32;

        while !remaining_devices.is_empty() {
            if self.cancel.load(Ordering::Relaxed) {
                break;
            }

            let _ = self
                .sender
                .send(TestEvent::Log("Checking APO installation...".to_string()));
            let pipe_name = DEVICE_TEST_PIPE_NAME;
            let receive_thread = ReceiveThread::new(pipe_name);

            let _ = registry_helper::write_value(
                eapo_common::APP_REGPATH,
                "DeviceTestPipeName",
                pipe_name,
            );

            for device_guid in remaining_devices.clone() {
                let Some(index) = self.index_of(&device_guid) else {
                    continue;
                };
                let test_info = &mut self.info_map[index].1;
                let selected = test_info.device_info.get_selected_install_state();

                if let Some(position) = test_info
                    .remaining_install_modes
                    .iter()
                    .position(|mode| *mode == selected.install_mode)
                {
                    test_info.remaining_install_modes.remove(position);
                }
                test_info.current_result.child_apo_pre_mix_ok =
                    !selected.use_original_apo_pre_mix
                        || test_info.device_info.get_original_apo_pre_mix().is_empty();
                test_info.current_result.child_apo_post_mix_ok =
                    !selected.use_original_apo_post_mix
                        || test_info.device_info.get_original_apo_post_mix().is_empty();

                if selected.install_pre_mix {
                    let _ = self.sender.send(TestEvent::SetItemStatus {
                        guid: device_guid.clone(),
                        post_mix: false,
                        status: ItemStatus::Waiting,
                    });
                }
                if selected.install_post_mix && !test_info.device_info.is_input() {
                    let _ = self.sender.send(TestEvent::SetItemStatus {
                        guid: device_guid.clone(),
                        post_mix: true,
                        status: ItemStatus::Waiting,
                    });
                }

                if let Err(error) = test_info.device_info.test_apo_installation() {
                    let _ = self
                        .sender
                        .send(TestEvent::ShowErrorDialog(error.get_message()));
                }
            }

            let deadline = Instant::now() + Duration::from_secs(3);
            loop {
                let message = match receive_thread.wait_until(deadline) {
                    Ok(message) => message,
                    Err(error) => {
                        let _ = self
                            .sender
                            .send(TestEvent::ShowErrorDialog(error.get_message()));
                        drop(receive_thread);
                        let _ = registry_helper::delete_value(
                            eapo_common::APP_REGPATH,
                            "DeviceTestPipeName",
                        );
                        return;
                    }
                };
                if message.is_empty() {
                    break;
                }

                let value: serde_json::Value = match serde_json::from_str(&message) {
                    Ok(value) => value,
                    Err(error) => {
                        let _ = self.sender.send(TestEvent::LogError(error.to_string()));
                        drop(receive_thread);
                        let _ = registry_helper::delete_value(
                            eapo_common::APP_REGPATH,
                            "DeviceTestPipeName",
                        );
                        return;
                    }
                };

                let device_guid = value
                    .get("deviceGuid")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default()
                    .to_string();
                let stage = value
                    .get("stage")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();
                let phase = value
                    .get("phase")
                    .and_then(|value| value.as_str())
                    .unwrap_or_default();

                let Some(index) = self.index_of(&device_guid.to_lowercase()) else {
                    let _ = self.sender.send(TestEvent::LogError(format!(
                        "Received unknown device GUID {}.",
                        device_guid
                    )));
                    drop(receive_thread);
                    let _ = registry_helper::delete_value(
                        eapo_common::APP_REGPATH,
                        "DeviceTestPipeName",
                    );
                    return;
                };

                let test_info = &mut self.info_map[index].1;
                let install_state = test_info.device_info.get_selected_install_state();
                let result = &mut test_info.current_result;

                if stage == "PreMix" {
                    if phase == "Initialize" {
                        result.pre_mix_ok = true;
                    } else if phase == "ChildAPO" {
                        result.child_apo_pre_mix_ok = true;
                    }
                    if result.pre_mix_ok && result.child_apo_pre_mix_ok {
                        let _ = self.sender.send(TestEvent::SetItemStatus {
                            guid: device_guid.clone(),
                            post_mix: false,
                            status: ItemStatus::Success,
                        });
                    }
                } else if stage == "PostMix" {
                    if phase == "Initialize" {
                        result.post_mix_ok = true;
                    } else if phase == "ChildAPO" {
                        result.child_apo_post_mix_ok = true;
                    }
                    if result.post_mix_ok && result.child_apo_post_mix_ok {
                        let _ = self.sender.send(TestEvent::SetItemStatus {
                            guid: device_guid.clone(),
                            post_mix: true,
                            status: ItemStatus::Success,
                        });
                    }
                }

                let all_pre = (result.pre_mix_ok && result.child_apo_pre_mix_ok)
                    || !install_state.install_pre_mix;
                let all_post = (result.post_mix_ok && result.child_apo_post_mix_ok)
                    || !install_state.install_post_mix
                    || test_info.device_info.is_input();
                if all_pre && all_post {
                    let lower_guid = device_guid.to_lowercase();
                    remaining_devices.retain(|guid| guid != &lower_guid);
                    if remaining_devices.is_empty() {
                        break;
                    }
                }
            }

            if !remaining_devices.is_empty() {
                let _ = self.sender.send(TestEvent::LogError(format!(
                    "Check failed for {} device(s).",
                    remaining_devices.len()
                )));

                let mut index = 0;
                while index < remaining_devices.len() {
                    let device_guid = remaining_devices[index].clone();
                    let Some(info_index) = self.index_of(&device_guid) else {
                        index += 1;
                        continue;
                    };
                    let test_info = &mut self.info_map[info_index].1;
                    let install_state = test_info.device_info.get_selected_install_state();

                    if test_info.current_result.score() > test_info.best_result.score() {
                        test_info.best_install_mode = install_state.install_mode;
                        test_info.best_result = test_info.current_result;
                    }

                    let (install_mode, result, remove) =
                        if let Some(first) = test_info.remaining_install_modes.first() {
                            (*first, test_info.current_result, false)
                        } else {
                            (test_info.best_install_mode, test_info.best_result, true)
                        };

                    if install_state.install_pre_mix {
                        let status = if result.pre_mix_ok {
                            if result.child_apo_pre_mix_ok
                                || !install_state.use_original_apo_pre_mix
                            {
                                ItemStatus::Success
                            } else {
                                ItemStatus::Warning
                            }
                        } else {
                            ItemStatus::Error
                        };
                        let _ = self.sender.send(TestEvent::SetItemStatus {
                            guid: device_guid.clone(),
                            post_mix: false,
                            status,
                        });
                    }
                    if install_state.install_post_mix && !test_info.device_info.is_input() {
                        let status = if result.post_mix_ok {
                            if result.child_apo_post_mix_ok
                                || !install_state.use_original_apo_post_mix
                            {
                                ItemStatus::Success
                            } else {
                                ItemStatus::Warning
                            }
                        } else {
                            ItemStatus::Error
                        };
                        let _ = self.sender.send(TestEvent::SetItemStatus {
                            guid: device_guid.clone(),
                            post_mix: true,
                            status,
                        });
                    }

                    let install_mode_name = match install_mode {
                        InstallMode::InstallLfxGfx => "LFX/GFX",
                        InstallMode::InstallSfxMfx => "SFX/MFX",
                        InstallMode::InstallSfxEfx => "SFX/EFX",
                    };
                    let _ = self.sender.send(TestEvent::Log(format!(
                        "Setting install mode for {} {} to {}.",
                        test_info.device_info.get_device_name(),
                        test_info.device_info.get_connection_name(),
                        install_mode_name
                    )));

                    // The original assigns the new install mode first and then
                    // queries the original APO GUIDs, which depend on the
                    // selected install mode.
                    test_info
                        .device_info
                        .update_selected_install_state(|selected| {
                            selected.install_mode = install_mode;
                        });
                    let use_original_pre = test_info.wants_original_apo_pre_mix
                        && !test_info.device_info.get_original_apo_pre_mix().is_empty();
                    let use_original_post = test_info.wants_original_apo_post_mix
                        && !test_info.device_info.get_original_apo_post_mix().is_empty();
                    test_info
                        .device_info
                        .update_selected_install_state(|selected| {
                            selected.use_original_apo_pre_mix = use_original_pre;
                            selected.use_original_apo_post_mix = use_original_post;
                        });
                    if let Err(error) = test_info.device_info.reinstall() {
                        let _ = self
                            .sender
                            .send(TestEvent::ShowErrorDialog(error.to_string()));
                    }
                    test_info.current_result = TestResult::default();

                    if remove {
                        remaining_devices.remove(index);
                        non_working_devices += 1;
                    } else {
                        index += 1;
                    }
                }

                if !remaining_devices.is_empty() {
                    let _ = self
                        .sender
                        .send(TestEvent::Log("Trying other configurations...".to_string()));
                }

                let _ = self
                    .sender
                    .send(TestEvent::Log("Restarting audio service...".to_string()));
                if let Err(error) = service_helper::restart_service("AudioSrv") {
                    let _ = self
                        .sender
                        .send(TestEvent::LogError("Restart failed.".to_string()));
                    let _ = self.sender.send(TestEvent::Abort(error.to_string(), -1));
                    drop(receive_thread);
                    let _ = registry_helper::delete_value(
                        eapo_common::APP_REGPATH,
                        "DeviceTestPipeName",
                    );
                    return;
                }
            }

            drop(receive_thread);
            let _ = registry_helper::delete_value(
                eapo_common::APP_REGPATH,
                "DeviceTestPipeName",
            );
        }

        if non_working_devices == 0 {
            let _ = self.sender.send(TestEvent::Log(
                "<b>Checks done. No problems were detected.</b>".to_string(),
            ));
        } else {
            let _ = self.sender.send(TestEvent::LogError(format!(
                "<b>Checks done. Problems were detected for {} device(s).</b>",
                non_working_devices
            )));
        }
    }
}
