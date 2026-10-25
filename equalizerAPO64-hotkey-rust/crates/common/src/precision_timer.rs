//! Port of `helpers/PrecisionTimer.h`.

use windows::Win32::System::Performance::{QueryPerformanceCounter, QueryPerformanceFrequency};

/// `QueryPerformanceCounter` based stopwatch (port of `PrecisionTimer`).
pub struct PrecisionTimer {
    freq: i64,
    start_count: i64,
}

impl PrecisionTimer {
    /// Queries the counter frequency once, like the C++ constructor.
    pub fn new() -> Self {
        let mut freq = 0i64;
        unsafe {
            let _ = QueryPerformanceFrequency(&mut freq);
        }
        Self {
            freq,
            start_count: 0,
        }
    }

    /// Starts (or restarts) the measurement.
    pub fn start(&mut self) {
        let mut count = 0i64;
        unsafe {
            let _ = QueryPerformanceCounter(&mut count);
        }
        self.start_count = count;
    }

    /// Seconds elapsed since `start()`.
    pub fn stop(&self) -> f64 {
        let mut count = 0i64;
        unsafe {
            let _ = QueryPerformanceCounter(&mut count);
        }
        (count - self.start_count) as f64 / self.freq as f64
    }
}

impl Default for PrecisionTimer {
    fn default() -> Self {
        Self::new()
    }
}
