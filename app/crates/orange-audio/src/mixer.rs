//! The part of playback that runs on the device's callback.
//!
//! [`Mixer::fill`] is all a callback does. It takes commands from a bounded channel
//! without waiting, adds every playing sound into the block and hands finished sounds
//! back on a second bounded channel, so sample memory is freed on a control thread. It
//! does not allocate, block, log or touch a file.
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, sync_channel};

/// Sounds one output plays at once. Further sounds are refused, not queued.
pub const MAX_VOICES: usize = 32;

/// One playing sound: samples already in the device's rate and channel layout.
#[derive(Debug)]
pub(crate) struct Voice {
    pub id: u64,
    pub sound: Arc<[f32]>,
    pub at: usize,
    pub gain: f32,
    /// Starts again from the beginning when it reaches the end, until it is stopped.
    pub looped: bool,
}

#[derive(Debug)]
pub(crate) enum Command {
    Play(Voice),
    Stop(u64),
    StopAll,
    /// Holds every sound where it is; the output is silent meanwhile.
    Pause,
    Resume,
    /// The gain applied to the whole output, on top of each sound's own.
    Volume(f32),
    /// Moves every sound to this sample offset from its start. A sound shorter than
    /// the offset ends; a repeating one wraps around.
    Seek(usize),
}

#[derive(Debug)]
pub(crate) struct Mixer {
    voices: [Option<Voice>; MAX_VOICES],
    commands: Receiver<Command>,
    retired: SyncSender<Voice>,
    paused: bool,
    volume: f32,
    blocks: Arc<AtomicU64>,
}

/// The mixer, the two channel ends its controller keeps, and a count of the blocks the
/// mixer has filled, by which the controller sees that the device is still running.
pub(crate) fn mixer() -> (Mixer, SyncSender<Command>, Receiver<Voice>, Arc<AtomicU64>) {
    // Every voice may be stopped and replaced within one block.
    let (commands, receive) = sync_channel(MAX_VOICES * 4);
    let (retired, done) = sync_channel(MAX_VOICES * 4);
    let blocks = Arc::new(AtomicU64::new(0));
    let mixer = Mixer {
        voices: [const { None }; MAX_VOICES],
        commands: receive,
        retired,
        paused: false,
        volume: 1.0,
        blocks: Arc::clone(&blocks),
    };
    (mixer, commands, done, blocks)
}

fn end(voice: &mut Voice) {
    voice.looped = false;
    voice.at = voice.sound.len();
}

impl Mixer {
    /// Writes the next block of interleaved samples, each within -1.0..=1.0.
    pub(crate) fn fill(&mut self, out: &mut [f32]) {
        while let Ok(command) = self.commands.try_recv() {
            match command {
                Command::Play(voice) => {
                    if let Some(free) = self.voices.iter_mut().find(|slot| slot.is_none()) {
                        *free = Some(voice);
                    } else {
                        // The controller counts voices, so this is not reached; a
                        // refused voice still goes back to be reported as ended.
                        let _ = self.retired.try_send(voice);
                    }
                }
                Command::Stop(id) => {
                    for voice in self.voices.iter_mut().flatten() {
                        if voice.id == id {
                            end(voice);
                        }
                    }
                }
                Command::StopAll => self.voices.iter_mut().flatten().for_each(end),
                Command::Pause => self.paused = true,
                Command::Resume => self.paused = false,
                Command::Volume(volume) => self.volume = volume,
                Command::Seek(offset) => {
                    for voice in self.voices.iter_mut().flatten() {
                        let length = voice.sound.len();
                        voice.at = if voice.looped && length > 0 {
                            offset % length
                        } else {
                            offset.min(length)
                        };
                    }
                }
            }
        }
        let _ = self.blocks.fetch_add(1, Ordering::Relaxed);
        out.fill(0.0);
        for slot in &mut self.voices {
            let Some(voice) = slot else { continue };
            let gain = voice.gain * self.volume;
            let mut written = 0;
            while !self.paused && written < out.len() {
                let rest = voice.sound.get(voice.at..).unwrap_or_default();
                if rest.is_empty() {
                    if voice.looped && !voice.sound.is_empty() {
                        voice.at = 0;
                        continue;
                    }
                    break;
                }
                let count = rest.len().min(out.len() - written);
                for (mixed, sample) in out.iter_mut().skip(written).zip(rest) {
                    *mixed += sample * gain;
                }
                voice.at += count;
                written += count;
            }
            // A stopped sound is handed back even while the output is paused.
            let ended = voice.at >= voice.sound.len() && !(voice.looped && !voice.sound.is_empty());
            if ended
                && let Some(ended) = slot.take()
                && let Err(TrySendError::Full(ended) | TrySendError::Disconnected(ended)) =
                    self.retired.try_send(ended)
            {
                // No room to hand it back: keep the silent voice and retry next block.
                *slot = Some(ended);
            }
        }
        for sample in out {
            *sample = sample.clamp(-1.0, 1.0);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn voice(id: u64, samples: &[f32], gain: f32) -> Voice {
        Voice {
            id,
            sound: samples.into(),
            at: 0,
            gain,
            looped: false,
        }
    }

    #[test]
    fn sounds_are_added_scaled_and_limited() {
        let (mut mixer, commands, done, blocks) = mixer();
        commands
            .send(Command::Play(voice(1, &[0.5, 0.5, 0.5], 1.0)))
            .unwrap();
        commands
            .send(Command::Play(voice(2, &[0.8, -0.25], 0.5)))
            .unwrap();
        let mut block = [9.0; 4];
        mixer.fill(&mut block);
        assert_eq!(block, [0.9, 0.375, 0.5, 0.0]);
        assert_eq!(blocks.load(Ordering::Relaxed), 1);
        // Both ended inside the block and came back to be freed off the callback.
        let mut ended: Vec<u64> = done.try_iter().map(|voice| voice.id).collect();
        ended.sort_unstable();
        assert_eq!(ended, [1, 2]);

        commands
            .send(Command::Play(voice(3, &[0.9, 0.9], 1.0)))
            .unwrap();
        commands
            .send(Command::Play(voice(4, &[0.9, -2.0], 1.0)))
            .unwrap();
        let mut block = [0.0; 2];
        mixer.fill(&mut block);
        assert_eq!(block, [1.0, -1.0]);
    }

    #[test]
    fn a_sound_continues_across_blocks_and_stops_on_command() {
        let (mut mixer, commands, done, _blocks) = mixer();
        commands
            .send(Command::Play(voice(1, &[0.1; 6], 1.0)))
            .unwrap();
        commands
            .send(Command::Play(voice(2, &[0.2; 6], 1.0)))
            .unwrap();
        let mut block = [0.0; 2];
        mixer.fill(&mut block);
        assert_eq!(block, [0.1 + 0.2; 2]);
        assert!(done.try_recv().is_err());

        commands.send(Command::Stop(2)).unwrap();
        mixer.fill(&mut block);
        assert_eq!(block, [0.1; 2]);
        assert_eq!(done.try_recv().unwrap().id, 2);

        commands.send(Command::StopAll).unwrap();
        mixer.fill(&mut block);
        assert_eq!(block, [0.0; 2]);
        assert_eq!(done.try_recv().unwrap().id, 1);
    }

    #[test]
    fn an_ended_sound_waits_for_room_to_be_handed_back() {
        let (mut mixer, commands, done, _blocks) = mixer();
        let mut block = [0.0; 1];
        // Fill the return channel, then end one more sound.
        for id in 0..(MAX_VOICES as u64 * 4) {
            commands
                .send(Command::Play(voice(id, &[0.0], 1.0)))
                .unwrap();
            mixer.fill(&mut block);
        }
        commands
            .send(Command::Play(voice(999, &[0.0], 1.0)))
            .unwrap();
        mixer.fill(&mut block);
        let first: Vec<u64> = done.try_iter().map(|voice| voice.id).collect();
        assert!(!first.contains(&999));
        mixer.fill(&mut block);
        assert_eq!(done.try_recv().unwrap().id, 999);
    }

    #[test]
    fn a_repeating_sound_wraps_until_it_is_stopped() {
        let (mut mixer, commands, done, _blocks) = mixer();
        let mut repeating = voice(1, &[0.1, 0.2, 0.3], 1.0);
        repeating.looped = true;
        commands.send(Command::Play(repeating)).unwrap();
        let mut block = [0.0; 5];
        mixer.fill(&mut block);
        assert_eq!(block, [0.1, 0.2, 0.3, 0.1, 0.2]);
        mixer.fill(&mut block);
        assert_eq!(block, [0.3, 0.1, 0.2, 0.3, 0.1]);
        assert!(done.try_recv().is_err());
        // An empty repeating sound ends at once instead of spinning.
        let mut empty = voice(2, &[], 1.0);
        empty.looped = true;
        commands.send(Command::Play(empty)).unwrap();
        commands.send(Command::Stop(1)).unwrap();
        mixer.fill(&mut block);
        assert_eq!(block, [0.0; 5]);
        let mut ended: Vec<u64> = done.try_iter().map(|voice| voice.id).collect();
        ended.sort_unstable();
        assert_eq!(ended, [1, 2]);
    }

    #[test]
    fn pause_holds_position_and_volume_scales_the_output() {
        let (mut mixer, commands, done, _blocks) = mixer();
        commands
            .send(Command::Play(voice(1, &[0.2, 0.4, 0.6, 0.8], 1.0)))
            .unwrap();
        let mut block = [0.0; 1];
        mixer.fill(&mut block);
        assert_eq!(block, [0.2]);

        commands.send(Command::Pause).unwrap();
        mixer.fill(&mut block);
        mixer.fill(&mut block);
        assert_eq!(block, [0.0]);
        commands.send(Command::Resume).unwrap();
        commands.send(Command::Volume(0.5)).unwrap();
        mixer.fill(&mut block);
        assert_eq!(block, [0.2]);

        // A sound stopped while paused is handed back without playing on.
        commands.send(Command::Pause).unwrap();
        commands.send(Command::StopAll).unwrap();
        mixer.fill(&mut block);
        assert_eq!(block, [0.0]);
        assert_eq!(done.try_recv().unwrap().id, 1);
    }

    #[test]
    fn seek_moves_every_sound_and_ends_the_ones_it_passes() {
        let (mut mixer, commands, done, _blocks) = mixer();
        commands
            .send(Command::Play(voice(1, &[0.1, 0.2, 0.3, 0.4], 1.0)))
            .unwrap();
        commands.send(Command::Play(voice(2, &[0.5], 1.0))).unwrap();
        let mut repeating = voice(3, &[0.01, 0.02, 0.03], 1.0);
        repeating.looped = true;
        commands.send(Command::Play(repeating)).unwrap();
        commands.send(Command::Seek(2)).unwrap();
        let mut block = [0.0; 1];
        mixer.fill(&mut block);
        assert!((block[0] - 0.33).abs() < 1e-6);
        assert_eq!(done.try_recv().unwrap().id, 2);
        // Past the end of the first sound; the repeating sound wraps the offset.
        commands.send(Command::Seek(4)).unwrap();
        mixer.fill(&mut block);
        assert!((block[0] - 0.02).abs() < 1e-6);
        assert_eq!(done.try_recv().unwrap().id, 1);
    }
}
