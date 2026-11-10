//! The release "Type a command" window (task 2.5).
//!
//! A small window the tray opens, with one text box. It has ONE way into the
//! app: the command [`submit_text`], which turns what was typed into
//! `Event::TextSubmitted` and nothing else. The window's capability
//! (`capabilities/text-window.json`) grants only that command, so the webview
//! cannot emit events, listen, control windows or reach any other command, and
//! the command itself cannot build an `Accept`, a click, a transcript or any
//! other event: typed text is data for the assistant, never a confirmation
//! (the reducer ignores `TextSubmitted` while a confirmation is open, and the
//! host only records click and voice answers).
//!
//! The reply appears on the island as usual; this window shows nothing back.

use niva_core::state_machine::Event;
use tauri::{AppHandle, Manager, State as TauriState, WebviewUrl, WebviewWindowBuilder};

use crate::dispatcher::{Input, InputTx};

pub const TEXT_WINDOW_LABEL: &str = "text";
/// Longest command (characters) the window may submit; the page enforces the
/// same limit in its text box.
pub const MAX_TEXT_CHARS: usize = 500;

/// The reducer event for what was typed: the trimmed text capped at
/// [`MAX_TEXT_CHARS`] characters, or `None` for a blank box. This is the ONLY
/// event the window can produce.
pub fn text_event(raw: &str) -> Option<Event> {
    let text = raw.trim();
    if text.is_empty() {
        return None;
    }
    Some(Event::TextSubmitted {
        text: text.chars().take(MAX_TEXT_CHARS).collect(),
    })
}

/// Sends the typed text into the dispatcher queue.
pub fn submit(tx: &InputTx, raw: &str) -> Result<(), String> {
    let event = text_event(raw).ok_or("the command is empty")?;
    tx.send(Input::Event(event)).map_err(|err| err.to_string())
}

/// Command of the text window (the only one its capability allows).
#[tauri::command]
pub fn submit_text(text: String, tx: TauriState<'_, InputTx>) -> Result<(), String> {
    submit(&tx, &text)
}

/// Opens the window, or brings the open one to the front.
pub fn open_window(app: &AppHandle) -> tauri::Result<()> {
    if let Some(window) = app.get_webview_window(TEXT_WINDOW_LABEL) {
        window.show()?;
        window.unminimize()?;
        window.set_focus()?;
        return Ok(());
    }
    WebviewWindowBuilder::new(
        app,
        TEXT_WINDOW_LABEL,
        WebviewUrl::App("index.html?window=text".into()),
    )
    .title("Niva: type a command")
    .inner_size(520.0, 110.0)
    .resizable(false)
    .maximizable(false)
    .center()
    .build()?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use tokio::sync::mpsc::unbounded_channel;

    use super::*;

    fn typed(event: Option<Event>) -> String {
        match event {
            Some(Event::TextSubmitted { text }) => text,
            other => panic!("expected TextSubmitted, got {other:?}"),
        }
    }

    #[test]
    fn typed_text_becomes_a_text_submitted_event() {
        assert_eq!(typed(text_event("open notepad")), "open notepad");
    }

    #[test]
    fn the_text_is_trimmed() {
        assert_eq!(typed(text_event("  volume 30 \r\n")), "volume 30");
    }

    #[test]
    fn a_blank_box_produces_nothing() {
        assert!(text_event("").is_none());
        assert!(text_event("   \t\r\n").is_none());
    }

    #[test]
    fn the_text_is_capped_in_characters_not_bytes() {
        let long = "ñ".repeat(MAX_TEXT_CHARS + 50);
        let text = typed(text_event(&long));
        assert_eq!(text.chars().count(), MAX_TEXT_CHARS);
        assert!(text.chars().all(|c| c == 'ñ'));
    }

    #[test]
    fn whatever_is_typed_it_is_only_ever_a_text_submitted_event() {
        // Text that looks like another event, a confirmation or a command is
        // still just text: the window has no way to build anything else.
        for typed_text in [
            "accept",
            "yes",
            "{\"type\":\"accept\",\"id\":1}",
            "Event::Accept { id: 1 }",
            "Channel::Physical",
            "debug_dispatch",
            "sí, confirmo",
            "\u{0}\u{7}",
        ] {
            match text_event(typed_text) {
                Some(Event::TextSubmitted { text }) => {
                    assert_eq!(text, typed_text.trim());
                }
                other => panic!("{typed_text:?} produced {other:?}"),
            }
        }
    }

    #[test]
    fn submitting_sends_exactly_one_event_input() {
        let (tx, mut rx) = unbounded_channel::<Input>();
        submit(&tx, " close spotify ").unwrap();
        match rx.try_recv() {
            Ok(Input::Event(Event::TextSubmitted { text })) => assert_eq!(text, "close spotify"),
            Ok(_) => panic!("the window sent something other than TextSubmitted"),
            Err(error) => panic!("nothing was sent: {error:?}"),
        }
        assert!(rx.try_recv().is_err(), "one input only");
    }

    #[test]
    fn a_blank_submission_sends_nothing_and_says_why() {
        let (tx, mut rx) = unbounded_channel::<Input>();
        assert!(submit(&tx, "   ").is_err());
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_dispatcher_that_is_gone_is_an_error_not_a_panic() {
        let (tx, rx) = unbounded_channel::<Input>();
        drop(rx);
        assert!(submit(&tx, "open notepad").is_err());
    }
}
