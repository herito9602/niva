//! The island's size and its exit animation are decided in the frontend
//! (`src/island-ui/layout.ts`, `choreography.ts`); the host owns the window
//! size and the hide delay. These tests read those TS constants and the window
//! config so the two sides cannot drift apart silently.

use serde_json::Value;

use crate::confirm_buttons::{ARM_MS, MAX_AREA_HEIGHT, MAX_AREA_WIDTH, MAX_AREAS};
use crate::visibility::HIDE_DELAY_MS;

const LAYOUT_TS: &str = include_str!("../../src/island-ui/layout.ts");
const CHOREOGRAPHY_TS: &str = include_str!("../../src/island-ui/choreography.ts");
const CONFIRM_TS: &str = include_str!("../../src/island-ui/confirmCommands.ts");
const TAURI_CONF: &str = include_str!("../tauri.conf.json");

/// Value of `export const NAME = <number>;` in a TS source.
fn ts_const(source: &str, name: &str) -> f64 {
    let prefix = format!("export const {name} = ");
    let line = source
        .lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("`export const {name} = <number>;` not found"));
    line[prefix.len()..]
        .trim_end_matches(';')
        .trim()
        .parse()
        .unwrap_or_else(|_| panic!("{name} is not a plain numeric literal"))
}

fn island_window() -> Value {
    let conf: Value = serde_json::from_str(TAURI_CONF).expect("valid tauri.conf.json");
    let windows = conf["app"]["windows"].as_array().expect("windows");
    assert_eq!(
        windows.len(),
        1,
        "only the island is declared in the config"
    );
    windows[0].clone()
}

/// How long the exit animation lasts, as declared in `choreography.ts`.
fn out_ms() -> f64 {
    ts_const(CHOREOGRAPHY_TS, "EDGE_OUT_MS")
}

#[test]
fn the_ts_constants_are_readable() {
    assert_eq!(ts_const(LAYOUT_TS, "MAX_ISLAND_WIDTH"), 560.0);
    assert!(out_ms() > 0.0);
}

#[test]
fn the_window_fits_the_largest_island_with_overshoot_and_shadow() {
    let window = island_window();
    let width = window["width"].as_f64().expect("width");
    let height = window["height"].as_f64().expect("height");

    let need_w =
        ts_const(LAYOUT_TS, "MAX_ISLAND_WIDTH") + 2.0 * ts_const(LAYOUT_TS, "SHADOW_ROOM_X");
    // The island hangs from the very top of the window: no gap above it.
    let need_h = ts_const(LAYOUT_TS, "MAX_ISLAND_HEIGHT")
        + ts_const(LAYOUT_TS, "SPRING_OVERSHOOT")
        + ts_const(LAYOUT_TS, "SHADOW_ROOM_Y");
    assert!(
        width >= need_w,
        "window {width} wide, island needs {need_w}"
    );
    assert!(
        height >= need_h,
        "window {height} tall, island needs {need_h}"
    );
}

#[test]
fn the_window_is_not_wastefully_large() {
    // A big transparent window is click-through, but it still covers the top
    // of the screen for the compositor: keep it close to the budget.
    let window = island_window();
    assert!(window["width"].as_f64().unwrap() <= ts_const(LAYOUT_TS, "MAX_ISLAND_WIDTH") + 160.0);
    assert!(window["height"].as_f64().unwrap() <= 240.0);
}

#[test]
fn the_window_never_takes_focus_and_stays_transparent() {
    let window = island_window();
    assert_eq!(window["transparent"], true);
    assert_eq!(window["focusable"], false);
    assert_eq!(window["decorations"], false);
}

#[test]
fn the_host_hides_the_window_only_after_the_exit_animation() {
    let delay = HIDE_DELAY_MS as f64;
    let out = out_ms();
    assert!(delay >= out, "hide delay {delay} cuts the {out} ms exit");
    assert!(
        delay <= out + 250.0,
        "hide delay {delay} lags the {out} ms exit"
    );
}

/// Value of `export const NAME = "<text>";` in a TS source.
fn ts_str(source: &str, name: &str) -> String {
    let prefix = format!("export const {name} = \"");
    let line = source
        .lines()
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("`export const {name} = \"...\";` not found"));
    line[prefix.len()..]
        .trim_end_matches(';')
        .trim_end_matches('"')
        .to_owned()
}

/// A confirmation with the buttons, at six lines of text, is the tallest island
/// there is: its budget is in `MAX_ISLAND_HEIGHT`, which the window test above
/// then checks against `tauri.conf.json`.
#[test]
fn the_height_budget_has_room_for_six_lines_and_the_buttons() {
    let text = ts_const(LAYOUT_TS, "LINE_HEIGHT") * ts_const(LAYOUT_TS, "MAX_LINES");
    let buttons = ts_const(LAYOUT_TS, "ACTIONS_GAP") + ts_const(LAYOUT_TS, "BUTTON_HEIGHT");
    let chrome = 2.0 * ts_const(LAYOUT_TS, "TEXT_PAD_Y") + 2.0 * ts_const(LAYOUT_TS, "BORDER");
    let need = text + buttons + chrome;
    let budget = ts_const(LAYOUT_TS, "MAX_ISLAND_HEIGHT");
    assert!(
        budget >= need,
        "budget {budget}, six lines and buttons need {need}"
    );
    assert!(
        budget <= need + 1.0,
        "budget {budget} is wasteful, need {need}"
    );
}

/// The widest island is the widest text column plus its chrome; the buttons'
/// row must fit in the narrowest column the layout gives a confirmation.
#[test]
fn the_buttons_row_fits_the_minimum_confirmation_column() {
    let row = 2.0 * ts_const(LAYOUT_TS, "BUTTON_WIDTH") + ts_const(LAYOUT_TS, "BUTTON_GAP");
    assert_eq!(ts_const(LAYOUT_TS, "ACTIONS_WIDTH"), row);
}

/// The frontend dims the buttons for the same half second the host ignores
/// Confirm (`confirm_buttons.rs`).
#[test]
fn the_dim_period_is_the_hosts_guard_period() {
    assert_eq!(ts_const(CONFIRM_TS, "CONFIRM_ARM_MS"), ARM_MS as f64);
}

/// The command names the island calls are the Rust function names (Tauri names
/// a command after its function).
#[test]
fn the_island_calls_the_commands_the_host_defines() {
    let host = include_str!("confirm_buttons.rs");
    for (constant, name) in [
        ("ANSWER_COMMAND", "answer_confirmation"),
        ("REPORT_COMMAND", "report_confirm_buttons"),
    ] {
        assert_eq!(ts_str(CONFIRM_TS, constant), name);
        assert!(
            host.contains(&format!("pub fn {name}(")),
            "no command {name} in confirm_buttons.rs"
        );
    }
}

/// A page may report at most two rectangles, none bigger than a button with some
/// room: the host's caps are derived from the button size in `layout.ts`, so a
/// page cannot make the window capture the mouse over more than its buttons.
#[test]
fn the_reported_rectangle_caps_fit_a_button_and_not_much_more() {
    let (w, h) = (
        ts_const(LAYOUT_TS, "BUTTON_WIDTH"),
        ts_const(LAYOUT_TS, "BUTTON_HEIGHT"),
    );
    assert!((w..=2.0 * w).contains(&MAX_AREA_WIDTH), "{MAX_AREA_WIDTH}");
    assert!(
        (h..=2.0 * h).contains(&MAX_AREA_HEIGHT),
        "{MAX_AREA_HEIGHT}"
    );
    assert_eq!(MAX_AREAS, 2);
}
