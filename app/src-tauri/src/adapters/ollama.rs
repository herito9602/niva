//! Ollama adapter: the product LLM behind the core's [`LlmProvider`] port
//! (design D3, task 2.4).
//!
//! Local only: the address is the constant [`ADDRESS`] (127.0.0.1:11434) and
//! the endpoints are a closed enum, and redirects are never followed, so no
//! request can be pointed at another host. The client is `ureq` (blocking, no async stack, no TLS): the
//! ports are synchronous and the host already runs each call on a worker
//! thread, so a small blocking client is the whole need.
//!
//! - [`wire`]: request bodies and response parsing, pure and tested against
//!   fixtures (tool-call numbers as `30` and `30.0`, arguments as an object or
//!   a string, empty content, malformed JSON).
//! - [`health`]: health probe, backoff, warm-up and `/api/ps` slow-mode check.
//! - A hard timeout on every request ([`CHAT_TIMEOUT`], [`WARM_UP_TIMEOUT`],
//!   [`PROBE_TIMEOUT`]), so a hung Ollama cannot hold a worker forever.

use std::sync::Arc;
use std::time::{Duration, Instant};

use niva_core::ports::{ChatReply, ChatRequest, LlmError, LlmProvider};

pub mod health;
pub mod wire;

#[cfg(test)]
pub(crate) mod fake;
#[cfg(test)]
mod tests;

/// The product model, chosen by E1 (design "Slice 0 results and decisions").
pub const MODEL: &str = "qwen3.5:4b-q4_K_M";
/// Ollama's default address. Never configurable: loopback only.
pub const ADDRESS: &str = "127.0.0.1:11434";
/// Hard limit of one chat call. Just above the reducer's LLM timer
/// (`TimerKind::Llm`, 15 s; a test pins the order): by the time this fires the
/// reducer has already given up on the request, and the worker is free again
/// soon after.
pub const CHAT_TIMEOUT: Duration = Duration::from_secs(16);
/// A cold start loads about 3 GB from disk.
pub const WARM_UP_TIMEOUT: Duration = Duration::from_secs(60);
/// Health and `/api/ps` probes.
pub const PROBE_TIMEOUT: Duration = Duration::from_secs(3);

/// The only Ollama endpoints Niva talks to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Endpoint {
    Tags,
    Ps,
    Chat,
    Version,
}

impl Endpoint {
    pub const fn path(self) -> &'static str {
        match self {
            Endpoint::Tags => "/api/tags",
            Endpoint::Ps => "/api/ps",
            Endpoint::Chat => "/api/chat",
            Endpoint::Version => "/api/version",
        }
    }

    pub fn url(self) -> String {
        format!("http://{ADDRESS}{}", self.path())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HttpReply {
    pub status: u16,
    pub body: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportError {
    /// Nothing listens on the port (Ollama is not running).
    Refused,
    Timeout,
    /// The connection died mid-request (reset, aborted, broken pipe, early EOF):
    /// Ollama went away or crashed while answering.
    Dropped,
    Other(String),
}

/// A blocking HTTP client with a hard timeout per call. A reply with any
/// status is `Ok`: the caller decides what 404 or 500 mean.
pub trait Transport: Send + Sync {
    fn get(&self, endpoint: Endpoint, timeout: Duration) -> Result<HttpReply, TransportError>;
    fn post(
        &self,
        endpoint: Endpoint,
        body: &str,
        timeout: Duration,
    ) -> Result<HttpReply, TransportError>;
}

/// The real transport. Proxies are switched off on purpose: `ureq` reads
/// `HTTP_PROXY`/`ALL_PROXY` by default, and a loopback call must never leave
/// the machine through one. Redirects are switched off for the same reason:
/// the address is a constant, and a 3xx must not move the call off it.
pub struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    pub fn new() -> Self {
        let config = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            // A redirect is never followed: whatever answers on the port must
            // not be able to send a request (the chat body) to another host.
            // The 3xx comes back as a reply and `interpret` / `get_ok` treat it
            // as a failure.
            .max_redirects(0)
            .build();
        UreqTransport {
            agent: ureq::Agent::new_with_config(config),
        }
    }
}

impl Default for UreqTransport {
    fn default() -> Self {
        Self::new()
    }
}

fn map_error(error: ureq::Error) -> TransportError {
    match error {
        ureq::Error::Timeout(_) => TransportError::Timeout,
        ureq::Error::ConnectionFailed => TransportError::Refused,
        ureq::Error::Io(io) => match io.kind() {
            std::io::ErrorKind::ConnectionRefused => TransportError::Refused,
            // ureq reports a peer that vanished mid-request as one of these
            // (`UnexpectedEof` is its own "Peer disconnected"; Windows gives
            // `ConnectionReset`/`ConnectionAborted`).
            std::io::ErrorKind::ConnectionReset
            | std::io::ErrorKind::ConnectionAborted
            | std::io::ErrorKind::BrokenPipe
            | std::io::ErrorKind::UnexpectedEof => TransportError::Dropped,
            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock => {
                TransportError::Timeout
            }
            _ => TransportError::Other(io.to_string()),
        },
        other => TransportError::Other(other.to_string()),
    }
}

fn read(mut response: ureq::http::Response<ureq::Body>) -> Result<HttpReply, TransportError> {
    let status = response.status().as_u16();
    let body = response.body_mut().read_to_string().map_err(map_error)?;
    Ok(HttpReply { status, body })
}

impl UreqTransport {
    fn get_url(&self, url: &str, timeout: Duration) -> Result<HttpReply, TransportError> {
        self.agent
            .get(url)
            .config()
            .timeout_global(Some(timeout))
            .build()
            .call()
            .map_err(map_error)
            .and_then(read)
    }

    fn post_url(
        &self,
        url: &str,
        body: &str,
        timeout: Duration,
    ) -> Result<HttpReply, TransportError> {
        self.agent
            .post(url)
            .header("Content-Type", "application/json")
            .config()
            .timeout_global(Some(timeout))
            .build()
            .send(body)
            .map_err(map_error)
            .and_then(read)
    }
}

impl Transport for UreqTransport {
    fn get(&self, endpoint: Endpoint, timeout: Duration) -> Result<HttpReply, TransportError> {
        self.get_url(&endpoint.url(), timeout)
    }

    fn post(
        &self,
        endpoint: Endpoint,
        body: &str,
        timeout: Duration,
    ) -> Result<HttpReply, TransportError> {
        self.post_url(&endpoint.url(), body, timeout)
    }
}

pub struct OllamaProvider {
    transport: Arc<dyn Transport>,
    model: String,
}

impl OllamaProvider {
    /// The product model over the real transport.
    pub fn new() -> Self {
        Self::with_transport(Arc::new(UreqTransport::new()))
    }

    pub fn with_transport(transport: Arc<dyn Transport>) -> Self {
        OllamaProvider {
            transport,
            model: MODEL.into(),
        }
    }

    /// Another model, for comparisons (`niva-eval --model`).
    pub fn with_model(mut self, model: &str) -> Self {
        self.model = model.into();
        self
    }

    pub fn model(&self) -> &str {
        &self.model
    }

    fn transport(&self) -> &dyn Transport {
        &*self.transport
    }

    pub fn health(&self) -> health::Health {
        health::check(self.transport(), &self.model)
    }

    /// The model's placement as `/api/ps` reports it (sizes in bytes).
    pub fn loaded(&self) -> Result<Option<wire::Loaded>, String> {
        health::loaded(self.transport(), &self.model)
    }

    pub fn offload(&self) -> Result<health::Offload, String> {
        health::offload(self.transport(), &self.model)
    }

    /// The Ollama server version, for the record of an eval run.
    pub fn version(&self) -> Result<String, String> {
        let reply = self.get_ok(Endpoint::Version)?;
        wire::parse_version(&reply.body)
    }

    /// The digest of the product model as installed (the tag is mutable).
    pub fn digest(&self) -> Result<Option<String>, String> {
        let reply = self.get_ok(Endpoint::Tags)?;
        wire::parse_digest(&reply.body, &self.model)
    }

    fn get_ok(&self, endpoint: Endpoint) -> Result<HttpReply, String> {
        let reply = self
            .transport()
            .get(endpoint, PROBE_TIMEOUT)
            .map_err(|e| format!("{e:?}"))?;
        if reply.status == 200 {
            Ok(reply)
        } else {
            Err(format!(
                "HTTP {}: {}",
                reply.status,
                wire::error_message(&reply.body)
            ))
        }
    }

    /// Loads the model with a trivial request (the spike's warm-up, same tool
    /// schemas and settings). Returns how long it took.
    pub fn warm_up(&self) -> Result<Duration, LlmError> {
        let started = Instant::now();
        let body = wire::warm_up_body(&self.model);
        let reply = self
            .transport()
            .post(Endpoint::Chat, &body, WARM_UP_TIMEOUT)
            .map_err(llm_error)?;
        interpret(reply)?;
        Ok(started.elapsed())
    }
}

impl Default for OllamaProvider {
    fn default() -> Self {
        Self::new()
    }
}

fn llm_error(error: TransportError) -> LlmError {
    match error {
        TransportError::Refused => LlmError::Unavailable,
        TransportError::Timeout => LlmError::Timeout,
        TransportError::Dropped => LlmError::Unavailable,
        TransportError::Other(why) => LlmError::Failed(why),
    }
}

/// 200 is a reply; 404 is a model Ollama does not have and 502/503 mean the
/// server (or something in front of it) cannot serve right now: all
/// unavailable. A 500 stays a failure: Ollama uses it for load errors such as
/// "requires more system memory", whose reason is worth keeping. Anything else
/// is a failure with Ollama's own reason.
fn interpret(reply: HttpReply) -> Result<ChatReply, LlmError> {
    match reply.status {
        200 => wire::parse_chat_response(&reply.body),
        404 | 502 | 503 => Err(LlmError::Unavailable),
        status => Err(LlmError::Failed(format!(
            "HTTP {status}: {}",
            wire::error_message(&reply.body)
        ))),
    }
}

impl LlmProvider for OllamaProvider {
    fn chat(&self, request: &ChatRequest) -> Result<ChatReply, LlmError> {
        let body = wire::chat_body(&self.model, request);
        let reply = self
            .transport()
            .post(Endpoint::Chat, &body, CHAT_TIMEOUT)
            .map_err(llm_error)?;
        interpret(reply)
    }
}
