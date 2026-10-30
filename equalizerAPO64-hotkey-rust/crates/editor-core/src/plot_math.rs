//! Plot view state shared by the analysis panel and the GraphicEQ editor.
//!
//! Port of `FrequencyPlotScene`/`FrequencyPlotView` state: logarithmic
//! frequency axis, linear dB axis, anchor-preserving zoom and panning.
//! UI-agnostic: positions are plain `f32` pairs.

pub const MAX_X: f64 = 1000.0;
pub const MAX_Y: f64 = 500.0;
pub const MIN_HZ: f64 = 1.0;
pub const MAX_HZ: f64 = 30000.0;
pub const MIN_DB: f64 = -100.0;
pub const MAX_DB: f64 = 100.0;

#[derive(Clone, Debug)]
pub struct PlotState {
    pub zoom_x: f64,
    pub zoom_y: f64,
    pub scroll_x: f32,
    pub scroll_y: f32,
}

impl Default for PlotState {
    fn default() -> Self {
        let mut state = Self {
            zoom_x: 1.0,
            zoom_y: 1.0,
            scroll_x: 0.0,
            scroll_y: 0.0,
        };
        state.default_scroll();
        state
    }
}

impl PlotState {
    pub fn scene_width(&self) -> f32 {
        (MAX_X * self.zoom_x) as f32
    }

    pub fn scene_height(&self) -> f32 {
        (MAX_Y * self.zoom_y) as f32
    }

    pub fn x_to_hz(&self, x: f64) -> f64 {
        (MAX_HZ / MIN_HZ).powf(x / self.zoom_x / MAX_X) * MIN_HZ
    }

    pub fn hz_to_x(&self, hz: f64) -> f64 {
        if hz < MIN_HZ || hz > MAX_HZ {
            return -1.0;
        }
        (hz / MIN_HZ).ln() / (MAX_HZ / MIN_HZ).ln() * MAX_X * self.zoom_x
    }

    pub fn y_to_db(&self, y: f64) -> f64 {
        MIN_DB + (1.0 - y / self.zoom_y / MAX_Y) * (MAX_DB - MIN_DB)
    }

    pub fn db_to_y(&self, db: f64) -> f64 {
        if db < MIN_DB || db > MAX_DB {
            return -1.0;
        }
        (1.0 - (db - MIN_DB) / (MAX_DB - MIN_DB)) * MAX_Y * self.zoom_y
    }

    /// Fits an explicit frequency range (e.g. 20 Hz..20 kHz) to the view
    /// width, so resizing the window keeps the scale instead of squeezing the
    /// audible range into an ever smaller fraction of the plot.
    pub fn fit_frequency_range(&mut self, min_hz: f64, max_hz: f64, view_width: f32) {
        if min_hz <= 0.0 || max_hz <= min_hz {
            return;
        }
        let fraction = (max_hz / min_hz).ln() / (MAX_HZ / MIN_HZ).ln();
        if fraction <= 0.0 {
            return;
        }
        self.zoom_x = (view_width.max(1.0) as f64 / (MAX_X * fraction)).clamp(0.001, 1000.0);
    }

    pub fn fit(&mut self, view_width: f32, view_height: f32) {
        self.zoom_x = (MAX_X as f32 / view_width.max(1.0)) as f64;
        self.zoom_y = (MAX_Y as f32 / view_height.max(1.0)) as f64;
        self.scroll_x = 0.0;
        self.scroll_y = 0.0;
    }

    fn clamp_scroll(&mut self, view_width: f32, view_height: f32) {
        self.scroll_x = self
            .scroll_x
            .clamp(0.0, (self.scene_width() - view_width).max(0.0));
        self.scroll_y = self
            .scroll_y
            .clamp(0.0, (self.scene_height() - view_height).max(0.0));
    }

    /// Zooms by a wheel delta around the given local position.
    pub fn zoom_at(&mut self, delta: f32, local_x: f32, local_y: f32, view_width: f32, view_height: f32) {
        if delta == 0.0 {
            return;
        }
        let scene_x = self.scroll_x + local_x;
        let scene_y = self.scroll_y + local_y;
        let hz = self.x_to_hz(scene_x as f64);
        let db = self.y_to_db(scene_y as f64);
        let factor = 1.001f64.powf(delta as f64 * 2.4);
        self.zoom_x = (self.zoom_x * factor).clamp(0.5, 30.0);
        self.zoom_y = (self.zoom_y * factor).clamp(0.5, 30.0);
        self.scroll_x = self.hz_to_x(hz) as f32 - local_x;
        self.scroll_y = self.db_to_y(db) as f32 - local_y;
        self.clamp_scroll(view_width, view_height);
    }

    pub fn pan(&mut self, delta_x: f32, delta_y: f32, view_width: f32, view_height: f32) {
        self.scroll_x -= delta_x;
        self.scroll_y -= delta_y;
        self.clamp_scroll(view_width, view_height);
    }

    /// Frequency under a plot-local position.
    pub fn freq_at(&self, local_x: f32) -> f64 {
        self.x_to_hz((self.scroll_x + local_x) as f64)
    }

    /// Level under a plot-local position.
    pub fn db_at(&self, local_y: f32) -> f64 {
        self.y_to_db((self.scroll_y + local_y) as f64)
    }

    /// Plot-local position for a frequency/level pair.
    pub fn pos_for(&self, hz: f64, db: f64) -> Option<(f32, f32)> {
        let x = self.hz_to_x(hz);
        let y = self.db_to_y(db);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        Some((x as f32 - self.scroll_x, y as f32 - self.scroll_y))
    }

    pub fn default_scroll(&mut self) {
        self.scroll_x = self.hz_to_x(20.0) as f32;
        self.scroll_y = self.db_to_y(22.0) as f32;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn frequency_axis_is_logarithmic() {
        let mut state = PlotState::default();
        state.fit(1000.0, 500.0);
        let twenty = state.hz_to_x(20.0);
        let twenty_k = state.hz_to_x(20000.0);
        assert!(twenty >= 0.0 && twenty_k > twenty);
        let ratio = (twenty_k - twenty) / state.scene_width() as f64;
        let expected = (1000.0f64).ln() / (MAX_HZ / MIN_HZ).ln();
        assert!((ratio - expected).abs() < 0.01);
        assert!((state.x_to_hz(twenty as f64) - 20.0).abs() < 0.01);
    }

    #[test]
    fn zoom_is_anchor_preserving() {
        let mut state = PlotState::default();
        state.fit(760.0, 300.0);
        state.default_scroll();
        let local_x = 300.0;
        let hz_before = state.freq_at(local_x);
        state.zoom_at(30.0, local_x, 150.0, 760.0, 300.0);
        let hz_after = state.freq_at(local_x);
        assert!((hz_before - hz_after).abs() / hz_before < 0.01);
    }
}
