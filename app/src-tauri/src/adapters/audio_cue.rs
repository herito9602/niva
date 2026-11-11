//! `AudioCue` on `rodio`: the reminder chime and the "listening" cue, played by
//! the host itself so they work when the voice sidecar is down (design
//! "Fullscreen policy": not the webview, not the sidecar).
//!
//! The sounds are generated (sine tones with an optional octave and release),
//! so no decoder, no sound file and no extra codec crate are needed (`rodio` is
//! built with `playback` only).
//!
//! One thread owns the output device: the device handle is not `Send` on every
//! platform and opening it can take a moment, so the port only sends commands
//! and never blocks. The device is opened at startup in the background (so
//! the first cue is not late), and re-opened at the next cue if that failed (a headset plugged in later). Without any output
//! device a cue is skipped and its `done` still runs, so a reminder alert can
//! end.

use std::f32::consts::TAU;
use std::num::NonZero;
use std::sync::Mutex;
use std::sync::mpsc::{RecvTimeoutError, Sender, channel};
use std::thread;
use std::time::Duration;

use niva_core::ports::AudioCue;
use rodio::{DeviceSinkBuilder, MixerDeviceSink, Player, buffer::SamplesBuffer};

/// One note of a cue.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Tone {
    pub hz: f32,
    pub ms: u64,
    /// Linear gain, 0.0 to 1.0. Cues are soft on purpose.
    pub gain: f32,
    /// Fade-in, so a note never starts with a click.
    pub attack_ms: u64,
    /// Time constant of the exponential release; 0 keeps the note flat.
    pub release_ms: u64,
    /// Gain of the 2nd harmonic relative to the fundamental; 0 is a pure sine.
    pub harmonic: f32,
}

/// Sample rate of the rendered cues; the mixer resamples to the device.
const SAMPLE_RATE: u32 = 44_100;

/// Renders the notes one after the other into mono samples. Each note starts
/// from silence: its fade-in restarts and its phase begins at zero.
fn render(tones: &[Tone]) -> Vec<f32> {
    let rate = SAMPLE_RATE as f32;
    let mut out = Vec::new();
    for tone in tones {
        let count = (SAMPLE_RATE as u64 * tone.ms / 1000) as usize;
        let attack = tone.attack_ms as f32 / 1000.0;
        let release = tone.release_ms as f32 / 1000.0;
        for i in 0..count {
            let t = i as f32 / rate;
            let fade_in = if attack > 0.0 {
                (t / attack).min(1.0)
            } else {
                1.0
            };
            let fade_out = if release > 0.0 {
                (-t / release).exp()
            } else {
                1.0
            };
            let phase = TAU * tone.hz * t;
            let wave = phase.sin() + tone.harmonic * (2.0 * phase).sin();
            out.push(tone.gain * wave * fade_in * fade_out);
        }
    }
    out
}

/// Two rising notes: "something needs your attention".
pub const CHIME: [Tone; 2] = [
    Tone {
        hz: 880.0,
        ms: 140,
        gain: 0.18,
        attack_ms: 8,
        release_ms: 0,
        harmonic: 0.0,
    },
    Tone {
        hz: 1318.5,
        ms: 260,
        gain: 0.18,
        attack_ms: 8,
        release_ms: 0,
        harmonic: 0.0,
    },
];

/// "I heard you": two warm rising notes, C5 then G5 (owner's pick, "soft
/// ascending"). Each is a sine plus a quiet octave with a quick release, so it
/// is brief and far less energetic than the chime despite its higher first peak.
pub const LISTENING: [Tone; 2] = [
    Tone {
        hz: 523.0,
        ms: 80,
        gain: 0.25,
        attack_ms: 5,
        release_ms: 60,
        harmonic: 0.2,
    },
    Tone {
        hz: 784.0,
        ms: 160,
        gain: 0.25,
        attack_ms: 5,
        release_ms: 100,
        harmonic: 0.2,
    },
];

type Done = Box<dyn FnOnce() + Send>;

enum Cmd {
    Play(&'static [Tone], Option<Done>),
    Stop,
}

/// A playing cue, as far as the audio thread needs to know.
trait Playback {
    fn is_done(&self) -> bool;
    fn stop(&self);
}

/// An opened output device.
trait Output {
    fn play(&self, samples: Vec<f32>) -> Box<dyn Playback>;
}

/// Opens the output device; the seam that lets a test count the opens.
type Opener = Box<dyn FnMut() -> Result<Box<dyn Output>, String> + Send>;

struct RodioPlayback(Player);

impl Playback for RodioPlayback {
    fn is_done(&self) -> bool {
        self.0.empty()
    }
    fn stop(&self) {
        self.0.stop();
    }
}

struct RodioOutput(MixerDeviceSink);

impl Output for RodioOutput {
    fn play(&self, samples: Vec<f32>) -> Box<dyn Playback> {
        let player = Player::connect_new(self.0.mixer());
        if let (Some(channels), Some(rate)) = (NonZero::new(1), NonZero::new(SAMPLE_RATE)) {
            player.append(SamplesBuffer::new(channels, rate, samples));
        }
        Box::new(RodioPlayback(player))
    }
}

fn open_default() -> Result<Box<dyn Output>, String> {
    let mut sink = DeviceSinkBuilder::open_default_sink().map_err(|err| err.to_string())?;
    sink.log_on_drop(false);
    Ok(Box::new(RodioOutput(sink)))
}

pub struct RodioCue {
    tx: Mutex<Sender<Cmd>>,
}

impl RodioCue {
    pub fn new() -> Self {
        Self::with_opener(Box::new(open_default))
    }

    /// The audio thread opens the device right away, in the background, so the
    /// first cue is not late; if that fails it tries again at the next cue.
    fn with_opener(mut open: Opener) -> Self {
        let (tx, rx) = channel::<Cmd>();
        let spawned = thread::Builder::new()
            .name("niva-audio-cue".into())
            .spawn(move || {
                let mut device: Option<Box<dyn Output>> = None;
                let mut logged = false;
                let mut try_open = |device: &mut Option<Box<dyn Output>>| {
                    if device.is_some() {
                        return;
                    }
                    match open() {
                        Ok(out) => *device = Some(out),
                        Err(err) if !logged => {
                            logged = true;
                            eprintln!("[niva-audio] no output device: {err}");
                        }
                        Err(_) => {}
                    }
                };
                try_open(&mut device);
                while let Ok(cmd) = rx.recv() {
                    let Cmd::Play(tones, done) = cmd else {
                        continue; // nothing is playing
                    };
                    try_open(&mut device);
                    if let Some(out) = &device {
                        let player = out.play(render(tones));
                        while !player.is_done() {
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

    fn listening_cue(&self, done: Done) {
        if let Err(Cmd::Play(_, Some(done))) = self.send(Cmd::Play(&LISTENING, Some(done))) {
            done();
        }
    }

    fn listening_cue_len(&self) -> Duration {
        Duration::from_millis(LISTENING.iter().map(|tone| tone.ms).sum())
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

    fn rms(samples: &[f32]) -> f32 {
        (samples.iter().map(|s| s * s).sum::<f32>() / samples.len() as f32).sqrt()
    }

    fn peak(samples: &[f32]) -> f32 {
        samples.iter().fold(0.0, |top, s| top.max(s.abs()))
    }

    fn samples_of(ms: u64) -> usize {
        (SAMPLE_RATE as u64 * ms / 1000) as usize
    }

    #[test]
    fn the_cues_are_soft_short_and_audible() {
        for tones in [&CHIME[..], &LISTENING[..]] {
            for tone in tones {
                assert!(tone.gain > 0.0 && tone.gain <= 0.25, "{tone:?}");
                assert!((200.0..=4_000.0).contains(&tone.hz), "{tone:?}");
            }
            assert!(peak(&render(tones)) <= 0.3, "{tones:?}");
        }
        assert!(total_ms(&CHIME) <= 600);
        assert!(total_ms(&LISTENING) <= 260);
    }

    #[test]
    fn the_acknowledgement_is_two_warm_rising_notes() {
        // "7 - soft ascending": C5 then G5, each a sine plus a quiet octave.
        assert_eq!(LISTENING.len(), 2);
        assert_eq!((LISTENING[0].hz, LISTENING[0].ms), (523.0, 80));
        assert_eq!((LISTENING[1].hz, LISTENING[1].ms), (784.0, 160));
        for tone in LISTENING {
            assert_eq!(tone.gain, 0.25);
            assert_eq!(tone.harmonic, 0.2);
            assert_eq!(tone.attack_ms, 5);
        }
        assert_eq!(LISTENING[0].release_ms, 60);
        assert_eq!(LISTENING[1].release_ms, 100);
    }

    #[test]
    fn the_acknowledgement_is_quieter_than_the_chime() {
        // Its first peak is a little higher (the owner's pick) but it dies away
        // at once, so its energy stays below the flat chime.
        assert!(rms(&render(&LISTENING)) < rms(&render(&CHIME)));
    }

    #[test]
    fn a_rendered_note_fades_in_from_silence_and_dies_away() {
        let audio = render(&LISTENING);
        let first = samples_of(80);
        assert_eq!(audio.len(), first + samples_of(160));
        // Each note starts from zero (no click) and its attack is 5 ms.
        assert_eq!(audio[0], 0.0);
        assert_eq!(audio[first], 0.0);
        assert!(peak(&audio[..samples_of(2)]) < 0.5 * peak(&audio[..first]));
        // The release leaves the end of the second note well below its start.
        let second = &audio[first..];
        let tail = &second[second.len() - samples_of(5)..];
        assert!(peak(tail) < 0.3 * peak(second), "{}", peak(tail));
        // The second note carries the octave on top of the fundamental.
        let pure = render(&[Tone {
            harmonic: 0.0,
            ..LISTENING[1]
        }]);
        assert!(pure != second);
    }

    #[test]
    fn a_flat_tone_has_no_release() {
        let audio = render(&CHIME[..1]);
        let tail = &audio[audio.len() - samples_of(20)..];
        assert!(peak(tail) > 0.15, "{}", peak(tail));
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
        let (done_tx, done_rx) = channel();
        cue.listening_cue(Box::new(move || {
            let _ = done_tx.send(());
        }));
        assert!(done_rx.try_recv().is_ok());
        assert_eq!(cue.listening_cue_len(), Duration::from_millis(240));
        cue.stop();
    }

    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};

    struct FakePlayback;
    impl Playback for FakePlayback {
        fn is_done(&self) -> bool {
            true
        }
        fn stop(&self) {}
    }
    struct FakeOutput;
    impl Output for FakeOutput {
        fn play(&self, _samples: Vec<f32>) -> Box<dyn Playback> {
            Box::new(FakePlayback)
        }
    }

    fn wait_done(cue: &RodioCue) {
        let (done_tx, done_rx) = channel();
        cue.listening_cue(Box::new(move || {
            let _ = done_tx.send(());
        }));
        assert!(done_rx.recv_timeout(Duration::from_secs(2)).is_ok());
    }

    #[test]
    fn the_device_is_opened_at_startup_and_not_again_on_the_first_cue() {
        let opens = Arc::new(AtomicUsize::new(0));
        let counter = opens.clone();
        let cue = RodioCue::with_opener(Box::new(move || {
            counter.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeOutput) as Box<dyn Output>)
        }));
        // Before any cue is played.
        let start = std::time::Instant::now();
        while opens.load(Ordering::SeqCst) == 0 && start.elapsed() < Duration::from_secs(2) {
            thread::sleep(Duration::from_millis(5));
        }
        assert_eq!(opens.load(Ordering::SeqCst), 1);
        wait_done(&cue);
        wait_done(&cue);
        assert_eq!(opens.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn without_an_output_device_the_cue_is_skipped_and_done_still_runs() {
        let cue = RodioCue::with_opener(Box::new(|| Err("no device".to_string())));
        wait_done(&cue);
        let (done_tx, done_rx) = channel();
        cue.chime(Box::new(move || {
            let _ = done_tx.send(());
        }));
        assert!(done_rx.recv_timeout(Duration::from_secs(2)).is_ok());
    }
}
