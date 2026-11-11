//! The supervisor against a fake sidecar: in-memory pipes and a process whose
//! exit the test decides. No Python, microphone, GPU or network.

use std::collections::VecDeque;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, Sender, channel};
use std::time::{Duration, Instant};

use niva_core::replies::Lang;
use niva_core::state_machine::{ListenMode, SidecarErrorCode};

use super::*;
use crate::adapters::voice_sidecar::policy::Degraded;
use crate::adapters::voice_sidecar::process::Spawned;
use crate::adapters::voice_sidecar::wire::Level;
use niva_core::ports::{Speaker, SpeechEngine};

// ---------------------------------------------------------------- the fake

struct ChanReader {
    rx: Receiver<Vec<u8>>,
    rest: Vec<u8>,
}

impl Read for ChanReader {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if self.rest.is_empty() {
            match self.rx.recv() {
                Ok(bytes) => self.rest = bytes,
                Err(_) => return Ok(0),
            }
        }
        let n = self.rest.len().min(buf.len());
        buf[..n].copy_from_slice(&self.rest[..n]);
        self.rest.drain(..n);
        Ok(n)
    }
}

struct RecordingStdin {
    bytes: Arc<Mutex<Vec<u8>>>,
    closed: Arc<AtomicBool>,
}

impl Write for RecordingStdin {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Drop for RecordingStdin {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::SeqCst);
    }
}

type StdoutTx = Arc<Mutex<Option<Sender<Vec<u8>>>>>;

/// The test's end of one fake sidecar.
struct Fake {
    model: String,
    stdout: StdoutTx,
    stdin_bytes: Arc<Mutex<Vec<u8>>>,
    stdin_closed: Arc<AtomicBool>,
    exit: Arc<Mutex<Option<Option<i32>>>>,
    killed: Arc<AtomicBool>,
}

impl Fake {
    fn say(&self, line: &str) {
        if let Some(tx) = self.stdout.lock().unwrap().as_ref() {
            let _ = tx.send(format!("{line}\n").into_bytes());
        }
    }

    fn say_raw(&self, bytes: &[u8]) {
        if let Some(tx) = self.stdout.lock().unwrap().as_ref() {
            let _ = tx.send(bytes.to_vec());
        }
    }

    fn ready(&self) {
        self.say(r#"{"type":"ready","now_ms":1000}"#);
    }

    fn exit_with(&self, code: i32) {
        *self.exit.lock().unwrap() = Some(Some(code));
        self.stdout.lock().unwrap().take();
    }

    fn stdin_text(&self) -> String {
        String::from_utf8_lossy(&self.stdin_bytes.lock().unwrap()).into_owned()
    }

    fn was_killed(&self) -> bool {
        self.killed.load(Ordering::SeqCst)
    }
}

struct FakeProc {
    exit: Arc<Mutex<Option<Option<i32>>>>,
    killed: Arc<AtomicBool>,
    stdin_closed: Arc<AtomicBool>,
    stdout: StdoutTx,
    /// A well-behaved sidecar ends when its stdin is closed.
    exits_on_eof: bool,
}

impl Proc for FakeProc {
    fn try_exit(&mut self) -> io::Result<Option<Option<i32>>> {
        let mut exit = self.exit.lock().unwrap();
        if exit.is_none() && self.exits_on_eof && self.stdin_closed.load(Ordering::SeqCst) {
            *exit = Some(Some(0));
            self.stdout.lock().unwrap().take();
        }
        Ok(*exit)
    }

    fn kill_tree(&mut self) {
        self.killed.store(true, Ordering::SeqCst);
        self.exit.lock().unwrap().get_or_insert(None);
        self.stdout.lock().unwrap().take();
    }
}

#[derive(Clone, Copy)]
struct Plan {
    fail: bool,
    /// The failure is "voice turned off" rather than "cannot start".
    off: bool,
    exits_on_eof: bool,
}

const NORMAL: Plan = Plan {
    fail: false,
    off: false,
    exits_on_eof: true,
};

#[derive(Default)]
struct FakeLauncher {
    plans: Mutex<VecDeque<Plan>>,
    runs: Mutex<Vec<Arc<Fake>>>,
}

impl FakeLauncher {
    fn plan(&self, plan: Plan) {
        self.plans.lock().unwrap().push_back(plan);
    }

    fn run(&self, n: usize) -> Arc<Fake> {
        wait_for(|| self.runs.lock().unwrap().len() > n);
        self.runs.lock().unwrap()[n].clone()
    }

    fn count(&self) -> usize {
        self.runs.lock().unwrap().len()
    }
}

impl Launcher for FakeLauncher {
    fn spawn(&self, whisper: &str) -> io::Result<Spawned> {
        let plan = self.plans.lock().unwrap().pop_front().unwrap_or(NORMAL);
        if plan.fail {
            let kind = if plan.off {
                io::ErrorKind::Unsupported
            } else {
                io::ErrorKind::NotFound
            };
            return Err(io::Error::new(kind, "no such program"));
        }
        let (tx, rx) = channel();
        let stdout: StdoutTx = Arc::new(Mutex::new(Some(tx)));
        let fake = Arc::new(Fake {
            model: whisper.to_owned(),
            stdout: stdout.clone(),
            stdin_bytes: Arc::default(),
            stdin_closed: Arc::default(),
            exit: Arc::default(),
            killed: Arc::default(),
        });
        self.runs.lock().unwrap().push(fake.clone());
        Ok(Spawned {
            stdin: Box::new(RecordingStdin {
                bytes: fake.stdin_bytes.clone(),
                closed: fake.stdin_closed.clone(),
            }),
            stdout: Box::new(ChanReader {
                rx,
                rest: Vec::new(),
            }),
            stderr: Box::new(io::empty()),
            proc: Box::new(FakeProc {
                exit: fake.exit.clone(),
                killed: fake.killed.clone(),
                stdin_closed: fake.stdin_closed.clone(),
                stdout,
                exits_on_eof: plan.exits_on_eof,
            }),
        })
    }
}

fn wait_for(mut condition: impl FnMut() -> bool) {
    let until = Instant::now() + Duration::from_secs(5);
    while !condition() {
        assert!(
            Instant::now() < until,
            "timed out waiting for the condition"
        );
        thread::sleep(Duration::from_millis(5));
    }
}

// ---------------------------------------------------------------- the rig

struct Rig {
    sup: Supervisor,
    launcher: Arc<FakeLauncher>,
    link: Arc<Link>,
    modes: Arc<ListenModes>,
    events: Arc<Mutex<Vec<Event>>>,
    statuses: Arc<Mutex<Vec<Status>>>,
    logs: Arc<Mutex<Vec<String>>>,
    levels: Arc<Mutex<Vec<Level>>>,
}

fn fast_settings() -> Settings {
    let mut settings = Settings::new(vec!["small".into(), "base".into()]);
    settings.timing = Timing {
        ready_timeout_ms: 400,
        stable_after_ms: 60_000,
        restart_delays_ms: [20, 20, 20],
        downgrade_delay_ms: 10,
        exit_grace_ms: 150,
        slow_retry_ms: 60_000,
    };
    settings.poll = Duration::from_millis(5);
    settings.drain = Duration::from_millis(100);
    settings.stop_grace = Duration::from_millis(300);
    settings
}

fn rig_with(settings: Settings, plans: &[Plan]) -> Rig {
    let launcher = Arc::new(FakeLauncher::default());
    for plan in plans {
        launcher.plan(*plan);
    }
    let modes = Arc::new(ListenModes::default());
    let link = Link::new(modes.clone());
    let events = Arc::new(Mutex::new(Vec::new()));
    let statuses = Arc::new(Mutex::new(Vec::new()));
    let logs = Arc::new(Mutex::new(Vec::new()));
    let levels = Arc::new(Mutex::new(Vec::new()));
    let started = Instant::now();
    let sup = Supervisor::start(Deps {
        launcher: launcher.clone(),
        link: link.clone(),
        modes: modes.clone(),
        events: {
            let events = events.clone();
            Arc::new(move |event| events.lock().unwrap().push(event))
        },
        clock: Arc::new(move || started.elapsed().as_millis() as u64),
        log: {
            let logs = logs.clone();
            Arc::new(move |line| logs.lock().unwrap().push(line))
        },
        status: {
            let statuses = statuses.clone();
            Arc::new(move |s| statuses.lock().unwrap().push(s))
        },
        levels: {
            let levels = levels.clone();
            Arc::new(move |level| levels.lock().unwrap().push(level))
        },
        settings,
    });
    Rig {
        sup,
        launcher,
        link,
        modes,
        events,
        statuses,
        logs,
        levels,
    }
}

fn rig() -> Rig {
    rig_with(fast_settings(), &[])
}

impl Rig {
    fn events(&self) -> Vec<Event> {
        self.events.lock().unwrap().clone()
    }

    fn wait_event(&self, wanted: Event) {
        wait_for(|| self.events().contains(&wanted));
    }

    fn wait_status(&self, wanted: Status) {
        wait_for(|| self.statuses.lock().unwrap().contains(&wanted));
    }

    fn voice_events(&self) -> Vec<bool> {
        self.events()
            .into_iter()
            .filter_map(|e| match e {
                Event::VoiceReadinessChanged { ready } => Some(ready),
                _ => None,
            })
            .collect()
    }
}

const READY_ON: Event = Event::VoiceReadinessChanged { ready: true };
const READY_OFF: Event = Event::VoiceReadinessChanged { ready: false };

// ---------------------------------------------------------------- tests

#[test]
fn ready_turns_voice_on_for_the_reducer_and_the_ports() {
    let r = rig();
    let fake = r.launcher.run(0);
    assert_eq!(fake.model, "small");
    assert!(!r.link.is_up());
    fake.ready();
    r.wait_event(READY_ON);
    assert!(r.link.is_up());
    assert!(r.link.is_available());
}

#[test]
fn sidecar_messages_become_reducer_events() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    assert!(r.link.listen(4, ListenMode::Command));
    fake.say(r#"{"type":"wake","score":0.8}"#);
    fake.say(r#"{"type":"speech_start","id":4}"#);
    fake.say(
        r#"{"type":"transcript","id":4,"text":"abre Spotify","lang":"es","lang_prob":0.97,"end_ms":1200}"#,
    );
    fake.say(r#"{"type":"no_speech","id":5}"#);
    fake.say(r#"{"type":"speak_started","id":6}"#);
    fake.say(r#"{"type":"speak_done","id":6}"#);
    fake.say(r#"{"type":"error","code":"TTS_FAILED","msg":"x","id":7}"#);
    r.wait_event(Event::TtsFailed { id: 7 });
    let got = r.events();
    for wanted in [
        Event::Wake,
        Event::SpeechStart { id: 4 },
        Event::Transcript {
            id: 4,
            text: "abre Spotify".into(),
            lang: Some(Lang::Es),
        },
        Event::NoSpeech { id: 5 },
        Event::SpeakStarted { id: 6 },
        Event::SpeakDone { id: 6 },
    ] {
        assert!(got.contains(&wanted), "{wanted:?} missing from {got:?}");
    }
    // TTS_FAILED is not a voice-down error.
    assert!(r.link.is_up());
}

#[test]
fn port_calls_are_written_to_the_sidecar_as_json_lines() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    assert!(r.link.listen(7, ListenMode::Confirm));
    assert!(r.link.speak(8, "Hola", Lang::Es));
    assert!(r.link.force_end_of_speech(7));
    assert!(r.link.cancel_listen(7));
    assert!(r.link.stop());
    wait_for(|| fake.stdin_text().lines().count() == 5);
    let text = fake.stdin_text();
    let lines: Vec<&str> = text.lines().collect();
    let first: serde_json::Value = serde_json::from_str(lines[0]).unwrap();
    assert_eq!(
        first,
        serde_json::json!({"type": "listen", "id": 7, "mode": "confirm"})
    );
    assert!(lines[1].contains(r#""text":"Hola""#) && lines[1].contains(r#""lang":"es""#));
    assert!(lines[2].contains("force_end") && lines[3].contains("cancel_listen"));
    assert!(lines[4].contains("stop_speaking"));
}

#[test]
fn malformed_and_unknown_lines_are_ignored_and_the_stream_goes_on() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.say("this is not json");
    fake.say("[1, 2, 3]");
    fake.say(r#"{"type":"teleport"}"#);
    fake.say(r#"{"type":"transcript","id":"x","text":"the secret"}"#);
    fake.say_raw(b"\xff\xfe\xfd garbage\n");
    fake.say_raw(&vec![b'z'; 300_000]);
    fake.say_raw(b"\n");
    fake.ready();
    fake.say(r#"{"type":"wake","score":0.9}"#);
    r.wait_event(Event::Wake);
    assert_eq!(r.voice_events(), vec![true]);
    let logs = r.logs.lock().unwrap().join("\n");
    assert!(logs.contains("unreadable line"));
    assert!(
        !logs.contains("secret"),
        "a rejected line was echoed: {logs}"
    );
    assert_eq!(
        r.launcher.count(),
        1,
        "garbage must not restart the sidecar"
    );
}

#[test]
fn a_crash_turns_voice_off_and_restarts_the_sidecar() {
    let r = rig();
    let first = r.launcher.run(0);
    first.ready();
    r.wait_event(READY_ON);
    first.exit_with(1);
    r.wait_event(READY_OFF);
    let second = r.launcher.run(1);
    assert!(!r.link.is_up());
    assert_eq!(r.voice_events(), vec![true, false]);
    assert_eq!(second.model, "small");
    second.ready();
    wait_for(|| r.voice_events() == vec![true, false, true]);
    assert!(r.link.is_up());
}

#[test]
fn the_fourth_crash_ends_in_text_mode_and_restart_voice_brings_it_back() {
    let r = rig();
    for n in 0..4 {
        let fake = r.launcher.run(n);
        fake.exit_with(1);
    }
    r.wait_status(Status::Degraded(Degraded::Crashed));
    assert_eq!(r.launcher.count(), 4);
    assert!(!r.link.is_up());
    thread::sleep(Duration::from_millis(100));
    assert_eq!(r.launcher.count(), 4, "no more automatic quick restarts");
    r.sup.restart();
    let again = r.launcher.run(4);
    again.ready();
    wait_for(|| r.link.is_up());
}

#[test]
fn out_of_memory_restarts_once_with_the_smaller_model() {
    let r = rig();
    let first = r.launcher.run(0);
    // The error line and the exit arrive together; the line must not be lost.
    first.say(r#"{"type":"error","code":"CUDA_OOM","msg":"out of memory"}"#);
    first.exit_with(3);
    let second = r.launcher.run(1);
    assert_eq!(second.model, "base");
    r.wait_status(Status::Downgraded {
        model: "base".into(),
    });
    assert!(r.events().contains(&Event::SidecarError {
        code: SidecarErrorCode::CudaOom,
        id: None
    }));
    second.ready();
    r.wait_event(READY_ON);
    assert!(r.link.is_up());
}

#[test]
fn the_latency_trace_can_read_the_whisper_model_in_use() {
    let r = rig();
    assert_eq!(r.modes.whisper(), None);
    let first = r.launcher.run(0);
    assert_eq!(r.modes.whisper().as_deref(), Some("small"));
    first.say(r#"{"type":"error","code":"CUDA_OOM","msg":"out of memory"}"#);
    first.exit_with(3);
    let second = r.launcher.run(1);
    assert_eq!(second.model, "base");
    wait_for(|| r.modes.whisper().as_deref() == Some("base"));
}

#[test]
fn out_of_memory_at_the_smallest_model_is_text_mode_without_a_loop() {
    let r = rig();
    for n in 0..2 {
        let fake = r.launcher.run(n);
        fake.say(r#"{"type":"error","code":"CUDA_OOM","msg":"out of memory"}"#);
        fake.exit_with(3);
    }
    r.wait_status(Status::Degraded(Degraded::OutOfMemory));
    thread::sleep(Duration::from_millis(100));
    assert_eq!(r.launcher.count(), 2);
}

#[test]
fn a_model_load_failure_is_text_mode_not_a_retry_loop() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.say(r#"{"type":"error","code":"MODEL_LOAD","msg":"wake model missing"}"#);
    fake.exit_with(3);
    r.wait_status(Status::Degraded(Degraded::LoadFailed));
    thread::sleep(Duration::from_millis(100));
    assert_eq!(r.launcher.count(), 1);
    assert!(r.events().contains(&Event::SidecarError {
        code: SidecarErrorCode::ModelLoad,
        id: None
    }));
}

#[test]
fn a_microphone_error_keeps_the_process_and_ready_brings_voice_back() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    fake.say(r#"{"type":"error","code":"MIC_BUSY","msg":"in use","id":3}"#);
    r.wait_event(Event::SidecarError {
        code: SidecarErrorCode::MicBusy,
        id: Some(3),
    });
    assert!(!r.link.is_up());
    assert!(
        !r.link.listen(9, ListenMode::Command),
        "never a listen to a deaf sidecar"
    );
    // Longer than the ready timeout: the process must not be killed for this.
    thread::sleep(Duration::from_millis(600));
    assert!(!fake.was_killed());
    assert_eq!(r.launcher.count(), 1);
    fake.ready();
    wait_for(|| r.link.is_up());
    assert_eq!(r.voice_events(), vec![true, false, true]);
}

#[test]
fn a_microphone_error_at_start_up_is_not_a_hang() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.say(r#"{"type":"error","code":"MIC_MISSING","msg":"none"}"#);
    thread::sleep(Duration::from_millis(600));
    assert!(!fake.was_killed());
    assert_eq!(r.launcher.count(), 1);
    fake.ready();
    r.wait_event(READY_ON);
}

#[test]
fn a_sidecar_that_never_says_ready_is_killed_and_restarted() {
    let r = rig();
    let hung = r.launcher.run(0);
    // Silence: a hung microphone open sends nothing at all.
    let second = r.launcher.run(1);
    assert!(hung.was_killed());
    assert_eq!(second.model, "small");
    assert!(!r.link.is_up());
}

#[test]
fn a_protocol_error_restarts_the_sidecar() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    fake.say(r#"{"type":"error","code":"PROTOCOL","msg":"bad line"}"#);
    let second = r.launcher.run(1);
    assert!(fake.was_killed());
    assert!(r.events().contains(&Event::SidecarError {
        code: SidecarErrorCode::Protocol,
        id: None
    }));
    second.ready();
    wait_for(|| r.voice_events() == vec![true, false, true]);
}

#[test]
fn an_internal_error_is_followed_by_the_exit_and_a_restart() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    fake.say(r#"{"type":"error","code":"INTERNAL","msg":"boom"}"#);
    fake.exit_with(1);
    let second = r.launcher.run(1);
    assert_eq!(second.model, "small");
    assert!(!r.link.is_up());
}

#[test]
fn a_sidecar_that_cannot_be_started_is_text_mode_and_restart_voice_tries_again() {
    let r = rig_with(
        fast_settings(),
        &[Plan {
            fail: true,
            off: false,
            exits_on_eof: true,
        }],
    );
    r.wait_status(Status::Degraded(Degraded::Unavailable));
    assert!(!r.link.is_up());
    assert_eq!(r.launcher.count(), 0);
    assert!(r.voice_events().contains(&false));
    r.sup.restart();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
}

#[test]
fn restart_voice_replaces_a_running_sidecar() {
    let r = rig();
    let first = r.launcher.run(0);
    first.ready();
    r.wait_event(READY_ON);
    r.sup.restart();
    let second = r.launcher.run(1);
    assert!(first.was_killed());
    assert!(!r.link.is_up());
    second.ready();
    wait_for(|| r.voice_events() == vec![true, false, true]);
}

#[test]
fn shutdown_asks_closes_stdin_and_does_not_kill_a_sidecar_that_exits() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    r.sup.shutdown();
    assert!(fake.stdin_text().contains(r#""type":"shutdown""#));
    assert!(
        fake.stdin_closed.load(Ordering::SeqCst),
        "stdin was not closed"
    );
    assert!(!fake.was_killed());
    assert!(!r.link.is_up());
    assert_eq!(r.voice_events().last(), Some(&false));
}

#[test]
fn shutdown_kills_a_sidecar_that_does_not_exit() {
    let r = rig_with(
        fast_settings(),
        &[Plan {
            fail: false,
            off: false,
            exits_on_eof: false,
        }],
    );
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    let started = Instant::now();
    r.sup.shutdown();
    assert!(fake.was_killed());
    assert!(started.elapsed() < Duration::from_secs(3));
}

#[test]
fn shutdown_while_the_sidecar_is_still_loading_does_not_wait_for_ready() {
    let r = rig();
    let fake = r.launcher.run(0);
    r.sup.shutdown();
    assert!(!fake.was_killed());
    assert!(fake.stdin_closed.load(Ordering::SeqCst));
}

#[test]
fn nothing_restarts_after_shutdown() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    r.sup.shutdown();
    thread::sleep(Duration::from_millis(150));
    assert_eq!(r.launcher.count(), 1);
}

#[test]
fn the_first_start_waits_for_the_gate_but_only_so_long() {
    let open = Arc::new(AtomicBool::new(false));
    let mut settings = fast_settings();
    settings.gate = Some(Gate {
        open: {
            let open = open.clone();
            Arc::new(move || open.load(Ordering::SeqCst))
        },
        max_wait: Duration::from_millis(5_000),
    });
    let r = rig_with(settings, &[]);
    thread::sleep(Duration::from_millis(300));
    assert_eq!(r.launcher.count(), 0, "started before the LLM was warm");
    open.store(true, Ordering::SeqCst);
    r.launcher.run(0);
}

#[test]
fn a_gate_that_never_opens_is_given_up_on() {
    let mut settings = fast_settings();
    settings.gate = Some(Gate {
        open: Arc::new(|| false),
        max_wait: Duration::from_millis(200),
    });
    let r = rig_with(settings, &[]);
    r.launcher.run(0);
}

#[test]
fn shutdown_during_the_gate_never_starts_the_sidecar() {
    let mut settings = fast_settings();
    settings.gate = Some(Gate {
        open: Arc::new(|| false),
        max_wait: Duration::from_millis(60_000),
    });
    let r = rig_with(settings, &[]);
    r.sup.shutdown();
    assert_eq!(r.launcher.count(), 0);
}

#[test]
fn the_logs_carry_no_transcript_and_no_reply_text() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    assert!(r.link.listen(1, ListenMode::Command));
    assert!(r.link.speak(2, "your private reply text", Lang::En));
    fake.say(
        r#"{"type":"transcript","id":1,"text":"my private words","lang":"en","lang_prob":0.9,"end_ms":1100}"#,
    );
    r.wait_event(Event::Transcript {
        id: 1,
        text: "my private words".into(),
        lang: Some(Lang::En),
    });
    let logs = r.logs.lock().unwrap().join("\n");
    assert!(!logs.contains("private"), "{logs}");
}

/// By hand: `cargo test -p niva --lib real_sidecar -- --ignored --nocapture`.
/// Starts the REAL sidecar through `uv` with a microphone index that does not
/// exist (so the microphone is never opened and nothing is played), checks that
/// the host sees `MIC_MISSING` without restarting it, and that `shutdown` ends
/// the whole process tree.
#[test]
#[ignore = "starts the real sidecar (uv, Python, Whisper on the GPU); run by hand"]
fn real_sidecar_without_a_microphone_reports_mic_missing_and_stops_on_shutdown() {
    use crate::adapters::voice_sidecar::config::VoiceConfig;
    use crate::adapters::voice_sidecar::process::CommandLauncher;

    let wake = std::env::var("NIVA_WAKE_MODEL_DIR").unwrap_or_default();
    let voice_dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../../voice");
    let config = VoiceConfig::read(
        &|name| match name {
            "NIVA_VOICE_DIR" => Some(voice_dir.to_owned()),
            "NIVA_WAKE_MODEL_DIR" => Some(wake.clone()),
            "NIVA_VOICE_ARGS" => Some("--mic-device 9999".to_owned()),
            _ => None,
        },
        None,
        None,
        &|path| path.exists(),
    );
    assert_eq!(config.absent(), None);
    let modes = Arc::new(ListenModes::default());
    let link = Link::new(modes.clone());
    let events = Arc::new(Mutex::new(Vec::new()));
    let started = Instant::now();
    let mut settings = Settings::new(config.ladder());
    settings.stop_grace = Duration::from_secs(10);
    let sup = Supervisor::start(Deps {
        launcher: Arc::new(CommandLauncher::new(move |m| config.launch_spec(m))),
        link,
        modes,
        events: {
            let events = events.clone();
            Arc::new(move |e| events.lock().unwrap().push(e))
        },
        clock: Arc::new(move || started.elapsed().as_millis() as u64),
        log: Arc::new(|line| eprintln!("{line}")),
        status: Arc::new(|s| eprintln!("status: {s:?}")),
        levels: Arc::new(|_| {}),
        settings,
    });
    let until = Instant::now() + Duration::from_secs(120);
    while !events.lock().unwrap().iter().any(|e| {
        matches!(
            e,
            Event::SidecarError {
                code: SidecarErrorCode::MicMissing,
                ..
            }
        )
    }) {
        assert!(
            Instant::now() < until,
            "no MIC_MISSING from the real sidecar"
        );
        thread::sleep(Duration::from_millis(100));
    }
    let stopping = Instant::now();
    sup.shutdown();
    eprintln!("shutdown took {:?}", stopping.elapsed());
    assert!(stopping.elapsed() < Duration::from_secs(10));
}

const LEVEL_LINE: &str = r#"{"type":"level","src":"tts","bands":[0,12,40,55,30,8,100]}"#;

#[test]
fn level_creates_no_reducer_event() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    fake.say(LEVEL_LINE);
    fake.say(r#"{"type":"speak_started","id":6}"#);
    r.wait_event(Event::SpeakStarted { id: 6 });
    // The sentinel went through after the level: the level made no event of its own.
    assert_eq!(r.events(), vec![READY_ON, Event::SpeakStarted { id: 6 }]);
    assert_eq!(
        *r.levels.lock().unwrap(),
        vec![Level {
            bands: [0, 12, 40, 55, 30, 8, 100]
        }]
    );
}

#[test]
fn level_flood_30hz_does_not_reach_dispatcher() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    for _ in 0..300 {
        fake.say(LEVEL_LINE); // ten seconds of speech at 30 Hz
    }
    fake.say(r#"{"type":"speak_done","id":6}"#);
    r.wait_event(Event::SpeakDone { id: 6 });
    assert_eq!(r.events(), vec![READY_ON, Event::SpeakDone { id: 6 }]);
    assert_eq!(r.levels.lock().unwrap().len(), 300);
}

#[test]
fn level_is_not_logged() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    let before = r.logs.lock().unwrap().len();
    for _ in 0..50 {
        fake.say(LEVEL_LINE);
    }
    fake.say(r#"{"type":"speak_done","id":6}"#);
    r.wait_event(Event::SpeakDone { id: 6 });
    let logs = r.logs.lock().unwrap().clone();
    assert_eq!(logs.len(), before, "{logs:?}");
    assert!(logs.iter().all(|l| !l.contains("level")), "{logs:?}");
}

#[test]
fn malformed_level_does_not_block_other_messages() {
    let r = rig();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    fake.say(r#"{"type":"level","src":"tts","bands":[1,2,3]}"#);
    fake.say(r#"{"type":"level","src":"mic","bands":[1,2,3,4,5,6,7]}"#);
    fake.say(r#"{"type":"level","src":"tts","bands":[0,0,0,0,0,0,101]}"#);
    fake.say(r#"{"type":"speak_started","id":6}"#);
    r.wait_event(Event::SpeakStarted { id: 6 });
    assert!(r.levels.lock().unwrap().is_empty());
}

#[test]
fn an_error_for_a_stale_id_still_takes_voice_down_in_the_reducer() {
    use niva_core::state_machine::{State, reduce};
    let r = rig();
    let fake = r.launcher.run(0);
    fake.ready();
    r.wait_event(READY_ON);
    // An error for a listen that is long over: the reducer drops it by id.
    fake.say(r#"{"type":"error","code":"MIC_BUSY","msg":"in use","id":999}"#);
    r.wait_event(READY_OFF);
    let mut state = State::default();
    let mut events = r.events().into_iter();
    // `ready`, then the user opens a listen, then the rest of what the host sent.
    let (s, _) = reduce(state, events.next().unwrap());
    let (s, _) = reduce(s, Event::Shortcut);
    state = s;
    assert!(state.voice_available);
    for event in events {
        state = reduce(state, event).0;
    }
    assert!(!state.voice_available, "the reducer kept voice on");
}

#[test]
fn voice_turned_off_is_reported_as_turned_off_not_as_not_installed() {
    let r = rig_with(
        fast_settings(),
        &[Plan {
            fail: true,
            off: true,
            exits_on_eof: true,
        }],
    );
    r.wait_status(Status::Degraded(Degraded::TurnedOff));
    assert!(!r.link.is_up());
}
