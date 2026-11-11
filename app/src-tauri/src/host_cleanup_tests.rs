//! Pins the host after the click channel left it (living-pc-notch, slice 4):
//! the island window is always click-through with nothing that toggles it, no
//! cursor poller runs, no webview command can answer a confirmation, and the
//! chord is the only physical way to accept a restart or a shutdown in a
//! release build. Source-level checks, like `capabilities_tests.rs`: the
//! wiring they pin lives in `run()`, which needs a real app to execute.

use std::path::{Path, PathBuf};

const LIB_RS: &str = include_str!("lib.rs");
const CLICK_THROUGH_RS: &str = include_str!("click_through.rs");
/// Test files whose strings name the things that must be gone.
const NEGATIVE_TESTS: [&str; 2] = ["host_cleanup_tests.rs", "capabilities_tests.rs"];

fn src_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("src")
}

/// Every `.rs` file under `src/` except the negative tests (see [`NEGATIVE_TESTS`]), as `(relative path, text)`.
fn sources() -> Vec<(String, String)> {
    fn walk(dir: &Path, root: &Path, out: &mut Vec<(String, String)>) {
        for entry in std::fs::read_dir(dir).expect("readable dir") {
            let path = entry.expect("entry").path();
            if path.is_dir() {
                walk(&path, root, out);
            } else if path.extension().is_some_and(|e| e == "rs") {
                let rel = path
                    .strip_prefix(root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/");
                if !NEGATIVE_TESTS.contains(&rel.as_str()) {
                    let text = std::fs::read_to_string(&path).expect("readable");
                    out.push((rel, text));
                }
            }
        }
    }
    let root = src_dir();
    let mut out = Vec::new();
    walk(&root, &root, &mut out);
    out
}

/// The part of a file before its test module.
fn production(text: &str) -> &str {
    text.split("#[cfg(test)]").next().unwrap_or(text)
}

#[test]
fn no_click_command_registered_in_invoke_handler() {
    let handlers: Vec<&str> = LIB_RS
        .match_indices("generate_handler![")
        .map(|(at, _)| &LIB_RS[at..])
        .map(|rest| rest.split(']').next().unwrap())
        .collect();
    assert_eq!(handlers.len(), 2, "one debug and one release handler");
    for handler in handlers {
        assert!(handler.contains("text_window::submit_text"), "{handler}");
        assert!(!handler.contains("confirm"), "{handler}");
        assert!(!handler.contains("report"), "{handler}");
        assert!(!handler.contains("answer"), "{handler}");
    }
}

#[test]
fn no_references_to_confirm_buttons_remain() {
    assert!(
        !src_dir().join("confirm_buttons.rs").exists(),
        "confirm_buttons.rs is deleted"
    );
    for (file, text) in sources() {
        for gone in [
            "confirm_buttons",
            "ConfirmGate",
            "GateUi",
            "answer_confirmation",
            "report_confirm_buttons",
        ] {
            assert!(!text.contains(gone), "{file} still mentions {gone}");
        }
    }
}

#[test]
fn the_ui_chain_is_the_keys_over_the_tauri_adapter() {
    assert!(LIB_RS.contains("KeysUi::new("));
    assert!(LIB_RS.contains("TauriUiAdapter::new("));
}

#[test]
fn no_cursor_poller_started() {
    assert!(
        !src_dir().join("cursor.rs").exists(),
        "cursor.rs is deleted"
    );
    for (file, text) in sources() {
        for gone in [
            "CursorPoller",
            "crate::cursor",
            "mod cursor",
            "CursorPayload",
        ] {
            assert!(!text.contains(gone), "{file} still mentions {gone}");
        }
    }
}

#[test]
fn nothing_toggles_the_islands_click_through_flag_but_always_true() {
    let production = production(CLICK_THROUGH_RS);
    assert!(!production.contains("HitTest"), "the hit tester is gone");
    assert!(!production.contains("Rect"), "no interactive rectangles");
    // The only call that sets the flag passes a literal `true`.
    let calls: Vec<&str> = production
        .lines()
        .filter(|line| line.contains(".set_ignore_cursor_events("))
        .collect();
    assert_eq!(calls.len(), 1, "{calls:?}");
    assert!(calls[0].contains("set_ignore_cursor_events(true)"));
}

/// In a release build the chord is the only physical way to accept: the only
/// production code that builds `Event::Accept` is `confirm_keys.rs`, and the
/// debug window (compiled out of release builds, see `lib.rs`).
#[test]
fn release_has_chord_as_only_power_confirmation_channel() {
    let allowed = ["confirm_keys.rs", "debug.rs"];
    let mut builders = Vec::new();
    for (file, text) in sources() {
        if production(&text).contains("Event::Accept {") {
            builders.push(file);
        }
    }
    builders.sort();
    assert_eq!(builders, allowed, "who can build Accept");
}

#[test]
fn debug_button_confirm_exists_in_debug_builds_only() {
    // The debug window's button is the only other physical answer. Its module
    // is compiled in debug builds alone, so a release build has no such path.
    let debug_mod = LIB_RS
        .lines()
        .position(|line| line.trim() == "mod debug;")
        .expect("mod debug");
    let gate = LIB_RS.lines().nth(debug_mod - 1).unwrap().trim();
    assert_eq!(gate, "#[cfg(debug_assertions)]");
    #[cfg(debug_assertions)]
    assert!(include_str!("debug.rs").contains("DebugEvent::Accept { id } => Event::Accept {"));
}
