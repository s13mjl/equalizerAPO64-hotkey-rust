//! Registry access used by the installer/uninstaller.
//!
//! `Setup.nsi` runs with `SetRegView 64` for the x64 installer and writes to
//! the 64-bit view of the registry. Every operation below uses
//! `KEY_WOW64_64KEY`, like `RegistryHelper` in the C++ tree.
//!
//! The [`RegistryStore`] trait keeps the install/uninstall decision logic
//! testable with an in-memory mock (no real registry writes in tests).

use windows::core::PCWSTR;
use windows::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyExW, RegDeleteTreeW, RegDeleteValueW, RegOpenKeyExW,
    RegQueryValueExW, RegSetValueExW, HKEY, HKEY_CURRENT_CONFIG, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_ALL_ACCESS, KEY_QUERY_VALUE, KEY_SET_VALUE,
    KEY_WOW64_64KEY, REG_DWORD, REG_OPTION_NON_VOLATILE, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
};

use crate::util::{error_text, wide};

/// `!define REGPATH "Software\EqualizerAPO"`.
#[allow(dead_code)]
pub const REGPATH: &str = r"SOFTWARE\EqualizerAPO";
/// `!define UNINST_REGPATH "Software\Microsoft\Windows\CurrentVersion\Uninstall\EqualizerAPO"`.
#[allow(dead_code)]
pub const UNINST_REGPATH: &str =
    r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\EqualizerAPO";
/// Protected Audio DG workaround key used by `Setup.nsi`.
#[allow(dead_code)]
pub const AUDIO_REGPATH: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Audio";

pub const HKLM_REGPATH: &str = r"HKEY_LOCAL_MACHINE\SOFTWARE\EqualizerAPO";
pub const HKLM_UNINST_REGPATH: &str =
    r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\EqualizerAPO";
pub const HKLM_AUDIO_REGPATH: &str =
    r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Audio";
pub const HKCU_REGPATH: &str = r"HKEY_CURRENT_USER\SOFTWARE\EqualizerAPO";

/// Registry abstraction.  Keys are full paths such as
/// `HKEY_LOCAL_MACHINE\SOFTWARE\EqualizerAPO`.
pub trait RegistryStore {
    fn read_string(&self, key: &str, value_name: &str) -> Option<String>;
    #[allow(dead_code)]
    fn read_dword(&self, key: &str, value_name: &str) -> Option<u32>;
    fn write_string(&self, key: &str, value_name: &str, value: &str) -> Result<(), String>;
    fn write_dword(&self, key: &str, value_name: &str, value: u32) -> Result<(), String>;
    fn delete_value(&self, key: &str, value_name: &str) -> Result<(), String>;
    fn delete_key_tree(&self, key: &str) -> Result<(), String>;
    fn delete_key_if_empty(&self, key: &str) -> Result<(), String>;
}

/// Real registry implementation.
#[derive(Debug, Default, Clone, Copy)]
pub struct WinRegistry;

impl WinRegistry {
    pub fn new() -> Self {
        Self
    }
}

fn split_key(key: &str) -> Result<(HKEY, String), String> {
    let Some(position) = key.find('\\') else {
        return Err(format!("Key {key} has invalid format"));
    };
    let root_name = &key[..position];
    let sub_key = key[position + 1..].to_string();
    let root = match root_name.to_ascii_uppercase().as_str() {
        "HKEY_LOCAL_MACHINE" | "HKLM" => HKEY_LOCAL_MACHINE,
        "HKEY_CURRENT_USER" | "HKCU" => HKEY_CURRENT_USER,
        "HKEY_USERS" | "HKU" => HKEY_USERS,
        "HKEY_CURRENT_CONFIG" | "HKCC" => HKEY_CURRENT_CONFIG,
        _ => return Err(format!("Unknown root key {root_name}")),
    };
    Ok((root, sub_key))
}

struct KeyHandle(HKEY);

impl Drop for KeyHandle {
    fn drop(&mut self) {
        if !self.0.is_invalid() {
            unsafe {
                let _ = RegCloseKey(self.0);
            }
        }
    }
}

fn open_key(key: &str, access: REG_SAM_FLAGS) -> Result<KeyHandle, String> {
    let (root, sub_key) = split_key(key)?;
    let sub_key = wide(&sub_key);
    let mut handle = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            root,
            PCWSTR(sub_key.as_ptr()),
            None,
            access | KEY_WOW64_64KEY,
            &mut handle,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "Error while opening registry key {key}: {}",
            error_text(status)
        ));
    }
    Ok(KeyHandle(handle))
}

fn create_key(key: &str) -> Result<KeyHandle, String> {
    let (root, sub_key) = split_key(key)?;
    let sub_key = wide(&sub_key);
    let mut handle = HKEY::default();
    let status = unsafe {
        RegCreateKeyExW(
            root,
            PCWSTR(sub_key.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_ALL_ACCESS | KEY_WOW64_64KEY,
            None,
            &mut handle,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(format!(
            "Error while creating registry key {key}: {}",
            error_text(status)
        ));
    }
    Ok(KeyHandle(handle))
}

fn query_value(
    handle: &KeyHandle,
    key: &str,
    value_name: &str,
) -> Option<(REG_VALUE_TYPE, Vec<u8>)> {
    let name = wide(value_name);
    let mut value_type = REG_VALUE_TYPE(0);
    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            handle.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut value_type),
            None,
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }

    let mut data = vec![0u8; size as usize + 2];
    let mut data_size = data.len() as u32;
    let status = unsafe {
        RegQueryValueExW(
            handle.0,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut value_type),
            Some(data.as_mut_ptr()),
            Some(&mut data_size),
        )
    };
    if status != ERROR_SUCCESS {
        return None;
    }
    data.truncate(data_size as usize);
    let _ = key;
    Some((value_type, data))
}

fn decode_string(data: &[u8]) -> String {
    let mut units: Vec<u16> = data
        .chunks_exact(2)
        .map(|chunk| u16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    if units.last() == Some(&0) {
        units.pop();
    }
    String::from_utf16_lossy(&units)
}

impl RegistryStore for WinRegistry {
    fn read_string(&self, key: &str, value_name: &str) -> Option<String> {
        let handle = open_key(key, KEY_QUERY_VALUE).ok()?;
        let (value_type, data) = query_value(&handle, key, value_name)?;
        if value_type != REG_SZ {
            return None;
        }
        Some(decode_string(&data))
    }

    fn read_dword(&self, key: &str, value_name: &str) -> Option<u32> {
        let handle = open_key(key, KEY_QUERY_VALUE).ok()?;
        let (value_type, data) = query_value(&handle, key, value_name)?;
        if value_type != REG_DWORD || data.len() < 4 {
            return None;
        }
        Some(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
    }

    fn write_string(&self, key: &str, value_name: &str, value: &str) -> Result<(), String> {
        let handle = create_key(key)?;
        let name = wide(value_name);
        let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
        let bytes = unsafe {
            std::slice::from_raw_parts(
                data.as_ptr() as *const u8,
                data.len() * std::mem::size_of::<u16>(),
            )
        };
        let status =
            unsafe { RegSetValueExW(handle.0, PCWSTR(name.as_ptr()), None, REG_SZ, Some(bytes)) };
        if status != ERROR_SUCCESS {
            return Err(format!(
                "Error while writing registry value {key}\\{value_name}: {}",
                error_text(status)
            ));
        }
        Ok(())
    }

    fn write_dword(&self, key: &str, value_name: &str, value: u32) -> Result<(), String> {
        let handle = create_key(key)?;
        let name = wide(value_name);
        let bytes = value.to_le_bytes();
        let status = unsafe {
            RegSetValueExW(
                handle.0,
                PCWSTR(name.as_ptr()),
                None,
                REG_DWORD,
                Some(&bytes),
            )
        };
        if status != ERROR_SUCCESS {
            return Err(format!(
                "Error while writing registry value {key}\\{value_name}: {}",
                error_text(status)
            ));
        }
        Ok(())
    }

    fn delete_value(&self, key: &str, value_name: &str) -> Result<(), String> {
        let Ok(handle) = open_key(key, KEY_SET_VALUE) else {
            return Ok(());
        };
        let name = wide(value_name);
        let status = unsafe { RegDeleteValueW(handle.0, PCWSTR(name.as_ptr())) };
        if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
            return Err(format!(
                "Error while deleting registry value {key}\\{value_name}: {}",
                error_text(status)
            ));
        }
        Ok(())
    }

    fn delete_key_tree(&self, key: &str) -> Result<(), String> {
        let (root, sub_key) = split_key(key)?;
        let sub_key = wide(&sub_key);
        let status = unsafe { RegDeleteTreeW(root, PCWSTR(sub_key.as_ptr())) };
        if status != ERROR_SUCCESS && status != ERROR_FILE_NOT_FOUND {
            return Err(format!(
                "Error while deleting registry key {key}: {}",
                error_text(status)
            ));
        }
        unsafe {
            let _ = RegDeleteKeyExW(root, PCWSTR(sub_key.as_ptr()), KEY_WOW64_64KEY.0, None);
        }
        Ok(())
    }

    fn delete_key_if_empty(&self, key: &str) -> Result<(), String> {
        let (root, sub_key) = split_key(key)?;
        let sub_key = wide(&sub_key);
        // RegDeleteKeyEx fails with ERROR_KEY_HAS_CHILDREN / values when the
        // key is not empty, matching NSIS' DeleteRegKey /ifempty.
        let _ = unsafe { RegDeleteKeyExW(root, PCWSTR(sub_key.as_ptr()), KEY_WOW64_64KEY.0, None) };
        Ok(())
    }
}

/// In-memory registry used by unit tests.
#[cfg(test)]
pub mod testing {
    use std::cell::RefCell;
    use std::collections::{BTreeMap, BTreeSet};

    use super::*;

    #[derive(Default, Debug)]
    pub struct MockRegistry {
        pub values: RefCell<BTreeMap<String, BTreeMap<String, String>>>,
        pub dwords: RefCell<BTreeMap<String, BTreeMap<String, u32>>>,
        pub keys: RefCell<BTreeSet<String>>,
        pub deleted_keys: RefCell<BTreeSet<String>>,
    }

    impl MockRegistry {
        pub fn with_install_path(path: &str) -> Self {
            let registry = Self::default();
            registry
                .write_string(HKLM_REGPATH, "InstallPath", path)
                .unwrap();
            registry
        }

        pub fn set_string(&self, key: &str, value_name: &str, value: &str) {
            self.keys.borrow_mut().insert(key.to_string());
            self.values
                .borrow_mut()
                .entry(key.to_string())
                .or_default()
                .insert(value_name.to_string(), value.to_string());
        }

        pub fn set_dword(&self, key: &str, value_name: &str, value: u32) {
            self.keys.borrow_mut().insert(key.to_string());
            self.dwords
                .borrow_mut()
                .entry(key.to_string())
                .or_default()
                .insert(value_name.to_string(), value);
        }

        pub fn key_is_gone(&self, key: &str) -> bool {
            !self.keys.borrow().contains(key)
                && !self
                    .values
                    .borrow()
                    .keys()
                    .any(|candidate| candidate.eq_ignore_ascii_case(key))
                && !self
                    .dwords
                    .borrow()
                    .keys()
                    .any(|candidate| candidate.eq_ignore_ascii_case(key))
        }
    }

    impl RegistryStore for MockRegistry {
        fn read_string(&self, key: &str, value_name: &str) -> Option<String> {
            self.values
                .borrow()
                .get(key)
                .and_then(|values| values.get(value_name))
                .cloned()
        }

        fn read_dword(&self, key: &str, value_name: &str) -> Option<u32> {
            self.dwords
                .borrow()
                .get(key)
                .and_then(|values| values.get(value_name))
                .copied()
        }

        fn write_string(&self, key: &str, value_name: &str, value: &str) -> Result<(), String> {
            self.set_string(key, value_name, value);
            Ok(())
        }

        fn write_dword(&self, key: &str, value_name: &str, value: u32) -> Result<(), String> {
            self.set_dword(key, value_name, value);
            Ok(())
        }

        fn delete_value(&self, key: &str, value_name: &str) -> Result<(), String> {
            if let Some(values) = self.values.borrow_mut().get_mut(key) {
                values.remove(value_name);
            }
            if let Some(values) = self.dwords.borrow_mut().get_mut(key) {
                values.remove(value_name);
            }
            Ok(())
        }

        fn delete_key_tree(&self, key: &str) -> Result<(), String> {
            self.values.borrow_mut().remove(key);
            self.dwords.borrow_mut().remove(key);
            self.keys.borrow_mut().remove(key);
            self.deleted_keys.borrow_mut().insert(key.to_string());
            Ok(())
        }

        fn delete_key_if_empty(&self, key: &str) -> Result<(), String> {
            let has_values = self
                .values
                .borrow()
                .get(key)
                .map(|values| !values.is_empty())
                .unwrap_or(false)
                || self
                    .dwords
                    .borrow()
                    .get(key)
                    .map(|values| !values.is_empty())
                    .unwrap_or(false);
            if !has_values {
                self.values.borrow_mut().remove(key);
                self.dwords.borrow_mut().remove(key);
                self.keys.borrow_mut().remove(key);
                self.deleted_keys.borrow_mut().insert(key.to_string());
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_paths_match_setup_nsi() {
        assert_eq!(REGPATH, r"SOFTWARE\EqualizerAPO");
        assert_eq!(HKLM_REGPATH, r"HKEY_LOCAL_MACHINE\SOFTWARE\EqualizerAPO");
        assert_eq!(
            HKLM_UNINST_REGPATH,
            r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall\EqualizerAPO"
        );
        assert_eq!(
            HKLM_AUDIO_REGPATH,
            r"HKEY_LOCAL_MACHINE\SOFTWARE\Microsoft\Windows\CurrentVersion\Audio"
        );
    }

    #[test]
    fn split_key_accepts_long_and_short_roots() {
        assert_eq!(split_key(HKLM_REGPATH).unwrap().1, REGPATH);
        assert_eq!(split_key(r"HKLM\SOFTWARE\EqualizerAPO").unwrap().1, REGPATH);
        assert_eq!(
            split_key(r"HKCU\SOFTWARE\EqualizerAPO").unwrap().0,
            HKEY_CURRENT_USER
        );
        assert!(split_key("invalid").is_err());
    }
}
