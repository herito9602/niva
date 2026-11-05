//! Least-privilege checks on the capability files (config verified by tests,
//! not by running a webview).

use serde_json::Value;

use crate::text_window::TEXT_WINDOW_LABEL;
use crate::ui_adapter::{DEBUG_LABEL, ISLAND_LABEL};

fn load(json: &str) -> Value {
    serde_json::from_str(json).expect("valid capability json")
}

fn permissions(capability: &Value) -> Vec<String> {
    capability["permissions"]
        .as_array()
        .expect("permissions")
        .iter()
        .map(|p| p.as_str().expect("string permission").to_owned())
        .collect()
}

#[test]
fn the_island_capability_can_only_listen_to_events() {
    let island = load(include_str!("../capabilities/default.json"));
    assert_eq!(island["windows"], serde_json::json!([ISLAND_LABEL]));
    let perms = permissions(&island);
    assert_eq!(
        perms,
        vec!["core:event:allow-listen", "core:event:allow-unlisten"]
    );
    for forbidden in [
        "global-shortcut",
        "emit",
        "debug",
        "window",
        "core:default",
        "submit-text",
    ] {
        assert!(
            !perms.iter().any(|p| p.contains(forbidden)),
            "island must not hold {forbidden}"
        );
    }
}

/// The release "Type a command" window may call exactly one command, the one
/// that can only produce `Event::TextSubmitted` (`text_window.rs`). It cannot
/// listen, emit, control windows or reach the debug command, and it is a
/// different window than the island, so the island gains nothing from it.
#[test]
fn the_text_window_capability_can_only_submit_text() {
    let text = load(include_str!("../capabilities/text-window.json"));
    assert_eq!(text["windows"], serde_json::json!([TEXT_WINDOW_LABEL]));
    assert_ne!(TEXT_WINDOW_LABEL, ISLAND_LABEL);
    assert_eq!(permissions(&text), vec!["allow-submit-text"]);
    assert!(
        text.get("remote").is_none() && text.get("webviews").is_none(),
        "local content of the one window only"
    );
}

/// `submit_text` is an app command, so it is gated by an app permission. The
/// release build defines that permission from a checked-in file (not from
/// `commands()`, whose generated directory can hold a stale debug permission).
#[test]
fn the_release_permission_file_defines_only_the_text_command() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("permissions-release");
    let mut files = Vec::new();
    for entry in std::fs::read_dir(dir).expect("permissions-release dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "toml") {
            files.push(std::fs::read_to_string(&path).expect("readable"));
        }
    }
    assert_eq!(files.len(), 1, "one permission file in the release set");
    let file = &files[0];
    assert!(file.contains("identifier = \"allow-submit-text\""));
    assert!(file.contains("commands.allow = [\"submit_text\"]"));
    assert_eq!(file.matches("[[permission]]").count(), 1);
    assert!(
        !file.contains("debug"),
        "no debug permission in the release set"
    );
}

#[test]
fn the_debug_capability_is_scoped_to_the_debug_window_and_has_no_emit() {
    let debug = load(include_str!("../capabilities-debug/debug.json"));
    assert_eq!(debug["windows"], serde_json::json!([DEBUG_LABEL]));
    let perms = permissions(&debug);
    assert!(perms.iter().any(|p| p == "allow-debug-dispatch"));
    assert!(perms.iter().any(|p| p == "core:event:allow-listen"));
    assert!(perms.iter().any(|p| p == "core:event:allow-unlisten"));
    assert!(!perms.iter().any(|p| p.contains("emit")));
    assert!(!perms.iter().any(|p| p == "core:default"));
}

/// Release builds load only `capabilities/`; nothing there may mention the
/// debug command or window, or it would ship in the release ACLs.
#[test]
fn the_release_capability_directory_holds_no_debug_permission() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    let mut seen = 0;
    for entry in std::fs::read_dir(dir).expect("capabilities dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "json") {
            seen += 1;
            let text = std::fs::read_to_string(&path).expect("readable");
            assert!(
                !text.contains("debug-dispatch") && !text.contains("debug_dispatch"),
                "{path:?} references the debug command"
            );
        }
    }
    assert_eq!(
        seen, 2,
        "the island and the text window capabilities are the release set"
    );
}

/// The build script must key the debug-only pieces on the same switch as
/// `mod debug` and gate the debug capability directory and command.
#[test]
fn the_build_script_gates_the_debug_command_and_capabilities_on_debug_assertions() {
    let build = include_str!("../build.rs");
    assert!(build.contains("CARGO_CFG_DEBUG_ASSERTIONS"));
    // Lines of the `else` (release) branch: after `} else {` up to the `    }`.
    let release_branch: Vec<&str> = build
        .lines()
        .skip_while(|line| !line.contains("} else {"))
        .skip(1)
        .take_while(|line| line.trim_end() != "    }")
        .collect();
    let release_branch = release_branch.join(" ");
    assert!(release_branch.contains("capabilities_path_pattern"));
    assert!(!release_branch.contains("debug_dispatch"));
    assert!(!release_branch.contains("capabilities-debug"));
    assert!(!release_branch.contains("capabilities*"));
    assert!(
        release_branch.contains("permissions_path_pattern"),
        "release must not glob the stale permissions/autogenerated files"
    );
    // `commands()` would make tauri-build glob permissions/autogenerated, where
    // a debug build leaves allow-debug-dispatch.
    assert!(
        !release_branch.contains(".commands("),
        "release must define its permissions from permissions-release only"
    );
    assert!(release_branch.contains("permissions-release/"));
}
