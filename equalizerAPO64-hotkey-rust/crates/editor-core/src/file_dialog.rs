//! Small native Win32 file dialogs with an rfd-like builder API.
//!
//! The Qt project used `QFileDialog`; `GetOpenFileNameW`/`GetSaveFileNameW`
//! provide the same Windows dialogs without an extra dependency.

use std::mem::size_of;
use std::path::PathBuf;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::UI::Controls::Dialogs::{
    GetOpenFileNameW, GetSaveFileNameW, OPENFILENAMEW, OFN_ALLOWMULTISELECT, OFN_EXPLORER,
    OFN_FILEMUSTEXIST, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST,
};

pub struct FileDialog {
    title: String,
    directory: Option<PathBuf>,
    file_name: Option<String>,
    filters: Vec<(String, Vec<String>)>,
}

impl FileDialog {
    pub fn new() -> Self {
        Self { title: String::new(), directory: None, file_name: None, filters: Vec::new() }
    }

    pub fn add_filter(mut self, name: &str, extensions: &[&str]) -> Self {
        self.filters.push((name.to_string(), extensions.iter().map(|e| e.to_string()).collect()));
        self
    }

    pub fn set_title(mut self, title: &str) -> Self {
        self.title = title.to_string();
        self
    }

    pub fn set_directory(mut self, dir: impl AsRef<std::path::Path>) -> Self {
        self.directory = Some(dir.as_ref().to_path_buf());
        self
    }

    pub fn set_file_name(mut self, name: impl Into<String>) -> Self {
        self.file_name = Some(name.into());
        self
    }

    pub fn pick_file(self) -> Option<PathBuf> {
        self.show(false, false).and_then(|mut v| if v.is_empty() { None } else { Some(v.remove(0)) })
    }

    pub fn pick_files(self) -> Option<Vec<PathBuf>> {
        self.show(true, false)
    }

    pub fn save_file(self) -> Option<PathBuf> {
        self.show(false, true).and_then(|mut v| if v.is_empty() { None } else { Some(v.remove(0)) })
    }

    fn show(self, multiple: bool, save: bool) -> Option<Vec<PathBuf>> {
        let mut filter = Vec::<u16>::new();
        if self.filters.is_empty() {
            filter.extend("All files (*.*)".encode_utf16());
            filter.push(0);
            filter.extend("*.*".encode_utf16());
            filter.push(0);
        } else {
            for (name, exts) in &self.filters {
                filter.extend(name.encode_utf16());
                filter.push(0);
                let pattern = exts
                    .iter()
                    .map(|e| if e == "*" { "*.*".to_string() } else { format!("*.{e}") })
                    .collect::<Vec<_>>()
                    .join(";");
                filter.extend(pattern.encode_utf16());
                filter.push(0);
            }
        }
        filter.push(0);

        let title: Vec<u16> = self.title.encode_utf16().chain(std::iter::once(0)).collect();
        let dir: Vec<u16> = self
            .directory
            .as_ref()
            .map(|d| d.to_string_lossy().encode_utf16().chain(std::iter::once(0)).collect())
            .unwrap_or_default();
        let mut buffer = vec![0u16; if multiple { 32768 } else { 2048 }];
        if let Some(name) = &self.file_name {
            let units: Vec<u16> = name.encode_utf16().collect();
            let n = units.len().min(buffer.len() - 1);
            buffer[..n].copy_from_slice(&units[..n]);
        }

        let mut flags = OFN_EXPLORER | OFN_NOCHANGEDIR | OFN_PATHMUSTEXIST;
        if save {
            flags |= OFN_OVERWRITEPROMPT;
        } else {
            flags |= OFN_FILEMUSTEXIST;
            if multiple {
                flags |= OFN_ALLOWMULTISELECT;
            }
        }
        let mut ofn = OPENFILENAMEW {
            lStructSize: size_of::<OPENFILENAMEW>() as u32,
            lpstrFilter: PCWSTR(filter.as_ptr()),
            lpstrFile: PWSTR(buffer.as_mut_ptr()),
            nMaxFile: buffer.len() as u32,
            lpstrInitialDir: if dir.is_empty() { PCWSTR::null() } else { PCWSTR(dir.as_ptr()) },
            lpstrTitle: PCWSTR(title.as_ptr()),
            Flags: flags,
            nFilterIndex: 1,
            ..Default::default()
        };
        let ok = unsafe {
            if save { GetSaveFileNameW(&mut ofn) } else { GetOpenFileNameW(&mut ofn) }.as_bool()
        };
        if !ok {
            return None;
        }
        let len = buffer.iter().position(|&c| c == 0).unwrap_or(0);
        let first = String::from_utf16_lossy(&buffer[..len]);
        if !multiple {
            return Some(vec![PathBuf::from(first)]);
        }
        // Multi-select buffer: directory\0file1\0file2\0\0
        let mut rest: Vec<String> = Vec::new();
        let mut start = len + 1;
        while start < buffer.len() {
            let end = buffer[start..].iter().position(|&c| c == 0).unwrap_or(0) + start;
            if end == start {
                break;
            }
            rest.push(String::from_utf16_lossy(&buffer[start..end]));
            start = end + 1;
        }
        if rest.is_empty() {
            Some(vec![PathBuf::from(first)])
        } else {
            let base = PathBuf::from(first);
            Some(rest.into_iter().map(|f| base.join(f)).collect())
        }
    }
}

impl Default for FileDialog {
    fn default() -> Self {
        Self::new()
    }
}
