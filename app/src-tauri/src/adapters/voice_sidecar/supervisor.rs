//! The voice supervisor: one thread that owns the sidecar process.
//!
//! It starts the process, reads its lines on a thread of their own, turns them
//! into reducer events ([`Translator`]), feeds what matters to the restart
//! [`Policy`] and carries out the policy's actions: spawn, kill, stop, tell the
//! reducer `VoiceReadinessChanged`, report a status. It also owns the clock of
//! the `ready` timeout, so a sidecar that hangs without a word is noticed.
//!
//! Threads per process: reader (stdout), logger (stderr), writer (stdin). The
//! writer exists so a sidecar that stops reading cannot block the dispatcher
//! (the ports only queue into a channel, see [`Link`]).
//!
//! Shutting down never leaves a sidecar behind: `shutdown` is sent, stdin is
//! closed, the process gets [`Settings::stop_grace`] to exit, and then it is
//! killed together with everything it started.

use std::io::Write;
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use niva_core::state_machine::Event;

use super::link::Link;
use super::policy::{Action, Policy, Signal, Status, Timing};
use super::process::{Launcher, Line, Proc, read_lines};
use super::translate::{HostClock, ListenModes, Log, Translator};
use super::wire::{self, Outgoing, ParseError};

pub type EventOut = Arc<dyn Fn(Event) + Send + Sync>;
pub type StatusOut = Arc<dyn Fn(Status) + Send + Sync>;

/// Holds the first start until the LLM is warm (design "Model-load order"):
/// the larger, less flexible GPU allocation lands first when it can, and a
/// Whisper out-of-memory is then a deterministic result. Bounded, so a missing
/// Ollama never keeps voice away.
pub struct Gate {
    pub open: Arc<dyn Fn() -> bool + Send + Sync>,
    pub max_wait: Duration,
}

pub struct Settings {
    pub timing: Timing,
    pub ladder: Vec<String>,
    pub gate: Option<Gate>,
    /// How often a running process is checked for having exited.
    pub poll: Duration,
    /// After the process exits, how long its last lines may still arrive.
    pub drain: Duration,
    /// How long `shutdown` waits for a clean exit before killing.
    pub stop_grace: Duration,
}

impl Settings {
    pub fn new(ladder: Vec<String>) -> Self {
        Settings {
            timing: Timing::default(),
            ladder,
            gate: None,
            poll: Duration::from_millis(100),
            drain: Duration::from_millis(500),
            stop_grace: Duration::from_secs(3),
        }
    }
}

pub struct Deps {
    pub launcher: Arc<dyn Launcher>,
    pub link: Arc<Link>,
    pub modes: Arc<ListenModes>,
    /// Where reducer events go (the dispatcher's queue).
    pub events: EventOut,
    pub clock: HostClock,
    pub log: Log,
    pub status: StatusOut,
    pub settings: Settings,
}

enum Msg {
    Restart,
    Shutdown,
    /// From the reader thread of run `run_id`.
    Line(u64, Line),
    /// The reader of run `run_id` reached the end of stdout.
    StdoutEnd(u64),
}

/// The handle the app keeps.
pub struct Supervisor {
    tx: Sender<Msg>,
    thread: Mutex<Option<JoinHandle<()>>>,
}

impl Supervisor {
    pub fn start(deps: Deps) -> Supervisor {
        let (tx, rx) = channel();
        let mut run = Run::new(deps, tx.clone(), rx);
        let thread = thread::Builder::new()
            .name("niva-voice-supervisor".into())
            .spawn(move || run.main())
            .ok();
        Supervisor {
            tx,
            thread: Mutex::new(thread),
        }
    }

    /// The tray's "Restart voice": resets the counter and the model size and
    /// starts the sidecar again, from any state.
    pub fn restart(&self) {
        let _ = self.tx.send(Msg::Restart);
    }

    /// Stops the sidecar and the thread. Blocks until both are gone (a clean
    /// exit is quick; the worst case is `stop_grace` plus the kill).
    pub fn shutdown(&self) {
        let _ = self.tx.send(Msg::Shutdown);
        let thread = self
            .thread
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(thread) = thread {
            let _ = thread.join();
        }
    }
}

impl Drop for Supervisor {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// The process of the current run and what is known about its end.
struct Live {
    run_id: u64,
    proc: Box<dyn Proc>,
    stdout_ended: bool,
    /// Set when the exit was seen: (exit code, give up waiting for its last lines at).
    exited: Option<(Option<i32>, u64)>,
}

struct Run {
    deps: Deps,
    tx: Sender<Msg>,
    rx: Receiver<Msg>,
    policy: Policy,
    translator: Translator,
    live: Option<Live>,
    run_id: u64,
    unreadable: u64,
}

impl Run {
    fn new(deps: Deps, tx: Sender<Msg>, rx: Receiver<Msg>) -> Run {
        let policy = Policy::new(deps.settings.timing.clone(), deps.settings.ladder.clone());
        let translator = Translator::new(deps.clock.clone(), deps.log.clone(), deps.modes.clone());
        Run {
            deps,
            tx,
            rx,
            policy,
            translator,
            live: None,
            run_id: 0,
            unreadable: 0,
        }
    }

    fn now(&self) -> u64 {
        (self.deps.clock)()
    }

    fn log(&self, line: String) {
        (self.deps.log)(line);
    }

    fn main(&mut self) {
        if !self.wait_for_gate() {
            self.feed(Signal::Shutdown);
            return;
        }
        self.feed(Signal::Start);
        loop {
            match self.rx.recv_timeout(self.timeout()) {
                Ok(Msg::Shutdown) | Err(RecvTimeoutError::Disconnected) => {
                    self.feed(Signal::Shutdown);
                    return;
                }
                Ok(Msg::Restart) => self.feed(Signal::Restart),
                Ok(Msg::Line(run_id, line)) => {
                    if self.live.as_ref().is_some_and(|l| l.run_id == run_id) {
                        self.on_line(line);
                    }
                }
                Ok(Msg::StdoutEnd(run_id)) => {
                    if let Some(live) = self.live.as_mut().filter(|l| l.run_id == run_id) {
                        live.stdout_ended = true;
                    }
                }
                Err(RecvTimeoutError::Timeout) => {}
            }
            self.poll_exit();
            if self.policy.next_wake().is_some_and(|at| self.now() >= at) {
                self.feed(Signal::Tick);
            }
        }
    }

    /// Waits for the LLM to be warm (or the bound). `false`: shut down meanwhile.
    fn wait_for_gate(&self) -> bool {
        let Some(gate) = &self.deps.settings.gate else {
            return true;
        };
        let started = self.now();
        let limit = u64::try_from(gate.max_wait.as_millis()).unwrap_or(u64::MAX);
        while !(gate.open)() && self.now().saturating_sub(started) < limit {
            match self.rx.recv_timeout(Duration::from_millis(100)) {
                Ok(Msg::Shutdown) | Err(RecvTimeoutError::Disconnected) => return false,
                Ok(Msg::Restart) => break,
                _ => {}
            }
        }
        true
    }

    fn timeout(&self) -> Duration {
        let mut wait = Duration::from_secs(1);
        if let Some(at) = self.policy.next_wake() {
            wait = wait.min(Duration::from_millis(at.saturating_sub(self.now())));
        }
        if self.live.is_some() {
            wait = wait.min(self.deps.settings.poll);
        }
        wait.max(Duration::from_millis(1))
    }

    /// Feeds one signal to the policy and carries out what it asks, including
    /// the signals that carrying it out produces.
    fn feed(&mut self, signal: Signal) {
        let mut pending = vec![signal];
        while let Some(signal) = pending.pop() {
            let actions = self.policy.on(self.now(), signal);
            for action in actions {
                if let Some(next) = self.execute(action) {
                    pending.push(next);
                }
            }
        }
    }

    fn execute(&mut self, action: Action) -> Option<Signal> {
        match action {
            Action::Spawn { model } => return self.spawn(&model),
            Action::Kill => self.kill(),
            Action::Stop => self.stop(),
            Action::Voice { ready } => {
                // The port flag first, so a reply the event produces already
                // sees the right state.
                self.deps.link.set_up(ready);
                (self.deps.events)(Event::VoiceReadinessChanged { ready });
            }
            Action::Status(status) => {
                self.log(format!("[niva-voice] {}", status.describe()));
                (self.deps.status)(status);
            }
        }
        None
    }

    fn spawn(&mut self, model: &str) -> Option<Signal> {
        match self.deps.launcher.spawn(model) {
            Err(why) => {
                self.log(format!("[niva-voice] cannot start the sidecar: {why}"));
                Some(if why.kind() == std::io::ErrorKind::Unsupported {
                    Signal::Disabled
                } else {
                    Signal::SpawnFailed
                })
            }
            Ok(spawned) => {
                self.run_id += 1;
                let run_id = self.run_id;
                self.log(format!("[niva-voice] sidecar started (whisper: {model})"));
                let (out_tx, out_rx) = channel::<Outgoing>();
                let mut stdin = spawned.stdin;
                thread::spawn(move || {
                    for message in out_rx {
                        let line = message.to_line();
                        if stdin
                            .write_all(line.as_bytes())
                            .and_then(|()| stdin.flush())
                            .is_err()
                        {
                            break;
                        }
                    }
                    // Dropping stdin closes the pipe: the sidecar's EOF.
                });
                let tx = self.tx.clone();
                let stdout = spawned.stdout;
                thread::spawn(move || {
                    read_lines(stdout, |line| {
                        let _ = tx.send(Msg::Line(run_id, line));
                    });
                    let _ = tx.send(Msg::StdoutEnd(run_id));
                });
                let log = self.deps.log.clone();
                let stderr = spawned.stderr;
                thread::spawn(move || {
                    read_lines(stderr, |line| {
                        if let Line::Text(text) = line {
                            let text: String = text.chars().take(500).collect();
                            log(format!("[niva-voice:py] {text}"));
                        }
                    });
                });
                self.deps.link.attach(out_tx);
                self.live = Some(Live {
                    run_id,
                    proc: spawned.proc,
                    stdout_ended: false,
                    exited: None,
                });
                None
            }
        }
    }

    /// Takes the process down at once.
    fn kill(&mut self) {
        self.deps.link.detach();
        if let Some(mut live) = self.live.take() {
            live.proc.kill_tree();
        }
    }

    /// The clean way: `shutdown`, close stdin, wait, and only then kill.
    fn stop(&mut self) {
        let Some(mut live) = self.live.take() else {
            return;
        };
        self.deps.link.send(Outgoing::Shutdown);
        // Drops the channel: the writer thread flushes `shutdown`, then closes stdin.
        self.deps.link.detach();
        let grace = self.deps.settings.stop_grace;
        let poll = Duration::from_millis(20).min(grace);
        let mut waited = Duration::ZERO;
        loop {
            if matches!(live.proc.try_exit(), Ok(Some(_))) {
                return;
            }
            if waited >= grace {
                break;
            }
            thread::sleep(poll);
            waited += poll;
        }
        self.log("[niva-voice] the sidecar did not exit in time: killing it".into());
        live.proc.kill_tree();
    }

    fn on_line(&mut self, line: Line) {
        let parsed = match line {
            Line::Text(text) => wire::parse_line(&text),
            Line::TooLong => Err(ParseError::TooLong),
        };
        match parsed {
            Err(why) => {
                // Not fatal and not echoed: the first few, then one in a hundred.
                self.unreadable += 1;
                if self.unreadable <= 5 || self.unreadable.is_multiple_of(100) {
                    self.log(format!(
                        "[niva-voice] ignored an unreadable line ({why}); {} so far",
                        self.unreadable
                    ));
                }
            }
            Ok(incoming) => {
                let translated = self.translator.translate(incoming);
                for event in translated.events {
                    if matches!(event, Event::SidecarError { .. }) {
                        self.deps.link.set_up(false);
                    }
                    (self.deps.events)(event);
                }
                if let Some(signal) = translated.signal {
                    self.feed(signal);
                }
            }
        }
    }

    /// Notices that the process ended, lets its last lines arrive, and only
    /// then tells the policy (so an error line sent just before exit code 3 is
    /// not lost).
    fn poll_exit(&mut self) {
        let now = self.now();
        let drain = u64::try_from(self.deps.settings.drain.as_millis()).unwrap_or(u64::MAX);
        let Some(live) = self.live.as_mut() else {
            return;
        };
        if live.exited.is_none() {
            match live.proc.try_exit() {
                Ok(None) => return,
                Ok(Some(code)) => live.exited = Some((code, now + drain)),
                Err(_) => live.exited = Some((None, now + drain)),
            }
        }
        let Some((code, until)) = live.exited else {
            return;
        };
        if live.stdout_ended || now >= until {
            self.live = None;
            self.deps.link.detach();
            self.feed(Signal::Exited { code });
        }
    }
}

#[cfg(test)]
mod tests;
