//! Port of the Equalizer APO `UpdateChecker` application.
//!
//! The original is a small Qt dialog which checks
//! `https://equalizerapo.sourceforge.io/checkVersion?installed=<version>`
//! for a newer release. This port keeps the same URL, registry keys,
//! command-line options and retry/timeout behaviour, but renders the dialog
//! with egui and performs HTTP requests through WinHTTP.

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod i18n;

use std::time::Duration;

use eapo_common::registry_helper;
use serde_json::Value;
use windows::core::{w, HSTRING, PCWSTR};
use windows::Win32::Networking::WinHttp::{
    WinHttpCloseHandle, WinHttpConnect, WinHttpOpen, WinHttpOpenRequest, WinHttpQueryDataAvailable,
    WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetOption, WinHttpSetTimeouts, ERROR_WINHTTP_NAME_NOT_RESOLVED, ERROR_WINHTTP_TIMEOUT,
    INTERNET_DEFAULT_HTTPS_PORT, WINHTTP_ACCESS_TYPE_DEFAULT_PROXY, WINHTTP_FLAG_SECURE,
    WINHTTP_OPTION_REDIRECT_POLICY, WINHTTP_OPTION_REDIRECT_POLICY_ALWAYS,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE,
};
use windows::Win32::System::SystemInformation::GetLocalTime;
use windows::Win32::UI::Shell::ShellExecuteW;
use windows::Win32::UI::WindowsAndMessaging::SW_SHOWNORMAL;

// UPDATE_CHECKER_REGPATH USER_REGPATH L"\\Update Checker"
const UPDATE_CHECKER_REGPATH: &str = r"HKEY_CURRENT_USER\SOFTWARE\EqualizerAPO\Update Checker";
const UPDATE_CHECK_URL: &str = "https://equalizerapo.sourceforge.io/checkVersion";
const TASK_NAME: &str = "EqualizerAPOUpdateChecker";
const CHECK_TIMEOUT_MS: i32 = 10_000;

#[derive(Clone, Copy, PartialEq, Eq)]
enum Mode {
    Normal,
    Auto,
}

struct VersionEntry {
    version: String,
    date: String,
    info: Vec<String>,
}

struct UpdateData {
    download_url: String,
    newest_version: String,
    entries: Vec<VersionEntry>,
}

fn main() {
    i18n::set_language(i18n::system_language());
    let args: Vec<String> = std::env::args().skip(1).collect();

    let mut auto_mode = false;
    let mut install_task = false;
    let mut uninstall_task = false;
    for arg in &args {
        match arg.as_str() {
            "-a" | "--a" => auto_mode = true,
            "-i" | "--i" => install_task = true,
            "-u" | "--u" => uninstall_task = true,
            "-h" | "--help" | "-?" => {
                print_help();
                std::process::exit(0);
            }
            other => {
                eprintln!("Unknown option '{}'.", other);
                print_help();
                std::process::exit(1);
            }
        }
    }

    // QCommandLineParser "last one wins" is irrelevant here; the original
    // handles install before uninstall, then treats the remaining options.
    if install_task {
        let exe = std::env::current_exe().unwrap_or_default();
        let program_path = exe.to_string_lossy().to_string();
        let working_dir = exe
            .parent()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        if let Err(error) = eapo_common::task_scheduler_helper::schedule_at_logon(
            TASK_NAME,
            &program_path,
            "-a",
            &working_dir,
        ) {
            run_info_message(&i18n::tr("Error installing Update Checker"), &error.to_string());
            std::process::exit(2);
        }
        std::process::exit(0);
    }
    if uninstall_task {
        if let Err(error) =
            eapo_common::task_scheduler_helper::unschedule(TASK_NAME)
        {
            run_info_message(&i18n::tr("Error uninstalling Update Checker"), &error.to_string());
            std::process::exit(2);
        }
        std::process::exit(0);
    }

    let mode = if auto_mode { Mode::Auto } else { Mode::Normal };
    let version = env!("CARGO_PKG_VERSION").to_string();
    let mut url = format!(
        "{}?installed={}",
        UPDATE_CHECK_URL,
        percent_encode(&version)
    );
    if mode == Mode::Auto {
        let last_check = registry_helper::read_value(UPDATE_CHECKER_REGPATH, "lastCheckDate")
            .ok()
            .unwrap_or_default();
        if let Some(last) = parse_iso_datetime(&last_check) {
            let now = local_now_seconds();
            if now - last < 24 * 60 * 60 {
                std::process::exit(1);
            }
        }
        let _ = registry_helper::write_value(
            UPDATE_CHECKER_REGPATH,
            "lastCheckDate",
            &format_local_iso_now(),
        );

        let skip_version = registry_helper::read_value(UPDATE_CHECKER_REGPATH, "skipVersion")
            .ok()
            .unwrap_or_default();
        if !skip_version.is_empty() {
            url.push_str("&skip=");
            url.push_str(&percent_encode(&skip_version));
        }
    } else {
        url.push_str("&manual=true");
    }

    let tries = if mode == Mode::Auto { 10 } else { 1 };
    let outcome = request_with_retries(&url, tries);

    match outcome {
        HttpOutcome::TimedOut => {
            // The Qt version leaves the event loop on timeout with a pending
            // reply and therefore does not report an error either.
            std::process::exit(0);
        }
        HttpOutcome::Error(message) => {
            run_failure_message(&message);
            std::process::exit(0);
        }
        HttpOutcome::Ok(body) => {
            if body.is_empty() {
                if mode != Mode::Auto {
                    run_info_message(
                        &i18n::tr("No update available"),
                        &i18n::tr_arg(
                            "The installed version %0 of Equalizer APO is up to date.",
                            &[&version],
                        ),
                    );
                }
                std::process::exit(0);
            }
            let text = String::from_utf8_lossy(&body).to_string();
            match parse_update_document(&text) {
                Err(message) => {
                    run_failure_message_titled(
                        &message,
                        &i18n::tr("Error while reading response of update check"),
                    );
                }
                Ok(data) => {
                    run_update_dialog(data);
                }
            }
            std::process::exit(0);
        }
    }
}

fn print_help() {
    println!("Usage: UpdateChecker [options]");
    println!();
    println!("Options:");
    println!("  -a, --a  Automatic mode (no dialog if no new version, respect skip version, only check every 24 hours)");
    println!("  -i, --i  Install scheduled task");
    println!("  -u, --u  Uninstall scheduled task");
    println!("  -h, --help  Displays help on commandline options.");
}

/// Equivalent of `QUrl::toPercentEncoding`.
fn percent_encode(value: &str) -> String {
    let mut result = String::new();
    for byte in value.bytes() {
        if byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'.' | b'_' | b'~') {
            result.push(byte as char);
        } else {
            result.push_str(&format!("%{:02X}", byte));
        }
    }
    result
}

enum HttpOutcome {
    Ok(Vec<u8>),
    TimedOut,
    Error(String),
}

fn request_with_retries(url: &str, mut tries: u32) -> HttpOutcome {
    let mut last = HttpOutcome::TimedOut;
    while tries > 0 {
        tries -= 1;
        last = http_get(url);
        match &last {
            HttpOutcome::Ok(_) => break,
            HttpOutcome::Error(message) => {
                // QNetworkReply::HostNotFoundError was the only error that was
                // retried by the original loop.
                if !message.contains("NAME_NOT_RESOLVED") {
                    break;
                }
            }
            HttpOutcome::TimedOut => {}
        }
        if tries > 0 {
            std::thread::sleep(Duration::from_secs(5));
        }
    }
    last
}

fn winhttp_error_string(error: &windows::core::Error) -> String {
    format!("{:?}", error)
}

fn http_get(url: &str) -> HttpOutcome {
    let (host, path) = match split_url(url) {
        Some(parts) => parts,
        None => return HttpOutcome::Error("Invalid URL".to_string()),
    };

    unsafe {
        let session = WinHttpOpen(
            w!("EqualizerAPO Update Checker"),
            WINHTTP_ACCESS_TYPE_DEFAULT_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        );
        if session.is_null() {
            return HttpOutcome::Error("Could not initialize WinHTTP".to_string());
        }
        let _guard_session = HandleGuard(session);

        let _ = WinHttpSetTimeouts(
            session,
            CHECK_TIMEOUT_MS,
            CHECK_TIMEOUT_MS,
            CHECK_TIMEOUT_MS,
            CHECK_TIMEOUT_MS,
        );

        let host_w = HSTRING::from(host);
        let connection = WinHttpConnect(session, &host_w, INTERNET_DEFAULT_HTTPS_PORT, 0);
        if connection.is_null() {
            return HttpOutcome::Error("Could not connect to the update server".to_string());
        }
        let _guard_connection = HandleGuard(connection);

        let path_w = HSTRING::from(path);
        let request = WinHttpOpenRequest(
            connection,
            w!("GET"),
            &path_w,
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        );
        if request.is_null() {
            return HttpOutcome::Error("Could not create HTTP request".to_string());
        }
        let _guard_request = HandleGuard(request);

        // QNetworkAccessManager follows redirects; WinHTTP does not by default.
        let redirect_policy: u32 = WINHTTP_OPTION_REDIRECT_POLICY_ALWAYS;
        let policy_bytes = std::slice::from_raw_parts(
            &redirect_policy as *const u32 as *const u8,
            std::mem::size_of::<u32>(),
        );
        let _ = WinHttpSetOption(
            Some(request as *const core::ffi::c_void),
            WINHTTP_OPTION_REDIRECT_POLICY,
            Some(policy_bytes),
        );

        if let Err(error) = WinHttpSendRequest(request, None, None, 0, 0, 0) {
            return winhttp_error_outcome(&error);
        }
        if let Err(error) = WinHttpReceiveResponse(request, std::ptr::null_mut()) {
            return winhttp_error_outcome(&error);
        }

        let mut status_code: u32 = 0;
        let mut size = std::mem::size_of::<u32>() as u32;
        let _ = WinHttpQueryHeaders(
            request,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status_code as *mut u32 as *mut core::ffi::c_void),
            &mut size,
            std::ptr::null_mut(),
        );
        if status_code >= 400 {
            return HttpOutcome::Error(format!(
                "Error transferring {} - server replied: HTTP {}",
                url, status_code
            ));
        }

        let mut body = Vec::new();
        loop {
            let mut available: u32 = 0;
            if let Err(error) = WinHttpQueryDataAvailable(request, &mut available) {
                return winhttp_error_outcome(&error);
            }
            if available == 0 {
                break;
            }
            let mut chunk = vec![0u8; available as usize];
            let mut read: u32 = 0;
            if let Err(error) = WinHttpReadData(
                request,
                chunk.as_mut_ptr() as *mut core::ffi::c_void,
                available,
                &mut read,
            ) {
                return winhttp_error_outcome(&error);
            }
            chunk.truncate(read as usize);
            body.extend_from_slice(&chunk);
        }
        HttpOutcome::Ok(body)
    }
}

fn win32_error_code(error: &windows::core::Error) -> u32 {
    let code = error.code().0 as u32;
    // BOOL::ok() converts GetLastError() with HRESULT_FROM_WIN32, so unwrap
    // the facility code before comparing with WinHTTP error constants.
    if (code >> 16) & 0x1fff == 7 {
        code & 0xffff
    } else {
        code
    }
}

fn winhttp_error_outcome(error: &windows::core::Error) -> HttpOutcome {
    if win32_error_code(error) == ERROR_WINHTTP_TIMEOUT {
        HttpOutcome::TimedOut
    } else if win32_error_code(error) == ERROR_WINHTTP_NAME_NOT_RESOLVED {
        HttpOutcome::Error(format!("NAME_NOT_RESOLVED: {}", winhttp_error_string(error)))
    } else {
        HttpOutcome::Error(winhttp_error_string(error))
    }
}

struct HandleGuard(*mut core::ffi::c_void);

impl Drop for HandleGuard {
    fn drop(&mut self) {
        if !self.0.is_null() {
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

fn split_url(url: &str) -> Option<(&str, &str)> {
    let rest = url.strip_prefix("https://")?;
    match rest.find('/') {
        Some(index) => Some((&rest[..index], &rest[index..])),
        None => Some((rest, "/")),
    }
}

/// Parses the JSON document returned by the update service. The error string
/// mirrors `QJsonParseError::errorString()` closely enough for the dialog.
fn parse_update_document(text: &str) -> Result<UpdateData, String> {
    let value: Value = serde_json::from_str(text).map_err(|e| e.to_string())?;
    let object = value
        .as_object()
        .ok_or_else(|| "The response is not a JSON object".to_string())?;
    let download_url = object
        .get("download-url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let mut entries = Vec::new();
    for entry in object
        .get("versions")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default()
    {
        let version = entry
            .get("version")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let date = entry
            .get("date")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string();
        let info = entry
            .get("info")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default();
        entries.push(VersionEntry {
            version,
            date,
            info,
        });
    }
    let newest_version = entries
        .first()
        .map(|entry| entry.version.clone())
        .unwrap_or_default();
    Ok(UpdateData {
        download_url,
        newest_version,
        entries,
    })
}

// ---------------------------------------------------------------------------
// Registry helpers
// ---------------------------------------------------------------------------

fn set_skip_version(version: &str) {
    let _ = registry_helper::write_value(UPDATE_CHECKER_REGPATH, "skipVersion", version);
}

fn clear_skip_version() {
    let _ = registry_helper::delete_value(UPDATE_CHECKER_REGPATH, "skipVersion");
}

fn open_download_page(url: &str) {
    let operation = HSTRING::from("open");
    let file = HSTRING::from(url);
    unsafe {
        ShellExecuteW(
            None,
            &operation,
            &file,
            PCWSTR::null(),
            PCWSTR::null(),
            SW_SHOWNORMAL,
        );
    }
}

// ---------------------------------------------------------------------------
// Time helpers (Qt ISODate handling)
// ---------------------------------------------------------------------------

fn local_now() -> (i64, u32, u32, u32, u32, u32) {
    unsafe {
        let st = GetLocalTime();
        (
            st.wYear as i64,
            st.wMonth as u32,
            st.wDay as u32,
            st.wHour as u32,
            st.wMinute as u32,
            st.wSecond as u32,
        )
    }
}

fn format_local_iso_now() -> String {
    let (year, month, day, hour, minute, second) = local_now();
    format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        year, month, day, hour, minute, second
    )
}

/// Seconds since the Unix epoch for a proleptic Gregorian date/time, ignoring
/// time zones. This is only used to compare two local wall-clock timestamps.
fn civil_seconds(year: i64, month: u32, day: u32, hour: u32, minute: u32, second: u32) -> i64 {
    let y = if month <= 2 { year - 1 } else { year };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (month as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146097 + doe - 719468;
    days * 86400 + hour as i64 * 3600 + minute as i64 * 60 + second as i64
}

fn local_now_seconds() -> i64 {
    let (year, month, day, hour, minute, second) = local_now();
    civil_seconds(year, month, day, hour, minute, second)
}

fn parse_iso_datetime(text: &str) -> Option<i64> {
    let bytes = text.as_bytes();
    if bytes.len() < 19 {
        return None;
    }
    let year = text.get(0..4)?.parse::<i64>().ok()?;
    let month = text.get(5..7)?.parse::<u32>().ok()?;
    let day = text.get(8..10)?.parse::<u32>().ok()?;
    let hour = text.get(11..13)?.parse::<u32>().ok()?;
    let minute = text.get(14..16)?.parse::<u32>().ok()?;
    let second = text.get(17..19)?.parse::<u32>().ok()?;
    if bytes.get(4) != Some(&b'-') || bytes.get(7) != Some(&b'-') || bytes.get(10) != Some(&b'T') {
        return None;
    }
    Some(civil_seconds(year, month, day, hour, minute, second))
}

// ---------------------------------------------------------------------------
// egui UI
// ---------------------------------------------------------------------------

fn native_options(width: f32, height: f32, title: &str) -> eframe::NativeOptions {
    eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title(title)
            .with_inner_size([width, height])
            .with_resizable(true),
        ..Default::default()
    }
}

fn run_update_dialog(data: UpdateData) {
    let title = i18n::tr("Update available");
    let options = native_options(400.0, 340.0, &title);
    let _ = eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            eapo_common::egui_fonts::install_cjk_font(&cc.egui_ctx);
            Ok(Box::new(UpdateDialog { data }))
        }),
    );
}

struct UpdateDialog {
    data: UpdateData,
}

impl eframe::App for UpdateDialog {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.add(
                egui::Label::new(
                    egui::RichText::new(i18n::tr("A newer version of Equalizer APO is available:"))
                        .strong()
                        .size(16.0),
                )
                .wrap(),
            );
            ui.add_space(6.0);
            egui::ScrollArea::vertical()
                .max_height(ui.available_height() - 40.0)
                .auto_shrink([false, false])
                .show(ui, |ui| {
                    for entry in &self.data.entries {
                        ui.horizontal_wrapped(|ui| {
                            ui.label(egui::RichText::new(&entry.version).strong());
                            if !entry.date.is_empty() {
                                ui.label(
                                    egui::RichText::new(format!("({})", entry.date))
                                        .italics()
                                        .color(egui::Color32::GRAY)
                                        .small(),
                                );
                            }
                        });
                        for item in &entry.info {
                            ui.horizontal_wrapped(|ui| {
                                ui.label(crate::i18n::tr("•"));
                                ui.label(item);
                            });
                        }
                        ui.add_space(4.0);
                    }
                });
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.button(crate::i18n::tr("Go to website")).clicked() {
                    open_download_page(&self.data.download_url);
                    clear_skip_version();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui.button(crate::i18n::tr("Remind me later")).clicked() {
                    clear_skip_version();
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
                if ui.button(crate::i18n::tr("Skip this version")).clicked() {
                    set_skip_version(&self.data.newest_version);
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }
}

fn run_failure_message(message: &str) {
    run_failure_message_titled(message, &i18n::tr("Error while checking for update"));
}

fn run_failure_message_titled(message: &str, title: &str) {
    let title = title.to_string();
    if registry_helper::read_value(UPDATE_CHECKER_REGPATH, "hideFailureMessage")
        .map(|value| value.eq_ignore_ascii_case("true") || value == "1")
        .unwrap_or(false)
    {
        return;
    }
    let options = native_options(460.0, 200.0, &title);
    let message = message.to_string();
    let dialog_title = title.clone();
    let _ = eframe::run_native(
        &title,
        options,
        Box::new(move |cc| {
            eapo_common::egui_fonts::install_cjk_font(&cc.egui_ctx);
            Ok(Box::new(MessageDialog {
                title: dialog_title,
                message,
                checkbox: Some((
                    i18n::tr("Don't show message for failed update check again"),
                    false,
                )),
                on_ok: MessageAction::HideFailures,
            }))
        }),
    );
}

fn run_info_message(title: &str, message: &str) {
    let options = native_options(380.0, 160.0, title);
    let title = title.to_string();
    let app_title = title.clone();
    let message = message.to_string();
    let _ = eframe::run_native(
        &app_title,
        options,
        Box::new(move |cc| {
            eapo_common::egui_fonts::install_cjk_font(&cc.egui_ctx);
            Ok(Box::new(MessageDialog {
                title,
                message,
                checkbox: None,
                on_ok: MessageAction::None,
            }))
        }),
    );
}

enum MessageAction {
    None,
    HideFailures,
}

struct MessageDialog {
    title: String,
    message: String,
    checkbox: Option<(String, bool)>,
    on_ok: MessageAction,
}

impl eframe::App for MessageDialog {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        egui::CentralPanel::default().show(ctx, |ui| {
            ui.heading(&self.title);
            ui.add_space(8.0);
            ui.label(&self.message);
            ui.add_space(8.0);
            if let Some((text, checked)) = &mut self.checkbox {
                ui.checkbox(checked, text.as_str());
            }
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.button(crate::i18n::tr("OK")).clicked() {
                    if let MessageAction::HideFailures = self.on_ok {
                        if self
                            .checkbox
                            .as_ref()
                            .map(|(_, checked)| *checked)
                            .unwrap_or(false)
                        {
                            let _ = registry_helper::write_value(
                                UPDATE_CHECKER_REGPATH,
                                "hideFailureMessage",
                                "true",
                            );
                        }
                    }
                    ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
        });
    }
}
