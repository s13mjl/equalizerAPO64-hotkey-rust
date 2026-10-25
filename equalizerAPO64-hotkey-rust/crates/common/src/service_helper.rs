//! Port of `helpers/ServiceHelper.{h,cpp}`.
//!
//! Implements the Windows Service Control Manager lifecycle used by the
//! DeviceSelector (restarting `AudioSrv` and its active dependents).

use std::fmt;
use std::mem::size_of;

use windows::core::PCWSTR;
use windows::Win32::Foundation::{GetLastError, ERROR_MORE_DATA};
use windows::Win32::System::Services::{
    CloseServiceHandle, ControlService, EnumDependentServicesW, OpenSCManagerW, OpenServiceW,
    QueryServiceStatusEx, StartServiceW, ENUM_SERVICE_STATUSW, SC_HANDLE, SC_MANAGER_ALL_ACCESS,
    SC_STATUS_PROCESS_INFO, SERVICE_ACTIVE, SERVICE_CONTROL_STOP, SERVICE_ENUMERATE_DEPENDENTS,
    SERVICE_QUERY_STATUS, SERVICE_RUNNING, SERVICE_START, SERVICE_STATUS, SERVICE_STATUS_PROCESS,
    SERVICE_STOP, SERVICE_STOPPED,
};
use windows::Win32::System::Threading::Sleep;

use crate::precision_timer::PrecisionTimer;
use crate::scope_guard::ScopeGuard;
use crate::string_helper::get_system_error_string;

/// Error type mirroring `ServiceException`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ServiceException(pub String);

impl ServiceException {
    /// Equivalent of the C++ `getMessage()`.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ServiceException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for ServiceException {}

fn to_wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn last_error() -> u32 {
    unsafe { GetLastError().0 }
}

unsafe fn struct_as_bytes_mut<T>(value: &mut T) -> &mut [u8] {
    unsafe { std::slice::from_raw_parts_mut(value as *mut T as *mut u8, size_of::<T>()) }
}

fn fail(function_name: &str, service_name: &str, error: u32) -> ServiceException {
    ServiceException(format!(
        "{} failed for service \"{}\" ({})",
        function_name,
        service_name,
        get_system_error_string(error as i32)
    ))
}

/// Port of the `Service` class.  Owns an `SC_HANDLE` and closes it on drop.
pub struct Service {
    service_handle: SC_HANDLE,
    service_name: String,
}

impl Service {
    /// Port of `Service::Service`.
    ///
    /// The C++ constructor computes `desiredAccess` from `allowEnumerate` but
    /// then always passes `SERVICE_ENUMERATE_DEPENDENTS`; this port keeps that
    /// behaviour for compatibility.
    pub fn new(
        sc_manager: SC_HANDLE,
        service_name: &str,
        _allow_enumerate: bool,
    ) -> Result<Self, ServiceException> {
        let name = to_wide(service_name);
        let desired_access =
            SERVICE_START | SERVICE_STOP | SERVICE_QUERY_STATUS | SERVICE_ENUMERATE_DEPENDENTS;

        match unsafe { OpenServiceW(sc_manager, PCWSTR(name.as_ptr()), desired_access) } {
            Ok(service_handle) => Ok(Self {
                service_handle,
                service_name: service_name.to_string(),
            }),
            Err(_) => Err(fail("OpenService", service_name, last_error())),
        }
    }

    /// Port of `Service::getServiceName`.
    pub fn name(&self) -> &str {
        &self.service_name
    }

    /// Alias for [`Service::name`], named after the C++ method.
    pub fn get_service_name(&self) -> &str {
        self.name()
    }

    /// Alias for [`Service::state`], named after the C++ method.
    pub fn get_state(&self) -> Result<u32, ServiceException> {
        self.state()
    }

    /// Port of `Service::getState`.  Returns a `SERVICE_*` state value.
    pub fn state(&self) -> Result<u32, ServiceException> {
        let mut status = SERVICE_STATUS_PROCESS::default();
        let mut bytes_needed = 0u32;
        let result = unsafe {
            QueryServiceStatusEx(
                self.service_handle,
                SC_STATUS_PROCESS_INFO,
                Some(struct_as_bytes_mut(&mut status)),
                &mut bytes_needed,
            )
        };
        result.map_err(|_| fail("QueryServiceStatusEx", &self.service_name, last_error()))?;
        Ok(status.dwCurrentState.0)
    }

    /// Port of `Service::start`.
    pub fn start(&self) -> Result<(), ServiceException> {
        unsafe { StartServiceW(self.service_handle, None) }
            .map_err(|_| fail("StartService", &self.service_name, last_error()))
    }

    /// Port of `Service::stop`.  Returns the state reported by
    /// `ControlService` (usually `SERVICE_STOP_PENDING`).
    pub fn stop(&self) -> Result<u32, ServiceException> {
        let mut status = SERVICE_STATUS::default();
        unsafe { ControlService(self.service_handle, SERVICE_CONTROL_STOP, &mut status) }
            .map_err(|_| fail("ControlService", &self.service_name, last_error()))?;
        Ok(status.dwCurrentState.0)
    }

    /// Alias for [`Service::active_dependent_services`], named after the C++
    /// method.
    pub fn get_active_dependent_services(&self) -> Result<Vec<String>, ServiceException> {
        self.active_dependent_services()
    }

    /// Port of `Service::getActiveDependentServices`.
    pub fn active_dependent_services(&self) -> Result<Vec<String>, ServiceException> {
        let mut bytes_needed = 0u32;
        let mut count = 0u32;

        let first = unsafe {
            EnumDependentServicesW(
                self.service_handle,
                SERVICE_ACTIVE,
                None,
                0,
                &mut bytes_needed,
                &mut count,
            )
        };
        if first.is_ok() {
            // If the call succeeds, there are no dependent services.
            return Ok(Vec::new());
        }

        let error = last_error();
        if error != ERROR_MORE_DATA.0 {
            return Err(fail("EnumDependentServices", &self.service_name, error));
        }

        // `Vec<usize>` guarantees the alignment HeapAlloc would provide.
        let word_count = (bytes_needed as usize).div_ceil(size_of::<usize>()).max(1);
        let mut buffer = vec![0usize; word_count];
        let entries = buffer.as_mut_ptr() as *mut ENUM_SERVICE_STATUSW;

        let result = unsafe {
            EnumDependentServicesW(
                self.service_handle,
                SERVICE_ACTIVE,
                Some(entries),
                bytes_needed,
                &mut bytes_needed,
                &mut count,
            )
        };
        result.map_err(|_| fail("EnumDependentServices", &self.service_name, last_error()))?;

        let entries = unsafe { std::slice::from_raw_parts(entries, count as usize) };
        Ok(entries
            .iter()
            .map(|entry| unsafe { entry.lpServiceName.to_string() }.unwrap_or_default())
            .collect())
    }
}

impl Drop for Service {
    fn drop(&mut self) {
        if !self.service_handle.is_invalid() {
            unsafe {
                let _ = CloseServiceHandle(self.service_handle);
            }
        }
    }
}

/// Port of `ServiceHelper::restartService`.
///
/// Stops the service and all active dependent services (dependents first) and
/// starts them again in reverse order.  A stop/start taking more than 30
/// seconds raises [`ServiceException`], like the C++ code.  Requires
/// administrator rights (`SC_MANAGER_ALL_ACCESS`).
pub fn restart_service(service_name: &str) -> Result<(), ServiceException> {
    let sc_manager =
        match unsafe { OpenSCManagerW(PCWSTR::null(), PCWSTR::null(), SC_MANAGER_ALL_ACCESS) } {
            Ok(handle) => handle,
            Err(_) => {
                return Err(ServiceException(format!(
                    "OpenSCManager failed ({})",
                    get_system_error_string(last_error() as i32)
                )))
            }
        };
    let _manager_guard = ScopeGuard::new(|| unsafe {
        let _ = CloseServiceHandle(sc_manager);
    });

    let main_service = Service::new(sc_manager, service_name, true)?;
    let main_state = main_service.state()?;

    let mut services = vec![main_service];
    if main_state == SERVICE_RUNNING.0 {
        let dependent_names = services[0].active_dependent_services()?;
        for dependent_name in dependent_names {
            let dependent = Service::new(sc_manager, &dependent_name, false)?;
            // C++ inserts before the main service (the last element), so the
            // list is [dependent..., main].
            services.insert(services.len() - 1, dependent);
        }
    }

    let mut timer = PrecisionTimer::new();
    timer.start();

    for service in &services {
        let mut state = service.state()?;
        if state == SERVICE_RUNNING.0 {
            state = service.stop()?;
        }

        while state != SERVICE_STOPPED.0 {
            if timer.stop() > 30.0 {
                return Err(ServiceException(format!(
                    "Service stop timed out on service \"{}\"",
                    service.name()
                )));
            }
            unsafe { Sleep(100) };
            state = service.state()?;
        }
    }

    // All services should be stopped now, so start them again in reverse.
    for service in services.iter().rev() {
        service.start()?;
        let mut state = service.state()?;
        let mut retry_delay = 5.0f64;
        while state != SERVICE_RUNNING.0 {
            let time = timer.stop();
            if time > 30.0 {
                return Err(ServiceException(format!(
                    "Service start timed out on service \"{}\"",
                    service.name()
                )));
            }
            if time > retry_delay {
                // Sometimes the service won't start on the first try.
                service.start()?;
                retry_delay = time + 5.0;
            }
            unsafe { Sleep(100) };
            state = service.state()?;
        }
    }

    Ok(())
}
