//! Pink-noise playback for the loudness calibration dialog.
//!
//! Port of `LoudnessCorrectionFilterGUIDialog`: the embedded mono FLAC is
//! decoded, gated to the selected channel(s), built as a stereo 16-bit WAV in
//! memory and looped with `PlaySound(SND_MEMORY | SND_ASYNC | SND_LOOP)`.

use std::io::Cursor;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::OnceLock;

use windows::core::PCSTR;
use windows::Win32::Media::Audio::{
    PlaySoundA, SND_ASYNC, SND_FLAGS, SND_LOOP, SND_MEMORY, SND_NODEFAULT,
};

pub const CHANNEL_LEFT: u8 = 1;
pub const CHANNEL_RIGHT: u8 = 2;
pub const CHANNEL_BOTH: u8 = 3;

static SOUNDS: OnceLock<Option<SoundBank>> = OnceLock::new();
static SELECTED_CHANNEL: AtomicU8 = AtomicU8::new(CHANNEL_BOTH);
static PLAYING: AtomicU8 = AtomicU8::new(0);

struct SoundBank {
    left: Vec<u8>,
    right: Vec<u8>,
    both: Vec<u8>,
}

pub fn selected_channel() -> u8 {
    SELECTED_CHANNEL.load(Ordering::Relaxed).max(1)
}

pub fn set_selected_channel(channel: u8) {
    SELECTED_CHANNEL.store(channel, Ordering::Relaxed);
    if is_playing() {
        play(channel);
    }
}

pub fn is_playing() -> bool {
    PLAYING.load(Ordering::Relaxed) != 0
}

pub fn play(channel: u8) {
    let Some(bank) = SOUNDS.get_or_init(build_sounds).as_ref() else {
        return;
    };
    stop();
    let buffer = match channel {
        CHANNEL_LEFT => &bank.left,
        CHANNEL_RIGHT => &bank.right,
        _ => &bank.both,
    };
    unsafe {
        let _ = PlaySoundA(
            PCSTR(buffer.as_ptr()),
            None,
            SND_MEMORY | SND_ASYNC | SND_LOOP | SND_NODEFAULT,
        );
    }
    PLAYING.store(channel, Ordering::Relaxed);
}

pub fn stop() {
    unsafe {
        let _ = PlaySoundA(PCSTR::null(), None, SND_FLAGS(0));
    }
    PLAYING.store(0, Ordering::Relaxed);
}

fn build_sounds() -> Option<SoundBank> {
    let data = include_bytes!("../assets/pinkNoise.flac");
    let mut reader = claxon::FlacReader::new(Cursor::new(data)).ok()?;
    let info = reader.streaminfo();
    let channels = info.channels.max(1) as usize;
    let bits = info.bits_per_sample;
    let mut mono: Vec<i16> = Vec::new();
    let mut frame: Vec<i32> = Vec::with_capacity(channels);
    for sample in reader.samples() {
        let sample = sample.ok()?;
        frame.push(sample);
        if frame.len() == channels {
            let value = frame[0];
            mono.push(to_i16(value, bits));
            frame.clear();
        }
    }
    if mono.is_empty() {
        return None;
    }

    Some(SoundBank {
        left: build_wav(&mono, info.sample_rate, true, false),
        right: build_wav(&mono, info.sample_rate, false, true),
        both: build_wav(&mono, info.sample_rate, true, true),
    })
}

fn to_i16(sample: i32, bits: u32) -> i16 {
    match bits {
        0..=15 => ((sample as i32) << (16 - bits)) as i16,
        16 => sample as i16,
        _ => (sample >> (bits - 16)) as i16,
    }
}

fn build_wav(samples: &[i16], sample_rate: u32, left: bool, right: bool) -> Vec<u8> {
    let data_len = (samples.len() * 4) as u32;
    let mut out = Vec::with_capacity(44 + data_len as usize);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data_len).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&1u16.to_le_bytes());
    out.extend_from_slice(&2u16.to_le_bytes());
    out.extend_from_slice(&sample_rate.to_le_bytes());
    out.extend_from_slice(&(sample_rate * 4).to_le_bytes());
    out.extend_from_slice(&4u16.to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&data_len.to_le_bytes());
    for sample in samples {
        let left_value = if left { *sample } else { 0 };
        let right_value = if right { *sample } else { 0 };
        out.extend_from_slice(&left_value.to_le_bytes());
        out.extend_from_slice(&right_value.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedded_pink_noise_decodes_and_builds_wav() {
        let bank = build_sounds().expect("embedded FLAC must decode");
        for buffer in [&bank.left, &bank.right, &bank.both] {
            assert_eq!(&buffer[..4], b"RIFF");
            assert_eq!(&buffer[8..12], b"WAVE");
            assert!(buffer.len() > 44);
        }
        // Left/right are gated: the opposite channel is silent.
        let left = &bank.left;
        let first_right = i16::from_le_bytes([left[46], left[47]]);
        assert_eq!(first_right, 0);
        let right = &bank.right;
        let first_left = i16::from_le_bytes([right[44], right[45]]);
        assert_eq!(first_left, 0);
    }
}
