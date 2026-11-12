//! Reads a WAV or FLAC file into samples in an output's rate and channel layout. The
//! format is told from the file's first bytes, not from its name.
use hound::{SampleFormat, WavReader};
use std::io::Read;
use std::path::Path;
use std::sync::Arc;

/// Largest sound file that is decoded.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Longest decoded sound, in seconds at the device's rate.
pub const MAX_SECONDS: u64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WavError {
    /// The file is missing, unreadable or not a WAV or FLAC file this reader supports.
    Unreadable,
    /// The file or the decoded sound is over a limit.
    TooLarge,
}

/// Decodes `path` for an output with `rate` and `channels`.
pub(crate) fn load(path: &Path, rate: u32, channels: u16) -> Result<Arc<[f32]>, WavError> {
    let size = std::fs::metadata(path)
        .map_err(|_| WavError::Unreadable)?
        .len();
    if size > MAX_FILE_BYTES {
        return Err(WavError::TooLarge);
    }
    let mut magic = [0u8; 4];
    std::fs::File::open(path)
        .and_then(|mut file| file.read_exact(&mut magic))
        .map_err(|_| WavError::Unreadable)?;
    if &magic == b"fLaC" {
        return flac(path, rate, channels);
    }
    let mut reader = WavReader::open(path).map_err(|_| WavError::Unreadable)?;
    let spec = reader.spec();
    let samples: Result<Vec<f32>, _> = match spec.sample_format {
        SampleFormat::Float => reader.samples::<f32>().collect(),
        SampleFormat::Int => {
            if !(1..=32).contains(&spec.bits_per_sample) {
                return Err(WavError::Unreadable);
            }
            let scale = 2f32.powi(i32::from(spec.bits_per_sample) - 1).recip();
            #[expect(
                clippy::cast_precision_loss,
                reason = "audio samples; the loss is inaudible"
            )]
            reader
                .samples::<i32>()
                .map(|sample| sample.map(|value| value as f32 * scale))
                .collect()
        }
    };
    let samples = samples.map_err(|_| WavError::Unreadable)?;
    convert(&samples, spec.channels, spec.sample_rate, channels, rate)
}

fn flac(path: &Path, rate: u32, channels: u16) -> Result<Arc<[f32]>, WavError> {
    let mut reader = claxon::FlacReader::open(path).map_err(|_| WavError::Unreadable)?;
    let info = reader.streaminfo();
    let source_channels = u16::try_from(info.channels).map_err(|_| WavError::Unreadable)?;
    if !(1..=32).contains(&info.bits_per_sample) {
        return Err(WavError::Unreadable);
    }
    // Refuse a stream that declares more than the limit before decoding any of it.
    let declared = info.samples.unwrap_or(0) / u64::from(info.sample_rate.max(1));
    if declared > MAX_SECONDS {
        return Err(WavError::TooLarge);
    }
    #[expect(
        clippy::cast_possible_wrap,
        reason = "bits_per_sample is within 1..=32"
    )]
    let scale = 2f32.powi(info.bits_per_sample as i32 - 1).recip();
    #[expect(
        clippy::cast_precision_loss,
        reason = "audio samples; the loss is inaudible"
    )]
    let samples: Result<Vec<f32>, _> = reader
        .samples()
        .map(|sample| sample.map(|value| value as f32 * scale))
        .collect();
    let samples = samples.map_err(|_| WavError::Unreadable)?;
    convert(&samples, source_channels, info.sample_rate, channels, rate)
}

/// Changes interleaved samples to another channel count and rate. Mono is copied to
/// every output channel; otherwise channels map one to one and extra output channels
/// are silent. The rate changes by linear interpolation.
pub(crate) fn convert(
    samples: &[f32],
    from_channels: u16,
    from_rate: u32,
    to_channels: u16,
    to_rate: u32,
) -> Result<Arc<[f32]>, WavError> {
    let (source, target) = (usize::from(from_channels), usize::from(to_channels));
    if source == 0 || target == 0 || from_rate == 0 || to_rate == 0 {
        return Err(WavError::Unreadable);
    }
    let frames = samples.len() / source;
    let out_frames = u64::try_from(frames).map_err(|_| WavError::TooLarge)? * u64::from(to_rate)
        / u64::from(from_rate);
    if out_frames > MAX_SECONDS * u64::from(to_rate) {
        return Err(WavError::TooLarge);
    }
    let out_frames = usize::try_from(out_frames).map_err(|_| WavError::TooLarge)?;
    let at = |frame: usize, channel: usize| -> f32 {
        let channel = if source == 1 { 0 } else { channel };
        if channel >= source {
            return 0.0;
        }
        samples
            .get(frame * source + channel)
            .copied()
            .unwrap_or(0.0)
    };
    let step = f64::from(from_rate) / f64::from(to_rate);
    let mut out = Vec::with_capacity(out_frames * target);
    for frame in 0..out_frames {
        #[expect(
            clippy::cast_precision_loss,
            clippy::cast_possible_truncation,
            clippy::cast_sign_loss,
            reason = "frame positions are bounded by MAX_SECONDS and never negative"
        )]
        let (first, mix) = {
            let position = frame as f64 * step;
            (position.floor() as usize, (position.fract()) as f32)
        };
        let next = (first + 1).min(frames.saturating_sub(1));
        for channel in 0..target {
            let (a, b) = (at(first, channel), at(next, channel));
            out.push(a + (b - a) * mix);
        }
    }
    Ok(out.into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mono_fills_every_channel_and_extra_channels_are_silent() {
        let stereo = convert(&[0.5, -0.5], 1, 100, 2, 100).unwrap();
        assert_eq!(&*stereo, [0.5, 0.5, -0.5, -0.5]);
        let wide = convert(&[0.1, 0.2], 2, 100, 3, 100).unwrap();
        assert_eq!(&*wide, [0.1, 0.2, 0.0]);
        let narrow = convert(&[0.1, 0.2, 0.3, 0.4], 2, 100, 1, 100).unwrap();
        assert_eq!(&*narrow, [0.1, 0.3]);
    }

    #[test]
    fn the_rate_changes_by_interpolation() {
        let doubled = convert(&[0.0, 1.0], 1, 100, 1, 200).unwrap();
        assert_eq!(&*doubled, [0.0, 0.5, 1.0, 1.0]);
        let halved = convert(&[0.0, 0.2, 0.4, 0.6], 1, 200, 1, 100).unwrap();
        assert_eq!(&*halved, [0.0, 0.4]);
    }

    #[test]
    fn a_sound_over_the_length_limit_is_refused() {
        let samples = vec![0.0; 1_000];
        // 1,000 frames at 1 Hz is 1,000 seconds.
        assert_eq!(convert(&samples, 1, 1, 1, 1), Err(WavError::TooLarge));
        assert_eq!(convert(&samples, 0, 1, 1, 1), Err(WavError::Unreadable));
    }

    #[test]
    fn integer_and_float_files_decode_to_the_same_scale() {
        let dir = std::env::temp_dir().join(format!("orange-audio-wav-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let ints = dir.join("ints.wav");
        let spec = hound::WavSpec {
            channels: 1,
            sample_rate: 8_000,
            bits_per_sample: 16,
            sample_format: SampleFormat::Int,
        };
        let mut writer = hound::WavWriter::create(&ints, spec).unwrap();
        for value in [0i16, 16_384, -32_768] {
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
        assert_eq!(&*load(&ints, 8_000, 1).unwrap(), [0.0, 0.5, -1.0]);

        let floats = dir.join("floats.wav");
        let spec = hound::WavSpec {
            bits_per_sample: 32,
            sample_format: SampleFormat::Float,
            ..spec
        };
        let mut writer = hound::WavWriter::create(&floats, spec).unwrap();
        for value in [0.0f32, 0.5, -1.0] {
            writer.write_sample(value).unwrap();
        }
        writer.finalize().unwrap();
        assert_eq!(&*load(&floats, 8_000, 1).unwrap(), [0.0, 0.5, -1.0]);

        assert_eq!(
            load(&dir.join("missing.wav"), 8_000, 1),
            Err(WavError::Unreadable)
        );
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// A FLAC stream of one frame of uncompressed 16-bit mono samples at 8 kHz.
    fn flac_file(samples: &[i16]) -> Vec<u8> {
        fn crc(bytes: &[u8], width: u32, polynomial: u32) -> u32 {
            let top = 1 << (width - 1);
            let mut crc = 0u32;
            for byte in bytes {
                crc ^= u32::from(*byte) << (width - 8);
                for _ in 0..8 {
                    crc = if crc & top == 0 {
                        crc << 1
                    } else {
                        (crc << 1) ^ polynomial
                    };
                }
            }
            crc & ((1u64 << width) - 1) as u32
        }
        let count = u16::try_from(samples.len()).unwrap();
        let mut file = b"fLaC".to_vec();
        // The last metadata block: STREAMINFO, 34 bytes.
        file.extend_from_slice(&[0x80, 0, 0, 34]);
        file.extend_from_slice(&count.to_be_bytes());
        file.extend_from_slice(&count.to_be_bytes());
        file.extend_from_slice(&[0; 6]);
        // 20 bits rate, 3 bits channels - 1, 5 bits depth - 1, 36 bits sample count.
        let info = (8_000u64 << 44) | (15 << 36) | u64::from(count);
        file.extend_from_slice(&info.to_be_bytes());
        file.extend_from_slice(&[0; 16]);
        // Frame: fixed block size given as 16 bits, rate from STREAMINFO, mono, 16 bits.
        let mut frame = vec![0xff, 0xf8, 0x70, 0x08, 0x00];
        frame.extend_from_slice(&(count - 1).to_be_bytes());
        frame.push(crc(&frame, 8, 0x07) as u8);
        // One verbatim subframe.
        frame.push(0x02);
        for sample in samples {
            frame.extend_from_slice(&sample.to_be_bytes());
        }
        let check = crc(&frame, 16, 0x8005) as u16;
        frame.extend_from_slice(&check.to_be_bytes());
        file.extend_from_slice(&frame);
        file
    }

    #[test]
    fn a_flac_file_decodes_like_the_same_wav_file() {
        let dir = std::env::temp_dir().join(format!("orange-audio-flac-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let samples: Vec<i16> = (0..64).map(|n| (n * 500 - 16_000) as i16).collect();
        // The extension does not matter: the content says what it is.
        let path = dir.join("tone.wav");
        std::fs::write(&path, flac_file(&samples)).unwrap();
        let decoded = load(&path, 8_000, 1).unwrap();
        let expected: Vec<f32> = samples.iter().map(|s| f32::from(*s) / 32_768.0).collect();
        assert_eq!(&decoded[..], &expected[..]);
        // A damaged stream is refused, not played as noise.
        let mut damaged = flac_file(&samples);
        let middle = damaged.len() - 20;
        damaged[middle] ^= 0xff;
        std::fs::write(&path, damaged).unwrap();
        assert_eq!(load(&path, 8_000, 1), Err(WavError::Unreadable));
        let _ = std::fs::remove_dir_all(&dir);
    }
}
