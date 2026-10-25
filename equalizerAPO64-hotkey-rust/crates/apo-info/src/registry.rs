//! Internal port of `helpers/RegistryHelper.{h,cpp}`.
//!
//! `eapo-common` exposes the same public surface but is still a stub in this
//! workspace, so `apo-info` carries a private, fully functional copy. The
//! error type is the shared [`RegistryException`] so consumers see a single
//! error type for the whole crate family.
//!
//! All operations use `KEY_WOW64_64KEY`, exactly like the C++ implementation.

use std::fs::File;
use std::io::Write;
use std::sync::OnceLock;

use eapo_common::registry_helper::RegistryException;
use eapo_common::string_helper::get_system_error_string;
use windows::core::{GUID, HSTRING, PCWSTR};
use windows::Win32::Foundation::{LocalFree, HLOCAL, WIN32_ERROR};
use windows::Win32::Security::Authorization::{
    SetEntriesInAclW, EXPLICIT_ACCESS_W, SET_ACCESS, TRUSTEE_IS_GROUP, TRUSTEE_IS_SID, TRUSTEE_W,
};
use windows::Win32::Security::{
    AllocateAndInitializeSid, FreeSid, GetSecurityDescriptorDacl, InitializeSecurityDescriptor,
    SetSecurityDescriptorDacl, SetSecurityDescriptorOwner, ACL, DACL_SECURITY_INFORMATION,
    OWNER_SECURITY_INFORMATION, PSECURITY_DESCRIPTOR, PSID, SECURITY_DESCRIPTOR,
    SECURITY_NT_AUTHORITY, SE_PRIVILEGE_ENABLED, SE_TAKE_OWNERSHIP_NAME,
    SUB_CONTAINERS_AND_OBJECTS_INHERIT, TOKEN_ADJUST_PRIVILEGES, TOKEN_PRIVILEGES, TOKEN_QUERY,
};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyExW, RegDeleteValueW, RegEnumKeyExW,
    RegGetKeySecurity, RegOpenKeyExW, RegQueryInfoKeyW, RegQueryValueExW, RegSetKeySecurity,
    RegSetValueExW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_CONFIG, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE, KEY_SET_VALUE,
    KEY_WOW64_64KEY, REG_BINARY, REG_DWORD, REG_MULTI_SZ, REG_OPEN_CREATE_OPTIONS, REG_SAM_FLAGS,
    REG_SZ, REG_VALUE_TYPE,
};
use windows::Win32::System::SystemServices::{
    DOMAIN_ALIAS_RID_ADMINS, SECURITY_BUILTIN_DOMAIN_RID, SECURITY_DESCRIPTOR_REVISION,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

const ERROR_SUCCESS: WIN32_ERROR = WIN32_ERROR(0);
const ERROR_NO_MORE_ITEMS: WIN32_ERROR = WIN32_ERROR(259);

const KEY_WRITE_OWNER: REG_SAM_FLAGS = REG_SAM_FLAGS(0x0008_0000);
const KEY_WRITE_DAC: REG_SAM_FLAGS = REG_SAM_FLAGS(0x0004_0000);
const KEY_READ_CONTROL: REG_SAM_FLAGS = REG_SAM_FLAGS(0x0002_0000);

struct HKey(HKEY);

impl Drop for HKey {
    fn drop(&mut self) {
        unsafe {
            let _ = RegCloseKey(self.0);
        }
    }
}

impl HKey {
    fn raw(&self) -> HKEY {
        self.0
    }
}

pub(crate) fn split_key(key: &str) -> Result<(HKEY, String), RegistryException> {
    let pos = key
        .find('\\')
        .ok_or_else(|| RegistryException(format!("Key {key} has invalid format")))?;
    let root_part = &key[..pos];
    let path_part = &key[pos + 1..];
    let root = match root_part.to_uppercase().as_str() {
        "HKEY_CLASSES_ROOT" => HKEY_CLASSES_ROOT,
        "HKEY_CURRENT_CONFIG" => HKEY_CURRENT_CONFIG,
        "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
        "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        "HKEY_USERS" => HKEY_USERS,
        _ => return Err(RegistryException(format!("Unknown root key {root_part}"))),
    };
    Ok((root, path_part.to_string()))
}

fn open_key(key: &str, sam_desired: REG_SAM_FLAGS) -> Result<HKey, RegistryException> {
    let (root, sub_key) = split_key(key)?;
    let mut handle = HKEY::default();
    let sub = HSTRING::from(sub_key);
    let status = unsafe { RegOpenKeyExW(root, &sub, None, sam_desired, &mut handle) };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while opening registry key {key}: {}",
            get_system_error_string(status.0 as i32)
        )));
    }
    Ok(HKey(handle))
}

fn query_raw(
    key: &str,
    valuename: &str,
    needed_type: REG_VALUE_TYPE,
    open_flags: REG_SAM_FLAGS,
) -> Result<(HKey, Vec<u8>), RegistryException> {
    let handle = open_key(key, open_flags)?;
    let name = HSTRING::from(valuename);
    let mut value_type = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            handle.raw(),
            &name,
            None,
            Some(&mut value_type),
            None,
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while reading registry value {key}\\{valuename}: {}",
            get_system_error_string(status.0 as i32)
        )));
    }
    if value_type != needed_type {
        return Err(RegistryException(format!(
            "Registry value {key}\\{valuename} has wrong type"
        )));
    }
    let mut data = vec![0u8; size as usize];
    let status = unsafe {
        RegQueryValueExW(
            handle.raw(),
            &name,
            None,
            None,
            Some(data.as_mut_ptr()),
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while reading registry value {key}\\{valuename}: {}",
            get_system_error_string(status.0 as i32)
        )));
    }
    data.truncate(size as usize);
    Ok((handle, data))
}

pub(crate) fn read_value(key: &str, valuename: &str) -> Result<String, RegistryException> {
    let (_handle, data) = query_raw(key, valuename, REG_SZ, KEY_QUERY_VALUE | KEY_WOW64_64KEY)?;
    let mut chars: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    // Remove zero-termination
    if chars.last() == Some(&0) {
        chars.pop();
    }
    Ok(String::from_utf16_lossy(&chars))
}

pub(crate) fn read_dword_value(key: &str, valuename: &str) -> Result<u32, RegistryException> {
    let (_handle, data) = query_raw(key, valuename, REG_DWORD, KEY_QUERY_VALUE | KEY_WOW64_64KEY)?;
    if data.len() < 4 {
        return Err(RegistryException(format!(
            "Registry value {key}\\{valuename} has wrong type"
        )));
    }
    Ok(u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
}

#[allow(dead_code)]
pub(crate) fn read_multi_value(
    key: &str,
    valuename: &str,
) -> Result<Vec<String>, RegistryException> {
    let (_handle, data) = query_raw(
        key,
        valuename,
        REG_MULTI_SZ,
        KEY_QUERY_VALUE | KEY_WOW64_64KEY,
    )?;
    let mut chars: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    while chars.last() == Some(&0) {
        chars.pop();
    }
    let mut result = Vec::new();
    let mut start = 0usize;
    for (i, ch) in chars.iter().enumerate() {
        if *ch == 0 {
            result.push(String::from_utf16_lossy(&chars[start..i]));
            start = i + 1;
        }
    }
    if chars.len() > start {
        result.push(String::from_utf16_lossy(&chars[start..]));
    }
    Ok(result)
}

pub(crate) fn read_binary_value(key: &str, valuename: &str) -> Result<Vec<u8>, RegistryException> {
    let (_handle, data) = query_raw(
        key,
        valuename,
        REG_BINARY,
        KEY_QUERY_VALUE | KEY_WOW64_64KEY,
    )?;
    Ok(data)
}

fn set_value(
    key: &str,
    valuename: &str,
    value_type: REG_VALUE_TYPE,
    data: &[u8],
) -> Result<(), RegistryException> {
    let handle = open_key(key, KEY_SET_VALUE | KEY_WOW64_64KEY)?;
    let name = HSTRING::from(valuename);
    let status = unsafe { RegSetValueExW(handle.raw(), &name, None, value_type, Some(data)) };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while writing to registry value {key}\\{valuename}: {}",
            get_system_error_string(status.0 as i32)
        )));
    }
    Ok(())
}

pub(crate) fn write_value(
    key: &str,
    valuename: &str,
    value: &str,
) -> Result<(), RegistryException> {
    let mut data: Vec<u8> = value.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    data.extend_from_slice(&[0, 0]);
    set_value(key, valuename, REG_SZ, &data)
}

pub(crate) fn write_dword_value(
    key: &str,
    valuename: &str,
    value: u32,
) -> Result<(), RegistryException> {
    set_value(key, valuename, REG_DWORD, &value.to_le_bytes())
}

pub(crate) fn write_multi_value(
    key: &str,
    valuename: &str,
    value: &str,
) -> Result<(), RegistryException> {
    let mut data: Vec<u8> = value.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    data.extend_from_slice(&[0, 0, 0, 0]);
    set_value(key, valuename, REG_MULTI_SZ, &data)
}

#[allow(dead_code)]
pub(crate) fn write_multi_values(
    key: &str,
    valuename: &str,
    values: &[String],
) -> Result<(), RegistryException> {
    let mut data = Vec::new();
    for value in values {
        for c in value.encode_utf16() {
            data.extend_from_slice(&c.to_le_bytes());
        }
        data.extend_from_slice(&[0, 0]);
    }
    data.extend_from_slice(&[0, 0]);
    set_value(key, valuename, REG_MULTI_SZ, &data)
}

pub(crate) fn delete_value(key: &str, valuename: &str) -> Result<(), RegistryException> {
    let handle = open_key(key, KEY_SET_VALUE | KEY_WOW64_64KEY)?;
    let name = HSTRING::from(valuename);
    let status = unsafe { RegDeleteValueW(handle.raw(), &name) };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while deleting registry value {key}\\{valuename}: {}",
            get_system_error_string(status.0 as i32)
        )));
    }
    Ok(())
}

pub(crate) fn create_key(key: &str) -> Result<(), RegistryException> {
    let (root, sub_key) = split_key(key)?;
    let mut handle = HKEY::default();
    let sub = HSTRING::from(sub_key);
    let status = unsafe {
        RegCreateKeyExW(
            root,
            &sub,
            None,
            PCWSTR::null(),
            REG_OPEN_CREATE_OPTIONS(0),
            KEY_SET_VALUE | KEY_WOW64_64KEY,
            None,
            &mut handle,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while creating registry key {key}: {}",
            get_system_error_string(status.0 as i32)
        )));
    }
    unsafe {
        let _ = RegCloseKey(handle);
    }
    Ok(())
}

pub(crate) fn delete_key(key: &str) -> Result<(), RegistryException> {
    let (root, sub_key) = split_key(key)?;
    let sub = HSTRING::from(sub_key);
    let status = unsafe { RegDeleteKeyExW(root, &sub, KEY_WOW64_64KEY.0, None) };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while deleting registry key {key}: {}",
            get_system_error_string(status.0 as i32)
        )));
    }
    Ok(())
}

pub(crate) fn key_exists(key: &str) -> bool {
    let Ok((root, sub_key)) = split_key(key) else {
        return false;
    };
    let sub = HSTRING::from(sub_key);
    let mut handle = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            root,
            &sub,
            None,
            KEY_QUERY_VALUE | KEY_WOW64_64KEY,
            &mut handle,
        )
    };
    if status != ERROR_SUCCESS {
        return false;
    }
    unsafe {
        let _ = RegCloseKey(handle);
    }
    true
}

/// `RegQueryValueExW`-based existence check. Returns `false` when the key or
/// value cannot be opened/queried (the C++ helper throws in the former case;
/// callers always guard with `key_exists` where that difference matters).
pub(crate) fn value_exists(key: &str, valuename: &str) -> bool {
    let Ok(handle) = open_key(key, KEY_QUERY_VALUE | KEY_WOW64_64KEY) else {
        return false;
    };
    let name = HSTRING::from(valuename);
    let mut value_type = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            handle.raw(),
            &name,
            None,
            Some(&mut value_type),
            None,
            Some(&mut size),
        )
    };
    status == ERROR_SUCCESS
}

pub(crate) fn enum_sub_keys(key: &str) -> Result<Vec<String>, RegistryException> {
    let handle = open_key(key, KEY_ENUMERATE_SUB_KEYS | KEY_WOW64_64KEY)?;
    let mut result = Vec::new();
    let mut buffer = [0u16; 256];
    let mut index = 0u32;
    loop {
        let mut length = buffer.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                handle.raw(),
                index,
                Some(windows::core::PWSTR(buffer.as_mut_ptr())),
                &mut length,
                None,
                None,
                None,
                None,
            )
        };
        if status == ERROR_SUCCESS {
            index += 1;
            result.push(String::from_utf16_lossy(&buffer[..length as usize]));
            continue;
        }
        if status == ERROR_NO_MORE_ITEMS {
            break;
        }
        return Err(RegistryException(format!(
            "Error while enumerating sub keys of registry key {key}: {}",
            get_system_error_string(status.0 as i32)
        )));
    }
    Ok(result)
}

pub(crate) fn key_empty(key: &str) -> Result<bool, RegistryException> {
    let handle = open_key(key, KEY_QUERY_VALUE | KEY_WOW64_64KEY)?;
    let mut sub_keys = 0u32;
    let mut values = 0u32;
    let status = unsafe {
        RegQueryInfoKeyW(
            handle.raw(),
            None,
            None,
            None,
            Some(&mut sub_keys),
            None,
            None,
            Some(&mut values),
            None,
            None,
            None,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while reading info for registry key {key}: {}",
            get_system_error_string(status.0 as i32)
        )));
    }
    Ok(sub_keys == 0 && values == 0)
}

/// Faithful port of `RegistryHelper::saveToFile`, including its historical
/// `HKEY_LOCAL_MACHINE\` prefixing of a key that already contains a root path
/// (the original writes the same text). The C++ code writes through a
/// `std::wofstream` with the default "C" locale; this port writes UTF-8.
pub(crate) fn save_to_file(
    key: &str,
    valuenames: &[String],
    filepath: &str,
) -> Result<(), RegistryException> {
    let mut file = File::create(filepath).map_err(|_| {
        RegistryException(format!("Error while opening file {filepath} for writing"))
    })?;
    let write_err =
        |_| RegistryException(format!("Error while opening file {filepath} for writing"));
    // std::wofstream opens in text mode, so the original backup uses CRLF.
    write!(file, "Windows Registry Editor Version 5.00\r\n\r\n").map_err(write_err)?;
    write!(file, "[HKEY_LOCAL_MACHINE\\{key}]\r\n").map_err(write_err)?;
    for valuename in valuenames {
        let value = read_value(key, valuename)?;
        write!(file, "\"{valuename}\"=\"{value}\"\r\n").map_err(write_err)?;
    }
    write!(file, "\r\n").map_err(write_err)?;
    Ok(())
}

pub(crate) fn get_guid_string(guid: GUID) -> String {
    format!(
        "{{{:08X}-{:04X}-{:04X}-{:02X}{:02X}-{:02X}{:02X}{:02X}{:02X}{:02X}{:02X}}}",
        guid.data1,
        guid.data2,
        guid.data3,
        guid.data4[0],
        guid.data4[1],
        guid.data4[2],
        guid.data4[3],
        guid.data4[4],
        guid.data4[5],
        guid.data4[6],
        guid.data4[7]
    )
}

static WINDOWS_VERSION: OnceLock<u32> = OnceLock::new();

fn product_version_ms() -> u32 {
    unsafe {
        let mut handle = 0u32;
        let size = GetFileVersionInfoSizeW(windows::core::w!("kernel32.dll"), Some(&mut handle));
        if size == 0 {
            return 0;
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(
            windows::core::w!("kernel32.dll"),
            Some(handle),
            size,
            data.as_mut_ptr() as *mut core::ffi::c_void,
        )
        .is_err()
        {
            return 0;
        }
        let mut info: *mut core::ffi::c_void = std::ptr::null_mut();
        let mut len = 0u32;
        if !VerQueryValueW(
            data.as_ptr() as *const core::ffi::c_void,
            windows::core::w!("\\"),
            &mut info,
            &mut len,
        )
        .as_bool()
            || info.is_null()
            || (len as usize) < std::mem::size_of::<VS_FIXEDFILEINFO>()
        {
            return 0;
        }
        let fixed = std::ptr::read_unaligned(info as *const VS_FIXEDFILEINFO);
        fixed.dwProductVersionMS
    }
}

/// Port of `RegistryHelper::isWindowsVersionAtLeast`. Caches `kernel32.dll`'s
/// product version exactly like the original.
pub(crate) fn is_windows_version_at_least(major: u32, minor: u32) -> bool {
    let version = *WINDOWS_VERSION.get_or_init(product_version_ms);
    let compare_version =
        ((major / 10) << 20) + ((major % 10) << 16) + ((minor / 10) << 4) + (minor % 10);
    version >= compare_version
}

fn open_process_token_for_privilege(
    privilege: PCWSTR,
) -> Result<(windows::Win32::Foundation::HANDLE, TOKEN_PRIVILEGES), RegistryException> {
    let mut token = windows::Win32::Foundation::HANDLE::default();
    unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY,
            &mut token,
        )
        .map_err(|_| {
            RegistryException("Error in OpenProcessToken while taking ownership".into())
        })?;
    }
    let mut luid = windows::Win32::Foundation::LUID::default();
    if unsafe {
        windows::Win32::Security::LookupPrivilegeValueW(PCWSTR::null(), privilege, &mut luid)
    }
    .is_err()
    {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(token);
        }
        return Err(RegistryException(
            "Error in LookupPrivilegeValue while taking ownership".into(),
        ));
    }
    let mut tp = TOKEN_PRIVILEGES::default();
    tp.PrivilegeCount = 1;
    tp.Privileges[0].Luid = luid;
    tp.Privileges[0].Attributes = SE_PRIVILEGE_ENABLED;
    if unsafe {
        windows::Win32::Security::AdjustTokenPrivileges(
            token,
            false,
            Some(&tp),
            std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
            None,
            None,
        )
    }
    .is_err()
    {
        unsafe {
            let _ = windows::Win32::Foundation::CloseHandle(token);
        }
        return Err(RegistryException(
            "Error in AdjustTokenPrivileges while taking ownership".into(),
        ));
    }
    Ok((token, tp))
}

/// Port of `RegistryHelper::takeOwnership`.
pub(crate) fn take_ownership(key: &str) -> Result<(), RegistryException> {
    let (token, mut tp) = open_process_token_for_privilege(SE_TAKE_OWNERSHIP_NAME)?;
    let close_token = || unsafe {
        let _ = windows::Win32::Foundation::CloseHandle(token);
    };

    let handle = match open_key(key, KEY_WRITE_OWNER | KEY_WOW64_64KEY) {
        Ok(h) => h,
        Err(e) => {
            close_token();
            return Err(e);
        }
    };

    let mut sd = SECURITY_DESCRIPTOR::default();
    let sd_ptr = PSECURITY_DESCRIPTOR(&mut sd as *mut _ as *mut core::ffi::c_void);
    if unsafe { InitializeSecurityDescriptor(sd_ptr, SECURITY_DESCRIPTOR_REVISION) }.is_err() {
        close_token();
        return Err(RegistryException(
            "Error in InitializeSecurityDescriptor while taking ownership".into(),
        ));
    }

    let mut sid = PSID::default();
    if unsafe {
        AllocateAndInitializeSid(
            &SECURITY_NT_AUTHORITY,
            2,
            SECURITY_BUILTIN_DOMAIN_RID as u32,
            DOMAIN_ALIAS_RID_ADMINS as u32,
            0,
            0,
            0,
            0,
            0,
            0,
            &mut sid,
        )
    }
    .is_err()
    {
        close_token();
        return Err(RegistryException(
            "Error in AllocateAndInitializeSid while taking ownership".into(),
        ));
    }

    let mut result = Ok(());
    if unsafe { SetSecurityDescriptorOwner(sd_ptr, Some(sid), false) }.is_err() {
        result = Err(RegistryException(
            "Error in SetSecurityDescriptorOwner while taking ownership".into(),
        ));
    } else {
        let status = unsafe { RegSetKeySecurity(handle.raw(), OWNER_SECURITY_INFORMATION, sd_ptr) };
        if status != ERROR_SUCCESS {
            result = Err(RegistryException(format!(
                "Error while setting security information for registry key {key}: {}",
                get_system_error_string(status.0 as i32)
            )));
        }
    }

    tp.Privileges[0].Attributes = windows::Win32::Security::TOKEN_PRIVILEGES_ATTRIBUTES(0);
    let _ = unsafe {
        windows::Win32::Security::AdjustTokenPrivileges(
            token,
            false,
            Some(&tp),
            std::mem::size_of::<TOKEN_PRIVILEGES>() as u32,
            None,
            None,
        )
    };
    unsafe {
        FreeSid(sid);
    }
    close_token();
    result
}

/// Port of `RegistryHelper::makeWritable`.
pub(crate) fn make_writable(key: &str) -> Result<(), RegistryException> {
    let handle = open_key(key, KEY_READ_CONTROL | KEY_WRITE_DAC | KEY_WOW64_64KEY)?;

    let mut descriptor_size = 0u32;
    unsafe {
        let _ = RegGetKeySecurity(
            handle.raw(),
            DACL_SECURITY_INFORMATION,
            None,
            &mut descriptor_size,
        );
    }
    let mut old_sd = vec![0u8; descriptor_size as usize];
    let old_sd_ptr = PSECURITY_DESCRIPTOR(old_sd.as_mut_ptr() as *mut core::ffi::c_void);
    let status = unsafe {
        RegGetKeySecurity(
            handle.raw(),
            DACL_SECURITY_INFORMATION,
            Some(old_sd_ptr),
            &mut descriptor_size,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while getting security information for registry key {key}: {}",
            get_system_error_string(status.0 as i32)
        )));
    }

    let mut acl_present = windows::core::BOOL(0);
    let mut old_acl: *mut ACL = std::ptr::null_mut();
    let mut acl_defaulted = windows::core::BOOL(0);
    if unsafe {
        GetSecurityDescriptorDacl(
            old_sd_ptr,
            &mut acl_present,
            &mut old_acl,
            &mut acl_defaulted,
        )
    }
    .is_err()
    {
        return Err(RegistryException(
            "Error in GetSecurityDescriptorDacl while ensuring writability".into(),
        ));
    }

    let mut sid = PSID::default();
    if unsafe {
        AllocateAndInitializeSid(
            &SECURITY_NT_AUTHORITY,
            2,
            SECURITY_BUILTIN_DOMAIN_RID as u32,
            DOMAIN_ALIAS_RID_ADMINS as u32,
            0,
            0,
            0,
            0,
            0,
            0,
            &mut sid,
        )
    }
    .is_err()
    {
        return Err(RegistryException(
            "Error in AllocateAndInitializeSid while ensuring writability".into(),
        ));
    }

    let ea = EXPLICIT_ACCESS_W {
        grfAccessPermissions: windows::Win32::System::Registry::KEY_ALL_ACCESS.0,
        grfAccessMode: SET_ACCESS,
        grfInheritance: SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        Trustee: TRUSTEE_W {
            pMultipleTrustee: std::ptr::null_mut(),
            MultipleTrusteeOperation: windows::Win32::Security::Authorization::NO_MULTIPLE_TRUSTEE,
            TrusteeForm: TRUSTEE_IS_SID,
            TrusteeType: TRUSTEE_IS_GROUP,
            ptstrName: windows::core::PWSTR(sid.0 as *mut u16),
        },
    };

    let mut acl: *mut ACL = std::ptr::null_mut();
    let status = unsafe { SetEntriesInAclW(Some(&[ea]), Some(old_acl), &mut acl) };
    if status != ERROR_SUCCESS {
        unsafe {
            FreeSid(sid);
        }
        return Err(RegistryException(
            "Error in SetEntriesInAcl while ensuring writability".into(),
        ));
    }

    let mut sd = SECURITY_DESCRIPTOR::default();
    let sd_ptr = PSECURITY_DESCRIPTOR(&mut sd as *mut _ as *mut core::ffi::c_void);
    if unsafe { InitializeSecurityDescriptor(sd_ptr, SECURITY_DESCRIPTOR_REVISION) }.is_err() {
        unsafe {
            FreeSid(sid);
            let _ = LocalFree(Some(HLOCAL(acl as *mut core::ffi::c_void)));
        }
        return Err(RegistryException(
            "Error in InitializeSecurityDescriptor while ensuring writability".into(),
        ));
    }
    if unsafe { SetSecurityDescriptorDacl(sd_ptr, true, Some(acl), false) }.is_err() {
        unsafe {
            FreeSid(sid);
            let _ = LocalFree(Some(HLOCAL(acl as *mut core::ffi::c_void)));
        }
        return Err(RegistryException(
            "Error in SetSecurityDescriptorDacl while ensuring writability".into(),
        ));
    }

    let status = unsafe { RegSetKeySecurity(handle.raw(), DACL_SECURITY_INFORMATION, sd_ptr) };
    unsafe {
        FreeSid(sid);
        let _ = LocalFree(Some(HLOCAL(acl as *mut core::ffi::c_void)));
    }
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while setting security information for registry key {key}: {}",
            get_system_error_string(status.0 as i32)
        )));
    }

    Ok(())
}
