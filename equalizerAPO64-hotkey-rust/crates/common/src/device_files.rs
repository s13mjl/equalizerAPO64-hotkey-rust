//! The per-device map of live configuration files.
//!
//! The audio engine reads a single configuration file by default. To let one
//! device follow several files (or a single file that is not `config.txt`), the
//! Configuration Editor maintains a small text manifest inside the
//! configuration directory:
//!
//! ```text
//! [device-files.cfg]
//! {4829078d-b25f-4c4b-b06a-f8b5bc97d5fd}
//! zmq.txt
//! D:\configs\bass.txt
//! ```
//!
//! A section header is an endpoint GUID, the lines below it are configuration
//! file paths in load order (relative paths are resolved against the
//! configuration directory). The file lives next to the configuration files
//! themselves because the installer grants the Users group full access to that
//! directory, so the editor can publish changes without elevation.

use std::collections::BTreeMap;
use std::io;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Name of the manifest inside the configuration directory. The `.cfg` suffix
/// keeps it out of every `*.txt` filter the editor and installer use.
pub const MANIFEST_NAME: &str = "device-files.cfg";

/// Attempts spent waiting out a sharing violation before giving up, matching
/// the retry budget of the configuration file reader.
const SHARE_RETRIES: u32 = 2000;

pub fn manifest_path(config_dir: &Path) -> PathBuf {
    config_dir.join(MANIFEST_NAME)
}

fn normalize_guid(guid: &str) -> String {
    guid.trim().to_lowercase()
}

/// Resolves one manifest entry against the configuration directory.
pub fn resolve(config_dir: &Path, entry: &str) -> PathBuf {
    let path = Path::new(entry.trim_matches('"').trim());
    if path.is_absolute() {
        path.to_path_buf()
    } else {
        config_dir.join(path)
    }
}

pub fn parse(text: &str) -> BTreeMap<String, Vec<String>> {
    let mut map: BTreeMap<String, Vec<String>> = BTreeMap::new();
    let mut current: Option<String> = None;
    for raw in text.lines() {
        let line = raw.trim().trim_start_matches('\u{feff}').trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(rest) = line.strip_prefix('[') {
            let guid = normalize_guid(rest.trim_end_matches(']'));
            // An empty section header would let a device with no identity match
            // something, so such a section is dropped along with its entries.
            current = if guid.is_empty() {
                None
            } else {
                Some(guid.clone())
            };
            if let Some(guid) = &current {
                map.entry(guid.clone()).or_default();
            }
            continue;
        }
        if line.ends_with(']') {
            continue;
        }
        let entry = line.trim_matches('"').trim();
        if !entry.is_empty() {
            if let Some(guid) = &current {
                map.entry(guid.clone()).or_default().push(entry.to_string());
            }
        }
    }
    map.retain(|_, files| !files.is_empty());
    map
}

pub fn render(map: &BTreeMap<String, Vec<String>>) -> String {
    let mut text = String::from(HEADER);
    for (guid, files) in map {
        if files.is_empty() {
            continue;
        }
        text.push_str(&format!("[{guid}]\r\n"));
        for file in files {
            text.push_str(&format!("{file}\r\n"));
        }
    }
    text
}

const HEADER: &str = "# Equalizer APO - live configuration files per device.\r\n\
# Maintained by the Configuration Editor; sections are endpoint GUIDs and the\r\n\
# lines below each one are configuration files, loaded top to bottom.\r\n";

/// Reads the manifest. A missing, unreadable or empty file yields no entries,
/// which leaves the engine on its default configuration file.
pub fn read(config_dir: &Path) -> BTreeMap<String, Vec<String>> {
    let Some(bytes) = read_sharing_retry(&manifest_path(config_dir)) else {
        return BTreeMap::new();
    };
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    };
    parse(&text)
}

/// The files a device must load, in order. An unknown device identity never
/// matches, so callers fall back to the default configuration file.
pub fn files_for(config_dir: &Path, device_guid: &str) -> Vec<PathBuf> {
    if device_guid.trim().is_empty() {
        return Vec::new();
    }
    let guid = normalize_guid(device_guid);
    let Some(files) = read(config_dir).remove(&guid) else {
        return Vec::new();
    };
    files
        .iter()
        .map(|entry| resolve(config_dir, entry))
        .filter(|path| path.is_file())
        .collect()
}

/// Replaces one device's entries, keeping every other section intact.
/// Returns whether anything was written.
pub fn update_device(
    config_dir: &Path,
    device_guid: &str,
    files: &[PathBuf],
) -> io::Result<bool> {
    if device_guid.trim().is_empty() {
        return Ok(false);
    }
    let guid = normalize_guid(device_guid);
    let mut map = read(config_dir);
    let entries: Vec<String> = files
        .iter()
        .map(|path| path.to_string_lossy().into_owned())
        .collect();
    if entries.is_empty() {
        map.remove(&guid);
    } else {
        map.insert(guid, entries);
    }
    let text = render(&map);
    let path = manifest_path(config_dir);
    if let Some(existing) = read_sharing_retry(&path) {
        if existing.as_slice() == text.as_bytes() {
            return Ok(false);
        }
    } else if map.is_empty() {
        // Nothing recorded and nothing on disk: don't create the manifest.
        return Ok(false);
    }
    write_sharing_retry(&path, text.as_bytes())?;
    Ok(true)
}

/// Removes one device's entries; the device goes back to the default file.
pub fn remove_device(config_dir: &Path, device_guid: &str) -> io::Result<bool> {
    update_device(config_dir, device_guid, &[])
}

fn read_sharing_retry(path: &Path) -> Option<Vec<u8>> {
    let mut attempts = 0;
    loop {
        match std::fs::read(path) {
            Ok(bytes) => return Some(bytes),
            Err(error)
                if matches!(error.raw_os_error(), Some(32) | Some(5)) && attempts < SHARE_RETRIES =>
            {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(_) => return None,
        }
    }
}

/// Writes through a temporary file so a reader never observes a half-written
/// manifest, which would make the engine load an unintended configuration.
fn write_sharing_retry(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| MANIFEST_NAME.to_string());
    let temp = path.with_file_name(format!("{name}.tmp"));
    std::fs::write(&temp, bytes)?;
    let mut attempts = 0;
    loop {
        match std::fs::rename(&temp, path) {
            Ok(()) => return Ok(()),
            Err(error)
                if matches!(error.raw_os_error(), Some(32) | Some(5)) && attempts < SHARE_RETRIES =>
            {
                attempts += 1;
                std::thread::sleep(Duration::from_millis(1));
            }
            Err(error) => {
                let _ = std::fs::remove_file(&temp);
                return Err(error);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_dir(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "eapo_device_files_{tag}_{}_{tag}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn guid(tag: &str) -> String {
        format!("{{{tag}-0000-0000-0000-000000000000}}")
    }

    #[test]
    fn roundtrips_a_manifest() {
        let dir = temp_dir("roundtrip");
        let first = guid("a");
        let second = guid("b");
        std::fs::write(dir.join("zmq.txt"), "Preamp: -6 dB").unwrap();
        update_device(
            &dir,
            &first,
            &[PathBuf::from("zmq.txt"), PathBuf::from("D:\\other.txt")],
        )
        .unwrap();
        update_device(&dir, &second, &[PathBuf::from("bass.txt")]).unwrap();

        let map = read(&dir);
        assert_eq!(map.get(&first).unwrap().len(), 2);
        assert_eq!(map.get(&second).unwrap().len(), 1);
        assert!(!dir.join("device-files.cfg.tmp").exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn comments_crlf_and_stray_lines_are_tolerated() {
        let device = guid("a").to_lowercase();
        let text = format!("\u{feff}# comment\r\n[{device}]\r\n\r\n  zmq.txt \r\noops]\r\n");
        let map = parse(&text);
        assert_eq!(map.len(), 1, "a section without entries is dropped");
        assert_eq!(map[&device], vec!["zmq.txt".to_string()]);
    }

    #[test]
    fn guid_lookup_is_case_insensitive() {
        let dir = temp_dir("case");
        let upper = guid("c").to_uppercase();
        std::fs::write(dir.join("zmq.txt"), "Preamp: -6 dB").unwrap();
        update_device(&dir, &upper, &[PathBuf::from("zmq.txt")]).unwrap();
        let found = files_for(&dir, &guid("c").to_lowercase());
        assert_eq!(found, vec![dir.join("zmq.txt")]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_guid_never_matches_or_writes() {
        let dir = temp_dir("empty");
        assert!(files_for(&dir, "").is_empty());
        assert!(files_for(&dir, "   ").is_empty());
        assert_eq!(update_device(&dir, "", &[PathBuf::from("zmq.txt")]).unwrap(), false);
        assert!(!manifest_path(&dir).exists());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn update_keeps_other_devices() {
        let dir = temp_dir("others");
        let first = guid("d");
        let second = guid("e");
        update_device(&dir, &first, &[PathBuf::from("one.txt")]).unwrap();
        update_device(&dir, &second, &[PathBuf::from("two.txt")]).unwrap();
        update_device(&dir, &first, &[PathBuf::from("three.txt")]).unwrap();
        let map = read(&dir);
        assert_eq!(map[&first], vec!["three.txt".to_string()]);
        assert_eq!(map[&second], vec!["two.txt".to_string()]);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn update_skips_identical_content() {
        let dir = temp_dir("skip");
        let device = guid("f");
        update_device(&dir, &device, &[PathBuf::from("one.txt")]).unwrap();
        let written = std::fs::metadata(manifest_path(&dir)).unwrap().modified().unwrap();
        std::thread::sleep(Duration::from_millis(20));
        assert_eq!(
            update_device(&dir, &device, &[PathBuf::from("one.txt")]).unwrap(),
            false
        );
        assert_eq!(
            std::fs::metadata(manifest_path(&dir)).unwrap().modified().unwrap(),
            written
        );
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn remove_device_restores_the_default() {
        let dir = temp_dir("remove");
        let device = guid("g");
        update_device(&dir, &device, &[PathBuf::from("one.txt")]).unwrap();
        assert!(remove_device(&dir, &device).unwrap());
        assert!(read(&dir).is_empty());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn missing_files_are_dropped_from_the_list() {
        let dir = temp_dir("missing");
        let device = guid("h");
        std::fs::write(dir.join("real.txt"), "Preamp: -6 dB").unwrap();
        update_device(&dir, &device, &[PathBuf::from("real.txt"), PathBuf::from("gone.txt")])
            .unwrap();
        assert_eq!(files_for(&dir, &device), vec![dir.join("real.txt")]);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
