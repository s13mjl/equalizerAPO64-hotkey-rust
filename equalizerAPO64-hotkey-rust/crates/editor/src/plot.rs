//! Port of `FrequencyPlotScene`/`FrequencyPlotView`/rulers and the
//! `GraphicEQFilterGUI` interactions to egui.

use eapo_common::gain_iterator::{FilterNode, GainIterator};
use egui::{Color32, Pos2, Rect, Sense, Stroke, Vec2};

use crate::model::{FilterNode as ModelNode, GraphicEqEditor};

pub const MAX_X: f64 = 1000.0;
pub const MAX_Y: f64 = 500.0;
pub const MIN_HZ: f64 = 1.0;
pub const MAX_HZ: f64 = 30000.0;
pub const MIN_DB: f64 = -100.0;
pub const MAX_DB: f64 = 100.0;
pub const LEFT_MARGIN: f32 = 32.0;
pub const BOTTOM_MARGIN: f32 = 20.0;

#[derive(Clone, Debug)]
pub struct PlotState {
    pub zoom_x: f64,
    pub zoom_y: f64,
    pub scroll: Vec2,
}

impl Default for PlotState {
    fn default() -> Self {
        let mut s = Self { zoom_x: 1.0, zoom_y: 1.0, scroll: Vec2::ZERO };
        s.scroll = Vec2::new(s.hz_to_x(20.0) as f32, s.db_to_y(22.0) as f32);
        s
    }
}

impl PlotState {
    pub fn scene_size(&self) -> Vec2 {
        Vec2::new((MAX_X * self.zoom_x) as f32, (MAX_Y * self.zoom_y) as f32)
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

    pub fn fit(&mut self, plot_size: Vec2) {
        self.zoom_x = (MAX_X as f32 / plot_size.x.max(1.0)) as f64;
        self.zoom_y = (MAX_Y as f32 / plot_size.y.max(1.0)) as f64;
        self.scroll = Vec2::ZERO;
    }

    fn clamp_scroll(&mut self, view: Vec2) {
        let scene = self.scene_size();
        self.scroll.x = self.scroll.x.clamp(0.0, (scene.x - view.x).max(0.0));
        self.scroll.y = self.scroll.y.clamp(0.0, (scene.y - view.y).max(0.0));
    }

    fn zoom_at(&mut self, delta: f32, local: Vec2, plot_size: Vec2) {
        if delta == 0.0 {
            return;
        }
        let scene_x = self.scroll.x + local.x;
        let scene_y = self.scroll.y + local.y;
        let hz = self.x_to_hz(scene_x as f64);
        let db = self.y_to_db(scene_y as f64);
        let factor = 1.001f64.powf(delta as f64 * 2.4);
        self.zoom_x = (self.zoom_x * factor).clamp(0.5, 30.0);
        self.zoom_y = (self.zoom_y * factor).clamp(0.5, 30.0);
        self.scroll.x = self.hz_to_x(hz) as f32 - local.x;
        self.scroll.y = self.db_to_y(db) as f32 - local.y;
        self.clamp_scroll(plot_size);
    }

    fn pan(&mut self, delta: Vec2, plot_size: Vec2) {
        self.scroll -= delta;
        self.clamp_scroll(plot_size);
    }

    pub fn freq_at(&self, plot: Rect, local: Pos2) -> f64 {
        self.x_to_hz((self.scroll.x + local.x - plot.left()) as f64)
    }

    pub fn db_at(&self, plot: Rect, local: Pos2) -> f64 {
        self.y_to_db((self.scroll.y + local.y - plot.top()) as f64)
    }

    pub fn pos_for(&self, plot: Rect, hz: f64, db: f64) -> Option<Pos2> {
        let x = self.hz_to_x(hz);
        let y = self.db_to_y(db);
        if x < 0.0 || y < 0.0 {
            return None;
        }
        Some(Pos2::new(
            plot.left() + x as f32 - self.scroll.x,
            plot.top() + y as f32 - self.scroll.y,
        ))
    }

    pub fn default_scroll(&mut self) {
        self.scroll = Vec2::new(self.hz_to_x(20.0) as f32, self.db_to_y(22.0) as f32);
    }
}

#[derive(Clone, Copy, Debug)]
pub struct PlotOptions {
    pub height: f32,
    pub fill_red_above_zero: bool,
    pub show_curve: bool,
}

impl Default for PlotOptions {
    fn default() -> Self {
        Self { height: 150.0, fill_red_above_zero: false, show_curve: true }
    }
}

#[derive(Clone, Debug, Default)]
pub struct PlotResponse {
    pub changed: bool,
    pub view_changed: bool,
}

pub struct PlotCanvas {
    pub response: egui::Response,
    pub plot_rect: Rect,
    pub hover: Option<Pos2>,
}

fn db_grid_step(state: &PlotState) -> f64 {
    let step = (state.y_to_db(0.0) - state.y_to_db(30.0)).abs();
    let base = 10f64.powf(step.log10().floor());
    if step >= 5.0 * base {
        5.0 * base
    } else if step >= 2.0 * base {
        2.0 * base
    } else {
        base
    }
}

fn draw_grid_and_rulers(
    ui: &mut egui::Ui,
    state: &PlotState,
    plot: Rect,
    mode_bands: &[f64],
    hover: Option<Pos2>,
) {
    let visuals = ui.visuals();
    let dark = visuals.dark_mode;
    let grid_color = if dark { Color32::from_rgb(90, 90, 90) } else { Color32::from_rgb(200, 200, 200) };
    let text_color = if dark { Color32::from_rgb(200, 200, 200) } else { Color32::from_rgb(50, 50, 50) };
    let painter = ui.painter();

    // dB lines / labels
    let db_step = db_grid_step(state);
    let from_scene_y = state.scroll.y as f64;
    let to_scene_y = (state.scroll.y + plot.height()) as f64;
    let from_db = (state.y_to_db(to_scene_y) / db_step).floor() * db_step;
    let to_db = (state.y_to_db(from_scene_y) / db_step).ceil() * db_step;
    let mut db = from_db;
    while db <= to_db + 1e-9 {
        let y = state.db_to_y(db);
        if y >= 0.0 {
            let sy = plot.top() + y as f32 - state.scroll.y;
            if sy >= plot.top() - 1.0 && sy <= plot.bottom() + 1.0 {
                painter.line_segment(
                    [Pos2::new(plot.left(), sy), Pos2::new(plot.right(), sy)],
                    Stroke::new(1.0, grid_color),
                );
                let label = if db.abs() < 1e-6 { "0".to_string() } else { format!("{db}") };
                painter.text(
                    Pos2::new(plot.left() - 2.0, sy),
                    egui::Align2::RIGHT_CENTER,
                    label,
                    egui::FontId::proportional(11.0),
                    text_color,
                );
            }
        }
        db += db_step;
    }

    // Frequency lines / labels
    let from_hz = state.x_to_hz(state.scroll.x as f64);
    let to_hz = state.x_to_hz((state.scroll.x + plot.width()) as f64);
    if mode_bands.is_empty() {
        let mut hz_base = 10f64.powf(from_hz.log10().floor());
        let mut hz = (from_hz / hz_base).floor() * hz_base;
        hz += hz_base;
        if (hz / hz_base).round() >= 10.0 {
            hz_base *= 10.0;
        }
        while hz <= to_hz {
            draw_freq_line(painter, state, plot, hz, grid_color, text_color);
            hz += hz_base;
            if (hz / hz_base).round() >= 10.0 {
                hz_base *= 10.0;
            }
        }
    } else {
        for &hz in mode_bands.iter().filter(|&&h| h >= from_hz && h <= to_hz) {
            draw_freq_line(painter, state, plot, hz, grid_color, text_color);
        }
    }

    // Hover read-outs in the ruler boxes, exactly like the C++ rulers.
    if let Some(h) = hover {
        if plot.contains(h) {
            let hz = state.freq_at(plot, h);
            let db = state.db_at(plot, h);
            let text = format!("{hz:.1}");
            let galley = painter.layout_no_wrap(
                text,
                egui::FontId::proportional(11.0),
                if dark { Color32::from_rgb(147, 161, 229) } else { Color32::BLUE },
            );
            let cx = h.x.clamp(plot.left() + 12.0, plot.right() - 12.0);
            let rect = Rect::from_center_size(
                Pos2::new(cx, plot.bottom() + BOTTOM_MARGIN * 0.5),
                galley.size() + Vec2::new(8.0, 2.0),
            );
            painter.rect_filled(rect, 1.0, if dark { Color32::BLACK } else { Color32::WHITE });
            painter.rect_stroke(rect, 1.0, Stroke::new(1.0, text_color), egui::StrokeKind::Inside);
            painter.galley(rect.center() - galley.size() * 0.5, galley, text_color);

            let dtext = format!("{db:.1}");
            let dgalley = painter.layout_no_wrap(
                dtext,
                egui::FontId::proportional(11.0),
                if dark { Color32::from_rgb(147, 161, 229) } else { Color32::BLUE },
            );
            let cy = h.y.clamp(plot.top() + 10.0, plot.bottom() - 10.0);
            let drect = Rect::from_center_size(
                Pos2::new(plot.left() - LEFT_MARGIN * 0.5, cy),
                dgalley.size() + Vec2::new(6.0, 2.0),
            );
            painter.rect_filled(drect, 1.0, if dark { Color32::BLACK } else { Color32::WHITE });
            painter.rect_stroke(drect, 1.0, Stroke::new(1.0, text_color), egui::StrokeKind::Inside);
            painter.galley(drect.center() - dgalley.size() * 0.5, dgalley, text_color);
        }
    }
}

fn draw_freq_line(
    painter: &egui::Painter,
    state: &PlotState,
    plot: Rect,
    hz: f64,
    grid_color: Color32,
    text_color: Color32,
) {
    let x = state.hz_to_x(hz);
    if x < 0.0 {
        return;
    }
    let sx = plot.left() + x as f32 - state.scroll.x;
    if sx < plot.left() - 1.0 || sx > plot.right() + 1.0 {
        return;
    }
    painter.line_segment(
        [Pos2::new(sx, plot.top()), Pos2::new(sx, plot.bottom())],
        Stroke::new(1.0, grid_color),
    );
    let label = if hz < 1000.0 {
        format!("{hz}")
    } else {
        let k = hz / 1000.0;
        if (k - k.round()).abs() < 1e-9 { format!("{}k", k.round() as i64) } else { format!("{k}") }
    };
    painter.text(
        Pos2::new(sx + 1.0, plot.bottom() + 2.0),
        egui::Align2::LEFT_TOP,
        label,
        egui::FontId::proportional(10.0),
        text_color,
    );
}

fn paint_curve(
    ui: &egui::Ui,
    state: &PlotState,
    plot: Rect,
    nodes: &[FilterNode],
    opts: PlotOptions,
    bands: &[f64],
) {
    if !opts.show_curve || nodes.is_empty() {
        return;
    }
    let mut iter = GainIterator::new(nodes.to_vec());
    let painter = ui.painter().with_clip_rect(plot);
    let dark = ui.visuals().dark_mode;
    let line = if dark { Color32::WHITE } else { Color32::BLACK };
    let mut points: Vec<Pos2> = Vec::with_capacity(plot.width().max(1.0) as usize + 2);
    let mut db_values: Vec<f64> = Vec::with_capacity(plot.width().max(1.0) as usize + 2);
    let mut x = plot.left();
    while x <= plot.right() + 0.5 {
        let scene_x = state.scroll.x + (x - plot.left());
        let hz = state.x_to_hz(scene_x as f64);
        let db = iter.gain_at(hz);
        db_values.push(db);
        let y = if db > MAX_DB {
            plot.top() - 1.0
        } else if db < MIN_DB {
            plot.bottom() + 1.0
        } else {
            plot.top() + state.db_to_y(db) as f32 - state.scroll.y
        };
        points.push(Pos2::new(x, y));
        x += 1.0;
    }
    if opts.fill_red_above_zero {
        let zero_y = plot.top() + state.db_to_y(0.0) as f32 - state.scroll.y;
        for (i, w) in points.windows(2).enumerate() {
            let y = w[0].y.min(w[1].y);
            if y < zero_y {
                let r = Rect::from_min_max(
                    Pos2::new(w[0].x, y),
                    Pos2::new(w[1].x + 1.0, zero_y),
                );
                painter.rect_filled(r, 0.0, Color32::RED);
            }
            let _ = i;
        }
    }
    painter.add(egui::Shape::line(points.clone(), Stroke::new(1.5, line)));
    let _ = bands;
    let _ = db_values;
}

/// Allocates and draws a frequency plot.  Returns the response and geometry
/// so callers can implement their own node interaction.
pub fn plot_frame(
    ui: &mut egui::Ui,
    state: &mut PlotState,
    height: f32,
    bands: &[f64],
    nodes: &[FilterNode],
    opts: PlotOptions,
) -> PlotCanvas {
    let width = ui.available_width().max(80.0);
    let (response, _painter) =
        ui.allocate_painter(Vec2::new(width, height), Sense::click_and_drag());
    ui.ctx().data_mut(|d| d.insert_temp(egui::Id::new("eapo_plot_hover"), response.hovered()));
    let full = response.rect;
    let plot = Rect::from_min_max(
        Pos2::new(full.left() + LEFT_MARGIN, full.top()),
        Pos2::new(full.right(), full.bottom() - BOTTOM_MARGIN),
    );
    ui.painter().rect_filled(full, 0.0, ui.visuals().extreme_bg_color);

    let mut hover = response.hover_pos();
    if !plot.contains(hover.unwrap_or(Pos2::ZERO)) {
        hover = Some(Pos2::new(
            hover.unwrap_or(plot.center()).x.clamp(plot.left(), plot.right()),
            hover.unwrap_or(plot.center()).y.clamp(plot.top(), plot.bottom()),
        ));
    }

    // Anchor-preserving wheel zoom.
    if response.hovered() {
        let scroll = ui.input(|i| i.raw_scroll_delta.y);
        if scroll != 0.0 {
            if let Some(h) = response.hover_pos() {
                let local = Vec2::new(h.x - plot.left(), h.y - plot.top());
                state.zoom_at(scroll, local, plot.size());
            }
        }
    }

    // Right button / middle button panning, equivalent to FrequencyPlotView.
    if response.dragged_by(egui::PointerButton::Secondary)
        || response.dragged_by(egui::PointerButton::Middle)
    {
        state.pan(response.drag_delta(), plot.size());
    }

    draw_grid_and_rulers(ui, state, plot, bands, hover);
    paint_curve(ui, state, plot, nodes, opts, bands);
    let painter2 = ui.painter();
    painter2.rect_stroke(
        plot,
        0.0,
        Stroke::new(1.0, ui.visuals().widgets.noninteractive.bg_stroke.color),
        egui::StrokeKind::Inside,
    );

    PlotCanvas { response, plot_rect: plot, hover }
}

#[derive(Clone, Debug, Default)]
pub struct GraphicEqResult {
    pub changed: bool,
    pub view_changed: bool,
    pub clicked_node: Option<usize>,
}

/// Interactive graphic EQ canvas: drag nodes, double-click to add, Delete to
/// remove, arrow keys to nudge.  Matches `GraphicEQFilterGUIScene`.
pub fn show_graphic_eq(ui: &mut egui::Ui, ed: &mut GraphicEqEditor) -> GraphicEqResult {
    let height = ed.view_height.max(60.0);
    let nodes: Vec<FilterNode> = ed.nodes.iter().map(|n| FilterNode { freq: n.freq, db_gain: n.db_gain }).collect();
    let bands: Vec<f64> = GraphicEqEditor::bands(ed.band_count).to_vec();
    let mut state = PlotState { zoom_x: ed.zoom_x, zoom_y: ed.zoom_y, scroll: ed.scroll };
    let canvas = plot_frame(
        ui,
        &mut state,
        height,
        &bands,
        &nodes,
        PlotOptions { height, fill_red_above_zero: false, show_curve: true },
    );
    ed.zoom_x = state.zoom_x;
    ed.zoom_y = state.zoom_y;
    ed.scroll = state.scroll;
    let mut result = GraphicEqResult::default();
    let response = canvas.response.clone();
    let plot = canvas.plot_rect;

    let node_pos = |ed: &GraphicEqEditor, state: &PlotState, i: usize| -> Option<Pos2> {
        state.pos_for(plot, ed.nodes[i].freq, ed.nodes[i].db_gain)
    };

    let mut nearest = None;
    if let Some(h) = response.hover_pos() {
        let mut best = 8.0;
        for (i, _) in ed.nodes.iter().enumerate() {
            if let Some(p) = node_pos(ed, &state, i) {
                let d = p.distance(h);
                if d < best {
                    best = d;
                    nearest = Some(i);
                }
            }
        }
    }

    if response.clicked() {
        let shift = ui.input(|i| i.modifiers.shift);
        if let Some(i) = nearest {
            if shift {
                if let Some(pos) = ed.selected.iter().position(|&x| x == i) {
                    ed.selected.remove(pos);
                } else {
                    ed.selected.push(i);
                }
            } else {
                ed.selected = vec![i];
            }
            result.clicked_node = Some(i);
        } else if let Some(h) = canvas.hover {
            if plot.contains(h) {
                ed.selected.clear();
                result.clicked_node = None;
            }
        }
        ui.ctx().request_repaint();
    }

    if response.double_clicked() && ed.band_count == -1 {
        if let Some(h) = canvas.hover {
            if plot.contains(h) {
                let hz = state.freq_at(plot, h);
                let db = state.db_at(plot, h).clamp(MIN_DB, MAX_DB);
                ed.nodes.push(ModelNode { freq: hz, db_gain: db });
                ed.sort_nodes();
                // addNode() emits insertion and reindexes.
                let idx = ed.nodes.iter().position(|n| (n.freq - hz).abs() < 1e-12).unwrap_or(0);
                ed.selected = vec![idx];
                result.changed = true;
            }
        }
    }

    if response.dragged_by(egui::PointerButton::Primary) {
        if nearest.is_none() {
            // Keep the node grabbed at drag start if the pointer moves fast.
            if let Some(i) = ed.selected.first().copied() {
                if i < ed.nodes.len() {
                    nearest = Some(i);
                }
            }
        }
        if let (Some(i), Some(h)) = (nearest, response.interact_pointer_pos()) {
            if i < ed.nodes.len() && plot.contains(h) {
                let mut hz = ed.nodes[i].freq;
                if ed.band_count == -1 {
                    hz = state.freq_at(plot, h);
                    hz = if hz >= 10000.0 { hz.round() } else { (hz * 10.0).round() / 10.0 };
                }
                let mut db = state.db_at(plot, h);
                db = if state.zoom_y < 4.0 { (db * 10.0).round() / 10.0 } else { (db * 100.0).round() / 100.0 };
                ed.nodes[i] = ModelNode { freq: hz, db_gain: db };
                ed.sort_nodes();
                let new_idx = ed.nodes.iter().position(|n| (n.freq - hz).abs() < 1e-9 && (n.db_gain - db).abs() < 1e-9).unwrap_or(i);
                ed.selected = vec![new_idx];
                result.changed = true;
            }
        }
    }

    // Keyboard: Delete and arrow nudges when the canvas has focus/hover.
    let modal_open = ui
        .ctx()
        .data(|d| d.get_temp::<bool>(egui::Id::new("eapo_modal")).unwrap_or(false));
    if (response.hovered() || response.has_focus()) && !modal_open {
        let (del, up, down, left, right) = ui.input(|i| {
            (
                i.key_pressed(egui::Key::Delete),
                i.key_pressed(egui::Key::ArrowUp),
                i.key_pressed(egui::Key::ArrowDown),
                i.key_pressed(egui::Key::ArrowLeft),
                i.key_pressed(egui::Key::ArrowRight),
            )
        });
        if del && ed.band_count == -1 && !ed.selected.is_empty() {
            ui.ctx()
                .input_mut(|i| i.consume_key(egui::Modifiers::NONE, egui::Key::Delete));
            let mut sel = ed.selected.clone();
            sel.sort_unstable();
            sel.dedup();
            for i in sel.into_iter().rev() {
                if i < ed.nodes.len() {
                    ed.nodes.remove(i);
                }
            }
            ed.selected.clear();
            result.changed = true;
        }
        if up || down || left || right {
            if !ed.selected.is_empty() {
                ui.ctx()
                    .input_mut(|i| {
                        i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowUp);
                        i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowDown);
                        i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowLeft);
                        i.consume_key(egui::Modifiers::NONE, egui::Key::ArrowRight);
                    });
            }
            for &i in &ed.selected {
                if i >= ed.nodes.len() {
                    continue;
                }
                if up { ed.nodes[i].db_gain += 1.0; }
                if down { ed.nodes[i].db_gain -= 1.0; }
                if left { ed.nodes[i].freq -= 1.0; }
                if right { ed.nodes[i].freq += 1.0; }
            }
            ed.sort_nodes();
            result.changed = true;
        }
        if ui.ctx().input_mut(|i| i.consume_key(egui::Modifiers::CTRL, egui::Key::A)) {
            ed.selected = (0..ed.nodes.len()).collect();
            ui.ctx().request_repaint();
        }
    }

    // Draw nodes on top.
    let painter = ui.painter().with_clip_rect(plot);
    for (i, node) in ed.nodes.iter().enumerate() {
        if let Some(p) = state.pos_for(plot, node.freq, node.db_gain) {
            if !plot.expand(8.0).contains(p) {
                continue;
            }
            let selected = ed.selected.contains(&i);
            let fill = if selected {
                Color32::from_rgb(38, 147, 255)
            } else if ui.visuals().dark_mode {
                Color32::BLACK
            } else {
                Color32::WHITE
            };
            let stroke = if selected { Color32::WHITE } else { Color32::from_gray(120) };
            painter.circle(p, 5.0, fill, Stroke::new(1.0, stroke));
            if i < 99 {
                painter.text(
                    p,
                    egui::Align2::CENTER_CENTER,
                    format!("{}", i + 1),
                    egui::FontId::proportional(8.0),
                    if selected { Color32::WHITE } else { stroke },
                );
            }
        }
    }

    if response.hovered() || response.dragged() {
        ui.ctx().request_repaint();
    }
    result
}

/// Static analysis curve.  `nodes` are the FFT bins as dB gains.
pub fn show_analysis(ui: &mut egui::Ui, state: &mut PlotState, height: f32, nodes: &[FilterNode]) -> PlotResponse {
    let canvas = plot_frame(
        ui,
        state,
        height,
        &[],
        nodes,
        PlotOptions { height, fill_red_above_zero: true, show_curve: true },
    );
    let mut r = PlotResponse::default();
    if canvas.response.dragged_by(egui::PointerButton::Secondary) {
        r.view_changed = true;
    }
    if ui.input(|i| i.zoom_delta()) != 1.0 && canvas.response.hovered() {
        r.view_changed = true;
    }
    r
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn axis_mapping_round_trips() {
        let s = PlotState::default();
        for hz in [1.0, 20.0, 100.0, 1000.0, 20000.0, 30000.0] {
            let x = s.hz_to_x(hz);
            assert!(x >= 0.0);
            assert!((s.x_to_hz(x) - hz).abs() / hz < 1e-9);
        }
        for db in [-100.0, -20.0, 0.0, 22.0, 100.0] {
            let y = s.db_to_y(db);
            assert!(y >= 0.0);
            assert!((s.y_to_db(y) - db).abs() < 1e-9);
        }
    }

    #[test]
    fn inverse_formula_matches_cpp() {
        let s = PlotState { zoom_x: 1.0, zoom_y: 1.0, scroll: Vec2::ZERO };
        assert!((s.x_to_hz(1000.0) - 30000.0).abs() < 1e-6);
        assert!((s.y_to_db(500.0) + 100.0).abs() < 1e-9);
        assert!((s.db_to_y(-100.0) - 500.0).abs() < 1e-9);
    }
}
