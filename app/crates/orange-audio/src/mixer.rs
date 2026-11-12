//! The part of playback that runs on the device's callback.
//!
//! [`Mixer::fill`] is all a callback does. It takes commands from a bounded channel
//! without waiting, adds every playing sound into the block and hands finished sounds
//! back on a second bounded channel, so sample memory is freed on a control thread. It
//! does not allocate, block, log or touch a file.
use std::sync::Arc;
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
}

#[derive(Debug)]
pub(crate) enum Command {
    Play(Voice),
    Stop(u64),
    StopAll,
}

#[derive(Debug)]
pub(crate) struct Mixer {
    voices: [Option<Voice>; MAX_VOICES],
    commands: Receiver<Command>,
    retired: SyncSender<Voice>,
}

/// The mixer and the two channel ends its controller keeps.
pub(crate) fn mixer() -> (Mixer, SyncSender<Command>, Receiver<Voice>) {
    // Every voice may be stopped and replaced within one block.
    let (commands, receive) = sync_channel(MAX_VOICES * 4);
    let (retired, done) = sync_channel(MAX_VOICES * 4);
    let mixer = Mixer {
        voices: [const { None }; MAX_VOICES],
        commands: receive,
        retired,
    };
    (mixer, commands, done)
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
                            voice.at = voice.sound.len();
                        }
                    }
                }
                Command::StopAll => {
                    for voice in self.voices.iter_mut().flatten() {
                        voice.at = voice.sound.len();
                    }
                }
            }
        }
        out.fill(0.0);
        for slot in &mut self.voices {
            let Some(voice) = slot else { continue };
            let rest = voice.sound.get(voice.at..).unwrap_or_default();
            let count = rest.len().min(out.len());
            for (mixed, sample) in out.iter_mut().zip(rest) {
                *mixed += sample * voice.gain;
            }
            voice.at += count;
            if voice.at >= voice.sound.len()
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
        }
    }

    #[test]
    fn sounds_are_added_scaled_and_limited() {
        let (mut mixer, commands, done) = mixer();
        commands
            .send(Command::Play(voice(1, &[0.5, 0.5, 0.5], 1.0)))
            .unwrap();
        commands
            .send(Command::Play(voice(2, &[0.8, -0.25], 0.5)))
            .unwrap();
        let mut block = [9.0; 4];
        mixer.fill(&mut block);
        assert_eq!(block, [0.9, 0.375, 0.5, 0.0]);
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
        let (mut mixer, commands, done) = mixer();
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
        let (mut mixer, commands, done) = mixer();
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
}
