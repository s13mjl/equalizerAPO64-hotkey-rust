//! Port of `filters/loudnessCorrection/VolumeController.{h,cpp}` using the
//! Windows Core Audio endpoint volume COM interface.

use std::ptr;

use windows::core::HRESULT;
use windows::Win32::Foundation::S_OK;
use windows::Win32::Media::Audio::Endpoints::IAudioEndpointVolume;
use windows::Win32::Media::Audio::{eMultimedia, eRender, IMMDeviceEnumerator, MMDeviceEnumerator};
use windows::Win32::System::Com::{
    CoCreateInstance, CoInitializeEx, CoUninitialize, CLSCTX_INPROC_SERVER, COINIT_APARTMENTTHREADED,
};

pub struct VolumeController {
    endpoint_volume: Option<IAudioEndpointVolume>,
    min_vol: f32,
    max_vol: f32,
    uninitialize_com: bool,
}

impl VolumeController {
    pub fn new() -> Self {
        let mut controller = Self {
            endpoint_volume: None,
            min_vol: 0.0,
            max_vol: 0.0,
            uninitialize_com: false,
        };

        unsafe {
            let init_result = CoInitializeEx(None, COINIT_APARTMENTTHREADED);
            controller.uninitialize_com = init_result == S_OK;

            let enumerator: Result<IMMDeviceEnumerator, _> =
                CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_INPROC_SERVER);
            let Ok(enumerator) = enumerator else {
                return controller;
            };
            let Ok(device) = enumerator.GetDefaultAudioEndpoint(eRender, eMultimedia) else {
                return controller;
            };
            let Ok(endpoint) = device.Activate::<IAudioEndpointVolume>(CLSCTX_INPROC_SERVER, None)
            else {
                return controller;
            };

            let mut increment = 0.0f32;
            let _ = endpoint.GetVolumeRange(
                &mut controller.min_vol,
                &mut controller.max_vol,
                &mut increment,
            );
            controller.endpoint_volume = Some(endpoint);
        }

        controller
    }

    pub fn get_volume(&self, current_volume: &mut f64) -> HRESULT {
        let Some(endpoint) = &self.endpoint_volume else {
            return HRESULT(0x8000_4005u32 as i32);
        };
        unsafe {
            match endpoint.GetMasterVolumeLevel() {
                Ok(volume) => {
                    *current_volume = volume as f64;
                    S_OK
                }
                Err(error) => error.code(),
            }
        }
    }

    pub fn set_volume(&self, volume: f64) -> HRESULT {
        let Some(endpoint) = &self.endpoint_volume else {
            return HRESULT(0x8000_4005u32 as i32);
        };
        let volume = volume.min(self.max_vol as f64).max(self.min_vol as f64);
        unsafe {
            match endpoint.SetMasterVolumeLevel(volume as f32, ptr::null()) {
                Ok(()) => S_OK,
                Err(error) => error.code(),
            }
        }
    }
}

impl Default for VolumeController {
    fn default() -> Self {
        Self::new()
    }
}

impl Drop for VolumeController {
    fn drop(&mut self) {
        self.endpoint_volume = None;
        if self.uninitialize_com {
            unsafe { CoUninitialize() };
        }
    }
}
