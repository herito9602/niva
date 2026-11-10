//! The notch's size and its exit animation are decided in the frontend
//! (`src/notch/motion.ts`, `src/notch/notch.css`); the host owns the window
//! size and the hide delay. These tests read those sources and the window config
//! so the two sides cannot drift apart silently.

use std::sync::Mutex;

use serde_json::Value;

use crate::click_through::{IslandWindow, prepare_island, show_island_window};
use crate::visibility::HIDE_DELAY_MS;

const MOTION_TS: &str = include_str!("../../src/notch/motion.ts");
const NOTCH_CSS: &str = include_str!("../../src/notch/notch.css");
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

/// Value of a `--name: <n>ms;` custom property in the notch stylesheet.
fn css_ms(name: &str) -> f64 {
    let prefix = format!("--{name}:");
    let line = NOTCH_CSS
        .lines()
        .map(str::trim)
        .find(|l| l.starts_with(&prefix))
        .unwrap_or_else(|| panic!("`{prefix} <n>ms;` not found in notch.css"));
    line[prefix.len()..]
        .trim()
        .trim_end_matches(';')
        .trim_end_matches("ms")
        .parse()
        .unwrap_or_else(|_| panic!("{name} is not a plain millisecond value"))
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

#[test]
fn the_ts_constants_are_readable() {
    assert_eq!(ts_const(MOTION_TS, "NOTCH_W"), 118.0);
    assert_eq!(ts_const(MOTION_TS, "NOTCH_H"), 30.0);
    assert!(ts_const(MOTION_TS, "NOTCH_OUT_MS") > 0.0);
}

#[test]
fn island_window_is_200x70_transparent_non_focusable() {
    let window = island_window();
    assert_eq!(window["label"], "island");
    assert_eq!(window["width"].as_f64(), Some(200.0));
    assert_eq!(window["height"].as_f64(), Some(70.0));
    assert_eq!(window["transparent"], true);
    assert_eq!(window["focusable"], false);
    assert_eq!(window["focus"], false);
    assert_eq!(window["decorations"], false);
    assert_eq!(window["shadow"], false);
    assert_eq!(window["alwaysOnTop"], true);
    assert_eq!(window["skipTaskbar"], true);
}

#[test]
fn the_window_fits_the_notch_with_room_for_overshoot_and_pulse() {
    let window = island_window();
    let (w, h) = (
        window["width"].as_f64().unwrap(),
        window["height"].as_f64().unwrap(),
    );
    let (nw, nh) = (
        ts_const(MOTION_TS, "NOTCH_W"),
        ts_const(MOTION_TS, "NOTCH_H"),
    );
    assert!(w >= nw + 2.0 * 40.0, "{w} leaves no side room for the notch");
    assert!(h >= nh + 40.0, "{h} leaves no room below for the overshoot");
    // The stylesheet centers the notch with half of its width.
    assert!(
        NOTCH_CSS.contains(&format!("left: calc(50% - {}px)", nw / 2.0)),
        "notch.css does not center a {nw} px notch"
    );
}

/// A window that records what it was told, in order.
#[derive(Default)]
struct RecordingWindow {
    calls: Mutex<Vec<String>>,
}

impl IslandWindow for RecordingWindow {
    fn position_top_center(&self) {
        self.calls.lock().unwrap().push("position".into());
    }
    fn set_ignore_cursor_events(&self, ignore: bool) -> Result<(), String> {
        self.calls.lock().unwrap().push(format!("ignore {ignore}"));
        Ok(())
    }
    fn show(&self) {
        self.calls.lock().unwrap().push("show".into());
    }
}

#[test]
fn window_ignores_cursor_on_prepare_and_every_show() {
    let win = RecordingWindow::default();
    prepare_island(&win);
    assert_eq!(*win.calls.lock().unwrap(), vec!["position", "ignore true"]);

    let win = RecordingWindow::default();
    for _ in 0..3 {
        show_island_window(&win);
    }
    let calls = win.calls.lock().unwrap().clone();
    // Each show: placed, click-through, and only then visible.
    assert_eq!(calls.len(), 9);
    for show in calls.chunks(3) {
        assert_eq!(show, ["position", "ignore true", "show"]);
    }
}

#[test]
fn hide_delay_parsed_from_motion_ts_within_exit_plus_100() {
    let out = ts_const(MOTION_TS, "NOTCH_OUT_MS");
    let delay = HIDE_DELAY_MS as f64;
    assert!(delay >= out, "hide delay {delay} cuts the {out} ms exit");
    assert!(
        delay <= out + 100.0,
        "hide delay {delay} lags the {out} ms exit"
    );
    // The frontend declares the same delay.
    assert_eq!(ts_const(MOTION_TS, "HIDE_DELAY_MS"), delay);
}

#[test]
fn the_stylesheet_plays_the_exit_and_the_fade_the_host_waits_for() {
    assert_eq!(css_ms("out-ms"), ts_const(MOTION_TS, "NOTCH_OUT_MS"));
    assert_eq!(css_ms("in-ms"), ts_const(MOTION_TS, "NOTCH_IN_MS"));
    let fade = css_ms("fade-ms");
    assert_eq!(fade, ts_const(MOTION_TS, "REDUCED_FADE_MS"));
    assert!(fade < HIDE_DELAY_MS as f64, "the fade outlasts the hide");
}

#[test]
fn arm_ms_is_500() {
    assert_eq!(crate::confirm_keys::ARM_MS, 500);
    assert_eq!(crate::confirm_buttons::ARM_MS, 500);
}
