//! Reading and writing Equalizer APO configuration files.

use std::path::Path;
use std::time::Duration;

pub fn save_config_file(path: &str, lines: &[String]) -> std::io::Result<()> {
    let mut text = String::new();
    for (i, line) in lines.iter().enumerate() {
        if i != 0 {
            text.push_str("\r\n");
        }
        text.push_str(line);
    }
    let bytes = text.into_bytes();

    // Prefer an atomic replace so a crash or a concurrent reader never sees a
    // half-written configuration.  If the target is locked without delete
    // sharing (the audio service may hold it), fall back to the old in-place
    // write below.
    let target = Path::new(path);
    let parent = target.parent().unwrap_or_else(|| Path::new("."));
    let name = target
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "config".to_string());
    let temp = parent.join(format!(".{name}.{}.tmp", std::process::id()));
    if std::fs::write(&temp, &bytes).is_ok() {
        if std::fs::rename(&temp, target).is_ok() {
            return Ok(());
        }
        let _ = std::fs::remove_file(&temp);
    }

    let mut attempts = 0;
    loop {
        match std::fs::write(path, &bytes) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let sharing = error.raw_os_error() == Some(32);
                attempts += 1;
                if !sharing || attempts > 200 {
                    return Err(error);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    }
}

pub fn load_config_lines(path: &str) -> std::io::Result<Vec<String>> {
    let mut attempts = 0;
    let bytes = loop {
        match std::fs::read(path) {
            Ok(bytes) => break bytes,
            Err(error) => {
                let sharing = error.raw_os_error() == Some(32);
                attempts += 1;
                if !sharing || attempts > 200 {
                    return Err(error);
                }
                std::thread::sleep(Duration::from_millis(10));
            }
        }
    };
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    };
    let text = text.strip_prefix('\u{feff}').unwrap_or(&text);
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect();
    if lines.is_empty() {
        lines.push(String::new());
    }
    Ok(lines)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_crlf_and_strips_bom_on_load() {
        let path = std::env::temp_dir().join(format!(
            "eapo_config_file_test_{}.txt",
            std::process::id()
        ));
        let _ = std::fs::remove_file(&path);
        let lines = vec![
            "Preamp: -6 dB".to_string(),
            "Filter: ON PK Fc 100 Hz Gain 3 dB Q 1".to_string(),
        ];
        save_config_file(path.to_string_lossy().as_ref(), &lines).unwrap();
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"Preamp: -6 dB\r\nFilter: ON PK Fc 100 Hz Gain 3 dB Q 1"
        );

        std::fs::write(&path, b"\xef\xbb\xbfPreamp: 0 dB").unwrap();
        let loaded = load_config_lines(path.to_string_lossy().as_ref()).unwrap();
        assert_eq!(loaded, vec!["Preamp: 0 dB".to_string()]);
        let _ = std::fs::remove_file(&path);
    }
}
