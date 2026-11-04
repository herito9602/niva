//! The Ollama provider over a fake transport (no network). The tests that
//! need a real Ollama are `#[ignore]`d at the bottom.

use std::sync::Arc;

use niva_core::ports::{ChatRequest, LlmError, LlmProvider};
use niva_core::tools::ArgValue;

use super::fake::{FakeTransport, Scripted, ok, status};
use super::*;

fn provider(answers: Vec<Scripted>) -> (OllamaProvider, Arc<FakeTransport>) {
    let transport = Arc::new(FakeTransport::default());
    for answer in answers {
        transport.push(Endpoint::Chat, answer);
    }
    (OllamaProvider::with_transport(transport.clone()), transport)
}

fn request(allow_tools: bool) -> ChatRequest {
    ChatRequest {
        system: "You are Niva.".into(),
        user: "set the volume to 30".into(),
        allow_tools,
    }
}

const VOLUME_CALL: &str = r#"{"message":{"role":"assistant","content":"","tool_calls":[{"function":{"name":"set_volume","arguments":{"level":30.0}}}]},"done":true}"#;

#[test]
fn a_chat_call_posts_to_chat_and_parses_the_tool_calls() {
    let (llm, transport) = provider(vec![ok(VOLUME_CALL)]);
    let reply = llm.chat(&request(true)).expect("a reply");
    assert_eq!(reply.calls.len(), 1);
    assert_eq!(reply.calls[0].name, "set_volume");
    assert_eq!(reply.calls[0].args["level"], ArgValue::Int(30));
    let seen = transport.seen();
    assert_eq!(seen.len(), 1);
    assert_eq!(seen[0].endpoint, Endpoint::Chat);
    assert_eq!(seen[0].timeout, CHAT_TIMEOUT);
    let body: serde_json::Value = serde_json::from_str(seen[0].body.as_ref().unwrap()).unwrap();
    assert_eq!(body["model"], MODEL);
    assert_eq!(body["think"], false);
    assert!(body["tools"].is_array());
}

#[test]
fn the_summary_call_offers_no_tools() {
    let (llm, transport) = provider(vec![ok(r#"{"message":{"content":"A summary."}}"#)]);
    let reply = llm.chat(&request(false)).unwrap();
    assert_eq!(reply.text, "A summary.");
    let body: serde_json::Value =
        serde_json::from_str(transport.seen()[0].body.as_ref().unwrap()).unwrap();
    assert!(body.get("tools").is_none());
}

#[test]
fn a_refused_connection_means_ollama_is_not_running() {
    let (llm, _) = provider(vec![Err(TransportError::Refused)]);
    assert_eq!(llm.chat(&request(true)), Err(LlmError::Unavailable));
}

#[test]
fn a_timeout_is_a_timeout() {
    let (llm, _) = provider(vec![Err(TransportError::Timeout)]);
    assert_eq!(llm.chat(&request(true)), Err(LlmError::Timeout));
}

#[test]
fn an_unknown_model_is_unavailable_not_a_crash() {
    let (llm, _) = provider(vec![status(404, r#"{"error":"model 'x' not found"}"#)]);
    assert_eq!(llm.chat(&request(true)), Err(LlmError::Unavailable));
}

#[test]
fn a_server_error_is_a_failure_with_the_reason() {
    let (llm, _) = provider(vec![status(
        500,
        r#"{"error":"model requires more system memory"}"#,
    )]);
    match llm.chat(&request(true)) {
        Err(LlmError::Failed(why)) => {
            assert!(why.contains("500"), "{why}");
            assert!(why.contains("more system memory"), "{why}");
        }
        other => panic!("{other:?}"),
    }
}

#[test]
fn a_transport_error_is_a_failure() {
    let (llm, _) = provider(vec![Err(TransportError::Other("reset".into()))]);
    assert!(matches!(llm.chat(&request(true)), Err(LlmError::Failed(_))));
}

#[test]
fn a_connection_dropped_mid_request_means_ollama_is_unavailable() {
    let (llm, _) = provider(vec![Err(TransportError::Dropped)]);
    assert_eq!(llm.chat(&request(true)), Err(LlmError::Unavailable));
}

#[test]
fn a_bad_gateway_or_unavailable_status_is_unavailable() {
    for code in [502, 503] {
        let (llm, _) = provider(vec![status(code, "")]);
        assert_eq!(
            llm.chat(&request(true)),
            Err(LlmError::Unavailable),
            "{code}"
        );
    }
}

#[test]
fn a_500_stays_a_failure_because_ollama_reports_load_errors_with_it() {
    let (llm, _) = provider(vec![status(
        500,
        r#"{"error":"llama runner process has terminated"}"#,
    )]);
    assert!(matches!(llm.chat(&request(true)), Err(LlmError::Failed(_))));
}

#[test]
fn ureq_io_errors_that_mean_a_dead_connection_map_to_dropped() {
    use std::io::{Error, ErrorKind};
    for kind in [
        ErrorKind::ConnectionReset,
        ErrorKind::ConnectionAborted,
        ErrorKind::BrokenPipe,
        ErrorKind::UnexpectedEof,
    ] {
        let mapped = map_error(ureq::Error::Io(Error::from(kind)));
        assert_eq!(mapped, TransportError::Dropped, "{kind:?}");
    }
    let other = map_error(ureq::Error::Io(Error::from(ErrorKind::PermissionDenied)));
    assert!(matches!(other, TransportError::Other(_)));
}

#[test]
fn the_chat_timeout_outlasts_the_reducer_llm_timer() {
    use niva_core::state_machine::TimerKind;
    let llm_timer = TimerKind::Llm
        .duration()
        .expect("the Llm timer has a duration");
    assert!(CHAT_TIMEOUT > llm_timer);
}

#[test]
fn a_garbled_body_is_a_failure_not_a_panic() {
    let (llm, _) = provider(vec![ok("<html>proxy error</html>")]);
    assert!(matches!(llm.chat(&request(true)), Err(LlmError::Failed(_))));
}

#[test]
fn the_model_can_be_chosen_for_comparisons() {
    let transport = Arc::new(FakeTransport::default());
    transport.push(Endpoint::Chat, ok(VOLUME_CALL));
    let llm = OllamaProvider::with_transport(transport.clone()).with_model("llama3.2:3b");
    llm.chat(&request(true)).unwrap();
    assert_eq!(llm.model(), "llama3.2:3b");
    let body: serde_json::Value =
        serde_json::from_str(transport.seen()[0].body.as_ref().unwrap()).unwrap();
    assert_eq!(body["model"], "llama3.2:3b");
}

#[test]
fn the_warm_up_posts_the_tiny_request_with_the_long_timeout() {
    let (llm, transport) = provider(vec![ok(r#"{"message":{"content":"OK"}}"#)]);
    llm.warm_up().expect("warm");
    let seen = transport.seen();
    assert_eq!(seen[0].endpoint, Endpoint::Chat);
    assert_eq!(seen[0].timeout, WARM_UP_TIMEOUT);
    assert!(seen[0].body.as_ref().unwrap().contains("Reply with OK."));
}

#[test]
fn a_failed_warm_up_reports_the_error() {
    let (llm, _) = provider(vec![Err(TransportError::Refused)]);
    assert_eq!(llm.warm_up(), Err(LlmError::Unavailable));
}

// --- transport and endpoints ---------------------------------------------------

#[test]
fn every_endpoint_is_a_fixed_path_on_loopback() {
    for (endpoint, path) in [
        (Endpoint::Tags, "/api/tags"),
        (Endpoint::Ps, "/api/ps"),
        (Endpoint::Chat, "/api/chat"),
        (Endpoint::Version, "/api/version"),
    ] {
        assert_eq!(endpoint.path(), path);
        let url = endpoint.url();
        assert_eq!(url, format!("http://127.0.0.1:11434{path}"));
    }
}

#[test]
fn provenance_comes_from_version_and_tags() {
    let transport = Arc::new(FakeTransport::default());
    transport
        .push(Endpoint::Version, ok(r#"{"version":"0.40.14"}"#))
        .push(
            Endpoint::Tags,
            ok(r#"{"models":[{"name":"qwen3.5:4b-q4_K_M","digest":"d8b0f5e9760c"}]}"#),
        );
    let llm = OllamaProvider::with_transport(transport);
    assert_eq!(llm.version().unwrap(), "0.40.14");
    assert_eq!(llm.digest().unwrap().as_deref(), Some("d8b0f5e9760c"));
}

#[test]
fn provenance_errors_are_reported_not_hidden() {
    let transport = Arc::new(FakeTransport::default());
    transport
        .push(Endpoint::Version, Err(TransportError::Refused))
        .push(Endpoint::Tags, status(500, "x"));
    let llm = OllamaProvider::with_transport(transport);
    assert!(llm.version().is_err());
    assert!(llm.digest().is_err());
}

#[test]
fn the_address_is_loopback_only() {
    assert!(ADDRESS.starts_with("127.0.0.1:"));
}

// --- redirects ------------------------------------------------------------------

#[test]
fn a_redirect_is_a_failure_for_chat_and_warm_up() {
    for code in [301, 302, 307, 308] {
        let (llm, _) = provider(vec![status(code, "")]);
        match llm.chat(&request(true)) {
            Err(LlmError::Failed(why)) => assert!(why.contains(&code.to_string()), "{why}"),
            other => panic!("{code}: {other:?}"),
        }
        let (llm, _) = provider(vec![status(code, "")]);
        assert!(matches!(llm.warm_up(), Err(LlmError::Failed(_))));
    }
}

#[test]
fn a_redirect_is_an_error_for_the_provenance_reads() {
    let transport = Arc::new(FakeTransport::default());
    transport
        .push(Endpoint::Version, status(302, ""))
        .push(Endpoint::Tags, status(302, ""));
    let llm = OllamaProvider::with_transport(transport);
    assert!(llm.version().unwrap_err().contains("302"));
    assert!(llm.digest().unwrap_err().contains("302"));
}

/// A one-shot local server that answers any request with a redirect to
/// `target`; returns its URL.
fn redirecting_server(target: String) -> String {
    use std::io::{Read, Write};
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}/api/chat", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        if let Ok((mut stream, _)) = listener.accept() {
            let mut buf = [0u8; 4096];
            let _ = stream.read(&mut buf);
            let reply = format!(
                "HTTP/1.1 302 Found\r\nLocation: {target}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            );
            let _ = stream.write_all(reply.as_bytes());
        }
    });
    url
}

/// Whatever listens on the port must not be able to send Niva elsewhere: the
/// real transport never follows a redirect, for GET or for POST.
#[test]
fn the_real_transport_never_follows_a_redirect() {
    let elsewhere = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    elsewhere.set_nonblocking(true).unwrap();
    let target = format!("http://{}/stolen", elsewhere.local_addr().unwrap());
    let transport = UreqTransport::new();
    let timeout = Duration::from_secs(5);

    let url = redirecting_server(target.clone());
    let got = transport.get_url(&url, timeout);
    assert!(!matches!(&got, Ok(reply) if reply.status == 200), "{got:?}");
    assert!(elsewhere.accept().is_err(), "GET followed the redirect");

    let url = redirecting_server(target);
    let got = transport.post_url(&url, "{}", timeout);
    assert!(!matches!(&got, Ok(reply) if reply.status == 200), "{got:?}");
    assert!(elsewhere.accept().is_err(), "POST followed the redirect");
}

// --- integration (real Ollama, run by hand) -------------------------------------

#[test]
#[ignore = "needs Ollama running with the product model"]
fn real_ollama_picks_set_volume_for_a_volume_command() {
    let llm = OllamaProvider::new();
    assert_eq!(llm.health(), health::Health::Ready);
    llm.warm_up().expect("warm-up");
    let system = niva_core::prompt::system_prompt(niva_core::replies::Lang::En, None);
    let reply = llm
        .chat(&ChatRequest {
            system,
            user: "Set the volume to 30 percent".into(),
            allow_tools: true,
        })
        .expect("a reply");
    assert_eq!(reply.calls.len(), 1, "{reply:?}");
    assert_eq!(reply.calls[0].name, "set_volume");
    assert_eq!(reply.calls[0].args["level"], ArgValue::Int(30));
}

#[test]
#[ignore = "needs Ollama running"]
fn real_ollama_reports_where_the_model_is_loaded() {
    let llm = OllamaProvider::new();
    llm.warm_up().expect("warm-up");
    let offload = llm.offload().expect("/api/ps");
    println!("offload: {offload:?}");
    assert_ne!(offload, health::Offload::NotLoaded);
}
