//! Uninstallation logic, in the same order as the uninstaller sections of
//! `Setup.nsi`.

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::Arc;

use eapo_apo_info::{DeviceAPOInfo, VoicemeeterAPOInfo};
use eapo_common::task_scheduler_helper::unschedule;

use crate::install::UPDATE_TASK_NAME;
use crate::lang::Language;
use crate::registry::{
    RegistryStore, WinRegistry, HKCU_REGPATH, HKLM_AUDIO_REGPATH, HKLM_REGPATH, HKLM_UNINST_REGPATH,
};
use crate::shortcuts;
use crate::util::{
    delete_file, delete_reboot_ok, remove_dir_all_reboot_ok, run_and_wait,
    schedule_delete_at_reboot, start_menu_programs_dir, system_directory,
};

#[derive(Debug, Clone)]
pub struct UninstallOptions {
    pub install_dir: PathBuf,
    pub start_menu_folder: String,
    pub remove_configs: bool,
    pub language: Language,
    pub from_temp: bool,
}

#[derive(Debug, Clone, Default)]
pub struct UninstallResult {
    pub success: bool,
    pub cancelled: bool,
    pub reboot: bool,
    pub error: Option<String>,
}

impl UninstallResult {
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
            error: Some(language.uninstallation_aborted().to_string()),
        }
    }
}

#[derive(Debug, Clone)]
pub enum UninstallMessage {
    Progress { fraction: f32, text: String },
    Log(String),
    Finished(UninstallResult),
}

struct Reporter {
    sender: Sender<UninstallMessage>,
    cancel: Arc<AtomicBool>,
}

impl Reporter {
    fn progress(&self, fraction: f32, text: &str) {
        let _ = self.sender.send(UninstallMessage::Progress {
            fraction: fraction.clamp(0.0, 1.0),
            text: text.to_string(),
        });
    }

    fn log(&self, message: impl Into<String>) {
        let _ = self.sender.send(UninstallMessage::Log(message.into()));
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Relaxed)
    }
}

pub fn run_uninstall(
    options: UninstallOptions,
    sender: Sender<UninstallMessage>,
    cancel: Arc<AtomicBool>,
) {
    let reporter = Reporter { sender, cancel };
    let result = uninstall_impl(&options, &reporter);

    // The uninstaller normally runs from %TEMP% so it can delete
    // $INSTDIR\Uninstall.exe.  Windows cannot delete a running image; schedule
    // the temporary copy for deletion at the next reboot (NSIS has the same
    // limitation for its ~nsu.tmp copy).
    if options.from_temp {
        let current = std::env::current_exe().unwrap_or_default();
        if schedule_delete_at_reboot(&current) {
            reporter.log(format!(
                "Scheduled temporary uninstaller for reboot deletion: {}",
                current.display()
            ));
            if let Some(parent) = current.parent() {
                // PendingFileRenameOperations processes the file first, then
                // the now-empty unique directory.
                schedule_delete_at_reboot(parent);
            }
        }
    }

    let _ = reporter.sender.send(UninstallMessage::Finished(result));
}

fn uninstall_impl(options: &UninstallOptions, reporter: &Reporter) -> UninstallResult {
    let language = options.language;
    let install_dir = &options.install_dir;
    let registry = WinRegistry::new();
    let mut reboot = false;

    reporter.progress(0.02, language.preparing());
    reporter.log(format!("Uninstall path: {}", install_dir.display()));

    // Optional section "Remove configurations and registry backups" is
    // declared before the hidden uninstall section, so it runs first.
    if options.remove_configs {
        reporter.progress(0.06, language.removing_files());
        delete_reg_files(install_dir, reporter);
        if !remove_dir_all_reboot_ok(&install_dir.join("config")) {
            reboot = true;
            reporter.log("Warning: config directory will be removed at reboot");
        }
        if let Err(error) = registry.delete_key_tree(HKCU_REGPATH) {
            reporter.log(format!("Warning: could not delete HKCU key: {error}"));
        }
    }
    if reporter.cancelled() {
        return UninstallResult::cancelled(language);
    }

    // Qt applications only work if working directory is application dir.
    reporter.progress(0.15, language.removing_files());
    let update_checker = install_dir.join("UpdateChecker.exe");
    if update_checker.is_file() {
        match run_and_wait(
            &update_checker,
            &[OsStr::new("-u")],
            Some(install_dir),
            false,
        ) {
            Ok(status) => reporter.log(format!("UpdateChecker.exe -u exited with {status}")),
            Err(error) => {
                reporter.log(format!("Warning: could not run UpdateChecker.exe: {error}"))
            }
        }
    }
    match unschedule(UPDATE_TASK_NAME) {
        Ok(()) => reporter.log(format!("Removed scheduled task {UPDATE_TASK_NAME}")),
        Err(error) => reporter.log(format!("Warning: could not remove update task: {error}")),
    }

    let device_selector = install_dir.join("DeviceSelector.exe");
    if device_selector.is_file() {
        match run_and_wait(
            &device_selector,
            &[OsStr::new("/u")],
            Some(install_dir),
            false,
        ) {
            Ok(status) => reporter.log(format!("DeviceSelector.exe /u exited with {status}")),
            Err(error) => reporter.log(format!(
                "Warning: could not run DeviceSelector.exe: {error}"
            )),
        }
    }

    // DeviceSelector /u handles the normal path.  The direct APO-info pass
    // makes sure no device registration is left behind if the selector is
    // missing, was cancelled or is a stub (shared crate API).
    uninstall_all_apos(reporter);
    // DeviceSelector /u calls this after removing APO registrations.
    match VoicemeeterAPOInfo::ensure_voicemeeter_client_running() {
        Ok(()) => {}
        Err(error) => reporter.log(format!("Warning: Voicemeeter client check: {error}")),
    }
    if reporter.cancelled() {
        return UninstallResult::cancelled(language);
    }

    // RegDLL doesn't work for 64 bit dlls (NSIS comment).
    let dll = install_dir.join("EqualizerAPO.dll");
    if dll.is_file() {
        let regsvr32 = system_directory().join("regsvr32.exe");
        let args: [&OsStr; 3] = [OsStr::new("/u"), OsStr::new("/s"), dll.as_os_str()];
        match run_and_wait(&regsvr32, &args, Some(install_dir), true) {
            Ok(status) => reporter.log(format!(
                "regsvr32 /u /s {} exited with {}",
                dll.display(),
                status
            )),
            Err(error) => reporter.log(format!("Warning: could not run regsvr32.exe: {error}")),
        }
    }

    if let Err(error) = registry.delete_value(HKLM_AUDIO_REGPATH, "DisableProtectedAudioDG") {
        reporter.log(format!(
            "Warning: could not delete DisableProtectedAudioDG: {error}"
        ));
    }

    if !shortcuts::remove_start_menu_folder(&start_menu_programs_dir(), &options.start_menu_folder)
    {
        reporter.log("Warning: could not remove start menu folder");
    }

    // Only remove if empty (RMDir without /r).
    let _ = fs::remove_dir(install_dir.join("VSTPlugins"));

    reporter.progress(0.55, language.removing_files());
    for name in [
        "Configuration reference (online).url",
        "Configuration tutorial (online).url",
    ] {
        delete_file(&install_dir.join(name));
    }

    let _ = fs::remove_dir_all(install_dir.join("qt"));

    for name in [
        "Qt6Widgets.dll",
        "Qt6Svg.dll",
        "Qt6Network.dll",
        "Qt6Gui.dll",
        "Qt6Core.dll",
    ] {
        delete_file(&install_dir.join(name));
    }

    reporter.progress(0.68, language.removing_files());
    for name in ["sndfile.dll", "libfftw3.dll"] {
        if !delete_reboot_ok(&install_dir.join(name)) {
            reboot = true;
        }
    }
    delete_file(&install_dir.join("Editor.exe"));

    for name in [
        "UpdateChecker.exe",
        "VoicemeeterClient.exe",
        "Benchmark.exe",
        "DeviceSelector.exe",
    ] {
        delete_file(&install_dir.join(name));
    }
    if !delete_reboot_ok(&dll) {
        reboot = true;
    }

    // $INSTDIR\Uninstall.exe.  Normally this is not the running image because
    // we relaunched from %TEMP%.
    if !delete_reboot_ok(&install_dir.join("Uninstall.exe")) {
        reboot = true;
    }

    // RMDir /REBOOTOK "$INSTDIR" (only if empty; never recursive).
    let empty = fs::read_dir(install_dir)
        .map(|mut entries| entries.next().is_none())
        .unwrap_or(true);
    if empty {
        match fs::remove_dir(install_dir) {
            Ok(()) => {}
            Err(_) => {
                schedule_delete_at_reboot(install_dir);
                reboot = true;
            }
        }
    }

    reporter.progress(0.86, language.removing_files());
    if let Err(error) = remove_uninstall_registry(&registry) {
        reporter.log(format!(
            "Warning: could not remove uninstall registry values: {error}"
        ));
    }

    reporter.progress(1.0, language.done());
    reporter.log("Uninstallation complete.");
    UninstallResult {
        success: true,
        cancelled: false,
        reboot,
        error: None,
    }
}

fn delete_reg_files(install_dir: &Path, reporter: &Reporter) {
    let Ok(entries) = fs::read_dir(install_dir) else {
        return;
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path
            .extension()
            .and_then(OsStr::to_str)
            .map(|extension| extension.eq_ignore_ascii_case("reg"))
            .unwrap_or(false)
        {
            if fs::remove_file(&path).is_ok() {
                reporter.log(format!("Delete: {}", path.display()));
            }
        }
    }
}

fn uninstall_all_apos(reporter: &Reporter) {
    let _com = crate::util::com_initialize_sta();
    for input in [false, true] {
        match DeviceAPOInfo::load_all_infos(input) {
            Ok(infos) => {
                for info in infos {
                    if !info.is_installed() {
                        continue;
                    }
                    let name = info.get_device_name();
                    match info.uninstall() {
                        Ok(()) => reporter.log(format!("APO uninstalled: {name}")),
                        Err(error) => reporter
                            .log(format!("Warning: could not uninstall APO {name}: {error}")),
                    }
                }
            }
            Err(error) => {
                reporter.log(format!(
                    "Warning: could not enumerate {} audio devices: {error}",
                    if input { "capture" } else { "render" }
                ));
            }
        }
    }
}

/// `DeleteRegKey HKLM ${UNINST_REGPATH}` and
/// `DeleteRegKey /ifempty HKLM ${REGPATH}`.
pub fn remove_uninstall_registry(store: &dyn RegistryStore) -> Result<(), String> {
    store.delete_key_tree(HKLM_UNINST_REGPATH)?;
    // DeleteRegKey /ifempty: non-empty HKLM\Software\EqualizerAPO values
    // (InstallPath, ConfigPath, EnableTrace, Start Menu Folder) stay.
    let _ = store.delete_key_if_empty(HKLM_REGPATH);
    Ok(())
}

/// Reads the installation directory created by the installer.
pub fn registered_install_dir() -> Option<PathBuf> {
    WinRegistry::new()
        .read_string(HKLM_REGPATH, "InstallPath")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

pub fn registered_start_menu_folder() -> String {
    WinRegistry::new()
        .read_string(HKLM_REGPATH, "Start Menu Folder")
        .unwrap_or_default()
}

/// Detects uninstall mode from `/uninstall` or the file name `Uninstall.exe`.
pub fn is_uninstall_mode(exe: &Path, args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg.eq_ignore_ascii_case("/uninstall"))
        || crate::util::file_name_is(exe, "Uninstall.exe")
}

pub fn has_temp_marker(args: &[String]) -> bool {
    args.iter()
        .any(|arg| arg.eq_ignore_ascii_case("/temp-uninstaller"))
}

/// Copies the uninstaller to %TEMP% and starts it there with `/uninstall` and
/// `/temp-uninstaller`.  This is the equivalent of NSIS' `WriteUninstaller`
/// temp-copy behavior that allows deleting `$INSTDIR\Uninstall.exe`.
pub fn relaunch_from_temp(exe: &Path, install_dir: Option<&Path>) -> Result<PathBuf, String> {
    // A unique, unpredictable directory avoids a predictable executable path
    // in a user-writable TEMP directory while Setup runs elevated.
    let nonce = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let directory = std::env::temp_dir().join(format!(
        "EqualizerAPO-Uninstall-{}-{nonce}",
        std::process::id()
    ));
    fs::create_dir(&directory).map_err(|error| {
        format!(
            "Could not create temporary uninstaller folder {}: {error}",
            directory.display()
        )
    })?;
    let temp = directory.join("Uninstall.exe");
    fs::copy(exe, &temp)
        .map_err(|error| format!("Could not copy uninstaller to {}: {error}", temp.display()))?;

    let mut command = Command::new(&temp);
    command.arg("/uninstall").arg("/temp-uninstaller");
    if let Some(install_dir) = install_dir {
        command.arg("/installdir").arg(install_dir);
    }
    command
        .spawn()
        .map_err(|error| format!("Could not start {}: {error}", temp.display()))?;
    Ok(temp)
}

/// Reads `/installdir <path>` from the temporary uninstaller command line.
/// This preserves the install location when the registry value is missing.
pub fn install_dir_from_args(args: &[String]) -> Option<PathBuf> {
    args.windows(2)
        .find(|pair| pair[0].eq_ignore_ascii_case("/installdir"))
        .map(|pair| PathBuf::from(&pair[1]))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::registry::testing::MockRegistry;

    #[test]
    fn uninstall_mode_detection() {
        assert!(is_uninstall_mode(
            Path::new(r"C:\Program Files\EqualizerAPO\Uninstall.exe"),
            &[]
        ));
        assert!(is_uninstall_mode(
            Path::new(r"C:\Setup.exe"),
            &["/uninstall".to_string()]
        ));
        assert!(!is_uninstall_mode(Path::new(r"C:\Setup.exe"), &[]));
        assert!(has_temp_marker(&["/TEMP-UNINSTALLER".to_string()]));
    }

    #[test]
    fn install_dir_is_parsed_from_temp_args() {
        assert_eq!(
            install_dir_from_args(&[
                "/uninstall".to_string(),
                "/temp-uninstaller".to_string(),
                "/installdir".to_string(),
                r"C:\Program Files\EqualizerAPO".to_string(),
            ]),
            Some(PathBuf::from(r"C:\Program Files\EqualizerAPO"))
        );
        assert_eq!(install_dir_from_args(&["/uninstall".to_string()]), None);
    }

    #[test]
    fn uninstall_registry_keeps_app_key_with_values() {
        let registry = MockRegistry::with_install_path(r"C:\Program Files\EqualizerAPO");
        registry.set_string(HKLM_UNINST_REGPATH, "DisplayName", "Equalizer APO");
        registry.set_string(
            HKLM_REGPATH,
            "ConfigPath",
            r"C:\Program Files\EqualizerAPO\config",
        );

        remove_uninstall_registry(&registry).unwrap();

        assert!(registry.key_is_gone(HKLM_UNINST_REGPATH));
        // InstallPath/ConfigPath remain, exactly like DeleteRegKey /ifempty.
        assert_eq!(
            registry.read_string(HKLM_REGPATH, "InstallPath").as_deref(),
            Some(r"C:\Program Files\EqualizerAPO")
        );
    }

    #[test]
    fn empty_app_key_is_removed() {
        let registry = MockRegistry::default();
        registry.set_string(HKLM_UNINST_REGPATH, "DisplayName", "Equalizer APO");
        remove_uninstall_registry(&registry).unwrap();
        assert!(registry.key_is_gone(HKLM_REGPATH));
    }
}
