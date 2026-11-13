//! Geometry and shared painting metrics for the native editor.

use super::*;

pub(crate) fn row_height(kind: FilterKind, dpi: u32) -> u32 {
    let px = match kind {
        FilterKind::GraphicEq | FilterKind::Copy => scale(220, dpi),
        FilterKind::Channel | FilterKind::Device => scale(72, dpi),
        FilterKind::Convolution => scale(56, dpi),
        FilterKind::Vst => scale(84, dpi),
        FilterKind::Loudness => scale(40, dpi),
        FilterKind::Include | FilterKind::Stage | FilterKind::Biquad | FilterKind::Delay
        | FilterKind::Preamp => scale(28, dpi),
        FilterKind::Unknown => scale(24, dpi),
    };
    px.max(scale(18, dpi)) as u32
}

pub(crate) fn geq_table_rect(dpi: u32, y: i32, row_height: i32, width: i32) -> RECT {
    let right = (width - scale(16, dpi)).max(scale(340, dpi));
    RECT {
        left: (right - scale(150, dpi)).max(scale(240, dpi)),
        top: y + scale(30, dpi),
        right,
        bottom: y + row_height - scale(10, dpi),
    }
}

pub(crate) fn geq_plot_rect(dpi: u32, y: i32, row_height: i32, width: i32) -> RECT {
    let table = geq_table_rect(dpi, y, row_height, width);
    RECT {
        left: scale(120, dpi),
        top: y + scale(30, dpi),
        right: (table.left - scale(8, dpi)).max(scale(200, dpi)),
        bottom: y + row_height - scale(10, dpi),
    }
}

/// Returns the top of the first node row and the row height.
pub(crate) fn geq_table_metrics(dpi: u32, table: &RECT) -> (i32, i32) {
    (table.top + scale(18, dpi), scale(20, dpi))
}

pub(crate) fn geq_table_visible_rows(dpi: u32, table: &RECT) -> usize {
    let (first, row_height) = geq_table_metrics(dpi, table);
    if row_height <= 0 {
        return 0;
    }
    ((table.bottom - first).max(0) / row_height) as usize
}

pub(crate) fn geq_table_column_split(dpi: u32, table: &RECT) -> i32 {
    table.left + scale(86, dpi)
}

pub(crate) fn geq_table_top_clamped(table_top: usize, nodes: usize, visible: usize) -> usize {
    table_top.min(nodes.saturating_sub(visible))
}

pub(crate) fn geq_table_cell_rect(
    dpi: u32,
    table: &RECT,
    table_top: usize,
    visible: usize,
    nodes: usize,
    index: usize,
    column: usize,
) -> Option<RECT> {
    if index < table_top || index >= table_top.saturating_add(visible) || index >= nodes {
        return None;
    }
    let (first, row_height) = geq_table_metrics(dpi, table);
    let split = geq_table_column_split(dpi, table);
    let top = first + (index - table_top) as i32 * row_height;
    let (left, right) = if column == 0 {
        (table.left, split)
    } else {
        (split, table.right)
    };
    Some(RECT {
        left,
        top,
        right,
        bottom: top + row_height,
    })
}

pub(crate) fn geq_table_cell_at(
    dpi: u32,
    table: &RECT,
    table_top: usize,
    visible: usize,
    nodes: usize,
    x: i32,
    y: i32,
) -> Option<(usize, usize)> {
    if !point_in(table, x, y) {
        return None;
    }
    let (first, row_height) = geq_table_metrics(dpi, table);
    if row_height <= 0 || y < first {
        return None;
    }
    let offset = ((y - first) / row_height) as usize;
    if offset >= visible {
        return None;
    }
    let index = table_top + offset;
    if index >= nodes {
        return None;
    }
    let split = geq_table_column_split(dpi, table);
    Some((index, if x < split { 0 } else { 1 }))
}

/// Clickable strip for the GraphicEQ node table scrollbar.
pub(crate) fn geq_table_scrollbar_rect(dpi: u32, table: &RECT) -> RECT {
    let (first, _) = geq_table_metrics(dpi, table);
    RECT {
        left: table.right - scale(10, dpi),
        top: first,
        right: table.right,
        bottom: table.bottom - 1,
    }
}

/// Maps a y coordinate on the scrollbar track to the first visible node.
pub(crate) fn geq_table_top_at_y(dpi: u32, table: &RECT, visible: usize, nodes: usize, y: i32) -> usize {
    if visible == 0 || nodes <= visible {
        return 0;
    }
    let (first, _) = geq_table_metrics(dpi, table);
    let track_height = (table.bottom - 1 - first).max(1);
    let relative = ((y - first).clamp(0, track_height) as f32) / track_height as f32;
    (relative * (nodes - visible) as f32).round() as usize
}

pub(crate) fn format_geq_value(value: f64) -> String {
    let rounded = if value.abs() < 0.005 {
        0.0
    } else {
        (value * 100.0).round() / 100.0
    };
    if (rounded - rounded.round()).abs() < 1e-9 {
        format!("{rounded:.0}")
    } else if (rounded * 10.0 - (rounded * 10.0).round()).abs() < 1e-9 {
        format!("{rounded:.1}")
    } else {
        format!("{rounded:.2}")
    }
}

pub(crate) fn geq_button_labels(band_count: i32) -> [String; 6] {
    // Short English captions keep the compact layout; other languages use the
    // original Editor translations (e.g. 15 频段 / 反转响应).
    let caption = |english: &str, source: &str| {
        if i18n::language() == i18n::Language::English {
            english.to_string()
        } else {
            i18n::tr(source)
        }
    };
    let marked = |label: String, active: bool| {
        if active {
            format!("{label} *")
        } else {
            label
        }
    };
    [
        marked(caption("15", "15-band"), band_count == 15),
        marked(caption("31", "31-band"), band_count == 31),
        marked(caption("Var", "variable"), band_count < 0),
        caption("Invert", "Invert response"),
        caption("Normalize", "Normalize response"),
        caption("Reset", "Reset response"),
    ]
}

pub(crate) fn geq_button_rects(dpi: u32, y: i32, band_count: i32) -> ([RECT; 6], [String; 6]) {
    let labels = geq_button_labels(band_count);
    let top = y + scale(4, dpi);
    let bottom = y + scale(24, dpi);
    let mut rects = [RECT::default(); 6];
    let mut left = scale(42, dpi);
    for (index, (rect, label)) in rects.iter_mut().zip(labels.iter()).enumerate() {
        // Widths ignore the active-band marker (with room reserved for it) so
        // the buttons do not move when the band count changes.
        let base = label.strip_suffix(" *").unwrap_or(label);
        let marker = if index < 3 { 14 } else { 0 };
        let width = scale((text_width_estimate(base) + marker + 12).max(44), dpi);
        *rect = RECT {
            left,
            top,
            right: left + width,
            bottom,
        };
        left += width + scale(5, dpi);
    }
    (rects, labels)
}

pub(crate) fn preamp_slider_rect(dpi: u32, y: i32) -> RECT {
    RECT {
        left: scale(120, dpi),
        top: y + scale(8, dpi),
        right: scale(370, dpi),
        bottom: y + scale(21, dpi),
    }
}

pub(crate) fn preamp_value_rect(dpi: u32, y: i32) -> RECT {
    RECT {
        left: scale(478, dpi),
        top: y + scale(4, dpi),
        right: scale(610, dpi),
        bottom: y + scale(24, dpi),
    }
}

pub(crate) fn biquad_field_rects(dpi: u32, y: i32) -> [RECT; 5] {
    let top = y + scale(4, dpi);
    let bottom = y + scale(24, dpi);
    let make = |left: i32, right: i32| RECT {
        left: scale(left, dpi),
        top,
        right: scale(right, dpi),
        bottom,
    };
    [
        make(42, 120),
        make(128, 230),
        make(238, 320),
        make(328, 400),
        make(408, 490),
    ]
}

pub(crate) fn delay_field_rects(dpi: u32, y: i32) -> [RECT; 2] {
    let top = y + scale(4, dpi);
    let bottom = y + scale(24, dpi);
    let make = |left: i32, right: i32| RECT {
        left: scale(left, dpi),
        top,
        right: scale(right, dpi),
        bottom,
    };
    [make(100, 220), make(228, 310)]
}

pub(crate) fn stage_field_rects(dpi: u32, y: i32) -> [RECT; 3] {
    let top = y + scale(4, dpi);
    let bottom = y + scale(24, dpi);
    let make = |left: i32, right: i32| RECT {
        left: scale(left, dpi),
        top,
        right: scale(right, dpi),
        bottom,
    };
    [make(90, 170), make(178, 258), make(266, 346)]
}

pub(crate) fn path_row_rects(dpi: u32, y: i32) -> [RECT; 2] {
    let top = y + scale(4, dpi);
    let bottom = y + scale(24, dpi);
    let make = |left: i32, right: i32| RECT {
        left: scale(left, dpi),
        top,
        right: scale(right, dpi),
        bottom,
    };
    [make(120, 500), make(508, 624)]
}

/// Second VST row line: "Open panel" and "Options" buttons.
pub(crate) fn vst_action_rects(dpi: u32, y: i32) -> [RECT; 2] {
    let top = y + scale(30, dpi);
    let bottom = y + scale(50, dpi);
    let make = |left: i32, right: i32| RECT {
        left: scale(left, dpi),
        top,
        right: scale(right, dpi),
        bottom,
    };
    [make(320, 420), make(428, 520)]
}

pub(crate) fn loudness_field_rects(dpi: u32, y: i32) -> [RECT; 4] {
    let top = y + scale(4, dpi);
    let bottom = y + scale(24, dpi);
    let make = |left: i32, right: i32| RECT {
        left: scale(left, dpi),
        top,
        right: scale(right, dpi),
        bottom,
    };
    [make(80, 190), make(198, 310), make(318, 420), make(428, 520)]
}

pub(crate) fn change_button_rect(dpi: u32, y: i32) -> RECT {
    RECT {
        left: scale(420, dpi),
        top: y + scale(4, dpi),
        right: scale(520, dpi),
        bottom: y + scale(24, dpi),
    }
}

/// Speaker positions present in `mask`, in the order used by the C++ channel
/// dialog: C, L, R, SL, SR, RL, RR, RC, LFE.
pub(crate) fn speaker_positions(mask: u32) -> Vec<(&'static str, &'static str)> {
    const DEFS: &[(u32, &str, &str)] = &[
        (channel_helper::SPEAKER_FRONT_CENTER, "C", "Center"),
        (channel_helper::SPEAKER_FRONT_LEFT, "L", "Left"),
        (channel_helper::SPEAKER_FRONT_RIGHT, "R", "Right"),
        (channel_helper::SPEAKER_SIDE_LEFT, "SL", "Side left"),
        (channel_helper::SPEAKER_SIDE_RIGHT, "SR", "Side right"),
        (channel_helper::SPEAKER_BACK_LEFT, "RL", "Rear left"),
        (channel_helper::SPEAKER_BACK_RIGHT, "RR", "Rear right"),
        (channel_helper::SPEAKER_BACK_CENTER, "RC", "Rear center"),
        (channel_helper::SPEAKER_LOW_FREQUENCY, "LFE", "Subwoofer"),
    ];
    DEFS.iter()
        .filter(|(bit, _, _)| mask & bit != 0)
        .map(|(_, name, label)| (*name, *label))
        .collect()
}

pub(crate) fn shorten(text: &str, max: usize) -> String {
    if text.chars().count() <= max {
        text.to_string()
    } else {
        let mut result: String = text.chars().take(max.saturating_sub(3)).collect();
        result.push_str("...");
        result
    }
}

/// Keeps the scroll offset inside `0..=total - viewport` after rows change
/// height or are removed.
pub(crate) fn clamp_scroll_offset(total: u32, viewport: i32, offset: i32) -> i32 {
    offset.clamp(0, (total as i32 - viewport).max(0))
}

pub(crate) fn point_in(rect: &RECT, x: i32, y: i32) -> bool {
    x >= rect.left && x < rect.right && y >= rect.top && y < rect.bottom
}

/// Compares two paths the way Windows does: canonical form when available,
/// otherwise a case-insensitive string comparison.
pub(crate) fn same_file(left: &std::path::Path, right: &std::path::Path) -> bool {
    if let (Ok(left), Ok(right)) = (left.canonicalize(), right.canonicalize()) {
        return left == right;
    }
    left == right
        || left
            .to_string_lossy()
            .eq_ignore_ascii_case(&right.to_string_lossy())
}

thread_local! {
    /// Cursor position (client coordinates) used by free paint helpers to
    /// highlight the control under the mouse.
    static HOVER_POS: Cell<Option<(i32, i32)>> = const { Cell::new(None) };
}

pub(crate) fn set_hover_pos(pos: Option<(i32, i32)>) {
    HOVER_POS.with(|cell| cell.set(pos));
}

pub(crate) fn hovering(rect: &RECT) -> bool {
    HOVER_POS
        .with(|cell| cell.get())
        .map(|(x, y)| point_in(rect, x, y))
        .unwrap_or(false)
}

pub(crate) fn biquad_kind_label(kind: BiquadKind) -> &'static str {
    match kind {
        BiquadKind::Peaking => "Peak",
        BiquadKind::LowPass => "Low-pass",
        BiquadKind::HighPass => "High-pass",
        BiquadKind::BandPass => "Band-pass",
        BiquadKind::LowShelf => "Low-shelf",
        BiquadKind::HighShelf => "High-shelf",
        BiquadKind::Notch => "Notch",
        BiquadKind::AllPass => "All-pass",
    }
}

pub(crate) fn q_mode_label(mode: QMode) -> &'static str {
    match mode {
        QMode::Fixed => "Fixed",
        QMode::Q => "Q",
        QMode::Bandwidth => "Bandwidth",
        QMode::Slope => "Slope",
    }
}

pub(crate) fn biquad_kind_source(kind: BiquadKind) -> &'static str {
    match kind {
        BiquadKind::Peaking => "Peaking filter",
        BiquadKind::LowPass => "Low-pass filter",
        BiquadKind::HighPass => "High-pass filter",
        BiquadKind::BandPass => "Band-pass filter",
        BiquadKind::LowShelf => "Low-shelf filter",
        BiquadKind::HighShelf => "High-shelf filter",
        BiquadKind::Notch => "Notch filter",
        BiquadKind::AllPass => "All-pass filter",
    }
}

/// Short English caption in the narrow combo; the original Editor wording
/// (and its translations) otherwise.
pub(crate) fn biquad_kind_display(kind: BiquadKind) -> String {
    if i18n::language() == i18n::Language::English {
        biquad_kind_label(kind).to_string()
    } else {
        i18n::tr(biquad_kind_source(kind))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scroll_offset_clamps_to_content() {
        assert_eq!(clamp_scroll_offset(1000, 400, 900), 600);
        assert_eq!(clamp_scroll_offset(1000, 400, 100), 100);
        // Content shorter than the viewport always scrolls back to the top.
        assert_eq!(clamp_scroll_offset(300, 400, 120), 0);
        assert_eq!(clamp_scroll_offset(1000, 400, -20), 0);
    }

    #[test]
    fn shorten_adds_ellipsis_only_when_needed() {
        assert_eq!(shorten("GraphicEQ", 20), "GraphicEQ");
        assert_eq!(shorten("abcdef", 5), "ab...");
        // Width is counted in characters, not bytes.
        assert_eq!(shorten("中文测试", 3), "...");
    }
}
