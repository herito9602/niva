mod cursor;
#[cfg(debug_assertions)]
mod debug;
mod dispatcher;
mod effect_runner;
mod ui_adapter;
mod visibility;

#[cfg(test)]
mod capabilities_tests;

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

use crate::cursor::CursorPoller;
use crate::dispatcher::{Core, Input, InputTx, startup_events};
use crate::effect_runner::{EffectRunner, EventSink, Host};
use crate::ui_adapter::{ISLAND_LABEL, TauriUiAdapter};
use crate::visibility::{HideGuard, ShortcutStatus, shortcut_status};

const HIDE_DELAY_MS: u64 = 250;

/// Center the window horizontally on the primary monitor, flush with the top edge.
fn position_top_center(win: &WebviewWindow) {
    if let (Ok(Some(mon)), Ok(size)) = (win.primary_monitor(), win.outer_size()) {
        let mon_pos = mon.position();
        let x = mon_pos.x + (mon.size().width as i32 - size.width as i32) / 2;
        let _ = win.set_position(PhysicalPosition::new(x, mon_pos.y));
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
            position_top_center(&win);
            let _ = win.show();
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
    #[cfg(debug_assertions)]
    let builder = builder.invoke_handler(tauri::generate_handler![debug::debug_dispatch]);

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
            let core = Core::new(
                EffectRunner::new(sink, host),
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
                position_top_center(&win);
            }

            #[cfg(debug_assertions)]
            debug::open_window(app.handle())?;

            // The tray toggles exactly like the shortcut: through the reducer.
            let toggle =
                MenuItem::with_id(app, "toggle", "Show / hide island", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&toggle, &quit])?;

            let mut tray = TrayIconBuilder::new()
                .tooltip("Niva")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "toggle" => send_event(app, Event::Shortcut),
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
