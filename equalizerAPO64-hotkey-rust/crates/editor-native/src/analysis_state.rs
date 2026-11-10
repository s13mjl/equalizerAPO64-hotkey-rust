//! Analysis scheduling and result polling for the native editor.

use super::*;

impl App {
    pub(super) fn start_analysis(&mut self) {
        if !self.show_analysis {
            return;
        }
        let path = if self.analysis_start_from == 1 {
            self.doc_path.clone()
        } else {
            Some(settings::config_dir_from_registry().join("config.txt"))
        };
        let Some(path) = path else {
            self.analysis_result = None;
            return;
        };
        let mut resolution = self.analysis_resolution.max(128);
        const MEMORY_LIMIT: i32 = 1_048_576;
        while resolution > MEMORY_LIMIT {
            resolution /= 2;
        }
        self.analysis_effective_resolution = resolution;
        self.analysis_generation = self.analysis_generation.wrapping_add(1);
        let (device_name, connection_name, device_guid, device_string, channel_count, sample_rate, device_mask) =
            if let Some(device) = &self.selected_device {
                (
                    device.get_device_name(),
                    device.get_connection_name(),
                    device.get_device_guid(),
                    device.get_device_string(),
                    device.get_channel_count(),
                    device.get_sample_rate(),
                    device.get_channel_mask(),
                )
            } else {
                (
                    "Benchmark".to_string(),
                    "File output".to_string(),
                    String::new(),
                    String::new(),
                    2,
                    48000,
                    3,
                )
            };
        let channel_mask = if self.channel_mask != 0 {
            self.channel_mask
        } else {
            device_mask
        };
        self.analysis.set_parameters(AnalysisRequest {
            device: DeviceSnapshot {
                capture: false,
                device_name,
                connection_name,
                device_guid,
                device_string,
                channel_count,
                sample_rate,
                channel_mask: device_mask,
            },
            channel_mask,
            channel_index: self.analysis_channel_index.min(channel_count.saturating_sub(1) as usize),
            config_path: path.to_string_lossy().into_owned(),
            frame_count: resolution as usize,
            generation: self.analysis_generation,
        });
    }

    pub(super) fn poll_analysis(&mut self, hwnd: HWND) -> bool {
        let mut updated = false;
        while let Some(result) = self.analysis.try_recv() {
            if result.generation == self.analysis_generation {
                self.analysis_result = Some(result);
                updated = true;
            }
        }
        if updated {
            unsafe {
                self.invalidate_analysis(hwnd);
            }
        }
        updated
    }

    /// Repaints only the analysis dock; a new result does not affect the table.
    pub(super) unsafe fn invalidate_analysis(&self, hwnd: HWND) {
        let mut client = RECT::default();
        let _ = GetClientRect(hwnd, &mut client);
        let top = client.bottom - self.analysis_height();
        if self.show_analysis && top >= 0 {
            let rect = RECT {
                left: 0,
                top,
                right: client.right,
                bottom: client.bottom,
            };
            let _ = InvalidateRect(Some(hwnd), Some(&rect), false);
        } else {
            let _ = InvalidateRect(Some(hwnd), None, false);
        }
    }

    pub(super) fn poll_volume(&mut self, hwnd: HWND) {
        // Recompute the loudness curve only after the volume stopped changing,
        // otherwise dragging the system slider restarts the analysis on every
        // poll.
        if let Some(due) = self.volume_analysis_due {
            if std::time::Instant::now() >= due {
                self.volume_analysis_due = None;
                self.start_analysis();
            }
        }
        if !self.rows.iter().any(|row| row.kind == FilterKind::Loudness) {
            return;
        }
        if self.volume_controller.is_none() {
            self.volume_controller = Some(VolumeController::new());
        }
        let mut volume = 0.0;
        let Some(controller) = &self.volume_controller else {
            return;
        };
        if controller.get_volume(&mut volume).ok().is_err() {
            return;
        }
        if self.master_volume == Some(volume) {
            return;
        }
        self.master_volume = Some(volume);
        let mut changed_rows = Vec::new();
        for (index, row) in self.rows.iter_mut().enumerate() {
            if row.kind != FilterKind::Loudness {
                continue;
            }
            if let Some(item) = row.item.as_mut() {
                if let Some(FilterEditor::Loudness {
                    volume: shown, ..
                }) = item.editor.as_mut()
                {
                    if *shown != volume {
                        *shown = volume;
                        changed_rows.push(index);
                    }
                }
            }
        }
        if !changed_rows.is_empty() {
            unsafe {
                for index in changed_rows {
                    self.invalidate_row(hwnd, index);
                }
            }
        }
        self.volume_analysis_due = Some(
            std::time::Instant::now() + std::time::Duration::from_millis(300),
        );
    }

}
