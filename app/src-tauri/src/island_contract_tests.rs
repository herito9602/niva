//! The island's size and its exit animation are decided in the frontend
//! (`src/island-ui/layout.ts`, `choreography.ts`); the host owns the window
//! size and the hide delay. These tests read those TS constants and the window
//! config so the two sides cannot drift apart silently.

use serde_json::Value;

use crate::visibility::HIDE_DELAY_MS;

const LAYOUT_TS: &str = include_str!("../../src/island-ui/layout.ts");
const CHOREOGRAPHY_TS: &str = include_str!("../../src/island-ui/choreography.ts");
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
