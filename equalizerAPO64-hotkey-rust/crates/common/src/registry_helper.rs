//! Port of `helpers/RegistryHelper.{h,cpp}`.
//!
//! Registry paths use the same textual form as the C++ code, e.g.
//! `HKEY_LOCAL_MACHINE\SOFTWARE\EqualizerAPO`.  The short forms `HKLM`,
//! `HKCU`, `HKCR`, `HKU` and `HKCC` are accepted as well (a small convenience
//! delta from the original, which only knew the long names).
//!
//! All operations use the 64 bit registry view (`KEY_WOW64_64KEY`), exactly
//! like the original implementation.  Errors are reported as
//! [`RegistryException`] with the same message wording as the C++ code.

use std::fmt;
use std::io::Write;
use std::mem::size_of;
use std::ptr::null_mut;
use std::sync::OnceLock;

use windows::core::{PCWSTR, PWSTR};
use windows::Win32::Foundation::{
    LocalFree, ERROR_NO_MORE_ITEMS, ERROR_SUCCESS, HANDLE, HLOCAL, LUID, WIN32_ERROR,
};
use windows::Win32::Security::Authorization::{
    AuthzAccessCheck, AuthzFreeContext, AuthzFreeResourceManager, AuthzInitializeContextFromSid,
    AuthzInitializeResourceManager, GetNamedSecurityInfoW, SetEntriesInAclW, AUTHZ_ACCESS_REPLY,
    AUTHZ_ACCESS_REQUEST, AUTHZ_RESOURCE_MANAGER_HANDLE, EXPLICIT_ACCESS_W, SET_ACCESS,
    SE_FILE_OBJECT, TRUSTEE_IS_GROUP, TRUSTEE_IS_SID, TRUSTEE_W,
};
use windows::Win32::Security::{
    AllocateAndInitializeSid, FreeSid, GetSecurityDescriptorDacl, InitializeSecurityDescriptor,
    SetSecurityDescriptorDacl, SetSecurityDescriptorOwner, ACL, PSECURITY_DESCRIPTOR, PSID,
    SECURITY_NT_AUTHORITY,
};
use windows::Win32::Storage::FileSystem::{
    GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW, VS_FIXEDFILEINFO,
};
use windows::Win32::System::Memory::{
    GetProcessHeap, HeapAlloc, HeapFree, LocalAlloc, HEAP_FLAGS, HEAP_ZERO_MEMORY, LPTR,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyExW, RegDeleteValueW, RegEnumKeyExW,
    RegGetKeySecurity, RegOpenKeyExW, RegQueryInfoKeyW, RegQueryValueExW, RegSetKeySecurity,
    RegSetValueExW, HKEY, HKEY_CLASSES_ROOT, HKEY_CURRENT_CONFIG, HKEY_CURRENT_USER,
    HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_ALL_ACCESS, KEY_ENUMERATE_SUB_KEYS, KEY_QUERY_VALUE,
    KEY_SET_VALUE, KEY_WOW64_64KEY, REG_BINARY, REG_DWORD, REG_MULTI_SZ, REG_OPTION_NON_VOLATILE,
    REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
};
use windows::Win32::System::SystemServices::{
    DOMAIN_ALIAS_RID_ADMINS, MAXIMUM_ALLOWED, SECURITY_BUILTIN_DOMAIN_RID,
    SECURITY_DESCRIPTOR_REVISION,
};
use windows::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

use crate::string_helper::{get_system_error_string, string_to_ansi};

/// Error type mirroring `RegistryException`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryException(pub String);

impl RegistryException {
    /// Equivalent of the C++ `getMessage()`.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for RegistryException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for RegistryException {}

pub type Result<T> = std::result::Result<T, RegistryException>;

/// `sizeof(SECURITY_DESCRIPTOR)` (40 bytes on x64, 20 on x86).
const SECURITY_DESCRIPTOR_MIN_LENGTH: usize =
    size_of::<windows::Win32::Security::SECURITY_DESCRIPTOR>();
/// `READ_CONTROL` (same numeric value for registry and file objects).
const READ_CONTROL_BITS: u32 = 0x0002_0000;
/// `WRITE_DAC`.
const WRITE_DAC_BITS: u32 = 0x0004_0000;
/// `WRITE_OWNER`.
const WRITE_OWNER_BITS: u32 = 0x0008_0000;

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn system_error(status: WIN32_ERROR) -> String {
    get_system_error_string(status.0 as i32)
}

fn read_error(key: &str, valuename: &str, status: WIN32_ERROR) -> RegistryException {
    RegistryException(format!(
        "Error while reading registry value {}\\{}: {}",
        key,
        valuename,
        system_error(status)
    ))
}

fn open_error(key: &str, status: WIN32_ERROR) -> RegistryException {
    RegistryException(format!(
        "Error while opening registry key {}: {}",
        key,
        system_error(status)
    ))
}

/// Closes a key opened with [`open_key`].  Equivalent to `RegCloseKey`.
pub fn close_key(key: HKEY) {
    unsafe {
        let _ = RegCloseKey(key);
    }
}

/// RAII wrapper used internally so early returns never leak handles.
struct KeyGuard(HKEY);

impl KeyGuard {
    fn new(key: HKEY) -> Self {
        Self(key)
    }

    fn raw(&self) -> HKEY {
        self.0
    }
}

impl Drop for KeyGuard {
    fn drop(&mut self) {
        close_key(self.0);
    }
}

/// Splits `HKEY_...\path` (or a short `HKLM\path`) into root key and sub path.
fn split_key(key: &str) -> Result<(HKEY, String)> {
    let Some(pos) = key.find('\\') else {
        return Err(RegistryException(format!("Key {} has invalid format", key)));
    };
    let root_part = &key[..pos];
    let path_part = key[pos + 1..].to_string();

    let upper = root_part.to_uppercase();
    let root = match upper.as_str() {
        "HKEY_CLASSES_ROOT" | "HKCR" => HKEY_CLASSES_ROOT,
        "HKEY_CURRENT_CONFIG" | "HKCC" => HKEY_CURRENT_CONFIG,
        "HKEY_CURRENT_USER" | "HKCU" => HKEY_CURRENT_USER,
        "HKEY_LOCAL_MACHINE" | "HKLM" => HKEY_LOCAL_MACHINE,
        "HKEY_USERS" | "HKU" => HKEY_USERS,
        _ => return Err(RegistryException(format!("Unknown root key {}", root_part))),
    };

    Ok((root, path_part))
}

/// Port of `RegistryHelper::openKey`.  Opens a key with the requested access
/// mask; the caller owns the returned handle and must call [`close_key`].
pub fn open_key(key: &str, sam_desired: REG_SAM_FLAGS) -> Result<HKEY> {
    let (root, sub_key) = split_key(key)?;
    let sub_wide = to_wide(&sub_key);
    let mut handle = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            root,
            PCWSTR(sub_wide.as_ptr()),
            None,
            sam_desired,
            &mut handle,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(open_error(key, status));
    }
    Ok(handle)
}

/// Reads the raw type and data of a registry value.
fn query_raw(key: HKEY, key_name: &str, valuename: &str) -> Result<(REG_VALUE_TYPE, Vec<u8>)> {
    let name = to_wide(valuename);
    let mut value_type = REG_VALUE_TYPE(0);
    let mut size = 0u32;

    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut value_type),
            None,
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(read_error(key_name, valuename, status));
    }

    // +2 bytes as in the original, so an ANSI string without terminator is
    // still safely terminated in the temporary buffer.
    let mut data = vec![0u8; size as usize + 2];
    let mut data_size = data.len() as u32;
    let status = unsafe {
        RegQueryValueExW(
            key,
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut value_type),
            Some(data.as_mut_ptr()),
            Some(&mut data_size),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(read_error(key_name, valuename, status));
    }
    data.truncate(data_size as usize);
    Ok((value_type, data))
}

fn check_type(
    key: &str,
    valuename: &str,
    actual: REG_VALUE_TYPE,
    expected: REG_VALUE_TYPE,
) -> Result<()> {
    if actual != expected {
        Err(RegistryException(format!(
            "Registry value {}\\{} has wrong type",
            key, valuename
        )))
    } else {
        Ok(())
    }
}

fn bytes_to_utf16(data: &[u8]) -> Vec<u16> {
    data.chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect()
}

/// Port of `RegistryHelper::readValue`.
pub fn read_value(key: &str, valuename: &str) -> Result<String> {
    let handle = KeyGuard::new(open_key(key, KEY_QUERY_VALUE | KEY_WOW64_64KEY)?);
    let (value_type, data) = query_raw(handle.raw(), key, valuename)?;
    check_type(key, valuename, value_type, REG_SZ)?;

    let mut units = bytes_to_utf16(&data);
    // The C++ code removes exactly one zero termination.
    if units.last() == Some(&0) {
        units.pop();
    }
    Ok(String::from_utf16_lossy(&units))
}

/// Port of `RegistryHelper::readDWORDValue`.  The C++ type is `unsigned long`,
/// which is a 32 bit value on Windows.
pub fn read_dword_value(key: &str, valuename: &str) -> Result<u32> {
    let handle = KeyGuard::new(open_key(key, KEY_QUERY_VALUE | KEY_WOW64_64KEY)?);
    let (value_type, data) = query_raw(handle.raw(), key, valuename)?;
    check_type(key, valuename, value_type, REG_DWORD)?;

    Ok(data
        .get(..4)
        .map(|b| u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
        .unwrap_or(0))
}

/// Port of `RegistryHelper::readMultiValue`.
pub fn read_multi_value(key: &str, valuename: &str) -> Result<Vec<String>> {
    let handle = KeyGuard::new(open_key(key, KEY_QUERY_VALUE | KEY_WOW64_64KEY)?);
    let (value_type, data) = query_raw(handle.raw(), key, valuename)?;
    check_type(key, valuename, value_type, REG_MULTI_SZ)?;

    let mut units = bytes_to_utf16(&data);
    while units.last() == Some(&0) {
        units.pop();
    }

    let mut result = Vec::new();
    let mut start = 0usize;
    for i in 0..units.len() {
        if units[i] == 0 {
            result.push(String::from_utf16_lossy(&units[start..i]));
            start = i + 1;
        }
    }
    if units.len() > start {
        result.push(String::from_utf16_lossy(&units[start..]));
    }
    Ok(result)
}

/// Port of `RegistryHelper::readBinaryValue`.
pub fn read_binary_value(key: &str, valuename: &str) -> Result<Vec<u8>> {
    let handle = KeyGuard::new(open_key(key, KEY_QUERY_VALUE | KEY_WOW64_64KEY)?);
    let (value_type, data) = query_raw(handle.raw(), key, valuename)?;
    check_type(key, valuename, value_type, REG_BINARY)?;
    Ok(data)
}

/// Port of `RegistryHelper::writeValue`.
pub fn write_value(key: &str, valuename: &str, value: &str) -> Result<()> {
    let handle = KeyGuard::new(open_key(key, KEY_SET_VALUE | KEY_WOW64_64KEY)?);
    let name = to_wide(valuename);
    let data: Vec<u16> = value.encode_utf16().chain(std::iter::once(0)).collect();
    let bytes = unsafe {
        std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * size_of::<u16>())
    };
    let status = unsafe {
        RegSetValueExW(
            handle.raw(),
            PCWSTR(name.as_ptr()),
            None,
            REG_SZ,
            Some(bytes),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while writing to registry value {}\\{}: {}",
            key,
            valuename,
            system_error(status)
        )));
    }
    Ok(())
}

/// Port of `RegistryHelper::writeDWORDValue`.
pub fn write_dword_value(key: &str, valuename: &str, value: u32) -> Result<()> {
    let handle = KeyGuard::new(open_key(key, KEY_SET_VALUE | KEY_WOW64_64KEY)?);
    let name = to_wide(valuename);
    let bytes = value.to_le_bytes();
    let status = unsafe {
        RegSetValueExW(
            handle.raw(),
            PCWSTR(name.as_ptr()),
            None,
            REG_DWORD,
            Some(&bytes),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while writing to registry value {}\\{}: {}",
            key,
            valuename,
            system_error(status)
        )));
    }
    Ok(())
}

/// Port of `RegistryHelper::writeMultiValue(key, valuename, vector<wstring>)`.
pub fn write_multi_value<S: AsRef<str>>(key: &str, valuename: &str, values: &[S]) -> Result<()> {
    let handle = KeyGuard::new(open_key(key, KEY_SET_VALUE | KEY_WOW64_64KEY)?);
    let name = to_wide(valuename);

    let mut data: Vec<u16> = Vec::new();
    for value in values {
        data.extend(value.as_ref().encode_utf16());
        data.push(0);
    }
    data.push(0);

    let bytes = unsafe {
        std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * size_of::<u16>())
    };
    let status = unsafe {
        RegSetValueExW(
            handle.raw(),
            PCWSTR(name.as_ptr()),
            None,
            REG_MULTI_SZ,
            Some(bytes),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while writing to registry value {}\\{}: {}",
            key,
            valuename,
            system_error(status)
        )));
    }
    Ok(())
}

/// Port of the single-string overload
/// `RegistryHelper::writeMultiValue(key, valuename, wstring)`.
pub fn write_multi_value_single(key: &str, valuename: &str, value: &str) -> Result<()> {
    write_multi_value_string(key, valuename, value)
}

/// Alias for [`write_multi_value_single`], named after the overload's argument.
pub fn write_multi_value_string(key: &str, valuename: &str, value: &str) -> Result<()> {
    let handle = KeyGuard::new(open_key(key, KEY_SET_VALUE | KEY_WOW64_64KEY)?);
    let name = to_wide(valuename);
    let data: Vec<u16> = value
        .encode_utf16()
        .chain(std::iter::once(0))
        .chain(std::iter::once(0))
        .collect();
    let bytes = unsafe {
        std::slice::from_raw_parts(data.as_ptr() as *const u8, data.len() * size_of::<u16>())
    };
    let status = unsafe {
        RegSetValueExW(
            handle.raw(),
            PCWSTR(name.as_ptr()),
            None,
            REG_MULTI_SZ,
            Some(bytes),
        )
    };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while writing to registry value {}\\{}: {}",
            key,
            valuename,
            system_error(status)
        )));
    }
    Ok(())
}

/// Port of `RegistryHelper::deleteValue`.
pub fn delete_value(key: &str, valuename: &str) -> Result<()> {
    let handle = KeyGuard::new(open_key(key, KEY_SET_VALUE | KEY_WOW64_64KEY)?);
    let name = to_wide(valuename);
    let status = unsafe { RegDeleteValueW(handle.raw(), PCWSTR(name.as_ptr())) };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while deleting registry value {}\\{}: {}",
            key,
            valuename,
            system_error(status)
        )));
    }
    Ok(())
}

/// Port of `RegistryHelper::createKey` (creates missing parent keys too).
pub fn create_key(key: &str) -> Result<()> {
    let (root, sub_key) = split_key(key)?;
    let sub_wide = to_wide(&sub_key);
    let mut handle = HKEY::default();
    let status = unsafe {
        RegCreateKeyExW(
            root,
            PCWSTR(sub_wide.as_ptr()),
            None,
            PCWSTR::null(),
            REG_OPTION_NON_VOLATILE,
            KEY_SET_VALUE | KEY_WOW64_64KEY,
            None,
            &mut handle,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while creating registry key {}: {}",
            key,
            system_error(status)
        )));
    }
    close_key(handle);
    Ok(())
}

/// Port of `RegistryHelper::deleteKey`.
pub fn delete_key(key: &str) -> Result<()> {
    let (root, sub_key) = split_key(key)?;
    let sub_wide = to_wide(&sub_key);
    let status =
        unsafe { RegDeleteKeyExW(root, PCWSTR(sub_wide.as_ptr()), KEY_WOW64_64KEY.0, None) };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while deleting registry key {}: {}",
            key,
            system_error(status)
        )));
    }
    Ok(())
}

/// Port of `RegistryHelper::makeWritable`.  Grants `KEY_ALL_ACCESS` to the
/// built-in Administrators group while preserving the existing DACL.
pub fn make_writable(key: &str) -> Result<()> {
    let handle = KeyGuard::new(open_key(
        key,
        REG_SAM_FLAGS(READ_CONTROL_BITS | WRITE_DAC_BITS) | KEY_WOW64_64KEY,
    )?);

    let mut descriptor_size = 0u32;
    unsafe {
        let _ = RegGetKeySecurity(
            handle.raw(),
            windows::Win32::Security::DACL_SECURITY_INFORMATION,
            None,
            &mut descriptor_size,
        );
    }
    if descriptor_size == 0 {
        return Err(RegistryException(
            "Error in RegGetKeySecurity while ensuring writability".into(),
        ));
    }

    let heap = unsafe { GetProcessHeap() }.map_err(|_| {
        RegistryException("Error in GetProcessHeap while ensuring writability".into())
    })?;
    let old_sd = unsafe { HeapAlloc(heap, HEAP_ZERO_MEMORY, descriptor_size as usize) };
    if old_sd.is_null() {
        return Err(RegistryException(
            "Error in HeapAlloc while ensuring writability".into(),
        ));
    }
    let old_sd_desc = PSECURITY_DESCRIPTOR(old_sd);

    let status = unsafe {
        RegGetKeySecurity(
            handle.raw(),
            windows::Win32::Security::DACL_SECURITY_INFORMATION,
            Some(old_sd_desc),
            &mut descriptor_size,
        )
    };
    if status != ERROR_SUCCESS {
        unsafe {
            let _ = HeapFree(heap, HEAP_FLAGS(0), Some(old_sd as *const _));
        }
        return Err(RegistryException(format!(
            "Error while getting security information for registry key {}: {}",
            key,
            system_error(status)
        )));
    }

    let mut acl_present = windows::core::BOOL(0);
    let mut acl_defaulted = windows::core::BOOL(0);
    let mut old_acl: *mut ACL = null_mut();
    if unsafe {
        GetSecurityDescriptorDacl(
            old_sd_desc,
            &mut acl_present,
            &mut old_acl,
            &mut acl_defaulted,
        )
    }
    .is_err()
    {
        unsafe {
            let _ = HeapFree(heap, HEAP_FLAGS(0), Some(old_sd as *const _));
        }
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
        unsafe {
            let _ = HeapFree(heap, HEAP_FLAGS(0), Some(old_sd as *const _));
        }
        return Err(RegistryException(
            "Error in AllocateAndInitializeSid while ensuring writability".into(),
        ));
    }

    let trustee = TRUSTEE_W {
        TrusteeForm: TRUSTEE_IS_SID,
        TrusteeType: TRUSTEE_IS_GROUP,
        ptstrName: PWSTR(sid.0 as *mut u16),
        ..Default::default()
    };
    let explicit_access = EXPLICIT_ACCESS_W {
        grfAccessPermissions: KEY_ALL_ACCESS.0,
        grfAccessMode: SET_ACCESS,
        grfInheritance: windows::Win32::Security::SUB_CONTAINERS_AND_OBJECTS_INHERIT,
        Trustee: trustee,
    };

    let mut new_acl: *mut ACL = null_mut();
    let old_acl_arg = if old_acl.is_null() {
        None
    } else {
        Some(old_acl as *const ACL)
    };
    let acl_status = unsafe {
        SetEntriesInAclW(
            Some(std::slice::from_ref(&explicit_access)),
            old_acl_arg,
            &mut new_acl,
        )
    };
    if acl_status != ERROR_SUCCESS {
        unsafe {
            let _ = FreeSid(sid);
            let _ = HeapFree(heap, HEAP_FLAGS(0), Some(old_sd as *const _));
        }
        return Err(RegistryException(
            "Error in SetEntriesInAcl while ensuring writability".into(),
        ));
    }

    let sd = match unsafe { LocalAlloc(LPTR, SECURITY_DESCRIPTOR_MIN_LENGTH) } {
        Ok(sd) => sd,
        Err(_) => {
            unsafe {
                let _ = FreeSid(sid);
                let _ = LocalFree(Some(HLOCAL(new_acl as *mut _)));
                let _ = HeapFree(heap, HEAP_FLAGS(0), Some(old_sd as *const _));
            }
            return Err(RegistryException(
                "Error in LocalAlloc while ensuring writability".into(),
            ));
        }
    };
    let sd_desc = PSECURITY_DESCRIPTOR(sd.0);

    if unsafe { InitializeSecurityDescriptor(sd_desc, SECURITY_DESCRIPTOR_REVISION) }.is_err() {
        unsafe {
            let _ = FreeSid(sid);
            let _ = LocalFree(Some(HLOCAL(new_acl as *mut _)));
            let _ = HeapFree(heap, HEAP_FLAGS(0), Some(old_sd as *const _));
            let _ = LocalFree(Some(sd));
        }
        return Err(RegistryException(
            "Error in InitializeSecurityDescriptor while ensuring writability".into(),
        ));
    }
    if unsafe { SetSecurityDescriptorDacl(sd_desc, true, Some(new_acl as *const ACL), false) }
        .is_err()
    {
        unsafe {
            let _ = FreeSid(sid);
            let _ = LocalFree(Some(HLOCAL(new_acl as *mut _)));
            let _ = HeapFree(heap, HEAP_FLAGS(0), Some(old_sd as *const _));
            let _ = LocalFree(Some(sd));
        }
        return Err(RegistryException(
            "Error in SetSecurityDescriptorDacl while ensuring writability".into(),
        ));
    }

    let status = unsafe {
        RegSetKeySecurity(
            handle.raw(),
            windows::Win32::Security::DACL_SECURITY_INFORMATION,
            sd_desc,
        )
    };
    unsafe {
        let _ = FreeSid(sid);
        let _ = LocalFree(Some(HLOCAL(new_acl as *mut _)));
        let _ = HeapFree(heap, HEAP_FLAGS(0), Some(old_sd as *const _));
        let _ = LocalFree(Some(sd));
    }
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while setting security information for registry key {}: {}",
            key,
            system_error(status)
        )));
    }
    Ok(())
}

/// Port of `RegistryHelper::takeOwnership`.  The built-in Administrators
/// group becomes the owner of the key.
pub fn take_ownership(key: &str) -> Result<()> {
    let mut token = HANDLE::default();
    if unsafe {
        OpenProcessToken(
            GetCurrentProcess(),
            windows::Win32::Security::TOKEN_ADJUST_PRIVILEGES
                | windows::Win32::Security::TOKEN_QUERY,
            &mut token,
        )
    }
    .is_err()
    {
        return Err(RegistryException(
            "Error in OpenProcessToken while taking ownership".into(),
        ));
    }

    let mut luid = LUID::default();
    if unsafe {
        windows::Win32::Security::LookupPrivilegeValueW(
            PCWSTR::null(),
            windows::Win32::Security::SE_TAKE_OWNERSHIP_NAME,
            &mut luid,
        )
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

    let mut privileges = windows::Win32::Security::TOKEN_PRIVILEGES {
        PrivilegeCount: 1,
        Privileges: [windows::Win32::Security::LUID_AND_ATTRIBUTES {
            Luid: luid,
            Attributes: windows::Win32::Security::SE_PRIVILEGE_ENABLED,
        }],
    };

    if unsafe {
        windows::Win32::Security::AdjustTokenPrivileges(
            token,
            false,
            Some(&privileges),
            size_of::<windows::Win32::Security::TOKEN_PRIVILEGES>() as u32,
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

    let handle = match open_key(key, REG_SAM_FLAGS(WRITE_OWNER_BITS) | KEY_WOW64_64KEY) {
        Ok(handle) => KeyGuard::new(handle),
        Err(e) => {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(token);
            }
            return Err(e);
        }
    };

    let sd = match unsafe { LocalAlloc(LPTR, SECURITY_DESCRIPTOR_MIN_LENGTH) } {
        Ok(sd) => sd,
        Err(_) => {
            unsafe {
                let _ = windows::Win32::Foundation::CloseHandle(token);
            }
            return Err(RegistryException(
                "Error in SetPrivilege while taking ownership".into(),
            ));
        }
    };
    let sd_desc = PSECURITY_DESCRIPTOR(sd.0);

    if unsafe { InitializeSecurityDescriptor(sd_desc, SECURITY_DESCRIPTOR_REVISION) }.is_err() {
        unsafe {
            let _ = LocalFree(Some(sd));
            let _ = windows::Win32::Foundation::CloseHandle(token);
        }
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
        unsafe {
            let _ = LocalFree(Some(sd));
            let _ = windows::Win32::Foundation::CloseHandle(token);
        }
        return Err(RegistryException(
            "Error in AllocateAndInitializeSid while taking ownership".into(),
        ));
    }

    if unsafe { SetSecurityDescriptorOwner(sd_desc, Some(sid), false) }.is_err() {
        unsafe {
            let _ = FreeSid(sid);
            let _ = LocalFree(Some(sd));
            let _ = windows::Win32::Foundation::CloseHandle(token);
        }
        return Err(RegistryException(
            "Error in SetSecurityDescriptorOwner while taking ownership".into(),
        ));
    }

    let status = unsafe {
        RegSetKeySecurity(
            handle.raw(),
            windows::Win32::Security::OWNER_SECURITY_INFORMATION,
            sd_desc,
        )
    };

    // Disable the privilege again, as the original does.
    privileges.Privileges[0].Attributes = windows::Win32::Security::TOKEN_PRIVILEGES_ATTRIBUTES(0);
    let _ = unsafe {
        windows::Win32::Security::AdjustTokenPrivileges(
            token,
            false,
            Some(&privileges),
            size_of::<windows::Win32::Security::TOKEN_PRIVILEGES>() as u32,
            None,
            None,
        )
    };

    unsafe {
        let _ = FreeSid(sid);
        let _ = LocalFree(Some(sd));
        let _ = windows::Win32::Foundation::CloseHandle(token);
    }

    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while setting security information for registry key {}: {}",
            key,
            system_error(status)
        )));
    }
    Ok(())
}

/// Port of `RegistryHelper::getFileAccessForUser`.  Returns the `ACCESS_MASK`
/// granted to the account with the given relative identifier (RID).
pub fn get_file_access_for_user(path: &str, rid: u32) -> Result<u32> {
    let path_wide = to_wide(path);
    let mut sd = PSECURITY_DESCRIPTOR::default();
    let status = unsafe {
        GetNamedSecurityInfoW(
            PCWSTR(path_wide.as_ptr()),
            SE_FILE_OBJECT,
            windows::Win32::Security::DACL_SECURITY_INFORMATION
                | windows::Win32::Security::OWNER_SECURITY_INFORMATION
                | windows::Win32::Security::GROUP_SECURITY_INFORMATION,
            None,
            None,
            None,
            None,
            &mut sd,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(
            "Error in GetNamedSecurityInfoW while getting file access".into(),
        ));
    }

    let mut manager = AUTHZ_RESOURCE_MANAGER_HANDLE(null_mut());
    if unsafe {
        AuthzInitializeResourceManager(
            windows::Win32::Security::Authorization::AUTHZ_RM_FLAG_NO_AUDIT.0,
            None,
            None,
            None,
            PCWSTR::null(),
            &mut manager,
        )
    }
    .is_err()
    {
        unsafe {
            let _ = LocalFree(Some(HLOCAL(sd.0)));
        }
        return Err(RegistryException(
            "Error in AuthzInitializeResourceManager while getting file access".into(),
        ));
    }

    let mut sid = PSID::default();
    if unsafe {
        AllocateAndInitializeSid(
            &SECURITY_NT_AUTHORITY,
            1,
            rid,
            0,
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
        unsafe {
            let _ = AuthzFreeResourceManager(manager);
            let _ = LocalFree(Some(HLOCAL(sd.0)));
        }
        return Err(RegistryException(
            "Error in AllocateAndInitializeSid while getting file access".into(),
        ));
    }

    let mut context =
        windows::Win32::Security::Authorization::AUTHZ_CLIENT_CONTEXT_HANDLE(null_mut());
    if unsafe {
        AuthzInitializeContextFromSid(
            0,
            sid,
            manager,
            None,
            LUID {
                LowPart: 0,
                HighPart: 0,
            },
            None,
            &mut context,
        )
    }
    .is_err()
    {
        unsafe {
            let _ = FreeSid(sid);
            let _ = AuthzFreeResourceManager(manager);
            let _ = LocalFree(Some(HLOCAL(sd.0)));
        }
        return Err(RegistryException(
            "Error in AuthzInitializeContextFromSid while getting file access".into(),
        ));
    }

    let request = AUTHZ_ACCESS_REQUEST {
        DesiredAccess: MAXIMUM_ALLOWED,
        ..Default::default()
    };
    let mut granted_access: u32 = 0;
    let mut access_error: u32 = 0;
    let mut reply = AUTHZ_ACCESS_REPLY {
        ResultListLength: 1,
        GrantedAccessMask: &mut granted_access,
        SaclEvaluationResults: null_mut(),
        Error: &mut access_error,
    };

    let check_result = unsafe {
        AuthzAccessCheck(
            windows::Win32::Security::Authorization::AUTHZ_ACCESS_CHECK_FLAGS(0),
            context,
            &request,
            None,
            sd,
            None,
            &mut reply,
            None,
        )
    };

    unsafe {
        let _ = AuthzFreeContext(context);
        let _ = FreeSid(sid);
        let _ = AuthzFreeResourceManager(manager);
        let _ = LocalFree(Some(HLOCAL(sd.0)));
    }

    if check_result.is_err() {
        return Err(RegistryException(
            "Error in AuthzAccessCheck while getting file access".into(),
        ));
    }

    Ok(granted_access)
}

/// Port of `RegistryHelper::enumSubKeys`.
pub fn enum_sub_keys(key: &str) -> Result<Vec<String>> {
    let handle = KeyGuard::new(open_key(key, KEY_ENUMERATE_SUB_KEYS | KEY_WOW64_64KEY)?);
    let mut result = Vec::new();
    let mut key_name = [0u16; 256];
    let mut index = 0u32;

    loop {
        let mut length = key_name.len() as u32;
        let status = unsafe {
            RegEnumKeyExW(
                handle.raw(),
                index,
                Some(PWSTR(key_name.as_mut_ptr())),
                &mut length,
                None,
                None,
                None,
                None,
            )
        };
        index += 1;
        if status == ERROR_SUCCESS {
            result.push(String::from_utf16_lossy(&key_name[..length as usize]));
        } else if status == ERROR_NO_MORE_ITEMS {
            break;
        } else {
            return Err(RegistryException(format!(
                "Error while enumerating sub keys of registry key {}: {}",
                key,
                system_error(status)
            )));
        }
    }
    Ok(result)
}

/// Alias for [`enum_sub_keys`], matching the capitalization of the stub.
pub fn enum_subkeys(key: &str) -> Result<Vec<String>> {
    enum_sub_keys(key)
}

/// Port of `RegistryHelper::keyExists`.  Invalid or missing keys return false.
pub fn key_exists(key: &str) -> bool {
    let Ok((root, sub_key)) = split_key(key) else {
        return false;
    };
    let sub_wide = to_wide(&sub_key);
    let mut handle = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            root,
            PCWSTR(sub_wide.as_ptr()),
            None,
            KEY_QUERY_VALUE | KEY_WOW64_64KEY,
            &mut handle,
        )
    };
    if status == ERROR_SUCCESS {
        close_key(handle);
        true
    } else {
        false
    }
}

/// Port of `RegistryHelper::valueExists`.  Missing keys and values return
/// false (the C++ implementation throws for a missing key).
pub fn value_exists(key: &str, valuename: &str) -> bool {
    let Ok(handle) = open_key(key, KEY_QUERY_VALUE | KEY_WOW64_64KEY) else {
        return false;
    };
    let handle = KeyGuard::new(handle);
    let name = to_wide(valuename);
    let mut value_type = REG_VALUE_TYPE(0);
    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            handle.raw(),
            PCWSTR(name.as_ptr()),
            None,
            Some(&mut value_type),
            None,
            Some(&mut size),
        )
    };
    status == ERROR_SUCCESS
}

/// Port of `RegistryHelper::keyEmpty`.
pub fn key_empty(key: &str) -> Result<bool> {
    let handle = KeyGuard::new(open_key(key, KEY_QUERY_VALUE | KEY_WOW64_64KEY)?);
    let mut sub_key_count = 0u32;
    let mut value_count = 0u32;
    let status = unsafe {
        RegQueryInfoKeyW(
            handle.raw(),
            None,
            None,
            None,
            Some(&mut sub_key_count),
            None,
            None,
            Some(&mut value_count),
            None,
            None,
            None,
            None,
        )
    };
    if status != ERROR_SUCCESS {
        return Err(RegistryException(format!(
            "Error while reading info for registry key {}: {}",
            key,
            system_error(status)
        )));
    }
    Ok(sub_key_count == 0 && value_count == 0)
}

/// Port of `RegistryHelper::saveToFile`.  Writes the same `.reg` layout as the
/// original (including its hard-coded `HKEY_LOCAL_MACHINE` prefix) using the
/// system ANSI code page.
pub fn save_to_file(key: &str, valuenames: &[String], filepath: &str) -> Result<()> {
    let mut content = String::new();
    content.push_str("Windows Registry Editor Version 5.00\r\n\r\n");
    content.push_str(&format!("[HKEY_LOCAL_MACHINE\\{}]\r\n", key));
    for valuename in valuenames {
        let value = read_value(key, valuename)?;
        content.push_str(&format!("\"{}\"=\"{}\"\r\n", valuename, value));
    }
    content.push_str("\r\n");

    let mut file = std::fs::File::create(filepath).map_err(|_| {
        RegistryException(format!("Error while opening file {} for writing", filepath))
    })?;
    file.write_all(&string_to_ansi(&content)).map_err(|_| {
        RegistryException(format!("Error while opening file {} for writing", filepath))
    })?;
    Ok(())
}

/// Port of `RegistryHelper::getGuidString`.  Produces the same brace-delimited
/// upper case representation as `StringFromCLSID`.
pub fn get_guid_string(guid: windows::core::GUID) -> String {
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

fn query_windows_version() -> u32 {
    let mut handle = 0u32;
    let size =
        unsafe { GetFileVersionInfoSizeW(windows::core::w!("kernel32.dll"), Some(&mut handle)) };
    if size == 0 {
        return 0;
    }
    let mut data = vec![0u8; size as usize];
    if unsafe {
        GetFileVersionInfoW(
            windows::core::w!("kernel32.dll"),
            Some(handle),
            size,
            data.as_mut_ptr() as *mut _,
        )
    }
    .is_err()
    {
        return 0;
    }
    let mut info: *mut std::ffi::c_void = null_mut();
    let mut length = 0u32;
    if unsafe {
        VerQueryValueW(
            data.as_ptr() as *const _,
            windows::core::w!("\\"),
            &mut info,
            &mut length,
        )
    }
    .as_bool()
        && !info.is_null()
    {
        let fixed = unsafe { &*(info as *const VS_FIXEDFILEINFO) };
        return fixed.dwProductVersionMS;
    }
    0
}

static WINDOWS_VERSION: OnceLock<u32> = OnceLock::new();

/// Port of `RegistryHelper::isWindowsVersionAtLeast`.  The kernel32 product
/// version is queried once and cached like the C++ static member.
pub fn is_windows_version_at_least(major: u32, minor: u32) -> bool {
    let windows_version = *WINDOWS_VERSION.get_or_init(query_windows_version);
    // Same encoding as the C++ implementation; only valid for 0..=99.
    let compare_version =
        ((major / 10) << 20) + ((major % 10) << 16) + ((minor / 10) << 4) + (minor % 10);
    windows_version >= compare_version
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_KEY: &str = r"HKEY_CURRENT_USER\Software\EqualizerAPO\RustPortTest";

    fn cleanup() {
        if let Ok(sub_keys) = enum_sub_keys(TEST_KEY) {
            for sub_key in sub_keys {
                let _ = delete_key(&format!("{}\\{}", TEST_KEY, sub_key));
            }
        }
        let _ = delete_key(TEST_KEY);
    }

    #[test]
    fn guid_string_uses_clsid_format() {
        assert_eq!(
            get_guid_string(windows::core::GUID::from_u128(
                0xeacd2258_fcac_4ff4_b36d_419e924a6d79
            )),
            "{EACD2258-FCAC-4FF4-B36D-419E924A6D79}"
        );
    }

    #[test]
    fn registry_round_trip_hkcu() {
        cleanup();
        create_key(TEST_KEY).expect("create test key");
        assert!(key_exists(TEST_KEY));
        assert!(key_empty(TEST_KEY).expect("key empty"));
        // Granting Administrators KEY_ALL_ACCESS must work on a key owned by
        // the current user, without requiring elevation.
        make_writable(TEST_KEY).expect("make writable");

        write_value(TEST_KEY, "Str", "Hello \u{00e4}").expect("write string");
        write_dword_value(TEST_KEY, "Dword", 0xDEAD_BEEF).expect("write dword");
        write_multi_value(TEST_KEY, "Multi", &["one".to_string(), "two".to_string()])
            .expect("write multi");
        write_multi_value_single(TEST_KEY, "MultiSingle", "only").expect("write multi single");

        assert_eq!(
            read_value(TEST_KEY, "Str").expect("read string"),
            "Hello \u{00e4}"
        );
        assert_eq!(
            read_dword_value(TEST_KEY, "Dword").expect("read dword"),
            0xDEAD_BEEF
        );
        assert_eq!(
            read_multi_value(TEST_KEY, "Multi").expect("read multi"),
            vec!["one".to_string(), "two".to_string()]
        );
        assert_eq!(
            read_multi_value(TEST_KEY, "MultiSingle").expect("read multi single"),
            vec!["only".to_string()]
        );

        assert!(value_exists(TEST_KEY, "Str"));
        assert!(!value_exists(TEST_KEY, "DoesNotExist"));
        assert!(!key_empty(TEST_KEY).expect("key not empty"));

        let export_path = std::env::temp_dir().join("eapo_common_rust_port_test.reg");
        save_to_file(
            TEST_KEY,
            &["Str".to_string()],
            &export_path.to_string_lossy(),
        )
        .expect("save to file");
        let exported = std::fs::read(&export_path).expect("read exported file");
        assert!(exported.starts_with(b"Windows Registry Editor Version 5.00\r\n"));
        assert!(String::from_utf8_lossy(&exported).contains("[HKEY_LOCAL_MACHINE\\"));
        let _ = std::fs::remove_file(&export_path);

        let sub_key = format!("{}\\Sub", TEST_KEY);
        create_key(&sub_key).expect("create sub key");
        assert_eq!(
            enum_sub_keys(TEST_KEY).expect("enum sub keys"),
            vec!["Sub".to_string()]
        );
        delete_key(&sub_key).expect("delete sub key");

        delete_value(TEST_KEY, "Str").expect("delete value");
        assert!(!value_exists(TEST_KEY, "Str"));
        assert!(read_value(TEST_KEY, "Str").is_err());

        delete_key(TEST_KEY).expect("delete test key");
        assert!(!key_exists(TEST_KEY));
    }

    #[test]
    fn version_query_works() {
        // Every supported Windows version is at least 6.1 (Windows 7).
        assert!(is_windows_version_at_least(6, 1));
        assert!(!is_windows_version_at_least(99, 99));
    }

    #[test]
    fn file_access_for_local_service_can_be_queried() {
        let exe = std::env::current_exe().expect("current exe");
        let access =
            get_file_access_for_user(&exe.to_string_lossy(), 19).expect("query file access");
        // The test binary is readable, so at least one access bit is granted.
        assert_ne!(access, 0);
    }

    #[test]
    fn invalid_keys_report_errors() {
        assert!(read_value("NoRoot\\Foo", "Bar").is_err());
        assert!(read_value(
            "HKEY_CURRENT_USER\\Software\\EqualizerAPO\\NoSuchKey",
            "Bar"
        )
        .is_err());
        assert!(!key_exists("NoRoot\\Foo"));
    }
}
