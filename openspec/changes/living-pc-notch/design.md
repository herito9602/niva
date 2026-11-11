# Design: Living PC notch

## Technical Approach

The reducer and its phases stay. Three things change around them:

- The island webview becomes a 118x30 notch with one mirrored spike waveform on a canvas, animated by one rAF loop that writes no React state.
- Audio levels go from the sidecar to the window as `niva://level`, without passing through the reducer.
- Confirmation loses the buttons and gains two temporary global shortcuts, owned by a host `ConfirmKeys` decorator that replaces `ConfirmGate`.

The safety logic is unchanged: the chord produces the same `Event::Accept{id}` that the click did.

Answers to the proposal's open questions are in "Open questions resolved".

## Decisions

| # | Topic | Choice | Rejected | Rationale |
|---|---|---|---|---|
| L1 | Waveform rendering (reworked 2026-11-10) | One Canvas 2D (DPR-aware, backing store 118x30 times devicePixelRatio) filling the notch, drawn by the existing rAF loop (`loop.ts`). One solid shape mirrored around the center line, drawn in a 236x60 space and scaled: x from 7% to 93% of the width, flat tails to 2.8% / 97.2% on the center line. The top edge is a polyline of N=30 points; point i (u=i/29) has half-height `clamp(cur[i]*amp, 0.6, H/2-1.5)` with `amp = 1 + level*24`; the bottom edge mirrors it. Fill is the state color, opaque; `lineJoin` miter, `miterLimit` 3. | The 7 DOM bars (owner rejected them live); React state per frame | The owner chose this look from prototypes. A single canvas redraw per frame is cheap at 118x30 and the loop writes no React state. |
| L2 | Frame logic | Pure `wave.ts`. Every ~70 ms (`RETARGET_MS`) new targets `tgt[i] = env(u) * (0.2 + 0.8*r*r*1.3)` with `r` from an injectable RNG (`Math.random` in production, a seeded RNG in tests) and `env(u) = exp(-((u-0.5)/0.22)^2)`. Every frame `cur[i] += (tgt[i]-cur[i]) * 0.45`. Level smoothing `lvl += (target-lvl) * (target > lvl ? 0.4 : 0.1)`. These constants are per frame, exactly as in the owner-approved prototype (which also ran per display frame), so motion scales with the refresh rate by design: faster displays respond sooner and settle faster. The onset bound of L14 holds at 60 Hz and only gets shorter above it (pinned in `wave.test.ts` at 60 to 240 Hz). Level source (owner decision 2026-11-10: the wave moves with Niva's voice, not with the user): only speaking is level-driven, by the TTS level (mean of the 7 bands / 100); a level older than 150 ms, or absent, falls back to the simulated voice envelope (syllable bursts with gaps) for that look only. Every other look, listening included, has level 0 and "breathes": an effective level of about 0.05, so the line shows tiny random spikes and stays calm. Silence is an almost flat thin line (half-height 0.6). | Smoothing in the sidecar | Pure and seedable, so vitest can test it. The UI owns the feel; the wire carries raw bands. |
| L3 | Drop-in/out | Drop-in is `translateY(-100%)` to 0 over 320 ms with an overshoot curve `cubic-bezier(.34,1.56,.64,1)`. Drop-out is 200 ms ease-in. `NOTCH_OUT_MS = 200` in `notch/motion.ts`. `HIDE_DELAY_MS = 240` (exit plus 40 ms slack). | Keeping 540 ms | The window hides right after the exit. A contract test pins `OUT <= HIDE <= OUT + 100`. Reduced motion uses a 120 ms fade, which stays below the same delay. |
| L4 | Window | 200x70, transparent, `focusable:false`, no decorations, no shadow, always on top. Always click-through: `set_ignore_cursor_events(true)` on prepare and on every show. | Keeping a hit-test | Nothing on the notch is clickable. The 41 px side room and 40 px bottom room fit the reminder's outward pulse and the overshoot. |
| L5 | View payload | `{state, speaking, confirm?: {id, expiresAt?, chord}, reminder?: {count}}`. Removed: `caption`, `prompt`, labels, `countdown.text`. `Countdown` gets its own name, `"countdown"`. | Keeping `caption` for the debug window | The island must not carry reply text. In debug builds, the reply text goes to the dev log (`DEV_LOG_EVENT`): one line per `Speak`. |
| L6 | `speaking` | New reducer field `State::audible_speak: Option<u64>`. Set on `SpeakStarted{id}` when `id == speak_id`. Cleared wherever `speak_id` clears. `speaking = audible_speak.is_some()`. | `speak_id.is_some()` (green during the ~0.9 s of synthesis); the frontend inferring it from TTS levels (fails under the simulation) | The reducer stays the single, pure source. The `SpeakStarted` row of the coverage matrix changes from ignore to a state-only update. |
| L7 | Hide timing | Delete `reading.rs` and `ReadingHold` in `effect_runner.rs`. `TimerKind::Settle` goes from 3 s to 1.5 s (owner-accepted). `settle_delay` moves out of `reading.rs` (it is imported at `effect_runner.rs:41`) into `effect_runner.rs` before the file is deleted. The 900 ms "acting stays visible" hold is kept in `effect_runner.rs` and is not part of the deleted reading hold. | Keeping 3 s | Nobody reads any more. 1.5 s covers the success bounce (600 ms) plus a beat. |
| L8 | Confirm channel | The chord sends `Event::Accept{id}` and Esc sends `Event::Reject{id}`. `confirmation_input` maps them to `ConfirmInput::Click`, as today. `safety::Channel::Click` (the safety gate's channel, not latency's `Channel`) becomes `Channel::Physical`: a mechanical rename, and the doc names the chord and the debug button. Only physical inputs can produce `Event::Accept` for click-only confirmations: the chord, and the debug window's button in debug builds only. Typed text and model output can never build `Accept`. | New `ConfirmInput::Key` / `Channel::Key` | The reducer and `SafetyGate` stay untouched. The guarantee is the same: an input the model and typed text cannot produce. `confirm_click_only` already makes the reducer ignore transcripts, so ambient speech never confirms a power action. |
| L9 | Shortcut lifecycle | `confirm_keys.rs`: a `KeysUi` decorator of `UiPort` (this replaces `GateUi`) over a `ShortcutRegistry` trait. Registration follows the published view (see "Key lifecycle"). | Hooking reducer effects | The host already learns about confirmations from views. A fake registry tests every exit path. |
| L10 | Registration failure | Esc taken: log it and carry on (voice, timeout and Ctrl+Shift+Space still cancel). Chord taken (the late registration, once the prompt has been spoken): send `Event::Reject{id}` (the user hears "Cancelled"), log it, and set the tray tooltip to "Confirm key unavailable". | Leaving a confirmation with no way to accept | Fail closed. The user hears "Cancelled" rather than wait 10 s for nothing. |
| L11 | Level wire | `{"type":"level","src":"tts","bands":[7 ints 0-100]}`; the message is TTS-only (owner decision 2026-11-10): there is no microphone tap and `src:"mic"` is dropped as unknown, which is simpler and more private (no microphone-derived signal leaves the sidecar). `wire.rs` drops a message with any other length or any value outside 0-100 (as the speech-io spec requires). `Translated` gains `level: Option<Level>`. A `LevelSink` emits `niva://level` to `island` only. It is not logged and does not go through the dispatcher. | Through the reducer | At 30 Hz, it would flood the dispatcher and the view diffing. |
| L12 | Bands | One pure function, `voice/levels.py` `bands(pcm, rate)`: Hann window, rfft, 7 log bands with edges 80/160/320/640/1280/2560/4000/8000 Hz, band RMS in dBFS mapped from [-60, -10] to 0-100. TTS only: each written chunk is cut into 32 ms slices. Rate guard: at most one emit per 25 ms. | Peak meter only | 7 bands give a voice-like level and cost under 1 ms per frame. |
| L13 | TTS lead | The sidecar paces its emits to the playback clock. Each slice gets `due = max(prev_due + slice_ms, write_return + latency)`, where `latency` is `stream.latency`. The first write anchors the clock and an underrun re-anchors it. A pacer thread emits each slice when it is due. The pacer is flushed and stopped on speech end and on barge-in, so levels stop within one period (about 33 ms). `NIVA_TTS_LEVEL_OFFSET_MS` (default 0) covers latency the device does not report. The `AudioOutput` protocol (`voice/audio_io.py`) is extended with `latency`, and tests get a fake implementation. | A per-device constant delay in the UI (needs calibration on every PC, drifts after an underrun) | It is self-correcting, and the host and UI need no knowledge of devices. |
| L14 | Level bound | Revised: onset (rise) only: mean lag of 50 ms or less and no sample above 100 ms, for TTS, on built-in or wired output. Bluetooth is excluded (its latency is not reported) and uses the offset. Measurement: debug "level test" clicks every 500 ms, filmed at 240 fps, measuring the rise of the spikes; a fake-clock unit proxy of the onset lag also runs in CI. The decay is intentionally slower (release 0.1 per frame) and unbounded. | 50 ms flat | The onset is what reads as "in sync"; a slow release is the look the owner approved. The TTS path is paced to playback (L13), so the same bound applies; a strict 50 ms cap on a raw sample would fail by construction. |
| L15 | Voice-down chime (D4) | When the sidecar or mic is down, the host plays the chime from `audio_cue.rs` (the existing host cue module) while the view shows `error`. No sidecar involvement. | A chime synthesized by the sidecar | The sidecar is the thing that is down. |
| L16 | Exclusive fullscreen | When the visibility policy reports exclusive fullscreen (the same detector that gates reminders), the host calls the same `audio_cue.rs` chime on listen start instead of relying on the notch. [superseded by L19: the cue now plays on every activation, so the exclusive-mode detection is no longer needed to trigger it] | Silent listening | The user needs to know Niva is listening without a visible notch. See L19: the cue now plays on every activation. |
| L17 | Reminder after speech | The coral pulse does NOT resume after the reminder is spoken. The alert ends with the speech, then the normal settle applies. The 30 s alert timer still bounds a reminder that is never spoken (fullscreen). | Resuming the pulse | Owner decision: the alert is over once it has been heard. |
| L18 | Last fullscreen reminder | No tray item for the last reminder (owner rule: no text, audio only). | A disabled tray item with the text | Deferred: speak a missed fullscreen reminder after fullscreen ends (BACKLOG). |
| L19 | Activation cue | Every activation (wake word or Ctrl+Shift+Space) plays a short host cue before the command listen: the effect `Listen` carries `ack: true` and the runner plays `AudioCue::listening_cue`, opening the microphone when it ends (the id-aware pending-listen guard of L16, now for every activation and not only over exclusive fullscreen). The cue is "soft ascending": C5 523 Hz for 80 ms then G5 784 Hz for 160 ms, each a sine plus its 2nd harmonic at 0.2, 5 ms attack, exponential release (60 ms and 100 ms), peak gain 0.25, rendered as samples by `audio_cue.rs` (240 ms in all). A listen that follows a question Niva spoke (follow-up, confirm) has `ack: false`: no cue, since Niva just spoke and the cue would only delay the answer. **Timing: cue first, then listen** (not in parallel). The sidecar keeps the audio while the host waits: after a wake it records from `wake_preroll_ms` before the detection for up to `wake_claim_s` (3 s) and a late `listen` replays it, frames after the wake counting as live speech; after a shortcut the rolling buffer (`pre_speech_ms`) is replayed. So a user who speaks at once loses nothing, while a parallel listen would let the VAD hear the cue as a turn. The cue happens before the user speaks, so the end-of-speech to first-audio path is unchanged; the no-speech window is extended by the cue length. | Opening the listen at once and playing the cue in parallel; a cue after a follow-up question | The wake pre-roll makes the delay lossless and the microphone stays deaf to the cue; the follow-up cue adds latency and tells the user nothing new. Replaces the fullscreen-only trigger of L16. |

## Notch looks

Only the color tells states apart (owner decision 2026-11-10). The shape is the same waveform in every state; the level decides how tall the spikes are.

| State | Color | Level (shape amplitude) |
|---|---|---|
| listening | #378ADD | 0, breathing (~0.05); never level-driven (owner decision 2026-11-10) |
| thinking / acting | #7F77DD | 0, breathing (~0.05) |
| speaking (overrides) | #5DCAA5 | TTS level (simulated voice until Slice 5, or when no level for 150 ms) |
| confirmation / countdown | #EF9F27 | 0, breathing |
| success | #97C459 | 0, breathing, then hide |
| error | #E24B4A | 0, breathing, then hide |
| reminder | #F0997B | 0, breathing |

Reduced motion: no random re-targeting. The shape is static (`env(u)` scaled by a fixed factor) and its height follows the smoothed level; with level 0 it is a flat line, and the loop stops once the line is drawn. Speaking keeps following the real level on purpose (information, not decoration). The level bound of L14 (mean lag 50 ms or less, none above 100 ms) applies to level to shape amplitude through the level smoothing above, measured on the onset (rise): with attack 0.4 per frame the level covers half a step in 2 frames (about 33 ms at 60 Hz). The owner's release 0.1 per frame is intentionally slower (half-life about 110 ms) so the spikes fall gracefully; the decay is not part of the bound. A visually hidden `role="status"` region announces the state name (never reply text); together with the voice it is the non-color channel.

## Key lifecycle

| Trigger (from views and island events) | Esc | Ctrl+Shift+Enter |
|---|---|---|
| A view with a new `confirm.id` | register | - |
| `confirm.chord && expiresAt` present (the prompt has been spoken and the 10 s window runs) | - | register; Accept is ignored for the first 500 ms (`ARM_MS`, kept and applied to the chord); a press before registration never reaches the reducer |
| Key pressed (`Pressed` only, so autorepeat is ignored) | Reject{id}, then unregister both | Accept{id}, then unregister both |
| A view with no confirm or another id (confirm, reject, timeout, Ctrl+Shift+Space) | unregister | unregister |
| `Island{visible:false}` | unregister | unregister |
| App quit (tray Quit, `RunEvent::Exit`) | unregister | unregister |

The `chord` field is `state.confirm_click_only`. Registration runs through `run_on_main_thread` [verify: required by the plugin on Windows]. The plugin handler, set once in `run()`, sends the event only through `ConfirmKeys::pressed`, which checks the id (a stale id is dropped), allows one answer and applies the arm delay.

## Data flow

```
TTS slice --bands--pacer@due--> sidecar --level{tts}--> wire -> translate -> LevelSink --niva://level--> Notch refs
                                                                                                                                                                          ^
reducer -> ViewModel{state,speaking,confirm} -> KeysUi(register/unregister) -> TauriUi --niva://view
```

## File changes

| Area | Change |
|---|---|
| `app/src/notch/{Notch.tsx,wave.ts,looks.ts,motion.ts,levelStore.ts}` | Create |
| `app/src/{mascot/**,island-ui/**,island.ts,island.test.ts}`, `app/mascot-preview.html` | Delete |
| `app/src/{App.tsx,styles.css,state/viewStore.ts,debug/DebugWindow.tsx}` | Modify: new payload, level injection in debug |
| `tauri.conf.json` | 200x70 |
| `src-tauri/src/confirm_keys.rs` | Create (gate, decorator, registry trait) |
| `src-tauri/src/{confirm_buttons.rs,cursor.rs}`, `capabilities/island-actions.json`, `permissions-release/island-actions.toml` | Delete |
| `click_through.rs` | Keep prepare/show only |
| `island_contract_tests.rs` | Rewrite: window size, focus, transparency, hide delay against `motion.ts`, `ARM_MS` |
| `capabilities_tests.rs`, `check-release-binary` | No island commands |
| `ui_adapter.rs`, `effect_runner.rs`, `lib.rs`, `visibility.rs` | Modify |
| `voice_sidecar/{wire,translate}.rs`, `voice_sidecar.rs` | `Level` and its sink |
| `niva-core`: `state_machine.rs` (+coverage), `safety.rs` (rename), `reading.rs` (delete), `replies` (power prompt names the chord) | Modify |
| `voice/{levels.py (new),capture.py,tts.py,protocol.py}` | `level` |

## Slices (PR order)

The power actions must never be left without a confirmation path. The chord therefore lands first, while the buttons still exist.

| # | PR | Prod size | Tests first |
|---|---|---|---|
| 1 | **Confirm keys** (safety, own PR): `confirm_keys.rs`, Esc and chord, the `Physical` rename, the power prompt template, and a `chord` field added to the existing `ConfirmView` (additive, from `confirm_click_only`). Purely additive: the buttons still work. | ~250 | Fake registry for every exit path in the lifecycle table, arm delay, id and double-answer checks, failure goes to Reject; core: a transcript never confirms `click_only` (existing tests, re-asserted) |
| 2 | **Notch UI** with a simulated voice level, window shrink, frontend deletions. | ~350 (plus deletions) | vitest for `wave.ts` (smoothing, 150 ms fallback, seeded targets, reduced motion), the `looks.ts` table (every state and color); contract test |
| 3 | **View-model and core**: payload trim, `audible_speak`, `countdown` name, `reading.rs` removal (moving `settle_delay` to `effect_runner.rs` first), settle at 1.5 s. | ~200 | Rewrite the reducer tests for SpeakStarted; ui_adapter tests stop pinning captions |
| 4 | **Host cleanup** (capabilities, own PR): delete the buttons, hit-test, cursor and the `island-actions` capability. | mostly deletions | capabilities_tests: the island has no commands |
| 5 | **Level**: sidecar bands, pacer (flush on speech end and barge-in), `AudioOutput.latency` plus a fake, wire, sink, notch consumption. | ~300 | pytest for `bands` (sine to band, silence to 0), pacer due times with a fake clock and underrun; wire parse tests; manual 240 fps check |

Rollback: revert in reverse order. Invariant: the buttons are removed only after the chord has shipped (slice 1 before slice 4); once the buttons are removed, the chord slice is never reverted alone.

## Open questions resolved

1. **Search summary length:** deferred, not part of this change. Long replies are spoken in full. The 300-character cut pairs with the conversation-memory work (BACKLOG).
2. **Fullscreen reminder:** chime plus coral pulse, no text and no tray item (L18). Deferred: speak a missed fullscreen reminder after fullscreen ends.
3. **Level bound:** revised, see L14.
4. **Slice order:** the chord comes first, see "Slices".

## Risks

- Ctrl+Shift+Enter collides with Excel array formulas while a power confirmation is pending. Accepted by the owner as a visible risk. Mitigated: it is registered only after the prompt is spoken, there is a 500 ms arm delay, and the user started the request.
- `stream.latency` may misreport on some drivers. The offset variable and the 240 fps check cover it.

- Fullscreen reminder policy (L18): the `silent` flag logic is delivered and tested in this change, but it is only wired to a production caller when the reminder service lands (niva-mvp reminders slice). Until then `fullscreen.rs` keeps `allow(dead_code)` with that comment.
