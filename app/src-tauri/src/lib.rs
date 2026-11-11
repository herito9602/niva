pub mod adapters;
mod click_through;
mod confirm_buttons;
mod confirm_keys;
mod cursor;
#[cfg(debug_assertions)]
mod debug;
#[cfg(debug_assertions)]
mod dev;
mod dispatcher;
mod effect_runner;
#[cfg(feature = "eval")]
pub mod eval;
mod fullscreen;
mod latency;
#[cfg(windows)]
pub mod release;
mod requests;
mod text_window;
mod ui_adapter;
mod visibility;
mod worker_pool;

#[cfg(test)]
mod capabilities_tests;
#[cfg(test)]
mod island_contract_tests;
#[cfg(test)]
mod test_support;

use std::{
    sync::Arc,
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use niva_core::state_machine::Event;
use tauri::{
    AppHandle, Manager, PhysicalPosition, RunEvent, WebviewWindow,
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

use crate::adapters::audio_cue::RodioCue;
use crate::adapters::ollama::{OllamaProvider, keeper};
use crate::adapters::voice_sidecar::{HostClock, ListenModes, Status, Voice};
use crate::click_through::{HitTesting, IslandWindow, prepare_island, show_island_window};
use crate::confirm_buttons::{ConfirmGate, GateUi};
use crate::confirm_keys::{
    ConfirmKeys, Key, KeyState, KeysUi, ShortcutRegistry, TooltipPort, key_of, shortcut,
};
use crate::cursor::CursorPoller;
use crate::dispatcher::{Core, Input, InputTx};
use crate::effect_runner::{EffectRunner, EventSink, Host};
use crate::latency::Latency;
use crate::requests::Requests;
use crate::ui_adapter::{ISLAND_LABEL, TauriUiAdapter};
use crate::visibility::{HIDE_DELAY_MS, HideGuard, ShortcutStatus, shortcut_status};

/// Center the window horizontally on the primary monitor, flush with the top edge.
fn position_top_center(win: &WebviewWindow) {
    if let (Ok(Some(mon)), Ok(size)) = (win.primary_monitor(), win.outer_size()) {
        let mon_pos = mon.position();
        let x = mon_pos.x + (mon.size().width as i32 - size.width as i32) / 2;
        let _ = win.set_position(PhysicalPosition::new(x, mon_pos.y));
    }
}

/// The real window behind the island seam. Click-through is set from Rust, so
/// the webview needs no permission for it.
impl IslandWindow for WebviewWindow {
    fn position_top_center(&self) {
        position_top_center(self);
    }

    fn set_ignore_cursor_events(&self, ignore: bool) -> Result<(), String> {
        WebviewWindow::set_ignore_cursor_events(self, ignore).map_err(|err| err.to_string())
    }

    fn show(&self) {
        let _ = WebviewWindow::show(self);
    }
}

/// Window side of the `ShowIsland` / `HideIsland` effects. The `niva://island`
/// event that drives the animation is published by the UI adapter.
struct TauriHost {
    app: AppHandle,
    cursor: CursorPoller,
    /// A show during the hide delay cancels the pending hide.
    guard: HideGuard,
    /// The confirmation on screen, if any, and where its buttons are.
    gate: Arc<ConfirmGate>,
    /// Decides, on every cursor sample, whether the window captures the mouse.
    hit: HitTesting,
}

impl Host for TauriHost {
    fn show_island(&self) {
        self.guard.show();
        if let Some(win) = self.app.get_webview_window(ISLAND_LABEL) {
            show_island_window(&win);
        }
        // Showing resets the window to click-through behind the tester's back.
        self.hit.forget();
        self.cursor.start(&self.app, ISLAND_LABEL, self.hit.clone());
    }

    fn hide_island(&self) {
        let ticket = self.guard.hide();
        // Nothing may capture the mouse while the island leaves. Closing the
        // gate and letting the mouse through happen under the hit tester's lock,
        // the one a poll holds while it reads the buttons and applies its
        // answer, so a poll cannot capture again after this.
        self.cursor.stop();
        if let Some(win) = self.app.get_webview_window(ISLAND_LABEL) {
            self.hit.release(&win, || self.gate.clear());
            let guard = self.guard.clone();
            // Let the frontend play the slide-up animation, then hide the window.
            thread::spawn(move || {
                thread::sleep(Duration::from_millis(HIDE_DELAY_MS));
                if guard.should_hide(ticket) {
                    let _ = win.hide();
                }
            });
        }
    }
}

fn now_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_millis() as u64)
}

// A tripwire, not a proof: this const only pins the SIGNATURE of
// `dev::assistant`, so a change to it breaks the build and gets noticed. It
// does not stop another constructor from being used. What enforces "debug
// builds act only on the dry-run PC" (the premise of the debug window's
// click-to-confirm, see the SECURITY note in `dev.rs`) is the behavioural test
// `the_assistant_of_a_debug_build_only_ever_acts_on_the_dry_run_pc` in `dev.rs`.
#[cfg(debug_assertions)]
const _: fn(dev::LogSink) -> niva_core::assistant::Assistant = dev::assistant;
// Same tripwire for the second constructor (PR 2c: a real model, dry-run PC).
#[cfg(debug_assertions)]
type DevAssistantWith = fn(
    dev::LogSink,
    Arc<dyn niva_core::ports::LlmProvider>,
    Arc<dyn niva_core::ports::Clock>,
    Arc<dyn niva_core::ports::SearchProvider>,
) -> niva_core::assistant::Assistant;
#[cfg(debug_assertions)]
const _: DevAssistantWith = dev::assistant_with;

/// Debug builds: the text of each reply, as one dev-log line in the debug window.
#[cfg(debug_assertions)]
fn dev_speak_log(app: AppHandle) -> effect_runner::SpeakLog {
    use tauri::Emitter;

    Arc::new(move |line| {
        eprintln!("{line}");
        let payload = serde_json::json!({ "line": line });
        let _ = app.emit_to(debug::DEBUG_WINDOW_LABEL, debug::DEV_LOG_EVENT, payload);
    })
}

/// Who answers the reducer's request effects. Debug builds run the assistant
/// loop on Ollama (dev LLM until it is ready) and the dry-run PC; release builds
/// run it on Ollama and the real Windows adapter of the safe actions, with no
/// dev code in them.
#[cfg(debug_assertions)]
fn build_requests(app: &AppHandle, sink: EventSink) -> (Arc<dyn Requests>, Arc<keeper::Readiness>) {
    use tauri::Emitter;

    let app = app.clone();
    let log: dev::LogSink = Arc::new(move |line| {
        eprintln!("{line}");
        let payload = serde_json::json!({ "line": line });
        let _ = app.emit_to(debug::DEBUG_WINDOW_LABEL, debug::DEV_LOG_EVENT, payload);
    });
    // The keeper waits for Ollama with the design's backoff (it never gives up),
    // warms the model up and keeps it warm; the assistant uses the real model
    // while the keeper says it is ready and the dev LLM until then.
    let ollama = Arc::new(OllamaProvider::new());
    let handle = keeper::spawn(ollama.clone(), log.clone());
    let llm_ready = handle.ready.clone();
    // Search is the real one in debug too (the user's own key; without one the
    // reply says so). Only the PC and the reminders stay dry-run.
    let search = Arc::new(adapters::search::TavilySearch::from_environment());
    let assistant = dev::adaptive_assistant(log, ollama, handle, search);
    (
        Arc::new(requests::AssistantRequests::new(assistant, sink)),
        llm_ready,
    )
}

/// Release builds: the real Ollama (kept warm by the keeper, never a fallback
/// model) and the Windows adapter of every action. A risky plan asks for a
/// confirmation, answered by the island's buttons; see `release.rs`.
#[cfg(not(debug_assertions))]
fn build_requests(
    _app: &AppHandle,
    sink: EventSink,
) -> (Arc<dyn Requests>, Arc<keeper::Readiness>) {
    let log: Arc<dyn Fn(String) + Send + Sync> = Arc::new(|line| eprintln!("{line}"));
    let ollama = Arc::new(OllamaProvider::new());
    let handle = keeper::spawn(ollama.clone(), log);
    let llm_ready = handle.ready.clone();
    let assistant = release::assistant(
        Arc::new(keeper::WarmLlm::new(ollama, handle)),
        Arc::new(adapters::windows_pc::WindowsPc::real()),
        Arc::new(adapters::system_clock::SystemClock),
        Arc::new(adapters::search::TavilySearch::from_environment()),
    );
    (
        Arc::new(requests::AssistantRequests::new(assistant, sink)),
        llm_ready,
    )
}

/// The id of the tray icon, so the voice status can reach its tooltip.
const TRAY_ID: &str = "niva-tray";

/// Registers the confirmation keys with the global-shortcut plugin. The plugin
/// sends every register and unregister to the main thread itself and waits for
/// it (checked in tauri-plugin-global-shortcut 2.4.1), so it is called from
/// any thread as it is, with no `run_on_main_thread` of ours around it.
struct PluginKeys(AppHandle);

impl ShortcutRegistry for PluginKeys {
    fn register(&self, key: Key) -> Result<(), String> {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        self.0
            .global_shortcut()
            .register(shortcut(key))
            .map_err(|err| err.to_string())
    }

    fn unregister(&self, key: Key) {
        use tauri_plugin_global_shortcut::GlobalShortcutExt;
        let _ = self.0.global_shortcut().unregister(shortcut(key));
    }
}

/// The tray tooltip, set on the main thread like the voice status.
struct TrayTooltip(AppHandle);

impl TooltipPort for TrayTooltip {
    fn set_tooltip(&self, text: &str) {
        let (app, text) = (self.0.clone(), text.to_owned());
        let _ = self.0.run_on_main_thread(move || {
            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                let _ = tray.set_tooltip(Some(text));
            }
        });
    }
}

/// Quit: let go of the confirmation keys, so the OS keeps no shortcut of ours.
fn release_confirm_keys(app: &AppHandle) {
    if let Some(keys) = app.try_state::<Arc<ConfirmKeys>>() {
        keys.quit();
    }
}

/// Starts the voice supervisor (task 3.4) in debug and release alike. It never
/// fails the app: with no sidecar the voice stays down and the text window is
/// the way in. Its events enter the dispatcher like any other, and its status
/// is the tray tooltip.
fn start_voice(
    app: &AppHandle,
    sink: EventSink,
    clock: HostClock,
    llm_ready: Arc<keeper::Readiness>,
) -> Voice {
    let handle = app.clone();
    let status = Arc::new(move |status: Status| {
        let text = format!("Niva: {}", status.describe());
        let app = handle.clone();
        // The tray belongs to the main thread; the status comes from the
        // supervisor's.
        let _ = handle.run_on_main_thread(move || {
            if let Some(tray) = app.tray_by_id(TRAY_ID) {
                let _ = tray.set_tooltip(Some(text));
            }
        });
    });
    Voice::start(
        sink,
        status,
        clock,
        Arc::new(|line| eprintln!("{line}")),
        Some(Arc::new(move || llm_ready.is_ready())),
    )
}

/// The per-turn latency log (task 3.5), in debug and release alike. `clock`
/// is the one the voice translator uses. With no folder to write to there is
/// no log; a failing disk never touches a turn (see `latency::sink`).
fn start_latency(clock: HostClock, modes: Arc<ListenModes>) -> Option<Latency> {
    let path = latency::sink::default_log_path()?;
    eprintln!("[niva] latency log: {}", path.display());
    let emit = latency::sink::start(path)?;
    let (for_stamps, for_whisper) = (modes.clone(), modes);
    Some(Latency::new(
        Box::new(move || clock()),
        Box::new(now_ms),
        Box::new(move |id| for_stamps.stamp(id)),
        Box::new(move || for_whisper.whisper()),
        emit,
    ))
}

/// Opens the "Type a command" window. Built off the event loop thread, so a
/// window creation never waits on the loop that is handling the tray click.
fn open_text_window(app: &AppHandle) {
    let app = app.clone();
    tauri::async_runtime::spawn(async move {
        if let Err(err) = text_window::open_window(&app) {
            eprintln!("[niva] could not open the text window: {err}");
        }
    });
}

fn send_event(app: &AppHandle, event: Event) {
    if let Err(err) = app.state::<InputTx>().send(Input::Event(event)) {
        eprintln!("[niva] dispatcher is gone: {err}");
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let toggle_shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space);

    let builder = tauri::Builder::default().plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, sc, event| {
                if sc == &toggle_shortcut && event.state() == ShortcutState::Pressed {
                    send_event(app, Event::Shortcut);
                } else if let Some(key) = key_of(sc) {
                    // The confirmation keys go only through `ConfirmKeys`, which
                    // checks what is pending and builds the answer itself.
                    if let Some(keys) = app.try_state::<Arc<ConfirmKeys>>() {
                        let state = match event.state() {
                            ShortcutState::Pressed => KeyState::Pressed,
                            ShortcutState::Released => KeyState::Released,
                        };
                        keys.press(key, state);
                    }
                }
            })
            .build(),
    );
    // The release webviews' commands: the text window's `submit_text` and the
    // island's two narrow ones (answer a confirmation, report where its buttons
    // are). Each window's capability grants its own and nothing else. The debug
    // window adds `debug_dispatch`.
    #[cfg(debug_assertions)]
    let builder = builder.invoke_handler(tauri::generate_handler![
        debug::debug_dispatch,
        text_window::submit_text,
        confirm_buttons::answer_confirmation,
        confirm_buttons::report_confirm_buttons
    ]);
    #[cfg(not(debug_assertions))]
    let builder = builder.invoke_handler(tauri::generate_handler![
        text_window::submit_text,
        confirm_buttons::answer_confirmation,
        confirm_buttons::report_confirm_buttons
    ]);

    builder
        .setup(move |app| {
            use tauri_plugin_global_shortcut::GlobalShortcutExt;

            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Input>();
            app.manage(tx.clone());

            // The buttons' gate runs on a monotonic clock: a wall-clock step
            // must not shorten the accidental-click guard.
            let started = Instant::now();
            let gate = Arc::new(ConfirmGate::new(Arc::new(move || {
                started.elapsed().as_millis() as u64
            })));
            app.manage(gate.clone());

            // Wiring: timers and synthesized replies re-enter through `tx`.
            let host = Arc::new(TauriHost {
                app: app.handle().clone(),
                cursor: CursorPoller::default(),
                guard: HideGuard::default(),
                gate: gate.clone(),
                hit: HitTesting::new({
                    let gate = gate.clone();
                    Arc::new(move || gate.areas())
                }),
            });
            let sink: EventSink = {
                let tx = tx.clone();
                Arc::new(move |event| {
                    let _ = tx.send(Input::Event(event));
                })
            };
            let (requests, llm_ready) = build_requests(app.handle(), sink.clone());
            // The voice sidecar: speech in and out, and the host's own cues.
            // One monotonic clock for the voice translator and the latency
            // trace: every stage of a turn is on the same axis.
            let host_clock: HostClock = {
                let started = Instant::now();
                Arc::new(move || started.elapsed().as_millis() as u64)
            };
            let voice = start_voice(app.handle(), sink.clone(), host_clock.clone(), llm_ready);
            let latency = start_latency(host_clock, voice.modes());
            let link = voice.link();
            app.manage(voice);
            // The sleep countdown reads the user's last input time (no hook). Without
            // a source the runner fails every countdown closed.
            let runner = EffectRunner::new(sink, host, requests)
                .with_voice(link.clone(), link)
                .with_cue(Arc::new(RodioCue::new()));
            #[cfg(windows)]
            let runner =
                runner.with_activity(Arc::new(adapters::windows_activity::WindowsActivity));
            // Over an exclusive fullscreen game the notch cannot be drawn: listening
            // is announced by the host chime instead.
            #[cfg(windows)]
            let runner = runner.with_fullscreen(Arc::new(fullscreen::WindowsFullscreen));
            // Debug builds only: what Niva says goes to the debug window's log. The
            // notch draws no text, and release builds log none.
            #[cfg(debug_assertions)]
            let runner = runner.with_speak_log(dev_speak_log(app.handle().clone()));
            // The confirmation keys: Esc and Ctrl+Shift+Enter, registered only
            // while a confirmation is on screen. Their answers re-enter through
            // the same queue as every other event.
            let keys = Arc::new(ConfirmKeys::new(
                Arc::new(PluginKeys(app.handle().clone())),
                Arc::new({
                    let tx = tx.clone();
                    move |event| {
                        let _ = tx.send(Input::Event(event));
                    }
                }),
                Arc::new(TrayTooltip(app.handle().clone())),
                {
                    let started = Instant::now();
                    Arc::new(move || started.elapsed().as_millis() as u64)
                },
                Arc::new(|line| eprintln!("{line}")),
            ));
            app.manage(keys.clone());
            let mut core = Core::new(
                runner,
                // The keys and the gate see every view before the island does.
                Arc::new(KeysUi::new(
                    Arc::new(GateUi::new(
                        Arc::new(TauriUiAdapter::new(
                            Arc::new(app.handle().clone()),
                            cfg!(debug_assertions),
                        )),
                        gate.clone(),
                    )),
                    keys,
                )),
                Box::new(now_ms),
            );
            if let Some(latency) = latency {
                core.set_latency(latency);
            }
            tauri::async_runtime::spawn(dispatcher::run(core, rx));

            // A taken shortcut must not abort startup: the tray toggle remains.
            if let ShortcutStatus::Unavailable(reason) =
                shortcut_status(app.global_shortcut().register(toggle_shortcut))
            {
                eprintln!("[niva] global shortcut unavailable, use the tray: {reason}");
            }

            if let Some(win) = app.get_webview_window(ISLAND_LABEL) {
                prepare_island(&win);
            }

            #[cfg(debug_assertions)]
            debug::open_window(app.handle())?;

            // The tray toggles exactly like the shortcut: through the reducer.
            let toggle =
                MenuItem::with_id(app, "toggle", "Show / hide island", true, None::<&str>)?;
            let type_command =
                MenuItem::with_id(app, "type", "Type a command", true, None::<&str>)?;
            let restart_voice =
                MenuItem::with_id(app, "restart_voice", "Restart voice", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &type_command, &restart_voice, &quit])?;

            let mut tray = TrayIconBuilder::with_id(TRAY_ID)
                .tooltip("Niva")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => send_event(app, Event::Shortcut),
                    "type" => open_text_window(app),
                    "restart_voice" => {
                        if let Some(voice) = app.try_state::<Voice>() {
                            voice.restart();
                        }
                    }
                    "quit" => {
                        release_confirm_keys(app);
                        app.exit(0);
                    }
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick {
                        button: MouseButton::Left,
                        ..
                    } = event
                    {
                        send_event(tray.app_handle(), Event::Shortcut);
                    }
                });
            if let Some(icon) = app.default_window_icon() {
                tray = tray.icon(icon.clone());
            }
            tray.build(app)?;
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("error while building tauri application")
        .run(|app, event| {
            // Stop the sidecar before the process ends: it holds the microphone.
            // (If the app is killed instead, the job object around the sidecar
            // ends it; see `adapters::voice_sidecar::process`.)
            if matches!(event, RunEvent::Exit) {
                release_confirm_keys(app);
                if let Some(voice) = app.try_state::<Voice>() {
                    voice.shutdown();
                }
            }
        });
}
