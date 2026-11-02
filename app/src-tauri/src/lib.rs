use std::{thread, time::Duration};

use tauri::{
    menu::{Menu, MenuItem},
    tray::{MouseButton, TrayIconBuilder, TrayIconEvent},
    AppHandle, Emitter, Manager, PhysicalPosition, WebviewWindow,
};
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut, ShortcutState};

const HIDE_DELAY_MS: u64 = 250;

/// Center the window horizontally on the primary monitor, flush with the top edge.
fn position_top_center(win: &WebviewWindow) {
    if let (Ok(Some(mon)), Ok(size)) = (win.primary_monitor(), win.outer_size()) {
        let mon_pos = mon.position();
        let x = mon_pos.x + (mon.size().width as i32 - size.width as i32) / 2;
        let _ = win.set_position(PhysicalPosition::new(x, mon_pos.y));
    }
}

fn show_island(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("island") {
        position_top_center(&win);
        let _ = win.show();
        let _ = win.emit("island:show", ());
    }
}

fn hide_island(app: &AppHandle) {
    if let Some(win) = app.get_webview_window("island") {
        let _ = win.emit("island:hide", ());
        // Let the frontend play the slide-up animation, then hide the window.
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(HIDE_DELAY_MS));
            let _ = win.hide();
        });
    }
}

fn toggle_island(app: &AppHandle) {
    let visible = app
        .get_webview_window("island")
        .and_then(|w| w.is_visible().ok())
        .unwrap_or(false);
    if visible {
        hide_island(app);
    } else {
        show_island(app);
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let shortcut = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space);

    tauri::Builder::default()
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, sc, event| {
                    if sc == &shortcut && event.state() == ShortcutState::Pressed {
                        toggle_island(app);
                    }
                })
                .build(),
        )
        .setup(move |app| {
            use tauri_plugin_global_shortcut::GlobalShortcutExt;
            app.global_shortcut().register(shortcut)?;

            if let Some(win) = app.get_webview_window("island") {
                position_top_center(&win);
            }

            let show = MenuItem::with_id(app, "show", "Show Niva", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;

            let mut tray = TrayIconBuilder::new()
                .tooltip("Niva")
                .menu(&menu)
                .show_menu_on_left_click(false)
                .on_menu_event(|app, event| match event.id.as_ref() {
                    "show" => show_island(app),
                    "quit" => app.exit(0),
                    _ => {}
                })
                .on_tray_icon_event(|tray, event| {
                    if let TrayIconEvent::DoubleClick { button: MouseButton::Left, .. } = event {
                        show_island(tray.app_handle());
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
