//! Toolbar and tab strip painting for the native editor.

use super::*;

impl App {
    pub(super) unsafe fn paint_chrome(&mut self, hdc: HDC, width: i32, chrome: i32) {
        let old_font = SelectObject(hdc, HGDIOBJ(self.body.0));
        fill(hdc, 0, 0, width, chrome, palette::chrome_bg());

        if self.show_toolbar {
            let margin = scale(8, self.dpi);
            let button_y = scale(6, self.dpi);
            let button_h = scale(22, self.dpi);
            let mut x = margin;
            for label in [&self.labels.new, &self.labels.open, &self.labels.save] {
                self.paint_button(hdc, x, button_y, scale(54, self.dpi), button_h, label);
                x += scale(58, self.dpi);
            }

            let instant_rect = self.toolbar_layout()[0];
            if self.is_hot_rect(&instant_rect) {
                fill(
                    hdc,
                    instant_rect.left,
                    instant_rect.top,
                    instant_rect.right,
                    instant_rect.bottom,
                    palette::hover(),
                );
            }
            let check_size = scale(13, self.dpi);
            let check_y = button_y + (button_h - check_size) / 2;
            fill(hdc, x, check_y, x + check_size, check_y + check_size, palette::white());
            self.frame(hdc, x, check_y, x + check_size, check_y + check_size);
            if self.instant_mode {
                let inset = scale(3, self.dpi).max(1);
                fill(
                    hdc,
                    x + inset,
                    check_y + inset,
                    x + check_size - inset,
                    check_y + check_size - inset,
                    palette::bar(),
                );
            }
            x += check_size + scale(5, self.dpi);
            draw_text(hdc, x, button_y + scale(4, self.dpi), &self.labels.instant, palette::text());

            let layout = self.toolbar_layout();
            let device_rect = layout[1];
            let device_text = self
                .selected_device
                .as_ref()
                .map(|device| {
                    format!(
                        "{} - {}",
                        device.get_connection_name(),
                        device.get_device_name()
                    )
                })
                .unwrap_or_else(|| "No device".to_string());
            self.paint_combo(
                hdc,
                device_rect.left,
                button_y,
                device_rect.right - device_rect.left,
                button_h,
                &wide(&shorten(&device_text, 20)),
            );
            let channel_rect = layout[2];
            let channel_text = if self.channel_names.is_empty() {
                "From device".to_string()
            } else {
                self.channel_names.join(" ")
            };
            self.paint_combo(
                hdc,
                channel_rect.left,
                button_y,
                channel_rect.right - channel_rect.left,
                button_h,
                &wide(&shorten(&channel_text, 14)),
            );
        }

        let tab_top = self.tab_top();
        fill(hdc, 0, tab_top, width, chrome, palette::chrome_bg());
        for (index, rect) in self.tab_layout() {
            let tab = &self.tabs[index];
            let title = format!("{}{}", tab.title, if tab.dirty { " *" } else { "" });
            let active = index == self.active_tab;
            let hover = self.is_hot_rect(&rect);
            fill(
                hdc,
                rect.left,
                rect.top,
                rect.right,
                rect.bottom,
                if active {
                    palette::white()
                } else if hover {
                    palette::hover()
                } else {
                    palette::chrome_bg()
                },
            );
            self.frame(hdc, rect.left, rect.top, rect.right, rect.bottom);
            SelectObject(hdc, HGDIOBJ(if active { self.bold.0 } else { self.body.0 }));
            let available = ((rect.right - rect.left - scale(24, self.dpi)) as f64 * 96.0
                / self.dpi as f64) as i32;
            let shown = fit_text(&title, available.max(16));
            let saved = SaveDC(hdc);
            let _ = IntersectClipRect(
                hdc,
                rect.left + scale(4, self.dpi),
                rect.top,
                rect.right - scale(20, self.dpi),
                rect.bottom,
            );
            draw_text(
                hdc,
                rect.left + scale(8, self.dpi),
                tab_top + scale(6, self.dpi),
                &wide(&shown),
                if active { palette::text() } else { palette::muted() },
            );
            let _ = RestoreDC(hdc, saved);
            draw_text(
                hdc,
                rect.right - scale(16, self.dpi),
                tab_top + scale(6, self.dpi),
                &wide("x"),
                palette::muted(),
            );
        }
        fill(hdc, 0, chrome - 1, width, chrome, palette::border());
        SelectObject(hdc, old_font);
    }

    pub(super) unsafe fn paint_button(&self, hdc: HDC, x: i32, y: i32, w: i32, h: i32, label: &[u16]) {
        let rect = RECT {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        };
        fill(
            hdc,
            x,
            y,
            x + w,
            y + h,
            if self.is_hot_rect(&rect) { palette::hover() } else { palette::white() },
        );
        self.frame(hdc, x, y, x + w, y + h);
        draw_text(hdc, x + scale(8, self.dpi), y + scale(4, self.dpi), label, palette::text());
    }

    pub(super) unsafe fn paint_combo(&self, hdc: HDC, x: i32, y: i32, w: i32, h: i32, label: &[u16]) {
        let rect = RECT {
            left: x,
            top: y,
            right: x + w,
            bottom: y + h,
        };
        fill(
            hdc,
            x,
            y,
            x + w,
            y + h,
            if self.is_hot_rect(&rect) { palette::hover() } else { palette::white() },
        );
        self.frame(hdc, x, y, x + w, y + h);
        let arrow_x = x + w - scale(14, self.dpi);
        let saved = SaveDC(hdc);
        let _ = IntersectClipRect(
            hdc,
            x + scale(2, self.dpi),
            y,
            arrow_x - scale(2, self.dpi),
            y + h,
        );
        draw_text(hdc, x + scale(6, self.dpi), y + scale(4, self.dpi), label, palette::text());
        let _ = RestoreDC(hdc, saved);
        let arrow_y = y + h / 2 - scale(2, self.dpi);
        for row in 0..scale(4, self.dpi) {
            let half = scale(4, self.dpi) - row;
            fill(
                hdc,
                arrow_x + half,
                arrow_y + row,
                arrow_x + scale(8, self.dpi) - half,
                arrow_y + row + 1,
                palette::muted(),
            );
        }
    }

    pub(super) unsafe fn frame(&self, hdc: HDC, left: i32, top: i32, right: i32, bottom: i32) {
        fill(hdc, left, top, right, top + 1, palette::border());
        fill(hdc, left, bottom - 1, right, bottom, palette::border());
        fill(hdc, left, top, left + 1, bottom, palette::border());
        fill(hdc, right - 1, top, right, bottom, palette::border());
    }
}
