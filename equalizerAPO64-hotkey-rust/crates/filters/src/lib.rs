//! Filter implementations ported 1:1 from the original C++ `filters/` directory.
//!
//! The [`IFilter`] trait mirrors `IFilter.h`. All processing is done on
//! non-interleaved `f64` buffers so results stay bit-compatible with the
//! double-precision upstream build.

pub mod audio_file;
pub mod biquad;
pub mod biquad_filter;
pub mod channel_filter;
pub mod convolution_filter;
pub mod copy_filter;
pub mod delay_filter;
pub mod graphic_eq_filter;
mod hybrid_conv;
pub mod iir_filter;
pub mod loudness_correction;
pub mod preamp_filter;
mod vst_abi;
pub mod vst_plugin;
pub mod vst_plugin_instance;
pub mod vst_plugin_library;

/// Mirrors `IFilter.h`.
///
/// `process` receives raw channel-pointer arrays exactly like the C++
/// interface: `output[i]` and `input[i]` are `frame_count` doubles and may
/// alias when the filter is in-place. Implementations must be real-time safe.
pub trait IFilter: Send {
    /// Request to get all channel names instead of the current selection.
    fn get_all_channels(&self) -> bool {
        false
    }

    /// Return false to request that output and input do not point to the same
    /// memory locations.
    fn get_in_place(&self) -> bool {
        true
    }

    /// Request that the channel names returned by `initialize` become the new
    /// selection.
    fn get_select_channels(&self) -> bool {
        false
    }

    /// Returns the channel names produced by this filter.
    fn initialize(
        &mut self,
        sample_rate: f32,
        max_frame_count: usize,
        channel_names: Vec<String>,
    ) -> Vec<String>;

    /// Processes `frame_count` samples.
    ///
    /// # Safety
    /// Every pointer in `output` and `input` that is used by the filter must
    /// point to at least `frame_count` valid `f64` values. Where the original
    /// C++ filter is in-place, output and input may alias.
    unsafe fn process(&mut self, output: &mut [*mut f64], input: &[*mut f64], frame_count: usize);
}

/// Helper used throughout the port to view one channel pointer as a slice.
///
/// # Safety
/// `ptr` must be valid for `len` doubles for the duration of the borrow.
#[inline]
pub unsafe fn channel<'a>(ptr: *mut f64, len: usize) -> &'a mut [f64] {
    std::slice::from_raw_parts_mut(ptr, len)
}

/// Helper used throughout the port to view one input channel as a slice.
///
/// # Safety
/// `ptr` must be valid for `len` doubles for the duration of the borrow and
/// must not be mutated while the slice lives.
#[inline]
pub unsafe fn channel_ref<'a>(ptr: *const f64, len: usize) -> &'a [f64] {
    std::slice::from_raw_parts(ptr, len)
}
