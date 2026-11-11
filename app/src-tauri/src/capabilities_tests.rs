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

/// The island webview only listens: it calls no command at all, so it holds no
/// app permission (neither `allow-*` nor a window, emit or shortcut one). The
/// click channel that needed two commands is gone.
#[test]
fn island_capability_grants_no_commands() {
    let capabilities = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("capabilities");
    for entry in std::fs::read_dir(capabilities).expect("capabilities dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "json") {
            let capability = load(&std::fs::read_to_string(&path).expect("readable"));
            let on_island = capability["windows"]
                .as_array()
                .is_some_and(|w| w.iter().any(|l| l == ISLAND_LABEL));
            if on_island {
                assert!(
                    permissions(&capability)
                        .iter()
                        .all(|p| p.starts_with("core:event:")),
                    "{path:?} gives the island more than event listening"
                );
            }
        }
    }
}

#[test]
fn no_island_actions_capability_or_permission_files() {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"));
    assert!(!root.join("capabilities/island-actions.json").exists());
    assert!(
        !root
            .join("permissions-release/island-actions.toml")
            .exists()
    );
}

/// Nobody but the island may answer a confirmation: the text window keeps its
/// one command and the debug window has no island command either.
#[test]
fn no_other_window_can_answer_a_confirmation() {
    let text = load(include_str!("../capabilities/text-window.json"));
    let debug = load(include_str!("../capabilities-debug/debug.json"));
    for capability in [text, debug] {
        assert!(
            !permissions(&capability)
                .iter()
                .any(|p| p.contains("answer-confirmation") || p.contains("confirm-buttons")),
            "{capability}"
        );
    }
}

/// App commands are gated by app permissions. The release build defines them
/// from checked-in files (not from `commands()`, whose generated directory can
/// hold a stale debug permission). One file per window, so each window's
/// permissions can be read at a glance.
#[test]
fn the_release_permission_files_define_only_the_window_commands() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("permissions-release");
    let mut files = std::collections::BTreeMap::new();
    for entry in std::fs::read_dir(dir).expect("permissions-release dir") {
        let path = entry.expect("entry").path();
        if path.extension().is_some_and(|e| e == "toml") {
            let name = path.file_name().unwrap().to_string_lossy().into_owned();
            files.insert(name, std::fs::read_to_string(&path).expect("readable"));
        }
    }
    assert_eq!(
        files.keys().map(String::as_str).collect::<Vec<_>>(),
        vec!["submit-text.toml"]
    );

    let text = &files["submit-text.toml"];
    assert!(text.contains("identifier = \"allow-submit-text\""));
    assert!(text.contains("commands.allow = [\"submit_text\"]"));
    assert_eq!(text.matches("[[permission]]").count(), 1);

    for (name, file) in &files {
        assert!(
            !file.contains("debug"),
            "{name}: no debug permission in the release set"
        );
    }
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
    // Debug builds list every command so tauri-build generates its permission.
    for command in ["debug_dispatch", "submit_text"] {
        assert!(build.contains(command), "{command} missing from build.rs");
    }
    for gone in ["answer_confirmation", "report_confirm_buttons"] {
        assert!(!build.contains(gone), "{gone} is still in build.rs");
    }
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
