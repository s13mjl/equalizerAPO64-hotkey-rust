//! Start menu shortcuts through the COM `IShellLinkW` interface.
//!
//! These are the same six shortcuts that `Setup.nsi` creates with
//! `CreateShortCut`; no desktop or Quick Launch shortcuts are created.

use std::fs;
use std::path::{Path, PathBuf};

use windows::core::{Interface, PCWSTR};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_INPROC_SERVER};
use windows::Win32::UI::Shell::{IShellLinkW, ShellLink};

use crate::util::{com_initialize_sta, wide};

const SHORTCUTS: [(&str, &str); 6] = [
    ("Equalizer APO Configuration Editor.lnk", "Editor.exe"),
    (
        "Configuration tutorial (online).lnk",
        "Configuration tutorial (online).url",
    ),
    (
        "Configuration reference (online).lnk",
        "Configuration reference (online).url",
    ),
    ("Equalizer APO Device Selector.lnk", "DeviceSelector.exe"),
    ("Benchmark.lnk", "Benchmark.exe"),
    ("Uninstall.lnk", "Uninstall.exe"),
];

/// `CreateShortCut "$SMPROGRAMS\$StartMenuFolder\name.lnk" "$INSTDIR\target"`.
pub fn create_shortcut(
    link_path: &Path,
    target: &Path,
    working_dir: &Path,
    arguments: Option<&str>,
) -> Result<(), String> {
    if let Some(parent) = link_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }
    crate::util::delete_file(link_path);

    let link: IShellLinkW = unsafe { CoCreateInstance(&ShellLink, None, CLSCTX_INPROC_SERVER) }
        .map_err(|error| format!("CoCreateInstance(ShellLink) failed: {error}"))?;

    let target_wide = wide(&target.to_string_lossy());
    let working_wide = wide(&working_dir.to_string_lossy());
    let link_wide = wide(&link_path.to_string_lossy());

    unsafe {
        link.SetPath(PCWSTR(target_wide.as_ptr()))
            .map_err(|error| format!("IShellLink::SetPath failed: {error}"))?;
        link.SetWorkingDirectory(PCWSTR(working_wide.as_ptr()))
            .map_err(|error| format!("IShellLink::SetWorkingDirectory failed: {error}"))?;
        if let Some(arguments) = arguments {
            let arguments = wide(arguments);
            link.SetArguments(PCWSTR(arguments.as_ptr()))
                .map_err(|error| format!("IShellLink::SetArguments failed: {error}"))?;
        }

        let persist: windows::Win32::System::Com::IPersistFile = link
            .cast()
            .map_err(|error| format!("QueryInterface(IPersistFile) failed: {error}"))?;
        persist
            .Save(PCWSTR(link_wide.as_ptr()), true)
            .map_err(|error| format!("IPersistFile::Save failed: {error}"))?;
    }

    Ok(())
}

/// Creates all shortcuts in `start_menu_programs\<folder>`.
pub fn create_start_menu_shortcuts(
    start_menu_programs: &Path,
    folder: &str,
    install_dir: &Path,
) -> Result<(), String> {
    if folder.is_empty() {
        return Ok(());
    }
    let _com = com_initialize_sta();
    let directory = start_menu_programs.join(folder);
    fs::create_dir_all(&directory)
        .map_err(|error| format!("Could not create {}: {error}", directory.display()))?;

    for (link_name, target_name) in SHORTCUTS {
        create_shortcut(
            &directory.join(link_name),
            &install_dir.join(target_name),
            install_dir,
            None,
        )?;
    }
    Ok(())
}

/// `RMDir /r "$SMPROGRAMS\$StartMenuFolder"`.  An empty value is ignored so a
/// missing registry value can never delete the whole Programs folder.
pub fn remove_start_menu_folder(start_menu_programs: &Path, folder: &str) -> bool {
    if folder.is_empty() {
        return true;
    }
    let directory = start_menu_programs.join(folder);
    if !directory.exists() {
        return true;
    }
    fs::remove_dir_all(&directory).is_ok()
}

#[allow(dead_code)]
pub fn shortcut_path(start_menu_programs: &Path, folder: &str, name: &str) -> PathBuf {
    start_menu_programs.join(folder).join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shortcut_names_match_setup_nsi() {
        let programs = Path::new(r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs");
        assert_eq!(
            shortcut_path(programs, "Equalizer APO 1.4.2", SHORTCUTS[0].0),
            PathBuf::from(
                r"C:\ProgramData\Microsoft\Windows\Start Menu\Programs\Equalizer APO 1.4.2\Equalizer APO Configuration Editor.lnk"
            )
        );
        assert_eq!(SHORTCUTS.len(), 6);
        assert_eq!(SHORTCUTS[5].1, "Uninstall.exe");
    }

    #[test]
    fn empty_folder_never_removes_programs_root() {
        let directory =
            std::env::temp_dir().join(format!("eapo-shortcut-test-{}", std::process::id()));
        let _ = fs::create_dir_all(&directory);
        fs::write(directory.join("keep.txt"), b"keep").unwrap();
        assert!(remove_start_menu_folder(&directory, ""));
        assert!(directory.join("keep.txt").exists());
        let _ = fs::remove_dir_all(&directory);
    }
}
