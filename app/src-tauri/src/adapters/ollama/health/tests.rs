//! Health, backoff, warm-up and slow-mode detection over a fake transport.

use std::cell::RefCell;
use std::sync::Arc;
use std::time::Duration;

use super::*;
use crate::adapters::ollama::fake::{FakeTransport, Scripted, ok, status, tags};
use crate::adapters::ollama::{
    CHAT_TIMEOUT, Endpoint, MODEL, OllamaProvider, PROBE_TIMEOUT, TransportError, WARM_UP_TIMEOUT,
};

fn transport() -> Arc<FakeTransport> {
    Arc::new(FakeTransport::default())
}

fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

// --- check ------------------------------------------------------------------

fn check_with(answer: Scripted) -> (Health, Arc<FakeTransport>) {
    let t = transport();
    t.push(Endpoint::Tags, answer);
    (check(&*t, MODEL), t)
}

#[test]
fn ready_when_the_model_is_installed() {
    let (health, t) = check_with(tags(&["llama3.2:3b", MODEL]));
    assert_eq!(health, Health::Ready);
    assert_eq!(t.seen()[0].endpoint, Endpoint::Tags);
    assert_eq!(t.seen()[0].timeout, PROBE_TIMEOUT);
}

#[test]
fn the_model_missing_names_the_pull_command() {
    let (health, _) = check_with(tags(&["llama3.2:3b"]));
    assert_eq!(health, Health::ModelMissing);
    assert!(
        Health::ModelMissing
            .message(MODEL)
            .contains("ollama pull qwen3.5:4b-q4_K_M")
    );
}

#[test]
fn a_refused_connection_is_not_running() {
    let (health, _) = check_with(Err(TransportError::Refused));
    assert_eq!(health, Health::NotRunning);
    assert!(
        Health::NotRunning
            .message(MODEL)
            .to_lowercase()
            .contains("not running")
    );
}

#[test]
fn timeouts_odd_statuses_and_garbage_are_failures() {
    for answer in [
        Err(TransportError::Timeout),
        Err(TransportError::Other("x".into())),
        status(500, "boom"),
        ok("not json"),
        ok(r#"{"nothing":1}"#),
    ] {
        let (health, _) = check_with(answer);
        assert!(matches!(health, Health::Failed(_)), "{health:?}");
    }
}

#[test]
fn an_untagged_model_name_matches_latest() {
    let t = transport();
    t.push(Endpoint::Tags, tags(&["llama3.2:latest"]));
    assert_eq!(check(&*t, "llama3.2"), Health::Ready);
    let t = transport();
    t.push(Endpoint::Tags, tags(&["llama3.2:3b"]));
    assert_eq!(check(&*t, "llama3.2"), Health::ModelMissing);
}

// --- backoff ----------------------------------------------------------------

#[test]
fn backoff_is_1_2_4_8_then_every_30_seconds() {
    let delays: Vec<u64> = (0..8).map(|n| backoff_delay(n).as_secs()).collect();
    assert_eq!(delays, [1, 2, 4, 8, 30, 30, 30, 30]);
    assert_eq!(backoff_delay(u32::MAX), secs(30));
}

// --- offload (slow mode) ----------------------------------------------------

fn ps(models: &[(&str, u64, u64)]) -> Scripted {
    let models: Vec<_> = models
        .iter()
        .map(|(name, size, vram)| {
            serde_json::json!({"name": name, "size": size, "size_vram": vram, "digest": "d"})
        })
        .collect();
    ok(&serde_json::json!({ "models": models }).to_string())
}

fn offload_with(answer: Scripted) -> Result<Offload, String> {
    let t = transport();
    t.push(Endpoint::Ps, answer);
    offload(&*t, MODEL)
}

#[test]
fn fully_on_the_gpu_is_not_slow() {
    let result = offload_with(ps(&[(MODEL, 5_000, 5_000)])).unwrap();
    assert_eq!(result, Offload::Gpu);
    assert!(!result.is_slow());
}

#[test]
fn cpu_only_is_slow() {
    let result = offload_with(ps(&[(MODEL, 5_000, 0)])).unwrap();
    assert_eq!(result, Offload::CpuOnly);
    assert!(result.is_slow());
}

#[test]
fn partly_offloaded_is_slow_and_says_how_much_is_on_the_gpu() {
    let result = offload_with(ps(&[(MODEL, 5_000, 3_750)])).unwrap();
    assert_eq!(result, Offload::Partial { vram_percent: 75 });
    assert!(result.is_slow());
}

#[test]
fn a_model_that_is_not_loaded_is_not_slow_just_cold() {
    let result = offload_with(ps(&[("other:1b", 1_000, 1_000)])).unwrap();
    assert_eq!(result, Offload::NotLoaded);
    assert!(!result.is_slow());
    assert_eq!(offload_with(ps(&[])).unwrap(), Offload::NotLoaded);
}

#[test]
fn only_the_product_model_counts_when_several_are_loaded() {
    let result = offload_with(ps(&[("other:1b", 1_000, 0), (MODEL, 5_000, 5_000)])).unwrap();
    assert_eq!(result, Offload::Gpu);
}

#[test]
fn the_loaded_model_is_found_with_its_sizes() {
    let t = transport();
    t.push(
        Endpoint::Ps,
        ps(&[("other:1b", 1, 1), (MODEL, 5_000, 4_000)]),
    );
    let found = loaded(&*t, MODEL).unwrap().expect("loaded");
    assert_eq!((found.size, found.size_vram), (5_000, 4_000));
    let t = transport();
    t.push(Endpoint::Ps, ps(&[]));
    assert_eq!(loaded(&*t, MODEL).unwrap(), None);
}

#[test]
fn ps_uses_the_probe_timeout_and_reports_errors() {
    let t = transport();
    t.push(Endpoint::Ps, Err(TransportError::Refused));
    assert!(offload(&*t, MODEL).is_err());
    assert_eq!(t.seen()[0].timeout, PROBE_TIMEOUT);
    assert!(offload_with(status(500, "x")).is_err());
    assert!(offload_with(ok("junk")).is_err());
}

// --- start-up: wait for Ollama, warm up, look at /api/ps ---------------------

const WARM_OK: &str = r#"{"message":{"content":"OK"}}"#;

struct Rig {
    transport: Arc<FakeTransport>,
    provider: OllamaProvider,
    slept: RefCell<Vec<Duration>>,
    log: RefCell<Vec<String>>,
}

impl Rig {
    fn new() -> Rig {
        let transport = transport();
        Rig {
            provider: OllamaProvider::with_transport(transport.clone()),
            transport,
            slept: RefCell::default(),
            log: RefCell::default(),
        }
    }

    fn start(&self, max_attempts: Option<u32>) -> Result<StartUp, StartUpError> {
        start_up(
            &self.provider,
            max_attempts,
            &|d| self.slept.borrow_mut().push(d),
            &|line| self.log.borrow_mut().push(line),
        )
    }
}

#[test]
fn start_up_waits_with_backoff_until_ollama_answers() {
    let rig = Rig::new();
    let t = &rig.transport;
    t.push(Endpoint::Tags, Err(TransportError::Refused))
        .push(Endpoint::Tags, Err(TransportError::Refused))
        .push(Endpoint::Tags, tags(&[MODEL]))
        .push(Endpoint::Chat, ok(WARM_OK))
        .push(Endpoint::Ps, ps(&[(MODEL, 5_000, 5_000)]));
    let started = rig.start(None).expect("starts");
    assert_eq!(*rig.slept.borrow(), [secs(1), secs(2)]);
    assert_eq!(started.attempts, 3);
    assert_eq!(started.offload, Some(Offload::Gpu));
    let seen = rig.transport.seen();
    assert_eq!(
        seen.iter().map(|s| s.endpoint).collect::<Vec<_>>(),
        [
            Endpoint::Tags,
            Endpoint::Tags,
            Endpoint::Tags,
            Endpoint::Chat,
            Endpoint::Ps
        ]
    );
    assert_eq!(seen[3].timeout, WARM_UP_TIMEOUT);
    assert!(WARM_UP_TIMEOUT > CHAT_TIMEOUT);
    assert!(rig.log.borrow().iter().any(|l| l.contains("ready")));
}

#[test]
fn start_up_gives_up_after_the_attempt_limit_with_the_reason() {
    let rig = Rig::new();
    for _ in 0..3 {
        rig.transport
            .push(Endpoint::Tags, Err(TransportError::Refused));
    }
    let err = rig.start(Some(3)).unwrap_err();
    assert_eq!(err, StartUpError::NotReady(Health::NotRunning));
    // It sleeps between attempts, not after the last one.
    assert_eq!(*rig.slept.borrow(), [secs(1), secs(2)]);
    assert!(
        rig.log
            .borrow()
            .iter()
            .any(|l| l.to_lowercase().contains("not running"))
    );
}

#[test]
fn a_missing_model_is_reported_and_retried_too() {
    let rig = Rig::new();
    rig.transport
        .push(Endpoint::Tags, tags(&["other:1b"]))
        .push(Endpoint::Tags, tags(&["other:1b"]));
    let err = rig.start(Some(2)).unwrap_err();
    assert_eq!(err, StartUpError::NotReady(Health::ModelMissing));
    assert!(rig.log.borrow().iter().any(|l| l.contains("ollama pull")));
}

#[test]
fn the_warm_up_is_retried_with_the_same_backoff() {
    let rig = Rig::new();
    rig.transport
        .push(Endpoint::Tags, tags(&[MODEL]))
        .push(Endpoint::Chat, Err(TransportError::Timeout))
        .push(Endpoint::Chat, ok(WARM_OK))
        .push(Endpoint::Ps, ps(&[(MODEL, 5_000, 5_000)]));
    let started = rig.start(Some(3)).expect("starts");
    assert_eq!(*rig.slept.borrow(), [secs(1)]);
    assert!(started.warm_up < secs(5));
}

#[test]
fn a_warm_up_that_never_works_is_an_error() {
    let rig = Rig::new();
    rig.transport.push(Endpoint::Tags, tags(&[MODEL]));
    for _ in 0..2 {
        rig.transport
            .push(Endpoint::Chat, Err(TransportError::Timeout));
    }
    let err = rig.start(Some(2)).unwrap_err();
    assert!(matches!(err, StartUpError::WarmUp(_)), "{err:?}");
}

#[test]
fn slow_mode_is_logged_loudly_but_does_not_stop_start_up() {
    let rig = Rig::new();
    rig.transport
        .push(Endpoint::Tags, tags(&[MODEL]))
        .push(Endpoint::Chat, ok(WARM_OK))
        .push(Endpoint::Ps, ps(&[(MODEL, 5_000, 0)]));
    let started = rig.start(None).expect("starts");
    assert_eq!(started.offload, Some(Offload::CpuOnly));
    assert!(started.offload.unwrap().is_slow());
    let log = lock_log(&rig);
    assert!(log.iter().any(|l| l.contains("slow mode")), "{log:?}");
}

#[test]
fn an_unreadable_ps_leaves_the_offload_unknown() {
    let rig = Rig::new();
    rig.transport
        .push(Endpoint::Tags, tags(&[MODEL]))
        .push(Endpoint::Chat, ok(WARM_OK))
        .push(Endpoint::Ps, status(500, "x"));
    let started = rig.start(None).expect("starts");
    assert_eq!(started.offload, None);
}

fn lock_log(rig: &Rig) -> Vec<String> {
    rig.log.borrow().clone()
}

#[test]
fn health_messages_are_distinct_for_the_three_degraded_states() {
    let messages = [
        Health::NotRunning.message(MODEL),
        Health::ModelMissing.message(MODEL),
        Health::Failed("x".into()).message(MODEL),
    ];
    assert_ne!(messages[0], messages[1]);
    assert_ne!(messages[1], messages[2]);
}
