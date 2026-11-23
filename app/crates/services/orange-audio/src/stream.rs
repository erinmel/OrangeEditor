//! Live audio between an input device and its consumers.
//!
//! A [`Ring`] holds the latest second of what an input captures, as interleaved `f32`
//! samples at the capture's own rate and channel count. The capture callback is its one
//! writer; any number of readers follow it, each with its own position, so a sound can
//! be played, measured and recorded at once without copying it per consumer. The ring
//! is allocated before the callback starts and never grows: a reader that falls half a
//! ring behind skips ahead to recent samples (the oldest are dropped), and a reader
//! that has caught up gets nothing until the writer adds more.
//!
//! Each atomic slot carries a sequence tag with its sample. Readers discard an
//! overwritten frame instead of combining samples from different positions.
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

/// Seconds of capture a ring holds.
pub const RING_SECONDS: u32 = 1;
/// Fewest samples a ring holds, whatever the rate.
const MIN_RING: usize = 4_096;

/// The latest captured samples of one opened input device.
#[derive(Debug)]
pub(crate) struct Ring {
    pub rate: u32,
    pub channels: u16,
    samples: Box<[AtomicU64]>,
    /// Samples written since the ring was made; slot `n % len` holds sample `n`.
    written: AtomicU64,
    /// Set when the device stopped: no more samples come.
    closed: AtomicBool,
}

impl Ring {
    /// A ring for [`RING_SECONDS`] of capture at `rate` and `channels`, a whole number
    /// of frames long.
    pub(crate) fn new(rate: u32, channels: u16) -> Arc<Self> {
        let frame = usize::from(channels.max(1));
        let wanted = usize::try_from(u64::from(rate) * u64::from(RING_SECONDS))
            .unwrap_or(usize::MAX / frame)
            .saturating_mul(frame)
            .max(MIN_RING);
        let length = wanted.div_ceil(frame) * frame;
        Arc::new(Self {
            rate,
            channels: channels.max(1),
            samples: (0..length).map(|_| AtomicU64::new(0)).collect(),
            written: AtomicU64::new(0),
            closed: AtomicBool::new(false),
        })
    }

    fn len(&self) -> u64 {
        self.samples.len() as u64
    }

    /// Adds captured samples. Runs on the capture callback: no allocation, no lock.
    pub(crate) fn push(&self, samples: impl Iterator<Item = f32>) {
        let mut at = self.written.load(Ordering::Relaxed);
        let length = self.len();
        for sample in samples {
            if let Some(slot) = usize::try_from(at % length)
                .ok()
                .and_then(|slot| self.samples.get(slot))
            {
                slot.store((at << 32) | u64::from(sample.to_bits()), Ordering::Relaxed);
            }
            at += 1;
        }
        self.written.store(at, Ordering::Release);
    }

    /// Samples written so far.
    pub(crate) fn written(&self) -> u64 {
        self.written.load(Ordering::Acquire)
    }

    fn sample(&self, at: u64) -> Option<f32> {
        let slot = self.samples.get(usize::try_from(at % self.len()).ok()?)?;
        let value = slot.load(Ordering::Relaxed);
        #[expect(
            clippy::cast_possible_truncation,
            reason = "the low 32 bits contain the sample"
        )]
        let sample = f32::from_bits(value as u32);
        ((value & 0xffff_ffff_0000_0000) == at << 32).then_some(sample)
    }

    /// Copies the newest samples into `out`, up to its length in whole frames and no
    /// more than the ring holds, oldest first; returns how many were copied.
    #[cfg(test)]
    pub(crate) fn recent(&self, out: &mut [f32]) -> usize {
        let written = self.written();
        let frame = usize::from(self.channels);
        let kept = usize::try_from(written.min(self.len())).unwrap_or(usize::MAX);
        let count = out.len().min(kept) / frame * frame;
        let start = written - count as u64;
        for (offset, slot) in (0u64..).zip(out.iter_mut().take(count)) {
            *slot = self.sample(start + offset).unwrap_or_default();
        }
        count
    }

    /// The device stopped; readers end once they have read what is left.
    pub(crate) fn close(&self) {
        self.closed.store(true, Ordering::Release);
    }

    pub(crate) fn closed(&self) -> bool {
        self.closed.load(Ordering::Acquire)
    }

    /// A position `back` frames before the newest sample, or the oldest one kept.
    fn behind(&self, written: u64, back: u64) -> u64 {
        let frame = u64::from(self.channels);
        let start = written.saturating_sub(back.saturating_mul(frame));
        start - start % frame
    }

    /// Where a reader that falls more than half a ring behind continues: a quarter of
    /// a ring before the newest sample.
    fn caught_up(&self, at: u64, written: u64) -> u64 {
        let length = self.len();
        if written.saturating_sub(at) > length / 2 {
            self.behind(written, length / 4 / u64::from(self.channels))
        } else {
            at
        }
    }
}

/// A reader that takes every sample from now on, in order, for a control thread
/// (recording). It keeps its own position, so other readers are not affected.
#[derive(Debug)]
pub(crate) struct Cursor {
    ring: Arc<Ring>,
    at: u64,
    /// Samples skipped because the reader fell behind.
    pub dropped: u64,
}
impl Cursor {
    /// A reader starting with the next sample captured.
    pub(crate) fn new(ring: Arc<Ring>) -> Self {
        let at = ring.written();
        Self {
            ring,
            at,
            dropped: 0,
        }
    }

    pub(crate) fn ring(&self) -> &Arc<Ring> {
        &self.ring
    }

    /// Copies the samples captured since the last read into `out`, a whole number of
    /// frames, and returns how many were copied.
    pub(crate) fn read(&mut self, out: &mut [f32]) -> usize {
        let written = self.ring.written();
        let at = self.ring.caught_up(self.at, written);
        self.dropped += at - self.at;
        self.at = at;
        let frame = usize::from(self.ring.channels);
        let ready = usize::try_from(written - self.at).unwrap_or(usize::MAX);
        let count = ready.min(out.len()) / frame * frame;
        let mut copied = 0;
        'frames: for output in out[..count].chunks_exact_mut(frame) {
            for (offset, slot) in output.iter_mut().enumerate() {
                let Some(sample) = self.ring.sample(self.at + offset as u64) else {
                    break 'frames;
                };
                *slot = sample;
            }
            self.at += frame as u64;
            copied += frame;
        }
        copied
    }
}

/// How a live sound plays on an output: a reader of a [`Ring`] that converts the
/// capture's rate and channels to the output's, inside the output callback.
///
/// Rate is converted by linear interpolation. Source channel `n` plays on output
/// channel `n`, a mono source on every channel, and an output with more channels than
/// the source repeats them in order (`n % source channels`); source channels beyond the
/// output's are left out. The reader stays close behind the capture: it starts
/// [`START_MS`] behind, and when it falls more than [`MAX_LAG_MS`] behind (the two
/// devices' clocks drift, or the output was paused) it jumps back to [`START_MS`].
/// When it catches up it plays silence until more is captured.
#[derive(Debug)]
pub(crate) struct Live {
    ring: Arc<Ring>,
    at: u64,
    fraction: f64,
    step: f64,
    channels: usize,
    scratch: Box<[f32]>,
    started: bool,
    /// Set by a stop command; the voice is then handed back.
    pub ended: bool,
}

/// How far behind the capture a live sound starts, in milliseconds.
pub const START_MS: u64 = 30;
/// How far behind the capture a live sound may fall before it jumps ahead.
pub const MAX_LAG_MS: u64 = 250;

impl Live {
    /// A live sound of `ring` for an output of `rate` and `channels`. Made on a control
    /// thread; the callback only plays it.
    pub(crate) fn new(ring: Arc<Ring>, rate: u32, channels: u16) -> Self {
        let step = f64::from(ring.rate) / f64::from(rate.max(1));
        Self {
            ring,
            at: 0,
            fraction: 0.0,
            step,
            channels: usize::from(channels.max(1)),
            scratch: vec![0.0; usize::from(channels.max(1))].into_boxed_slice(),
            started: false,
            ended: false,
        }
    }

    /// Whether it has ended: stopped, or its device closed and every sample is played.
    pub(crate) fn finished(&self) -> bool {
        let source = u64::from(self.ring.channels);
        self.ended || (self.ring.closed() && self.ring.written() < self.at + 2 * source)
    }

    /// Adds the next block, scaled by `gain`, into interleaved `out`. Runs on the
    /// output callback: no allocation, no lock, no waiting.
    pub(crate) fn mix(&mut self, out: &mut [f32], gain: f32) {
        if self.ended {
            return;
        }
        let ring = &self.ring;
        let written = ring.written();
        let frames_ms = |ms: u64| u64::from(ring.rate) * ms / 1_000;
        let source = u64::from(ring.channels);
        if !self.started || written.saturating_sub(self.at) > frames_ms(MAX_LAG_MS) * source {
            self.started = true;
            self.at = ring.behind(written, frames_ms(START_MS));
            self.fraction = 0.0;
        }
        let source_channels = usize::from(ring.channels);
        'frames: for frame in out.chunks_exact_mut(self.channels) {
            // Interpolation needs this frame and the next.
            if written < self.at + 2 * source {
                break;
            }
            #[expect(clippy::cast_possible_truncation, reason = "a fraction within 0..1")]
            let mix = self.fraction as f32;
            for (channel, sample) in self.scratch.iter_mut().enumerate() {
                let offset = (channel % source_channels) as u64;
                let (Some(a), Some(b)) = (
                    ring.sample(self.at + offset),
                    ring.sample(self.at + source + offset),
                ) else {
                    self.started = false;
                    break 'frames;
                };
                *sample = (a + (b - a) * mix) * gain;
            }
            for (sample, value) in frame.iter_mut().zip(&self.scratch) {
                *sample += value;
            }
            self.fraction += self.step;
            while self.fraction >= 1.0 {
                self.fraction -= 1.0;
                self.at += source;
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_ring_keeps_whole_frames_of_a_second() {
        let ring = Ring::new(48_000, 2);
        assert_eq!(ring.len(), 96_000);
        let small = Ring::new(8_000, 3);
        assert_eq!(small.len() % 3, 0);
        assert!(small.len() >= MIN_RING as u64);
    }

    #[test]
    fn a_cursor_reads_each_sample_once_and_skips_ahead_when_far_behind() {
        let ring = Ring::new(8_000, 1);
        let mut cursor = Cursor::new(Arc::clone(&ring));
        let mut out = [0.0; 4];
        assert_eq!(cursor.read(&mut out), 0);
        ring.push([0.1, 0.2, 0.3].into_iter());
        assert_eq!(cursor.read(&mut out), 3);
        assert_eq!(out[..3], [0.1, 0.2, 0.3]);
        assert_eq!(cursor.read(&mut out), 0);
        // Most of a ring arrives without being read: the oldest are dropped.
        let length = ring.len();
        ring.push((0..length).map(|n| n as f32));
        let mut out = vec![0.0; 8_192];
        let read = cursor.read(&mut out);
        assert_eq!(cursor.dropped, length - length / 4);
        assert_eq!(out[0], (length - length / 4) as f32);
        assert_eq!(read, 8_192.min(usize::try_from(length / 4).unwrap()));
    }

    #[test]
    fn a_cursor_reads_whole_frames() {
        let ring = Ring::new(8_000, 2);
        let mut cursor = Cursor::new(Arc::clone(&ring));
        ring.push([0.1, 0.2, 0.3, 0.4].into_iter());
        let mut out = [0.0; 3];
        assert_eq!(cursor.read(&mut out), 2);
        assert_eq!(out[..2], [0.1, 0.2]);
    }

    #[test]
    fn lagging_and_prompt_readers_share_one_ring_without_changing_each_other() {
        let ring = Ring::new(1_024, 2);
        let mut prompt = Cursor::new(Arc::clone(&ring));
        let mut lagging = Cursor::new(Arc::clone(&ring));
        let samples = u16::try_from(ring.len()).unwrap();
        let mut read = [0.0; 64];
        for start in (0..samples).step_by(read.len()) {
            let end = start + u16::try_from(read.len()).unwrap();
            ring.push((start..end).map(f32::from));
            assert_eq!(prompt.read(&mut read), 64);
            assert!(read.iter().copied().eq((start..end).map(f32::from)));
        }
        assert_eq!(prompt.dropped, 0);
        assert_eq!(lagging.read(&mut read), 64);
        assert_eq!(lagging.dropped, u64::from(samples / 4 * 3));
        assert_eq!(read[0], f32::from(samples / 4 * 3));
        assert_eq!(prompt.read(&mut read), 0);
        ring.push([0.25, -0.25].into_iter());
        assert_eq!(prompt.read(&mut read), 2);
        assert_eq!(read[..2], [0.25, -0.25]);
    }

    #[test]
    fn a_live_sound_converts_channels_and_rate_and_waits_for_samples() {
        // Mono at 8 kHz plays on both channels of a 16 kHz output: every captured
        // frame becomes two, the second halfway to the next.
        let ring = Ring::new(8_000, 1);
        let mut live = Live::new(Arc::clone(&ring), 16_000, 2);
        let mut out = [0.0f32; 8];
        live.mix(&mut out, 1.0);
        assert_eq!(out, [0.0; 8], "nothing captured yet");
        ring.push([0.0, 0.4, 0.8, 0.8].into_iter());
        let mut out = [0.0f32; 8];
        live.mix(&mut out, 0.5);
        let expected = [0.0, 0.0, 0.1, 0.1, 0.2, 0.2, 0.3, 0.3];
        for (got, want) in out.iter().zip(expected) {
            assert!((got - want).abs() < 1e-6, "{out:?}");
        }
        let mut out = [0.0f32; 4];
        live.mix(&mut out, 1.0);
        assert_eq!(out, [0.8; 4]);
        // Caught up: silence until more arrives, not a repeat.
        let mut out = [0.0f32; 4];
        live.mix(&mut out, 1.0);
        assert_eq!(out, [0.0; 4]);
        assert!(!live.finished());
        ring.close();
        let mut out = [0.0f32; 64];
        live.mix(&mut out, 1.0);
        assert!(live.finished());
    }

    #[test]
    fn a_live_sound_that_falls_behind_jumps_close_to_the_capture() {
        let ring = Ring::new(1_000, 1);
        let mut live = Live::new(Arc::clone(&ring), 1_000, 1);
        let mut out = [0.0f32; 1];
        live.mix(&mut out, 1.0);
        // 600 ms arrive while nothing plays (a paused output).
        ring.push((0..600).map(|n| n as f32 / 1_000.0));
        live.mix(&mut out, 1.0);
        // It plays from 30 ms before the newest sample.
        assert!((out[0] - 0.570).abs() < 1e-6, "{out:?}");
        assert!(!live.finished());
        live.ended = true;
        assert!(live.finished());
    }
}
