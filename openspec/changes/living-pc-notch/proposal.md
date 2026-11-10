# Proposal: Living PC notch

## Intent

Replace "a robot lives in my PC" with "my PC is alive". When Niva is summoned, a small black notch drops from the top center and its equalizer bars move with the sound, colored by state. Niva answers by voice only: no reply text, no buttons. Supersedes niva-mvp's criterion "The mascot visibly shows all 8 states" (niva-mvp files stay unedited). The Belkys voice work on `feat/3g-voice-belkys` is separate; this change lands on top of it.

## Owner decisions (2026-11-10)

| # | Decision |
|---|---|
| Look | 118x30 px notch, ~14 px bottom corners, 7 rounded bars, spring drop-in. Idle hidden. |
| States | listening #378ADD mic level; thinking #7F77DD travelling wave; speaking #5DCAA5 voice level; confirm #EF9F27 slow pulse; success #97C459 bounce, then hide; error #E24B4A low jitter, then hide; reminder #F0997B outward pulse |
| Confirm | Voice yes/no as today; timeout cancels; Esc cancels (global shortcut, registered only while a confirmation is pending); Ctrl+Shift+Space still rejects |
| D1 | Restart/shutdown: Ctrl+Shift+Enter within the window replaces the click (registered only while pending). Speech never confirms them. |
| D2 | Acting = thinking wave; Countdown = amber pulse; speaking overrides the phase look while TTS plays |
| D3 | Hide = settle only, no reading hold; settle time 1.5 s (down from 3 s) |
| D4 | Voice down: error color + host chime only |
| D6 | Reminder: audio only, no text anywhere (no tray item); the coral pulse ends with the speech |
| D5 | `level` = 7 bands; UI simulation if no level for >150 ms |
| A11y | Reduced motion and a non-color cue per state are required |

## Scope

**In:** notch UI and window shrink; trimmed view-model (no caption, add `speaking`); sidecar `level` message; Esc and Ctrl+Shift+Enter confirmation; removal of mascot, buttons, reading time, cursor poller.

**Out:** conversation memory (BACKLOG "Continuous conversation"), WhatsApp, Piper voice swap (BACKLOG robot voice item), voice-down text echo (BACKLOG "Island text for a voice failure").

## Capabilities

### New Capabilities
- `notch`: palette and motion per state, level sources, simulation fallback, reduced motion, non-color cues

### Modified Capabilities
- `mascot`: REMOVED
- `island-shell`: tiny, always click-through notch; fits-message, reading time, buttons, countdown text and edge animation removed
- `fullscreen-policy`: reminder = coral pulse + chime, no text
- `action-safety`: click confirmation replaced by Esc and Ctrl+Shift+Enter
- `reminders`: spoken, not shown
- `speech-io`: `level` message (~30 Hz, none while idle)

## Approach

Exploration's recommendation: `Notch` component driven by refs + rAF; host sink emits `niva://level` straight to the window, bypassing the reducer. Reducer phases stay. Shortcuts register on entering `ConfirmationNeeded` and unregister on every exit.

## Delivery slicing

As in design.md:

1. Confirm keys (Esc + chord, D1), own safety PR, buttons still present
2. Notch UI with simulated bars + window shrink
3. View-model and core trim
4. Host cleanup (buttons, hit-test, cursor, capability), own PR
5. Sidecar `level`

## Risks

| Risk | Likelihood | Mitigation |
|---|---|---|
| Esc/chord stolen or leaked | Med | Lifecycle tests on every exit path |
| TTS level leads audio 100-200 ms | High | Playback-clock pacer in the sidecar (design L13); measure |
| Silent voice-down, lost a11y text | Med | D4 error color and host chime; non-color cues |
| Pinned tests (~30 caption, 9 vitest) | High | Keep size/focus/transparency/hide coverage |

## Rollback Plan

Revert in reverse slice order. Restart/shutdown buttons and `confirm_buttons.rs` are removed only in slice 4, after the chord (slice 1) has shipped; once the buttons are removed, the chord slice is never reverted alone, so the power actions are never left without a confirmation path.

## Success Criteria

- [ ] Notch is 118x30 px; window ~200x70, non-focusable, transparent
- [ ] Bars track level: mean lag 50 ms or less and no sample above 100 ms (mic and TTS, built-in or wired output; Bluetooth excluded)
- [ ] Each state is distinguishable in grayscale and with reduced motion
- [ ] Tests: Esc and Ctrl+Shift+Enter registered only while pending, unregistered on confirm, reject, timeout and quit
- [ ] No regression in E1 eval or end-of-speech latency (< 2.6 s)

## Resolved questions

1. Spoken-length cap for search summaries: deferred; pairs with conversation-memory work. Long replies are spoken in full.
2. Fullscreen reminder content: chime and pulse only, no text, no tray item. Deferred: speak a missed reminder after fullscreen ends.
3. Level bound: revised in design (L14), see success criteria.
4. Slice order: the chord ships first (slice 1); the buttons are removed in slice 4.
