//! Analysis dock painting for the native editor.

use super::*;

impl App {
    pub(super) unsafe fn paint_analysis(&mut self, hdc: HDC, width: i32, top: i32, bottom: i32) {
        if bottom <= top {
            return;
        }
        fill(hdc, 0, top, width, bottom, PLOT_BG);
        fill(hdc, 0, top, width, top + 1, BORDER);

        let left_width = scale(250, self.dpi);
        let margin = scale(12, self.dpi);
        SelectObject(hdc, HGDIOBJ(self.bold.0));
        draw_text(hdc, margin, top + scale(8, self.dpi), &trw("Analysis"), TEXT);
        SelectObject(hdc, HGDIOBJ(self.body.0));

        let mut line_y = top + scale(30, self.dpi);
        self.analysis_hits.clear();
        match &self.analysis_result {
            Some(result) => {
                let peak_color = if result.peak_gain > 0.0 {
                    COLORREF(0x0020_20C0)
                } else {
                    TEXT
                };
                for (label, value_color) in [
                    (format!("{} {:.2} dB", i18n::tr("Peak gain:"), result.peak_gain), peak_color),
                    (
                        format!(
                            "{} {} {} ({:.2} ms)",
                            i18n::tr("Latency:"),
                            result.latency,
                            i18n::tr(" samples"),
                            result.latency as f64 * 1000.0
                                / result.freq_data_sample_rate.max(1) as f64
                        ),
                        TEXT,
                    ),
                    (format!("{} {:.1} ms", i18n::tr("Init:"), result.initialization_time), TEXT),
                    (format!("{} {:.2} %", i18n::tr("CPU usage:"), result.cpu_usage()), TEXT),
                    (
                        format!("{} {}", i18n::tr("Resolution:"), self.analysis_effective_resolution),
                        MUTED,
                    ),
                    (
                        format!("{} {}", i18n::tr("Channels:"), shorten(&self.channel_names.join(" "), 28)),
                        MUTED,
                    ),
                ] {
                    draw_text(hdc, margin, line_y, &wide(&label), value_color);
                    line_y += scale(18, self.dpi);
                }
            }
            None => {
                draw_text(hdc, margin, line_y, &trw("Analyzing..."), MUTED);
            }
        }

        let start_value = if self.analysis_start_from == 1 {
            i18n::tr("Current file")
        } else {
            "config.txt".to_string()
        };
        let channel_value = self
            .channel_names
            .get(self.analysis_channel_index)
            .cloned()
            .unwrap_or_default();
        let controls = [
            (AnalysisSetting::StartFrom, i18n::tr("Start from"), start_value),
            (AnalysisSetting::Channel, i18n::tr("Channel"), channel_value),
            (
                AnalysisSetting::Resolution,
                i18n::tr("Resolution"),
                self.analysis_resolution.to_string(),
            ),
        ];
        let controls_top = line_y.max(top + scale(196, self.dpi)) + scale(6, self.dpi);
        let mut controls_y = controls_top;
        for (kind, label, value) in controls {
            let rect = RECT {
                left: margin,
                top: controls_y,
                right: left_width - scale(10, self.dpi),
                bottom: controls_y + scale(20, self.dpi),
            };
            draw_control(
                hdc,
                self.dpi,
                &rect,
                &format!("{label}: {value}"),
                true,
                None,
            );
            self.analysis_hits.push((kind, rect));
            controls_y += scale(24, self.dpi);
        }
        if self.analysis_effective_resolution != self.analysis_resolution {
            draw_text(
                hdc,
                margin,
                controls_y + scale(2, self.dpi),
                &wide(&i18n::tr_arg(
                    "Resolution reduced to %0 to limit memory",
                    &[&self.analysis_effective_resolution.to_string()],
                )),
                WARNING,
            );
        }

        let plot_left = left_width;
        let plot_right = width - scale(14, self.dpi);
        let plot_top = top + scale(22, self.dpi);
        let plot_bottom = bottom - scale(20, self.dpi);
        if plot_right <= plot_left + scale(20, self.dpi) || plot_bottom <= plot_top + scale(20, self.dpi) {
            return;
        }
        let plot_width = (plot_right - plot_left) as f32;
        let plot_height = (plot_bottom - plot_top) as f32;
        self.plot_rect = RECT {
            left: plot_left,
            top: plot_top,
            right: plot_right,
            bottom: plot_bottom,
        };
        if self.plot_fitted_for != (plot_width as i32, plot_height as i32) {
            // Always fit the audible range (20 Hz..20 kHz) to the current
            // plot width, so resizing the window scales the chart instead of
            // squeezing it (wider window => wider, filled plot).
            self.plot.fit(plot_width, plot_height);
            self.plot.fit_frequency_range(20.0, 20000.0, plot_width);
            if let Some((zoom_x, zoom_y, scroll_x, scroll_y)) = self.plot_restore.take() {
                self.plot.zoom_x = zoom_x.clamp(0.5, 30.0);
                self.plot.zoom_y = zoom_y.clamp(0.5, 30.0);
                self.plot.scroll_x = scroll_x as f32;
                self.plot.scroll_y = scroll_y as f32;
            } else {
                self.plot.default_scroll();
            }
            self.plot.pan(0.0, 0.0, plot_width, plot_height);
            self.plot_fitted_for = (plot_width as i32, plot_height as i32);
        }
        let plot = &self.plot;

        for db in [-100i32, -80, -60, -40, -20, 0, 20, 40, 60, 80, 100] {
            let Some((_, scene_y)) = plot.pos_for(1000.0, db as f64) else {
                continue;
            };
            let y = plot_top + scene_y as i32;
            if y < plot_top || y >= plot_bottom {
                continue;
            }
            fill(hdc, plot_left, y, plot_right, y + 1, if db == 0 { BORDER } else { GRID });
            draw_text(
                hdc,
                plot_left + scale(2, self.dpi),
                y + scale(1, self.dpi),
                &wide(&format!("{db}")),
                MUTED,
            );
        }
        for step in [1u32, 2, 5, 10, 20, 50, 100, 200, 500, 1000, 2000, 5000, 10000, 20000] {
            let x_scene = plot.hz_to_x(step as f64);
            if x_scene < 0.0 {
                continue;
            }
            let x = plot_left + x_scene as i32 - plot.scroll_x as i32;
            if x < plot_left || x >= plot_right {
                continue;
            }
            fill(hdc, x, plot_top, x + 1, plot_bottom, GRID);
            draw_text(
                hdc,
                x + scale(2, self.dpi),
                plot_bottom - scale(14, self.dpi),
                &wide(&format!("{step:.0}")),
                MUTED,
            );
        }

        // Crosshair and readout under the cursor.
        if let Some((hover_x, hover_y)) = self.plot_hover {
            if hover_x >= plot_left && hover_x < plot_right && hover_y >= plot_top && hover_y < plot_bottom {
                fill(hdc, hover_x, plot_top, hover_x + 1, plot_bottom, BORDER);
                fill(hdc, plot_left, hover_y, plot_right, hover_y + 1, BORDER);
                let hz = plot.freq_at((hover_x - plot_left) as f32);
                let db = plot.db_at((hover_y - plot_top) as f32);
                let label = if hz >= 1000.0 {
                    format!("{:.2} kHz, {:.1} dB", hz / 1000.0, db)
                } else {
                    format!("{:.0} Hz, {:.1} dB", hz, db)
                };
                let text = wide(&label);
                let text_width = scale(110, self.dpi);
                let label_x = (hover_x + scale(6, self.dpi)).min(plot_right - text_width);
                let label_y = plot_top + scale(2, self.dpi);
                fill(
                    hdc,
                    label_x - scale(2, self.dpi),
                    label_y - scale(1, self.dpi),
                    label_x + text_width,
                    label_y + scale(16, self.dpi),
                    PLOT_BG,
                );
                draw_text(hdc, label_x, label_y, &text, TEXT);
            }
        }

        let Some(result) = &self.analysis_result else {
            return;
        };
        // The curve only depends on the FFT data, the plot geometry and the
        // view transform.  Hovering only moves the crosshair, so rebuilding
        // every run on each mouse move was wasted work.
        let curve_key = (
            self.analysis_generation,
            plot_left,
            plot_top,
            plot_right,
            plot_bottom,
            plot.scroll_x.to_bits(),
            plot.scroll_y.to_bits(),
            plot.zoom_x.to_bits(),
            plot.zoom_y.to_bits(),
        );
        let rebuild = self
            .analysis_curve
            .as_ref()
            .is_none_or(|curve| curve.key != curve_key);
        if rebuild {
            let sample_rate = result.freq_data_sample_rate.max(1) as f64;
            let bins = result.freq_data.len().max(1);
            // 0 dB reference in view coordinates (pos_for already subtracts
            // the scroll offset, unlike the raw scene-space db_to_y).
            let zero_y = plot
                .pos_for(1000.0, 0.0)
                .map(|(_, view_y)| plot_top + view_y as i32)
                .unwrap_or(plot_bottom);
            let mut runs: Vec<(COLORREF, Vec<PointF>)> = Vec::new();
            let mut fills: Vec<(i32, i32, i32, i32)> = Vec::new();
            let mut run_points: Vec<PointF> = Vec::new();
            let mut run_positive = false;
            let mut path_open = false;
            for px in plot_left..plot_right {
                let hz = plot.freq_at((px - plot_left) as f32);
                if hz <= 0.0 {
                    continue;
                }
                // Linear interpolation between FFT bins: nearest-bin lookup
                // made the curve piecewise constant ("staircase") at lower
                // resolutions.
                let position = (hz / sample_rate) * result.freq_data_length as f64;
                let lower = position.floor().max(0.0) as usize;
                let upper = (lower + 1).min(bins - 1);
                let fraction = (position - lower as f64).clamp(0.0, 1.0);
                let magnitude_at = |index: usize| {
                    let value = result.freq_data[index.min(bins - 1)];
                    (value.re * value.re + value.im * value.im).sqrt()
                };
                let magnitude =
                    magnitude_at(lower) * (1.0 - fraction) + magnitude_at(upper) * fraction;
                let db = (20.0 * magnitude.log10()).clamp(-100.0, 100.0);
                let Some((_, scene_y)) = plot.pos_for(hz, db) else {
                    if path_open && run_points.len() >= 2 {
                        runs.push((
                            if run_positive { CURVE_POS } else { CURVE },
                            std::mem::take(&mut run_points),
                        ));
                    }
                    run_points.clear();
                    path_open = false;
                    continue;
                };
                // Keep the sub-pixel Y for the curve; rounding to integer
                // pixels here produced the visible staircase even with
                // anti-aliasing.
                let y_subpixel = plot_top as f32 + scene_y;
                let y = y_subpixel as i32;
                let positive = db > 0.0;
                if positive && y < zero_y {
                    fills.push((px, y, px + 1, zero_y));
                }
                if path_open && positive != run_positive {
                    if run_points.len() >= 2 {
                        runs.push((
                            if run_positive { CURVE_POS } else { CURVE },
                            std::mem::take(&mut run_points),
                        ));
                    }
                    run_points.clear();
                }
                run_positive = positive;
                run_points.push(PointF {
                    X: px as f32 + 0.5,
                    Y: y_subpixel.clamp(
                        plot_top as f32,
                        plot_bottom.saturating_sub(1).max(plot_top) as f32,
                    ),
                });
                path_open = true;
            }
            if path_open && run_points.len() >= 2 {
                runs.push((if run_positive { CURVE_POS } else { CURVE }, run_points));
            }
            self.analysis_curve = Some(AnalysisCurve {
                key: curve_key,
                runs,
                fills,
            });
        }
        let saved_dc = SaveDC(hdc);
        let _ = IntersectClipRect(hdc, plot_left, plot_top, plot_right, plot_bottom);
        if let Some(curve) = &self.analysis_curve {
            for &(left, top, right, bottom) in &curve.fills {
                fill(hdc, left, top, right, bottom, FILL_POS);
            }
            draw_aa_runs(hdc, self.paint_offset, &curve.runs, self.plot_rect);
        }
        let _ = RestoreDC(hdc, saved_dc);
    }
}
