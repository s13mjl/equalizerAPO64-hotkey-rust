//! Installer payload handling.
//!
//! The NSIS installer compiles the files into the executable and unpacks them
//! relative to `$INSTDIR`.  The Rust port uses a *staging* directory next to
//! Setup.exe (`files\` when present, otherwise the executable directory, i.e.
//! `target\release`).  File names and installation order mirror `Setup.nsi`.
//!
//! The binaries are produced by Cargo; the Qt/FFTW/libsndfile DLLs of the C++
//! build do not exist in this fork.  They are still copied when present so a
//! staging directory assembled from an upstream release keeps working.

use std::fs::{self, OpenOptions};
use std::io::Write as _;
use std::path::{Path, PathBuf};

/// `File "${BINPATH}\..."`, in `Setup.nsi` order.
pub const APPLICATION_FILES: [&str; 6] = [
    "EqualizerAPO.dll",
    "DeviceSelector.exe",
    "Benchmark.exe",
    "VoicemeeterClient.exe",
    "UpdateChecker.exe",
    "Editor.exe",
];

/// `File "${LIBPATH}\..."` (only copied when present).
pub const OPTIONAL_LIBRARY_FILES: [&str; 7] = [
    "libfftw3.dll",
    "sndfile.dll",
    "Qt6Core.dll",
    "Qt6Gui.dll",
    "Qt6Network.dll",
    "Qt6Svg.dll",
    "Qt6Widgets.dll",
];

/// `Delete "$INSTDIR\..."` before upgrading (no reboot flag).
pub const LEGACY_FILES: [&str; 7] = [
    "Configurator.exe",
    "Qt5Core.dll",
    "Qt5Gui.dll",
    "Qt5Widgets.dll",
    r"qt\imageformats\qgif.dll",
    r"qt\imageformats\qjpeg.dll",
    r"qt\styles\qwindowsvistastyle.dll",
];

/// `!insertmacro RenameAndDelete "$INSTDIR\..."`, in `Setup.nsi` order.
pub const RENAME_AND_DELETE_FILES: [&str; 15] = [
    "EqualizerAPO.dll",
    "libfftw3-3.dll",
    "libfftw3.dll",
    "libsndfile-1.dll",
    "sndfile.dll",
    "msvcp100.dll",
    "msvcr100.dll",
    "msvcp120.dll",
    "msvcr120.dll",
    "msvcp140.dll",
    "msvcp140_1.dll",
    "msvcp140_2.dll",
    "VoicemeeterClient.exe",
    "vcruntime140.dll",
    "vcruntime140_1.dll",
];

/// Files copied with `SetOverwrite off` (`Setup/config`).
pub const CONFIG_FILES: [(&str, &str); 6] = [
    (
        "config.txt",
        include_str!("../../../assets/setup-config/config.txt"),
    ),
    (
        "example.txt",
        include_str!("../../../assets/setup-config/example.txt"),
    ),
    (
        "demo.txt",
        include_str!("../../../assets/setup-config/demo.txt"),
    ),
    (
        "multichannel.txt",
        include_str!("../../../assets/setup-config/multichannel.txt"),
    ),
    (
        "iir_lowpass.txt",
        include_str!("../../../assets/setup-config/iir_lowpass.txt"),
    ),
    (
        "selective_delay.txt",
        include_str!("../../../assets/setup-config/selective_delay.txt"),
    ),
];

/// `File "Configuration tutorial (online).url"` and its reference counterpart.
pub const URL_FILES: [(&str, &str); 2] = [
    (
        "Configuration reference (online).url",
        include_str!("../../../assets/Configuration reference (online).url"),
    ),
    (
        "Configuration tutorial (online).url",
        include_str!("../../../assets/Configuration tutorial (online).url"),
    ),
];

/// NSIS embeds `License.txt` with `MUI_PAGE_LICENSE ..\License.txt`.
pub const LICENSE_TEXT: &str = include_str!("../../../License.txt");

pub const VST_PLUGINS_DIR: &str = "VSTPlugins";
pub const CONFIG_DIR: &str = "config";
pub const QT_DIR: &str = "qt";

/// Locates the payload directory.
///
/// 1. `files\` next to Setup.exe (development layout; the uninstaller never
///    needs a payload).
/// 2. The archive appended to the running executable by `PackPayload`
///    (single-file release), extracted once into `%TEMP%`.
/// 3. The executable directory (`target\release`), which holds the binaries when
///    Setup is started from the build output.
pub fn find_staging(exe_dir: &Path) -> PathBuf {
    let files = exe_dir.join("files");
    if files.join("EqualizerAPO.dll").is_file() {
        return files;
    }
    if let Some(extracted) = crate::payload::embedded_staging() {
        return extracted;
    }
    exe_dir.to_path_buf()
}

/// Returns the required payload files that are missing from `staging`.
pub fn missing_required_files(staging: &Path) -> Vec<String> {
    APPLICATION_FILES
        .iter()
        .filter(|name| !staging.join(name).is_file())
        .map(|name| (*name).to_string())
        .collect()
}

/// Deletes the pre-1.4 Qt5/Configurator files.  Failures are ignored exactly
/// like NSIS `Delete` on a missing or locked file.
pub fn delete_legacy_files(install_dir: &Path, log: &mut dyn FnMut(String)) {
    for relative in LEGACY_FILES {
        let path = install_dir.join(relative);
        if path.exists() {
            crate::util::delete_file(&path);
            log(format!("Delete: {}", path.display()));
        }
    }
}

/// `!insertmacro RenameAndDelete` for all in-use candidates.  Returns the
/// renamed paths, which are deleted after DeviceSelector has stopped audio.
pub fn rename_files_for_deletion(install_dir: &Path, log: &mut dyn FnMut(String)) -> Vec<PathBuf> {
    let mut renamed = Vec::new();
    for relative in RENAME_AND_DELETE_FILES {
        let path = install_dir.join(relative);
        if path.exists() {
            if let Some(new_path) = crate::util::rename_then_delete(&path) {
                log(format!(
                    "Rename: {} -> {}",
                    path.display(),
                    new_path.display()
                ));
                renamed.push(new_path);
            } else {
                log(format!("Warning: could not rename {}", path.display()));
            }
        }
    }
    renamed
}

fn same_file(source: &Path, destination: &Path) -> bool {
    if source == destination {
        return true;
    }
    match (fs::canonicalize(source), fs::canonicalize(destination)) {
        (Ok(source), Ok(destination)) => source == destination,
        _ => false,
    }
}

fn copy_file(
    source: &Path,
    destination: &Path,
    renamed: &mut Vec<PathBuf>,
    log: &mut dyn FnMut(String),
) -> Result<(), String> {
    if same_file(source, destination) {
        log(format!("Keeping: {}", destination.display()));
        return Ok(());
    }
    if let Some(parent) = destination.parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("Could not create {}: {error}", parent.display()))?;
    }

    match fs::copy(source, destination) {
        Ok(_) => {
            log(format!(
                "Extract: {} -> {}",
                source.display(),
                destination.display()
            ));
            Ok(())
        }
        Err(first_error) => {
            // The target may be in use (EqualizerAPO.dll loaded by audiodg).
            // Rename it and retry, like the NSIS RenameAndDelete macro.
            if destination.exists() {
                if let Some(new_path) = crate::util::rename_then_delete(destination) {
                    renamed.push(new_path);
                    match fs::copy(source, destination) {
                        Ok(_) => {
                            log(format!(
                                "Extract: {} -> {}",
                                source.display(),
                                destination.display()
                            ));
                            return Ok(());
                        }
                        Err(second_error) => {
                            return Err(format!(
                            "Could not copy {} to {}: {first_error}; retry failed: {second_error}",
                            source.display(),
                            destination.display()
                        ))
                        }
                    }
                }
            }
            Err(format!(
                "Could not copy {} to {}: {first_error}",
                source.display(),
                destination.display()
            ))
        }
    }
}

fn copy_tree(
    source: &Path,
    destination: &Path,
    renamed: &mut Vec<PathBuf>,
    log: &mut dyn FnMut(String),
) -> Result<(), String> {
    fs::create_dir_all(destination)
        .map_err(|error| format!("Could not create {}: {error}", destination.display()))?;
    for entry in fs::read_dir(source)
        .map_err(|error| format!("Could not read {}: {error}", source.display()))?
    {
        let entry = entry.map_err(|error| format!("Could not read directory entry: {error}"))?;
        let source_path = entry.path();
        let destination_path = destination.join(entry.file_name());
        if source_path.is_dir() {
            copy_tree(&source_path, &destination_path, renamed, log)?;
        } else {
            copy_file(&source_path, &destination_path, renamed, log)?;
        }
    }
    Ok(())
}

/// Copies the application payload.  Missing required files are reported by
/// [`missing_required_files`] before this function is called.
pub fn copy_application_files(
    staging: &Path,
    install_dir: &Path,
    renamed: &mut Vec<PathBuf>,
    log: &mut dyn FnMut(String),
) -> Result<(), String> {
    fs::create_dir_all(install_dir)
        .map_err(|error| format!("Could not create {}: {error}", install_dir.display()))?;

    for name in APPLICATION_FILES {
        copy_file(&staging.join(name), &install_dir.join(name), renamed, log)?;
    }

    for name in OPTIONAL_LIBRARY_FILES {
        let source = staging.join(name);
        if source.is_file() {
            copy_file(&source, &install_dir.join(name), renamed, log)?;
        }
    }

    // Qt plugin tree of the upstream C++ build, if a staging directory from a
    // full upstream release is used.
    let qt = staging.join(QT_DIR);
    if qt.is_dir() {
        copy_tree(&qt, &install_dir.join(QT_DIR), renamed, log)?;
    }

    Ok(())
}

/// Copies the two `.url` files.  They are embedded because the Cargo build
/// does not stage them; a staged copy takes precedence.
pub fn copy_url_files(
    staging: &Path,
    install_dir: &Path,
    log: &mut dyn FnMut(String),
) -> Result<(), String> {
    for (name, embedded) in URL_FILES {
        let destination = install_dir.join(name);
        let source = staging.join(name);
        if source.is_file() {
            if !same_file(&source, &destination) {
                fs::copy(&source, &destination)
                    .map_err(|error| format!("Could not copy {}: {error}", source.display()))?;
            }
        } else {
            fs::write(&destination, embedded)
                .map_err(|error| format!("Could not write {}: {error}", destination.display()))?;
        }
        log(format!("Extract: {name}"));
    }
    Ok(())
}

/// Creates `config\` and writes every missing default example.  Existing user
/// configurations are never overwritten (`SetOverwrite off` in Setup.nsi).
pub fn create_default_config(
    staging: &Path,
    install_dir: &Path,
    log: &mut dyn FnMut(String),
) -> Result<PathBuf, String> {
    let config_dir = install_dir.join(CONFIG_DIR);
    fs::create_dir_all(&config_dir)
        .map_err(|error| format!("Could not create {}: {error}", config_dir.display()))?;

    for (name, embedded) in CONFIG_FILES {
        let destination = config_dir.join(name);
        if destination.exists() {
            continue;
        }
        let source = staging.join(CONFIG_DIR).join(name);
        let contents = if source.is_file() {
            fs::read(&source)
                .map_err(|error| format!("Could not read {}: {error}", source.display()))?
        } else {
            embedded.as_bytes().to_vec()
        };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&destination)
            .map_err(|error| format!("Could not create {}: {error}", destination.display()))?;
        file.write_all(&contents)
            .map_err(|error| format!("Could not write {}: {error}", destination.display()))?;
        log(format!("Extract: config\\{name}"));
    }

    Ok(config_dir)
}

/// Creates the empty `VSTPlugins` directory.
pub fn create_vst_plugins_dir(install_dir: &Path) -> Result<(), String> {
    let path = install_dir.join(VST_PLUGINS_DIR);
    fs::create_dir_all(&path)
        .map_err(|error| format!("Could not create {}: {error}", path.display()))
}

/// `WriteUninstaller "$INSTDIR\Uninstall.exe"`: the same Rust executable is
/// shipped as Uninstall.exe and switches to uninstall mode based on its file
/// name or `/uninstall`.
pub fn write_uninstaller(
    current_exe: &Path,
    install_dir: &Path,
    renamed: &mut Vec<PathBuf>,
    log: &mut dyn FnMut(String),
) -> Result<PathBuf, String> {
    let destination = install_dir.join("Uninstall.exe");
    if current_exe == destination {
        return Ok(destination);
    }
    if destination.exists() {
        if fs::remove_file(&destination).is_err() {
            if let Some(old) = crate::util::rename_then_delete(&destination) {
                renamed.push(old);
            }
        }
    }
    fs::copy(current_exe, &destination).map_err(|error| {
        format!(
            "Could not write {} from {}: {error}",
            destination.display(),
            current_exe.display()
        )
    })?;
    log(format!("WriteUninstaller: {}", destination.display()));
    Ok(destination)
}

/// Approximate payload size for the MUI directory page ("Space required").
pub fn payload_size(staging: &Path) -> u64 {
    let mut total = 0;
    for name in APPLICATION_FILES {
        total += fs::metadata(staging.join(name))
            .map(|metadata| metadata.len())
            .unwrap_or(0);
    }
    for (_, contents) in CONFIG_FILES {
        total += contents.len() as u64;
    }
    for (_, contents) in URL_FILES {
        total += contents.len() as u64;
    }
    total + LICENSE_TEXT.len() as u64
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(name: &str) -> PathBuf {
        let path = std::env::temp_dir().join(format!(
            "eapo-setup-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        let _ = fs::remove_dir_all(&path);
        fs::create_dir_all(&path).unwrap();
        path
    }

    #[test]
    fn reports_missing_required_payload() {
        let directory = temp_dir("missing");
        let missing = missing_required_files(&directory);
        assert_eq!(missing.len(), APPLICATION_FILES.len());

        for name in APPLICATION_FILES {
            fs::write(directory.join(name), b"x").unwrap();
        }
        assert!(missing_required_files(&directory).is_empty());
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn staging_prefers_complete_files_subdirectory() {
        let directory = temp_dir("staging");
        assert_eq!(find_staging(&directory), directory);
        fs::create_dir_all(directory.join("files")).unwrap();
        // An incomplete files\ directory does not hide binaries next to Setup.
        assert_eq!(find_staging(&directory), directory);
        fs::write(directory.join("files").join("EqualizerAPO.dll"), b"x").unwrap();
        assert_eq!(find_staging(&directory), directory.join("files"));
        let _ = fs::remove_dir_all(&directory);
    }

    #[test]
    fn default_config_is_never_overwritten() {
        let directory = temp_dir("config");
        let staging = temp_dir("config-staging");
        let mut log = |_message: String| {};

        let config_dir =
            create_default_config(&staging, &directory, &mut log).expect("config created");
        assert!(config_dir.join("config.txt").is_file());
        fs::write(config_dir.join("config.txt"), "user configuration").unwrap();

        create_default_config(&staging, &directory, &mut log).expect("config preserved");
        assert_eq!(
            fs::read_to_string(config_dir.join("config.txt")).unwrap(),
            "user configuration"
        );

        let _ = fs::remove_dir_all(&directory);
        let _ = fs::remove_dir_all(&staging);
    }

    #[test]
    fn copies_payload_and_keeps_existing_configuration() {
        let staging = temp_dir("payload-staging");
        let install = temp_dir("payload-install");
        for name in APPLICATION_FILES {
            fs::write(staging.join(name), format!("new {name}")).unwrap();
        }
        fs::create_dir_all(install.join("config")).unwrap();
        fs::write(install.join("config").join("config.txt"), "user config").unwrap();

        let mut renamed = Vec::new();
        let mut log = |_message: String| {};
        copy_application_files(&staging, &install, &mut renamed, &mut log).unwrap();
        copy_url_files(&staging, &install, &mut log).unwrap();
        create_default_config(&staging, &install, &mut log).unwrap();
        create_vst_plugins_dir(&install).unwrap();

        for name in APPLICATION_FILES {
            assert_eq!(
                fs::read_to_string(install.join(name)).unwrap(),
                format!("new {name}")
            );
        }
        assert_eq!(
            fs::read_to_string(install.join("config").join("config.txt")).unwrap(),
            "user config"
        );
        assert!(install.join("VSTPlugins").is_dir());
        assert!(install
            .join("Configuration reference (online).url")
            .is_file());

        let _ = fs::remove_dir_all(&staging);
        let _ = fs::remove_dir_all(&install);
    }

    #[test]
    fn urls_are_written_from_embedded_assets() {
        let directory = temp_dir("urls");
        let staging = temp_dir("urls-staging");
        let mut log = |_message: String| {};
        copy_url_files(&staging, &directory, &mut log).unwrap();
        let text =
            fs::read_to_string(directory.join("Configuration reference (online).url")).unwrap();
        assert!(
            text.contains("http://sourceforge.net/p/equalizerapo/wiki/Configuration%20reference")
        );
        let _ = fs::remove_dir_all(&directory);
        let _ = fs::remove_dir_all(&staging);
    }

    #[test]
    fn rename_and_delete_uses_next_free_old_name() {
        let directory = temp_dir("rename");
        let path = directory.join("test.dll");
        fs::write(&path, b"a").unwrap();
        fs::write(directory.join("test.dll.old"), b"b").unwrap();
        let renamed = crate::util::rename_then_delete(&path).expect("renamed");
        assert_eq!(renamed, directory.join("test.dll.old.0"));
        assert!(!path.exists());
        let _ = fs::remove_dir_all(&directory);
    }
}
