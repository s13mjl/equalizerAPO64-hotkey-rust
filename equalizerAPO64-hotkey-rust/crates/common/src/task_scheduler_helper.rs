//! Port of `helpers/TaskSchedulerHelper.{h,cpp}`.
//!
//! Uses the Task Scheduler COM API (`ITaskService`) exactly like the original:
//! a logon trigger (`TASK_TRIGGER_LOGON`) with a daily repetition, an execute
//! action, `TASK_CREATE_OR_UPDATE` and `TASK_LOGON_INTERACTIVE_TOKEN`.
//!
//! Unlike the C++ code (which assumes COM has already been initialized) this
//! port calls `CoInitializeEx` for the duration of each operation and tolerates
//! `RPC_E_TOO_LATE` from `CoInitializeSecurity`, so `schedule_at_logon` and
//! `unschedule` can be called in either order.

use std::fmt;
use std::sync::OnceLock;

use windows::core::{Interface, BSTR, PWSTR};
use windows::Win32::Foundation::{
    GetLastError, RPC_E_CHANGED_MODE, RPC_E_TOO_LATE, VARIANT_FALSE, VARIANT_TRUE,
};
use windows::Win32::Security::Authentication::Identity::{GetUserNameExW, NameSamCompatible};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoInitializeSecurity, CoUninitialize, CLSCTX_INPROC_SERVER,
    COINIT_MULTITHREADED, EOLE_AUTHENTICATION_CAPABILITIES, RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
    RPC_C_IMP_LEVEL_IMPERSONATE,
};
use windows::Win32::System::TaskScheduler::{
    IActionCollection, IExecAction, ILogonTrigger, ITaskDefinition, ITaskFolder, ITaskService,
    ITaskSettings, ITriggerCollection, TaskScheduler, TASK_ACTION_EXEC, TASK_CREATE_OR_UPDATE,
    TASK_LOGON_INTERACTIVE_TOKEN, TASK_TRIGGER_LOGON,
};
use windows::Win32::System::Variant::VARIANT;

use crate::string_helper::get_system_error_string;

/// Error type mirroring `TaskSchedulerException`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskSchedulerException(pub String);

impl TaskSchedulerException {
    /// Equivalent of the C++ `getMessage()`.
    pub fn message(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for TaskSchedulerException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl std::error::Error for TaskSchedulerException {}

fn fail(function_name: &str, error: i32) -> TaskSchedulerException {
    TaskSchedulerException(format!(
        "{} failed in task scheduling ({})",
        function_name,
        get_system_error_string(error)
    ))
}

fn last_error() -> i32 {
    unsafe { GetLastError().0 as i32 }
}

/// Balances a successful `CoInitializeEx` call.
struct ComGuard {
    uninitialize: bool,
}

impl Drop for ComGuard {
    fn drop(&mut self) {
        if self.uninitialize {
            unsafe { CoUninitialize() };
        }
    }
}

fn com_initialize() -> ComGuard {
    let hr = unsafe { CoInitializeEx(None, COINIT_MULTITHREADED) };
    if hr == RPC_E_CHANGED_MODE {
        // Another thread initialized the apartment differently; that is fine,
        // but this call must not be balanced with CoUninitialize.
        ComGuard {
            uninitialize: false,
        }
    } else {
        ComGuard {
            uninitialize: hr.0 >= 0,
        }
    }
}

/// Process-wide `CoInitializeSecurity` (the API can only be called once).
fn ensure_com_security() -> Result<(), TaskSchedulerException> {
    static SECURITY_DONE: OnceLock<()> = OnceLock::new();
    if SECURITY_DONE.get().is_some() {
        return Ok(());
    }

    let result = unsafe {
        CoInitializeSecurity(
            None,
            -1,
            None,
            None,
            RPC_C_AUTHN_LEVEL_PKT_PRIVACY,
            RPC_C_IMP_LEVEL_IMPERSONATE,
            None,
            EOLE_AUTHENTICATION_CAPABILITIES(0),
            None,
        )
    };

    match result {
        Ok(()) => {
            let _ = SECURITY_DONE.set(());
            Ok(())
        }
        Err(error) if error.code() == RPC_E_TOO_LATE => {
            // Someone already called CoInitializeSecurity in this process.
            let _ = SECURITY_DONE.set(());
            Ok(())
        }
        Err(error) => Err(fail("CoInitializeSecurity", error.code().0)),
    }
}

unsafe fn create_task_service() -> Result<ITaskService, TaskSchedulerException> {
    let service: ITaskService = unsafe {
        CoCreateInstance(
            &TaskScheduler,
            None::<&windows::core::IUnknown>,
            CLSCTX_INPROC_SERVER,
        )
    }
    .map_err(|error| fail("CoCreateInstance", error.code().0))?;

    let empty = VARIANT::default();
    unsafe { service.Connect(&empty, &empty, &empty, &empty) }
        .map_err(|error| fail("ITaskService::Connect", error.code().0))?;

    Ok(service)
}

fn get_user_name() -> Result<String, TaskSchedulerException> {
    let mut buffer = [0u16; 256];
    let mut size = buffer.len() as u32;
    let ok = unsafe {
        GetUserNameExW(
            NameSamCompatible,
            Some(PWSTR(buffer.as_mut_ptr())),
            &mut size,
        )
    };
    if !ok {
        return Err(fail("GetUserNameExW", last_error()));
    }
    let length = (size as usize).min(buffer.len());
    Ok(String::from_utf16_lossy(&buffer[..length]))
}

/// Port of `TaskSchedulerHelper::scheduleAtLogon`.  Requires the caller to be
/// able to register a task for the current user.
pub fn schedule_at_logon(
    task_name: &str,
    program_path: &str,
    program_args: &str,
    working_dir: &str,
) -> Result<(), TaskSchedulerException> {
    let _com = com_initialize();
    ensure_com_security()?;

    let service = unsafe { create_task_service()? };
    let root_folder: ITaskFolder = unsafe { service.GetFolder(&BSTR::from("\\")) }
        .map_err(|error| fail("ITaskService::GetFolder", error.code().0))?;

    // Possibly delete an existing task; errors are ignored like in C++.
    let _ = unsafe { root_folder.DeleteTask(&BSTR::from(task_name), 0) };

    let task: ITaskDefinition = unsafe { service.NewTask(0) }
        .map_err(|error| fail("ITaskService::NewTask", error.code().0))?;

    let user_name = get_user_name()?;

    let registration_info = unsafe { task.RegistrationInfo() }
        .map_err(|error| fail("ITaskDefinition::get_RegistrationInfo", error.code().0))?;
    unsafe { registration_info.SetAuthor(&BSTR::from(user_name.as_str())) }
        .map_err(|error| fail("IRegistrationInfo::put_Author", error.code().0))?;

    {
        let settings: ITaskSettings = unsafe { task.Settings() }
            .map_err(|error| fail("ITaskDefinition::get_Settings", error.code().0))?;

        unsafe { settings.SetStartWhenAvailable(VARIANT_TRUE) }
            .map_err(|error| fail("ITaskSettings::put_StartWhenAvailable", error.code().0))?;
        unsafe { settings.SetDisallowStartIfOnBatteries(VARIANT_FALSE) }.map_err(|error| {
            fail(
                "ITaskSettings::put_DisallowStartIfOnBatteries",
                error.code().0,
            )
        })?;
        unsafe { settings.SetStopIfGoingOnBatteries(VARIANT_FALSE) }
            .map_err(|error| fail("ITaskSettings::put_StopIfGoingOnBatteries", error.code().0))?;
        unsafe { settings.SetRunOnlyIfNetworkAvailable(VARIANT_TRUE) }.map_err(|error| {
            fail(
                "ITaskSettings::put_RunOnlyIfNetworkAvailable",
                error.code().0,
            )
        })?;
    }

    {
        let trigger_collection: ITriggerCollection = unsafe { task.Triggers() }
            .map_err(|error| fail("ITaskDefinition::get_Triggers", error.code().0))?;
        let trigger = unsafe { trigger_collection.Create(TASK_TRIGGER_LOGON) }
            .map_err(|error| fail("ITriggerCollection::Create", error.code().0))?;
        let logon_trigger: ILogonTrigger = trigger
            .cast()
            .map_err(|error| fail("QueryInterface(ILogonTrigger)", error.code().0))?;

        unsafe { logon_trigger.SetId(&BSTR::from("Trigger1")) }
            .map_err(|error| fail("ITrigger::put_Id", error.code().0))?;
        unsafe { logon_trigger.SetUserId(&BSTR::from(user_name.as_str())) }
            .map_err(|error| fail("ILogonTrigger::put_UserId", error.code().0))?;

        let repetition = unsafe { logon_trigger.Repetition() }
            .map_err(|error| fail("ILogonTrigger::get_Repetition", error.code().0))?;
        unsafe { repetition.SetInterval(&BSTR::from("P1D")) }
            .map_err(|error| fail("IRepetitionPattern::put_Interval", error.code().0))?;
    }

    {
        let action_collection: IActionCollection = unsafe { task.Actions() }
            .map_err(|error| fail("ITaskDefinition::get_Actions", error.code().0))?;
        let action = unsafe { action_collection.Create(TASK_ACTION_EXEC) }
            .map_err(|error| fail("IActionCollection::Create", error.code().0))?;
        let exec_action: IExecAction = action
            .cast()
            .map_err(|error| fail("QueryInterface(IExecAction)", error.code().0))?;

        unsafe { exec_action.SetPath(&BSTR::from(program_path)) }
            .map_err(|error| fail("IExecAction::put_Path", error.code().0))?;
        unsafe { exec_action.SetArguments(&BSTR::from(program_args)) }
            .map_err(|error| fail("IExecAction::put_Arguments", error.code().0))?;
        unsafe { exec_action.SetWorkingDirectory(&BSTR::from(working_dir)) }
            .map_err(|error| fail("IExecAction::put_WorkingDirectory", error.code().0))?;
    }

    // The C++ code passes `_variant_t()` for user/password and an empty BSTR
    // for the SDDL; an empty VARIANT has the same effect here.
    let empty = VARIANT::default();
    let _registered = unsafe {
        root_folder.RegisterTaskDefinition(
            &BSTR::from(task_name),
            &task,
            TASK_CREATE_OR_UPDATE.0,
            &empty,
            &empty,
            TASK_LOGON_INTERACTIVE_TOKEN,
            &empty,
        )
    }
    .map_err(|error| fail("ITaskFolder::RegisterTaskDefinition", error.code().0))?;

    Ok(())
}

/// Port of `TaskSchedulerHelper::unschedule`.
pub fn unschedule(task_name: &str) -> Result<(), TaskSchedulerException> {
    let _com = com_initialize();
    ensure_com_security()?;

    let service = unsafe { create_task_service()? };
    let root_folder: ITaskFolder = unsafe { service.GetFolder(&BSTR::from("\\")) }
        .map_err(|error| fail("ITaskService::GetFolder", error.code().0))?;

    // C++ ignores the result of DeleteTask.
    let _ = unsafe { root_folder.DeleteTask(&BSTR::from(task_name), 0) };
    Ok(())
}
