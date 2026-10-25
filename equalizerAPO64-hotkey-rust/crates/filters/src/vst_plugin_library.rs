//! Port of `helpers/VSTPluginLibrary.{h,cpp}` on top of `libloading`.

use std::collections::HashMap;
use std::path::Path;
use std::sync::{Arc, Mutex, OnceLock, Weak};

use eapo_common::abstract_library::{
    get_file_architecture, FUNCTIONS_MISSING, LOADING_FAILED, WRONG_ARCHITECTURE,
};
use eapo_common::{registry_helper, trace_log, APP_REGPATH};

use crate::vst_abi::VstPluginMain;

struct LoadState {
    library: Option<libloading::Library>,
    vst_plugin_main: Option<VstPluginMain>,
}

pub struct VSTPluginLibrary {
    lib_path: String,
    state: Mutex<LoadState>,
}

impl VSTPluginLibrary {
    fn new(lib_path: String) -> Self {
        Self {
            lib_path,
            state: Mutex::new(LoadState {
                library: None,
                vst_plugin_main: None,
            }),
        }
    }

    pub fn get_instance(lib_path: &str) -> Arc<Self> {
        static INSTANCES: OnceLock<Mutex<HashMap<String, Weak<VSTPluginLibrary>>>> =
            OnceLock::new();
        let mut instances = INSTANCES
            .get_or_init(|| Mutex::new(HashMap::new()))
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if let Some(instance) = instances.get(lib_path).and_then(Weak::upgrade) {
            return instance;
        }

        let instance = Arc::new(Self::new(lib_path.to_string()));
        instances.insert(lib_path.to_string(), Arc::downgrade(&instance));
        instance
    }

    pub fn get_default_plugin_path() -> String {
        static DEFAULT_PLUGIN_PATH: OnceLock<String> = OnceLock::new();
        DEFAULT_PLUGIN_PATH
            .get_or_init(|| {
                let install_path =
                    registry_helper::read_value(APP_REGPATH, "InstallPath").unwrap_or_default();
                format!("{}\\VSTPlugins", install_path)
            })
            .clone()
    }

    pub fn lib_path(&self) -> &str {
        &self.lib_path
    }

    pub fn initialize(&self) -> i32 {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.library.is_some() {
            return 0;
        }

        if !Path::new(&self.lib_path).exists() {
            return eapo_common::abstract_library::FILE_NOT_FOUND;
        }

        let library = match unsafe { libloading::Library::new(&self.lib_path) } {
            Ok(library) => library,
            Err(_) => {
                let architecture = get_file_architecture(&self.lib_path);
                #[cfg(target_pointer_width = "64")]
                let expected = 0x8664u16;
                #[cfg(target_pointer_width = "32")]
                let expected = 0x014cu16;
                if architecture != 0 && architecture != expected {
                    return WRONG_ARCHITECTURE;
                }
                return LOADING_FAILED;
            }
        };

        let vst_plugin_main: VstPluginMain = match unsafe { library.get(b"VSTPluginMain\0") } {
            Ok(symbol) => *symbol,
            Err(_) => {
                return FUNCTIONS_MISSING;
            }
        };

        state.library = Some(library);
        state.vst_plugin_main = Some(vst_plugin_main);
        trace_log!("Loaded library {}", self.lib_path);
        1
    }

    pub(crate) fn vst_plugin_main(&self) -> Option<VstPluginMain> {
        self.state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .vst_plugin_main
    }
}

impl Drop for VSTPluginLibrary {
    fn drop(&mut self) {
        let mut state = self
            .state
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        if state.library.take().is_some() {
            trace_log!("Unloaded library {}", self.lib_path);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn missing_library_is_reported() {
        let library = VSTPluginLibrary::get_instance(r"C:\Does\Not\Exist\eapo-test-vst.dll");
        assert_eq!(library.initialize(), eapo_common::abstract_library::FILE_NOT_FOUND);
    }

    #[test]
    fn instances_are_shared_per_path() {
        let first = VSTPluginLibrary::get_instance(r"C:\Does\Not\Exist\shared.dll");
        let second = VSTPluginLibrary::get_instance(r"C:\Does\Not\Exist\shared.dll");
        assert!(Arc::ptr_eq(&first, &second));
    }
}
