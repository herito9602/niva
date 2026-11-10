# Exploration: living-pc-notch

Owner request (2026-11-10): replace the robot mascot ("a robot lives in my PC") with "my PC is alive". When summoned, a small notch drops at the top center with equalizer bars that move with the sound and change color by state. No reply text: voice only, no confirm buttons. Visual reference: the chat prototype (118x30 px black notch, 7 rounded bars; listening #378ADD, thinking #7F77DD wave, speaking #5DCAA5, confirm #EF9F27 pulse, success #97C459 bounce, error #E24B4A jitter, reminder #F0997B outward pulse; idle hidden). The voice change to es-CU-BelkysNeural without the robot effect is separate (branch feat/3g-voice-belkys).

## Answer first

- **Structure:** a separate change `living-pc-notch`. Its delta specs reference `niva-mvp`: they remove `mascot`, modify `island-shell`, `fullscreen-policy`, `action-safety`, `reminders` and `speech-io`, and add a new `notch` capability.
- **Audio levels:** add one sidecar message `level`, throttled to ~30 Hz. A UI-only simulation is the fallback, and it ships first as a vertical slice.
- **Blocking decision (D1):** `restart` and `shutdown` can only be confirmed by click (`PowerKind::click_only`, `tools.rs:134`). Removing the buttons removes their only confirmation channel.
- **Esc:** feasible as a temporary global shortcut. Ctrl+Shift+Space already rejects a pending confirmation.
- **Two regressions need a call:** the voice-down degraded mode, and fullscreen reminders with no readable content.

## Current state

**Frontend (`app/src`)**
- `App.tsx` renders `island-ui/Island.tsx`, an SVG path hanging from y=0 with a 124 px `MascotCanvas`, a text label and `ConfirmButtons.tsx`.
- `layout.ts` sizes the pill to its text: `computeLayout`, `MAX_ISLAND_WIDTH=560`, `MAX_ISLAND_HEIGHT=182`, `MASCOT_SIZE=124` and the button constants.
- `viewStore.ts` mirrors the `niva://view`, `niva://island` and `niva://cursor` events. `island.ts` holds the state labels and the countdown text.
- The mascot lives in `app/src/mascot/*`. Other parts that depend on it: `island-ui/{edge,choreography,scroll,shape,springs,confirmUi,confirmCommands}.ts` and `app/mascot-preview.html`.
- The debug window has an 8-state switcher (`STATE_SCRIPTS`, `ALL_STATES`) that drives the reducer.

**Window (`tauri.conf.json`)**
- One window, label `island`: 640x232, transparent, always on top, `focusable:false`, no shadow.
- Click-through (`click_through.rs`) ignores the mouse everywhere except the two button rectangles. The island reports those rectangles through `confirm_buttons.rs` (`ConfirmGate`, `ARM_MS=500`, capability `island-actions`).
- A cursor poller (`cursor.rs`, ~30 Hz) feeds the mascot gaze and the hit-test.
- `HIDE_DELAY_MS=540` is tied to `EDGE_OUT_MS` by a contract test.

**Host view-model (`ui_adapter.rs`)**
- `ViewPayload` carries `state`, `caption`, `confirm`, `reminder` and `countdown`. `caption` is the reply text.
- The phase names are the 8 mascot states, and `Countdown` maps to `acting`. There is no "speaking" flag.

**Core (`state_machine.rs`)**
- Phases: `Idle, Listening, Thinking, Acting, ConfirmationNeeded, Success, Error, ReminderAlert, Countdown`. `ReplyKind` is `Final | ConfirmPrompt | AskFollowUp | Alert | Countdown`. Speech is tracked by `speak_id`.
- Timers: confirm 10 s, settle 3 s, alert end 30 s, countdown 10 s.
- `reading.rs` stretches the hide delay to give time to read: clamp(1.5 s + 55 ms per char, 2.5 s, 12 s).
- The shortcut already rejects a pending confirmation (`on_shortcut`, `state_machine.rs:898`).

**Voice confirm**
- The voice path exists (`ListenMode::Confirm`). It is skipped when `confirm_click_only` is set or voice is unavailable. The 10 s timer cancels.

**Audio signals**
- None today; the protocol has no level message.
- Mic frames are 512 samples at 16 kHz, one every 32 ms, through `Capture.on_frame` (`capture.py:226`).
- TTS PCM goes through `Speaker._write` (`tts.py:655`), after the optional effect.
- The latency and eval tooling do not reference island, caption or mascot.

## Affected areas

- **Delete:** `app/src/mascot/**` and its tests; `island-ui/{layout,edge,choreography,scroll,shape,springs,confirmUi,confirmCommands,ConfirmButtons,preview}`; `island.ts`; `app/mascot-preview.html`.
- **Replace:** `Island.tsx` becomes a `Notch` component with 7 bars, driven by refs and rAF rather than React state. `styles.css` and the `viewStore` snapshot change with it.
- **Window:** `tauri.conf.json` goes from 640x232 to roughly 200x70, keeping `focusable:false` and `transparent:true`. Rewrite `island_contract_tests.rs` and give `HIDE_DELAY_MS` a new value tied to the new exit animation.
- **Host, unless D1 keeps the buttons:** remove `confirm_buttons.rs`, the `island-actions` capability and its test, the click-through hit-test and the cursor poller.
- **View-model:** drop `caption`, `confirm.prompt`, the labels and `countdown.text` from the island payload, and add a `speaking` flag. About 30 tests pin captions.
- **Core:** `reading.rs` and its use in `effect_runner.rs` go, leaving plain settle. The reducer phases stay, since the owner declined collapsing states earlier.
- **Sidecar:** `level` in `voice/protocol.py`, a mic tap in `capture.py` and a tap in `tts.py` `_write`. `wire.rs` and `translate.rs` parse it, and a host sink emits `niva://level` straight to the island, not through the reducer.
- **Debug window:** keep the 8-state switcher and add level injection.

## Spec impact (niva-mvp)

| Capability | Impact |
|---|---|
| `mascot` | REMOVED entirely |
| `island-shell` | REMOVED: fits-message, reading time, confirmation buttons, countdown text, edge animation. MODIFIED: tiny notch window that is always click-through. Unchanged: tray, shortcut, show/hide, Type window (input only) |
| `fullscreen-policy` | MODIFIED: a reminder shows the coral pulse and plays the chime; no text is shown |
| `action-safety` | MODIFIED: the click confirmation and the button guard go, depending on D1 |
| `reminders` | MODIFIED: the reminder is spoken, not shown |
| `speech-io` | ADDED: `level` message |
| `notch` (new) | Palette and motion per state, level sources, reduced motion, accessibility |
| proposal success criterion "mascot shows all 8 states" | Superseded; note it in the new proposal and leave niva-mvp unedited |

## Structuring the change

| Option | Pros | Cons |
|---|---|---|
| **A. Separate `living-pc-notch`, delta specs referencing niva-mvp** (recommended) | Clean review and rollback; the verify reports and approved history stay valid; one concern per PR | `openspec/specs/` is empty, so the deltas must name niva-mvp spec paths and requirement titles; archive niva-mvp first |
| B. Amend niva-mvp in place | One source of truth | Rewrites verified work; mixes a visual pivot with MVP closure |

## Audio levels for the bars

| Approach | Cost and latency | Verdict |
|---|---|---|
| **1. Sidecar `level` message** (7 bands 0-100 from the per-frame rfft for the mic, 30-40 ms windows in `Speaker._write` for TTS) | ~30 lines/s of ~60 bytes. Mic lag is under 40 ms. TTS levels lead the audible sound by the device latency (~100-200 ms), so they need a constant UI delay or a playback-clocked emitter, measured. | Recommended. Only numbers leave the sidecar; nothing is emitted while idle. |
| 2. UI simulation from state | Zero protocol work, but the bars don't react to real sound | First slice, and the fallback when no `level` arrives for more than 150 ms |
| 3. Host WASAPI peak meter | Heavy and duplicates what the sidecar knows | Rejected |

## What depends on reply text

| Case | Without text |
|---|---|
| Normal reply | Speech only, fine |
| Follow-ups (`AskWhichApp`, `AskReminderTime`) | Spoken question, then a normal listen; works by voice |
| Search summaries (up to ~400 chars) | Spoken in full, about 25 s; consider a spoken-length cap. Barge-in still stops it |
| Voice down (mic error or sidecar down) | The typed-command path goes completely silent: the biggest hole. Needs D4 |
| Muted PC or TTS failure | The reply is lost silently; accept and document |
| Fullscreen reminder | Chime and coral pulse; the content is not available until later |
| Accessibility | All text output is lost. Color must not be the only cue: give each state a distinct motion, plus static patterns under reduced motion |

## Confirm without buttons

- **Voice:** works today, and a timeout cancels.
- **Esc:** the window can't take focus, so use `tauri_plugin_global_shortcut`. Register Esc only while `ConfirmationNeeded` and send `Event::Reject{id}`. The risk is taking Esc from the foreground app for up to 10 s, or leaking the registration; test the register/unregister lifecycle.
- **Ctrl+Shift+Space:** already rejects.

## Decisions needed

| # | Question | Recommended default |
|---|---|---|
| **D1** | Restart and shutdown can only be confirmed by click. How are they confirmed without buttons? (a) Niva declines by voice and says to use Windows; (b) confirm by voice, which breaks the "ambient speech must not confirm" rule; (c) a key chord within the 10 s window | (a) now, revisit (c) later, never (b) |
| D2 | Looks for `Acting`, `Countdown` and speaking | Acting reuses the thinking wave, Countdown the amber pulse; while TTS plays, speaking overrides the phase look |
| D3 | Hide timing | Settle only, probably under 3 s |
| D4 | What happens when voice is down | Error color plus chime only in the first slice |
| D5 | Level payload | 7 bands |

## Risks

- D1 left unresolved would weaken the click-only safety gate. Keep any `action-safety` change in its own PR.
- Tests pin the old behavior: `island_contract_tests.rs`, `capabilities_tests.rs`, about 30 `ui_adapter.rs` caption tests and 9 vitest files. Replacements must keep coverage of window size, focus, transparency and hide delay.
- Level protocol: flooding, and TTS levels running ahead of the sound. Measure the lead, and fall back to the simulation after 150 ms without a `level`.
- The Esc global shortcut could steal the key or leak its registration.
- Accessibility regression and the silent voice-down mode.
- This must land on top of `feat/3g-voice-belkys` or after it merges.

## Suggested slices

1. Notch UI with simulated bars, and the window shrink.
2. Trim the view-model and island-shell.
3. Sidecar `level` message.
4. Esc and confirmation cleanup.
5. The `action-safety` change for D1, in its own PR.
