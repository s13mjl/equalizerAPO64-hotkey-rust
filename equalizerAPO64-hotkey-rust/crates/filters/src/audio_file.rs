//! Audio file loading for convolution and Benchmark.
//!
//! The original uses libsndfile. This pure-Rust port supports the formats
//! used for impulse responses and benchmark input with native decoders:
//! WAV (hound), FLAC (claxon) and OGG/Vorbis (lewton). Samples are returned
//! as non-interleaved `f64`, normalized like `sf_readf_double`.

use std::io::Cursor;
use std::path::Path;

#[derive(Clone, Debug)]
pub struct AudioFile {
    pub sample_rate: u32,
    pub channels: Vec<Vec<f64>>,
}

impl AudioFile {
    pub fn frames(&self) -> usize {
        self.channels.first().map(Vec::len).unwrap_or(0)
    }

    pub fn channel_count(&self) -> usize {
        self.channels.len()
    }
}

pub fn read_audio_file(path: impl AsRef<Path>) -> Result<AudioFile, String> {
    let bytes = std::fs::read(path.as_ref()).map_err(|error| error.to_string())?;
    read_audio_bytes(&bytes)
}

pub fn read_audio_bytes(bytes: &[u8]) -> Result<AudioFile, String> {
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        read_wav(bytes)
    } else if bytes.starts_with(b"fLaC") {
        read_flac(bytes)
    } else if bytes.starts_with(b"OggS") {
        read_ogg(bytes)
    } else {
        Err("unsupported audio file format".to_string())
    }
}

/// Reads only the metadata/length; used by the Editor's file-info panel.
pub fn probe_audio_file(path: impl AsRef<Path>) -> Result<(u32, u64), String> {
    let path = path.as_ref();
    let bytes = std::fs::read(path).map_err(|error| error.to_string())?;
    if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WAVE" {
        let reader =
            hound::WavReader::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
        let spec = reader.spec();
        let frames = reader.duration() as u64;
        return Ok((spec.sample_rate, frames));
    }
    if bytes.starts_with(b"fLaC") {
        let reader = claxon::FlacReader::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
        let info = reader.streaminfo();
        let frames = info.samples.unwrap_or(0);
        return Ok((info.sample_rate, frames));
    }
    if bytes.starts_with(b"OggS") {
        let file = read_ogg(&bytes)?;
        return Ok((file.sample_rate, file.frames() as u64));
    }
    Err("unsupported audio file format".to_string())
}

fn read_wav(bytes: &[u8]) -> Result<AudioFile, String> {
    let mut reader =
        hound::WavReader::new(Cursor::new(bytes)).map_err(|error| error.to_string())?;
    let spec = reader.spec();
    let channel_count = spec.channels as usize;
    if channel_count == 0 {
        return Err("file has no channels".to_string());
    }
    let frame_count = reader.duration() as usize;
    let mut interleaved = vec![0.0f64; frame_count * channel_count];

    match spec.sample_format {
        hound::SampleFormat::Int => {
            let scale = match spec.bits_per_sample {
                8 => 128.0,
                16 => 32768.0,
                24 => 8_388_608.0,
                32 => 2_147_483_648.0,
                bits => return Err(format!("unsupported PCM bit depth {bits}")),
            };
            let mut index = 0;
            for sample in reader.samples::<i32>() {
                if index >= interleaved.len() {
                    break;
                }
                interleaved[index] =
                    sample.map_err(|error| error.to_string())? as f64 / scale;
                index += 1;
            }
        }
        hound::SampleFormat::Float => {
            if spec.bits_per_sample != 32 {
                return Err(format!(
                    "unsupported floating point bit depth {}",
                    spec.bits_per_sample
                ));
            }
            let mut index = 0;
            for sample in reader.samples::<f32>() {
                if index >= interleaved.len() {
                    break;
                }
                interleaved[index] =
                    sample.map_err(|error| error.to_string())? as f64;
                index += 1;
            }
        }
    }

    Ok(split_channels(
        interleaved,
        frame_count,
        channel_count,
        spec.sample_rate,
    ))
}

fn read_flac(bytes: &[u8]) -> Result<AudioFile, String> {
    let mut reader = claxon::FlacReader::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let info = reader.streaminfo();
    let channel_count = info.channels as usize;
    let bits = info.bits_per_sample;
    let scale = (1u64 << (bits - 1)) as f64;
    let mut interleaved: Vec<f64> = Vec::new();
    for sample in reader.samples() {
        interleaved.push(sample.map_err(|error| error.to_string())? as f64 / scale);
    }
    let frame_count = if channel_count == 0 {
        0
    } else {
        interleaved.len() / channel_count
    };
    if channel_count == 0 {
        return Err("file has no channels".to_string());
    }
    Ok(split_channels(
        interleaved,
        frame_count,
        channel_count,
        info.sample_rate,
    ))
}

fn read_ogg(bytes: &[u8]) -> Result<AudioFile, String> {
    let mut reader = lewton::inside_ogg::OggStreamReader::new(Cursor::new(bytes))
        .map_err(|error| error.to_string())?;
    let sample_rate = reader.ident_hdr.audio_sample_rate;
    let channel_count = reader.ident_hdr.audio_channels as usize;
    if channel_count == 0 {
        return Err("file has no channels".to_string());
    }
    let mut channels = vec![Vec::<f64>::new(); channel_count];
    while let Some(packet) = reader
        .read_dec_packet_generic::<Vec<Vec<f32>>>()
        .map_err(|error| error.to_string())?
    {
        for (channel, samples) in packet.into_iter().enumerate() {
            if channel < channels.len() {
                channels[channel].extend(samples.into_iter().map(|sample| sample as f64));
            }
        }
    }
    let frames = channels.first().map(Vec::len).unwrap_or(0);
    for channel in &mut channels {
        channel.truncate(frames);
    }
    Ok(AudioFile {
        sample_rate,
        channels,
    })
}

fn split_channels(
    interleaved: Vec<f64>,
    frames: usize,
    channel_count: usize,
    sample_rate: u32,
) -> AudioFile {
    let mut channels = vec![vec![0.0f64; frames]; channel_count];
    for frame in 0..frames {
        for channel in 0..channel_count {
            channels[channel][frame] = interleaved[frame * channel_count + channel];
        }
    }
    AudioFile {
        sample_rate,
        channels,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hound::{SampleFormat, WavSpec, WavWriter};
    use std::io::{Read, Write};
    use std::path::Path;

    fn temp_wav(samples: &[i16], channels: u16) -> Vec<u8> {
        let path = std::env::temp_dir().join(format!(
            "eapo_audio_file_{}_{}.wav",
            std::process::id(),
            samples.len()
        ));
        {
            let spec = WavSpec {
                channels,
                sample_rate: 48000,
                bits_per_sample: 16,
                sample_format: SampleFormat::Int,
            };
            let mut writer = WavWriter::create(&path, spec).unwrap();
            for sample in samples {
                writer.write_sample(*sample).unwrap();
            }
            writer.finalize().unwrap();
        }
        let mut file = std::fs::File::open(&path).unwrap();
        let mut bytes = Vec::new();
        file.read_to_end(&mut bytes).unwrap();
        let _ = std::fs::remove_file(path);
        bytes
    }

    #[test]
    fn reads_pcm16_wav() {
        let bytes = temp_wav(&[0, 16384, -32768, 32767], 1);
        let file = read_audio_bytes(&bytes).unwrap();
        assert_eq!(file.sample_rate, 48000);
        assert_eq!(file.channels.len(), 1);
        assert_eq!(file.channels[0][1], 0.5);
        assert_eq!(file.channels[0][2], -1.0);
    }

    #[test]
    fn decodes_embedded_pink_noise_flac() {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("editor")
            .join("assets")
            .join("pinkNoise.flac");
        let file = read_audio_file(&path).expect("FLAC decode");
        assert!(file.sample_rate > 0);
        assert!(!file.channels.is_empty());
        assert!(file.frames() > 0);
    }

    #[test]
    fn rejects_unknown_format() {
        let mut data = Vec::new();
        data.write_all(b"not audio data").unwrap();
        assert!(read_audio_bytes(&data).is_err());
    }
}
