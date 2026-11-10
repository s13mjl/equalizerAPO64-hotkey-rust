//! Virtualized filter row painting for the native editor.

use super::*;

impl App {
    pub(super) unsafe fn paint_rows(&mut self, hwnd: HWND, hdc: HDC, client: RECT, chrome: i32) {
        if client.bottom <= chrome || self.total == 0 {
            return;
        }
        let saved = SaveDC(hdc);
        let _ = IntersectClipRect(hdc, 0, chrome, client.right, client.bottom);

        let first = self
            .prefix
            .partition_point(|&start| start <= self.offset as u32)
            .saturating_sub(1);
        let mut index = first;
        let mut y = chrome + self.prefix[first] as i32 - self.offset;

        let while_guard = self.rows.len() + 1;
        let mut guard = 0;
        while index < self.rows.len() && y < client.bottom && guard < while_guard {
            guard += 1;
            self.ensure_item(index);
            let vst_path = match self.rows[index]
                .item
                .as_ref()
                .and_then(|item| item.editor.as_ref())
            {
                Some(FilterEditor::Vst(vst)) => Some(vst.path.clone()),
                _ => None,
            };
            let vst_readable = vst_path
                .as_deref()
                .and_then(|path| self.vst_readable(path));
            let row = &self.rows[index];
            let height = row.height as i32;
            if y + height > chrome {
                let background = if self.selection.contains(&index) {
                    SELECTED
                } else {
                    WHITE
                };
                fill(
                    hdc,
                    0,
                    y.max(chrome),
                    client.right,
                    (y + height).min(client.bottom),
                    background,
                );

                let mut number = [0u16; 24];
                let number_len = format_row_number(&mut number, index);
                SelectObject(hdc, HGDIOBJ(self.small.0));
                draw_text(
                    hdc,
                    scale(6, self.dpi),
                    y + scale(5, self.dpi),
                    &number[..number_len],
                    MUTED,
                );
                SelectObject(hdc, HGDIOBJ(self.body.0));

                let box_size = scale(12, self.dpi);
                let box_x = scale(20, self.dpi);
                let box_y = y + (height - box_size) / 2;
                let box_rect = RECT {
                    left: box_x,
                    top: box_y,
                    right: box_x + box_size,
                    bottom: box_y + box_size,
                };
                let box_bg = if self.is_hot_rect(&box_rect) {
                    HOVER
                } else {
                    WHITE
                };
                fill(hdc, box_x, box_y, box_x + box_size, box_y + box_size, box_bg);
                self.frame(hdc, box_x, box_y, box_x + box_size, box_y + box_size);
                if row.enabled {
                    let inset = scale(3, self.dpi).max(1);
                    fill(
                        hdc,
                        box_x + inset,
                        box_y + inset,
                        box_x + box_size - inset,
                        box_y + box_size - inset,
                        BAR,
                    );
                }

                let mut painted = false;
                if let Some(item) = &row.item {
                    if let Some(editor) = &item.editor {
                        match editor {
                            FilterEditor::Preamp { db } => {
                                paint_preamp_row(hdc, self.dpi, y, *db, row.enabled);
                                painted = true;
                            }
                            FilterEditor::Biquad(biquad) => {
                                paint_biquad_row(hdc, self.dpi, y, biquad, row.enabled);
                                painted = true;
                            }
                            FilterEditor::Delay { delay, is_ms } => {
                                paint_delay_row(hdc, self.dpi, y, *delay, *is_ms, row.enabled);
                                painted = true;
                            }
                            FilterEditor::Stage {
                                pre_mix,
                                post_mix,
                                capture,
                            } => {
                                paint_stage_row(
                                    hdc, self.dpi, y, *pre_mix, *post_mix, *capture, row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::Include { path } => {
                                paint_path_row(
                                    hdc, self.dpi, y, "Include", path, "Select file", row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::Convolution { path, .. } => {
                                paint_path_row(
                                    hdc,
                                    self.dpi,
                                    y,
                                    "Convolution",
                                    path,
                                    "Select file",
                                    row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::Vst(vst) => {
                                paint_vst_row(hdc, self.dpi, y, vst, vst_readable, row.enabled);
                                painted = true;
                            }
                            FilterEditor::Loudness {
                                ref_level,
                                ref_offset,
                                att,
                                ..
                            } => {
                                paint_loudness_row(
                                    hdc,
                                    self.dpi,
                                    y,
                                    *ref_level,
                                    *ref_offset,
                                    *att,
                                    row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::Channel { selected, .. } => {
                                paint_summary_row(
                                    hdc,
                                    self.dpi,
                                    y,
                                    "Channel",
                                    &selected.join(" "),
                                    row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::Device { pattern, .. } => {
                                paint_summary_row(
                                    hdc, self.dpi, y, "Device", pattern, row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::GraphicEq(graphic) => {
                                paint_graphic_eq_row(
                                    hdc,
                                    self.paint_offset,
                                    self.dpi,
                                    y,
                                    height,
                                    client.right,
                                    graphic,
                                    row.geq_table_top,
                                    row.enabled,
                                );
                                painted = true;
                            }
                            FilterEditor::Copy { .. } => {
                                let parameters = row
                                    .line
                                    .split_once(':')
                                    .map(|(_, value)| value.trim())
                                    .unwrap_or("");
                                paint_copy_row(hdc, self.dpi, y, parameters, row.enabled);
                                painted = true;
                            }
                        }
                    }
                }
                if !painted {
                    let mut text = [0u16; 1024];
                    let text_len = format_row_text(&mut text, row);
                    SelectObject(hdc, HGDIOBJ(self.body.0));
                    draw_text(
                        hdc,
                        scale(42, self.dpi),
                        y + scale(4, self.dpi),
                        &text[..text_len],
                        if row.enabled { TEXT } else { MUTED },
                    );
                }
                if let Some(hotkey) = row.hotkey.as_ref() {
                    SelectObject(hdc, HGDIOBJ(self.small.0));
                    draw_text(
                        hdc,
                        client.right - scale(170, self.dpi),
                        y + scale(5, self.dpi),
                        &wide(&hotkey.to_string_qt()),
                        MUTED,
                    );
                }

                fill(
                    hdc,
                    0,
                    (y + height - 1).max(chrome),
                    client.right,
                    (y + height).min(client.bottom),
                    BORDER,
                );
            }
            y += height;
            index += 1;
        }
        if self.reorder.as_ref().map(|state| state.active).unwrap_or(false) {
            if let Some(drop) = self.reorder_drop {
                let drop = drop.min(self.rows.len());
                let line_y = (chrome + self.prefix[drop] as i32 - self.offset)
                    .clamp(chrome, client.bottom.saturating_sub(1));
                fill(hdc, 0, line_y, client.right, line_y + 2, CURVE_POS);
            }
        }
        let _ = RestoreDC(hdc, saved);
        let _ = hwnd;
    }
}
