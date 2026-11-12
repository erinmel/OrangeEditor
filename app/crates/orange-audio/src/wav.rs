//! Reads a WAV file into samples in an output's rate and channel layout.
use hound::{SampleFormat, WavReader};
use std::path::Path;
use std::sync::Arc;

/// Largest sound file that is decoded.
pub const MAX_FILE_BYTES: u64 = 64 * 1024 * 1024;
/// Longest decoded sound, in seconds at the device's rate.
pub const MAX_SECONDS: u64 = 600;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum WavError {
    /// The file is missing, unreadable or not a WAV file this reader supports.
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
}
