//! Qt `QKeySequence` compatible parsing and formatting.
//!
//! Only one chord per sequence is supported, which is exactly what the fork's
//! `HotkeyManager` uses (`keySeq[0]`).  The textual representation matches
//! Qt's portable/native text on Windows: modifiers are emitted in the order
//! Meta, Ctrl, Alt, Shift and the key names are the ones Qt stores in the
//! registry (`Esc`, `Del`, `PgUp`, `Return`, ...).  Parsing is deliberately
//! more forgiving and accepts the long names as well.

use std::fmt;

pub const MOD_ALT: u32 = 0x0001;
pub const MOD_CONTROL: u32 = 0x0002;
pub const MOD_SHIFT: u32 = 0x0004;
pub const MOD_WIN: u32 = 0x0008;
pub const MOD_NOREPEAT: u32 = 0x4000;

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Default)]
pub struct Modifiers {
    pub ctrl: bool,
    pub alt: bool,
    pub shift: bool,
    pub meta: bool,
}

impl Modifiers {
    pub fn is_empty(self) -> bool {
        !(self.ctrl || self.alt || self.shift || self.meta)
    }

    fn set(&mut self, name: &str) -> bool {
        match name.to_ascii_lowercase().as_str() {
            "ctrl" | "control" => self.ctrl = true,
            "alt" => self.alt = true,
            "shift" => self.shift = true,
            "meta" | "win" | "super" | "cmd" | "command" => self.meta = true,
            _ => return false,
        }
        true
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum KeyCode {
    A,
    B,
    C,
    D,
    E,
    F,
    G,
    H,
    I,
    J,
    K,
    L,
    M,
    N,
    O,
    P,
    Q,
    R,
    S,
    T,
    U,
    V,
    W,
    X,
    Y,
    Z,
    Digit(u8), // 0..=9
    Function(u8), // 1..=24
    Space,
    Return,
    Escape,
    Tab,
    Backspace,
    Insert,
    Delete,
    Home,
    End,
    PageUp,
    PageDown,
    Left,
    Right,
    Up,
    Down,
    Print,
    ScrollLock,
    Pause,
}

impl KeyCode {
    /// Qt's portable/native key name (default `toString()`).
    pub fn name(self) -> String {
        match self {
            KeyCode::A => "A".into(),
            KeyCode::B => "B".into(),
            KeyCode::C => "C".into(),
            KeyCode::D => "D".into(),
            KeyCode::E => "E".into(),
            KeyCode::F => "F".into(),
            KeyCode::G => "G".into(),
            KeyCode::H => "H".into(),
            KeyCode::I => "I".into(),
            KeyCode::J => "J".into(),
            KeyCode::K => "K".into(),
            KeyCode::L => "L".into(),
            KeyCode::M => "M".into(),
            KeyCode::N => "N".into(),
            KeyCode::O => "O".into(),
            KeyCode::P => "P".into(),
            KeyCode::Q => "Q".into(),
            KeyCode::R => "R".into(),
            KeyCode::S => "S".into(),
            KeyCode::T => "T".into(),
            KeyCode::U => "U".into(),
            KeyCode::V => "V".into(),
            KeyCode::W => "W".into(),
            KeyCode::X => "X".into(),
            KeyCode::Y => "Y".into(),
            KeyCode::Z => "Z".into(),
            KeyCode::Digit(d) => d.to_string(),
            KeyCode::Function(n) => format!("F{n}"),
            KeyCode::Space => "Space".into(),
            KeyCode::Return => "Return".into(),
            KeyCode::Escape => "Esc".into(),
            KeyCode::Tab => "Tab".into(),
            KeyCode::Backspace => "Backspace".into(),
            KeyCode::Insert => "Ins".into(),
            KeyCode::Delete => "Del".into(),
            KeyCode::Home => "Home".into(),
            KeyCode::End => "End".into(),
            KeyCode::PageUp => "PgUp".into(),
            KeyCode::PageDown => "PgDown".into(),
            KeyCode::Left => "Left".into(),
            KeyCode::Right => "Right".into(),
            KeyCode::Up => "Up".into(),
            KeyCode::Down => "Down".into(),
            KeyCode::Print => "Print".into(),
            KeyCode::ScrollLock => "ScrollLock".into(),
            KeyCode::Pause => "Pause".into(),
        }
    }

    /// Human readable name used by the hotkey dialog (Qt native text is the
    /// same on Windows for all keys we support).
    pub fn display_name(self) -> String {
        self.name()
    }

    pub fn to_vk(self) -> u32 {
        match self {
            KeyCode::A => b'A' as u32,
            KeyCode::B => b'B' as u32,
            KeyCode::C => b'C' as u32,
            KeyCode::D => b'D' as u32,
            KeyCode::E => b'E' as u32,
            KeyCode::F => b'F' as u32,
            KeyCode::G => b'G' as u32,
            KeyCode::H => b'H' as u32,
            KeyCode::I => b'I' as u32,
            KeyCode::J => b'J' as u32,
            KeyCode::K => b'K' as u32,
            KeyCode::L => b'L' as u32,
            KeyCode::M => b'M' as u32,
            KeyCode::N => b'N' as u32,
            KeyCode::O => b'O' as u32,
            KeyCode::P => b'P' as u32,
            KeyCode::Q => b'Q' as u32,
            KeyCode::R => b'R' as u32,
            KeyCode::S => b'S' as u32,
            KeyCode::T => b'T' as u32,
            KeyCode::U => b'U' as u32,
            KeyCode::V => b'V' as u32,
            KeyCode::W => b'W' as u32,
            KeyCode::X => b'X' as u32,
            KeyCode::Y => b'Y' as u32,
            KeyCode::Z => b'Z' as u32,
            KeyCode::Digit(d) => '0' as u32 + d as u32,
            KeyCode::Function(n) => 0x70 + (n as u32 - 1),
            KeyCode::Space => 0x20,
            KeyCode::Return => 0x0D,
            KeyCode::Escape => 0x1B,
            KeyCode::Tab => 0x09,
            KeyCode::Backspace => 0x08,
            KeyCode::Insert => 0x2D,
            KeyCode::Delete => 0x2E,
            KeyCode::Home => 0x24,
            KeyCode::End => 0x23,
            KeyCode::PageUp => 0x21,
            KeyCode::PageDown => 0x22,
            KeyCode::Left => 0x25,
            KeyCode::Right => 0x27,
            KeyCode::Up => 0x26,
            KeyCode::Down => 0x28,
            KeyCode::Print => 0x2A,
            KeyCode::ScrollLock => 0x91,
            KeyCode::Pause => 0x13,
        }
    }

    pub fn from_vk(vk: u32) -> Option<KeyCode> {
        match vk {
            0x41..=0x5A => Some(match vk {
                0x41 => KeyCode::A,
                0x42 => KeyCode::B,
                0x43 => KeyCode::C,
                0x44 => KeyCode::D,
                0x45 => KeyCode::E,
                0x46 => KeyCode::F,
                0x47 => KeyCode::G,
                0x48 => KeyCode::H,
                0x49 => KeyCode::I,
                0x4A => KeyCode::J,
                0x4B => KeyCode::K,
                0x4C => KeyCode::L,
                0x4D => KeyCode::M,
                0x4E => KeyCode::N,
                0x4F => KeyCode::O,
                0x50 => KeyCode::P,
                0x51 => KeyCode::Q,
                0x52 => KeyCode::R,
                0x53 => KeyCode::S,
                0x54 => KeyCode::T,
                0x55 => KeyCode::U,
                0x56 => KeyCode::V,
                0x57 => KeyCode::W,
                0x58 => KeyCode::X,
                0x59 => KeyCode::Y,
                _ => KeyCode::Z,
            }),
            0x30..=0x39 => Some(KeyCode::Digit((vk - 0x30) as u8)),
            0x70..=0x87 => Some(KeyCode::Function((vk - 0x70 + 1) as u8)),
            0x20 => Some(KeyCode::Space),
            0x0D => Some(KeyCode::Return),
            0x1B => Some(KeyCode::Escape),
            0x09 => Some(KeyCode::Tab),
            0x08 => Some(KeyCode::Backspace),
            0x2D => Some(KeyCode::Insert),
            0x2E => Some(KeyCode::Delete),
            0x24 => Some(KeyCode::Home),
            0x23 => Some(KeyCode::End),
            0x21 => Some(KeyCode::PageUp),
            0x22 => Some(KeyCode::PageDown),
            0x25 => Some(KeyCode::Left),
            0x27 => Some(KeyCode::Right),
            0x26 => Some(KeyCode::Up),
            0x28 => Some(KeyCode::Down),
            0x2A => Some(KeyCode::Print),
            0x91 => Some(KeyCode::ScrollLock),
            0x13 => Some(KeyCode::Pause),
            _ => None,
        }
    }

    fn parse(name: &str) -> Option<KeyCode> {
        let s = name.trim();
        if s.is_empty() {
            return None;
        }
        if s.chars().count() == 1 {
            let c = s.chars().next().unwrap();
            if c.is_ascii_alphabetic() {
                return letters().get(c.to_ascii_uppercase() as usize - 'A' as usize).copied();
            }
            if c.is_ascii_digit() {
                return Some(KeyCode::Digit(c as u8 - b'0'));
            }
        }
        let lower = s.to_ascii_lowercase();
        match lower.as_str() {
            "space" => Some(KeyCode::Space),
            "return" | "enter" => Some(KeyCode::Return),
            "esc" | "escape" => Some(KeyCode::Escape),
            "tab" => Some(KeyCode::Tab),
            "backspace" | "back" => Some(KeyCode::Backspace),
            "ins" | "insert" => Some(KeyCode::Insert),
            "del" | "delete" => Some(KeyCode::Delete),
            "home" => Some(KeyCode::Home),
            "end" => Some(KeyCode::End),
            "pgup" | "pageup" | "page up" => Some(KeyCode::PageUp),
            "pgdown" | "pgdn" | "pagedown" | "page down" => Some(KeyCode::PageDown),
            "left" => Some(KeyCode::Left),
            "right" => Some(KeyCode::Right),
            "up" => Some(KeyCode::Up),
            "down" => Some(KeyCode::Down),
            "print" | "printscreen" | "print screen" => Some(KeyCode::Print),
            "scrolllock" | "scroll lock" => Some(KeyCode::ScrollLock),
            "pause" => Some(KeyCode::Pause),
            _ => {
                if let Some(rest) = lower.strip_prefix('f') {
                    if let Ok(n) = rest.parse::<u8>() {
                        if (1..=24).contains(&n) {
                            return Some(KeyCode::Function(n));
                        }
                    }
                }
                None
            }
        }
    }
}

fn letters() -> &'static [KeyCode; 26] {
    &[
        KeyCode::A,
        KeyCode::B,
        KeyCode::C,
        KeyCode::D,
        KeyCode::E,
        KeyCode::F,
        KeyCode::G,
        KeyCode::H,
        KeyCode::I,
        KeyCode::J,
        KeyCode::K,
        KeyCode::L,
        KeyCode::M,
        KeyCode::N,
        KeyCode::O,
        KeyCode::P,
        KeyCode::Q,
        KeyCode::R,
        KeyCode::S,
        KeyCode::T,
        KeyCode::U,
        KeyCode::V,
        KeyCode::W,
        KeyCode::X,
        KeyCode::Y,
        KeyCode::Z,
    ]
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct KeySequence {
    pub modifiers: Modifiers,
    pub key: KeyCode,
}

impl KeySequence {
    pub fn new(modifiers: Modifiers, key: KeyCode) -> Self {
        Self { modifiers, key }
    }

    /// Parses Qt's string form.  Returns `None` for empty or malformed input
    /// (an empty sequence is represented by `Option<KeySequence>::None`).
    pub fn parse(text: &str) -> Option<KeySequence> {
        let text = text.trim();
        if text.is_empty() {
            return None;
        }
        let mut parts: Vec<&str> = text.split('+').map(|s| s.trim()).collect();
        if parts.last().map(|s| s.is_empty()).unwrap_or(true) {
            // "Ctrl++" would be the plus key; not part of the supported set.
            return None;
        }
        let key = KeyCode::parse(parts.pop()?)?;
        let mut mods = Modifiers::default();
        for part in parts {
            if !part.is_empty() && !mods.set(part) {
                return None;
            }
        }
        Some(KeySequence { modifiers: mods, key })
    }

    /// Qt `QKeySequence::toString()` (portable text).
    pub fn to_string_qt(self) -> String {
        let mut s = String::new();
        if self.modifiers.meta {
            s.push_str("Meta+");
        }
        if self.modifiers.ctrl {
            s.push_str("Ctrl+");
        }
        if self.modifiers.alt {
            s.push_str("Alt+");
        }
        if self.modifiers.shift {
            s.push_str("Shift+");
        }
        s.push_str(&self.key.name());
        s
    }

    pub fn to_win(self) -> (u32, u32) {
        let mut mods = 0;
        if self.modifiers.alt {
            mods |= MOD_ALT;
        }
        if self.modifiers.ctrl {
            mods |= MOD_CONTROL;
        }
        if self.modifiers.shift {
            mods |= MOD_SHIFT;
        }
        if self.modifiers.meta {
            mods |= MOD_WIN;
        }
        (mods, self.key.to_vk())
    }

    pub fn from_win(mods: u32, vk: u32) -> Option<KeySequence> {
        Some(KeySequence {
            modifiers: Modifiers {
                ctrl: mods & MOD_CONTROL != 0,
                alt: mods & MOD_ALT != 0,
                shift: mods & MOD_SHIFT != 0,
                meta: mods & MOD_WIN != 0,
            },
            key: KeyCode::from_vk(vk)?,
        })
    }
}

impl fmt::Display for KeySequence {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_string_qt())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_formats_common_sequences() {
        for text in [
            "Alt+X",
            "Ctrl+Alt+X",
            "Ctrl+Shift+F5",
            "Meta+Ctrl+Alt+Shift+F24",
            "Alt+F4",
            "Ctrl+S",
            "Ctrl+Space",
            "Ctrl+Return",
            "Ctrl+Esc",
            "Ctrl+Del",
            "Alt+PgUp",
            "Alt+PgDown",
            "Ctrl+Print",
            "Ctrl+ScrollLock",
            "Ctrl+Pause",
            "Ctrl+Left",
        ] {
            let seq = KeySequence::parse(text).unwrap_or_else(|| panic!("parse {text}"));
            assert_eq!(seq.to_string_qt(), text, "round trip {text}");
        }
    }

    #[test]
    fn accepts_long_aliases() {
        let a = KeySequence::parse("Control+Alt+Delete").unwrap();
        assert_eq!(a.to_string_qt(), "Ctrl+Alt+Del");
        let b = KeySequence::parse("Shift+PageDown").unwrap();
        assert_eq!(b.to_string_qt(), "Shift+PgDown");
        let c = KeySequence::parse("win+escape").unwrap();
        assert_eq!(c.to_string_qt(), "Meta+Esc");
        let d = KeySequence::parse("Ctrl+Enter").unwrap();
        assert_eq!(d, KeySequence::parse("Ctrl+Return").unwrap());
    }

    #[test]
    fn rejects_invalid_sequences() {
        assert!(KeySequence::parse("").is_none());
        assert!(KeySequence::parse("Ctrl").is_none());
        assert!(KeySequence::parse("Ctrl+").is_none());
        assert!(KeySequence::parse("Bogus+X").is_none());
        assert!(KeySequence::parse("F25").is_none());
        assert!(KeySequence::parse("Ctrl+Shift").is_none());
    }

    #[test]
    fn windows_vk_round_trip() {
        for text in [
            "Alt+X",
            "Ctrl+Alt+X",
            "Ctrl+Shift+F5",
            "Meta+Ctrl+Alt+Shift+F24",
            "Alt+F4",
            "Ctrl+Space",
            "Ctrl+Return",
            "Ctrl+Esc",
            "Ctrl+Del",
            "Alt+PgUp",
            "Alt+PgDown",
            "Ctrl+Print",
            "Ctrl+ScrollLock",
            "Ctrl+Pause",
            "Ctrl+Left",
        ] {
            let seq = KeySequence::parse(text).unwrap();
            let (mods, vk) = seq.to_win();
            assert_eq!(KeySequence::from_win(mods, vk).unwrap(), seq);
        }
    }

    #[test]
    fn all_function_keys_round_trip() {
        for n in 1..=24u8 {
            let text = format!("Ctrl+F{n}");
            let seq = KeySequence::parse(&text).unwrap();
            assert_eq!(seq.to_string_qt(), text);
            assert_eq!(KeySequence::from_win(seq.to_win().0, seq.to_win().1), Some(seq));
        }
    }
}
