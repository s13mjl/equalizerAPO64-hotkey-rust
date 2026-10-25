//! 1:1 port of the fork's `HotkeyManager`/`HotkeyWorker`.
//!
//! Registration must happen on a thread that owns a Win32 message queue:
//! eframe/winit does not hand `WM_HOTKEY` to application code, exactly like
//! Qt 6 in the original fork.  The worker below is therefore an independent
//! thread running a classic `GetMessage` loop.

use std::collections::{HashMap, HashSet};
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use windows::Win32::Foundation::{LPARAM, WPARAM};
use windows::Win32::System::Registry::HKEY_CURRENT_USER;
use windows::Win32::UI::Input::KeyboardAndMouse::{
    RegisterHotKey, UnregisterHotKey, HOT_KEY_MODIFIERS,
};
use windows::Win32::UI::WindowsAndMessaging::{
    GetMessageW, PeekMessageW, PostThreadMessageW, MSG, PM_NOREMOVE, WM_HOTKEY, WM_NULL, WM_QUIT,
};

use crate::keyseq::{KeySequence, MOD_NOREPEAT};

pub const HOTKEY_BASE_ID: i32 = 0x0100;
pub const HOTKEY_MAX_ID: i32 = 0xBFFF;
pub const HOTKEY_REGPATH: &str = r"HKEY_CURRENT_USER\SOFTWARE\EqualizerAPO\Hotkeys";

pub fn hotkey_log_path() -> std::path::PathBuf {
    std::env::temp_dir().join("equalizerapo_hotkey.log")
}

/// `QDateTime::toString("yyyy-MM-dd hh:mm:ss.zzz")` with local time.
pub fn log_hotkey(message: &str) {
    let st = unsafe { windows::Win32::System::SystemInformation::GetLocalTime() };
    let timestamp = format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}.{:03}",
        st.wYear, st.wMonth, st.wDay, st.wHour, st.wMinute, st.wSecond, st.wMilliseconds
    );
    if let Ok(mut f) = OpenOptions::new().create(true).append(true).open(hotkey_log_path()) {
        let _ = writeln!(f, "{timestamp} {message}");
    }
}

#[derive(Clone, Debug)]
pub struct HotkeyInfo {
    pub id: i32,
    pub modifiers: u32,
    pub vk: u32,
    pub filter_id: String,
    pub key: KeySequence,
    pub enabled: bool,
}

enum OpKind {
    Register { id: i32, modifiers: u32, vk: u32 },
    Unregister { id: i32 },
}

struct PendingOp {
    kind: OpKind,
    done: Option<Sender<bool>>,
}

struct WorkerShared {
    queue: Mutex<std::collections::VecDeque<PendingOp>>,
    thread_id: AtomicU32,
    stopping: AtomicBool,
    events: Mutex<Sender<i32>>,
    wake: Mutex<Option<egui::Context>>,
}

pub struct HotkeyWorker {
    shared: Arc<WorkerShared>,
    thread: Option<JoinHandle<()>>,
}

impl HotkeyWorker {
    fn new(events: Sender<i32>) -> Self {
        let shared = Arc::new(WorkerShared {
            queue: Mutex::new(std::collections::VecDeque::new()),
            thread_id: AtomicU32::new(0),
            stopping: AtomicBool::new(false),
            events: Mutex::new(events),
            wake: Mutex::new(None),
        });
        let (started_tx, started_rx) = mpsc::channel::<u32>();
        let thread_shared = shared.clone();
        let join = thread::spawn(move || worker_main(thread_shared, started_tx));
        // Wait until the worker has created its message queue.  Without this
        // the first PostThreadMessage can be lost.
        let _ = started_rx.recv_timeout(Duration::from_secs(3));
        Self { shared, thread: Some(join) }
    }

    fn post_op(&self, kind: OpKind, wait: bool) -> bool {
        let (tx, rx) = mpsc::channel();
        {
            let mut q = self.shared.queue.lock().unwrap();
            q.push_back(PendingOp { kind, done: if wait { Some(tx) } else { None } });
        }
        let tid = self.shared.thread_id.load(Ordering::SeqCst);
        if tid != 0 {
            unsafe {
                let _ = PostThreadMessageW(tid, WM_NULL, WPARAM(0), LPARAM(0));
            }
        }
        if wait {
            match rx.recv_timeout(Duration::from_secs(2)) {
                Ok(v) => v,
                Err(RecvTimeoutError::Timeout) | Err(RecvTimeoutError::Disconnected) => false,
            }
        } else {
            true
        }
    }

    pub fn register_hotkey(&self, id: i32, modifiers: u32, vk: u32) -> bool {
        self.post_op(OpKind::Register { id, modifiers, vk }, true)
    }

    pub fn unregister_hotkey(&self, id: i32) {
        let _ = self.post_op(OpKind::Unregister { id }, true);
    }

    fn stop(&self) {
        log_hotkey("HotkeyWorker::stop() called");
        self.shared.stopping.store(true, Ordering::SeqCst);
        let tid = self.shared.thread_id.load(Ordering::SeqCst);
        if tid != 0 {
            unsafe {
                let _ = PostThreadMessageW(tid, WM_QUIT, WPARAM(0), LPARAM(0));
            }
        }
    }
}

impl Drop for HotkeyWorker {
    fn drop(&mut self) {
        log_hotkey("HotkeyWorker::~HotkeyWorker() destructor");
        self.stop();
        if let Some(t) = self.thread.take() {
            let _ = t.join();
        }
    }
}

fn worker_main(shared: Arc<WorkerShared>, started: Sender<u32>) {
    let tid = unsafe { windows::Win32::System::Threading::GetCurrentThreadId() };
    shared.thread_id.store(tid, Ordering::SeqCst);
    let mut registered: HashSet<i32> = HashSet::new();

    // Force creation of this thread's message queue before reporting ready.
    unsafe {
        let mut msg = MSG::default();
        let _ = PeekMessageW(&mut msg, None, 0, 0, PM_NOREMOVE);
    }
    log_hotkey(&format!("Worker thread started, tid={tid}"));
    let _ = started.send(tid);

    loop {
        let mut msg = MSG::default();
        let ret = unsafe { GetMessageW(&mut msg, None, 0, 0) };
        if ret.0 <= 0 {
            break;
        }
        match msg.message {
            WM_HOTKEY => {
                let id = msg.wParam.0 as i32;
                log_hotkey(&format!("Worker WM_HOTKEY id={id}"));
                if let Ok(tx) = shared.events.lock() {
                    let _ = tx.send(id);
                }
                // Wake an idle eframe event loop; otherwise a global hotkey
                // would only be handled on the next mouse/keyboard event.
                if let Ok(wake) = shared.wake.lock() {
                    if let Some(ctx) = wake.as_ref() {
                        ctx.request_repaint();
                    }
                }
            }
            WM_NULL => {
                loop {
                    let op = { shared.queue.lock().unwrap().pop_front() };
                    let Some(op) = op else { break };
                    match op.kind {
                        OpKind::Register { id, modifiers, vk } => {
                            let mods = modifiers | MOD_NOREPEAT;
                            let ok = unsafe {
                                RegisterHotKey(None, id, HOT_KEY_MODIFIERS(mods), vk).is_ok()
                            };
                            if ok {
                                registered.insert(id);
                                log_hotkey(&format!(
                                    "Worker RegisterHotKey OK: id={id} mod={mods} vk={vk}"
                                ));
                            } else {
                                let err = unsafe { windows::Win32::Foundation::GetLastError() };
                                log_hotkey(&format!(
                                    "Worker RegisterHotKey FAILED: id={id} err={}",
                                    err.0
                                ));
                            }
                            if let Some(tx) = op.done {
                                let _ = tx.send(ok);
                            }
                        }
                        OpKind::Unregister { id } => {
                            unsafe {
                                let _ = UnregisterHotKey(None, id);
                            }
                            registered.remove(&id);
                            log_hotkey(&format!("Worker unregistered: id={id}"));
                            if let Some(tx) = op.done {
                                let _ = tx.send(true);
                            }
                        }
                    }
                }
            }
            WM_QUIT => break,
            _ => {}
        }
    }

    // Unregister all hotkeys on the owning thread.
    for id in registered {
        unsafe {
            let _ = UnregisterHotKey(None, id);
        }
    }
    log_hotkey("Worker thread exiting");
}

pub struct HotkeyManager {
    worker: HotkeyWorker,
    hotkeys: HashMap<String, HotkeyInfo>,
    next_hotkey_id: i32,
    events: Receiver<i32>,
    pending_filter_ids: Vec<String>,
}

impl HotkeyManager {
    pub fn new() -> Self {
        let (tx, rx) = mpsc::channel();
        let worker = HotkeyWorker::new(tx);
        log_hotkey("HotkeyManager created, worker started");
        Self {
            worker,
            hotkeys: HashMap::new(),
            next_hotkey_id: HOTKEY_BASE_ID,
            events: rx,
            pending_filter_ids: Vec::new(),
        }
    }

    /// Drains worker events; the caller applies the returned filter toggles on
    /// the UI thread.
    pub fn poll(&mut self) -> Vec<String> {
        while let Ok(id) = self.events.try_recv() {
            log_hotkey(&format!("onWorkerHotkey: id={id}"));
            self.handle_hotkey(id);
        }
        std::mem::take(&mut self.pending_filter_ids)
    }

    pub fn register_hotkey(&mut self, filter_id: &str, key_sequence: Option<KeySequence>) -> bool {
        let Some(key) = key_sequence else {
            self.unregister_hotkey(filter_id);
            return true;
        };
        let (modifiers, vk) = key.to_win();

        // Unregister existing hotkey for this filter.
        self.unregister_hotkey(filter_id);

        // Check for conflicts against the manager's own registrations.
        if self.hotkeys.values().any(|h| h.modifiers == modifiers && h.vk == vk) {
            return false;
        }

        let id = self.next_hotkey_id;
        self.next_hotkey_id += 1;
        if self.next_hotkey_id > HOTKEY_MAX_ID {
            self.next_hotkey_id = HOTKEY_BASE_ID;
        }

        if !self.worker.register_hotkey(id, modifiers, vk) {
            log_hotkey(&format!(
                "RegisterHotKey FAILED: filter={filter_id} key={}",
                key.to_string_qt()
            ));
            return false;
        }

        log_hotkey(&format!(
            "RegisterHotKey OK: filter={filter_id} id={id} key={} mod={modifiers} vk={vk}",
            key.to_string_qt()
        ));
        self.hotkeys.insert(
            filter_id.to_string(),
            HotkeyInfo {
                id,
                modifiers,
                vk,
                filter_id: filter_id.to_string(),
                key,
                enabled: true,
            },
        );
        self.save_settings();
        true
    }

    pub fn unregister_hotkey(&mut self, filter_id: &str) {
        let Some(info) = self.hotkeys.remove(filter_id) else { return };
        self.worker.unregister_hotkey(info.id);
        self.save_settings();
    }

    pub fn clear_all(&mut self) {
        let ids: Vec<i32> = self.hotkeys.values().map(|h| h.id).collect();
        for id in ids {
            self.worker.unregister_hotkey(id);
        }
        self.hotkeys.clear();
    }

    pub fn get_hotkey(&self, filter_id: &str) -> Option<KeySequence> {
        self.hotkeys.get(filter_id).map(|h| h.key)
    }

    pub fn has_hotkey(&self, filter_id: &str) -> bool {
        self.hotkeys.contains_key(filter_id)
    }

    pub fn handle_hotkey(&mut self, id: i32) -> bool {
        if let Some(info) = self.hotkeys.values().find(|h| h.id == id) {
            log_hotkey(&format!(
                "handleHotkey matched: id={id} filter={}",
                info.filter_id
            ));
            self.pending_filter_ids.push(info.filter_id.clone());
            return true;
        }
        log_hotkey(&format!("handleHotkey NO MATCH: id={id}"));
        false
    }

    /// Reads the persisted groups without registering them.  Registration
    /// happens in `FilterTable::register_filter_hotkey` when the GUIs are
    /// created, exactly like the C++ implementation.
    pub fn load_settings(&mut self) -> HashMap<String, KeySequence> {
        let ids: Vec<i32> = self.hotkeys.values().map(|h| h.id).collect();
        for id in ids {
            self.worker.unregister_hotkey(id);
        }
        self.hotkeys.clear();
        load_hotkeys_from(HOTKEY_REGPATH)
    }

    /// Gives the worker a way to wake an idle eframe event loop when a global
    /// hotkey fires.
    pub fn set_repaint_context(&self, ctx: egui::Context) {
        if let Ok(mut wake) = self.worker.shared.wake.lock() {
            *wake = Some(ctx);
        }
    }

    pub fn save_settings(&self) {
        let map: Vec<(String, String)> = self
            .hotkeys
            .values()
            .map(|h| (h.filter_id.clone(), h.key.to_string_qt()))
            .collect();
        save_hotkeys_to(HOTKEY_REGPATH, &map);
    }

    pub fn format_key_sequence(key: Option<KeySequence>) -> String {
        match key {
            Some(k) => k.to_string_qt(),
            None => "None".to_string(),
        }
    }
}

impl Default for HotkeyManager {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for HotkeyManager {
    fn drop(&mut self) {
        log_hotkey("HotkeyManager::~HotkeyManager() destructor");
        self.clear_all();
        // Worker stops and joins in its own Drop.
    }
}

pub fn save_hotkeys_to(base: &str, hotkeys: &[(String, String)]) {
    delete_registry_tree(base);
    let _ = eapo_common::registry_helper::create_key(base);
    for (filter_id, sequence) in hotkeys {
        let key = format!("{base}\\{filter_id}");
        if eapo_common::registry_helper::create_key(&key).is_ok() {
            let _ = eapo_common::registry_helper::write_value(&key, "keySequence", sequence);
        }
    }
}

pub fn load_hotkeys_from(base: &str) -> HashMap<String, KeySequence> {
    let mut map = HashMap::new();
    let Ok(ids) = eapo_common::registry_helper::enum_sub_keys(base) else {
        return map;
    };
    for id in ids {
        let key = format!("{base}\\{id}");
        if let Ok(text) = eapo_common::registry_helper::read_value(&key, "keySequence") {
            if let Some(seq) = KeySequence::parse(&text) {
                map.insert(id, seq);
            }
        }
    }
    map
}

fn delete_registry_tree(base: &str) {
    let Some(sub) = base.strip_prefix(r"HKEY_CURRENT_USER\") else { return };
    let wide: Vec<u16> = sub.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        let _ = windows::Win32::System::Registry::RegDeleteTreeW(
            HKEY_CURRENT_USER,
            windows::core::PCWSTR(wide.as_ptr()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_BASE: &str = r"HKEY_CURRENT_USER\SOFTWARE\EqualizerAPO\RustEditorHotkeyTest";

    #[test]
    fn registry_round_trip_uses_qkeysequence_strings() {
        let entries = vec![
            ("abc123".to_string(), "Ctrl+Alt+X".to_string()),
            ("def456".to_string(), "Ctrl+Shift+F5".to_string()),
            ("x".to_string(), "Alt+F4".to_string()),
        ];
        save_hotkeys_to(TEST_BASE, &entries);
        let loaded = load_hotkeys_from(TEST_BASE);
        assert_eq!(loaded.get("abc123").unwrap().to_string_qt(), "Ctrl+Alt+X");
        assert_eq!(loaded.get("def456").unwrap().to_string_qt(), "Ctrl+Shift+F5");
        assert_eq!(loaded.get("x").unwrap().to_string_qt(), "Alt+F4");
        // A second save rewrites the whole key (settings.clear()).
        save_hotkeys_to(TEST_BASE, &[("only".to_string(), "Alt+Z".to_string())]);
        let loaded = load_hotkeys_from(TEST_BASE);
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.get("only").unwrap().to_string_qt(), "Alt+Z");
        delete_registry_tree(TEST_BASE);
    }

    /// Requires an interactive desktop; run with
    /// `cargo test -p eapo-editor hotkey_round_trip -- --ignored`.
    #[test]
    #[ignore]
    fn hotkey_round_trip_through_win32_message_loop() {
        use windows::Win32::UI::Input::KeyboardAndMouse::{
            SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_KEYUP,
            VIRTUAL_KEY, VK_CONTROL, VK_F9, VK_MENU, VK_SHIFT,
        };

        fn send(vk: VIRTUAL_KEY, up: bool) -> u32 {
            let mut input = INPUT::default();
            input.r#type = INPUT_KEYBOARD;
            input.Anonymous = INPUT_0 {
                ki: KEYBDINPUT {
                    wVk: vk,
                    wScan: 0,
                    dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) },
                    time: 0,
                    dwExtraInfo: 0,
                },
            };
            unsafe { SendInput(&[input], std::mem::size_of::<INPUT>() as i32) }
        }

        let mut manager = HotkeyManager::new();
        let key = KeySequence {
            modifiers: crate::keyseq::Modifiers {
                ctrl: true,
                alt: true,
                shift: true,
                meta: false,
            },
            key: crate::keyseq::KeyCode::Function(9),
        };
        assert!(manager.register_hotkey("e2e-filter", Some(key)));

        std::thread::sleep(Duration::from_millis(150));
        send(VK_CONTROL, false);
        send(VK_MENU, false);
        send(VK_SHIFT, false);
        send(VK_F9, false);
        send(VK_F9, true);
        send(VK_SHIFT, true);
        send(VK_MENU, true);
        send(VK_CONTROL, true);

        let deadline = std::time::Instant::now() + Duration::from_secs(2);
        let mut toggled = Vec::new();
        while std::time::Instant::now() < deadline {
            toggled = manager.poll();
            if toggled.iter().any(|id| id == "e2e-filter") {
                break;
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        manager.unregister_hotkey("e2e-filter");
        assert!(
            toggled.iter().any(|id| id == "e2e-filter"),
            "hotkey was not delivered: {:?}",
            toggled
        );
    }

    #[test]
    fn log_messages_match_the_cpp_catalogue() {
        let path = hotkey_log_path();
        let _ = std::fs::remove_file(&path);
        log_hotkey("unit test message");
        let text = std::fs::read_to_string(&path).unwrap();
        assert!(text.contains(" unit test message"));
        assert_eq!(&text[4..5], "-");
        assert_eq!(&text[7..8], "-");
        assert_eq!(&text[10..11], " ");
        assert_eq!(&text[13..14], ":");
        assert_eq!(&text[16..17], ":");
        assert_eq!(&text[19..20], ".");
    }
}
