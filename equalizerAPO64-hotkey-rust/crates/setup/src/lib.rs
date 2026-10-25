//! Library side of the pure Rust Equalizer APO Setup/Uninstall port.
//!
//! Keeping the implementation in a library target allows unit tests to run
//! without the `requireAdministrator` manifest, which is linked only into the
//! `Setup` binary by `build.rs`.

#![allow(clippy::too_many_arguments)]

mod app;
mod arch;
mod elevation;
mod files;
mod install;
mod lang;
mod registry;
mod shortcuts;
mod uninstall;
mod util;
mod version;

use lang::Language;
use version::{full_name, TARGET_ARCH};

/// Internal marker preventing an elevation retry loop if `ShellExecuteW
/// "runas"` starts a process whose token is still filtered.
const ELEVATION_ATTEMPT: &str = "/elevation-attempt";

pub fn run() {
    let executable = std::env::current_exe().unwrap_or_default();
    let args: Vec<String> = std::env::args().skip(1).collect();
    let language = Language::from_system();
    let uninstall_mode = uninstall::is_uninstall_mode(&executable, &args);
    let silent = args
        .iter()
        .any(|argument| argument.eq_ignore_ascii_case("/S"));
    // NSIS: /D=<path> must be the last argument and is not quoted.
    let install_dir_override = args
        .iter()
        .find_map(|argument| {
            argument
                .strip_prefix("/D=")
                .or_else(|| argument.strip_prefix("/d="))
        })
        .filter(|value| !value.is_empty())
        .map(std::path::PathBuf::from);

    // RequestExecutionLevel admin: build.rs embeds the manifest; this runtime
    // check also covers toolchains/launch paths where the manifest is ignored.
    if !elevation::is_elevated() {
        if args
            .iter()
            .any(|argument| argument.eq_ignore_ascii_case(ELEVATION_ATTEMPT))
        {
            util::message_error(language.elevation_failed(), &language.window_title());
            std::process::exit(1);
        }

        let mut elevated_args = args.clone();
        elevated_args.push(ELEVATION_ATTEMPT.to_string());
        match elevation::relaunch_elevated(&executable, &elevated_args) {
            Ok(()) => return,
            Err(error) => {
                util::message_error(
                    &format!("{}\n\n{error}", language.elevation_failed()),
                    &language.window_title(),
                );
                std::process::exit(1);
            }
        }
    }

    if !uninstall_mode {
        // Setup64.nsi: TARGET_ARCH is x64, abort on every other native arch.
        let detected = arch::native_arch();
        if detected != TARGET_ARCH {
            util::message_error(
                &language.version_error(TARGET_ARCH, detected),
                &language.window_title(),
            );
            return;
        }

        // Setup.nsi checks ucrtbase.dll on Windows versions before 10 and
        // offers the VC++ 2015-2022 redistributable.
        if !util::at_least_windows_10() && !util::ucrt_available() {
            if util::confirm_yes_no(language.ucrt_error(), &language.window_title()) {
                let _ = util::open_url(language.update_download_url());
            }
            return;
        }

        if silent {
            std::process::exit(app::run_silent_install(install_dir_override));
        }
        if let Err(error) = app::run_installer() {
            util::message_error(
                &format!("{}\n\n{error}", full_name()),
                language.error_title(),
            );
        }
        return;
    }

    let from_temp = uninstall::has_temp_marker(&args);

    // Resolve $INSTDIR before copying to TEMP so a missing registry value can
    // still fall back to the directory containing Uninstall.exe.
    let registered = uninstall::registered_install_dir();
    let from_args = uninstall::install_dir_from_args(&args);
    let located_parent = if from_temp || !util::file_name_is(&executable, "Uninstall.exe") {
        // Only an Uninstall.exe located in an installation directory is
        // authoritative.  Setup.exe /uninstall always uses InstallPath so a
        // payload folder next to Setup.exe is never mistaken for $INSTDIR.
        None
    } else {
        executable.parent().and_then(|parent| {
            if parent.join("EqualizerAPO.dll").exists() || parent.join("Uninstall.exe").exists() {
                Some(parent.to_path_buf())
            } else {
                None
            }
        })
    };
    let install_dir = if from_temp {
        from_args.or(registered)
    } else {
        located_parent.or(registered).or(from_args)
    };

    // Uninstaller: copy ourselves to %TEMP% so $INSTDIR\Uninstall.exe can be
    // deleted while the uninstaller is running.  Pass the resolved directory
    // along because the temporary process may not see the same fallback.
    if !from_temp {
        if let Some(directory) = install_dir.as_ref() {
            match uninstall::relaunch_from_temp(&executable, Some(directory)) {
                Ok(_) => return,
                Err(error) => {
                    // Fall through and uninstall in place; Uninstall.exe will
                    // be scheduled for deletion at reboot.
                    eprintln!("Warning: could not relaunch uninstaller from TEMP: {error}");
                }
            }
        }
    }

    let Some(install_dir) = install_dir else {
        util::message_error(language.not_installed(), language.error_title());
        return;
    };

    let start_menu_folder = uninstall::registered_start_menu_folder();
    if silent {
        std::process::exit(app::run_silent_uninstall(
            install_dir,
            start_menu_folder,
            from_temp,
        ));
    }
    let window_result = app::run_uninstaller(install_dir, start_menu_folder, from_temp);

    // Schedule the temporary uninstaller for removal even if the user closed
    // it before starting the uninstall (the worker does this after a run).
    if from_temp {
        if let Ok(current) = std::env::current_exe() {
            util::schedule_delete_at_reboot(&current);
            if let Some(parent) = current.parent() {
                util::schedule_delete_at_reboot(parent);
            }
        }
    }

    if let Err(error) = window_result {
        util::message_error(&format!("{error}"), language.error_title());
    }
}
