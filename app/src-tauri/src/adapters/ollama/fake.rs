//! A scripted HTTP transport for the Ollama adapter's tests: no network.

use std::sync::Mutex;
use std::time::Duration;

use super::{Endpoint, HttpReply, Transport, TransportError};

pub type Scripted = Result<HttpReply, TransportError>;

/// One call the adapter made.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Seen {
    pub endpoint: Endpoint,
    pub body: Option<String>,
    pub timeout: Duration,
}

#[derive(Default)]
pub struct FakeTransport {
    script: Mutex<Vec<(Endpoint, Scripted)>>,
    seen: Mutex<Vec<Seen>>,
}

pub fn ok(body: &str) -> Scripted {
    Ok(HttpReply {
        status: 200,
        body: body.into(),
    })
}

pub fn status(code: u16, body: &str) -> Scripted {
    Ok(HttpReply {
        status: code,
        body: body.into(),
    })
}

/// `/api/tags` listing these models.
pub fn tags(models: &[&str]) -> Scripted {
    let models: Vec<_> = models
        .iter()
        .map(|name| serde_json::json!({"name": name, "model": name}))
        .collect();
    ok(&serde_json::json!({ "models": models }).to_string())
}

pub fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex
        .lock()
        .unwrap_or_else(|poisoned| poisoned.into_inner())
}

impl FakeTransport {
    /// Queue an answer for `endpoint`; answers for one endpoint come back in order.
    pub fn push(&self, endpoint: Endpoint, answer: Scripted) -> &Self {
        lock(&self.script).push((endpoint, answer));
        self
    }

    pub fn seen(&self) -> Vec<Seen> {
        lock(&self.seen).clone()
    }

    fn answer(&self, endpoint: Endpoint, body: Option<&str>, timeout: Duration) -> Scripted {
        lock(&self.seen).push(Seen {
            endpoint,
            body: body.map(str::to_string),
            timeout,
        });
        let mut script = lock(&self.script);
        match script.iter().position(|(e, _)| *e == endpoint) {
            Some(at) => script.remove(at).1,
            None => Err(TransportError::Other("script exhausted".into())),
        }
    }
}

impl Transport for FakeTransport {
    fn get(&self, endpoint: Endpoint, timeout: Duration) -> Scripted {
        self.answer(endpoint, None, timeout)
    }

    fn post(&self, endpoint: Endpoint, body: &str, timeout: Duration) -> Scripted {
        self.answer(endpoint, Some(body), timeout)
    }
}
