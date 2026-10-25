//! Port of `HotkeyEditDialog`.

use egui::{Key, RichText};

use crate::keyseq::{KeyCode, KeySequence, Modifiers};

#[derive(Default)]
pub struct HotkeyDialog {
    pub open: bool,
    pub sequence: Option<KeySequence>,
    pub accepted: bool,
    pub cancelled: bool,
    pub target_filter_id: Option<String>,
}

impl HotkeyDialog {
    pub fn begin(&mut self, filter_id: String, current: Option<KeySequence>) {
        self.open = true;
        self.sequence = current;
        self.accepted = false;
        self.cancelled = false;
        self.target_filter_id = Some(filter_id);
    }

    pub fn show(&mut self, ctx: &egui::Context) -> Option<Option<KeySequence>> {
        if !self.open {
            return None;
        }

        // Capture key presses before any other egui shortcut sees them.
        let events = ctx.input(|i| i.events.clone());
        for event in events {
            if let egui::Event::Key { key, pressed: true, repeat: false, modifiers, .. } = event {
                if key == Key::Escape {
                    self.open = false;
                    self.cancelled = true;
                    break;
                }
                if let Some(code) = map_key(key) {
                    let mut mods = Modifiers {
                        ctrl: modifiers.ctrl,
                        alt: modifiers.alt,
                        shift: modifiers.shift,
                        meta: win_key_down(),
                    };
                    if !mods.ctrl && !mods.alt && !mods.shift && !mods.meta {
                        // A bare key is a valid QKeySequence.
                    }
                    if let Some(hk) = sequence_from(code, &mut mods) {
                        self.sequence = Some(hk);
                    }
                }
            }
        }

        let mut open = self.open;
        let mut result = None;
        egui::Window::new("Set Hotkey")
            .collapsible(false)
            .resizable(false)
            .default_width(400.0)
            .open(&mut open)
            .show(ctx, |ui| {
                ui.vertical_centered(|ui| {
                    ui.label("Press the key combination you want to use as a hotkey:");
                    ui.add_space(12.0);
                    let (text, color) = match self.sequence {
                        Some(seq) => (seq.to_string_qt(), egui::Color32::from_rgb(0, 120, 0)),
                        None => ("Press a key combination...".to_string(), egui::Color32::GRAY),
                    };
                    let label = RichText::new(text).size(16.0).color(color);
                    egui::Frame::group(ui.style()).show(ui, |ui| {
                        ui.set_min_width(320.0);
                        ui.vertical_centered(|ui| {
                            ui.add_space(10.0);
                            ui.label(label);
                            ui.add_space(10.0);
                        });
                    });
                    ui.add_space(12.0);
                    ui.horizontal(|ui| {
                        if ui.button("Clear").clicked() {
                            self.sequence = None;
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("Cancel").clicked() {
                                self.open = false;
                                self.cancelled = true;
                            }
                            if ui.button("OK").clicked() {
                                self.open = false;
                                self.accepted = true;
                                result = Some(self.sequence);
                            }
                        });
                    });
                });
            });
        if !open && self.open {
            self.open = false;
            self.cancelled = true;
        }
        if self.cancelled {
            return None;
        }
        result
    }
}

fn sequence_from(code: KeyCode, mods: &mut Modifiers) -> Option<KeySequence> {
    Some(KeySequence::new(*mods, code))
}

fn win_key_down() -> bool {
    unsafe {
        let l = windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x5B);
        let r = windows::Win32::UI::Input::KeyboardAndMouse::GetAsyncKeyState(0x5C);
        (l as u16 & 0x8000) != 0 || (r as u16 & 0x8000) != 0
    }
}

fn map_key(key: Key) -> Option<KeyCode> {
    Some(match key {
        Key::A => KeyCode::A,
        Key::B => KeyCode::B,
        Key::C => KeyCode::C,
        Key::D => KeyCode::D,
        Key::E => KeyCode::E,
        Key::F => KeyCode::F,
        Key::G => KeyCode::G,
        Key::H => KeyCode::H,
        Key::I => KeyCode::I,
        Key::J => KeyCode::J,
        Key::K => KeyCode::K,
        Key::L => KeyCode::L,
        Key::M => KeyCode::M,
        Key::N => KeyCode::N,
        Key::O => KeyCode::O,
        Key::P => KeyCode::P,
        Key::Q => KeyCode::Q,
        Key::R => KeyCode::R,
        Key::S => KeyCode::S,
        Key::T => KeyCode::T,
        Key::U => KeyCode::U,
        Key::V => KeyCode::V,
        Key::W => KeyCode::W,
        Key::X => KeyCode::X,
        Key::Y => KeyCode::Y,
        Key::Z => KeyCode::Z,
        Key::Num0 => KeyCode::Digit(0),
        Key::Num1 => KeyCode::Digit(1),
        Key::Num2 => KeyCode::Digit(2),
        Key::Num3 => KeyCode::Digit(3),
        Key::Num4 => KeyCode::Digit(4),
        Key::Num5 => KeyCode::Digit(5),
        Key::Num6 => KeyCode::Digit(6),
        Key::Num7 => KeyCode::Digit(7),
        Key::Num8 => KeyCode::Digit(8),
        Key::Num9 => KeyCode::Digit(9),
        Key::F1 => KeyCode::Function(1),
        Key::F2 => KeyCode::Function(2),
        Key::F3 => KeyCode::Function(3),
        Key::F4 => KeyCode::Function(4),
        Key::F5 => KeyCode::Function(5),
        Key::F6 => KeyCode::Function(6),
        Key::F7 => KeyCode::Function(7),
        Key::F8 => KeyCode::Function(8),
        Key::F9 => KeyCode::Function(9),
        Key::F10 => KeyCode::Function(10),
        Key::F11 => KeyCode::Function(11),
        Key::F12 => KeyCode::Function(12),
        Key::F13 => KeyCode::Function(13),
        Key::F14 => KeyCode::Function(14),
        Key::F15 => KeyCode::Function(15),
        Key::F16 => KeyCode::Function(16),
        Key::F17 => KeyCode::Function(17),
        Key::F18 => KeyCode::Function(18),
        Key::F19 => KeyCode::Function(19),
        Key::F20 => KeyCode::Function(20),
        Key::F21 => KeyCode::Function(21),
        Key::F22 => KeyCode::Function(22),
        Key::F23 => KeyCode::Function(23),
        Key::F24 => KeyCode::Function(24),
        Key::Space => KeyCode::Space,
        Key::Enter => KeyCode::Return,
        Key::Tab => KeyCode::Tab,
        Key::Backspace => KeyCode::Backspace,
        Key::Insert => KeyCode::Insert,
        Key::Delete => KeyCode::Delete,
        Key::Home => KeyCode::Home,
        Key::End => KeyCode::End,
        Key::PageUp => KeyCode::PageUp,
        Key::PageDown => KeyCode::PageDown,
        Key::ArrowLeft => KeyCode::Left,
        Key::ArrowRight => KeyCode::Right,
        Key::ArrowUp => KeyCode::Up,
        Key::ArrowDown => KeyCode::Down,
        _ => return None,
    })
}
