//! Port of `helpers/ChannelHelper.{h,cpp}`.

use crate::log_helper::log_error_static;

pub const SPEAKER_FRONT_LEFT: u32 = 0x1;
pub const SPEAKER_FRONT_RIGHT: u32 = 0x2;
pub const SPEAKER_FRONT_CENTER: u32 = 0x4;
pub const SPEAKER_LOW_FREQUENCY: u32 = 0x8;
pub const SPEAKER_BACK_LEFT: u32 = 0x10;
pub const SPEAKER_BACK_RIGHT: u32 = 0x20;
pub const SPEAKER_FRONT_LEFT_OF_CENTER: u32 = 0x40;
pub const SPEAKER_FRONT_RIGHT_OF_CENTER: u32 = 0x80;
pub const SPEAKER_BACK_CENTER: u32 = 0x100;
pub const SPEAKER_SIDE_LEFT: u32 = 0x200;
pub const SPEAKER_SIDE_RIGHT: u32 = 0x400;

pub const KSAUDIO_SPEAKER_MONO: u32 = SPEAKER_FRONT_CENTER;
pub const KSAUDIO_SPEAKER_STEREO: u32 = SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT;
pub const KSAUDIO_SPEAKER_QUAD: u32 =
    SPEAKER_FRONT_LEFT | SPEAKER_FRONT_RIGHT | SPEAKER_BACK_LEFT | SPEAKER_BACK_RIGHT;
pub const KSAUDIO_SPEAKER_5POINT1_SURROUND: u32 = SPEAKER_FRONT_LEFT
    | SPEAKER_FRONT_RIGHT
    | SPEAKER_FRONT_CENTER
    | SPEAKER_LOW_FREQUENCY
    | SPEAKER_SIDE_LEFT
    | SPEAKER_SIDE_RIGHT;
pub const KSAUDIO_SPEAKER_7POINT1_SURROUND: u32 = SPEAKER_FRONT_LEFT
    | SPEAKER_FRONT_RIGHT
    | SPEAKER_FRONT_CENTER
    | SPEAKER_LOW_FREQUENCY
    | SPEAKER_BACK_LEFT
    | SPEAKER_BACK_RIGHT
    | SPEAKER_SIDE_LEFT
    | SPEAKER_SIDE_RIGHT;

fn channel_pos_to_name(pos: u32) -> Option<&'static str> {
    match pos {
        SPEAKER_FRONT_LEFT => Some("L"),
        SPEAKER_FRONT_RIGHT => Some("R"),
        SPEAKER_FRONT_CENTER => Some("C"),
        SPEAKER_LOW_FREQUENCY => Some("LFE"),
        SPEAKER_BACK_LEFT => Some("RL"),
        SPEAKER_BACK_RIGHT => Some("RR"),
        SPEAKER_BACK_CENTER => Some("RC"),
        SPEAKER_SIDE_LEFT => Some("SL"),
        SPEAKER_SIDE_RIGHT => Some("SR"),
        _ => None,
    }
}

/// Returns the `KSAUDIO_SPEAKER_*` mask used for a channel count.
pub fn get_default_channel_mask(channel_count: i32) -> u32 {
    match channel_count {
        1 => KSAUDIO_SPEAKER_MONO,
        2 => KSAUDIO_SPEAKER_STEREO,
        4 => KSAUDIO_SPEAKER_QUAD,
        6 => KSAUDIO_SPEAKER_5POINT1_SURROUND,
        8 => KSAUDIO_SPEAKER_7POINT1_SURROUND,
        _ => 0,
    }
}

/// Empty `channelMask` yields no named channels and numeric names from 1.
pub fn get_channel_names(channel_count: i32, channel_mask: u32) -> Vec<String> {
    let mut channel_names = Vec::new();
    let mut c: i32 = 1;
    for i in 0..31 {
        let channel_pos = 1u32 << i;
        if channel_mask & channel_pos != 0 {
            if let Some(name) = channel_pos_to_name(channel_pos) {
                channel_names.push(name.to_string());
            } else {
                channel_names.push(c.to_string());
            }
            c += 1;
        }
    }
    while c <= channel_count {
        channel_names.push(c.to_string());
        c += 1;
    }
    channel_names
}

/// Resolves a numeric or positional channel name to a zero-based index.
///
/// `SL`/`SR` and `RL`/`RR` are accepted interchangeably, and `SUB` is accepted
/// for `LFE`, mirroring the C++ special cases.  Returns -1 when not found.
pub fn get_channel_index(word: &str, channel_names: &[String], allow_new: bool) -> i32 {
    let mut channel_index: i32 = -1;
    let first = match word.chars().next() {
        Some(c) => c,
        None => return -1,
    };
    if first.is_ascii_digit() {
        let parsed: i32 = word
            .chars()
            .take_while(|c| c.is_ascii_digit())
            .collect::<String>()
            .parse()
            .unwrap_or(0);
        channel_index = parsed - 1;
        if channel_index < 0 || channel_index >= channel_names.len() as i32 {
            log_error_static(&format!(
                "Channel number {} out of range (1 - {})",
                word,
                channel_names.len()
            ));
            channel_index = -1;
        }
    } else {
        let mut pos = channel_names.iter().position(|n| n == word);
        if pos.is_none() {
            let alternate = match word {
                "SL" => Some("RL"),
                "SR" => Some("RR"),
                "RL" => Some("SL"),
                "RR" => Some("SR"),
                "SUB" => Some("LFE"),
                _ => None,
            };
            if let Some(alt) = alternate {
                pos = channel_names.iter().position(|n| n == alt);
            }
        }
        match pos {
            Some(p) => channel_index = p as i32,
            None => {
                if !allow_new {
                    log_error_static(&format!("Invalid channel position {}", word));
                }
            }
        }
    }
    channel_index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_masks_match_ksmedia() {
        assert_eq!(get_default_channel_mask(1), KSAUDIO_SPEAKER_MONO);
        assert_eq!(get_default_channel_mask(2), KSAUDIO_SPEAKER_STEREO);
        assert_eq!(get_default_channel_mask(4), KSAUDIO_SPEAKER_QUAD);
        assert_eq!(
            get_default_channel_mask(6),
            KSAUDIO_SPEAKER_5POINT1_SURROUND
        );
        assert_eq!(
            get_default_channel_mask(8),
            KSAUDIO_SPEAKER_7POINT1_SURROUND
        );
        assert_eq!(get_default_channel_mask(3), 0);
    }

    #[test]
    fn channel_names_follow_mask_order() {
        assert_eq!(
            get_channel_names(2, KSAUDIO_SPEAKER_STEREO),
            vec!["L".to_string(), "R".to_string()]
        );
        assert_eq!(
            get_channel_names(6, KSAUDIO_SPEAKER_5POINT1_SURROUND),
            vec![
                "L".to_string(),
                "R".to_string(),
                "C".to_string(),
                "LFE".to_string(),
                "SL".to_string(),
                "SR".to_string()
            ]
        );
        // Channels not covered by the mask get numeric names.
        assert_eq!(
            get_channel_names(3, KSAUDIO_SPEAKER_STEREO),
            vec!["L".to_string(), "R".to_string(), "3".to_string()]
        );
        // An empty mask falls back to 1..=channelCount.
        assert_eq!(
            get_channel_names(2, 0),
            vec!["1".to_string(), "2".to_string()]
        );
    }

    #[test]
    fn channel_index_numeric_and_positions() {
        let names = vec![
            "L".to_string(),
            "R".to_string(),
            "C".to_string(),
            "LFE".to_string(),
            "RL".to_string(),
            "RR".to_string(),
        ];
        assert_eq!(get_channel_index("1", &names, false), 0);
        assert_eq!(get_channel_index("6", &names, false), 5);
        assert_eq!(get_channel_index("7", &names, false), -1);
        assert_eq!(get_channel_index("C", &names, false), 2);
        // SL/SR are accepted for back channels and vice versa.
        assert_eq!(get_channel_index("SL", &names, false), 4);
        assert_eq!(get_channel_index("SR", &names, false), 5);
        // SUB is the old name for LFE.
        assert_eq!(get_channel_index("SUB", &names, false), 3);
        assert_eq!(get_channel_index("XX", &names, false), -1);
        assert_eq!(get_channel_index("XX", &names, true), -1);
        assert_eq!(get_channel_index("", &names, false), -1);
    }
}
