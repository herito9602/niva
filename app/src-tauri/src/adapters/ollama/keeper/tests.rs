//! The keeper loop, the failure report and the pass-through LLM, over a fake
//! transport. No network, no real waiting.

use std::cell::{Cell, RefCell};
use std::sync::mpsc::channel;

use super::*;
use crate::adapters::ollama::fake::{FakeTransport, ok, tags};
use crate::adapters::ollama::{Endpoint, MODEL, TransportError};

const WARM_OK: &str = r#"{"message":{"content":"OK"}}"#;
const PS_GPU: &str = r#"{"models":[{"name":"qwen3.5:4b-q4_K_M","size":5000,"size_vram":5000}]}"#;

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

fn timing() -> Timing {
    Timing {
        check_every: Duration::from_millis(5),
        settle: secs(3),
    }
}

/// One round of "Ollama is fine": a probe, a warm-up, a `/api/ps` read.
fn healthy_round(t: &FakeTransport) {
    t.push(Endpoint::Tags, tags(&[MODEL]))
        .push(Endpoint::Chat, ok(WARM_OK))
        .push(Endpoint::Ps, ok(PS_GPU));
}

struct Rig {
    transport: Arc<FakeTransport>,
    provider: OllamaProvider,
    ready: Readiness,
    slept: RefCell<Vec<Duration>>,
    log: RefCell<Vec<String>>,
    rounds: Cell<usize>,
}

impl Rig {
    fn new() -> Rig {
        let transport = Arc::new(FakeTransport::default());
        Rig {
            provider: OllamaProvider::with_transport(transport.clone()),
            transport,
            ready: Readiness::default(),
            slept: RefCell::default(),
            log: RefCell::default(),
            rounds: Cell::new(0),
        }
    }

    /// Runs until `rounds` warm-ups have completed (asked after each one).
    fn run(&self, wakes: &Receiver<()>, timing: Timing, rounds: usize) {
        run(
            &self.provider,
            wakes,
            &self.ready,
            timing,
            &|d| self.slept.borrow_mut().push(d),
            &|line| self.log.borrow_mut().push(line),
            &|| {
                self.rounds.set(self.rounds.get() + 1);
                self.rounds.get() < rounds
            },
        );
    }

    fn chats(&self) -> usize {
        self.transport
            .seen()
            .iter()
            .filter(|s| s.endpoint == Endpoint::Chat)
            .count()
    }
}

// --- the loop ----------------------------------------------------------------

#[test]
fn it_waits_for_an_ollama_that_is_not_ready_at_the_first_probe() {
    // The autostart race: Niva starts before Ollama.
    let rig = Rig::new();
    rig.transport
        .push(Endpoint::Tags, Err(TransportError::Refused))
        .push(Endpoint::Tags, Err(TransportError::Refused))
        .push(Endpoint::Tags, tags(&[MODEL]))
        .push(Endpoint::Chat, ok(WARM_OK))
        .push(Endpoint::Ps, ok(PS_GPU));
    let (_keep, wakes) = channel::<()>();
    rig.run(
        &wakes,
        Timing {
            check_every: secs(3600),
            ..timing()
        },
        1,
    );
    assert_eq!(
        *rig.slept.borrow(),
        [secs(1), secs(2)],
        "the design's backoff"
    );
    assert!(rig.ready.is_ready());
    assert_eq!(rig.chats(), 1);
}

#[test]
fn it_never_gives_up_and_the_wait_settles_at_30_seconds() {
    let rig = Rig::new();
    for _ in 0..9 {
        rig.transport
            .push(Endpoint::Tags, Err(TransportError::Refused));
    }
    healthy_round(&rig.transport);
    let (_keep, wakes) = channel::<()>();
    rig.run(
        &wakes,
        Timing {
            check_every: secs(3600),
            ..timing()
        },
        1,
    );
    let slept = rig.slept.borrow();
    assert_eq!(
        slept.len(),
        9,
        "more attempts than the app's old limit of six"
    );
    assert_eq!(slept[..4], [secs(1), secs(2), secs(4), secs(8)]);
    assert!(slept[4..].iter().all(|d| *d == secs(30)));
    assert!(rig.ready.is_ready());
}

#[test]
fn a_failure_report_warms_the_model_up_again_after_a_short_settle() {
    let rig = Rig::new();
    healthy_round(&rig.transport);
    healthy_round(&rig.transport);
    let (wake, wakes) = channel::<()>();
    wake.send(()).unwrap();
    wake.send(()).unwrap();
    wake.send(()).unwrap();
    rig.run(
        &wakes,
        Timing {
            check_every: secs(3600),
            ..timing()
        },
        2,
    );
    assert_eq!(rig.chats(), 2, "a burst of reports costs one extra warm-up");
    assert_eq!(*rig.slept.borrow(), [secs(3)], "settled once");
    assert!(rig.ready.is_ready());
}

#[test]
fn a_timer_round_with_a_healthy_server_makes_no_warm_up_call() {
    // The keep_alive must be allowed to expire: an idle Niva does not renew it.
    let rig = Rig::new();
    healthy_round(&rig.transport);
    // The periodic rounds only probe.
    rig.transport.push(Endpoint::Tags, tags(&[MODEL]));
    rig.transport.push(Endpoint::Tags, tags(&[MODEL]));
    let (_keep, wakes) = channel::<()>();
    rig.run(&wakes, timing(), 3);
    assert_eq!(rig.chats(), 1, "only the start-up warm-up");
    let probes = rig
        .transport
        .seen()
        .iter()
        .filter(|s| s.endpoint == Endpoint::Tags)
        .count();
    assert_eq!(probes, 3, "start-up probe plus one probe per timer round");
    assert!(rig.ready.is_ready(), "a healthy probe keeps it ready");
    assert!(
        rig.slept.borrow().is_empty(),
        "no settle pause on the timer"
    );
}

#[test]
fn an_ollama_that_went_away_is_warmed_up_when_it_returns_via_the_timer() {
    let rig = Rig::new();
    healthy_round(&rig.transport);
    // Round 2: the probe finds Ollama gone, the backoff waits, it comes back.
    rig.transport
        .push(Endpoint::Tags, Err(TransportError::Refused))
        .push(Endpoint::Tags, Err(TransportError::Refused))
        .push(Endpoint::Tags, tags(&[MODEL]))
        .push(Endpoint::Chat, ok(WARM_OK))
        .push(Endpoint::Ps, ok(PS_GPU));
    // Round 3: healthy again, so the timer only probes.
    rig.transport.push(Endpoint::Tags, tags(&[MODEL]));
    let (_keep, wakes) = channel::<()>();
    rig.run(&wakes, timing(), 3);
    assert_eq!(
        rig.chats(),
        2,
        "start-up and the return, not the third round"
    );
    assert_eq!(*rig.slept.borrow(), [secs(1)]);
    assert!(rig.ready.is_ready());
}

#[test]
fn an_ollama_that_restarts_is_noticed_and_reconnected() {
    let rig = Rig::new();
    healthy_round(&rig.transport);
    // The timer tick finds Ollama gone, waits with backoff, then warms up.
    rig.transport
        .push(Endpoint::Tags, Err(TransportError::Refused))
        .push(Endpoint::Tags, Err(TransportError::Refused))
        .push(Endpoint::Tags, tags(&[MODEL]))
        .push(Endpoint::Chat, ok(WARM_OK))
        .push(Endpoint::Ps, ok(PS_GPU));
    let (_keep, wakes) = channel::<()>();
    rig.run(&wakes, timing(), 2);
    assert_eq!(*rig.slept.borrow(), [secs(1)]);
    assert!(rig.ready.is_ready());
    assert_eq!(rig.chats(), 2);
}

#[test]
fn it_is_not_ready_while_ollama_is_down() {
    let rig = Rig::new();
    healthy_round(&rig.transport);
    rig.transport
        .push(Endpoint::Tags, Err(TransportError::Refused))
        .push(Endpoint::Tags, Err(TransportError::Refused));
    healthy_round(&rig.transport);
    let (_keep, wakes) = channel::<()>();
    let during_backoff = RefCell::new(Vec::new());
    let rounds = Cell::new(0);
    run(
        &rig.provider,
        &wakes,
        &rig.ready,
        timing(),
        &|_| during_backoff.borrow_mut().push(rig.ready.is_ready()),
        &|_| {},
        &|| {
            rounds.set(rounds.get() + 1);
            rounds.get() < 2
        },
    );
    assert_eq!(*during_backoff.borrow(), [false], "not ready while waiting");
    assert!(rig.ready.is_ready(), "ready again once it is back");
}

#[test]
fn the_loop_ends_when_nobody_can_wake_it_any_more() {
    let rig = Rig::new();
    healthy_round(&rig.transport);
    let (wake, wakes) = channel::<()>();
    drop(wake);
    // `keep_going` would allow more rounds, but the channel is disconnected.
    rig.run(
        &wakes,
        Timing {
            check_every: secs(3600),
            ..timing()
        },
        5,
    );
    assert_eq!(rig.chats(), 1);
}

#[test]
fn a_model_that_is_missing_keeps_waiting_and_says_what_to_pull() {
    let rig = Rig::new();
    rig.transport.push(Endpoint::Tags, tags(&["llama3.2:3b"]));
    healthy_round(&rig.transport);
    let (_keep, wakes) = channel::<()>();
    rig.run(
        &wakes,
        Timing {
            check_every: secs(3600),
            ..timing()
        },
        1,
    );
    assert!(rig.log.borrow().iter().any(|l| l.contains("ollama pull")));
    assert!(rig.ready.is_ready());
}

// --- when a call means "warm it up again" -----------------------------------------

fn reply() -> Result<ChatReply, LlmError> {
    Ok(ChatReply::default())
}

#[test]
fn a_quick_success_needs_nothing() {
    assert!(!needs_rewarm(&reply(), secs(1)));
    assert!(!needs_rewarm(
        &reply(),
        SLOW_CALL - Duration::from_millis(1)
    ));
}

#[test]
fn a_slow_success_does_not_need_a_rewarm() {
    // That call is the one that loaded the model (the normal path after an idle
    // unload): it is warm now, and a re-warm would only flip readiness off.
    assert!(!needs_rewarm(&reply(), SLOW_CALL));
    assert!(!needs_rewarm(&reply(), secs(16)));
}

#[test]
fn a_slow_error_asks_for_a_rewarm() {
    for error in [
        LlmError::Failed("HTTP 500".into()),
        LlmError::Failed("bad tool call".into()),
    ] {
        assert!(needs_rewarm(&Err(error.clone()), SLOW_CALL), "{error:?}");
    }
    assert!(!needs_rewarm(
        &Err(LlmError::Failed("HTTP 500".into())),
        SLOW_CALL - Duration::from_millis(1)
    ));
}

#[test]
fn an_unreachable_or_timed_out_call_asks_for_a_rewarm() {
    for error in [LlmError::Unavailable, LlmError::Timeout] {
        assert!(
            needs_rewarm(&Err(error.clone()), Duration::ZERO),
            "{error:?}"
        );
    }
}

#[test]
fn a_fast_failed_call_is_bad_model_output_not_a_cold_model() {
    // LlmError::Failed covers a reply the adapter could not use (malformed
    // JSON, an unknown tool shape): the model is warm, warming it up again would
    // only burn a refresh and flip the island to "not ready" for a few seconds.
    assert!(!needs_rewarm(
        &Err(LlmError::Failed("bad tool call".into())),
        Duration::from_millis(300)
    ));
    // ... but a failure that took as long as a model load is still slow.
    assert!(needs_rewarm(
        &Err(LlmError::Failed("HTTP 500".into())),
        SLOW_CALL
    ));
}

// --- the pass-through LLM ------------------------------------------------------------

struct Scripted(std::sync::Mutex<Vec<Result<ChatReply, LlmError>>>);

impl LlmProvider for Scripted {
    fn chat(&self, _request: &ChatRequest) -> Result<ChatReply, LlmError> {
        self.0.lock().unwrap().remove(0)
    }
}

fn request() -> ChatRequest {
    ChatRequest {
        system: "s".into(),
        user: "u".into(),
        allow_tools: true,
    }
}

fn warm_llm(script: Vec<Result<ChatReply, LlmError>>) -> (WarmLlm, Receiver<()>, Arc<Readiness>) {
    let (wake, wakes) = channel();
    let ready = Arc::new(Readiness::default());
    ready.set(true);
    let keeper = KeeperHandle::new(ready.clone(), wake);
    (
        WarmLlm::new(Arc::new(Scripted(std::sync::Mutex::new(script))), keeper),
        wakes,
        ready,
    )
}

#[test]
fn a_good_answer_passes_through_and_wakes_nobody() {
    let answer = ChatReply {
        text: "hi".into(),
        calls: Vec::new(),
    };
    let (llm, wakes, ready) = warm_llm(vec![Ok(answer.clone())]);
    assert_eq!(llm.chat(&request()), Ok(answer));
    assert!(wakes.try_recv().is_err());
    assert!(ready.is_ready());
}

#[test]
fn a_failed_answer_passes_through_marks_not_ready_and_wakes_the_keeper() {
    let (llm, wakes, ready) = warm_llm(vec![Err(LlmError::Unavailable)]);
    assert_eq!(llm.chat(&request()), Err(LlmError::Unavailable));
    assert!(wakes.try_recv().is_ok(), "the keeper was told");
    assert!(!ready.is_ready());
}

#[test]
fn a_fast_failed_answer_does_not_wake_the_keeper_or_mark_the_model_cold() {
    let (llm, wakes, ready) = warm_llm(vec![Err(LlmError::Failed("bad output".into()))]);
    assert_eq!(
        llm.chat(&request()),
        Err(LlmError::Failed("bad output".into()))
    );
    assert!(wakes.try_recv().is_err(), "the keeper was not told");
    assert!(ready.is_ready());
}

#[test]
fn reporting_with_no_keeper_left_does_not_panic() {
    let (wake, wakes) = channel();
    drop(wakes);
    let keeper = KeeperHandle::new(Arc::new(Readiness::default()), wake);
    keeper.report_cold();
}
