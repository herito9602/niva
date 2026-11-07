//! The host's handle on the running sidecar: the speech and speaker ports.
//!
//! `Link` outlives every sidecar process. The supervisor attaches the writer of
//! the process it started and detaches it when that process goes away, so a
//! port call made while the sidecar is down fails fast (`false`) and never
//! blocks or panics. Commands only go into a channel; a writer thread owns the
//! pipe, so a sidecar that stops reading cannot freeze the dispatcher.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, PoisonError};

use niva_core::ports::{Speaker, SpeechEngine, SpokenPrompt};
use niva_core::replies::Lang;
use niva_core::state_machine::ListenMode;

use super::translate::ListenModes;
use super::wire::{ListenContext, Outgoing, Private};

pub struct Link {
    /// Voice is usable: the sidecar said `ready` and has not failed since.
    /// This mirrors the reducer's `voice_available`, set by the supervisor in
    /// the same step that sends the reducer its event.
    up: AtomicBool,
    out: Mutex<Option<Sender<Outgoing>>>,
    modes: Arc<ListenModes>,
}

impl Link {
    pub fn new(modes: Arc<ListenModes>) -> Arc<Link> {
        Arc::new(Link {
            up: AtomicBool::new(false),
            out: Mutex::new(None),
            modes,
        })
    }

    pub fn set_up(&self, up: bool) {
        self.up.store(up, Ordering::SeqCst);
    }

    pub fn is_up(&self) -> bool {
        self.up.load(Ordering::SeqCst)
    }

    /// The writer of the process that just started.
    pub fn attach(&self, out: Sender<Outgoing>) {
        *self.out.lock().unwrap_or_else(PoisonError::into_inner) = Some(out);
    }

    /// The process is gone: further commands fail at once.
    pub fn detach(&self) {
        self.set_up(false);
        *self.out.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }

    /// Queues a command for the sidecar. `false`: nobody to send it to.
    pub fn send(&self, message: Outgoing) -> bool {
        let out = self.out.lock().unwrap_or_else(PoisonError::into_inner);
        out.as_ref().is_some_and(|tx| tx.send(message).is_ok())
    }
}

impl SpeechEngine for Link {
    /// Never a listen to a sidecar that is not ready: the reducer already
    /// guards on `voice_available`, and this is the same rule one step later.
    fn listen(&self, id: u64, mode: ListenMode) -> bool {
        self.listen_with_prompt(id, mode, None)
    }

    /// Only a confirm listen carries the question; a command listen never does.
    fn listen_with_prompt(&self, id: u64, mode: ListenMode, prompt: Option<&SpokenPrompt>) -> bool {
        if !self.is_up() {
            return false;
        }
        self.modes.set(id, mode);
        let context = prompt
            .filter(|_| mode == ListenMode::Confirm)
            .map(|prompt| ListenContext {
                lang: prompt.lang,
                text: Private::new(prompt.text.as_str()),
            });
        self.send(Outgoing::Listen { id, mode, context })
    }

    fn cancel_listen(&self, id: u64) -> bool {
        self.send(Outgoing::CancelListen { id })
    }

    fn force_end_of_speech(&self, id: u64) -> bool {
        self.send(Outgoing::ForceEnd { id })
    }
}

impl Speaker for Link {
    fn is_available(&self) -> bool {
        self.is_up()
    }

    fn speak(&self, id: u64, text: &str, lang: Lang) -> bool {
        if !self.is_up() {
            return false;
        }
        self.send(Outgoing::Speak {
            id,
            text: Private::new(text),
            lang,
        })
    }

    fn stop(&self) -> bool {
        self.send(Outgoing::StopSpeaking)
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc::{Receiver, channel};

    use super::*;

    fn attached() -> (Arc<Link>, Receiver<Outgoing>, Arc<ListenModes>) {
        let modes = Arc::new(ListenModes::default());
        let link = Link::new(modes.clone());
        let (tx, rx) = channel();
        link.attach(tx);
        (link, rx, modes)
    }

    #[test]
    fn nothing_is_sent_before_the_sidecar_is_up() {
        let (link, rx, modes) = attached();
        assert!(!link.listen(1, ListenMode::Command));
        assert!(!link.speak(2, "hi", Lang::En));
        assert!(!link.is_up());
        assert!(rx.try_recv().is_err());
        assert_eq!(modes.get(1), None);
    }

    #[test]
    fn when_up_commands_reach_the_writer_and_the_mode_is_remembered() {
        let (link, rx, modes) = attached();
        link.set_up(true);
        assert!(link.listen(7, ListenMode::Confirm));
        assert_eq!(modes.get(7), Some(ListenMode::Confirm));
        assert!(link.speak(8, "Hola", Lang::Es));
        assert!(link.force_end_of_speech(7));
        assert!(link.cancel_listen(7));
        assert!(link.stop());
        let got: Vec<_> = rx.try_iter().collect();
        assert_eq!(got.len(), 5);
        assert_eq!(
            got[0],
            Outgoing::Listen {
                id: 7,
                mode: ListenMode::Confirm,
                context: None
            }
        );
        assert_eq!(got[2], Outgoing::ForceEnd { id: 7 });
        assert_eq!(got[3], Outgoing::CancelListen { id: 7 });
        assert_eq!(got[4], Outgoing::StopSpeaking);
    }

    #[test]
    fn a_confirm_listen_carries_the_question_and_a_command_listen_never_does() {
        let (link, rx, _) = attached();
        link.set_up(true);
        let prompt = SpokenPrompt {
            lang: Lang::Es,
            text: "¿Cierro la calculadora?".into(),
        };
        assert!(link.listen_with_prompt(1, ListenMode::Confirm, Some(&prompt)));
        assert!(link.listen_with_prompt(2, ListenMode::Command, Some(&prompt)));
        assert!(link.listen_with_prompt(3, ListenMode::Confirm, None));
        let got: Vec<_> = rx.try_iter().collect();
        assert_eq!(
            got[0],
            Outgoing::Listen {
                id: 1,
                mode: ListenMode::Confirm,
                context: Some(ListenContext {
                    lang: Lang::Es,
                    text: Private::new("¿Cierro la calculadora?")
                })
            }
        );
        assert_eq!(
            got[1],
            Outgoing::Listen {
                id: 2,
                mode: ListenMode::Command,
                context: None
            }
        );
        assert_eq!(
            got[2],
            Outgoing::Listen {
                id: 3,
                mode: ListenMode::Confirm,
                context: None
            }
        );
    }

    #[test]
    fn cancelling_and_stopping_still_go_out_while_voice_is_marked_down() {
        // A microphone error marks voice down, but the sidecar is alive and a
        // reply may still be playing: stop and cancel must reach it.
        let (link, rx, _) = attached();
        link.set_up(false);
        assert!(link.stop());
        assert!(link.cancel_listen(3));
        assert_eq!(rx.try_iter().count(), 2);
    }

    #[test]
    fn after_detach_everything_fails_fast() {
        let (link, rx, _) = attached();
        link.set_up(true);
        link.detach();
        assert!(!link.is_up());
        assert!(!link.stop());
        assert!(!link.cancel_listen(1));
        assert!(!link.speak(1, "x", Lang::En));
        assert!(rx.try_recv().is_err());
    }

    #[test]
    fn a_writer_that_is_gone_is_a_failed_send_not_a_panic() {
        let (link, rx, _) = attached();
        link.set_up(true);
        drop(rx);
        assert!(!link.stop());
        assert!(!link.listen(1, ListenMode::Command));
    }
}
