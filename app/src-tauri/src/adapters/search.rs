//! Tavily web search: the product [`SearchProvider`] (design D5, spike S3).
//!
//! One fixed endpoint ([`ENDPOINT`], HTTPS only, no redirects, no proxy), one
//! JSON body, one bearer key that belongs to the user. Nothing from the network
//! is trusted: the snippets go back to the assistant, which quotes them as data
//! in the no-tools summary call (`assistant::untrusted`) and never lets them
//! reach a tool.
//!
//! What leaves the machine is the search query and the key, to
//! `api.tavily.com`. The query goes only in the JSON body, never in a URL. The
//! key goes only in the `Authorization` header; it has no `Debug`/`Display`
//! that prints it and no error, log line or reply ever carries it, or any text
//! from a response.
//!
//! - [`ApiKey`]: the key, read once at start from `NIVA_TAVILY_KEY` (and, on
//!   Windows, from `HKCU\Environment` when the process did not inherit it:
//!   `setx` only reaches processes started afterwards).
//! - [`SearchTransport`]: the blocking HTTP call, a trait so the provider is
//!   tested with a fake and no test touches the network.
//! - [`TavilySearch`]: query limits, request body, response parsing and the
//!   mapping of status codes to [`SearchError`].

use std::fmt;
use std::sync::Arc;
use std::time::Duration;

use niva_core::ports::{SearchError, SearchProvider};

#[cfg(test)]
mod tests;

/// The only URL Niva sends a search to.
pub const ENDPOINT: &str = "https://api.tavily.com/search";
/// Environment variable (and `HKCU\Environment` value) holding the user's key.
pub const KEY_VAR: &str = "NIVA_TAVILY_KEY";
/// Hard limit of one search call. Measured latency is 1.2 to 2.3 s; the
/// reducer's tool timer (`TimerKind::Tool`, 5 s; a test pins the order) is the
/// ceiling, so this fires first and leaves a second for the worker to hand the
/// failure back. Other safe calls of the same response share that timer.
pub const SEARCH_TIMEOUT: Duration = Duration::from_secs(4);
/// Longest query sent, in characters. The query is model output.
pub const MAX_QUERY_CHARS: usize = 400;
/// Most snippets handed to the assistant.
pub const MAX_RESULTS: usize = 4;
/// Longest snippet, in characters (the assistant caps again when quoting).
pub const MAX_SNIPPET_CHARS: usize = 500;
/// Largest response body read, in bytes.
pub const MAX_BODY_BYTES: u64 = 256 * 1024;

/// The user's Tavily key. It never prints: `Debug` is redacted, there is no
/// `Display` and no `Clone`, and only [`ApiKey::expose`] (used by the
/// transport for the header) gives the text.
pub struct ApiKey(String);

impl ApiKey {
    /// `None` for an empty or blank value.
    pub fn new(raw: &str) -> Option<ApiKey> {
        let trimmed = raw.trim();
        (!trimmed.is_empty()).then(|| ApiKey(trimmed.to_string()))
    }

    /// The key text, for the `Authorization` header and nothing else.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for ApiKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ApiKey(<redacted>)")
    }
}

/// The key from the process environment first, then from the user's stored
/// environment. A blank value counts as missing. The fallback is only
/// consulted when the first source gave nothing.
pub fn resolve_key(
    process_env: Option<String>,
    stored_env: impl FnOnce() -> Option<String>,
) -> Option<ApiKey> {
    process_env
        .as_deref()
        .and_then(ApiKey::new)
        .or_else(|| stored_env().as_deref().and_then(ApiKey::new))
}

/// A reply with any HTTP status; the provider decides what it means.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Reply {
    pub status: u16,
    pub body: String,
}

/// Why no reply arrived. No text on purpose: nothing a library says about a
/// failed call may reach a reply or a log.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NetError {
    Timeout,
    /// Could not connect, TLS failed, or the connection dropped.
    Network,
    /// The body is larger than [`MAX_BODY_BYTES`].
    TooLarge,
}

/// A blocking HTTPS POST to [`ENDPOINT`] with a hard timeout.
pub trait SearchTransport: Send + Sync {
    fn post(&self, key: &ApiKey, body: &str, timeout: Duration) -> Result<Reply, NetError>;
}

/// The real transport: HTTPS only (through the operating system's TLS), no
/// redirects, no proxy (the ask for Ollama's transport: nothing may add a hop
/// the code did not choose), response read up to [`MAX_BODY_BYTES`].
pub struct UreqSearchTransport {
    agent: ureq::Agent,
}

impl UreqSearchTransport {
    pub fn new() -> Self {
        Self::build(true)
    }

    fn build(https_only: bool) -> Self {
        // The roots must be named: ureq's default (`WebPki`) panics with
        // `native-tls` when its webpki roots feature is off, which it is. The
        // platform roots are Windows' own trust store; certificate and host
        // checks stay on.
        let tls = ureq::tls::TlsConfig::builder()
            .provider(ureq::tls::TlsProvider::NativeTls)
            .root_certs(ureq::tls::RootCerts::PlatformVerifier)
            .build();
        let config = ureq::Agent::config_builder()
            .proxy(None)
            .http_status_as_error(false)
            // A 3xx comes back as a reply and counts as a failure: the key and
            // the query must not follow a redirect to another host.
            .max_redirects(0)
            .https_only(https_only)
            .tls_config(tls)
            .build();
        UreqSearchTransport {
            agent: ureq::Agent::new_with_config(config),
        }
    }

    /// Tests only: plain HTTP, to a local listener.
    #[cfg(test)]
    fn plain_http_for_tests() -> Self {
        Self::build(false)
    }

    fn post_url(
        &self,
        url: &str,
        key: &ApiKey,
        body: &str,
        timeout: Duration,
    ) -> Result<Reply, NetError> {
        let mut response = self
            .agent
            .post(url)
            .header("Authorization", format!("Bearer {}", key.expose()))
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .config()
            .timeout_global(Some(timeout))
            .build()
            .send(body)
            .map_err(|e| net_error(&e))?;
        let status = response.status().as_u16();
        let body = response
            .body_mut()
            .with_config()
            .limit(MAX_BODY_BYTES)
            .read_to_string()
            .map_err(|e| net_error(&e))?;
        Ok(Reply { status, body })
    }
}

impl Default for UreqSearchTransport {
    fn default() -> Self {
        Self::new()
    }
}

fn net_error(error: &ureq::Error) -> NetError {
    match error {
        ureq::Error::Timeout(_) => NetError::Timeout,
        ureq::Error::BodyExceedsLimit(_) => NetError::TooLarge,
        ureq::Error::Io(io)
            if matches!(
                io.kind(),
                std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
            ) =>
        {
            NetError::Timeout
        }
        _ => NetError::Network,
    }
}

impl SearchTransport for UreqSearchTransport {
    fn post(&self, key: &ApiKey, body: &str, timeout: Duration) -> Result<Reply, NetError> {
        self.post_url(ENDPOINT, key, body, timeout)
    }
}

/// [`SearchProvider`] over Tavily. Without a key it makes no call at all and
/// answers [`SearchError::NotConfigured`].
pub struct TavilySearch {
    transport: Arc<dyn SearchTransport>,
    key: Option<ApiKey>,
}

impl TavilySearch {
    pub fn new(transport: Arc<dyn SearchTransport>, key: Option<ApiKey>) -> Self {
        TavilySearch { transport, key }
    }

    /// The real transport and the key from the environment, read once now.
    pub fn from_environment() -> Self {
        let key = resolve_key(std::env::var(KEY_VAR).ok(), || stored_value(KEY_VAR));
        Self::new(Arc::new(UreqSearchTransport::new()), key)
    }

    pub fn is_configured(&self) -> bool {
        self.key.is_some()
    }
}

impl fmt::Debug for TavilySearch {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("TavilySearch")
            .field("configured", &self.is_configured())
            .finish()
    }
}

/// The JSON body of one search. The query is only ever a JSON string here.
fn request_body(query: &str) -> String {
    let query: String = query.trim().chars().take(MAX_QUERY_CHARS).collect();
    serde_json::json!({
        "query": query,
        "max_results": MAX_RESULTS,
        "search_depth": "basic",
    })
    .to_string()
}

fn cut(text: &str) -> String {
    text.trim().chars().take(MAX_SNIPPET_CHARS).collect()
}

/// `results[].title/content` as snippets: at most [`MAX_RESULTS`], each cut to
/// [`MAX_SNIPPET_CHARS`], empty ones dropped. `None` when the body is not the
/// expected JSON.
fn parse_snippets(body: &str) -> Option<Vec<String>> {
    let value: serde_json::Value = serde_json::from_str(body).ok()?;
    let results = value.get("results")?.as_array()?;
    let text = |item: &serde_json::Value, field: &str| -> String {
        item.get(field)
            .and_then(serde_json::Value::as_str)
            .unwrap_or("")
            .trim()
            .to_string()
    };
    let snippets = results
        .iter()
        .filter_map(|item| {
            let (title, content) = (text(item, "title"), text(item, "content"));
            let joined = match (title.is_empty(), content.is_empty()) {
                (true, true) => return None,
                (false, true) => title,
                (true, false) => content,
                (false, false) => format!("{title}: {content}"),
            };
            Some(cut(&joined))
        })
        .take(MAX_RESULTS)
        .collect();
    Some(snippets)
}

/// What a status means. 429 is a rate limit; 432 and 433 are Tavily's plan
/// and pay-as-you-go limits; all say "used up". No body text is kept.
fn interpret(reply: Reply) -> Result<Vec<String>, SearchError> {
    match reply.status {
        200 => parse_snippets(&reply.body).ok_or(SearchError::Unavailable),
        401 | 403 => Err(SearchError::KeyRejected),
        429 | 432 | 433 => Err(SearchError::QuotaExceeded),
        _ => Err(SearchError::Unavailable),
    }
}

impl SearchProvider for TavilySearch {
    fn search(&self, query: &str) -> Result<Vec<String>, SearchError> {
        let Some(key) = &self.key else {
            return Err(SearchError::NotConfigured);
        };
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let body = request_body(query);
        // The pool swallows a panicking job, which would leave the user with no
        // reply until the tool timer fires: a panic in the call is "cannot
        // search now" instead.
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            self.transport.post(key, &body, SEARCH_TIMEOUT)
        }));
        match outcome {
            Ok(Ok(reply)) => interpret(reply),
            Ok(Err(NetError::Timeout | NetError::Network | NetError::TooLarge)) | Err(_) => {
                Err(SearchError::Unavailable)
            }
        }
    }
}

/// The value `name` has in the user's stored environment (`HKCU\Environment`),
/// for a process that started before `setx` ran. `None` when it is not there.
#[cfg(windows)]
pub fn stored_value(name: &str) -> Option<String> {
    registry_string("Environment", name)
}

/// A `REG_SZ` or `REG_EXPAND_SZ` value (expanded) of `HKCU\<subkey>`.
#[cfg(windows)]
fn registry_string(subkey: &str, name: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{
        HKEY_CURRENT_USER, RRF_RT_REG_EXPAND_SZ, RRF_RT_REG_SZ, RegGetValueW,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(std::iter::once(0)).collect()
    }
    let subkey = wide(subkey);
    let value = wide(name);
    let flags = RRF_RT_REG_SZ | RRF_RT_REG_EXPAND_SZ;
    let mut bytes: u32 = 0;
    // SAFETY: both names are NUL-terminated and outlive the call; a null data
    // pointer asks only for the size.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            flags,
            std::ptr::null_mut(),
            std::ptr::null_mut(),
            &mut bytes,
        )
    };
    if status != 0 || !(2..=8192).contains(&bytes) {
        return None;
    }
    let mut buffer = vec![0u16; bytes as usize / 2 + 1];
    let mut size = (buffer.len() * 2) as u32;
    // SAFETY: `buffer` holds `size` bytes and lives past the call.
    let status = unsafe {
        RegGetValueW(
            HKEY_CURRENT_USER,
            subkey.as_ptr(),
            value.as_ptr(),
            flags,
            std::ptr::null_mut(),
            buffer.as_mut_ptr().cast(),
            &mut size,
        )
    };
    if status != 0 {
        return None;
    }
    let end = buffer
        .iter()
        .position(|unit| *unit == 0)
        .unwrap_or(buffer.len());
    String::from_utf16(&buffer[..end]).ok()
}

#[cfg(not(windows))]
pub fn stored_value(_name: &str) -> Option<String> {
    None
}
