//! Client geometry and hit-test rectangles for the native editor.

use super::*;

impl App {
    pub(super) fn chrome_height(&self) -> i32 {
        self.tab_top() + scale(33, self.dpi)
    }

    /// Layout of the three toolbar combos, shared by painting and hit tests.
    pub(super) fn toolbar_layout(&self) -> [RECT; 3] {
        let dpi = self.dpi;
        let top = scale(6, dpi);
        let bottom = top + scale(22, dpi);
        let text_x = scale(200, dpi);
        let text_width = scale(text_width_estimate(&i18n::tr("Instant mode")), dpi);
        let instant = RECT {
            left: scale(180, dpi),
            top,
            right: text_x + text_width + scale(8, dpi),
            bottom,
        };
        let device = RECT {
            left: instant.right + scale(8, dpi),
            top,
            right: instant.right + scale(8, dpi) + scale(128, dpi),
            bottom,
        };
        let channel = RECT {
            left: device.right + scale(8, dpi),
            top,
            right: device.right + scale(8, dpi) + scale(112, dpi),
            bottom,
        };
        [instant, device, channel]
    }

    /// Top of the document tab strip (the toolbar above it can be hidden).
    pub(super) fn tab_top(&self) -> i32 {
        if self.show_toolbar {
            scale(31, self.dpi)
        } else {
            0
        }
    }

    pub(super) fn is_hot_rect(&self, rect: &RECT) -> bool {
        self.hover_pos
            .map(|(x, y)| point_in(rect, x, y))
            .unwrap_or(false)
    }

    /// Tab rectangles in paint order (client coordinates). Widths shrink so
    /// the close button of the last tab never leaves the window.
    pub(super) fn tab_layout(&self) -> Vec<(usize, RECT)> {
        let dpi = self.dpi;
        let chrome = self.chrome_height();
        let top = self.tab_top() + scale(2, dpi);
        let margin = scale(8, dpi);
        let gap = scale(2, dpi);
        let available = (self.client_width - margin * 2 - scale(8, dpi)).max(scale(90, dpi));
        let count = self.tabs.len().max(1) as i32;
        let max_per_tab = ((available - gap * (count - 1)) / count).max(scale(48, dpi));
        let mut tab_x = margin;
        let mut rects = Vec::with_capacity(self.tabs.len());
        for (index, tab) in self.tabs.iter().enumerate() {
            let text_width = text_width_estimate(&tab.title) + if tab.dirty { 14 } else { 0 };
            let width = scale(text_width + 34, dpi)
                .clamp(scale(90, dpi), scale(240, dpi))
                .min(max_per_tab);
            rects.push((
                index,
                RECT {
                    left: tab_x,
                    top,
                    right: tab_x + width,
                    bottom: chrome,
                },
            ));
            tab_x += width + gap;
        }
        rects
    }

    /// Rect of the clickable control under the cursor, used so that only the
    /// controls whose hover state changed get repainted.
    pub(super) unsafe fn hot_control_at(&self, hwnd: HWND, x: i32, y: i32) -> Option<RECT> {
        let dpi = self.dpi;
        let chrome = self.chrome_height();
        let table_bottom = self.table_bottom(hwnd);
        if y >= table_bottom {
            return self
                .analysis_hits
                .iter()
                .find(|(_, rect)| point_in(rect, x, y))
                .map(|(_, rect)| *rect);
        }
        if self.show_toolbar && y < scale(30, dpi) {
            let top = scale(6, dpi);
            let bottom = top + scale(22, dpi);
            let mut left = scale(8, dpi);
            for _ in 0..3 {
                let rect = RECT {
                    left,
                    top,
                    right: left + scale(54, dpi),
                    bottom,
                };
                if point_in(&rect, x, y) {
                    return Some(rect);
                }
                left += scale(58, dpi);
            }
            for rect in self.toolbar_layout() {
                if point_in(&rect, x, y) {
                    return Some(rect);
                }
            }
            return None;
        }
        if y < chrome {
            return self
                .tab_layout()
                .into_iter()
                .find(|(_, rect)| point_in(rect, x, y))
                .map(|(_, rect)| rect);
        }
        if self.rows.is_empty() {
            return None;
        }
        let content = y - chrome + self.offset;
        if content < 0 || content as u32 >= self.total {
            return None;
        }
        let row = self
            .prefix
            .partition_point(|&start| start <= content as u32)
            .saturating_sub(1)
            .min(self.rows.len() - 1);
        let row_y = chrome + self.prefix[row] as i32 - self.offset;
        let height = self.rows[row].height as i32;
        let box_size = scale(12, dpi);
        let box_x = scale(20, dpi);
        let box_y = row_y + (height - box_size) / 2;
        let power_rect = RECT {
            left: box_x,
            top: box_y,
            right: box_x + box_size,
            bottom: box_y + box_size,
        };
        if point_in(&power_rect, x, y) {
            return Some(power_rect);
        }
        if !self.rows[row].enabled {
            return None;
        }
        let item = self.rows[row].item.as_ref()?;
        let editor = item.editor.as_ref()?;
        let hit = |rects: &[RECT]| rects.iter().find(|rect| point_in(rect, x, y)).copied();
        match editor {
            FilterEditor::Preamp { .. } => {
                let value = preamp_value_rect(dpi, row_y);
                if point_in(&value, x, y) {
                    return Some(value);
                }
                let mut track = preamp_slider_rect(dpi, row_y);
                track.right = scale(470, dpi);
                if point_in(&track, x, y) {
                    Some(track)
                } else {
                    None
                }
            }
            FilterEditor::Biquad(_) => hit(&biquad_field_rects(dpi, row_y)),
            FilterEditor::Delay { .. } => hit(&delay_field_rects(dpi, row_y)),
            FilterEditor::Stage { .. } => hit(&stage_field_rects(dpi, row_y)),
            FilterEditor::Include { .. } | FilterEditor::Convolution { .. } => {
                let rects = path_row_rects(dpi, row_y);
                if point_in(&rects[1], x, y) {
                    Some(rects[1])
                } else {
                    None
                }
            }
            FilterEditor::Vst(_) => {
                let mut rects = path_row_rects(dpi, row_y).to_vec();
                rects.extend_from_slice(&vst_action_rects(dpi, row_y));
                hit(&rects)
            }
            FilterEditor::Loudness { .. } => hit(&loudness_field_rects(dpi, row_y)),
            FilterEditor::Channel { .. } | FilterEditor::Device { .. } | FilterEditor::Copy { .. } => {
                let rect = change_button_rect(dpi, row_y);
                if point_in(&rect, x, y) {
                    Some(rect)
                } else {
                    None
                }
            }
            FilterEditor::GraphicEq(graphic) => {
                let (buttons, _) = geq_button_rects(dpi, row_y, graphic.band_count);
                if let Some(rect) = hit(&buttons) {
                    return Some(rect);
                }
                let mut client = RECT::default();
                let _ = GetClientRect(hwnd, &mut client);
                let table = geq_table_rect(dpi, row_y, self.rows[row].height as i32, client.right);
                let visible = geq_table_visible_rows(dpi, &table);
                let table_top = geq_table_top_clamped(
                    self.rows[row].geq_table_top,
                    graphic.nodes.len(),
                    visible,
                );
                if let Some((index, column)) = geq_table_cell_at(
                    dpi,
                    &table,
                    table_top,
                    visible,
                    graphic.nodes.len(),
                    x,
                    y,
                ) {
                    return geq_table_cell_rect(
                        dpi,
                        &table,
                        table_top,
                        visible,
                        graphic.nodes.len(),
                        index,
                        column,
                    );
                }
                None
            }
        }
    }

    pub(super) fn analysis_height(&self) -> i32 {
        if self.show_analysis {
            scale(320, self.dpi)
        } else {
            0
        }
    }

    pub(super) unsafe fn table_bottom(&self, hwnd: HWND) -> i32 {
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        client.bottom - self.analysis_height()
    }

    pub(super) unsafe fn viewport_height(&self, hwnd: HWND) -> i32 {
        (self.table_bottom(hwnd) - self.chrome_height()).max(0)
    }

}
