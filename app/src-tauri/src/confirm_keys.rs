//! The confirmation keys: Esc and Ctrl+Shift+Enter (design L8, L9, L10).
//!
//! Two global shortcuts that exist only while a confirmation is on screen:
//!
//! * **Esc** exists for any confirmation and rejects it (`Event::Reject{id}`).
//! * **Ctrl+Shift+Enter** exists only for a confirmation that voice cannot
//!   answer (restart and shutdown, `chord` in the view) and only once the
//!   prompt has been spoken (`expiresAt` is present). It accepts it
//!   (`Event::Accept{id}`), and a press in the first [`ARM_MS`] after it was
//!   registered is ignored.
//!
//! The OS shortcut is a physical input: neither the model, typed text nor
//! speech can produce it. A press carries no id of its own; it is attributed
//! to the confirmation this module is tracking, never to an older one. One
//! answer per confirmation; both keys are released on every exit (answer,
//! timeout, replacement, island hidden, quit).
//!
//! Registration goes through the [`ShortcutRegistry`] seam, so the lifecycle is
//! tested without an OS. The real registry calls the global-shortcut plugin,
//! which dispatches to the main thread by itself and blocks until it is done;
//! that is why no lock is held across a registry call here: a press is handled
//! on the main thread and takes the same lock.
//!
//! [`KeysUi`] keeps the keys in step with the views the host publishes. It sits
//! next to the buttons' `GateUi` and changes nothing about them.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use niva_core::state_machine::Event;
use tauri_plugin_global_shortcut::{Code, Modifiers, Shortcut};

use crate::effect_runner::EventSink;
use crate::ui_adapter::{UiEvent, UiPort};

/// A press of the chord within this many ms of registering it is ignored.
pub const ARM_MS: u64 = 500;

/// Tray tooltip when the chord could not be registered.
pub const KEY_UNAVAILABLE_TOOLTIP: &str = "Confirm key unavailable";

/// Tray tooltip once the chord registers again.
pub const DEFAULT_TOOLTIP: &str = "Niva";

/// The two confirmation keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    /// Esc: rejects.
    Esc,
    /// Ctrl+Shift+Enter: accepts a restart or shutdown.
    Chord,
}

/// Whether a key event is the press or the release.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyState {
    Pressed,
    Released,
}

/// The OS shortcut of a key.
pub fn shortcut(key: Key) -> Shortcut {
    match key {
        Key::Esc => Shortcut::new(None, Code::Escape),
        Key::Chord => Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Enter),
    }
}

/// The confirmation key behind an OS shortcut, if it is one of the two.
pub fn key_of(candidate: &Shortcut) -> Option<Key> {
    [Key::Esc, Key::Chord]
        .into_iter()
        .find(|key| shortcut(*key).id() == candidate.id())
}

/// Registers and unregisters the OS-wide shortcuts.
pub trait ShortcutRegistry: Send + Sync {
    fn register(&self, key: Key) -> Result<(), String>;
    /// Best effort: unregistering a key that is not registered is harmless.
    fn unregister(&self, key: Key);
}

/// The tray tooltip.
pub trait TooltipPort: Send + Sync {
    fn set_tooltip(&self, text: &str);
}

/// Milliseconds on a monotonic clock.
pub type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// One log line (the host prints it to stderr).
pub type LogFn = Arc<dyn Fn(String) + Send + Sync>;

/// What the host needs from a confirmation view to manage the keys.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Shown {
    pub id: u64,
    /// Voice cannot answer it: the chord applies.
    pub chord: bool,
    /// The prompt has been spoken and the 10 s window runs (`expiresAt`).
    pub spoken: bool,
}

struct Pending {
    id: u64,
    esc_registered: bool,
    /// When the chord was registered (host clock); `None` while it is not.
    chord_armed_at: Option<u64>,
    /// The chord was pressed inside the arm window, so it may be a key that was
    /// already held when it was registered: no press counts until a release.
    chord_held: bool,
    answered: bool,
}

pub struct ConfirmKeys {
    registry: Arc<dyn ShortcutRegistry>,
    sink: EventSink,
    tooltip: Arc<dyn TooltipPort>,
    clock: Clock,
    log: LogFn,
    pending: Mutex<Option<Pending>>,
    /// The tray says the chord is unavailable and has not been reset yet.
    tooltip_flagged: AtomicBool,
}

impl ConfirmKeys {
    pub fn new(
        registry: Arc<dyn ShortcutRegistry>,
        sink: EventSink,
        tooltip: Arc<dyn TooltipPort>,
        clock: Clock,
        log: LogFn,
    ) -> Self {
        ConfirmKeys {
            registry,
            sink,
            tooltip,
            clock,
            log,
            pending: Mutex::new(None),
            tooltip_flagged: AtomicBool::new(false),
        }
    }

    fn lock(&self) -> MutexGuard<'_, Option<Pending>> {
        self.pending.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// A view was published: `shown` is its confirmation, if any.
    pub fn on_view(&self, shown: Option<Shown>) {
        let Some(shown) = shown else {
            self.release();
            return;
        };
        let want_chord = shown.chord && shown.spoken;
        let mut ops = Vec::new();
        {
            let mut pending = self.lock();
            match pending.as_mut() {
                Some(current) if current.id == shown.id => {
                    if current.answered {
                        return;
                    }
                    if want_chord && current.chord_armed_at.is_none() {
                        ops.push(Op::Register(Key::Chord));
                    } else if !want_chord && current.chord_armed_at.take().is_some() {
                        ops.push(Op::Unregister(Key::Chord));
                    }
                }
                _ => {
                    if let Some(mut old) = pending.take() {
                        ops.extend(old.held());
                    }
                    *pending = Some(Pending {
                        id: shown.id,
                        esc_registered: false,
                        chord_armed_at: None,
                        chord_held: false,
                        answered: false,
                    });
                    ops.push(Op::Register(Key::Esc));
                    if want_chord {
                        ops.push(Op::Register(Key::Chord));
                    }
                }
            }
        }
        self.run(shown.id, ops);
    }

    /// The island is hidden.
    pub fn on_island_hidden(&self) {
        self.release();
    }

    /// The app is quitting.
    pub fn quit(&self) {
        self.release();
    }

    /// A key event from the OS. Only a press counts, and only once per
    /// confirmation: a repeat of a held key finds the confirmation answered.
    /// A chord pressed inside the arm window may be a key held since before it
    /// was registered, so after one the chord needs a release before a press
    /// counts (the plugin reports every `WM_HOTKEY`, repeats included).
    pub fn press(&self, key: Key, state: KeyState) {
        if state == KeyState::Released {
            if key == Key::Chord
                && let Some(current) = self.lock().as_mut()
            {
                current.chord_held = false;
            }
            return;
        }
        let now = (self.clock)();
        let (event, ops) = {
            let mut pending = self.lock();
            let Some(current) = pending.as_mut() else {
                return;
            };
            if current.answered {
                return;
            }
            let id = current.id;
            let event = match key {
                Key::Esc if current.esc_registered => Event::Reject { id },
                Key::Chord => match current.chord_armed_at {
                    Some(armed_at) if now.saturating_sub(armed_at) >= ARM_MS => {
                        if current.chord_held {
                            return;
                        }
                        Event::Accept { id }
                    }
                    Some(_) => {
                        current.chord_held = true;
                        return;
                    }
                    None => return,
                },
                Key::Esc => return,
            };
            current.answered = true;
            (event, current.held())
        };
        // The answer goes first; releasing the keys can wait for the main thread.
        (self.sink)(event);
        for op in ops {
            self.apply(op);
        }
    }

    /// Forgets the confirmation and releases whatever it held.
    fn release(&self) {
        let ops = self.lock().take().map(|mut p| p.held()).unwrap_or_default();
        for op in ops {
            self.apply(op);
        }
    }

    /// Runs the operations for confirmation `id`, outside the lock, and records
    /// what the OS accepted. A refused chord rejects the confirmation.
    fn run(&self, id: u64, ops: Vec<Op>) {
        for op in ops {
            match op {
                Op::Unregister(key) => self.registry.unregister(key),
                Op::Register(Key::Esc) => match self.try_register(Key::Esc) {
                    Ok(()) => self.record(id, Key::Esc, |p| p.esc_registered = true),
                    Err(err) => {
                        (self.log)(format!("[niva] Esc is unavailable as a confirm key: {err}"))
                    }
                },
                Op::Register(Key::Chord) => match self.try_register(Key::Chord) {
                    Ok(()) => {
                        let now = (self.clock)();
                        self.record(id, Key::Chord, |p| {
                            p.chord_armed_at = Some(now);
                            p.chord_held = false;
                        });
                        if self.tooltip_flagged.swap(false, Ordering::SeqCst) {
                            self.tooltip.set_tooltip(DEFAULT_TOOLTIP);
                        }
                    }
                    Err(err) => self.chord_refused(id, &err),
                },
            }
        }
    }

    /// Fail closed: with no way to accept, the confirmation is rejected at once
    /// (the user hears "cancelled" instead of waiting for the timeout).
    fn chord_refused(&self, id: u64, err: &str) {
        (self.log)(format!(
            "[niva] Ctrl+Shift+Enter is unavailable, rejecting the confirmation: {err}"
        ));
        self.tooltip_flagged.store(true, Ordering::SeqCst);
        self.tooltip.set_tooltip(KEY_UNAVAILABLE_TOOLTIP);
        let ops = {
            let mut pending = self.lock();
            match pending.as_mut() {
                Some(current) if current.id == id && !current.answered => {
                    current.answered = true;
                    current.held()
                }
                _ => return,
            }
        };
        (self.sink)(Event::Reject { id });
        for op in ops {
            self.apply(op);
        }
    }

    fn apply(&self, op: Op) {
        if let Op::Unregister(key) = op {
            self.registry.unregister(key);
        }
    }

    /// Registers through the seam. A panic in the registry is a refusal: the
    /// key is unregistered best effort, so nothing stays registered that no
    /// bookkeeping will ever release.
    fn try_register(&self, key: Key) -> Result<(), String> {
        match catch_unwind(AssertUnwindSafe(|| self.registry.register(key))) {
            Ok(result) => result,
            Err(_) => {
                self.registry.unregister(key);
                Err("the shortcut registry failed".into())
            }
        }
    }

    /// Records a key the OS accepted for confirmation `id`. If that
    /// confirmation was released, answered or replaced while the registration
    /// ran (the lock is not held across it), nobody else would unregister the
    /// key, so it goes now.
    fn record(&self, id: u64, key: Key, change: impl FnOnce(&mut Pending)) {
        let recorded = match self.lock().as_mut() {
            Some(current) if current.id == id && !current.answered => {
                change(current);
                true
            }
            _ => false,
        };
        if !recorded {
            self.registry.unregister(key);
        }
    }
}

enum Op {
    Register(Key),
    Unregister(Key),
}

impl Pending {
    /// The unregistrations for what this confirmation holds, which it no
    /// longer holds afterwards.
    fn held(&mut self) -> Vec<Op> {
        let mut ops = Vec::new();
        if std::mem::take(&mut self.esc_registered) {
            ops.push(Op::Unregister(Key::Esc));
        }
        if self.chord_armed_at.take().is_some() {
            ops.push(Op::Unregister(Key::Chord));
        }
        ops
    }
}

/// A [`UiPort`] that keeps the keys in step with the views, then forwards the
/// event untouched.
pub struct KeysUi {
    inner: Arc<dyn UiPort<UiEvent>>,
    keys: Arc<ConfirmKeys>,
}

impl KeysUi {
    pub fn new(inner: Arc<dyn UiPort<UiEvent>>, keys: Arc<ConfirmKeys>) -> Self {
        KeysUi { inner, keys }
    }
}

impl UiPort<UiEvent> for KeysUi {
    fn publish(&self, event: UiEvent) {
        match &event {
            UiEvent::View(view) => self.keys.on_view(view.confirm.as_ref().map(|c| Shown {
                id: c.id,
                chord: c.chord,
                spoken: c.expires_at.is_some(),
            })),
            UiEvent::Island { visible: false } => self.keys.on_island_hidden(),
            UiEvent::Island { visible: true } => {}
        }
        self.inner.publish(event);
    }
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU64, Ordering};

    use niva_core::state_machine::{Effect, Phase, State, TimerKind, ToolVerdict, reduce};

    use super::*;
    use crate::ui_adapter::{ViewModel, ViewPayload};

    /// Blocks the next `register` of a key until the test lets it go.
    struct Blocker {
        key: Key,
        entered: std::sync::mpsc::Sender<()>,
        proceed: std::sync::mpsc::Receiver<()>,
    }

    /// Behaves like the plugin: registering a key that is already registered is
    /// an error, and unregistering one that is not does nothing.
    #[derive(Default)]
    struct FakeRegistry {
        on: Mutex<Vec<Key>>,
        log: Mutex<Vec<String>>,
        failing: Mutex<Vec<Key>>,
        /// `register` succeeds at the OS and then panics.
        panicking: Mutex<Vec<Key>>,
        blocker: Mutex<Option<Blocker>>,
    }

    impl FakeRegistry {
        fn heal(&self, key: Key) {
            self.failing.lock().unwrap().retain(|k| *k != key);
        }

        fn panic_after_registering(&self, key: Key) {
            self.panicking.lock().unwrap().push(key);
        }

        /// The next `register(key)` reports it entered, then waits for the
        /// returned sender.
        fn block_next(
            &self,
            key: Key,
        ) -> (std::sync::mpsc::Receiver<()>, std::sync::mpsc::Sender<()>) {
            let (entered, entered_rx) = std::sync::mpsc::channel();
            let (proceed_tx, proceed) = std::sync::mpsc::channel();
            *self.blocker.lock().unwrap() = Some(Blocker {
                key,
                entered,
                proceed,
            });
            (entered_rx, proceed_tx)
        }

        fn registered(&self) -> Vec<Key> {
            let mut on = self.on.lock().unwrap().clone();
            on.sort_by_key(|k| *k as u8);
            on
        }

        fn fail(&self, key: Key) {
            self.failing.lock().unwrap().push(key);
        }

        fn ops(&self) -> Vec<String> {
            self.log.lock().unwrap().clone()
        }
    }

    impl ShortcutRegistry for FakeRegistry {
        fn register(&self, key: Key) -> Result<(), String> {
            self.log.lock().unwrap().push(format!("register {key:?}"));
            if self.failing.lock().unwrap().contains(&key) {
                return Err("taken".into());
            }
            let blocker = {
                let mut slot = self.blocker.lock().unwrap();
                if slot.as_ref().is_some_and(|b| b.key == key) {
                    slot.take()
                } else {
                    None
                }
            };
            if let Some(blocker) = blocker {
                blocker.entered.send(()).unwrap();
                blocker.proceed.recv().unwrap();
            }
            {
                let mut on = self.on.lock().unwrap();
                if on.contains(&key) {
                    return Err("already registered".into());
                }
                on.push(key);
            }
            if self.panicking.lock().unwrap().contains(&key) {
                panic!("the registry blew up");
            }
            Ok(())
        }

        fn unregister(&self, key: Key) {
            self.log.lock().unwrap().push(format!("unregister {key:?}"));
            self.on.lock().unwrap().retain(|k| *k != key);
        }
    }

    #[derive(Default)]
    struct FakeTooltip(Mutex<Vec<String>>);

    impl TooltipPort for FakeTooltip {
        fn set_tooltip(&self, text: &str) {
            self.0.lock().unwrap().push(text.to_owned());
        }
    }

    struct Rig {
        keys: Arc<ConfirmKeys>,
        reg: Arc<FakeRegistry>,
        tip: Arc<FakeTooltip>,
        sent: Arc<Mutex<Vec<Event>>>,
        logs: Arc<Mutex<Vec<String>>>,
        now: Arc<AtomicU64>,
    }

    fn rig() -> Rig {
        let reg = Arc::new(FakeRegistry::default());
        let tip = Arc::new(FakeTooltip::default());
        let sent = Arc::new(Mutex::new(Vec::new()));
        let logs = Arc::new(Mutex::new(Vec::new()));
        let now = Arc::new(AtomicU64::new(10_000));
        let keys = Arc::new(ConfirmKeys::new(
            reg.clone(),
            {
                let sent = sent.clone();
                Arc::new(move |event| sent.lock().unwrap().push(event))
            },
            tip.clone(),
            {
                let now = now.clone();
                Arc::new(move || now.load(Ordering::SeqCst))
            },
            {
                let logs = logs.clone();
                Arc::new(move |line| logs.lock().unwrap().push(line))
            },
        ));
        Rig {
            keys,
            reg,
            tip,
            sent,
            logs,
            now,
        }
    }

    impl Rig {
        fn advance(&self, ms: u64) {
            self.now.fetch_add(ms, Ordering::SeqCst);
        }

        fn sent(&self) -> Vec<Event> {
            self.sent.lock().unwrap().clone()
        }

        fn press(&self, key: Key) {
            self.keys.press(key, KeyState::Pressed);
        }
    }

    fn close(id: u64) -> Shown {
        Shown {
            id,
            chord: false,
            spoken: true,
        }
    }

    fn power(id: u64, spoken: bool) -> Shown {
        Shown {
            id,
            chord: true,
            spoken,
        }
    }

    // --- registration ----------------------------------------------------------

    #[test]
    fn the_keys_are_esc_and_ctrl_shift_enter_and_nothing_else_maps() {
        assert_eq!(key_of(&shortcut(Key::Esc)), Some(Key::Esc));
        assert_eq!(key_of(&shortcut(Key::Chord)), Some(Key::Chord));
        assert_eq!(shortcut(Key::Esc).mods, Modifiers::empty());
        assert_eq!(shortcut(Key::Esc).key, Code::Escape);
        assert_eq!(
            shortcut(Key::Chord).mods,
            Modifiers::CONTROL | Modifiers::SHIFT
        );
        assert_eq!(shortcut(Key::Chord).key, Code::Enter);
        let toggle = Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::Space);
        assert_eq!(key_of(&toggle), None);
    }

    #[test]
    fn close_confirmation_registers_esc_only() {
        let r = rig();
        r.keys.on_view(Some(close(1)));
        assert_eq!(r.reg.registered(), vec![Key::Esc]);
    }

    #[test]
    fn restart_confirmation_registers_esc_immediately_and_chord_only_after_prompt_spoken() {
        let r = rig();
        r.keys.on_view(Some(power(7, false)));
        assert_eq!(r.reg.registered(), vec![Key::Esc]);
        r.keys.on_view(Some(power(7, true)));
        assert_eq!(r.reg.registered(), vec![Key::Esc, Key::Chord]);
        // The same view again registers nothing twice.
        r.keys.on_view(Some(power(7, true)));
        assert_eq!(
            r.reg
                .ops()
                .iter()
                .filter(|op| *op == "register Chord")
                .count(),
            1
        );
    }

    #[test]
    fn chord_pressed_before_registration_never_reaches_reducer() {
        let r = rig();
        r.keys.on_view(Some(power(7, false)));
        r.advance(5_000);
        r.press(Key::Chord);
        assert!(r.sent().is_empty());
        // And nothing is answered: Esc still works.
        r.press(Key::Esc);
        assert_eq!(r.sent(), vec![Event::Reject { id: 7 }]);
    }

    #[test]
    fn chord_within_500ms_of_arming_is_ignored_and_confirmation_stays_pending() {
        let r = rig();
        r.keys.on_view(Some(power(7, true)));
        r.advance(ARM_MS - 1);
        r.press(Key::Chord);
        assert!(r.sent().is_empty());
        assert_eq!(r.reg.registered(), vec![Key::Esc, Key::Chord]);
        // The early press was let go; a fresh press at 500 ms counts.
        r.keys.press(Key::Chord, KeyState::Released);
        r.advance(1);
        r.press(Key::Chord);
        assert_eq!(r.sent(), vec![Event::Accept { id: 7 }]);
    }

    #[test]
    fn arm_delay_counts_from_the_chord_registration_not_the_view() {
        let r = rig();
        r.keys.on_view(Some(power(7, false)));
        r.advance(5_000);
        r.keys.on_view(Some(power(7, true)));
        r.advance(100);
        r.press(Key::Chord);
        assert!(r.sent().is_empty());
    }

    // --- answers -----------------------------------------------------------------

    #[test]
    fn esc_press_emits_reject_and_unregisters_both() {
        let r = rig();
        r.keys.on_view(Some(power(7, true)));
        r.press(Key::Esc);
        assert_eq!(r.sent(), vec![Event::Reject { id: 7 }]);
        assert!(r.reg.registered().is_empty());
    }

    #[test]
    fn chord_press_emits_accept_and_unregisters_both() {
        let r = rig();
        r.keys.on_view(Some(power(7, true)));
        r.advance(ARM_MS);
        r.press(Key::Chord);
        assert_eq!(r.sent(), vec![Event::Accept { id: 7 }]);
        assert!(r.reg.registered().is_empty());
    }

    #[test]
    fn chord_on_close_confirmation_confirms_nothing() {
        let r = rig();
        r.keys.on_view(Some(close(3)));
        r.advance(5_000);
        r.press(Key::Chord);
        assert!(r.sent().is_empty());
        assert_eq!(r.reg.registered(), vec![Key::Esc]);
    }

    #[test]
    fn esc_with_nothing_pending_is_not_registered() {
        let r = rig();
        assert!(r.reg.registered().is_empty());
        r.keys.on_view(None);
        assert!(r.reg.registered().is_empty());
        r.press(Key::Esc);
        assert!(r.sent().is_empty());
    }

    #[test]
    fn stale_id_press_is_dropped_state_unchanged() {
        let r = rig();
        r.keys.on_view(Some(power(1, true)));
        r.keys.on_view(Some(close(2)));
        r.advance(5_000);
        // A chord press queued for the first confirmation finds only the
        // second, which has no chord: nothing is accepted, ever, under id 1.
        r.press(Key::Chord);
        assert!(r.sent().is_empty());
        r.press(Key::Esc);
        assert_eq!(r.sent(), vec![Event::Reject { id: 2 }]);
        // After the view cleared, an old press finds nothing.
        r.keys.on_view(None);
        r.press(Key::Esc);
        assert_eq!(r.sent().len(), 1);
    }

    #[test]
    fn second_press_after_answer_is_dropped() {
        let r = rig();
        r.keys.on_view(Some(close(1)));
        r.press(Key::Esc);
        r.press(Key::Esc);
        assert_eq!(r.sent(), vec![Event::Reject { id: 1 }]);
        // Republishing the answered confirmation's view does not re-arm it.
        r.keys.on_view(Some(close(1)));
        assert!(r.reg.registered().is_empty());
    }

    #[test]
    fn only_pressed_state_counts_autorepeat_ignored() {
        let r = rig();
        r.keys.on_view(Some(power(7, true)));
        r.advance(ARM_MS);
        r.keys.press(Key::Chord, KeyState::Released);
        assert!(r.sent().is_empty());
        r.press(Key::Chord);
        r.press(Key::Chord);
        r.press(Key::Chord);
        assert_eq!(r.sent(), vec![Event::Accept { id: 7 }]);
    }

    // --- every exit path ------------------------------------------------------------

    #[test]
    fn island_hidden_unregisters_both() {
        let r = rig();
        r.keys.on_view(Some(power(7, true)));
        r.keys.on_island_hidden();
        assert!(r.reg.registered().is_empty());
        r.advance(5_000);
        r.press(Key::Chord);
        r.press(Key::Esc);
        assert!(r.sent().is_empty());
    }

    #[test]
    fn quit_unregisters_both_shortcuts() {
        let r = rig();
        r.keys.on_view(Some(power(7, true)));
        assert_eq!(r.reg.registered().len(), 2);
        r.keys.quit();
        assert!(r.reg.registered().is_empty());
        r.keys.quit();
        assert!(r.reg.registered().is_empty());
    }

    #[test]
    fn view_with_no_confirm_or_other_id_unregisters() {
        let r = rig();
        r.keys.on_view(Some(power(1, true)));
        r.keys.on_view(None);
        assert!(r.reg.registered().is_empty());

        r.keys.on_view(Some(power(1, true)));
        r.keys.on_view(Some(close(2)));
        assert_eq!(r.reg.registered(), vec![Key::Esc]);
        assert!(
            r.reg.ops().iter().any(|op| op == "unregister Chord"),
            "the chord of id 1 was released"
        );
    }

    /// A reducer, a view model and the keys wired like the host, so every exit
    /// is a real one.
    struct Flow {
        rig: Rig,
        ui: KeysUi,
        state: State,
        vm: ViewModel,
        confirm_token: Option<u64>,
    }

    impl Flow {
        fn new() -> Self {
            let rig = rig();
            let ui = KeysUi::new(Arc::new(Sink), rig.keys.clone());
            let mut flow = Flow {
                rig,
                ui,
                state: State::default(),
                vm: ViewModel::new(),
                confirm_token: None,
            };
            flow.feed(Event::VoiceReadinessChanged { ready: true });
            flow.feed(Event::Shortcut);
            flow
        }

        fn feed(&mut self, event: Event) {
            let (state, fx) = reduce(self.state.clone(), event);
            self.state = state;
            for effect in &fx {
                if let Effect::StartTimer {
                    kind: TimerKind::Confirm,
                    token,
                } = effect
                {
                    self.confirm_token = Some(*token);
                }
            }
            for event in self.vm.update(&self.state, &fx, 0) {
                self.ui.publish(event);
            }
        }

        /// The events the keys sent so far reach the reducer.
        fn pump(&mut self) {
            let events: Vec<Event> = self.rig.sent.lock().unwrap().drain(..).collect();
            for event in events {
                self.feed(event);
            }
        }

        /// A confirmation of the given kind, its prompt spoken.
        fn confirm(&mut self, click_only: bool) {
            let id = self.state.listen_id.expect("a listen");
            self.feed(Event::Transcript {
                id,
                text: "do it".into(),
                lang: None,
            });
            let req = self.state.request_id.expect("a request");
            self.feed(Event::ToolCalls {
                req,
                verdict: ToolVerdict::Confirm {
                    prompt: "Sure?".into(),
                    click_only,
                    cancel_text: None,
                },
            });
            assert_eq!(self.state.phase, Phase::ConfirmationNeeded);
            let speak = self.state.speak_id.expect("the prompt is spoken");
            self.feed(Event::SpeakDone { id: speak });
            self.rig.advance(ARM_MS);
        }
    }

    struct Sink;

    impl UiPort<UiEvent> for Sink {
        fn publish(&self, _event: UiEvent) {}
    }

    #[test]
    fn no_shortcut_leaks_on_any_exit_path() {
        type Exit = fn(&mut Flow);
        let exits: [(&str, bool, Exit); 7] = [
            ("accept (chord)", true, |f| {
                f.rig.press(Key::Chord);
                f.pump();
            }),
            ("voice reject", false, |f| {
                let id = f.state.listen_id.expect("a confirm listen");
                f.feed(Event::Transcript {
                    id,
                    text: "no".into(),
                    lang: None,
                });
            }),
            ("Esc", false, |f| {
                f.rig.press(Key::Esc);
                f.pump();
            }),
            ("Ctrl+Shift+Space", true, |f| f.feed(Event::Shortcut)),
            ("timeout", true, |f| {
                let token = f.confirm_token.expect("the confirm timer runs");
                f.feed(Event::Watchdog {
                    kind: TimerKind::Confirm,
                    token,
                });
            }),
            ("quit", true, |f| f.rig.keys.quit()),
            ("island hidden", true, |f| f.rig.keys.on_island_hidden()),
        ];
        for (name, click_only, exit) in exits {
            let mut flow = Flow::new();
            flow.confirm(click_only);
            let expected = if click_only { 2 } else { 1 };
            assert_eq!(flow.rig.reg.registered().len(), expected, "{name}: armed");
            exit(&mut flow);
            assert!(
                flow.rig.reg.registered().is_empty(),
                "{name}: a shortcut leaked: {:?}",
                flow.rig.reg.registered()
            );
        }
    }

    // --- registration failures --------------------------------------------------------

    #[test]
    fn esc_registration_failure_is_logged_and_confirmation_continues() {
        let r = rig();
        r.reg.fail(Key::Esc);
        r.keys.on_view(Some(power(7, true)));
        assert!(!r.logs.lock().unwrap().is_empty(), "the failure is logged");
        assert!(r.sent().is_empty(), "the confirmation is not rejected");
        assert_eq!(r.reg.registered(), vec![Key::Chord]);
        r.advance(ARM_MS);
        r.press(Key::Chord);
        assert_eq!(r.sent(), vec![Event::Accept { id: 7 }]);
    }

    #[test]
    fn chord_registration_failure_emits_reject_and_sets_tooltip_confirm_key_unavailable() {
        let r = rig();
        r.reg.fail(Key::Chord);
        r.keys.on_view(Some(power(7, false)));
        assert!(r.sent().is_empty(), "nothing yet: the prompt is playing");
        r.keys.on_view(Some(power(7, true)));
        assert_eq!(r.sent(), vec![Event::Reject { id: 7 }]);
        assert_eq!(
            *r.tip.0.lock().unwrap(),
            vec![KEY_UNAVAILABLE_TOOLTIP.to_owned()]
        );
        assert!(!r.logs.lock().unwrap().is_empty());
        assert!(r.reg.registered().is_empty(), "Esc is released too");
        // Fail closed also means one answer: nothing else goes out.
        r.press(Key::Esc);
        assert_eq!(r.sent().len(), 1);
    }

    #[test]
    fn tooltip_is_reset_at_the_next_successful_chord_registration() {
        let r = rig();
        r.reg.fail(Key::Chord);
        r.keys.on_view(Some(power(7, true)));
        assert_eq!(
            *r.tip.0.lock().unwrap(),
            vec![KEY_UNAVAILABLE_TOOLTIP.to_owned()]
        );
        r.reg.heal(Key::Chord);
        r.keys.on_view(None);
        r.keys.on_view(Some(power(8, true)));
        assert_eq!(
            *r.tip.0.lock().unwrap(),
            vec![
                KEY_UNAVAILABLE_TOOLTIP.to_owned(),
                DEFAULT_TOOLTIP.to_owned()
            ]
        );
        // Later registrations leave the tooltip alone (the voice status owns it).
        r.keys.on_view(None);
        r.keys.on_view(Some(power(9, true)));
        assert_eq!(r.tip.0.lock().unwrap().len(), 2);
    }

    #[test]
    fn a_panicking_register_leaves_no_key_behind_and_fails_closed() {
        let r = rig();
        r.reg.panic_after_registering(Key::Esc);
        r.keys.on_view(Some(close(1)));
        assert!(r.reg.registered().is_empty(), "Esc was released");
        assert!(!r.logs.lock().unwrap().is_empty());

        let r = rig();
        r.reg.panic_after_registering(Key::Chord);
        r.keys.on_view(Some(power(7, true)));
        assert!(r.reg.registered().is_empty(), "nothing is left registered");
        assert_eq!(r.sent(), vec![Event::Reject { id: 7 }]);
    }

    // --- races ----------------------------------------------------------------------

    type ExitFn = fn(&Rig);

    /// Runs `view` on another thread, holds its `register(key)` open, runs
    /// `exit` meanwhile, then lets the register finish.
    fn exit_during_register(key: Key, shown: Shown, exit: impl FnOnce(&Rig)) -> Rig {
        let r = rig();
        let (entered, proceed) = r.reg.block_next(key);
        let keys = r.keys.clone();
        let worker = std::thread::spawn(move || keys.on_view(Some(shown)));
        entered.recv().unwrap();
        exit(&r);
        proceed.send(()).unwrap();
        worker.join().unwrap();
        r
    }

    #[test]
    fn release_while_esc_is_registering_does_not_leave_esc_registered() {
        let exits: [(&str, ExitFn); 3] = [
            ("island hidden", |r| r.keys.on_island_hidden()),
            ("no confirmation", |r| r.keys.on_view(None)),
            ("quit", |r| r.keys.quit()),
        ];
        for (name, exit) in exits {
            let r = exit_during_register(Key::Esc, close(1), exit);
            assert!(r.reg.registered().is_empty(), "{name}: Esc leaked");
        }
    }

    #[test]
    fn release_while_chord_is_registering_does_not_leave_chord_registered() {
        let exits: [(&str, ExitFn); 4] = [
            ("island hidden", |r| r.keys.on_island_hidden()),
            ("no confirmation", |r| r.keys.on_view(None)),
            ("quit", |r| r.keys.quit()),
            ("Esc answer", |r| r.press(Key::Esc)),
        ];
        for (name, exit) in exits {
            let r = exit_during_register(Key::Chord, power(7, true), exit);
            assert!(
                r.reg.registered().is_empty(),
                "{name}: leaked {:?}",
                r.reg.registered()
            );
        }
    }

    // --- a held chord ---------------------------------------------------------------

    #[test]
    fn chord_held_through_arming_is_ignored_until_released() {
        let r = rig();
        r.keys.on_view(Some(power(7, true)));
        // Held when it was registered: presses inside the guard window...
        r.advance(100);
        r.press(Key::Chord);
        // ...and the auto-repeat that follows past it.
        r.advance(ARM_MS);
        r.press(Key::Chord);
        r.advance(30);
        r.press(Key::Chord);
        assert!(r.sent().is_empty(), "a held key never answers");
        // Let go, press again: that is a fresh keystroke.
        r.keys.press(Key::Chord, KeyState::Released);
        r.press(Key::Chord);
        assert_eq!(r.sent(), vec![Event::Accept { id: 7 }]);
    }

    #[test]
    fn press_release_press_after_arming_is_accepted() {
        let r = rig();
        r.keys.on_view(Some(power(7, true)));
        r.advance(100);
        r.press(Key::Chord);
        r.keys.press(Key::Chord, KeyState::Released);
        r.advance(ARM_MS);
        r.press(Key::Chord);
        assert_eq!(r.sent(), vec![Event::Accept { id: 7 }]);
    }

    #[test]
    fn a_single_fresh_press_after_arming_is_accepted() {
        let r = rig();
        r.keys.on_view(Some(power(7, true)));
        r.advance(ARM_MS + 1);
        r.press(Key::Chord);
        assert_eq!(r.sent(), vec![Event::Accept { id: 7 }]);
    }

    // --- same id, chord withdrawn ---------------------------------------------------

    #[test]
    fn same_id_view_without_chord_unregisters_chord_and_keeps_esc() {
        let r = rig();
        r.keys.on_view(Some(power(7, true)));
        assert_eq!(r.reg.registered(), vec![Key::Esc, Key::Chord]);
        r.keys.on_view(Some(Shown {
            id: 7,
            chord: true,
            spoken: false,
        }));
        assert_eq!(r.reg.registered(), vec![Key::Esc]);
        r.advance(5_000);
        r.press(Key::Chord);
        assert!(r.sent().is_empty(), "the withdrawn chord answers nothing");
        r.press(Key::Esc);
        assert_eq!(r.sent(), vec![Event::Reject { id: 7 }]);
    }

    // --- following the views ----------------------------------------------------------

    #[derive(Default)]
    struct Recorder(Mutex<Vec<UiEvent>>);

    impl UiPort<UiEvent> for Recorder {
        fn publish(&self, event: UiEvent) {
            self.0.lock().unwrap().push(event);
        }
    }

    fn view(confirm: Option<Shown>) -> UiEvent {
        use crate::ui_adapter::ConfirmView;
        UiEvent::View(ViewPayload {
            state: "confirmation-needed",
            caption: None,
            confirm: confirm.map(|s| ConfirmView {
                id: s.id,
                prompt: "Sure?".into(),
                confirm_label: "Confirm",
                cancel_label: "Cancel",
                expires_at: s.spoken.then_some(1),
                chord: s.chord,
            }),
            reminder: None,
            countdown: None,
        })
    }

    #[test]
    fn keys_ui_forwards_view_and_registers_from_view_sequence() {
        let r = rig();
        let inner = Arc::new(Recorder::default());
        let ui = KeysUi::new(inner.clone(), r.keys.clone());

        ui.publish(UiEvent::Island { visible: true });
        ui.publish(view(Some(power(7, false))));
        assert_eq!(r.reg.registered(), vec![Key::Esc]);
        ui.publish(view(Some(power(7, true))));
        assert_eq!(r.reg.registered(), vec![Key::Esc, Key::Chord]);
        ui.publish(UiEvent::View(ViewPayload::idle()));
        assert!(r.reg.registered().is_empty());

        assert_eq!(inner.0.lock().unwrap().len(), 4, "nothing was swallowed");
    }

    #[test]
    fn keys_ui_unregisters_on_island_hidden() {
        let r = rig();
        let inner = Arc::new(Recorder::default());
        let ui = KeysUi::new(inner.clone(), r.keys.clone());
        ui.publish(view(Some(power(7, true))));
        assert_eq!(r.reg.registered().len(), 2);
        ui.publish(UiEvent::Island { visible: false });
        assert!(r.reg.registered().is_empty());
        assert_eq!(inner.0.lock().unwrap().len(), 2);
    }
}
