# Verify report: task 2.11 and Batch 27 (sleep countdown), change `niva-mvp`

Branch `feat/sleep-countdown`, uncommitted working tree, artifact store openspec, strict TDD. Verified 2026-11-04. No production code was edited and nothing was committed.

## Verdict

PASS with warnings. 0 CRITICAL, 5 WARNING, 4 SUGGESTION. Archive of 2.11 is not blocked.

## Commands rerun (cargo from PowerShell)

| Command | Result |
|---|---|
| `cargo test --workspace` | host 291 passed (2 ignored), niva-core 464 passed, doc tests 11 passed; matches the reported counts |
| `cargo test -p niva --features eval --lib` | 362 passed (2 ignored); matches |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo clippy -p niva --features eval --all-targets -- -D warnings` | clean |
| `cargo clippy --release -p niva -- -D warnings` | clean |
| `cargo fmt --check` | clean |
| `npm test` (app/) | 19 files, 220 passed; matches |
| `npm run build` | ok |
| `npm run tauri build -- --no-bundle` | ok (release niva.exe built) |
| `npm run check:release` | ok (no debug command, dev adapter, eval strings or testing fakes in the release binary) |

## Unchanged against main

- `eval/` (commands.yaml, results/) and the prompt / tool schema files: `git diff main --stat` is empty for them.
- design.md: 32 insertions, 0 deletions (append-only at the end).
- `Cargo.lock`: content identical to main once CR is stripped (md5 equal, same byte length 111194). `git status` shows M only because of LF/CRLF; `git diff --stat` shows no content change.
- `PowerKind::click_only` unchanged; only `needs_countdown` was added (`crates/niva-core/src/tools.rs`).

## Spec vs code vs owner decision

Matches: sleep voice-confirmable; 10 s (`TimerKind::Countdown`, asserted in `tests_countdown::the_host_ready_starts_the_ten_second_deadline_and_announces`); any key or mouse via `GetLastInputInfo` (`app/src-tauri/src/adapters/windows_activity.rs`, equality of an opaque stamp, no hook, no key identity); shortcut cancels (`the_shortcut_cancels_the_countdown_with_the_same_words`, dispatcher `the_shortcut_cancels_the_countdown`); the OS call happens only via `FinishCountdown` -> `finish_countdown` -> `release_held` (`SafetyGate::execute_power` refuses sleep). Restart/shutdown click-only and lock without countdown: `lock_restart_and_shutdown_still_have_no_countdown`, `dispatcher::lock_has_no_countdown_and_restart_stays_click_only`.

Phase x event: `Phase::ALL: [Phase; 9]` (`crates/niva-core/src/state_machine.rs:53`), 27 event kinds, rows are `[Expect; 9]`, and `every_event_x_state_cell_is_asserted` (`state_machine/coverage.rs:607`) also asserts that `FinishCountdown` appears only in Watchdog x Countdown and `StopWatchingInput` only on exits from Countdown. Exhaustive and compile-enforced.

Scenario to test mapping (all passing; assertions are on exact effect vectors and phases, so a mutation would fail them):
- Accepted and left alone: dispatcher `suspende_la_pc_asks_then_counts_down_in_spanish_and_sleeps_at_the_end` (0 sleeps at 9 999 ms, 1 at 10 000 ms).
- Cancelled by input / shortcut: dispatcher `input_after_the_grace_period_cancels_and_the_pc_never_sleeps`, `the_shortcut_cancels_the_countdown`; reducer `a_cancel_leaves_the_countdown_says_so_and_drops_the_request`.
- Accepting input does not cancel: `activity::mouse_jitter_right_after_the_accept_click_is_ignored`, `the_grace_period_is_one_second_and_ends_exactly_there`, dispatcher `jitter_right_after_the_accept_click_does_not_cancel`.
- Race / stale deadline: `cancel_then_deadline_never_finishes`, `deadline_then_cancel_finishes_once_and_the_late_cancel_is_ignored`, `a_timer_of_an_earlier_countdown_never_fires_a_later_one`, `an_old_deadline_timer_does_nothing_after_the_cancel`.
- Fail closed (unreadable, host silent): `a_fault_ends_in_an_honest_error_and_never_finishes`, `a_host_that_never_answers_ends_in_an_error_with_no_finish`, `there_is_no_deadline_before_the_host_says_it_is_watching`, dispatcher `an_unreadable_activity_source_never_suspends_and_says_so`.
- Polling only during the countdown: dispatcher read-count-flat assertion, `effect_runner::nothing_polls_the_input_before_a_watch_is_asked_for`, `stopping_the_watch_stops_the_polling`, `watching_stops_on_every_way_out_of_the_countdown`.
- Reminder queued: `a_reminder_due_during_the_countdown_is_queued_not_shown_over_it`.
- Island seconds from a deadline: vitest "sleep countdown label" (pure functions).

## Findings

### CRITICAL

None.

### WARNING

1. `openspec/changes/niva-mvp/design.md:205` (E1 row) still says "Follow-up for a safety PR: make `sleep` click-only like restart and shutdown", which contradicts the new section at the end of design.md ("Sleep countdown", "This replaces the click-only idea"). The state-machine matrix (design.md:114-117 and the other 8-column tables) also still has eight phase columns while the code has nine (`Countdown`). Both are deliberate (the batch promised not to edit existing lines) but leave the design self-contradictory. Recommend a one-line "superseded by Sleep countdown" note at line 205 and a pointer near the matrix, with owner approval since it edits existing lines.
2. Vacuous assertions: `crates/niva-core/src/safety/tests.rs:919` (`holding_a_sleep_does_not_call_the_pc`) and `:959` (`a_held_sleep_that_is_dropped_never_reaches_the_pc`) build a `FakePc` but never pass it to anything (`hold_sleep` takes no PC), so `pc.calls().is_empty()` cannot fail. The real guarantee is covered by `execute_power_never_runs_sleep_and_never_touches_the_pc`, the compile-fail doc tests and the assistant test `a_cancel_before_the_deadline_never_sleeps_and_says_cancelled`, so there is no coverage gap, but these two tests prove nothing. Remove or rewrite them.
3. Spec wording vs behavior on a late cancel: the scenario "Cancel and deadline race" (`specs/action-safety/spec.md`, "Scenario: Cancel and deadline race") says that after the deadline wins "a later cancel is ignored". True for `CountdownCancel` (`tests_countdown::deadline_then_cancel_finishes_once_and_the_late_cancel_is_ignored`), but a late Shortcut in `Acting` ends the request and emits `CancelRequest`, which can drop the held approval before the worker takes it, so the PC may not sleep (`crates/niva-core/src/state_machine/tests_countdown.rs:377`, documented in the test name). This is the fail-safe direction, but the spec text is not exact. Reword it.
4. Island scenarios are weakly verified: "Reduced motion" (`specs/island-shell/spec.md`) and the "click-through, no buttons" clause have no test at all, and "pill does not resize" is only covered by `islandLabel` being time-independent. `Island.tsx` wiring and `useNow.ts` (200 ms ticker) have no component test. Manual only.
5. Real-desktop behavior unverified (stated by the implementer in tasks.md Batch 27): that real key and mouse input changes the `GetLastInputInfo` stamp in the owner's session, how the number looks, and Ctrl+Shift+Space from another app. Only two live reads and fake-based logic tests exist. Run the hand test in tasks.md before relying on it.

### SUGGESTION

1. `Cargo.lock` shows as modified only through line endings. Do not stage it (or `git checkout Cargo.lock`) so the commits stay clean; the commit plan correctly omits it.
2. tasks.md open point 5: the fault reply drops earlier results of the same plan (for example a volume change made before the sleep), so it says "did not suspend" without saying the volume changed. Cancel and finish replies keep them. Consider aligning.
3. The island text says "Suspending in {n} s." while the spoken line says "10 seconds"; fine, but the spec scenarios quote only the spoken form. Spanish "Pulsa" vs voseo is already flagged as an owner choice.
4. A cancel seen less than one 100 ms poll before the deadline loses the race by design (tasks open point 4). Acceptable; keep it in the hand test notes.

## Commit plan check (tasks.md Batch 27, Proposed commits)

Checked by reading what each file references; I did not build each commit in isolation.
1. Safety alone (`safety.rs`, `safety/tests.rs`, `tools.rs`, `tools/tests.rs`): builds. It uses only ports and tools items that exist on main; `hold_sleep`, `release_held` and `HeldPower` are self-contained. Existing tests still pass: on main no test executes a sleep (the only sleep-named test, `lock_and_sleep_may_be_confirmed_by_voice`, uses lock). Behavior at that commit: the assistant's `execute_power` call for sleep fails closed with "sleep must wait for its countdown" until commit 4. Acceptable.
2. Port and watch (`ports.rs`, `ports/testing.rs`, `activity.rs`, `activity/tests.rs`, `lib.rs`): builds; `activity.rs` imports only `std::time::Duration` and `crate::ports`; the tests use `FakeActivity` from the same commit.
3. Windows adapter (`windows_activity.rs`, `adapters/mod.rs`, `Cargo.toml`): builds; needs `InputActivity` and `ActivityError` from commit 2 and the `Win32_UI_Input_KeyboardAndMouse` feature added in the same commit.
4. Reducer, assistant, host: builds; `lib.rs` references `adapters::windows_activity::WindowsActivity` from commit 3. `debug.rs` tests `include_str!` `events.json` (commit 5) but only parse and replay the old entries (`buttons.len() >= 22`, 8 scripts), so they still pass with the old JSON.
5. TS (`events.ts`, `island.ts`, `Island.tsx`, `useNow.ts`, debug files): self-contained.
6. Docs: independent.
Order 1 to 6 compiles at every step as far as the references show.

## Result Contract

- status: done
- executive_summary: Task 2.11 / Batch 27 verified: every command rerun and counts confirmed (291 / 464 / 11 / 362 / 220); 0 CRITICAL, 5 WARNING, 4 SUGGESTION.
- artifacts: D:\trabajo\ProyectoAsistente\openspec\changes\niva-mvp\verify-report-countdown.md (openspec store; Engram not connected, no mem_save)
- next_recommended: sdd-archive for 2.11 once the owner decides on the design.md:205 note (warning 1); optionally fix warnings 2 and 3 first; run the hand test (warning 5) before PR 2d.
- risks: no unresolved CRITICAL. Open: design.md contradiction (E1 row and follow-up), two vacuous tests, spec wording on late cancel, island behavior and real GetLastInputInfo behavior verified only by pure functions and fakes.
- skill_resolution: none

## Post-review fixes (2026-11-04)

Applied on the same uncommitted tree, strict TDD, nothing committed. Warnings 1, 2 and 3 are closed; suggestion 2 is deferred.

- **Types (new, from the review).** `SafetyGate::approve` now returns a sleep as `Approval::Sleep(HeldPower)`; `Approval::Power(Approved<Power>)` is never a sleep and `hold_sleep` is removed, so `release_held` is the only route to `PcActions::power` for a sleep. A new compile-fail doc test (E0308) shows that handing a `HeldPower` to the port does not build; the earlier ones are unchanged. `execute_power` still refuses a sleep as a second barrier.
- **Cancel race.** `cancel_request` remembers the last 32 cancelled ids and `hold_for_countdown` refuses to store an approval for one of them (a worker that had already taken the plan can no longer re-arm a cancelled request). Tested deterministically with a test-only pause hook.
- **`finish_countdown`** also requires `latest_req == req`.
- **Warning 2 (vacuous tests)** removed with the old API; replaced by tests that can fail (`approving_a_sleep_gives_a_held_value_by_voice_and_by_click`, `approving_any_other_power_action_still_gives_an_approved_power`, `a_wrong_or_denied_answer_does_not_hold_a_sleep`).
- **Warning 3 (spec wording)** the scenario now says what the code does for a late shortcut.
- **Warning 1 (design.md)** "superseded by the sleep countdown section" notes added at the E1 row and above the transition matrix; line 272 not touched.
- **Suggestion 2 (fault reply)** not a few-line fix (needs a new field in `ToolsOutcome` and `State`); recorded in `docs/BACKLOG.md`.
- **Message wording.** The countdown text says "Press a key or move the mouse to cancel." / "Presiona una tecla o mueve el mouse para cancelar." (en and es templates and the island text). Earlier wording in this report ("Pulsa") is out of date; the remaining open question is voseo ("Presioná").
- **Commit plan.** Commit 1 can no longer be the safety files alone: `assistant.rs` on `main` matches `Approval` exhaustively and does not build with the new variant. The safety files now go in the same commit as the reducer, assistant and host (tasks.md Batch 27 updated).
- **Counts after the round:** `cargo test --workspace` host 291 (2 ignored), niva-core 468, doc tests 12; `cargo test -p niva --features eval --lib` 362; `npm test` 220; the three clippy runs, `cargo fmt --check`, `npm run tauri build -- --no-bundle` and `npm run check:release` clean.
