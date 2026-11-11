# Proposal: Living PC notch

## Intent

Replace "a robot lives in my PC" with "my PC is alive". When Niva is summoned, a small black notch drops from the top center and a mirrored spike waveform moves with the sound, colored by state. Niva answers by voice only: no reply text, no buttons. Supersedes niva-mvp's criterion "The mascot visibly shows all 8 states" (niva-mvp files stay unedited). The Belkys voice work on `feat/3g-voice-belkys` is separate; this change lands on top of it.

## Owner decisions (2026-11-10)

| # | Decision |
|---|---|
| Look | 210x36 px notch (widened from 118x30 by owner decision 2026-11-11, see "Widened notch" below), ~17 px bottom corners, one solid mirrored spike waveform (superseded the 7-bar prototype, see "Visual rework" below), spring drop-in. Idle hidden. |
| States | Color only tells states apart: listening #378ADD (calm breathing line, not level-driven); thinking #7F77DD; speaking #5DCAA5 (voice level); confirm #EF9F27; success #97C459, then hide; error #E24B4A, then hide; reminder #F0997B. Only speaking is level-driven (Niva's voice); every other state, listening included, is a calm, barely breathing flat line. |
| Confirm | Voice yes/no as today; timeout cancels; Esc cancels (global shortcut, registered only while a confirmation is pending); Ctrl+Shift+Space still rejects |
| D1 | Restart/shutdown: Ctrl+Shift+Enter within the window replaces the click (registered only while pending). Speech never confirms them. |
| D2 | Acting = thinking wave; Countdown = amber pulse; speaking overrides the phase look while TTS plays |
| D3 | Hide = settle only, no reading hold; settle time 1.5 s (down from 3 s) |
| D4 | Voice down: error color + host chime only |
| D6 | Reminder: audio only, no text anywhere (no tray item); the coral pulse ends with the speech |
| D5 | `level` = 7 bands, Niva's TTS only; UI simulation of the voice if no level for >150 ms while speaking |
| A11y | Reduced motion is required (static shape). The per-state non-color motion/silhouette cue is removed by owner decision (see "Visual rework"); the voice and the hidden status text remain the non-color channels |

## Visual rework (2026-11-10)

After seeing the 7-bar notch live, the owner rejected the bars and chose, from chat prototypes, "mirrored spikes, solid": one solid shape mirrored around the horizontal center line whose spikes move with the level. This supersedes the 7-bar decisions above and in the first version of the design (L1, L2, the motion and silhouette columns). Only the color distinguishes states, by explicit owner choice; the requirement for a non-color motion/silhouette cue per state is removed. Voice and the hidden status text remain the non-color channels. Geometry (118x30, window 200x70; widened to 210x36 and 290x76 on 2026-11-11), drop-in/out, click-through, levels wire (7 bands) and the 150 ms fallback are unchanged.

## Widened notch (2026-11-11)

Owner decision: make the notch wider "so it's noticeable that the assistant peeked out when called". Chosen size 210x36 CSS px (was 118x30), bottom corner radius ~0.47 x height (17 px). The waveform algorithm is unchanged (N=30, envelope, amplitudes relative to the 236x60 drawing space, which is now scaled non-uniformly to the notch). The window grows to 290x76 so the 40 px side and bottom room of design L4 still hold.

## Level source decision (2026-11-10, after the live test)

The owner, after the live test: "the wave must move with Niva's voice, not with me". So the listening look is NOT level-driven: it shows the same calm breathing line as thinking and confirmation, in the listening color #378ADD, with no microphone level and no simulated voice. Only `speaking` follows a level, and that level is Niva's TTS output. The sidecar `level` message is TTS-only: the microphone tap and `src:"mic"` are dropped. This is also simpler (no capture-side code) and more private (no microphone-derived signal ever leaves the sidecar). Supersedes the "mic level" parts of D5, the states row and the earlier notch/speech-io level requirements.

## Scope

**In:** notch UI and window shrink; trimmed view-model (no caption, add `speaking`); sidecar `level` message; Esc and Ctrl+Shift+Enter confirmation; removal of mascot, buttons, reading time, cursor poller.

**Out:** conversation memory (BACKLOG "Continuous conversation"), WhatsApp, Piper voice swap (BACKLOG robot voice item), voice-down text echo (BACKLOG "Island text for a voice failure").

## Capabilities

### New Capabilities
- `notch`: color per state, waveform rendering, level sources, simulation fallback, reduced motion

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
2. Notch UI with simulated waveform + window shrink
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

- [ ] Notch is 210x36 px; window ~290x76, non-focusable, transparent
- [ ] The waveform tracks level on the onset (rise) only: mean lag 50 ms or less and no sample above 100 ms (mic and TTS, built-in or wired output; Bluetooth excluded). The decay is intentionally slower and unbounded
- [ ] Reduced motion shows a static waveform (no random re-targeting)
- [ ] Tests: Esc and Ctrl+Shift+Enter registered only while pending, unregistered on confirm, reject, timeout and quit
- [ ] No regression in E1 eval or end-of-speech latency (< 2.6 s)

## Resolved questions

1. Spoken-length cap for search summaries: deferred; pairs with conversation-memory work. Long replies are spoken in full.
2. Fullscreen reminder content: chime and pulse only, no text, no tray item. Deferred: speak a missed reminder after fullscreen ends.
3. Level bound: revised in design (L14), see success criteria.
4. Slice order: the chord ships first (slice 1); the buttons are removed in slice 4.
