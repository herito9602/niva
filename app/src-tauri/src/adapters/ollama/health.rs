//! Ollama health, start-up and CPU-offload detection (design "Environment and
//! Health"). Pure over a [`Transport`], so it is tested without a network.

use std::time::Duration;

use super::wire::{Loaded, error_message, parse_ps, parse_tags};
use super::{Endpoint, OllamaProvider, PROBE_TIMEOUT, Transport, TransportError};

#[cfg(test)]
mod tests;

/// Where Ollama stands. Three distinct degraded states, each with its own
/// message (design: not running, model not pulled, anything else). "Not
/// installed" cannot be told apart from "not running" over HTTP; the message
/// covers both.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Health {
    Ready,
    NotRunning,
    ModelMissing,
    Failed(String),
}

impl Health {
    /// A line for the log and the debug window.
    pub fn message(&self, model: &str) -> String {
        match self {
            Health::Ready => "Ollama is ready.".into(),
            Health::NotRunning => {
                "Ollama is not running (or not installed). Start it and Niva will connect.".into()
            }
            Health::ModelMissing => {
                format!("The model {model} is not installed. Run: ollama pull {model}")
            }
            Health::Failed(why) => format!("Ollama is not answering as expected: {why}"),
        }
    }
}

/// `installed` is a name from `/api/tags` or `/api/ps`; a wanted name without a
/// tag means `:latest`, as in Ollama.
fn same_model(installed: &str, wanted: &str) -> bool {
    installed == wanted || (!wanted.contains(':') && installed == format!("{wanted}:latest"))
}

/// One probe of `GET /api/tags`.
pub fn check(transport: &dyn Transport, model: &str) -> Health {
    match transport.get(Endpoint::Tags, PROBE_TIMEOUT) {
        Err(TransportError::Refused) => Health::NotRunning,
        Err(TransportError::Timeout) => Health::Failed("timed out".into()),
        Err(TransportError::Other(why)) => Health::Failed(why),
        Ok(reply) if reply.status != 200 => Health::Failed(format!(
            "HTTP {}: {}",
            reply.status,
            error_message(&reply.body)
        )),
        Ok(reply) => match parse_tags(&reply.body) {
            Err(why) => Health::Failed(format!("unreadable /api/tags: {why}")),
            Ok(models) if models.iter().any(|name| same_model(name, model)) => Health::Ready,
            Ok(_) => Health::ModelMissing,
        },
    }
}

/// Where the loaded model lives.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Offload {
    /// All of it in GPU memory.
    Gpu,
    /// Part of it spilled to the CPU.
    Partial { vram_percent: u8 },
    /// None of it on the GPU.
    CpuOnly,
    /// Not loaded yet (cold): nothing to judge.
    NotLoaded,
}

impl Offload {
    /// "Slow mode": the latency target will not hold (design: partial CPU offload).
    pub fn is_slow(self) -> bool {
        matches!(self, Offload::Partial { .. } | Offload::CpuOnly)
    }
}

/// The product model as `GET /api/ps` reports it, if it is loaded.
pub fn loaded(transport: &dyn Transport, model: &str) -> Result<Option<Loaded>, String> {
    let reply = transport
        .get(Endpoint::Ps, PROBE_TIMEOUT)
        .map_err(|e| format!("{e:?}"))?;
    if reply.status != 200 {
        return Err(format!(
            "HTTP {}: {}",
            reply.status,
            error_message(&reply.body)
        ));
    }
    Ok(parse_ps(&reply.body)?
        .into_iter()
        .find(|m| same_model(&m.name, model)))
}

/// `GET /api/ps`: compares `size_vram` with `size` for the product model.
pub fn offload(transport: &dyn Transport, model: &str) -> Result<Offload, String> {
    let Some(ours) = loaded(transport, model)? else {
        return Ok(Offload::NotLoaded);
    };
    Ok(if ours.size == 0 {
        Offload::NotLoaded
    } else if ours.size_vram == 0 {
        Offload::CpuOnly
    } else if ours.size_vram >= ours.size {
        Offload::Gpu
    } else {
        Offload::Partial {
            vram_percent: (ours.size_vram.saturating_mul(100) / ours.size) as u8,
        }
    })
}

/// Wait before retry number `attempt` (0-based): 1, 2, 4, 8 s, then every 30 s.
pub fn backoff_delay(attempt: u32) -> Duration {
    match attempt {
        0 => Duration::from_secs(1),
        1 => Duration::from_secs(2),
        2 => Duration::from_secs(4),
        3 => Duration::from_secs(8),
        _ => Duration::from_secs(30),
    }
}

/// What a successful start-up learned.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StartUp {
    /// Health probes it took until Ollama was ready.
    pub attempts: u32,
    /// How long the successful warm-up call took (the model load).
    pub warm_up: Duration,
    /// `None` when `/api/ps` could not be read.
    pub offload: Option<Offload>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StartUpError {
    /// Out of attempts while waiting for Ollama or the model.
    NotReady(Health),
    /// Ollama is up but the warm-up call kept failing.
    WarmUp(String),
}

/// Waits for Ollama (health probe with backoff), warms the model up (the same
/// backoff), then reads `/api/ps` for slow mode. `max_attempts` bounds each of
/// the two waits (`None` = keep trying, as the app does in the background).
/// `sleep` is injected so tests do not wait.
pub fn start_up(
    provider: &OllamaProvider,
    max_attempts: Option<u32>,
    sleep: &dyn Fn(Duration),
    log: &dyn Fn(String),
) -> Result<StartUp, StartUpError> {
    let model = provider.model();
    let out_of_attempts = |attempt: u32| max_attempts.is_some_and(|max| attempt + 1 >= max);

    let mut attempt = 0;
    loop {
        let health = provider.health();
        if health == Health::Ready {
            log(format!("[niva-ollama] {}", health.message(model)));
            break;
        }
        log(format!("[niva-ollama] {}", health.message(model)));
        if out_of_attempts(attempt) {
            return Err(StartUpError::NotReady(health));
        }
        sleep(backoff_delay(attempt));
        attempt += 1;
    }
    let attempts = attempt + 1;

    let mut tries = 0;
    let warm_up = loop {
        match provider.warm_up() {
            Ok(took) => break took,
            Err(why) => {
                log(format!("[niva-ollama] warm-up failed: {why:?}"));
                if out_of_attempts(tries) {
                    return Err(StartUpError::WarmUp(format!("{why:?}")));
                }
                sleep(backoff_delay(tries));
                tries += 1;
            }
        }
    };
    log(format!(
        "[niva-ollama] {model} warmed up in {:.1} s",
        warm_up.as_secs_f64()
    ));

    let offload = match provider.offload() {
        Ok(offload) => {
            if offload.is_slow() {
                log(format!(
                    "[niva-ollama] slow mode: {offload:?}. The model is not fully on the GPU, so the latency target will not hold."
                ));
            }
            Some(offload)
        }
        Err(why) => {
            log(format!("[niva-ollama] could not read /api/ps: {why}"));
            None
        }
    };
    Ok(StartUp {
        attempts,
        warm_up,
        offload,
    })
}
