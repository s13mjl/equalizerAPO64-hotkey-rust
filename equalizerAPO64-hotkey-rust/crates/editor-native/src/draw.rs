//! Shared GDI control and filter-row painting for the native editor.

use super::*;

pub(crate) unsafe fn draw_frame(hdc: HDC, left: i32, top: i32, right: i32, bottom: i32) {
    fill(hdc, left, top, right, top + 1, BORDER);
    fill(hdc, left, bottom - 1, right, bottom, BORDER);
    fill(hdc, left, top, left + 1, bottom, BORDER);
    fill(hdc, right - 1, top, right, bottom, BORDER);
}

pub(crate) unsafe fn draw_control(
    hdc: HDC,
    dpi: u32,
    rect: &RECT,
    text: &str,
    enabled: bool,
    checked: Option<bool>,
) {
    draw_control_state(hdc, dpi, rect, text, enabled, checked, true);
}

/// Like [`draw_control`] but without hover feedback, for fields that merely
/// display a value.
pub(crate) unsafe fn draw_control_flat(
    hdc: HDC,
    dpi: u32,
    rect: &RECT,
    text: &str,
    enabled: bool,
    checked: Option<bool>,
) {
    draw_control_state(hdc, dpi, rect, text, enabled, checked, false);
}

pub(crate) unsafe fn draw_control_state(
    hdc: HDC,
    dpi: u32,
    rect: &RECT,
    text: &str,
    enabled: bool,
    checked: Option<bool>,
    interactive: bool,
) {
    let background = if enabled && interactive && hovering(rect) {
        HOVER
    } else {
        WHITE
    };
    fill(hdc, rect.left, rect.top, rect.right, rect.bottom, background);
    draw_frame(hdc, rect.left, rect.top, rect.right, rect.bottom);
    let mut x = rect.left + scale(4, dpi);
    if let Some(checked) = checked {
        let size = scale(11, dpi);
        let y = rect.top + (rect.bottom - rect.top - size) / 2;
        fill(hdc, x, y, x + size, y + size, WHITE);
        draw_frame(hdc, x, y, x + size, y + size);
        if checked {
            let inset = scale(3, dpi).max(1);
            fill(hdc, x + inset, y + inset, x + size - inset, y + size - inset, BAR);
        }
        x += size + scale(5, dpi);
    }
    let saved = SaveDC(hdc);
    let _ = IntersectClipRect(hdc, rect.left + 1, rect.top, rect.right - 1, rect.bottom);
    draw_text(
        hdc,
        x,
        rect.top + scale(3, dpi),
        &wide(text),
        if enabled { TEXT } else { MUTED },
    );
    let _ = RestoreDC(hdc, saved);
}

pub(crate) unsafe fn paint_preamp_row(hdc: HDC, dpi: u32, y: i32, db: f64, enabled: bool) {
    let title = fit_text(&i18n::tr("Preamp"), 76);
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide(&title), if enabled { TEXT } else { MUTED });
    let mut track = preamp_slider_rect(dpi, y);
    track.right = scale(470, dpi);
    let track_bg = if hovering(&track) {
        HOVER
    } else {
        COLORREF(0x00E8_E8E8)
    };
    fill(hdc, track.left, track.top, track.right, track.bottom, track_bg);
    draw_frame(hdc, track.left, track.top, track.right, track.bottom);
    let t = ((db.clamp(-20.0, 20.0) + 20.0) / 40.0) as f32;
    let thumb_x = track.left + ((track.right - track.left) as f32 * t) as i32;
    fill(hdc, thumb_x - scale(2, dpi), track.top, thumb_x + scale(2, dpi), track.bottom, BAR);
    draw_control(
        hdc,
        dpi,
        &preamp_value_rect(dpi, y),
        &format!("{db:.1} dB"),
        enabled,
        None,
    );
}

pub(crate) unsafe fn paint_biquad_row(hdc: HDC, dpi: u32, y: i32, editor: &eapo_editor_core::model::BiquadEditor, enabled: bool) {
    let rects = biquad_field_rects(dpi, y);
    draw_control(hdc, dpi, &rects[0], &biquad_kind_display(editor.kind), enabled, None);
    draw_control(hdc, dpi, &rects[1], &format!("Fc {:.0} Hz", editor.freq), enabled, None);
    draw_control(hdc, dpi, &rects[2], &format!("{:.1} dB", editor.gain), enabled, None);
    draw_control(hdc, dpi, &rects[3], &format!("Q {:.2}", editor.q), enabled, None);
    draw_control(hdc, dpi, &rects[4], &i18n::tr(q_mode_label(editor.q_mode)), enabled, None);
}

pub(crate) unsafe fn paint_delay_row(hdc: HDC, dpi: u32, y: i32, delay: f64, is_ms: bool, enabled: bool) {
    let title = fit_text(&i18n::tr("Delay"), 76);
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide(&title), if enabled { TEXT } else { MUTED });
    let rects = delay_field_rects(dpi, y);
    draw_control(hdc, dpi, &rects[0], &format!("{delay:.2}"), enabled, None);
    let unit = if is_ms {
        "ms".to_string()
    } else {
        i18n::tr(" samples").trim().to_string()
    };
    draw_control(hdc, dpi, &rects[1], &unit, enabled, None);
}

pub(crate) unsafe fn paint_stage_row(hdc: HDC, dpi: u32, y: i32, pre_mix: bool, post_mix: bool, capture: bool, enabled: bool) {
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide("Stage"), if enabled { TEXT } else { MUTED });
    let rects = stage_field_rects(dpi, y);
    draw_control(hdc, dpi, &rects[0], &i18n::tr("Pre-mix"), enabled, Some(pre_mix));
    draw_control(hdc, dpi, &rects[1], &i18n::tr("Post-mix"), enabled, Some(post_mix));
    draw_control(hdc, dpi, &rects[2], &i18n::tr("Capture"), enabled, Some(capture));
}

/// Wheel-zoomable view of the GraphicEQ plot: 20 Hz..20 kHz and +20..-20 dB
/// at zoom 1, both axes scrollable once zoomed in. Zoom and scroll are kept
/// in the editor so they survive node edits.
#[derive(Clone, Copy)]
pub(crate) struct GeqView {
    zoom_x: f64,
    zoom_y: f64,
    scroll_x: f32,
    scroll_y: f32,
}

/// Content coordinates are `t_x = log10(f / 20 Hz) / 3` and
/// `t_y = (20 - dB) / 40`, so zoom 1 covers 20 Hz..20 kHz and +20..-20 dB.
/// The reachable world matches the C++ scene (1 Hz..30 kHz, +100..-100 dB)
/// so nodes beyond the default window can still be viewed.
pub(crate) const GEQ_WORLD_MIN_X: f32 = -0.433_676_7;
pub(crate) const GEQ_WORLD_MAX_X: f32 = 1.058_697_1;
pub(crate) const GEQ_WORLD_MIN_Y: f32 = -2.0;
pub(crate) const GEQ_WORLD_MAX_Y: f32 = 3.0;
pub(crate) const GEQ_WORLD_MIN_DB: f64 = -100.0;
pub(crate) const GEQ_WORLD_MAX_DB: f64 = 100.0;

/// Keeps a viewport of `span` inside `min..max`, centering it when it is
/// larger than the world.
pub(crate) fn clamp_axis(value: f32, min: f32, max: f32, span: f32) -> f32 {
    if span >= max - min {
        (min + max - span) / 2.0
    } else {
        value.clamp(min, max - span)
    }
}

impl GeqView {
    pub(crate) fn new(editor: &GraphicEqEditor) -> Self {
        let mut view = Self {
            zoom_x: editor.zoom_x.clamp(0.5, 30.0),
            zoom_y: editor.zoom_y.clamp(0.5, 30.0),
            scroll_x: 0.0,
            scroll_y: 0.0,
        };
        // The model default is the C++ scene-pixel scroll, which is far
        // outside these content units; treat it as "not set".
        let scroll_x = if editor.scroll.0.is_finite()
            && (GEQ_WORLD_MIN_X..=GEQ_WORLD_MAX_X).contains(&editor.scroll.0)
        {
            editor.scroll.0
        } else {
            0.0
        };
        let scroll_y = if editor.scroll.1.is_finite()
            && (GEQ_WORLD_MIN_Y..=GEQ_WORLD_MAX_Y).contains(&editor.scroll.1)
        {
            editor.scroll.1
        } else {
            0.0
        };
        view.set_scroll(scroll_x, scroll_y);
        view
    }

    pub(crate) fn set_scroll(&mut self, x: f32, y: f32) {
        self.scroll_x = clamp_axis(
            x,
            GEQ_WORLD_MIN_X,
            GEQ_WORLD_MAX_X,
            1.0 / self.zoom_x as f32,
        );
        self.scroll_y = clamp_axis(
            y,
            GEQ_WORLD_MIN_Y,
            GEQ_WORLD_MAX_Y,
            1.0 / self.zoom_y as f32,
        );
    }

    pub(crate) fn store(&self, editor: &mut GraphicEqEditor) {
        editor.zoom_x = self.zoom_x;
        editor.zoom_y = self.zoom_y;
        editor.scroll = (self.scroll_x, self.scroll_y);
    }

    pub(crate) fn x_of(&self, freq: f64, plot: &RECT) -> f32 {
        let width = (plot.right - plot.left).max(1) as f64;
        let t = (freq / 20.0).log10() / 3.0;
        (plot.left as f64 + (t - self.scroll_x as f64) * self.zoom_x * width) as f32
    }

    pub(crate) fn y_of(&self, db: f64, plot: &RECT) -> f32 {
        let height = (plot.bottom - plot.top).max(1) as f64;
        let t = (20.0 - db) / 40.0;
        (plot.top as f64 + (t - self.scroll_y as f64) * self.zoom_y * height) as f32
    }

    pub(crate) fn frequency_at(&self, x: f32, plot: &RECT) -> f64 {
        let width = (plot.right - plot.left).max(1) as f32;
        let local = ((x - plot.left as f32) / width).clamp(0.0, 1.0);
        let t = (self.scroll_x + local / self.zoom_x as f32)
            .clamp(GEQ_WORLD_MIN_X, GEQ_WORLD_MAX_X);
        20.0 * 1000.0f64.powf(t as f64)
    }

    pub(crate) fn db_at(&self, y: f32, plot: &RECT) -> f64 {
        let height = (plot.bottom - plot.top).max(1) as f32;
        let local = ((y - plot.top as f32) / height).clamp(0.0, 1.0);
        let t = (self.scroll_y + local / self.zoom_y as f32)
            .clamp(GEQ_WORLD_MIN_Y, GEQ_WORLD_MAX_Y);
        20.0 - t as f64 * 40.0
    }

    pub(crate) fn pan(&mut self, delta_x: f32, delta_y: f32, plot: &RECT) {
        let width = (plot.right - plot.left).max(1) as f32;
        let height = (plot.bottom - plot.top).max(1) as f32;
        let x = self.scroll_x - delta_x / (self.zoom_x as f32 * width);
        let y = self.scroll_y - delta_y / (self.zoom_y as f32 * height);
        self.set_scroll(x, y);
    }

    pub(crate) fn zoom_at(&mut self, delta: f32, local_x: f32, local_y: f32, plot: &RECT) {
        if delta == 0.0 {
            return;
        }
        let width = (plot.right - plot.left).max(1) as f32;
        let height = (plot.bottom - plot.top).max(1) as f32;
        let fx = (local_x / width).clamp(0.0, 1.0);
        let fy = (local_y / height).clamp(0.0, 1.0);
        let anchor_x = self.scroll_x + fx / self.zoom_x as f32;
        let anchor_y = self.scroll_y + fy / self.zoom_y as f32;
        let factor = 1.001f64.powf(delta as f64 * 2.4);
        self.zoom_x = (self.zoom_x * factor).clamp(0.5, 30.0);
        self.zoom_y = (self.zoom_y * factor).clamp(0.5, 30.0);
        self.set_scroll(
            anchor_x - fx / self.zoom_x as f32,
            anchor_y - fy / self.zoom_y as f32,
        );
    }
}

/// Picks a 1/2/5 dB grid step giving roughly 56 px spacing.
pub(crate) fn db_grid_step(range: f64, height: i32) -> f64 {
    if !range.is_finite() || range <= 0.0 || height <= 0 {
        return 10.0;
    }
    let target = range * 56.0 / height as f64;
    let base = 10f64.powf(target.log10().floor());
    if target >= 5.0 * base {
        5.0 * base
    } else if target >= 2.0 * base {
        2.0 * base
    } else {
        base
    }
}

pub(crate) fn geq_gain_at(nodes: &[FilterNode], freq: f64) -> f64 {
    if nodes.is_empty() {
        return 0.0;
    }
    if freq <= nodes[0].freq {
        return nodes[0].db_gain;
    }
    if freq >= nodes[nodes.len() - 1].freq {
        return nodes[nodes.len() - 1].db_gain;
    }
    for pair in nodes.windows(2) {
        if freq >= pair[0].freq && freq <= pair[1].freq {
            let t = (freq / pair[0].freq).ln() / (pair[1].freq / pair[0].freq).ln().max(1e-9);
            return pair[0].db_gain + t * (pair[1].db_gain - pair[0].db_gain);
        }
    }
    0.0
}

#[allow(clippy::too_many_arguments)]
pub(crate) unsafe fn paint_graphic_eq_row(
    hdc: HDC,
    offset: (i32, i32),
    dpi: u32,
    y: i32,
    row_height: i32,
    width: i32,
    editor: &eapo_editor_core::model::GraphicEqEditor,
    table_top: usize,
    enabled: bool,
) {
    let (buttons, labels) = geq_button_rects(dpi, y, editor.band_count);
    for (rect, label) in buttons.iter().zip(labels.iter()) {
        draw_control(hdc, dpi, rect, label, enabled, None);
    }

    let plot = geq_plot_rect(dpi, y, row_height, width);
    fill(hdc, plot.left, plot.top, plot.right, plot.bottom, WHITE);
    draw_frame(hdc, plot.left, plot.top, plot.right, plot.bottom);
    let view = GeqView::new(editor);
    let db_high = view.db_at(plot.top as f32, &plot);
    let db_low = view.db_at((plot.bottom - 1).max(plot.top) as f32, &plot);
    let step = db_grid_step(db_high - db_low, plot.bottom - plot.top);
    let mut db = (db_high / step).floor() * step;
    while db >= db_low - 1e-9 {
        let line_y = view.y_of(db, &plot).round() as i32;
        fill(hdc, plot.left, line_y, plot.right, line_y + 1, GRID);
        db -= step;
    }
    if db_low <= 0.0 && db_high >= 0.0 {
        let zero_line = view.y_of(0.0, &plot).round() as i32;
        fill(hdc, plot.left, zero_line, plot.right, zero_line + 1, BORDER);
    }
    for frequency in [
        1.0, 2.0, 5.0, 10.0, 20.0, 50.0, 100.0, 200.0, 500.0, 1000.0, 2000.0, 5000.0, 10000.0,
        20000.0, 30000.0,
    ] {
        let x = view.x_of(frequency, &plot).round() as i32;
        if x >= plot.left && x < plot.right {
            fill(hdc, x, plot.top, x + 1, plot.bottom, GRID);
        }
    }
    let zero_y = view.y_of(0.0, &plot);
    let mut curve: Vec<PointF> = Vec::with_capacity((plot.right - plot.left).max(0) as usize);
    let mut positive_areas: Vec<Vec<PointF>> = Vec::new();
    let mut positive: Vec<PointF> = Vec::new();
    for px in plot.left..plot.right {
        let frequency = view.frequency_at(px as f32 + 0.5, &plot);
        let db =
            geq_gain_at(&editor.nodes, frequency).clamp(GEQ_WORLD_MIN_DB, GEQ_WORLD_MAX_DB);
        let point = PointF {
            X: px as f32 + 0.5,
            Y: view.y_of(db, &plot),
        };
        curve.push(point);
        if db > 0.0 {
            positive.push(point);
        } else if !positive.is_empty() {
            if positive.len() >= 2 {
                positive_areas.push(std::mem::take(&mut positive));
            } else {
                positive.clear();
            }
        }
    }
    if positive.len() >= 2 {
        positive_areas.push(positive);
    }
    draw_aa_filled_areas(hdc, offset, &positive_areas, zero_y, plot, FILL_POS);
    draw_aa_runs(hdc, offset, &[(CURVE, curve)], plot);
    let half = scale(3, dpi);
    for (index, node) in editor.nodes.iter().enumerate() {
        let x = view.x_of(node.freq, &plot).round() as i32;
        let py = view
            .y_of(node.db_gain.clamp(GEQ_WORLD_MIN_DB, GEQ_WORLD_MAX_DB), &plot)
            .round() as i32;
        if x < plot.left - half
            || x > plot.right + half
            || py < plot.top - half
            || py > plot.bottom + half
        {
            continue;
        }
        let color = if editor.selected.contains(&index) { CURVE_POS } else { CURVE };
        fill(hdc, x - half, py - half, x + half, py + half, color);
    }

    // Node table (original Editor: Freq./Gain columns next to the plot).
    let table = geq_table_rect(dpi, y, row_height, width);
    fill(hdc, table.left, table.top, table.right, table.bottom, WHITE);
    draw_frame(hdc, table.left, table.top, table.right, table.bottom);
    let (first_row, row_h) = geq_table_metrics(dpi, &table);
    let split = geq_table_column_split(dpi, &table);
    let visible = geq_table_visible_rows(dpi, &table);
    let top = geq_table_top_clamped(table_top, editor.nodes.len(), visible);
    fill(hdc, table.left + 1, table.top + 1, table.right - 1, first_row - 1, CHROME_BG);
    fill(hdc, split, table.top + 1, split + 1, table.bottom - 1, BORDER);
    fill(hdc, table.left + 1, first_row - 1, table.right - 1, first_row, BORDER);
    draw_text(
        hdc,
        table.left + scale(4, dpi),
        table.top + scale(2, dpi),
        &wide(&i18n::tr("Freq.")),
        TEXT,
    );
    draw_text(
        hdc,
        split + scale(4, dpi),
        table.top + scale(2, dpi),
        &wide(&i18n::tr("Gain")),
        TEXT,
    );
    for (offset, index) in (top..editor.nodes.len()).take(visible).enumerate() {
        let cell_top = first_row + offset as i32 * row_h;
        let node = editor.nodes[index];
        let selected = editor.selected.contains(&index);
        for column in 0..2 {
            let (left, right) = if column == 0 {
                (table.left, split)
            } else {
                (split, table.right)
            };
            let cell = RECT {
                left,
                top: cell_top,
                right,
                bottom: cell_top + row_h,
            };
            let background = if selected {
                SELECTED
            } else if enabled && hovering(&cell) {
                HOVER
            } else {
                WHITE
            };
            fill(hdc, left, cell_top, right, cell_top + row_h, background);
            fill(hdc, left, cell_top + row_h - 1, right, cell_top + row_h, BORDER);
            let text = if column == 0 {
                format_geq_value(node.freq)
            } else {
                format_geq_value(node.db_gain)
            };
            draw_text(
                hdc,
                left + scale(4, dpi),
                cell_top + scale(3, dpi),
                &wide(&text),
                if enabled { TEXT } else { MUTED },
            );
        }
    }
    if visible > 0 && editor.nodes.len() > visible {
        let track_top = first_row;
        let track_bottom = table.bottom - 1;
        let track_height = (track_bottom - track_top).max(1);
        let thumb_height = ((track_height as f32 * visible as f32 / editor.nodes.len() as f32)
            as i32)
            .max(scale(12, dpi))
            .min(track_height);
        let max_top = editor.nodes.len() - visible;
        let thumb_top = track_top
            + ((track_height - thumb_height) as f32 * top as f32 / max_top.max(1) as f32) as i32;
        let bar_x = table.right - scale(5, dpi);
        fill(
            hdc,
            bar_x,
            track_top,
            bar_x + scale(3, dpi),
            track_bottom,
            COLORREF(0x00E8_E8E8),
        );
        fill(
            hdc,
            bar_x,
            thumb_top,
            bar_x + scale(3, dpi),
            thumb_top + thumb_height,
            BAR,
        );
    }
}

pub(crate) unsafe fn paint_path_row(
    hdc: HDC,
    dpi: u32,
    y: i32,
    title: &str,
    path: &str,
    button: &str,
    enabled: bool,
) {
    let title = fit_text(&i18n::tr(title), 76);
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide(&title), if enabled { TEXT } else { MUTED });
    let rects = path_row_rects(dpi, y);
    let shown = if path.is_empty() { "(none)" } else { path };
    draw_control_flat(hdc, dpi, &rects[0], &shorten(shown, 60), enabled, None);
    draw_control(hdc, dpi, &rects[1], &i18n::tr(button), enabled, None);
}

/// Returns the status line for a VST row plus a warning flag for unreadable
/// plugin files (the audio service runs as LOCAL SERVICE).
pub(crate) fn vst_status(vst: &eapo_editor_core::model::VstEditor, readable: Option<bool>) -> (String, bool) {
    if vst.path.is_empty() {
        return (i18n::tr("No file selected."), true);
    }
    let unreadable = readable == Some(false);
    let text = if vst.status.is_empty() {
        i18n::tr("VST plugin")
    } else {
        i18n::tr(&vst.status)
    };
    (text, unreadable)
}

pub(crate) unsafe fn paint_vst_row(
    hdc: HDC,
    dpi: u32,
    y: i32,
    vst: &eapo_editor_core::model::VstEditor,
    readable: Option<bool>,
    enabled: bool,
) {
    let rects = path_row_rects(dpi, y);
    let shown = if vst.path.is_empty() { "(none)" } else { &vst.path };
    let title = fit_text(&i18n::tr("VST plugin"), 76);
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide(&title), if enabled { TEXT } else { MUTED });
    draw_control(hdc, dpi, &rects[0], &shorten(shown, 60), enabled, None);
    draw_control(hdc, dpi, &rects[1], &i18n::tr("Select file"), enabled, None);
    let actions = vst_action_rects(dpi, y);
    let (status, warning) = vst_status(vst, readable);
    draw_text(
        hdc,
        scale(42, dpi),
        y + scale(35, dpi),
        &wide(&status),
        if warning { WARNING } else if enabled { TEXT } else { MUTED },
    );
    draw_control(hdc, dpi, &actions[0], &i18n::tr("Open panel"), enabled, None);
    draw_control(hdc, dpi, &actions[1], &i18n::tr("Options"), enabled, None);
}

pub(crate) unsafe fn paint_loudness_row(
    hdc: HDC,
    dpi: u32,
    y: i32,
    ref_level: i32,
    ref_offset: i32,
    att: f64,
    enabled: bool,
) {
    // There is no short "Loudness" translation in the original catalog and the
    // row title only has ~38 px before the first field, so keep it short.
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide("Loudness"), if enabled { TEXT } else { MUTED });
    let field = |short: &str, source: &str, value: String| {
        if i18n::language() == i18n::Language::English {
            format!("{short} {value}")
        } else {
            format!("{} {value}", i18n::tr(source))
        }
    };
    let rects = loudness_field_rects(dpi, y);
    draw_control(
        hdc,
        dpi,
        &rects[0],
        &field("Ref", "Reference level:", ref_level.to_string()),
        enabled,
        None,
    );
    draw_control(
        hdc,
        dpi,
        &rects[1],
        &field("Offset", "Reference offset:", ref_offset.to_string()),
        enabled,
        None,
    );
    draw_control(
        hdc,
        dpi,
        &rects[2],
        &field("Att", "Attenuation:", format!("{att:.2}")),
        enabled,
        None,
    );
    draw_control(hdc, dpi, &rects[3], &i18n::tr("Calibrate"), enabled, None);
}

pub(crate) unsafe fn paint_summary_row(
    hdc: HDC,
    dpi: u32,
    y: i32,
    title: &str,
    summary: &str,
    enabled: bool,
) {
    let title = fit_text(&i18n::tr(title), 76);
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide(&title), if enabled { TEXT } else { MUTED });
    draw_text(
        hdc,
        scale(120, dpi),
        y + scale(5, dpi),
        &wide(&shorten(summary, 70)),
        if enabled { TEXT } else { MUTED },
    );
    let button = change_button_rect(dpi, y);
    draw_control(hdc, dpi, &button, &i18n::tr("Change..."), enabled, None);
}

pub(crate) unsafe fn paint_copy_row(hdc: HDC, dpi: u32, y: i32, parameters: &str, enabled: bool) {
    let title = fit_text(&i18n::tr("Copy"), 76);
    draw_text(hdc, scale(42, dpi), y + scale(5, dpi), &wide(&title), if enabled { TEXT } else { MUTED });
    let mut line_y = y + scale(28, dpi);
    for part in parameters.split(';').take(3) {
        draw_text(
            hdc,
            scale(48, dpi),
            line_y,
            &wide(&shorten(part.trim(), 80)),
            if enabled { TEXT } else { MUTED },
        );
        line_y += scale(18, dpi);
    }
    let button = change_button_rect(dpi, y);
    draw_control(hdc, dpi, &button, &format!("{}...", i18n::tr("Edit")), enabled, None);
}
