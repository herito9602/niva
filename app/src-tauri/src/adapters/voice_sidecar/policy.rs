//! The supervisor's decisions, pure: restart counter, out-of-memory downgrade,
//! degraded text mode, the `ready` timeout and the slow automatic retry.
//!
//! Time is a number of milliseconds the caller passes in, and the policy never
//! does I/O: it takes [`Signal`]s and returns [`Action`]s, so every rule below is
//! tested without a process or a clock (design "Environment and Health":
//! "Restart counter", "OOM handling", "Degraded-mode recovery").
//!
//! The rules:
//!
//! - A crash (exit code 1, any other code, a kill from outside, a `PROTOCOL`
//!   error, or no `ready` in time) is restarted after a short backoff, up to
//!   [`MAX_RESTARTS`] times. The counter returns to zero after
//!   [`Timing::stable_after`] of `ready` with no `error` in between.
//! - An out-of-memory failure (`CUDA_OOM`, or `MODEL_LOAD` whose message says
//!   memory) is not a crash: the sidecar restarts once per rung of the Whisper
//!   ladder, each time with the next smaller model, and the counter is not
//!   touched. With no smaller model left the voice is degraded.
//! - Any other failure to load the models (exit code 3) is not retried at once:
//!   the same files would fail the same way. The voice is degraded.
//! - Degraded means text mode. It ends by a manual restart (tray entry, which
//!   resets the counter and the model size) or by a slow automatic retry.
//! - `MIC_MISSING` and `MIC_BUSY` keep the process: the sidecar retries the
//!   microphone itself and sends `ready` again, so the policy only stops waiting
//!   for the first `ready` (the process is alive and says what is wrong) and
//!   restarts the stability clock.

use std::cmp::min;

use niva_core::state_machine::SidecarErrorCode;

/// Restarts allowed for crashes before the voice is degraded.
pub const MAX_RESTARTS: u32 = 3;

/// Exit codes of `voice/main.py`.
pub const EXIT_OK: i32 = 0;
pub const EXIT_RUNTIME: i32 = 1;
pub const EXIT_LOAD: i32 = 3;

/// Whisper sizes from the preferred one down. S1 measured `small`; `base` is the
/// smaller candidate that is also in the local model cache.
pub const DEFAULT_LADDER: [&str; 2] = ["small", "base"];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Timing {
    /// From the spawn to the first `ready` (or a microphone error). Loading
    /// Whisper from a cold disk and CUDA start-up take several seconds.
    pub ready_timeout_ms: u64,
    /// `ready` with no `error` for this long resets the restart counter.
    pub stable_after_ms: u64,
    /// Pause before restart number 1, 2, 3.
    pub restart_delays_ms: [u64; MAX_RESTARTS as usize],
    /// Pause before the restart with a smaller model.
    pub downgrade_delay_ms: u64,
    /// How long a sidecar that reported a fatal error may take to exit.
    pub exit_grace_ms: u64,
    /// While degraded, one more try after this long, again and again.
    pub slow_retry_ms: u64,
}

impl Default for Timing {
    fn default() -> Self {
        Timing {
            ready_timeout_ms: 90_000,
            stable_after_ms: 5 * 60_000,
            restart_delays_ms: [1_000, 2_000, 5_000],
            downgrade_delay_ms: 500,
            exit_grace_ms: 10_000,
            slow_retry_ms: 5 * 60_000,
        }
    }
}

/// Why the voice is in text mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Degraded {
    /// The sidecar cannot be started (no Python, no `voice/` folder, disabled).
    Unavailable,
    /// The user turned it off (`NIVA_VOICE=off`).
    TurnedOff,
    /// The GPU is out of memory even with the smallest model.
    OutOfMemory,
    /// The models did not load for a reason that is not memory.
    LoadFailed,
    /// It kept crashing.
    Crashed,
}

/// What the user can be told about the voice (tray tooltip and log).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Status {
    Starting,
    Ready,
    /// The sidecar runs but has no working microphone; it keeps trying.
    MicProblem,
    Restarting {
        attempt: u32,
    },
    /// Out of memory: restarting with a smaller model.
    Downgraded {
        model: String,
    },
    Degraded(Degraded),
    Stopped,
}

impl Status {
    pub fn describe(&self) -> String {
        match self {
            Status::Starting => "voice starting".into(),
            Status::Ready => "voice ready".into(),
            Status::MicProblem => "voice waiting for a microphone".into(),
            Status::Restarting { attempt } => {
                format!("voice restarting ({attempt}/{MAX_RESTARTS})")
            }
            Status::Downgraded { model } => {
                format!(
                    "voice restarting with a smaller speech model ({model}): GPU memory is short"
                )
            }
            Status::Degraded(Degraded::TurnedOff) => "voice is turned off (NIVA_VOICE=off)".into(),
            Status::Degraded(Degraded::Unavailable) => {
                "voice is not installed: use the text window".into()
            }
            Status::Degraded(Degraded::OutOfMemory) => {
                "voice is off: not enough GPU memory, use the text window".into()
            }
            Status::Degraded(Degraded::LoadFailed) => {
                "voice is off: its models did not load, use the text window".into()
            }
            Status::Degraded(Degraded::Crashed) => {
                "voice is off: it kept stopping, use the text window".into()
            }
            Status::Stopped => "voice stopped".into(),
        }
    }
}

/// What happened (all with the time it was noticed, in milliseconds).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Signal {
    /// Begin: spawn the sidecar.
    Start,
    /// `ready` from the sidecar.
    Ready,
    /// `error{code}` of one of the six codes that leave voice unusable.
    /// `memory` says the message speaks of memory (for `MODEL_LOAD`).
    Error {
        code: SidecarErrorCode,
        memory: bool,
    },
    /// The process ended. `None`: no exit code (killed).
    Exited { code: Option<i32> },
    /// The spawn itself failed.
    SpawnFailed,
    /// The spawn was refused because the user turned voice off.
    Disabled,
    /// Time passed.
    Tick,
    /// The tray's "Restart voice".
    Restart,
    /// The app is closing.
    Shutdown,
}

/// What the caller must do, in order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Action {
    /// Start the sidecar with this Whisper model.
    Spawn {
        model: String,
    },
    /// Take the process down at once (no grace) and forget it.
    Kill,
    /// Ask it to shut down, give it time, then take it down.
    Stop,
    /// Tell the reducer: `VoiceReadinessChanged{ready}`.
    Voice {
        ready: bool,
    },
    Status(Status),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Phase {
    /// Nothing runs and nothing is scheduled.
    Idle,
    /// Waiting to spawn.
    Waiting {
        until: u64,
    },
    /// Spawned, no `ready` and no microphone error yet.
    Starting {
        deadline: u64,
    },
    /// Alive and talking.
    Running,
    /// Text mode. `retry_at`: the next slow automatic try.
    Degraded {
        retry_at: u64,
    },
    Closed,
}

/// What the sidecar said about its own failure, in one run.
#[derive(Debug, Clone, Copy, Default)]
struct Failure {
    /// The sidecar said it ran out of memory.
    memory: bool,
    /// It sent a fatal error and must exit by then.
    exit_by: Option<u64>,
    /// The models did not load (`CUDA_OOM` or `MODEL_LOAD`).
    load: bool,
}

#[derive(Debug)]
pub struct Policy {
    timing: Timing,
    ladder: Vec<String>,
    size: usize,
    restarts: u32,
    phase: Phase,
    /// When the current stretch of `ready` with no error began.
    stable_since: Option<u64>,
    failure: Failure,
    /// This run is one slow automatic try.
    slow_try: bool,
}

impl Policy {
    pub fn new(timing: Timing, ladder: Vec<String>) -> Self {
        assert!(!ladder.is_empty(), "the Whisper ladder needs a model");
        Policy {
            timing,
            ladder,
            size: 0,
            restarts: 0,
            phase: Phase::Idle,
            stable_since: None,
            failure: Failure::default(),
            slow_try: false,
        }
    }

    pub fn restarts(&self) -> u32 {
        self.restarts
    }

    pub fn model(&self) -> &str {
        &self.ladder[self.size]
    }

    pub fn is_degraded(&self) -> bool {
        matches!(self.phase, Phase::Degraded { .. })
    }

    /// When the policy next wants a [`Signal::Tick`], if ever.
    pub fn next_wake(&self) -> Option<u64> {
        let phase = match self.phase {
            Phase::Waiting { until } => Some(until),
            Phase::Starting { deadline } => Some(deadline),
            Phase::Degraded { retry_at } => Some(retry_at),
            Phase::Running => self
                .stable_since
                .filter(|_| self.restarts > 0)
                .map(|since| since + self.timing.stable_after_ms),
            Phase::Idle | Phase::Closed => None,
        };
        match (phase, self.failure.exit_by) {
            (Some(a), Some(b)) => Some(min(a, b)),
            (a, b) => a.or(b),
        }
    }

    pub fn on(&mut self, now: u64, signal: Signal) -> Vec<Action> {
        let mut out = Vec::new();
        if self.phase == Phase::Closed {
            return out;
        }
        self.settle_counter(now);
        match signal {
            Signal::Start => {
                if self.phase == Phase::Idle {
                    self.spawn(now, &mut out);
                }
            }
            Signal::Ready => self.on_ready(now, &mut out),
            Signal::Error { code, memory } => self.on_error(now, code, memory, &mut out),
            Signal::Exited { code } => self.on_exit(now, code, &mut out),
            Signal::Disabled => {
                out.push(Action::Voice { ready: false });
                self.degrade(now, Degraded::TurnedOff, &mut out);
            }
            Signal::SpawnFailed => {
                out.push(Action::Voice { ready: false });
                self.degrade(now, Degraded::Unavailable, &mut out);
            }
            Signal::Tick => self.on_tick(now, &mut out),
            Signal::Restart => self.on_restart(now, &mut out),
            Signal::Shutdown => {
                if !matches!(self.phase, Phase::Idle | Phase::Degraded { .. }) {
                    out.push(Action::Stop);
                }
                out.push(Action::Voice { ready: false });
                out.push(Action::Status(Status::Stopped));
                self.phase = Phase::Closed;
            }
        }
        out
    }

    /// The counter goes back to zero after a stable stretch.
    fn settle_counter(&mut self, now: u64) {
        if let Some(since) = self.stable_since
            && now >= since + self.timing.stable_after_ms
        {
            self.restarts = 0;
        }
    }

    fn spawn(&mut self, now: u64, out: &mut Vec<Action>) {
        self.failure = Failure::default();
        self.stable_since = None;
        self.phase = Phase::Starting {
            deadline: now + self.timing.ready_timeout_ms,
        };
        out.push(Action::Status(Status::Starting));
        out.push(Action::Spawn {
            model: self.model().to_owned(),
        });
    }

    fn on_ready(&mut self, now: u64, out: &mut Vec<Action>) {
        if !matches!(self.phase, Phase::Starting { .. } | Phase::Running) {
            return; // a late line of a process that is already being replaced
        }
        self.phase = Phase::Running;
        self.failure = Failure::default();
        // A slow retry that worked is an ordinary run from now on.
        self.slow_try = false;
        self.stable_since.get_or_insert(now);
        out.push(Action::Voice { ready: true });
        out.push(Action::Status(Status::Ready));
    }

    fn on_error(&mut self, now: u64, code: SidecarErrorCode, memory: bool, out: &mut Vec<Action>) {
        if !matches!(self.phase, Phase::Starting { .. } | Phase::Running) {
            return;
        }
        self.stable_since = None;
        // The reducer drops a `SidecarError` whose id is stale and would keep
        // voice on; this makes it agree (a no-op when it is already down), and a
        // later `ready` turns it back on.
        out.push(Action::Voice { ready: false });
        match code {
            SidecarErrorCode::MicMissing | SidecarErrorCode::MicBusy => {
                // Alive and diagnosing: stop the clock of the first `ready`.
                self.phase = Phase::Running;
                out.push(Action::Status(Status::MicProblem));
            }
            SidecarErrorCode::CudaOom | SidecarErrorCode::ModelLoad => {
                self.failure.memory = code == SidecarErrorCode::CudaOom || memory;
                self.failure.load = true;
                // It sends this just before exiting with code 3.
                self.failure.exit_by = Some(now + self.timing.exit_grace_ms);
            }
            SidecarErrorCode::Internal => {
                self.failure.exit_by = Some(now + self.timing.exit_grace_ms);
            }
            SidecarErrorCode::Protocol => {
                // Alive but confused, and it will never send `ready` by itself.
                out.push(Action::Kill);
                self.after_crash(now, out);
            }
        }
    }

    fn on_exit(&mut self, now: u64, code: Option<i32>, out: &mut Vec<Action>) {
        if !matches!(self.phase, Phase::Starting { .. } | Phase::Running) {
            return; // the process of a previous run, already accounted for
        }
        out.push(Action::Voice { ready: false });
        if self.failure.memory {
            self.after_memory_failure(now, out);
        } else if code == Some(EXIT_LOAD) || self.failure.load {
            self.degrade(now, Degraded::LoadFailed, out);
        } else {
            self.after_crash(now, out);
        }
    }

    fn after_memory_failure(&mut self, now: u64, out: &mut Vec<Action>) {
        if self.size + 1 < self.ladder.len() {
            self.size += 1;
            self.failure = Failure::default();
            self.stable_since = None;
            self.phase = Phase::Waiting {
                until: now + self.timing.downgrade_delay_ms,
            };
            out.push(Action::Status(Status::Downgraded {
                model: self.model().to_owned(),
            }));
        } else {
            self.degrade(now, Degraded::OutOfMemory, out);
        }
    }

    fn after_crash(&mut self, now: u64, out: &mut Vec<Action>) {
        self.stable_since = None;
        self.failure = Failure::default();
        if self.slow_try || self.restarts >= MAX_RESTARTS {
            self.degrade(now, Degraded::Crashed, out);
            return;
        }
        let delay = self.timing.restart_delays_ms[self.restarts as usize];
        self.restarts += 1;
        self.phase = Phase::Waiting { until: now + delay };
        out.push(Action::Status(Status::Restarting {
            attempt: self.restarts,
        }));
    }

    fn degrade(&mut self, now: u64, why: Degraded, out: &mut Vec<Action>) {
        self.failure = Failure::default();
        self.stable_since = None;
        self.slow_try = false;
        self.phase = Phase::Degraded {
            retry_at: now + self.timing.slow_retry_ms,
        };
        out.push(Action::Status(Status::Degraded(why)));
    }

    fn on_tick(&mut self, now: u64, out: &mut Vec<Action>) {
        if let Some(by) = self.failure.exit_by
            && now >= by
            && matches!(self.phase, Phase::Starting { .. } | Phase::Running)
        {
            // It said it was failing and did not exit: take it down.
            out.push(Action::Kill);
            out.push(Action::Voice { ready: false });
            if self.failure.memory {
                self.after_memory_failure(now, out);
            } else if self.failure.load {
                self.degrade(now, Degraded::LoadFailed, out);
            } else {
                self.after_crash(now, out);
            }
            return;
        }
        match self.phase {
            Phase::Waiting { until } if now >= until => self.spawn(now, out),
            Phase::Starting { deadline } if now >= deadline => {
                // A hung microphone open or a stuck model load sends nothing.
                out.push(Action::Kill);
                out.push(Action::Voice { ready: false });
                self.after_crash(now, out);
            }
            Phase::Degraded { retry_at } if now >= retry_at => {
                self.spawn(now, out);
                self.slow_try = true;
            }
            _ => {}
        }
    }

    fn on_restart(&mut self, now: u64, out: &mut Vec<Action>) {
        match self.phase {
            Phase::Starting { .. } | Phase::Running => {
                out.push(Action::Kill);
                out.push(Action::Voice { ready: false });
            }
            Phase::Idle | Phase::Waiting { .. } | Phase::Degraded { .. } => {}
            Phase::Closed => return,
        }
        self.restarts = 0;
        self.size = 0;
        self.slow_try = false;
        self.spawn(now, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn timing() -> Timing {
        Timing {
            ready_timeout_ms: 1_000,
            stable_after_ms: 10_000,
            restart_delays_ms: [100, 200, 300],
            downgrade_delay_ms: 50,
            exit_grace_ms: 500,
            slow_retry_ms: 5_000,
        }
    }

    fn policy() -> Policy {
        Policy::new(timing(), vec!["small".into(), "base".into()])
    }

    fn spawns(actions: &[Action]) -> Vec<String> {
        actions
            .iter()
            .filter_map(|a| match a {
                Action::Spawn { model } => Some(model.clone()),
                _ => None,
            })
            .collect()
    }

    fn has(actions: &[Action], wanted: &Action) -> bool {
        actions.contains(wanted)
    }

    fn voice(ready: bool) -> Action {
        Action::Voice { ready }
    }

    /// Start and reach `Ready` at `t`.
    fn running(p: &mut Policy, t: u64) {
        let a = p.on(0, Signal::Start);
        assert_eq!(spawns(&a), ["small"]);
        let a = p.on(t, Signal::Ready);
        assert!(has(&a, &voice(true)));
    }

    #[test]
    fn start_spawns_the_preferred_model_and_ready_turns_voice_on() {
        let mut p = policy();
        let a = p.on(0, Signal::Start);
        assert_eq!(spawns(&a), ["small"]);
        assert!(has(&a, &Action::Status(Status::Starting)));
        let a = p.on(5, Signal::Ready);
        assert_eq!(a, vec![voice(true), Action::Status(Status::Ready)]);
    }

    #[test]
    fn no_ready_in_time_kills_it_and_counts_as_a_crash() {
        let mut p = policy();
        p.on(0, Signal::Start);
        assert_eq!(p.next_wake(), Some(1_000));
        assert!(p.on(999, Signal::Tick).is_empty());
        let a = p.on(1_000, Signal::Tick);
        assert!(has(&a, &Action::Kill) && has(&a, &voice(false)));
        assert_eq!(p.restarts(), 1);
        let a = p.on(1_100, Signal::Tick);
        assert_eq!(spawns(&a), ["small"]);
    }

    #[test]
    fn a_microphone_error_before_ready_stops_the_ready_clock() {
        let mut p = policy();
        p.on(0, Signal::Start);
        let a = p.on(
            2_000_000,
            Signal::Error {
                code: SidecarErrorCode::MicBusy,
                memory: false,
            },
        );
        assert_eq!(a, vec![voice(false), Action::Status(Status::MicProblem)]);
        // No deadline, no kill: the sidecar retries the microphone by itself.
        assert_eq!(p.next_wake(), None);
        assert!(p.on(9_000_000, Signal::Tick).is_empty());
        // And when the microphone works, `ready` brings voice back.
        let a = p.on(9_000_001, Signal::Ready);
        assert!(has(&a, &voice(true)));
    }

    #[test]
    fn a_repeated_ready_after_an_error_is_accepted() {
        let mut p = policy();
        running(&mut p, 10);
        p.on(
            20,
            Signal::Error {
                code: SidecarErrorCode::MicMissing,
                memory: false,
            },
        );
        let a = p.on(30, Signal::Ready);
        assert!(has(&a, &voice(true)));
        let a = p.on(40, Signal::Ready);
        assert!(has(&a, &voice(true)));
        assert_eq!(p.restarts(), 0);
    }

    #[test]
    fn a_crash_restarts_up_to_three_times_then_degrades() {
        let mut p = policy();
        p.on(0, Signal::Start);
        let mut t = 0;
        for (n, delay) in [100, 200, 300].into_iter().enumerate() {
            t += 10;
            let a = p.on(t, Signal::Exited { code: Some(1) });
            assert!(has(&a, &voice(false)), "restart {}", n + 1);
            assert!(has(
                &a,
                &Action::Status(Status::Restarting {
                    attempt: n as u32 + 1
                })
            ));
            assert_eq!(p.next_wake(), Some(t + delay));
            t += delay;
            let a = p.on(t, Signal::Tick);
            assert_eq!(spawns(&a), ["small"]);
        }
        let a = p.on(t + 10, Signal::Exited { code: Some(1) });
        assert!(has(
            &a,
            &Action::Status(Status::Degraded(Degraded::Crashed))
        ));
        assert!(p.is_degraded());
        assert!(spawns(&a).is_empty());
    }

    #[test]
    fn an_exit_with_a_surprising_code_is_a_crash_too() {
        for code in [Some(0), Some(-1), Some(1), None] {
            let mut p = policy();
            running(&mut p, 5);
            let a = p.on(6, Signal::Exited { code });
            assert!(has(&a, &voice(false)));
            assert_eq!(p.restarts(), 1, "{code:?}");
        }
    }

    #[test]
    fn five_stable_minutes_reset_the_counter() {
        let mut p = policy();
        p.on(0, Signal::Start);
        p.on(1, Signal::Exited { code: Some(1) });
        p.on(101, Signal::Tick);
        p.on(102, Signal::Ready);
        assert_eq!(p.restarts(), 1);
        // Not yet stable.
        p.on(10_000, Signal::Tick);
        assert_eq!(p.restarts(), 1);
        // 10 s after the ready: the counter is reset (this timing's "5 minutes").
        p.on(10_102, Signal::Tick);
        assert_eq!(p.restarts(), 0);
    }

    #[test]
    fn an_error_between_resets_the_stability_clock() {
        let mut p = policy();
        p.on(0, Signal::Start);
        p.on(1, Signal::Exited { code: Some(1) });
        p.on(101, Signal::Tick);
        p.on(102, Signal::Ready);
        p.on(
            9_000,
            Signal::Error {
                code: SidecarErrorCode::MicBusy,
                memory: false,
            },
        );
        p.on(9_500, Signal::Ready);
        // 10 s after the FIRST ready would be 10_102; the clock restarted at 9_500.
        p.on(10_200, Signal::Tick);
        assert_eq!(p.restarts(), 1);
        p.on(19_500, Signal::Tick);
        assert_eq!(p.restarts(), 0);
    }

    #[test]
    fn a_crash_after_a_stable_stretch_starts_from_a_fresh_counter() {
        let mut p = policy();
        p.on(0, Signal::Start);
        p.on(1, Signal::Exited { code: Some(1) });
        p.on(101, Signal::Tick);
        p.on(102, Signal::Ready);
        // The crash arrives at once, before any tick noticed the stable stretch.
        p.on(20_000, Signal::Exited { code: Some(1) });
        assert_eq!(p.restarts(), 1);
    }

    #[test]
    fn out_of_memory_restarts_with_the_next_smaller_model_without_using_the_counter() {
        let mut p = policy();
        p.on(0, Signal::Start);
        p.on(
            5,
            Signal::Error {
                code: SidecarErrorCode::CudaOom,
                memory: false,
            },
        );
        let a = p.on(
            6,
            Signal::Exited {
                code: Some(EXIT_LOAD),
            },
        );
        assert!(has(
            &a,
            &Action::Status(Status::Downgraded {
                model: "base".into()
            })
        ));
        assert_eq!(p.restarts(), 0);
        assert_eq!(p.next_wake(), Some(56));
        let a = p.on(56, Signal::Tick);
        assert_eq!(spawns(&a), ["base"]);
        assert_eq!(p.model(), "base");
    }

    #[test]
    fn out_of_memory_at_the_smallest_model_degrades_to_text_mode() {
        let mut p = policy();
        p.on(0, Signal::Start);
        p.on(
            1,
            Signal::Error {
                code: SidecarErrorCode::CudaOom,
                memory: false,
            },
        );
        p.on(2, Signal::Exited { code: Some(3) });
        p.on(60, Signal::Tick); // spawns "base"
        p.on(
            70,
            Signal::Error {
                code: SidecarErrorCode::CudaOom,
                memory: false,
            },
        );
        let a = p.on(71, Signal::Exited { code: Some(3) });
        assert!(has(
            &a,
            &Action::Status(Status::Degraded(Degraded::OutOfMemory))
        ));
        assert!(spawns(&a).is_empty());
    }

    #[test]
    fn a_model_load_failure_about_memory_downgrades_like_out_of_memory() {
        let mut p = policy();
        p.on(0, Signal::Start);
        p.on(
            1,
            Signal::Error {
                code: SidecarErrorCode::ModelLoad,
                memory: true,
            },
        );
        let a = p.on(2, Signal::Exited { code: Some(3) });
        assert!(has(
            &a,
            &Action::Status(Status::Downgraded {
                model: "base".into()
            })
        ));
    }

    #[test]
    fn a_model_load_failure_that_is_not_memory_does_not_restart_into_the_same_failure() {
        let mut p = policy();
        p.on(0, Signal::Start);
        p.on(
            1,
            Signal::Error {
                code: SidecarErrorCode::ModelLoad,
                memory: false,
            },
        );
        let a = p.on(2, Signal::Exited { code: Some(3) });
        assert!(has(
            &a,
            &Action::Status(Status::Degraded(Degraded::LoadFailed))
        ));
        assert!(spawns(&a).is_empty());
        assert_eq!(p.restarts(), 0);
    }

    #[test]
    fn exit_code_three_without_an_error_line_is_a_load_failure() {
        let mut p = policy();
        p.on(0, Signal::Start);
        let a = p.on(2, Signal::Exited { code: Some(3) });
        assert!(has(
            &a,
            &Action::Status(Status::Degraded(Degraded::LoadFailed))
        ));
    }

    #[test]
    fn a_fatal_error_without_an_exit_is_taken_down_after_the_grace() {
        let mut p = policy();
        running(&mut p, 5);
        p.on(
            100,
            Signal::Error {
                code: SidecarErrorCode::CudaOom,
                memory: false,
            },
        );
        assert_eq!(p.next_wake(), Some(600));
        let a = p.on(600, Signal::Tick);
        assert!(has(&a, &Action::Kill) && has(&a, &voice(false)));
        assert!(
            a.iter()
                .any(|x| matches!(x, Action::Status(Status::Downgraded { .. })))
        );
    }

    #[test]
    fn an_internal_error_then_exit_one_is_a_crash() {
        let mut p = policy();
        running(&mut p, 5);
        p.on(
            100,
            Signal::Error {
                code: SidecarErrorCode::Internal,
                memory: false,
            },
        );
        let a = p.on(110, Signal::Exited { code: Some(1) });
        assert!(has(&a, &voice(false)));
        assert_eq!(p.restarts(), 1);
    }

    #[test]
    fn a_protocol_error_restarts_the_sidecar_because_no_ready_would_ever_follow() {
        let mut p = policy();
        running(&mut p, 5);
        let a = p.on(
            50,
            Signal::Error {
                code: SidecarErrorCode::Protocol,
                memory: false,
            },
        );
        assert!(has(&a, &Action::Kill) && has(&a, &voice(false)));
        assert_eq!(p.restarts(), 1);
        let a = p.on(150, Signal::Tick);
        assert_eq!(spawns(&a), ["small"]);
    }

    #[test]
    fn the_exit_of_a_killed_process_is_not_counted_twice() {
        let mut p = policy();
        running(&mut p, 5);
        p.on(
            50,
            Signal::Error {
                code: SidecarErrorCode::Protocol,
                memory: false,
            },
        );
        // The supervisor killed it and reaped it; if its exit is still reported
        // it is the old run's, and the policy is already waiting to respawn.
        let a = p.on(60, Signal::Exited { code: None });
        assert!(a.is_empty());
        assert_eq!(p.restarts(), 1);
    }

    #[test]
    fn a_spawn_failure_is_text_mode_with_a_slow_retry() {
        let mut p = policy();
        p.on(0, Signal::Start);
        let a = p.on(1, Signal::SpawnFailed);
        assert!(has(&a, &voice(false)));
        assert!(has(
            &a,
            &Action::Status(Status::Degraded(Degraded::Unavailable))
        ));
        assert_eq!(p.next_wake(), Some(5_001));
        let a = p.on(5_001, Signal::Tick);
        assert_eq!(spawns(&a), ["small"]);
    }

    #[test]
    fn the_slow_retry_is_one_try_and_a_failed_one_goes_back_to_text_mode() {
        let mut p = policy();
        p.on(0, Signal::Start);
        for _ in 0..3 {
            p.on(1, Signal::Exited { code: Some(1) });
            p.on(10_000, Signal::Tick);
        }
        let a = p.on(10_001, Signal::Exited { code: Some(1) });
        assert!(p.is_degraded() && !a.is_empty());
        let retry = p.next_wake().unwrap();
        let a = p.on(retry, Signal::Tick);
        assert_eq!(spawns(&a), ["small"]);
        // It fails again at once: back to text mode, with the next slow try.
        let a = p.on(retry + 5, Signal::Exited { code: Some(1) });
        assert!(has(
            &a,
            &Action::Status(Status::Degraded(Degraded::Crashed))
        ));
        assert_eq!(p.next_wake(), Some(retry + 5 + 5_000));
    }

    #[test]
    fn a_slow_retry_that_works_leaves_text_mode() {
        let mut p = policy();
        p.on(0, Signal::Start);
        p.on(1, Signal::SpawnFailed);
        p.on(5_001, Signal::Tick);
        let a = p.on(5_100, Signal::Ready);
        assert!(has(&a, &voice(true)));
        assert!(!p.is_degraded());
    }

    #[test]
    fn manual_restart_from_text_mode_resets_the_counter_and_the_model_size() {
        let mut p = policy();
        p.on(0, Signal::Start);
        p.on(
            1,
            Signal::Error {
                code: SidecarErrorCode::CudaOom,
                memory: false,
            },
        );
        p.on(2, Signal::Exited { code: Some(3) });
        p.on(60, Signal::Tick); // "base"
        p.on(
            61,
            Signal::Error {
                code: SidecarErrorCode::CudaOom,
                memory: false,
            },
        );
        p.on(62, Signal::Exited { code: Some(3) });
        assert!(p.is_degraded());
        let a = p.on(100, Signal::Restart);
        assert_eq!(spawns(&a), ["small"]);
        assert_eq!(p.restarts(), 0);
        assert!(!p.is_degraded());
    }

    #[test]
    fn manual_restart_while_running_kills_and_respawns() {
        let mut p = policy();
        running(&mut p, 5);
        let a = p.on(50, Signal::Restart);
        let order: Vec<_> = a
            .iter()
            .filter(|x| {
                matches!(
                    x,
                    Action::Kill | Action::Spawn { .. } | Action::Voice { .. }
                )
            })
            .cloned()
            .collect();
        assert_eq!(
            order,
            vec![
                Action::Kill,
                voice(false),
                Action::Spawn {
                    model: "small".into()
                }
            ]
        );
    }

    #[test]
    fn manual_restart_while_waiting_for_a_backoff_spawns_at_once() {
        let mut p = policy();
        p.on(0, Signal::Start);
        p.on(1, Signal::Exited { code: Some(1) });
        let a = p.on(2, Signal::Restart);
        assert_eq!(spawns(&a), ["small"]);
        assert!(!has(&a, &Action::Kill));
        assert_eq!(p.restarts(), 0);
    }

    #[test]
    fn shutdown_stops_the_process_and_nothing_restarts_afterwards() {
        let mut p = policy();
        running(&mut p, 5);
        let a = p.on(50, Signal::Shutdown);
        assert!(has(&a, &Action::Stop) && has(&a, &voice(false)));
        assert!(p.on(60, Signal::Exited { code: Some(0) }).is_empty());
        assert!(p.on(10_000_000, Signal::Tick).is_empty());
        assert!(p.on(10_000_001, Signal::Restart).is_empty());
        assert_eq!(p.next_wake(), None);
    }

    #[test]
    fn shutdown_in_text_mode_has_no_process_to_stop() {
        let mut p = policy();
        p.on(0, Signal::Start);
        p.on(1, Signal::SpawnFailed);
        let a = p.on(2, Signal::Shutdown);
        assert!(!has(&a, &Action::Stop));
    }

    #[test]
    fn a_late_ready_of_a_replaced_process_is_ignored() {
        let mut p = policy();
        p.on(0, Signal::Start);
        p.on(1, Signal::Exited { code: Some(1) });
        // Waiting for the backoff: a stray `ready` must not turn voice on.
        assert!(p.on(2, Signal::Ready).is_empty());
    }

    #[test]
    fn a_crash_after_a_successful_slow_retry_gets_the_normal_restarts() {
        let mut p = policy();
        p.on(0, Signal::Start);
        p.on(1, Signal::SpawnFailed);
        p.on(5_001, Signal::Tick); // the slow retry
        p.on(5_100, Signal::Ready); // and it works
        let a = p.on(5_200, Signal::Exited { code: Some(1) });
        assert!(
            has(&a, &Action::Status(Status::Restarting { attempt: 1 })),
            "{a:?}"
        );
        assert!(!p.is_degraded());
        let a = p.on(5_300, Signal::Tick);
        assert_eq!(spawns(&a), ["small"]);
    }

    #[test]
    fn every_fatal_or_microphone_error_also_tells_the_reducer_voice_is_down() {
        // The reducer drops a SidecarError whose id is stale and keeps voice on;
        // this event is what makes it agree (a no-op when it is already down).
        for code in SidecarErrorCode::ALL {
            let mut p = policy();
            running(&mut p, 5);
            let a = p.on(
                50,
                Signal::Error {
                    code,
                    memory: false,
                },
            );
            assert!(has(&a, &voice(false)), "{code:?}");
        }
    }

    #[test]
    fn turned_off_is_text_mode_and_says_so() {
        let mut p = policy();
        p.on(0, Signal::Start);
        let a = p.on(1, Signal::Disabled);
        assert!(has(&a, &voice(false)));
        assert!(has(
            &a,
            &Action::Status(Status::Degraded(Degraded::TurnedOff))
        ));
        assert!(
            Status::Degraded(Degraded::TurnedOff)
                .describe()
                .contains("turned off")
        );
    }

    #[test]
    fn the_default_timing_is_the_designs() {
        let t = Timing::default();
        assert_eq!(t.stable_after_ms, 300_000);
        assert_eq!(t.slow_retry_ms, 300_000);
        assert_eq!(MAX_RESTARTS, 3);
        assert!(t.ready_timeout_ms >= 30_000);
    }
}
