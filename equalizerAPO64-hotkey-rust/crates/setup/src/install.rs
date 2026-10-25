//! Installation logic, in the same order as the `-Install` section of
//! `Setup.nsi`.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use eapo_apo_info::DeviceAPOInfo;
use eapo_common::task_scheduler_helper::{schedule_at_logon, unschedule};

use crate::files;
use crate::lang::Language;
use crate::registry::{
    RegistryStore, WinRegistry, HKLM_AUDIO_REGPATH, HKLM_REGPATH, HKLM_UNINST_REGPATH,
};
use crate::shortcuts;
use crate::util::{
    delete_reboot_ok, grant_users_full_access, program_files_dir, run_and_wait,
    start_menu_programs_dir, system_directory,
};
use crate::version::{display_version, APP_NAME};

pub const UPDATE_TASK_NAME: &str = "EqualizerAPOUpdateChecker";

#[derive(Debug, Clone)]
pub struct InstallOptions {
    pub install_dir: PathBuf,
    pub start_menu_folder: String,
    pub old_start_menu_folder: String,
    pub check_updates: bool,
    pub staging: PathBuf,
    pub current_exe: PathBuf,
    pub language: Language,
}

#[derive(Debug, Clone, Default)]
pub struct InstallResult {
    pub success: bool,
    pub cancelled: bool,
    pub reboot: bool,
    pub error: Option<String>,
}

impl InstallResult {
    pub fn failed(error: impl Into<String>) -> Self {
        Self {
            success: false,
            cancelled: false,
            reboot: false,
            error: Some(error.into()),
        }
    }

    pub fn cancelled(language: Language) -> Self {
        Self {
            success: false,
            cancelled: true,
            reboot: false,
            error: Some(language.installation_aborted().to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum InstallMessage {
    Progress { fraction: f32, text: String },
    Log(String),
    Finished(InstallResult),
}

struct Reporter {
    sender: Sender<InstallMessage>,
    cancel: Arc<AtomicBool>,
}

impl Reporter {
    fn progress(&self, fraction: f32, text: &str) {
        let _ = self.sender.send(InstallMessage::Progress {
            fraction: fraction.clamp(0.0, 1.0),
            text: text.to_string(),
        });
    }

    fn log(&self, message: impl Into<String>) {
        let _ = self.sender.send(InstallMessage::Log(message.into()));
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

/// Runs the whole installation.  Intended to be called on a worker thread.
pub fn run_install(
    options: InstallOptions,
    sender: Sender<InstallMessage>,
    cancel: Arc<AtomicBool>,
) {
    let reporter = Reporter { sender, cancel };
    let result = install_impl(&options, &reporter);
    let _ = reporter.sender.send(InstallMessage::Finished(result));
}

fn install_impl(options: &InstallOptions, reporter: &Reporter) -> InstallResult {
    let language = options.language;
    let install_dir = &options.install_dir;

    reporter.progress(0.02, language.preparing());
    reporter.log(format!("Install path: {}", install_dir.display()));
    reporter.log(format!("Staging path: {}", options.staging.display()));

    let missing = files::missing_required_files(&options.staging);
    if !missing.is_empty() {
        let mut message = format!("{}\n\n{}", language.staging_missing(), missing.join("\n"));
        message.push_str(&format!("\n\nStaging: {}", options.staging.display()));
        return InstallResult::failed(message);
    }

    if let Err(error) = fs::create_dir_all(install_dir) {
        return InstallResult::failed(format!(
            "Could not create {}: {error}",
            install_dir.display()
        ));
    }

    let programs = start_menu_programs_dir();

    // Possibly remove files from previous installation (old Start Menu folder).
    if !options.old_start_menu_folder.is_empty() {
        reporter.log(format!(
            "Remove start menu folder: {}",
            options.old_start_menu_folder
        ));
        shortcuts::remove_start_menu_folder(&programs, &options.old_start_menu_folder);
    }
    if reporter.cancelled() {
        return InstallResult::cancelled(language);
    }

    files::delete_legacy_files(install_dir, &mut |message| reporter.log(message));
    reporter.progress(0.08, language.installing_files());

    // Rename before delete as these files may be in use.
    let mut renamed =
        files::rename_files_for_deletion(install_dir, &mut |message| reporter.log(message));
    if reporter.cancelled() {
        return InstallResult::cancelled(language);
    }

    reporter.progress(0.18, language.installing_files());
    if let Err(error) = files::copy_application_files(
        &options.staging,
        install_dir,
        &mut renamed,
        &mut |message| reporter.log(message),
    ) {
        return InstallResult::failed(error);
    }
    if reporter.cancelled() {
        return InstallResult::cancelled(language);
    }

    reporter.progress(0.45, language.installing_files());
    if let Err(error) = files::copy_url_files(&options.staging, install_dir, &mut |message| {
        reporter.log(message)
    }) {
        return InstallResult::failed(error);
    }
    let config_dir =
        match files::create_default_config(&options.staging, install_dir, &mut |message| {
            reporter.log(message)
        }) {
            Ok(directory) => directory,
            Err(error) => return InstallResult::failed(error),
        };
    if let Err(error) = files::create_vst_plugins_dir(install_dir) {
        return InstallResult::failed(error);
    }

    // Grant write access to the config directory for all users.
    if let Err(error) = grant_users_full_access(&config_dir) {
        reporter.log(format!("Warning: could not grant config access: {error}"));
    } else {
        reporter.log(format!("Granted FullAccess to {}", config_dir.display()));
    }
    if reporter.cancelled() {
        return InstallResult::cancelled(language);
    }

    reporter.progress(0.58, language.installing_files());
    let registry = WinRegistry::new();
    let old_install_dir = registry
        .read_string(HKLM_REGPATH, "InstallPath")
        .unwrap_or_default();
    if let Err(error) = write_install_registry(
        &registry,
        install_dir,
        &old_install_dir,
        &options.start_menu_folder,
    ) {
        return InstallResult::failed(error);
    }

    let uninstall_exe = match files::write_uninstaller(
        &options.current_exe,
        install_dir,
        &mut renamed,
        &mut |message| reporter.log(message),
    ) {
        Ok(path) => path,
        Err(error) => return InstallResult::failed(error),
    };

    reporter.progress(0.66, language.installing_files());
    if let Err(error) =
        shortcuts::create_start_menu_shortcuts(&programs, &options.start_menu_folder, install_dir)
    {
        return InstallResult::failed(error);
    }
    reporter.log(format!("Uninstaller: {}", uninstall_exe.display()));

    if let Err(error) = write_uninstall_registry(&registry, install_dir) {
        return InstallResult::failed(error);
    }
    if let Err(error) = registry.write_dword(HKLM_AUDIO_REGPATH, "DisableProtectedAudioDG", 1) {
        return InstallResult::failed(error);
    }
    if reporter.cancelled() {
        return InstallResult::cancelled(language);
    }

    // RegDLL doesn't work for 64 bit dlls (NSIS comment).
    reporter.progress(0.76, language.installing_files());
    let dll = install_dir.join("EqualizerAPO.dll");
    let regsvr32 = system_directory().join("regsvr32.exe");
    let regsvr_args: [&OsStr; 2] = [OsStr::new("/s"), dll.as_os_str()];
    match run_and_wait(&regsvr32, &regsvr_args, Some(install_dir), true) {
        Ok(status) => reporter.log(format!(
            "regsvr32 /s {} exited with {}",
            dll.display(),
            status
        )),
        Err(error) => reporter.log(format!("Warning: could not run regsvr32.exe: {error}")),
    }

    // Verify the APO registration and protected-audio flag through the shared
    // APO info crate.  check_apo_registration(true) is not used here because
    // it resolves EqualizerAPO.dll relative to Setup.exe, not $INSTDIR.
    {
        let _com = crate::util::com_initialize_sta();
        let registered = DeviceAPOInfo::check_apo_registration(false);
        reporter.log(format!(
            "APO registration check: {}",
            if registered { "registered" } else { "missing" }
        ));
        match DeviceAPOInfo::check_protected_audio_dg(false) {
            Ok(true) => reporter.log("DisableProtectedAudioDG is set"),
            Ok(false) => reporter.log("Warning: DisableProtectedAudioDG is not set"),
            Err(error) => reporter.log(format!("Warning: {error}")),
        }
        match DeviceAPOInfo::load_all_infos(false) {
            Ok(infos) => {
                let installed = infos.iter().filter(|info| info.is_installed()).count();
                reporter.log(format!(
                    "Audio devices: {} render endpoints, {} with Equalizer APO",
                    infos.len(),
                    installed
                ));
            }
            Err(error) => reporter.log(format!(
                "Warning: could not enumerate audio devices: {error}"
            )),
        }
    }
    if reporter.cancelled() {
        return InstallResult::cancelled(language);
    }

    // NSIS launches the Device Selector UI in installer mode and uses its exit
    // code for the reboot flag.
    reporter.progress(0.86, language.installing_files());
    let device_selector = install_dir.join("DeviceSelector.exe");
    let reboot = match run_and_wait(
        &device_selector,
        &[OsStr::new("/i")],
        Some(install_dir),
        false,
    ) {
        Ok(status) => {
            reporter.log(format!("DeviceSelector.exe /i exited with {status}"));
            !status.success()
        }
        Err(error) => {
            reporter.log(format!(
                "Warning: could not start DeviceSelector.exe: {error}"
            ));
            true
        }
    };

    // Check the outcome of DeviceSelector /i through the shared APO info
    // crate.  A stub DeviceSelector leaves every device uninstalled; surface
    // that as a warning instead of claiming success silently.
    {
        let _com = crate::util::com_initialize_sta();
        match DeviceAPOInfo::load_all_infos(false) {
            Ok(infos) => {
                let installed = infos.iter().filter(|info| info.is_installed()).count();
                reporter.log(format!(
                    "After DeviceSelector: {} render endpoints, {} with Equalizer APO",
                    infos.len(),
                    installed
                ));
                if installed == 0 {
                    reporter.log(
                        "Warning: no audio device has Equalizer APO installed. Install it later from the Device Selector.",
                    );
                }
            }
            Err(error) => reporter.log(format!(
                "Warning: could not verify APO installation: {error}"
            )),
        }
    }

    // UpdateChecker -i/-u, exactly like Setup.nsi.  Additionally call the
    // TaskSchedulerHelper equivalent so the task is correct even if the
    // UpdateChecker binary is a stub or fails silently.
    let update_checker = install_dir.join("UpdateChecker.exe");
    let update_arg = if options.check_updates { "-i" } else { "-u" };
    match run_and_wait(
        &update_checker,
        &[OsStr::new(update_arg)],
        Some(install_dir),
        false,
    ) {
        Ok(status) => reporter.log(format!(
            "UpdateChecker.exe {update_arg} exited with {status}"
        )),
        Err(error) => reporter.log(format!(
            "Warning: could not start UpdateChecker.exe: {error}"
        )),
    }
    if options.check_updates {
        let program = update_checker.to_string_lossy().to_string();
        let working_dir = install_dir.to_string_lossy().to_string();
        match schedule_at_logon(UPDATE_TASK_NAME, &program, "-a", &working_dir) {
            Ok(()) => reporter.log(format!("Scheduled task {UPDATE_TASK_NAME}")),
            Err(error) => reporter.log(format!("Warning: could not schedule update task: {error}")),
        }
    } else {
        match unschedule(UPDATE_TASK_NAME) {
            Ok(()) => reporter.log(format!("Removed scheduled task {UPDATE_TASK_NAME}")),
            Err(error) => reporter.log(format!("Warning: could not remove update task: {error}")),
        }
    }

    // Hopefully the renamed files can be deleted without reboot after the
    // Device Selector has restarted the audio service.
    for path in &renamed {
        if !delete_reboot_ok(path) {
            reporter.log(format!("Warning: could not delete {}", path.display()));
        }
    }

    reporter.progress(1.0, language.done());
    reporter.log("Installation complete.");
    InstallResult {
        success: true,
        cancelled: false,
        reboot,
        error: None,
    }
}

/// Registry values written by the `-Install` section before the shortcuts.
pub fn write_install_registry(
    store: &dyn RegistryStore,
    install_dir: &Path,
    old_install_dir: &str,
    start_menu_folder: &str,
) -> Result<(), String> {
    let install = install_dir.to_string_lossy().to_string();
    store.write_string(HKLM_REGPATH, "InstallPath", &install)?;

    // Write ConfigPath if non-existing or if InstallPath has changed.
    let config_path = store
        .read_string(HKLM_REGPATH, "ConfigPath")
        .unwrap_or_default();
    if config_path.is_empty() || install != old_install_dir {
        store.write_string(
            HKLM_REGPATH,
            "ConfigPath",
            &install_dir.join("config").to_string_lossy(),
        )?;
    }

    let enable_trace = store
        .read_string(HKLM_REGPATH, "EnableTrace")
        .unwrap_or_default();
    if enable_trace.is_empty() {
        store.write_string(HKLM_REGPATH, "EnableTrace", "false")?;
    }

    store.write_string(HKLM_REGPATH, "Start Menu Folder", start_menu_folder)?;
    Ok(())
}

/// `HKLM\Software\Microsoft\Windows\CurrentVersion\Uninstall\EqualizerAPO`.
pub fn write_uninstall_registry(
    store: &dyn RegistryStore,
    install_dir: &Path,
) -> Result<(), String> {
    store.write_string(HKLM_UNINST_REGPATH, "DisplayName", APP_NAME)?;
    store.write_string(HKLM_UNINST_REGPATH, "DisplayVersion", &display_version())?;
    store.write_string(
        HKLM_UNINST_REGPATH,
        "UninstallString",
        &uninstall_string(install_dir),
    )?;
    store.write_dword(HKLM_UNINST_REGPATH, "NoModify", 1)?;
    store.write_dword(HKLM_UNINST_REGPATH, "NoRepair", 1)?;
    Ok(())
}

/// NSIS: `WriteRegStr ... "UninstallString" '"$INSTDIR\Uninstall.exe"'`.
pub fn uninstall_string(install_dir: &Path) -> String {
    format!("\"{}\"", install_dir.join("Uninstall.exe").display())
}

/// Default install folder used by `Setup.nsi`.
pub fn default_install_dir() -> PathBuf {
    program_files_dir().join("EqualizerAPO")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::testing::MockRegistry;

    #[test]
    fn default_folder_is_program_files_equalizerapo() {
        assert!(default_install_dir().ends_with("EqualizerAPO"));
    }

    #[test]
    fn uninstall_string_is_quoted() {
        assert_eq!(
            uninstall_string(Path::new(r"C:\Program Files\EqualizerAPO")),
            r#""C:\Program Files\EqualizerAPO\Uninstall.exe""#
        );
    }

    #[test]
    fn fresh_install_writes_config_path_and_enable_trace() {
        let registry = MockRegistry::default();
        write_install_registry(
            &registry,
            Path::new(r"C:\Program Files\EqualizerAPO"),
            "",
            "Equalizer APO 1.4.2",
        )
        .unwrap();

        assert_eq!(
            registry.read_string(HKLM_REGPATH, "InstallPath").as_deref(),
            Some(r"C:\Program Files\EqualizerAPO")
        );
        assert_eq!(
            registry.read_string(HKLM_REGPATH, "ConfigPath").as_deref(),
            Some(r"C:\Program Files\EqualizerAPO\config")
        );
        assert_eq!(
            registry.read_string(HKLM_REGPATH, "EnableTrace").as_deref(),
            Some("false")
        );
        assert_eq!(
            registry
                .read_string(HKLM_REGPATH, "Start Menu Folder")
                .as_deref(),
            Some("Equalizer APO 1.4.2")
        );
    }

    #[test]
    fn unchanged_install_keeps_custom_config_path_and_trace_setting() {
        let registry = MockRegistry::default();
        registry.set_string(
            HKLM_REGPATH,
            "InstallPath",
            r"C:\Program Files\EqualizerAPO",
        );
        registry.set_string(HKLM_REGPATH, "ConfigPath", r"D:\MyAPOConfig");
        registry.set_string(HKLM_REGPATH, "EnableTrace", "true");

        write_install_registry(
            &registry,
            Path::new(r"C:\Program Files\EqualizerAPO"),
            r"C:\Program Files\EqualizerAPO",
            "Equalizer APO 1.4.2",
        )
        .unwrap();

        assert_eq!(
            registry.read_string(HKLM_REGPATH, "ConfigPath").as_deref(),
            Some(r"D:\MyAPOConfig")
        );
        assert_eq!(
            registry.read_string(HKLM_REGPATH, "EnableTrace").as_deref(),
            Some("true")
        );
    }

    #[test]
    fn changed_install_updates_config_path() {
        let registry = MockRegistry::default();
        registry.set_string(HKLM_REGPATH, "ConfigPath", r"C:\Old\config");

        write_install_registry(
            &registry,
            Path::new(r"C:\New\EqualizerAPO"),
            r"C:\Old\EqualizerAPO",
            "Equalizer APO 1.4.2",
        )
        .unwrap();

        assert_eq!(
            registry.read_string(HKLM_REGPATH, "ConfigPath").as_deref(),
            Some(r"C:\New\EqualizerAPO\config")
        );
    }

    #[test]
    fn uninstall_registry_matches_add_remove_programs() {
        let registry = MockRegistry::default();
        write_uninstall_registry(&registry, Path::new(r"C:\Program Files\EqualizerAPO")).unwrap();
        assert_eq!(
            registry
                .read_string(HKLM_UNINST_REGPATH, "DisplayName")
                .as_deref(),
            Some("Equalizer APO")
        );
        assert_eq!(
            registry
                .read_string(HKLM_UNINST_REGPATH, "DisplayVersion")
                .as_deref(),
            Some("1.4.2")
        );
        assert_eq!(
            registry.read_dword(HKLM_UNINST_REGPATH, "NoModify"),
            Some(1)
        );
        assert_eq!(
            registry.read_dword(HKLM_UNINST_REGPATH, "NoRepair"),
            Some(1)
        );
        assert_eq!(
            registry
                .read_string(HKLM_UNINST_REGPATH, "UninstallString")
                .as_deref(),
            Some(r#""C:\Program Files\EqualizerAPO\Uninstall.exe""#)
        );
    }
}
