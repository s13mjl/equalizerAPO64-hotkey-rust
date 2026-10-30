//! Port of `FilterEngine.{h,cpp}`.
//!
//! Loading happens on a separate thread, exactly like the original: a new
//! configuration is built off the audio thread and crossfaded in. A Win32
//! semaphore gates reloads while a crossfade is still running.

use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;

use eapo_common::channel_helper;
use eapo_common::precision_timer::PrecisionTimer;
use eapo_common::{registry_helper, APP_REGPATH};
use eapo_expression::{ExpressionHost, ParserError, ParserX, Value};
use eapo_filters::IFilter;
use windows::core::PCWSTR;
use windows::Win32::Foundation::{CloseHandle, HANDLE, WAIT_OBJECT_0};
use windows::Win32::Storage::FileSystem::{
    FindCloseChangeNotification, FindFirstChangeNotificationW, FindNextChangeNotification,
    FILE_NOTIFY_CHANGE_FILE_NAME, FILE_NOTIFY_CHANGE_LAST_WRITE,
};
use windows::Win32::System::Registry::{
    RegCloseKey, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_CLASSES_ROOT,
    HKEY_CURRENT_CONFIG, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, HKEY_USERS, KEY_NOTIFY,
    KEY_WOW64_64KEY, REG_NOTIFY_CHANGE_LAST_SET, REG_SAM_FLAGS,
};
use windows::Win32::System::Threading::{
    CreateEventW, CreateSemaphoreW, ReleaseSemaphore, ResetEvent, SetEvent, WaitForMultipleObjects,
    WaitForSingleObject, INFINITE,
};

use crate::factories::{self, ExpressionHostImpl};
use crate::filter_configuration::{FilterConfiguration, FilterInfo};

pub(crate) struct AudioSettings {
    pub sample_rate: f32,
    pub input_channel_count: u32,
    pub real_channel_count: u32,
    pub output_channel_count: u32,
    pub channel_mask: u32,
    pub max_frame_count: u32,
}

impl Default for AudioSettings {
    fn default() -> Self {
        Self {
            sample_rate: 0.0,
            input_channel_count: 0,
            real_channel_count: 0,
            output_channel_count: 0,
            channel_mask: 0,
            max_frame_count: 0,
        }
    }
}

pub(crate) struct ConfigState {
    pub current: Option<FilterConfiguration>,
    pub next: Option<FilterConfiguration>,
    pub previous: Option<FilterConfiguration>,
    pub transition_counter: u32,
}

/// Holds everything used while (re)loading a configuration. It is shared with
/// the notification thread.
pub(crate) struct Loader {
    pub parser: ParserX,
    pub settings: AudioSettings,
    pub config_path: String,
    pub pre_mix: bool,
    pub capture: bool,
    pub post_mix_installed: bool,
    pub device_name: String,
    pub connection_name: String,
    pub device_guid: String,
    pub device_string: String,
    pub watch_registry_keys: HashSet<String>,
    pub configs: Arc<Mutex<ConfigState>>,
    pub transition_length: u32,

    // channels while loading
    pub filter_infos: Vec<FilterInfo>,
    pub current_channel_names: Vec<String>,
    pub last_channel_names: Vec<String>,
    pub last_new_channel_names: Vec<String>,
    pub all_channel_names: Vec<String>,
    pub last_in_place: bool,

    // factory state
    pub device_matches: bool,
    pub if_true_count: u32,
    pub if_false_count: u32,
    pub if_true_count_stack: Vec<u32>,
    pub if_execute_else: bool,
    pub include_recursion_depth: i32,
    pub stage_matches: bool,
    pub stage_matches_stack: Vec<bool>,
}

impl Loader {
    fn new(configs: Arc<Mutex<ConfigState>>) -> Self {
        let mut parser = ParserX::new();
        parser.enable_auto_create_var(true);
        Self {
            parser,
            settings: AudioSettings::default(),
            config_path: String::new(),
            pre_mix: false,
            capture: false,
            post_mix_installed: true,
            device_name: String::new(),
            connection_name: String::new(),
            device_guid: String::new(),
            device_string: String::new(),
            watch_registry_keys: HashSet::new(),
            configs,
            transition_length: 0,
            filter_infos: Vec::new(),
            current_channel_names: Vec::new(),
            last_channel_names: Vec::new(),
            last_new_channel_names: Vec::new(),
            all_channel_names: Vec::new(),
            last_in_place: false,
            device_matches: true,
            if_true_count: 0,
            if_false_count: 0,
            if_true_count_stack: Vec::new(),
            if_execute_else: false,
            include_recursion_depth: -1,
            stage_matches: false,
            stage_matches_stack: Vec::new(),
        }
    }

    pub(crate) fn watch_registry_key(&mut self, key: &str) {
        self.watch_registry_keys.insert(key.to_string());
    }

    pub(crate) fn eval_expression(&mut self, expression: &str) -> Result<Value, ParserError> {
        let mut parser = std::mem::take(&mut self.parser);
        parser.enable_auto_create_var(true);
        let mut host = ExpressionHostImpl {
            watched: &mut self.watch_registry_keys,
        };
        let host_ref: &mut dyn ExpressionHost = &mut host;
        let result = parser.eval_with_host(expression, Some(host_ref));
        self.parser = parser;
        result
    }

    pub(crate) fn add_filters(&mut self, filters: Vec<Box<dyn IFilter>>) {
        for mut filter in filters {
            let in_place = filter.get_in_place();
            let select_channels = filter.get_select_channels();
            let saved_channel_names = self.current_channel_names.clone();
            let all_channels = filter.get_all_channels();
            if all_channels {
                self.current_channel_names = self.all_channel_names.clone();
            }

            let in_channels = if self.last_channel_names == self.current_channel_names {
                Vec::new()
            } else {
                self.current_channel_names
                    .iter()
                    .map(|name| {
                        self.all_channel_names
                            .iter()
                            .position(|n| n == name)
                            .unwrap_or(self.all_channel_names.len())
                    })
                    .collect()
            };
            self.last_channel_names = self.current_channel_names.clone();

            let new_channel_names = filter.initialize(
                self.settings.sample_rate,
                self.settings.max_frame_count as usize,
                self.current_channel_names.clone(),
            );

            let out_channels = if in_place
                && self.last_in_place
                && self.last_new_channel_names == new_channel_names
            {
                Vec::new()
            } else {
                let mut channels = Vec::with_capacity(new_channel_names.len());
                for name in &new_channel_names {
                    match self.all_channel_names.iter().position(|n| n == name) {
                        Some(p) => channels.push(p),
                        None => {
                            channels.push(self.all_channel_names.len());
                            self.all_channel_names.push(name.clone());
                        }
                    }
                }
                channels
            };

            self.last_new_channel_names = new_channel_names.clone();
            self.last_in_place = in_place;
            if !in_place {
                std::mem::swap(&mut self.last_channel_names, &mut self.last_new_channel_names);
            }

            self.filter_infos.push(FilterInfo {
                filter,
                in_place,
                in_channels,
                out_channels,
            });

            if select_channels {
                self.current_channel_names = new_channel_names;
            } else {
                self.current_channel_names = saved_channel_names;
            }
        }
    }

    pub(crate) fn load_config(&mut self, custom_path: &str) {
        let mut timer = PrecisionTimer::new();
        timer.start();
        {
            let mut configs = self.configs.lock().unwrap();
            configs.previous = None;
        }

        let device_channel_count = if self.capture {
            self.settings.input_channel_count
        } else {
            self.settings.output_channel_count
        };
        let mut channel_mask = self.settings.channel_mask;
        if channel_mask == 0 {
            channel_mask = channel_helper::get_default_channel_mask(device_channel_count as i32);
        }

        let max_channels = self.settings.real_channel_count.max(self.settings.output_channel_count);
        self.all_channel_names = channel_helper::get_channel_names(max_channels as i32, channel_mask);
        self.current_channel_names = self.all_channel_names.clone();
        self.last_channel_names.clear();
        self.last_new_channel_names.clear();
        self.last_in_place = false;
        self.watch_registry_keys.clear();
        self.parser.clear_var();

        factories::start_of_configuration(self);

        if custom_path.is_empty() {
            let path = format!("{}\\config.txt", self.config_path);
            self.load_config_file(&path);
        } else {
            self.load_config_file(custom_path);
        }

        factories::end_of_configuration(self);

        let filter_infos = std::mem::take(&mut self.filter_infos);
        let config = FilterConfiguration::new(
            self.settings.real_channel_count as usize,
            self.settings.output_channel_count as usize,
            self.all_channel_names.len(),
            self.settings.max_frame_count as usize,
            filter_infos,
        );

        {
            let mut configs = self.configs.lock().unwrap();
            if configs.current.is_none() {
                configs.current = Some(config);
            } else {
                configs.next = Some(config);
            }
        }

        let load_time = timer.stop();
        eapo_common::trace_log!("Finished loading configuration after {} milliseconds", load_time * 1000.0);
    }

    pub(crate) fn load_config_file(&mut self, path: &str) {
        eapo_common::trace_log!("Loading configuration from {}", path);

        let data = loop {
            match std::fs::read(path) {
                Ok(d) => break d,
                Err(e) => {
                    if e.raw_os_error() == Some(32) {
                        // ERROR_SHARING_VIOLATION: file is being written
                        std::thread::sleep(std::time::Duration::from_millis(1));
                        continue;
                    }
                    eapo_common::error_log!(
                        "Error while reading configuration file {}: {}",
                        path,
                        e
                    );
                    return;
                }
            }
        };

        let saved_channel_names = self.current_channel_names.clone();
        factories::start_of_file(self, path);

        let mut start = 0usize;
        for index in 0..=data.len() {
            if index == data.len() || data[index] == b'\n' {
                let mut line_bytes = &data[start..index];
                if line_bytes.last() == Some(&b'\r') {
                    line_bytes = &line_bytes[..line_bytes.len() - 1];
                }
                let mut line = eapo_common::string_helper::utf8_to_wstring(line_bytes);
                if line.contains('\u{FFFD}') {
                    line = eapo_common::string_helper::ansi_to_wstring(line_bytes);
                }
                if let Some(pos) = line.find(':') {
                    let key = eapo_common::string_helper::trim(&line[..pos]);
                    let value = line[pos + 1..].to_string();
                    if !key.is_empty() {
                        factories::dispatch_line(self, path, &key, &value);
                    }
                }
                start = index + 1;
            }
        }

        factories::end_of_file(self, path);
        self.current_channel_names = saved_channel_names;
    }
}

#[derive(Clone, Copy)]
struct SendHandle(HANDLE);
unsafe impl Send for SendHandle {}

pub struct FilterEngine {
    settings: AudioSettings,
    configs: Arc<Mutex<ConfigState>>,
    loader: Arc<Mutex<Loader>>,
    load_semaphore: SendHandle,
    shutdown_event: SendHandle,
    notification_thread: Option<JoinHandle<()>>,
    input_buf_1d: Vec<f64>,
    output_buf_1d: Vec<f64>,
    input_buf_2d: Vec<Vec<f64>>,
    output_buf_2d: Vec<Vec<f64>>,
    allocated_frame_count: usize,
    transition_length: u32,
}

unsafe impl Send for FilterEngine {}

impl FilterEngine {
    pub fn new() -> Self {
        let configs = Arc::new(Mutex::new(ConfigState {
            current: None,
            next: None,
            previous: None,
            transition_counter: 0,
        }));
        let loader = Arc::new(Mutex::new(Loader::new(configs.clone())));
        let load_semaphore = unsafe { CreateSemaphoreW(None, 1, 1, PCWSTR::null()) }
            .expect("failed to create load semaphore");
        let shutdown_event =
            unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.expect("failed to create event");
        Self {
            settings: AudioSettings::default(),
            configs,
            loader,
            load_semaphore: SendHandle(load_semaphore),
            shutdown_event: SendHandle(shutdown_event),
            notification_thread: None,
            input_buf_1d: Vec::new(),
            output_buf_1d: Vec::new(),
            input_buf_2d: Vec::new(),
            output_buf_2d: Vec::new(),
            allocated_frame_count: 0,
            transition_length: 0,
        }
    }

    pub fn set_pre_mix(&mut self, pre_mix: bool) {
        self.loader.lock().unwrap().pre_mix = pre_mix;
    }

    pub fn set_device_info(
        &mut self,
        capture: bool,
        post_mix_installed: bool,
        device_name: &str,
        connection_name: &str,
        device_guid: &str,
        device_string: &str,
    ) {
        let mut loader = self.loader.lock().unwrap();
        loader.capture = capture;
        loader.post_mix_installed = post_mix_installed;
        loader.device_name = device_name.to_string();
        loader.connection_name = connection_name.to_string();
        loader.device_guid = device_guid.to_string();
        loader.device_string = device_string.to_string();
    }

    pub fn initialize(
        &mut self,
        sample_rate: f32,
        input_channel_count: u32,
        real_channel_count: u32,
        output_channel_count: u32,
        channel_mask: u32,
        max_frame_count: u32,
        custom_path: &str,
    ) {
        {
            let mut configs = self.configs.lock().unwrap();
            configs.current = None;
            configs.next = None;
            configs.previous = None;
            configs.transition_counter = 0;
        }

        self.settings = AudioSettings {
            sample_rate,
            input_channel_count,
            real_channel_count,
            output_channel_count,
            channel_mask,
            max_frame_count,
        };
        self.transition_length = (sample_rate / 100.0) as u32;
        self.resize_buffers(max_frame_count as usize);

        let device_channel_count = if self.loader.lock().unwrap().capture {
            input_channel_count
        } else {
            output_channel_count
        };
        let mut mask = channel_mask;
        if mask == 0 {
            mask = channel_helper::get_default_channel_mask(device_channel_count as i32);
        }
        self.settings.channel_mask = mask;

        let channel_names = channel_helper::get_channel_names(device_channel_count as i32, mask);
        eapo_common::trace_log!(
            "{} channels for this device: {}",
            device_channel_count,
            channel_names.join(" ")
        );

        let config_path = match registry_helper::read_value(APP_REGPATH, "ConfigPath") {
            Ok(v) => v,
            Err(e) => {
                if custom_path.is_empty() {
                    eapo_common::error_log!("Can't read config path because of: {}", e);
                    return;
                }
                // The editor's analysis preview passes an explicit path; keep
                // working when EqualizerAPO is not registered (yet).
                eapo_common::trace_log!(
                    "Can't read config path because of: {}. Using the explicitly requested file.",
                    e
                );
                String::new()
            }
        };

        {
            let mut loader = self.loader.lock().unwrap();
            loader.settings = AudioSettings {
                sample_rate,
                input_channel_count,
                real_channel_count,
                output_channel_count,
                channel_mask: self.settings.channel_mask,
                max_frame_count,
            };
            loader.config_path = config_path.clone();
            loader.transition_length = (sample_rate / 100.0) as u32;
            loader.parser.clear_const();
            loader.parser.clear_var();
            loader.parser.enable_auto_create_var(true);
            factories::initialize_factories(&mut loader);
        }

        if !config_path.is_empty() || !custom_path.is_empty() {
            self.load_config(custom_path);

            if !config_path.is_empty() && self.notification_thread.is_none() && custom_path.is_empty() {
                let loader = self.loader.clone();
                let semaphore = self.load_semaphore.0 .0 as usize;
                let shutdown = self.shutdown_event.0 .0 as usize;
                let path = config_path.clone();
                self.notification_thread = Some(std::thread::spawn(move || {
                    notification_thread(loader, semaphore, shutdown, path);
                }));
                eapo_common::trace_log!(
                    "Successfully created directory change notification thread for {} and its subtree",
                    config_path
                );
            }
        }
    }

    pub fn load_config(&mut self, custom_path: &str) {
        self.loader.lock().unwrap().load_config(custom_path);
    }

    pub fn load_config_file(&mut self, path: &str) {
        self.loader.lock().unwrap().load_config_file(path);
    }

    pub fn watch_registry_key(&mut self, key: &str) {
        self.loader.lock().unwrap().watch_registry_key(key);
    }

    fn resize_buffers(&mut self, frame_count: usize) {
        if self.allocated_frame_count < frame_count
            || self.input_buf_2d.len() != self.settings.input_channel_count as usize
            || self.output_buf_2d.len() != self.settings.output_channel_count as usize
        {
            eapo_common::trace_log!(
                "Reallocating internal double-precision buffers for {} frames and {}/{} channels.",
                frame_count,
                self.settings.input_channel_count,
                self.settings.output_channel_count
            );
            self.allocated_frame_count = frame_count;
            self.input_buf_1d
                .resize(self.settings.input_channel_count as usize * frame_count, 0.0);
            self.output_buf_1d
                .resize(self.settings.output_channel_count as usize * frame_count, 0.0);
            self.input_buf_2d = (0..self.settings.input_channel_count)
                .map(|_| vec![0.0; frame_count])
                .collect();
            self.output_buf_2d = (0..self.settings.output_channel_count)
                .map(|_| vec![0.0; frame_count])
                .collect();
        }
    }

    fn transition_finished(&self) {
        unsafe {
            let _ = ReleaseSemaphore(self.load_semaphore.0, 1, None);
        }
    }

    /// Interleaved f32 processing, matching the original overload.
    ///
    /// # Safety
    /// Input has `input_channel_count * frame_count` floats, output has
    /// `output_channel_count * frame_count` floats.
    pub unsafe fn process_f32_interleaved(
        &mut self,
        output: *mut f32,
        input: *const f32,
        frame_count: u32,
    ) {
        let configs = self.configs.clone();
        let mut cfg = configs.lock().unwrap();
        if cfg.current.as_ref().map(|c| c.is_empty()).unwrap_or(true) && cfg.next.is_none() {
            if self.settings.real_channel_count == self.settings.output_channel_count
                && output as *const f32 != input
            {
                std::ptr::copy_nonoverlapping(
                    input,
                    output,
                    self.settings.output_channel_count as usize * frame_count as usize,
                );
            }
            return;
        }
        {
            let ConfigState {
                current,
                next,
                transition_counter,
                ..
            } = &mut *cfg;
            let Some(current) = current.as_mut() else {
                return;
            };
            self.resize_buffers(frame_count as usize);
            let input_samples = self.settings.input_channel_count as usize * frame_count as usize;
            let src = std::slice::from_raw_parts(input, input_samples);
            for i in 0..input_samples {
                self.input_buf_1d[i] = src[i] as f64;
            }
            current.read(self.input_buf_1d.as_ptr(), frame_count as usize);
            current.process(frame_count as usize);

            if let Some(next) = next.as_mut() {
                next.read(self.input_buf_1d.as_ptr(), frame_count as usize);
                next.process(frame_count as usize);
                *transition_counter = current.do_transition(
                    next,
                    frame_count as usize,
                    *transition_counter,
                    self.transition_length,
                );
            }

            current.write(self.output_buf_1d.as_mut_ptr(), frame_count as usize);
            let output_samples = self.settings.output_channel_count as usize * frame_count as usize;
            let dst = std::slice::from_raw_parts_mut(output, output_samples);
            for i in 0..output_samples {
                dst[i] = self.output_buf_1d[i] as f32;
            }
        }

        self.finish_transition_if_done(&mut cfg);
    }

    /// Non-interleaved f32 processing.
    ///
    /// # Safety
    /// Each used channel pointer must have room for `frame_count` floats.
    pub unsafe fn process_f32(
        &mut self,
        output: *mut *mut f32,
        input: *mut *mut f32,
        frame_count: u32,
    ) {
        let configs = self.configs.clone();
        let mut cfg = configs.lock().unwrap();
        if cfg.current.as_ref().map(|c| c.is_empty()).unwrap_or(true) && cfg.next.is_none() {
            if self.settings.real_channel_count == self.settings.output_channel_count
                && output != input
            {
                for c in 0..self.settings.real_channel_count as usize {
                    std::ptr::copy_nonoverlapping(
                        *input.add(c),
                        *output.add(c),
                        frame_count as usize,
                    );
                }
            }
            return;
        }
        {
            let ConfigState {
                current,
                next,
                transition_counter,
                ..
            } = &mut *cfg;
            let Some(current) = current.as_mut() else {
                return;
            };
            self.resize_buffers(frame_count as usize);
            for c in 0..self.settings.input_channel_count as usize {
                let src = std::slice::from_raw_parts(*input.add(c), frame_count as usize);
                for i in 0..frame_count as usize {
                    self.input_buf_2d[c][i] = src[i] as f64;
                }
            }
            let input_ptrs: Vec<*const f64> = self
                .input_buf_2d
                .iter()
                .map(|buffer| buffer.as_ptr())
                .collect();
            current.read_channels(&input_ptrs, frame_count as usize);
            current.process(frame_count as usize);

            if let Some(next) = next.as_mut() {
                next.read_channels(&input_ptrs, frame_count as usize);
                next.process(frame_count as usize);
                *transition_counter = current.do_transition(
                    next,
                    frame_count as usize,
                    *transition_counter,
                    self.transition_length,
                );
            }

            let mut output_ptrs: Vec<*mut f64> = self
                .output_buf_2d
                .iter_mut()
                .map(|buffer| buffer.as_mut_ptr())
                .collect();
            current.write_channels(&mut output_ptrs, frame_count as usize);
            for c in 0..self.settings.output_channel_count as usize {
                let dst = std::slice::from_raw_parts_mut(*output.add(c), frame_count as usize);
                for i in 0..frame_count as usize {
                    dst[i] = self.output_buf_2d[c][i] as f32;
                }
            }
        }

        self.finish_transition_if_done(&mut cfg);
    }

    /// Interleaved f64 processing without conversion.
    ///
    /// # Safety
    /// Input has `input_channel_count * frame_count` doubles, output has
    /// `output_channel_count * frame_count` doubles.
    pub unsafe fn process_f64_interleaved(
        &mut self,
        output: *mut f64,
        input: *const f64,
        frame_count: u32,
    ) {
        let configs = self.configs.clone();
        let mut cfg = configs.lock().unwrap();
        if cfg.current.as_ref().map(|c| c.is_empty()).unwrap_or(true) && cfg.next.is_none() {
            if self.settings.real_channel_count == self.settings.output_channel_count
                && output as *const f64 != input
            {
                std::ptr::copy_nonoverlapping(
                    input,
                    output,
                    self.settings.output_channel_count as usize * frame_count as usize,
                );
            }
            return;
        }
        {
            let ConfigState {
                current,
                next,
                transition_counter,
                ..
            } = &mut *cfg;
            let Some(current) = current.as_mut() else {
                return;
            };
            current.read(input, frame_count as usize);
            current.process(frame_count as usize);

            if let Some(next) = next.as_mut() {
                next.read(input, frame_count as usize);
                next.process(frame_count as usize);
                *transition_counter = current.do_transition(
                    next,
                    frame_count as usize,
                    *transition_counter,
                    self.transition_length,
                );
            }

            current.write(output, frame_count as usize);
        }

        self.finish_transition_if_done(&mut cfg);
    }

    /// Non-interleaved f64 processing.
    ///
    /// # Safety
    /// Each used channel pointer must have room for `frame_count` doubles.
    pub unsafe fn process_f64(
        &mut self,
        output: *mut *mut f64,
        input: *mut *mut f64,
        frame_count: u32,
    ) {
        let configs = self.configs.clone();
        let mut cfg = configs.lock().unwrap();
        if cfg.current.as_ref().map(|c| c.is_empty()).unwrap_or(true) && cfg.next.is_none() {
            if self.settings.real_channel_count == self.settings.output_channel_count
                && output != input
            {
                for c in 0..self.settings.real_channel_count as usize {
                    std::ptr::copy_nonoverlapping(
                        *input.add(c),
                        *output.add(c),
                        frame_count as usize,
                    );
                }
            }
            return;
        }
        {
            let ConfigState {
                current,
                next,
                transition_counter,
                ..
            } = &mut *cfg;
            let Some(current) = current.as_mut() else {
                return;
            };
            let input_ptrs: Vec<*const f64> = (0..self.settings.input_channel_count as usize)
                .map(|c| *input.add(c) as *const f64)
                .collect();
            current.read_channels(&input_ptrs, frame_count as usize);
            current.process(frame_count as usize);

            if let Some(next) = next.as_mut() {
                next.read_channels(&input_ptrs, frame_count as usize);
                next.process(frame_count as usize);
                *transition_counter = current.do_transition(
                    next,
                    frame_count as usize,
                    *transition_counter,
                    self.transition_length,
                );
            }

            let mut output_ptrs: Vec<*mut f64> = (0..self.settings.output_channel_count as usize)
                .map(|c| *output.add(c))
                .collect();
            current.write_channels(&mut output_ptrs, frame_count as usize);
        }

        self.finish_transition_if_done(&mut cfg);
    }

    fn finish_transition_if_done(&self, cfg: &mut ConfigState) {
        if cfg.next.is_some() && cfg.transition_counter >= self.transition_length {
            cfg.previous = cfg.current.take();
            cfg.current = cfg.next.take();
            cfg.transition_counter = 0;
            self.transition_finished();
        }
    }

    pub fn is_pre_mix(&self) -> bool {
        self.loader.lock().unwrap().pre_mix
    }

    pub fn is_capture(&self) -> bool {
        self.loader.lock().unwrap().capture
    }

    pub fn is_post_mix_installed(&self) -> bool {
        self.loader.lock().unwrap().post_mix_installed
    }

    pub fn get_device_name(&self) -> String {
        self.loader.lock().unwrap().device_name.clone()
    }

    pub fn get_connection_name(&self) -> String {
        self.loader.lock().unwrap().connection_name.clone()
    }

    pub fn get_device_guid(&self) -> String {
        self.loader.lock().unwrap().device_guid.clone()
    }

    pub fn get_device_string(&self) -> String {
        self.loader.lock().unwrap().device_string.clone()
    }

    pub fn get_input_channel_count(&self) -> u32 {
        self.settings.input_channel_count
    }

    pub fn get_real_channel_count(&self) -> u32 {
        self.settings.real_channel_count
    }

    pub fn get_output_channel_count(&self) -> u32 {
        self.settings.output_channel_count
    }

    pub fn get_channel_mask(&self) -> u32 {
        self.settings.channel_mask
    }

    pub fn get_sample_rate(&self) -> f32 {
        self.settings.sample_rate
    }

    pub fn get_max_frame_count(&self) -> u32 {
        self.settings.max_frame_count
    }

    /// Returns the number of active filters in the current configuration.
    pub fn current_filter_count(&self) -> usize {
        self.configs
            .lock()
            .unwrap()
            .current
            .as_ref()
            .map(|c| c.filter_count())
            .unwrap_or(0)
    }
}

impl Default for FilterEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for FilterEngine {
    fn drop(&mut self) {
        unsafe {
            let _ = SetEvent(self.shutdown_event.0);
        }
        if let Some(thread) = self.notification_thread.take() {
            let _ = thread.join();
        }
        unsafe {
            let _ = CloseHandle(self.shutdown_event.0);
            let _ = CloseHandle(self.load_semaphore.0);
        }
    }
}

fn notification_thread(loader: Arc<Mutex<Loader>>, semaphore: usize, shutdown: usize, path: String) {
    let semaphore = HANDLE(semaphore as *mut std::ffi::c_void);
    let shutdown = HANDLE(shutdown as *mut std::ffi::c_void);

    let wide: Vec<u16> = path.encode_utf16().chain(std::iter::once(0)).collect();
    let notification = unsafe {
        FindFirstChangeNotificationW(
            PCWSTR(wide.as_ptr()),
            true,
            FILE_NOTIFY_CHANGE_FILE_NAME | FILE_NOTIFY_CHANGE_LAST_WRITE,
        )
    }
    .ok();
    let registry_event = unsafe { CreateEventW(None, true, false, PCWSTR::null()) }.ok();

    loop {
        let keys: Vec<String> = loader
            .lock()
            .unwrap()
            .watch_registry_keys
            .iter()
            .cloned()
            .collect();
        let mut key_handles = Vec::new();
        if let Some(event) = registry_event {
            for key in &keys {
                if let Some(handle) = open_key(key) {
                    unsafe {
                        let _ = RegNotifyChangeKeyValue(
                            handle,
                            false,
                            REG_NOTIFY_CHANGE_LAST_SET,
                            Some(event),
                            true,
                        );
                    }
                    key_handles.push(handle);
                }
            }
        }

        let mut handles = vec![shutdown];
        let file_index = notification.map(|handle| {
            handles.push(handle);
            handles.len() - 1
        });
        let registry_index = registry_event.map(|event| {
            handles.push(event);
            handles.len() - 1
        });
        let which = unsafe { WaitForMultipleObjects(&handles, false, INFINITE) };
        for handle in key_handles {
            unsafe {
                let _ = RegCloseKey(handle);
            }
        }

        if which.0 == WAIT_OBJECT_0.0 {
            break;
        }

        if file_index == Some(which.0 as usize) {
            if let Some(notification) = notification {
                unsafe {
                    let _ = FindNextChangeNotification(notification);
                    // debounce: wait for a second event within 10 ms
                    let _ = WaitForSingleObject(notification, 10);
                }
            }
        }
        let _ = registry_index;

        let wait_handles = [shutdown, semaphore];
        let which2 = unsafe { WaitForMultipleObjects(&wait_handles, false, INFINITE) };
        if which2.0 == WAIT_OBJECT_0.0 {
            break;
        }

        {
            let mut loader = loader.lock().unwrap();
            loader.load_config("");
        }

        unsafe {
            if let Some(notification) = notification {
                let _ = FindNextChangeNotification(notification);
            }
            if let Some(event) = registry_event {
                let _ = ResetEvent(event);
            }
        }
    }

    unsafe {
        if let Some(notification) = notification {
            let _ = FindCloseChangeNotification(notification);
        }
        if let Some(event) = registry_event {
            let _ = CloseHandle(event);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn test_loader() -> Loader {
        let configs = Arc::new(Mutex::new(ConfigState {
            current: None,
            next: None,
            previous: None,
            transition_counter: 0,
        }));
        let mut loader = Loader::new(configs);
        loader.settings = AudioSettings {
            sample_rate: 48000.0,
            input_channel_count: 2,
            real_channel_count: 2,
            output_channel_count: 2,
            channel_mask: 3,
            max_frame_count: 1024,
        };
        loader.capture = false;
        loader.pre_mix = false;
        loader.post_mix_installed = true;
        loader.device_name = "Test Device".into();
        loader.connection_name = "Test Connection".into();
        loader.device_guid = "{00000000-0000-0000-0000-000000000000}".into();
        loader.device_string = "Test Device (Test Connection) {00000000-0000-0000-0000-000000000000}".into();
        loader.all_channel_names = channel_helper::get_channel_names(2, 3);
        loader.current_channel_names = loader.all_channel_names.clone();
        factories::start_of_configuration(&mut loader);
        factories::initialize_factories(&mut loader);
        loader
    }

    fn temp_config(name: &str, contents: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "eapo_engine_test_{}_{}",
            std::process::id(),
            name
        ));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.txt");
        let mut file = std::fs::File::create(&path).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
        path
    }

    #[test]
    fn parses_all_basic_filters_and_if() {
        let path = temp_config(
            "basic",
            "Preamp: -6 dB\n\
             Filter: ON PK Fc 1000 Hz Gain 5 dB Q 1\n\
             GraphicEQ: 20 0; 1000 3\n\
             If: sampleRate == 48000\n\
             Delay: 10 ms\n\
             Else:\n\
             Delay: 20 ms\n\
             EndIf:\n",
        );
        let mut loader = test_loader();
        loader.load_config_file(path.to_str().unwrap());
        assert_eq!(loader.filter_infos.len(), 4);
        assert_eq!(loader.filter_infos[0].in_channels, vec![0, 1]);
    }

    #[test]
    fn if_constants_survive_eval() {
        let path = temp_config(
            "constants",
            "If: inputChannelCount == 2\n\
             Preamp: -1 dB\n\
             Else:\n\
             Preamp: -2 dB\n\
             EndIf:\n",
        );
        let mut loader = test_loader();
        loader.load_config_file(path.to_str().unwrap());
        assert_eq!(loader.filter_infos.len(), 1);
    }

    #[test]
    fn eval_assignments_and_inline_expressions() {
        let path = temp_config(
            "inline",
            "Eval: a = 2\n\
             Eval: b = a * 3\n\
             Preamp: `b` dB\n\
             Filter: ON PK Fc 100 Hz Gain `a + b` dB Q 1\n",
        );
        let mut loader = test_loader();
        loader.load_config_file(path.to_str().unwrap());
        assert_eq!(loader.filter_infos.len(), 2);
    }

    #[test]
    fn channel_selection_updates_routing() {
        let path = temp_config(
            "channel",
            "Channel: L\n\
             Preamp: -6 dB\n",
        );
        let mut loader = test_loader();
        loader.load_config_file(path.to_str().unwrap());
        assert_eq!(loader.filter_infos.len(), 2);
        let preamp = &loader.filter_infos[1];
        assert_eq!(preamp.in_channels, vec![0]);
        assert!(preamp.out_channels.is_empty());
    }

    #[test]
    fn device_filter_skips_non_matching_lines() {
        let path = temp_config(
            "device",
            "Device: test device\n\
             Preamp: -6 dB\n\
             Device: doesnotexist\n\
             Delay: 5 ms\n\
             Device: all\n\
             Delay: 6 ms\n",
        );
        let mut loader = test_loader();
        loader.load_config_file(path.to_str().unwrap());
        assert_eq!(loader.filter_infos.len(), 2);
    }

    #[test]
    fn include_recurses_with_relative_paths() {
        let dir = std::env::temp_dir().join(format!("eapo_engine_test_{}_include", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let child = dir.join("child.txt");
        std::fs::write(&child, "Preamp: -3 dB\n").unwrap();
        let main = dir.join("config.txt");
        std::fs::write(&main, "Preamp: -1 dB\nInclude: child.txt\n").unwrap();
        let mut loader = test_loader();
        loader.load_config_file(main.to_str().unwrap());
        assert_eq!(loader.filter_infos.len(), 2);
    }

    #[test]
    fn registry_functions_and_watch() {
        let key = r"HKEY_CURRENT_USER\Software\EqualizerAPO\RustPortEngineTest";
        registry_helper::create_key(key).unwrap();
        registry_helper::write_value(key, "StrValue", "abc").unwrap();
        registry_helper::write_dword_value(key, "DwordValue", 5).unwrap();

        let path = temp_config(
            "registry",
            "If: readRegString(\"HKEY_CURRENT_USER\\\\Software\\\\EqualizerAPO\\\\RustPortEngineTest\", \"StrValue\") == \"abc\" && readRegDWORD(\"HKEY_CURRENT_USER\\\\Software\\\\EqualizerAPO\\\\RustPortEngineTest\", \"DwordValue\") == 5\n\
             Preamp: -2 dB\n\
             EndIf:\n",
        );
        let mut loader = test_loader();
        loader.load_config_file(path.to_str().unwrap());
        assert_eq!(loader.filter_infos.len(), 1);
        assert!(loader.watch_registry_keys.contains(key));

        let _ = registry_helper::delete_key(key);
    }

    #[test]
    fn stage_and_stage_blocks() {
        let path = temp_config(
            "stage",
            "Stage: pre-mix\n\
             Preamp: -9 dB\n\
             Stage: post-mix\n\
             Delay: 3 ms\n",
        );
        let mut loader = test_loader();
        loader.load_config_file(path.to_str().unwrap());
        // Engine is in post-mix stage, so only the Delay is active.
        assert_eq!(loader.filter_infos.len(), 1);
    }
}

fn open_key(key: &str) -> Option<HKEY> {
    let (root, subkey) = if let Some(rest) = key.strip_prefix("HKEY_LOCAL_MACHINE\\") {
        (HKEY_LOCAL_MACHINE, rest)
    } else if let Some(rest) = key.strip_prefix("HKEY_CURRENT_USER\\") {
        (HKEY_CURRENT_USER, rest)
    } else if let Some(rest) = key.strip_prefix("HKEY_CLASSES_ROOT\\") {
        (HKEY_CLASSES_ROOT, rest)
    } else if let Some(rest) = key.strip_prefix("HKEY_USERS\\") {
        (HKEY_USERS, rest)
    } else if let Some(rest) = key.strip_prefix("HKEY_CURRENT_CONFIG\\") {
        (HKEY_CURRENT_CONFIG, rest)
    } else {
        return None;
    };
    let wide: Vec<u16> = subkey.encode_utf16().chain(std::iter::once(0)).collect();
    let mut result = HKEY::default();
    let status = unsafe {
        RegOpenKeyExW(
            root,
            PCWSTR(wide.as_ptr()),
            Some(0),
            REG_SAM_FLAGS(KEY_NOTIFY.0 | KEY_WOW64_64KEY.0),
            &mut result,
        )
    };
    if status.0 == 0 {
        Some(result)
    } else {
        None
    }
}
