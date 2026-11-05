pub mod adapters;
mod click_through;
mod cursor;
#[cfg(debug_assertions)]
mod debug;
#[cfg(debug_assertions)]
mod dev;
mod dispatcher;
mod effect_runner;
#[cfg(feature = "eval")]
pub mod eval;
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
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use niva_core::state_machine::Event;
use tauri::{
    AppHandle, Manager, PhysicalPosition, WebviewWindow,
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
};
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

use crate::adapters::ollama::{OllamaProvider, keeper};
use crate::click_through::{IslandWindow, prepare_island, show_island_window};
use crate::cursor::CursorPoller;
use crate::dispatcher::{Core, Input, InputTx, startup_events};
use crate::effect_runner::{EffectRunner, EventSink, Host};
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
}

impl Host for TauriHost {
    fn show_island(&self) {
        self.guard.show();
        if let Some(win) = self.app.get_webview_window(ISLAND_LABEL) {
            show_island_window(&win);
        }
        self.cursor.start(&self.app, ISLAND_LABEL);
    }

    fn hide_island(&self) {
        let ticket = self.guard.hide();
        self.cursor.stop();
        if let Some(win) = self.app.get_webview_window(ISLAND_LABEL) {
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
const _: fn(
    dev::LogSink,
    Arc<dyn niva_core::ports::LlmProvider>,
    Arc<dyn niva_core::ports::Clock>,
) -> niva_core::assistant::Assistant = dev::assistant_with;

/// Who answers the reducer's request effects. Debug builds run the assistant
/// loop on Ollama (dev LLM until it is ready) and the dry-run PC; release builds
/// run it on Ollama and the real Windows adapter of the safe actions, with no
/// dev code in them.
#[cfg(debug_assertions)]
fn build_requests(app: &AppHandle, sink: EventSink) -> Arc<dyn Requests> {
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
    let assistant = dev::adaptive_assistant(log, ollama, handle);
    Arc::new(requests::AssistantRequests::new(assistant, sink))
}

/// Release builds: the real Ollama (kept warm by the keeper, never a fallback
/// model) and the Windows adapter of the SAFE actions. The assistant is built
/// without risky actions, so `close_app` and power never reach Windows here;
/// see `release.rs`.
#[cfg(not(debug_assertions))]
fn build_requests(_app: &AppHandle, sink: EventSink) -> Arc<dyn Requests> {
    let log: Arc<dyn Fn(String) + Send + Sync> = Arc::new(|line| eprintln!("{line}"));
    let ollama = Arc::new(OllamaProvider::new());
    let handle = keeper::spawn(ollama.clone(), log);
    let assistant = release::assistant(
        Arc::new(keeper::WarmLlm::new(ollama, handle)),
        Arc::new(adapters::windows_pc::WindowsPc::real()),
        Arc::new(adapters::system_clock::SystemClock),
    );
    Arc::new(requests::AssistantRequests::new(assistant, sink))
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
    let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space);

    let builder = tauri::Builder::default().plugin(
        tauri_plugin_global_shortcut::Builder::new()
            .with_handler(move |app, sc, event| {
                if sc == &shortcut && event.state() == ShortcutState::Pressed {
                    send_event(app, Event::Shortcut);
                }
            })
            .build(),
    );
    // The release webview's only command is the text window's `submit_text`
    // (its capability grants nothing else). The debug window adds its own.
    #[cfg(debug_assertions)]
    let builder = builder.invoke_handler(tauri::generate_handler![
        debug::debug_dispatch,
        text_window::submit_text
    ]);
    #[cfg(not(debug_assertions))]
    let builder = builder.invoke_handler(tauri::generate_handler![text_window::submit_text]);

    builder
        .setup(move |app| {
            use tauri_plugin_global_shortcut::GlobalShortcutExt;

            let (tx, rx) = tokio::sync::mpsc::unbounded_channel::<Input>();
            app.manage(tx.clone());

            // Wiring: timers and synthesized replies re-enter through `tx`.
            let host = Arc::new(TauriHost {
                app: app.handle().clone(),
                cursor: CursorPoller::default(),
                guard: HideGuard::default(),
            });
            let sink: EventSink = {
                let tx = tx.clone();
                Arc::new(move |event| {
                    let _ = tx.send(Input::Event(event));
                })
            };
            let requests = build_requests(app.handle(), sink.clone());
            // The sleep countdown reads the user's last input time (no hook). Without
            // a source the runner fails every countdown closed.
            let runner = EffectRunner::new(sink, host, requests);
            #[cfg(windows)]
            let runner =
                runner.with_activity(Arc::new(adapters::windows_activity::WindowsActivity));
            let core = Core::new(
                runner,
                Arc::new(TauriUiAdapter::new(
                    Arc::new(app.handle().clone()),
                    cfg!(debug_assertions),
                )),
                Box::new(now_ms),
            );
            for event in startup_events(cfg!(debug_assertions)) {
                let _ = tx.send(Input::Event(event));
            }
            tauri::async_runtime::spawn(dispatcher::run(core, rx));

            // A taken shortcut must not abort startup: the tray toggle remains.
            if let ShortcutStatus::Unavailable(reason) =
                shortcut_status(app.global_shortcut().register(shortcut))
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
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &type_command, &quit])?;

            let mut tray = TrayIconBuilder::new()
                .tooltip("Niva")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => send_event(app, Event::Shortcut),
                    "type" => open_text_window(app),
                    "quit" => app.exit(0),
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
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
