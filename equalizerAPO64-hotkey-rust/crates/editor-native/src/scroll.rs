//! Scrolling, wheel handling and row invalidation.

use super::*;

impl App {
    pub(super) unsafe fn update_scrollbar(&mut self, hwnd: HWND) {
        let viewport = self.viewport_height(hwnd);
        self.offset = clamp_scroll_offset(self.total, viewport, self.offset);
        let info = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: self.total.saturating_sub(1) as i32,
            nPage: viewport as u32,
            nPos: self.offset,
            nTrackPos: self.offset,
        };
        SetScrollInfo(hwnd, SB_VERT, &info, true);
    }

    pub(super) unsafe fn set_offset(&mut self, hwnd: HWND, offset: i32) {
        let viewport = self.viewport_height(hwnd);
        let max_offset = (self.total as i32 - viewport).max(0);
        self.offset = offset.clamp(0, max_offset);
        let info = SCROLLINFO {
            cbSize: size_of::<SCROLLINFO>() as u32,
            fMask: SIF_RANGE | SIF_PAGE | SIF_POS,
            nMin: 0,
            nMax: self.total.saturating_sub(1) as i32,
            nPage: viewport as u32,
            nPos: self.offset,
            nTrackPos: self.offset,
        };
        SetScrollInfo(hwnd, SB_VERT, &info, true);
        self.prune_items(hwnd);
        let _ = InvalidateRect(Some(hwnd), None, false);
    }

    pub(super) unsafe fn on_vscroll(&mut self, hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
        let command = (wparam.0 & 0xFFFF) as i32;
        let viewport = self.viewport_height(hwnd);
        let line = scale(24, self.dpi);
        let max_offset = (self.total as i32 - viewport).max(0);
        let mut offset = self.offset;
        match command {
            c if c == SB_LINEUP.0 => offset -= line,
            c if c == SB_LINEDOWN.0 => offset += line,
            c if c == SB_PAGEUP.0 => offset -= viewport,
            c if c == SB_PAGEDOWN.0 => offset += viewport,
            c if c == SB_THUMBPOSITION.0 || c == SB_THUMBTRACK.0 => {
                offset = ((lparam.0 >> 16) & 0xFFFF) as i32;
            }
            c if c == SB_TOP.0 => offset = 0,
            c if c == SB_BOTTOM.0 => offset = max_offset,
            c if c == SB_ENDSCROLL.0 => {}
            _ => {}
        }
        self.set_offset(hwnd, offset);
    }

    pub(super) unsafe fn on_wheel(&mut self, hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
        self.commit_value_edit(hwnd, true);
        let delta = ((wparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32;
        let mut cursor = POINT {
            x: (lparam.0 & 0xFFFF) as u16 as i16 as i32,
            y: ((lparam.0 >> 16) & 0xFFFF) as u16 as i16 as i32,
        };
        let _ = ScreenToClient(hwnd, &mut cursor);
        if self.show_analysis && point_in(&self.plot_rect, cursor.x, cursor.y) {
            let view = (
                (self.plot_rect.right - self.plot_rect.left) as f32,
                (self.plot_rect.bottom - self.plot_rect.top) as f32,
            );
            self.plot.zoom_at(
                delta as f32,
                (cursor.x - self.plot_rect.left) as f32,
                (cursor.y - self.plot_rect.top) as f32,
                view.0,
                view.1,
            );
            self.invalidate_plot(hwnd);
            return;
        }
        if let Some((row, table)) = self.geq_row_table_at(hwnd, cursor.x, cursor.y) {
            self.ensure_item(row);
            let visible = geq_table_visible_rows(self.dpi, &table);
            let nodes = match self.rows[row]
                .item
                .as_ref()
                .and_then(|item| item.editor.as_ref())
            {
                Some(FilterEditor::GraphicEq(graphic)) => graphic.nodes.len(),
                _ => 0,
            };
            if nodes > 0 {
                let max_top = nodes.saturating_sub(visible);
                let current = self.rows[row].geq_table_top.min(max_top);
                let step = 3usize;
                let next = if delta > 0 {
                    current.saturating_sub(step)
                } else {
                    current.saturating_add(step).min(max_top)
                };
                if next != current {
                    self.rows[row].geq_table_top = next;
                    self.invalidate_row(hwnd, row);
                }
                return;
            }
        }
        if let Some((row, plot)) = self.geq_row_plot_at(hwnd, cursor.x, cursor.y) {
            self.ensure_item(row);
            let mut zoomed = false;
            if let Some(item) = self.rows[row].item.as_mut() {
                if let Some(FilterEditor::GraphicEq(graphic)) = &mut item.editor {
                    let mut view = GeqView::new(graphic);
                    view.zoom_at(
                        delta as f32,
                        (cursor.x - plot.left) as f32,
                        (cursor.y - plot.top) as f32,
                        &plot,
                    );
                    view.store(graphic);
                    zoomed = true;
                }
            }
            if zoomed {
                self.invalidate_row(hwnd, row);
                return;
            }
        }
        let line = scale(24, self.dpi);
        let offset = self.offset - delta / 120 * line * 3;
        self.set_offset(hwnd, offset);
    }

    /// Finds the GraphicEQ row plot under a client point, if any.
    pub(super) unsafe fn geq_row_plot_at(&self, hwnd: HWND, x: i32, y: i32) -> Option<(usize, RECT)> {
        let chrome = self.chrome_height();
        if self.rows.is_empty() || y < chrome || y >= self.table_bottom(hwnd) {
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
        if self.rows[row].kind != FilterKind::GraphicEq {
            return None;
        }
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let row_y = chrome + self.prefix[row] as i32 - self.offset;
        let plot = geq_plot_rect(self.dpi, row_y, self.rows[row].height as i32, client.right);
        point_in(&plot, x, y).then_some((row, plot))
    }

    /// Finds the GraphicEQ row node table under a client point, if any.
    pub(super) unsafe fn geq_row_table_at(&self, hwnd: HWND, x: i32, y: i32) -> Option<(usize, RECT)> {
        let chrome = self.chrome_height();
        if self.rows.is_empty() || y < chrome || y >= self.table_bottom(hwnd) {
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
        if self.rows[row].kind != FilterKind::GraphicEq {
            return None;
        }
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let row_y = chrome + self.prefix[row] as i32 - self.offset;
        let table = geq_table_rect(self.dpi, row_y, self.rows[row].height as i32, client.right);
        point_in(&table, x, y).then_some((row, table))
    }


    pub(super) unsafe fn invalidate_row(&self, hwnd: HWND, row: usize) {
        if row >= self.rows.len() {
            return;
        }
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let top = self.chrome_height() + self.prefix[row] as i32 - self.offset;
        let rect = RECT {
            left: 0,
            top,
            right: client.right,
            bottom: top + self.rows[row].height as i32,
        };
        let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
    }

    pub(super) unsafe fn invalidate_plot(&self, hwnd: HWND) {
        if self.plot_rect.right > self.plot_rect.left && self.plot_rect.bottom > self.plot_rect.top
        {
            let _ = InvalidateRect(Some(hwnd), Some(&self.plot_rect), false);
        } else {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }


}
