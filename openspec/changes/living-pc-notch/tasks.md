# Tasks: Living PC notch

Sources: `proposal.md`, `design.md` (decisions L1-L18, "Key lifecycle", "Slices"), `specs/*/spec.md` (7 capabilities: notch, action-safety, island-shell, speech-io, fullscreen-policy, reminders, mascot).

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines (production + tests, additions only; deletions in brackets) | S1 ~450, S2 ~600 [~1500 deleted], S3 ~350, S4 ~80 [~600 deleted], S5 ~650 |
| 400-line budget risk | High for S1, S2, S5; Medium for S3; Low for S4 (mostly deletions) |
| Chained PRs recommended | Yes |
| Suggested split | 5 PRs, one per design slice, in order S1 -> S2 -> S3 -> S4 -> S5 |
| Delivery strategy | ask-on-risk |
| Chain strategy | `stacked-to-main`: five PRs, merged on GitHub one by one (owner, 2026-11-10). |

Decision needed before apply: No. The owner kept the five slices as planned (2026-11-10); S1, S2 and S5 ship with `size:exception`.
Chained PRs recommended: Yes
Chain strategy: pending confirmation (proposed: `stacked-to-main`)
400-line budget risk: High

### Work units (one PR each)

| Unit | Goal | PR | Depends on |
|------|------|----|-----------|
| S1 | Confirm keys (Esc + Ctrl+Shift+Enter), purely additive | PR 1 | none |
| S2 | Notch UI with simulated bars, window 200x70, frontend deletions | PR 2 | S1 merged (gate G1) |
| S3 | View-model trim, `audible_speak`, `reading.rs` removal, settle 1.5 s, voice-down chime | PR 3 | S2 merged |
| S4 | Host cleanup: buttons, hit-test, cursor, `island-actions` capability | PR 4 | S1 merged (gate G2), S2 and S3 merged |
| S5 | Level messages: sidecar bands, pacer, wire, sink, notch consumption | PR 5 | S2, S3 merged |

## Conventions

- Strict TDD (`strict_tdd: true`): every implementation task is RED (write the named failing test), GREEN (smallest change), REFACTOR (tests stay green). The test name is given in each task.
- Run cargo/Tauri only from PowerShell, never Git Bash (Git's `link` shadows MSVC `link.exe`).
- Commands (real ones, from `app/package.json` and the repo):
  - `cargo test -p niva-core` (from repo root)
  - `cargo test` (from `app/src-tauri`)
  - `npm test` (from `app`; runs `vitest run`) and `npm run build` (from `app`; runs `tsc && vite build`)
  - `npm run check:release` (from `app`)
  - `voice\.venv\Scripts\python.exe -m pytest -q` (from `voice`)
- Commits are work-unit commits (tests and the code they drive in the same commit), conventional commit messages, no AI attribution.
- Artifacts are in English; all UI strings and comments in English.

## Confirmation-channel invariant (applies to every slice boundary)

Restart and shutdown must always have a working confirmation channel. At each boundary:

| After slice | Buttons in UI | Host click channel | Chord + Esc | Restart/shutdown channel |
|-------------|---------------|--------------------|-------------|--------------------------|
| S1 merged | yes | yes | yes (new) | click and chord |
| S2 merged | removed from UI | still in host (dead, no UI sends) | yes | chord (S1 is merged: gate G1) |
| S3 merged | no | still in host (dead) | yes | chord |
| S4 merged | no | removed | yes | chord only |
| S5 merged | no | removed | yes | chord only |

Gates:
- **G1:** S2 MUST NOT be opened for merge until S1 is merged to the base branch. S2 removes the buttons from the UI.
- **G2:** S4 MUST NOT be opened until S1 is merged and S1's lifecycle tests are green on the base branch.
- Once S2 is merged, S1 is never reverted alone.

---

## Slice 1 (PR 1): Confirm keys (safety, own PR)

Goal: add `Esc` and `Ctrl+Shift+Enter` as temporary global shortcuts owned by `ConfirmKeys`, producing the same `Accept{id}` / `Reject{id}` the click did. Purely additive: the buttons still work.

- [ ] 1.1 Rename `safety::Channel::Click` to `Channel::Physical` in `crates/niva-core/src/safety.rs` and all users (doc names the chord and the debug button). RED: `physical_channel_builds_accept_for_click_only` and `transcript_channel_never_builds_accept` fail to compile/run. GREEN: rename. Also re-assert `model_bypass_still_needs_confirmation` (existing gate test).
- [ ] 1.2 Add `chord: bool` to the existing `ConfirmView` (additive), from `state.confirm_click_only`. Files: the view-model/adapter in `app/src-tauri/src/ui_adapter.rs` (locate with `rg ConfirmView`). RED: `view_confirm_chord_true_for_power_confirmation`, `view_confirm_chord_false_for_close`. GREEN. Frontend ignores the new field for now.
- [ ] 1.3 Power prompt names the chord, in `niva-core` replies. RED: `power_prompt_names_chord`. Re-assert in `niva-core` tests: `transcript_never_confirms_click_only` (restart and shutdown; includes ambient "yes, do it"), `ctrl_shift_space_rejects_power_confirmation`, `voice_yes_confirms_close_and_sleep`, `no_countdown_for_restart_shutdown_lock`, `release_enters_confirmation_for_risky_request`, `timeout_cancels_pending_confirmation`.
- [ ] 1.4 Create `app/src-tauri/src/confirm_keys.rs`: `ShortcutRegistry` trait, `FakeRegistry` for tests, and `ConfirmKeys` (id check, one answer, `ARM_MS = 500`). Write these tests first, one at a time, each RED then GREEN:
  - `close_confirmation_registers_esc_only`
  - `restart_confirmation_registers_esc_immediately_and_chord_only_after_prompt_spoken` (chord registers when `chord && expiresAt` is present)
  - `chord_pressed_before_registration_never_reaches_reducer`
  - `chord_within_500ms_of_arming_is_ignored_and_confirmation_stays_pending`
  - `esc_press_emits_reject_and_unregisters_both`
  - `chord_press_emits_accept_and_unregisters_both`
  - `chord_on_close_confirmation_confirms_nothing`
  - `esc_with_nothing_pending_is_not_registered`
  - `stale_id_press_is_dropped_state_unchanged`
  - `second_press_after_answer_is_dropped` (one answer only)
  - `only_pressed_state_counts_autorepeat_ignored`
  - `no_shortcut_leaks_on_any_exit_path` (table-driven: accept, voice reject, Esc, Ctrl+Shift+Space, timeout, quit)
  - `island_hidden_unregisters_both`
  - `view_with_no_confirm_or_other_id_unregisters`
  - `esc_registration_failure_is_logged_and_confirmation_continues`
  - `chord_registration_failure_emits_reject_and_sets_tooltip_confirm_key_unavailable` (fail closed)
- [ ] 1.5 Add `KeysUi` (decorator of `UiPort`) in `confirm_keys.rs`, driven by published views. It composes next to the existing `GateUi`/`ConfirmGate` (the button guard stays until S4). RED: `keys_ui_forwards_view_and_registers_from_view_sequence`, `keys_ui_unregisters_on_island_hidden`. Add a `TooltipPort` trait with a fake for the tray tooltip.
- [ ] 1.6 Wire in `app/src-tauri/src/lib.rs`: global-shortcut plugin handler set once in `run()`, sending only through `ConfirmKeys::pressed`; registration via `run_on_main_thread` (`[verify]` plugin requirement on Windows); unregister on `RunEvent::Exit` and tray Quit. RED: `quit_unregisters_both_shortcuts` (through the fake registry and the exit hook function). Keep host-only permissions; do not add webview capabilities.
- [ ] 1.7 Docs: add the two shortcuts to the user-facing shortcuts section where `Ctrl+Shift+Space` is documented (find with `rg "Ctrl\+Shift\+Space" --glob "*.md"`). No test; docs only.

Verification:
- `cargo test -p niva-core`
- `cargo test` (from `app/src-tauri`)
- `npm test` (from `app`) and `npm run build` (unchanged UI, must stay green)

Live hand-test (owner):
1. Say "close Spotify". Esc cancels, Niva says it was cancelled. Press Esc with nothing pending: it reaches the focused app (for example, closes a dialog).
2. Say "shut down the PC" (use restart on a safe session). While the prompt is still speaking, press Ctrl+Shift+Enter: nothing happens. After the prompt, wait 1 s, press Ctrl+Shift+Enter: the request is accepted. Do this once as a dry run if shutdown is stubbed in debug.
3. Press Ctrl+Shift+Enter within 500 ms of the prompt ending: ignored.
4. Say "yes" to a restart confirmation (or play speech): it does not run.
5. Ctrl+Shift+Space rejects. Wait 10 s: times out. Afterwards Esc and Ctrl+Shift+Enter reach other apps again.
6. The buttons still work in this slice.

Rollback boundary: revert the PR. Nothing else depends on it until S2 merges. After S2 merges, S1 is never reverted alone.

---

## Slice 2 (PR 2): Notch UI, window shrink, frontend deletions

Gate G1: S1 is merged. This slice removes the buttons from the UI, so the chord must already exist on the base branch.

Goal: the island webview becomes a 118x30 notch with 7 bars animated by one rAF loop, simulated bars only (no real levels yet). Works against the existing view payload: ignores `caption`, `prompt`, labels and `countdown.text`; treats a missing `speaking` as false.

- [ ] 2.1 Create `app/src/notch/motion.ts`: `NOTCH_W = 118`, `NOTCH_H = 30`, `NOTCH_OUT_MS = 200`, `HIDE_DELAY_MS = 240`, reduced-motion fade 120 ms. RED in `app/src/notch/motion.test.ts`: `notch_is_118_by_30`, `hide_delay_within_exit_plus_100` (`OUT <= HIDE <= OUT + 100`), `reduced_motion_fade_below_hide_delay`.
- [ ] 2.2 Create `app/src/notch/looks.ts`: look table (color, motion kind, reduced-motion silhouette) and `viewToLook(view, speaking)`. RED in `looks.test.ts`:
  - `every_state_has_look_with_design_color` (#378ADD, #7F77DD, #5DCAA5, #EF9F27, #97C459, #E24B4A, #F0997B)
  - `reduced_motion_silhouettes_pairwise_distinct` (all seven)
  - `acting_uses_thinking_look`, `countdown_uses_confirmation_look`
  - `speaking_overrides_phase_look_and_reverts` (success + speaking = speaking look; then success)
  - `idle_is_hidden`
  - `reminder_alert_look_and_no_text`
- [ ] 2.3 Create `app/src/notch/bars.ts`: pure `frame(nowMs, look, levels) -> number[7]` (0..1), attack tau 25 ms, release tau 110 ms, simulation after 150 ms without levels. RED in `bars.test.ts`:
  - `attack_faster_than_release`
  - `falls_back_to_simulation_after_150ms_without_levels`
  - `returns_to_real_levels_when_levels_resume`
  - `wave_phase_travels_left_to_right_1200ms`
  - `levels_ignored_in_thinking_and_confirmation`
  - `reduced_motion_simulation_does_not_animate`
  - `fake_clock_lag_mean_at_most_50ms_max_at_most_100ms`
- [ ] 2.4 Create `app/src/notch/levelStore.ts`: latest `{src, bands, at}` outside React. RED in `levelStore.test.ts`: `stores_latest_levels_with_timestamp`, `rejects_wrong_length_or_range_bands`, `source_filter_mic_for_listening_tts_for_speaking`.
- [ ] 2.5 Create `app/src/notch/Notch.tsx`: 7 `<span>` bars, one rAF loop writing `transform: scaleY()` and `opacity` through refs, color through a CSS custom property set once per state, visually hidden `role="status"` announcing the state name only. RED in `Notch.test.ts` (using `react-dom/server`, no new dependencies): `renders_seven_bars_in_118x30_shell`, `status_region_has_state_name_and_no_reply_text`, `no_buttons_and_no_text_in_markup`.
- [ ] 2.6 Modify `app/src/App.tsx`, `app/src/styles.css`, `app/src/state/viewStore.ts`: render `Notch` instead of island UI; drop-in `translateY(-100%)` to 0, 320 ms, `cubic-bezier(.34,1.56,.64,1)`; drop-out 200 ms ease-in; `prefers-reduced-motion` fade. RED in `viewStore.test.ts`: `existing_payload_with_caption_is_accepted_and_text_ignored`, `missing_speaking_defaults_false`.
- [ ] 2.7 Modify `app/src/debug/DebugWindow.tsx`: the 8-state switcher drives the notch; add a "level test" control that injects mic/tts levels into `levelStore` (clicks every 500 ms for the later filming). RED: `debug_state_switch_sets_view_for_each_state`.
- [ ] 2.8 Delete `app/src/mascot/**`, `app/src/island-ui/**`, `app/src/island.ts`, `app/src/island.test.ts`, `app/mascot-preview.html`. RED: `notch_has_no_image_assets` in `looks.test.ts` (walks `app/src` and `app/public`, asserts no `.png/.jpg/.svg/.webp/.gif` for the notch). GREEN: deletions; `npm run build` (tsc) proves no dangling imports.
- [ ] 2.9 Modify `app/src-tauri/tauri.conf.json` to 200x70 (transparent, `focusable: false`, no decorations, no shadow, always on top). Modify `click_through.rs`: call `set_ignore_cursor_events(true)` on prepare and on every show; keep prepare/show only. Rewrite `app/src-tauri/src/island_contract_tests.rs` first (RED): `island_window_is_200x70_transparent_non_focusable`, `window_ignores_cursor_on_prepare_and_every_show`, `hide_delay_parsed_from_motion_ts_within_exit_plus_100`, `arm_ms_is_500`. GREEN: config and click-through change.
- [ ] 2.10 Spike (no production code): find how to detect exclusive fullscreen with the same detector that gates reminders (`visibility.rs`), and whether the notch can be drawn over it (design L16, `[verify]`). Deliverable: a short written finding in apply-progress, with a yes/no on detection feasibility. If not feasible, task 3.10 is marked deferred.

Verification:
- `npm test` and `npm run build` (from `app`)
- `cargo test` (from `app/src-tauri`) and `cargo test -p niva-core`
- `npm run check:release`

Live hand-test (owner):
1. Press Ctrl+Shift+Space: a small black notch drops in at the top center with a springy bounce and the blue listening look. It is 118x30; the window around it is clear.
2. Click on the notch and the area around it: the click reaches the app below.
3. Run the debug state switcher through all seven states: color and motion match the table; hide with the system reduced-motion setting on: each state is a distinct static silhouette.
4. Issue a restart/shutdown request: amber pulse only, no text, no buttons. Confirm with the chord (S1). Reply text is not shown anywhere.
5. Grayscale check (screenshot to grayscale): states tell apart by bar shape.
6. Bars are simulated here (real levels arrive in S5).

Rollback boundary: revert PR 2 restores the old island UI and window size. S1 must stay (the old UI still works with S1 present). S3 depends on S2's `viewStore` shape; revert S3 first.

---

## Slice 3 (PR 3): View-model and core trim

Goal: remove reply text and labels from the payload, add `audible_speak`, delete the reading hold, settle at 1.5 s, voice-down chime, reminder behavior.

- [ ] 3.1 `crates/niva-core/src/state_machine.rs`: add `State::audible_speak: Option<u64>`; update the coverage matrix row for `SpeakStarted` from ignore to a state-only update. RED: `speak_started_matching_id_sets_audible_speak`, `speak_started_mismatched_id_does_not_set_it`, `audible_speak_clears_wherever_speak_id_clears` (table-driven over every site that clears `speak_id`), `coverage_matrix_speak_started_is_state_only`. Rewrite the existing SpeakStarted reducer tests.
- [ ] 3.2 `ui_adapter.rs`: `speaking = audible_speak.is_some()`. RED: `view_speaking_true_only_while_audible_speak_set`, `view_not_speaking_during_synthesis`.
- [ ] 3.3 Trim the payload to `{state, speaking, confirm?: {id, expiresAt?, chord}, reminder?: {count}}`; remove `caption`, `prompt`, labels, `countdown.text`; name `Countdown` as `"countdown"`. RED: `view_payload_has_no_reply_text_keys`, `countdown_phase_serializes_as_countdown`, `reminder_view_carries_count_only`, `ui_adapter_tests_no_longer_pin_captions` (rewrite). Update TS types in `viewStore.ts`; RED in vitest: `view_store_parses_trimmed_payload`.
- [ ] 3.4 Debug builds only: one dev-log line per `Speak` via `DEV_LOG_EVENT`. RED: `speak_logs_reply_text_to_dev_log_in_debug_only` (fake log sink; release cfg does not log).
- [ ] 3.5 Hide timing. Step a (refactor, tests stay green): move `settle_delay` from `reading.rs` into `effect_runner.rs` (import at `effect_runner.rs:41`). Step b: delete `reading.rs` and `ReadingHold`; `TimerKind::Settle` 3 s to 1.5 s. RED: `settle_timer_is_1500ms`, `long_reply_settle_starts_at_speech_end_without_hold` (400-character reply), `success_hides_after_speech_plus_settle`, `error_hides_after_settle`, `activation_during_settle_cancels_pending_hide`, `exit_hold_matches_motion_hide_delay`.
- [ ] 3.6 Keep the 900 ms "acting stays visible" hold in `effect_runner.rs` (not part of the deleted hold). Regression tests: `quick_action_holds_acting_900ms_before_result`, `reminder_during_hold_shows_at_once_and_drops_held_result`, `countdown_is_never_held`.
- [ ] 3.7 Reminders. RED: `reminder_alert_pulse_does_not_resume_after_speech`, `reminder_alert_ends_with_speech_then_settles`, `reminder_30s_alert_timer_bounds_unspoken_reminder`, `reminder_fires_with_chime_and_speaks_without_text`, `fullscreen_reminder_chime_no_speech_no_text`, `speaking_overrides_reminder_pulse`.
- [ ] 3.8 Voice-down chime (L15). RED: `sidecar_down_activation_plays_host_chime_and_error_view`, `mic_error_plays_chime_and_error_view`, `voice_down_view_hides_after_settle`. Uses `audio_cue.rs` through a fake `AudioCue`; no sidecar involvement.
- [ ] 3.9 Empty transcript. RED: `empty_transcript_runs_no_tool_and_shows_error_then_hides` (core).
- [ ] 3.10 Exclusive fullscreen (L16), depends on spike 2.10. RED: `exclusive_fullscreen_plays_cue_on_listen_start_instead_of_notch`. If the spike found detection infeasible, record this task as deferred in apply-progress and skip the test.
- [ ] 3.11 Frontend consumes `speaking` and `countdown`. RED in `looks.test.ts`: `speaking_flag_from_payload_overrides_phase_look`, `countdown_payload_maps_to_confirmation_look`. Regression: `typed_text_in_confirmation_is_ignored` (core), `typed_command_enter_submits_and_clears_box` (frontend, existing behavior), reply to typed command shows no text.

Verification:
- `cargo test -p niva-core`
- `cargo test` (from `app/src-tauri`)
- `npm test`, `npm run build` (from `app`)
- `voice\.venv\Scripts\python.exe -m pytest -q` (from `voice`; unchanged, must stay green)

Live hand-test (owner):
1. Ask "open notepad": the notch goes green only while the voice is audible (not during the ~0.9 s synthesis), then success bounce and hide about 1.5 s after the speech ends.
2. A long reply (about 400 characters): spoken in full; hide starts at the end of the speech.
3. Quick action: acting wave stays at least 0.9 s before the result.
4. Reminder: coral pulse, chime, spoken; after speech the pulse does not return and the notch hides. Reminder over a fullscreen video: chime and coral pulse, no speech.
5. Stop the sidecar, press Ctrl+Shift+Space: error look and the chime, then hide.
6. Say something unintelligible: error look, then hide.
7. Debug build: the reply text appears in the dev log, not on the screen.

Rollback boundary: revert PR 3 (and any later PR first). It restores the old payload; S2's frontend tolerates both shapes only for fields it read, so revert S5 and S4 before S3.

---

## Slice 4 (PR 4): Host cleanup (capabilities, own PR)

Gate G2: S1 is merged and its lifecycle tests are green on the base branch. S2 and S3 are merged so nothing in the UI or view-model uses the click channel.

Goal: delete the click channel and hit-test machinery. Restart and shutdown are confirmable only by the chord (and by the debug button in debug builds).

- [ ] 4.1 RED first in `app/src-tauri/src/capabilities_tests.rs`: `island_capability_grants_no_commands`, `no_island_actions_capability_or_permission_files`. GREEN: delete `capabilities/island-actions.json` and `permissions-release/island-actions.toml`; update `app/scripts/check-release-binary.mjs` so `npm run check:release` expects no island commands.
- [ ] 4.2 Delete `confirm_buttons.rs`, the host click command, the rectangles report, the 500 ms button guard (`GateUi`/`ConfirmGate`, replaced by `KeysUi`). RED: `no_click_command_registered_in_invoke_handler`, `no_references_to_confirm_buttons_remain` (build). The S1 `ConfirmKeys` tests must stay green and untouched.
- [ ] 4.3 Delete `cursor.rs` and the cursor poller; `click_through.rs` keeps prepare/show only. RED: `window_ignores_cursor_events_in_every_state`, `no_cursor_poller_started`.
- [ ] 4.4 Re-assert the invariant in release: RED/GREEN `release_has_chord_as_only_power_confirmation_channel`, `debug_button_confirm_exists_in_debug_builds_only` (cfg-gated), `chord_registration_failure_still_rejects_in_release`. Update docs for `Channel::Physical` (the debug window's button, debug builds only).

Verification:
- `cargo test` (from `app/src-tauri`) and `cargo test -p niva-core`
- `npm test`, `npm run build`, `npm run check:release` (from `app`)
- `rg "confirm_buttons|island-actions|cursor_poller"` returns no hits outside docs and changelog.

Live hand-test (owner):
1. Release build: restart/shutdown confirmation answered by Ctrl+Shift+Enter only; voice "yes" does not confirm.
2. Click anywhere on the island area with a confirmation pending: click goes to the app below, nothing is answered.
3. Esc and Ctrl+Shift+Space still work. Chord registration is released after every exit.

Rollback boundary: revert PR 4 only (it restores the click channel in the host). Do NOT revert S1 or S2 while PR 4 is merged.

---

## Slice 5 (PR 5): Level messages

Goal: real audio levels from the sidecar to the notch as `niva://level`, bypassing the reducer.

- [ ] 5.1 Create `voice/levels.py` `bands(pcm, rate)`: Hann window, rfft, 7 log bands with edges 80/160/320/640/1280/2560/4000/8000 Hz, band RMS in dBFS mapped from [-60, -10] to 0-100. RED in `voice/tests/test_levels.py`: `test_sine_lands_in_expected_band`, `test_silence_is_all_zero`, `test_returns_seven_ints_in_0_100`, `test_dbfs_mapping_endpoints`, `test_short_frame_does_not_raise`.
- [ ] 5.2 `voice/protocol.py`: `{"type":"level","src":"mic"|"tts","bands":[7 ints]}`. RED: `test_level_message_shape`, `test_level_message_has_no_samples_or_text`.
- [ ] 5.3 `voice/capture.py`: one value per 32 ms frame, only while a listen is open; rate guard at most one emit per 25 ms per source. RED: `test_mic_level_only_while_listen_open`, `test_idle_sends_no_level`, `test_mic_level_rate_20_to_40_hz`, `test_rate_guard_25ms`.
- [ ] 5.4 `voice/audio_io.py`: extend the `AudioOutput` protocol with `latency` and add a fake implementation for tests. RED: `test_fake_output_exposes_latency`.
- [ ] 5.5 `voice/tts.py`: cut each written chunk into 32 ms slices; pacer thread emits each slice at `due = max(prev_due + slice_ms, write_return + latency)`; first write anchors, underrun re-anchors; flush and stop on speech end and on barge-in; `NIVA_TTS_LEVEL_OFFSET_MS` (default 0). RED with a fake clock and fake output: `test_slices_are_32ms`, `test_due_times_follow_playback_clock`, `test_first_write_anchors_clock`, `test_underrun_reanchors`, `test_pacer_flushed_on_speech_end`, `test_pacer_stops_within_one_period_on_barge_in`, `test_level_offset_env_shifts_due`, `test_tts_level_rate_20_to_40_hz`, `test_idle_sends_no_tts_level`.
- [ ] 5.6 `app/src-tauri/src/voice_sidecar/wire.rs` and `translate.rs`: parse `level`; `Translated` gains `level: Option<Level>`. RED: `parse_valid_level_message`, `level_with_six_bands_is_dropped`, `level_with_value_101_is_dropped`, `level_with_negative_or_non_integer_is_dropped`, `level_with_unknown_src_is_dropped`, `malformed_level_does_not_block_other_messages`.
- [ ] 5.7 `voice_sidecar.rs`: `LevelSink` emits `niva://level` to `island` only; not logged, not through the dispatcher. RED: `level_creates_no_reducer_event`, `level_flood_30hz_does_not_reach_dispatcher`, `level_is_not_logged`, `level_emitted_to_island_window_only`.
- [ ] 5.8 Frontend: subscribe to `niva://level` into `levelStore`; Notch uses mic bands while listening and tts bands while speaking. RED: `level_event_updates_store`, `wrong_source_ignored_for_state`, `missing_level_stream_leaves_other_view_updates_untouched` (vitest with a mocked event listener).
- [ ] 5.9 Manual measurement of the L14 bound (no code): debug "level test" clicks every 500 ms, filmed at 240 fps, mic and TTS, built-in or wired output; result logged in apply-progress. Bluetooth excluded; use `NIVA_TTS_LEVEL_OFFSET_MS` if needed. Docs: add the env variable to `voice/README.md`.

Verification:
- `voice\.venv\Scripts\python.exe -m pytest -q` (from `voice`)
- `cargo test` (from `app/src-tauri`) and `cargo test -p niva-core`
- `npm test`, `npm run build` (from `app`)

Live hand-test (owner):
1. Say a sentence after Ctrl+Shift+Space: the blue bars follow your voice, no visible delay.
2. Ask a question: the green bars follow Niva's voice in step with what you hear (wired or built-in speakers).
3. Interrupt Niva mid-sentence (barge-in): bars stop within about a frame.
4. Thinking and confirmation states do not react to sound.
5. Kill the sidecar mid-listen: bars go on from the simulation after about 150 ms and the next state works.
6. Film the debug "level test" at 240 fps: mean lag at most 50 ms, no sample above 100 ms.

Rollback boundary: revert PR 5 only. The notch keeps working on simulated bars (the S2 behavior); no other slice depends on it.

---

## Traceability: spec scenario to task

| Spec | Scenario | Task(s) |
|------|----------|---------|
| notch | Notch dimensions | 2.1, 2.5, 2.9 |
| notch | Asset audit | 2.2, 2.8 |
| notch | Idle | 2.2 |
| notch | Summoned | 2.6, 2.1, S2 hand-test |
| notch | Each state has its look | 2.2, 2.7 |
| notch | Acting and countdown reuse looks | 2.2, 3.11 |
| notch | Speaking overrides the phase look | 2.2, 3.1, 3.2, 3.11 |
| notch | Grayscale | 2.2 |
| notch | Reduced motion | 2.2, 2.3 |
| notch | Listening follows the microphone | 2.3, 5.3, 5.8, 5.9 |
| notch | Speaking follows the voice | 2.3, 5.5, 5.8, 5.9 |
| notch | Levels do not drive other states | 2.3, 5.8 |
| notch | Levels stop | 2.3 |
| notch | Levels resume | 2.3 |
| notch | Success hides | 3.5 |
| notch | Error hides | 3.5 |
| notch | Activation during settle | 3.5 |
| notch | A reply | 2.5, 3.3 |
| notch | A confirmation | 2.5, 3.3 |
| notch | Click on the notch | 2.9, 4.3 |
| notch | Sidecar down | 3.8 |
| notch | Reminder alert | 2.2, 3.7 |
| action-safety | Confirmed | 1.3 |
| action-safety | Denied | 1.3, 1.4 |
| action-safety | Model bypass attempt | 1.1 |
| action-safety | Other power actions unchanged | 1.3 |
| action-safety | The release asks before it acts | 1.3, 4.4 |
| action-safety | Restart and shutdown never confirm by speech | 1.3 |
| action-safety | Esc cancels | 1.4 |
| action-safety | Esc with nothing pending | 1.4 |
| action-safety | Chord confirms shutdown | 1.4 |
| action-safety | Chord on another kind | 1.4 |
| action-safety | Ambient speech | 1.3 |
| action-safety | Ctrl+Shift+Space rejects | 1.3 |
| action-safety | Stale or repeated chord | 1.4 |
| action-safety | Registration lifecycle | 1.4, 1.5 |
| action-safety | Close confirmation registers Esc only | 1.4 |
| action-safety | Chord pressed before the prompt ends is dropped | 1.4 |
| action-safety | Chord pressed within 500 ms of arming is ignored | 1.4 |
| action-safety | Leak check on every exit path | 1.4 |
| action-safety | Hidden island and quit unregister | 1.4, 1.5, 1.6 |
| action-safety | Stale confirmation id | 1.4 |
| action-safety | Registration fails (chord) | 1.4 |
| action-safety | Registration fails (Esc) | 1.4 |
| action-safety | Slice order | Gates G1, G2; 4.4 |
| action-safety | Chord slice is never reverted alone | Gates G1, G2; rollback boundaries S1, S4 |
| island-shell | Island placement | 2.9 |
| island-shell | Click on an app under the window | 2.9 |
| island-shell | No capture in any state | 2.9, 4.3 |
| island-shell | A quick action | 3.6 |
| island-shell | A reminder during the hold | 3.6 |
| island-shell | Submit with Enter | 3.11 |
| island-shell | Typed text is not a confirmation | 3.11 |
| island-shell | Auto-hide | 2.1, 3.5 |
| island-shell | Long reply | 3.5 |
| speech-io | Empty transcript | 3.9 |
| speech-io | Level while listening | 5.1, 5.3 |
| speech-io | Level while speaking | 5.5, 5.9 |
| speech-io | Idle sends nothing | 5.3, 5.5 |
| speech-io | No audio content | 5.2 |
| speech-io | Flood does not touch the reducer | 5.7 |
| speech-io | Malformed level | 5.6 |
| speech-io | audible_speak follows the audible speech | 3.1, 3.2 |
| speech-io | Mismatched SpeakStarted | 3.1 |
| fullscreen-policy | Reminder during video | 3.7 |
| fullscreen-policy | Notch cannot draw over exclusive fullscreen | 2.10, 3.10 |
| reminders | Fires at due time | 3.7 |
| reminders | Speaking overrides the pulse | 2.2, 3.7 |
| reminders | The alert ends with the speech | 3.7 |
| mascot | (all REMOVED requirements) | 2.8 |

Unchanged baseline scenarios (sleep countdown cancel, grace, races and so on) are covered by the existing suites; each slice's verification run must keep them green.

## Open items for apply

- Exact locations of `ConfirmView`, `GateUi`/`ConfirmGate` and the `Speak` effect sites are named from the design; confirm with `rg` before editing.
- Exclusive-fullscreen detection (2.10) is a spike; 3.10 may be deferred.
- Chain strategy must be confirmed before PR 1 is opened.
