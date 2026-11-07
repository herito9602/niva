//! `AudioCue` on `rodio`: the reminder chime and the "listening" cue, played by
//! the host itself so they work when the voice sidecar is down (design
//! "Fullscreen policy": not the webview, not the sidecar).
//!
//! The sounds are generated sine tones, so no decoder, no sound file and no
//! extra codec crate are needed (`rodio` is built with `playback` only).
//!
//! One thread owns the output device: the device handle is not `Send` on every
//! platform and opening it can take a moment, so the port only sends commands
//! and never blocks. The device is opened at the first cue, and re-opened at
//! the next one if it failed (a headset plugged in later). Without any output
//! device a cue is skipped and its `done` still runs, so a reminder alert can
//! end.

use std::sync::Mutex;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::Duration;

use niva_core::ports::AudioCue;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player, Source, source::SineWave};

/// One note of a cue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tone {
    pub hz: f32,
    pub ms: u64,
    /// Linear gain, 0.0 to 1.0. Cues are soft on purpose.
    pub gain: f32,
}

/// Two rising notes: "something needs your attention".
pub const CHIME: [Tone; 2] = [
    Tone {
        hz: 880.0,
        ms: 140,
        gain: 0.18,
    },
    Tone {
        hz: 1318.5,
        ms: 260,
        gain: 0.18,
    },
];

/// One short, quiet note: "I am listening".
pub const LISTENING: [Tone; 1] = [Tone {
    hz: 660.0,
    ms: 90,
    gain: 0.12,
}];

type Done = Box<dyn FnOnce() + Send>;

enum Cmd {
    Play(&'static [Tone], Option<Done>),
    Stop,
}

pub struct RodioCue {
    tx: Mutex<Sender<Cmd>>,
}

impl RodioCue {
    pub fn new() -> Self {
        let (tx, rx) = channel::<Cmd>();
        let spawned = thread::Builder::new()
            .name("niva-audio-cue".into())
            .spawn(move || {
                let mut device: Option<MixerDeviceSink> = None;
                while let Ok(cmd) = rx.recv() {
                    let Cmd::Play(tones, done) = cmd else {
                        continue; // nothing is playing
                    };
                    if device.is_none() {
                        match DeviceSinkBuilder::open_default_sink() {
                            Ok(mut sink) => {
                                sink.log_on_drop(false);
                                device = Some(sink);
                            }
                            Err(err) => eprintln!("[niva-audio] no output device: {err}"),
                        }
                    }
                    if let Some(sink) = &device {
                        let player = Player::connect_new(sink.mixer());
                        for tone in tones {
                            player.append(
                                SineWave::new(tone.hz)
                                    .take_duration(Duration::from_millis(tone.ms))
                                    .fade_in(Duration::from_millis(8))
                                    .amplify(tone.gain),
                            );
                        }
                        while !player.empty() {
                            match rx.recv_timeout(Duration::from_millis(20)) {
                                Ok(Cmd::Stop) => {
                                    player.stop();
                                    break;
                                }
                                // A cue that arrives during another is skipped.
                                Ok(Cmd::Play(_, skipped)) => {
                                    if let Some(done) = skipped {
                                        done();
                                    }
                                }
                                Err(RecvTimeoutError::Timeout) => {}
                                Err(RecvTimeoutError::Disconnected) => return,
                            }
                        }
                    }
                    if let Some(done) = done {
                        done();
                    }
                }
            });
        if let Err(why) = spawned {
            eprintln!("[niva-audio] could not start the cue thread: {why}");
        }
        RodioCue { tx: Mutex::new(tx) }
    }

    fn send(&self, cmd: Cmd) -> Result<(), Cmd> {
        let tx = self.tx.lock().unwrap_or_else(|e| e.into_inner());
        tx.send(cmd).map_err(|e| e.0)
    }
}

impl Default for RodioCue {
    fn default() -> Self {
        Self::new()
    }
}

impl AudioCue for RodioCue {
    fn chime(&self, done: Done) {
        // No thread left to play it: the alert must still be able to end.
        if let Err(Cmd::Play(_, Some(done))) = self.send(Cmd::Play(&CHIME, Some(done))) {
            done();
        }
    }

    fn listening_cue(&self) {
        let _ = self.send(Cmd::Play(&LISTENING, None));
    }

    fn stop(&self) {
        let _ = self.send(Cmd::Stop);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn total_ms(tones: &[Tone]) -> u64 {
        tones.iter().map(|t| t.ms).sum()
    }

    #[test]
    fn the_cues_are_soft_short_and_audible() {
        for tones in [&CHIME[..], &LISTENING[..]] {
            for tone in tones {
                assert!(tone.gain > 0.0 && tone.gain <= 0.25, "{tone:?}");
                assert!((200.0..=4_000.0).contains(&tone.hz), "{tone:?}");
            }
        }
        assert!(total_ms(&CHIME) <= 600);
        assert!(total_ms(&LISTENING) <= 150);
    }

    #[test]
    fn the_listening_cue_is_quieter_than_the_chime() {
        assert!(LISTENING[0].gain < CHIME[0].gain);
    }

    #[test]
    fn a_chime_whose_thread_is_gone_still_reports_done() {
        // Build the adapter without its thread: the receiver is dropped at once.
        let (tx, rx) = channel::<Cmd>();
        drop(rx);
        let cue = RodioCue { tx: Mutex::new(tx) };
        let (done_tx, done_rx) = channel();
        cue.chime(Box::new(move || {
            let _ = done_tx.send(());
        }));
        assert!(done_rx.try_recv().is_ok());
        cue.listening_cue();
        cue.stop();
    }
}
