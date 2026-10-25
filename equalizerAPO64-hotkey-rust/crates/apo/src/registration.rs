//! Registry side of `DllMain.cpp`: `DllRegisterServer` /
//! `DllUnregisterServer`, plus a pure-Rust replacement for the WDK
//! `RegisterAPO`/`UnregisterAPO` helpers.
//!
//! `RegisterAPO` lives in the static WDK library
//! `audiobaseprocessingobject.lib`; linking it would pull C++ code into this
//! crate. The persistent APO registration it produces is documented (and
//! written by INF files) as `HKLM\SOFTWARE\Classes\AudioEngine\
//! AudioProcessingObjects\{clsid}` with the values below, so this module
//! writes exactly that data. Deviation: the original also notifies the audio
//! engine through `RegisterAPO`'s internal RPC; no notification is sent here.

use std::sync::atomic::{AtomicI32, Ordering};

use eapo_apo_info::guid_to_string;
use eapo_common::{EQUALIZERAPO_POST_MIX_GUID, EQUALIZERAPO_PRE_MIX_GUID};
use windows::core::{GUID, HRESULT, HSTRING, PCWSTR};
use windows::Win32::Foundation::WIN32_ERROR;
use windows::Win32::System::Registry::{
    RegCloseKey, RegCreateKeyExW, RegDeleteKeyExW, RegOpenKeyExW, RegQueryValueExW, RegSetValueExW,
    HKEY, HKEY_LOCAL_MACHINE, KEY_QUERY_VALUE, KEY_SET_VALUE, KEY_WOW64_64KEY, REG_DWORD,
    REG_OPEN_CREATE_OPTIONS, REG_SAM_FLAGS, REG_SZ, REG_VALUE_TYPE,
};

use crate::apo::{post_mix_properties, pre_mix_properties};

const S_OK: HRESULT = HRESULT(0);
const E_FAIL: HRESULT = HRESULT(0x80004005u32 as i32);
const ERROR_SUCCESS: WIN32_ERROR = WIN32_ERROR(0);

fn split_key(key: &str) -> Option<(HKEY, String)> {
    let pos = key.find('\\')?;
    let root = match key[..pos].to_uppercase().as_str() {
        "HKEY_CLASSES_ROOT" => windows::Win32::System::Registry::HKEY_CLASSES_ROOT,
        "HKEY_CURRENT_CONFIG" => windows::Win32::System::Registry::HKEY_CURRENT_CONFIG,
        "HKEY_CURRENT_USER" => windows::Win32::System::Registry::HKEY_CURRENT_USER,
        "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
        "HKEY_USERS" => windows::Win32::System::Registry::HKEY_USERS,
        _ => return None,
    };
    Some((root, key[pos + 1..].to_string()))
}

fn open_key(key: &str, access: REG_SAM_FLAGS) -> Option<HKEY> {
    let (root, sub_key) = split_key(key)?;
    let sub = HSTRING::from(sub_key);
    let mut handle = HKEY::default();
    let status = unsafe { RegOpenKeyExW(root, &sub, None, access, &mut handle) };
    if status == ERROR_SUCCESS {
        Some(handle)
    } else {
        None
    }
}

pub(crate) fn value_exists(key: &str, valuename: &str) -> bool {
    let Some(handle) = open_key(key, KEY_QUERY_VALUE | KEY_WOW64_64KEY) else {
        return false;
    };
    let name = HSTRING::from(valuename);
    let mut value_type = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            handle,
            &name,
            None,
            Some(&mut value_type),
            None,
            Some(&mut size),
        )
    };
    unsafe {
        let _ = RegCloseKey(handle);
    }
    status == ERROR_SUCCESS
}

pub(crate) fn read_value(key: &str, valuename: &str) -> Result<String, String> {
    let handle = open_key(key, KEY_QUERY_VALUE | KEY_WOW64_64KEY)
        .ok_or_else(|| format!("Error while opening registry key {key}"))?;
    let name = HSTRING::from(valuename);
    let mut value_type = REG_VALUE_TYPE::default();
    let mut size = 0u32;
    let status = unsafe {
        RegQueryValueExW(
            handle,
            &name,
            None,
            Some(&mut value_type),
            None,
            Some(&mut size),
        )
    };
    if status != ERROR_SUCCESS || value_type != REG_SZ {
        unsafe {
            let _ = RegCloseKey(handle);
        }
        return Err(format!(
            "Error while reading registry value {key}\\{valuename}"
        ));
    }
    let mut data = vec![0u8; size as usize];
    let status = unsafe {
        RegQueryValueExW(
            handle,
            &name,
            None,
            None,
            Some(data.as_mut_ptr()),
            Some(&mut size),
        )
    };
    unsafe {
        let _ = RegCloseKey(handle);
    }
    if status != ERROR_SUCCESS {
        return Err(format!(
            "Error while reading registry value {key}\\{valuename}"
        ));
    }
    let mut chars: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    if chars.last() == Some(&0) {
        chars.pop();
    }
    Ok(String::from_utf16_lossy(&chars))
}

fn create_key(key: &str) -> Result<(), ()> {
    let (root, sub_key) = split_key(key).ok_or(())?;
    let sub = HSTRING::from(sub_key);
    let mut handle = HKEY::default();
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
        return Err(());
    }
    unsafe {
        let _ = RegCloseKey(handle);
    }
    Ok(())
}

fn write_value(key: &str, valuename: &str, value: &str) -> Result<(), ()> {
    let handle = open_key(key, KEY_SET_VALUE | KEY_WOW64_64KEY).ok_or(())?;
    let name = HSTRING::from(valuename);
    let mut data: Vec<u8> = value.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    data.extend_from_slice(&[0, 0]);
    let status = unsafe { RegSetValueExW(handle, &name, None, REG_SZ, Some(&data)) };
    unsafe {
        let _ = RegCloseKey(handle);
    }
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(())
    }
}

fn write_dword_value(key: &str, valuename: &str, value: u32) -> Result<(), ()> {
    let handle = open_key(key, KEY_SET_VALUE | KEY_WOW64_64KEY).ok_or(())?;
    let name = HSTRING::from(valuename);
    let data = value.to_le_bytes();
    let status = unsafe { RegSetValueExW(handle, &name, None, REG_DWORD, Some(&data)) };
    unsafe {
        let _ = RegCloseKey(handle);
    }
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(())
    }
}

fn delete_key(key: &str) -> Result<(), ()> {
    let (root, sub_key) = split_key(key).ok_or(())?;
    let sub = HSTRING::from(sub_key);
    let status = unsafe { RegDeleteKeyExW(root, &sub, KEY_WOW64_64KEY.0, None) };
    if status == ERROR_SUCCESS {
        Ok(())
    } else {
        Err(())
    }
}

fn wide_array_to_string(array: &[u16]) -> String {
    let len = array.iter().position(|&c| c == 0).unwrap_or(array.len());
    String::from_utf16_lossy(&array[..len])
}

fn register_apo(properties: &windows::Win32::Media::Audio::Apo::APO_REG_PROPERTIES) -> HRESULT {
    let clsid = guid_to_string(properties.clsid);
    let key =
        format!(r"HKEY_LOCAL_MACHINE\SOFTWARE\Classes\AudioEngine\AudioProcessingObjects\{clsid}");
    // Intermediate keys exist on a stock Windows installation; create them
    // defensively so a fresh profile can register as well.
    let _ = create_key(r"HKEY_LOCAL_MACHINE\SOFTWARE\Classes\AudioEngine");
    let _ = create_key(r"HKEY_LOCAL_MACHINE\SOFTWARE\Classes\AudioEngine\AudioProcessingObjects");
    if create_key(&key).is_err() {
        return E_FAIL;
    }

    let interface_guid = properties
        .iidAPOInterfaceList
        .first()
        .map(|guid| guid_to_string(*guid))
        .unwrap_or_default();

    let result = (|| -> Result<(), ()> {
        write_value(
            &key,
            "FriendlyName",
            &wide_array_to_string(&properties.szFriendlyName),
        )?;
        write_value(
            &key,
            "Copyright",
            &wide_array_to_string(&properties.szCopyrightInfo),
        )?;
        write_dword_value(&key, "MajorVersion", properties.u32MajorVersion)?;
        write_dword_value(&key, "MinorVersion", properties.u32MinorVersion)?;
        write_dword_value(&key, "Flags", properties.Flags.0 as u32)?;
        write_dword_value(
            &key,
            "MinInputConnections",
            properties.u32MinInputConnections,
        )?;
        write_dword_value(
            &key,
            "MaxInputConnections",
            properties.u32MaxInputConnections,
        )?;
        write_dword_value(
            &key,
            "MinOutputConnections",
            properties.u32MinOutputConnections,
        )?;
        write_dword_value(
            &key,
            "MaxOutputConnections",
            properties.u32MaxOutputConnections,
        )?;
        write_dword_value(&key, "MaxInstances", properties.u32MaxInstances)?;
        write_dword_value(&key, "NumAPOInterfaces", properties.u32NumAPOInterfaces)?;
        write_value(&key, "APOInterface0", &interface_guid)?;
        Ok(())
    })();

    if result.is_err() {
        let _ = delete_key(&key);
        E_FAIL
    } else {
        S_OK
    }
}

fn unregister_apo(clsid: GUID) -> HRESULT {
    let clsid = guid_to_string(clsid);
    let key =
        format!(r"HKEY_LOCAL_MACHINE\SOFTWARE\Classes\AudioEngine\AudioProcessingObjects\{clsid}");
    if delete_key(&key).is_ok() {
        S_OK
    } else {
        E_FAIL
    }
}

fn register_clsid(clsid: GUID, description: &str, filename: &str) -> Result<(), ()> {
    let clsid = guid_to_string(clsid);
    let base = format!(r"HKEY_LOCAL_MACHINE\SOFTWARE\Classes\CLSID\{clsid}");
    create_key(&base)?;
    write_value(&base, "", description)?;
    let inproc = format!("{base}\\InprocServer32");
    create_key(&inproc)?;
    write_value(&inproc, "", filename)?;
    write_value(&inproc, "ThreadingModel", "Both")?;
    Ok(())
}

fn unregister_clsid(clsid: GUID) -> Result<(), ()> {
    let clsid = guid_to_string(clsid);
    let base = format!(r"HKEY_LOCAL_MACHINE\SOFTWARE\Classes\CLSID\{clsid}");
    delete_key(&format!("{base}\\InprocServer32"))?;
    delete_key(&base)?;
    Ok(())
}

fn module_file_name() -> String {
    use windows::Win32::System::LibraryLoader::GetModuleFileNameW;
    let handle = crate::module_handle();
    let mut buffer = [0u16; 1024];
    let len = unsafe { GetModuleFileNameW(handle, &mut buffer) } as usize;
    if len == 0 {
        return String::new();
    }
    String::from_utf16_lossy(&buffer[..len])
}

pub(crate) unsafe fn dll_register_server() -> HRESULT {
    let filename = module_file_name();

    let mut hr = register_apo(post_mix_properties());
    if hr.is_err() {
        let _ = unregister_apo(EQUALIZERAPO_POST_MIX_GUID);
        return hr;
    }

    hr = register_apo(pre_mix_properties());
    if hr.is_err() {
        let _ = unregister_apo(EQUALIZERAPO_POST_MIX_GUID);
        let _ = unregister_apo(EQUALIZERAPO_PRE_MIX_GUID);
        return hr;
    }

    let result = (|| -> Result<(), ()> {
        register_clsid(
            EQUALIZERAPO_POST_MIX_GUID,
            "EqualizerAPO Post-Mix Class",
            &filename,
        )?;
        register_clsid(
            EQUALIZERAPO_PRE_MIX_GUID,
            "EqualizerAPO Pre-Mix Class",
            &filename,
        )?;
        Ok(())
    })();

    if result.is_err() {
        let _ = unregister_apo(EQUALIZERAPO_POST_MIX_GUID);
        let _ = unregister_apo(EQUALIZERAPO_PRE_MIX_GUID);
        return E_FAIL;
    }

    S_OK
}

pub(crate) unsafe fn dll_unregister_server() -> HRESULT {
    if unregister_clsid(EQUALIZERAPO_POST_MIX_GUID).is_err() {
        return E_FAIL;
    }
    if unregister_clsid(EQUALIZERAPO_PRE_MIX_GUID).is_err() {
        return E_FAIL;
    }

    let hr = unregister_apo(EQUALIZERAPO_POST_MIX_GUID);
    let _ = unregister_apo(EQUALIZERAPO_PRE_MIX_GUID);
    hr
}

/// C++ `ClassFactory::lockCount` equivalent.
pub(crate) static CLASS_FACTORY_LOCK_COUNT: AtomicI32 = AtomicI32::new(0);

/// Small helper so `DllMain` state remains reachable from registration.
pub(crate) fn add_factory_lock() {
    CLASS_FACTORY_LOCK_COUNT.fetch_add(1, Ordering::SeqCst);
}

pub(crate) fn release_factory_lock() {
    CLASS_FACTORY_LOCK_COUNT.fetch_sub(1, Ordering::SeqCst);
}
