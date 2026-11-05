//! Keeps the model warm and the connection honest for as long as the app runs
//! (PR 2d-a; BACKLOG items of the PR 2c review).
//!
//! `start_up` (health + warm-up with backoff) used to run once, and gave up
//! after six attempts. That left three holes: Ollama not ready at the first
//! probe (the autostart race of design "Environment and Health"), a model that
//! went cold (a timed-out request, `keep_alive` expiry) and an Ollama that
//! restarted. The keeper closes them:
//!
//! - it never gives up: it waits for Ollama with the design's backoff
//!   (1, 2, 4, 8 s, then every 30 s) and warms the model up;
//! - every [`Timing::check_every`] it only PROBES Ollama (a cheap health
//!   call). It warms the model up again only when the probe is not
//!   `Health::Ready` (an Ollama that went away or restarted). It never renews
//!   the `keep_alive` on its own: owner decision, the model leaves the GPU after
//!   30 minutes without requests, and the first request after that is a cold
//!   load. Real requests renew it, because their body carries the `keep_alive`;
//! - [`WarmLlm`] tells it when a request failed (Ollama unreachable, a timeout,
//!   or any error that took as long as a model load), so the model is warmed up
//!   again at once instead of at the next check. A quick `LlmError::Failed`
//!   (bad model output) and a slow SUCCESS do not count: the slow success is the
//!   call that loaded the model.
//!
//! Readiness means "Ollama answered and the model was warmed up at least once
//! since it last went wrong". It does NOT mean the model is in VRAM right now.
//!
//! The loop is [`run`], pure over the provider's transport, an injected sleep
//! and a stop switch, so it is tested without a network or real waiting.

use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, Sender, channel};
use std::time::{Duration, Instant};

use niva_core::ports::{ChatReply, ChatRequest, LlmError, LlmProvider};

use super::OllamaProvider;
use super::health::{Health, start_up};

#[cfg(test)]
mod tests;

/// Whether Ollama answered and the model was warmed up at least once since it
/// last went wrong. NOT "the model is in VRAM now": after 30 idle minutes Ollama
/// unloads it, and the first request loads it again.
#[derive(Default)]
pub struct Readiness(AtomicBool);

impl Readiness {
    pub fn is_ready(&self) -> bool {
        self.0.load(Ordering::SeqCst)
    }

    pub fn set(&self, ready: bool) {
        self.0.store(ready, Ordering::SeqCst);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Timing {
    /// How often Ollama is probed while nothing went wrong. A probe never
    /// loads or renews the model; only a failed probe leads to a warm-up.
    pub check_every: Duration,
    /// Pause after a failure report before the model is warmed up again, so a
    /// burst of failures costs one warm-up and an Ollama that is restarting
    /// has a moment to come up.
    pub settle: Duration,
}

pub const TIMING: Timing = Timing {
    check_every: Duration::from_secs(10 * 60),
    settle: Duration::from_secs(3),
};

/// A chat call that FAILED after at least this long suggests a model load or a
/// stuck server: the reducer gives up on a request after 15 s, so a call that
/// took 10 s was not a quick refusal.
pub const SLOW_CALL: Duration = Duration::from_secs(10);

/// Whether the outcome of a chat call says the model should be warmed up again:
/// Ollama could not be reached, the call timed out (the warm-up finishes the
/// load, so the retry is fast), or it failed after at least [`SLOW_CALL`]. NOT
/// a quick `LlmError::Failed` (a reply the adapter could not use says nothing
/// about the model being cold), and NOT a slow success: that call is the one
/// that loaded the model, so it is warm now and a re-warm would only flip the
/// readiness to false for the settle pause.
pub fn needs_rewarm(outcome: &Result<ChatReply, LlmError>, took: Duration) -> bool {
    match outcome {
        Ok(_) => false,
        Err(LlmError::Unavailable | LlmError::Timeout) => true,
        Err(_) => took >= SLOW_CALL,
    }
}

/// Waits for Ollama, warms the model up, then watches it. Returns when
/// `wakes` has no sender left or `keep_going` (asked after each round; the app
/// passes "always") says stop.
///
/// The first round and a round that follows a failure report start cold: the
/// model is not trusted until it has been warmed up. A round that follows the
/// timer only probes Ollama: with a `Health::Ready` answer it makes no chat
/// call, so Ollama's own `keep_alive` can expire. A probe that is not ready
/// marks the server not ready and goes through the backoff and warm-up.
pub fn run(
    provider: &OllamaProvider,
    wakes: &Receiver<()>,
    ready: &Readiness,
    timing: Timing,
    sleep: &dyn Fn(Duration),
    log: &dyn Fn(String),
    keep_going: &dyn Fn() -> bool,
) {
    let mut cold = true;
    loop {
        if cold || provider.health() != Health::Ready {
            ready.set(false);
            // `None`: never give up (autostart race, Ollama restarting).
            match start_up(provider, None, sleep, log) {
                Ok(_) => ready.set(true),
                Err(why) => log(format!("[niva-ollama] start-up ended: {why:?}")),
            }
        }
        if !keep_going() {
            return;
        }
        match wakes.recv_timeout(timing.check_every) {
            Ok(()) => {
                // A burst of reports is one warm-up.
                while wakes.try_recv().is_ok() {}
                sleep(timing.settle);
                cold = true;
            }
            Err(RecvTimeoutError::Timeout) => cold = false,
            Err(RecvTimeoutError::Disconnected) => return,
        }
    }
}

/// What the app's code holds on to: the readiness flag and the way to say
/// "warm it up again".
#[derive(Clone)]
pub struct KeeperHandle {
    pub ready: Arc<Readiness>,
    wake: Sender<()>,
}

impl KeeperHandle {
    pub fn new(ready: Arc<Readiness>, wake: Sender<()>) -> Self {
        KeeperHandle { ready, wake }
    }

    /// The model should be warmed up again. Never blocks.
    pub fn report_cold(&self) {
        self.ready.set(false);
        // No keeper left (the app is closing): nothing to wake.
        let _ = self.wake.send(());
    }
}

/// The provider as the assistant sees it: every call is passed through, and a
/// failed or slow one tells the keeper.
pub struct WarmLlm {
    inner: Arc<dyn LlmProvider>,
    keeper: KeeperHandle,
}

impl WarmLlm {
    pub fn new(inner: Arc<dyn LlmProvider>, keeper: KeeperHandle) -> Self {
        WarmLlm { inner, keeper }
    }
}

impl LlmProvider for WarmLlm {
    fn chat(&self, request: &ChatRequest) -> Result<ChatReply, LlmError> {
        let started = Instant::now();
        let outcome = self.inner.chat(request);
        if needs_rewarm(&outcome, started.elapsed()) {
            self.keeper.report_cold();
        }
        outcome
    }
}

/// Starts the keeper on its own thread. The provider is warmed up in the
/// background; commands before that get the provider's own truthful errors.
pub fn spawn(
    provider: Arc<OllamaProvider>,
    log: Arc<dyn Fn(String) + Send + Sync>,
) -> KeeperHandle {
    let ready = Arc::new(Readiness::default());
    let (wake, wakes) = channel();
    let handle = KeeperHandle::new(ready.clone(), wake);
    let spawned = std::thread::Builder::new()
        .name("niva-ollama-keeper".into())
        .spawn(move || {
            run(
                &provider,
                &wakes,
                &ready,
                TIMING,
                &std::thread::sleep,
                &|line| log(line),
                &|| true,
            );
        });
    if let Err(why) = spawned {
        eprintln!("[niva] could not start the Ollama keeper: {why}");
    }
    handle
}
