//! Reading and writing Equalizer APO configuration files.

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
    let mut attempts = 0;
    loop {
        match std::fs::write(path, &bytes) {
            Ok(()) => return Ok(()),
            Err(error) => {
                let sharing = error.raw_os_error() == Some(32);
                attempts += 1;
                if !sharing || attempts > 2000 {
                    return Err(error);
                }
                std::thread::sleep(Duration::from_millis(1));
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
                if !sharing || attempts > 2000 {
                    return Err(error);
                }
                std::thread::sleep(Duration::from_millis(1));
            }
        }
    };
    let text = match String::from_utf8(bytes) {
        Ok(text) => text,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    };
    let mut lines: Vec<String> = text
        .split('\n')
        .map(|line| line.strip_suffix('\r').unwrap_or(line).to_string())
        .collect();
    if lines.is_empty() {
        lines.push(String::new());
    }
    Ok(lines)
}
