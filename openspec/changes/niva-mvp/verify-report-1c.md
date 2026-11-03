# Verify report: niva-mvp PR 1c (tasks 1.14-1.20), branch feat/1c-host

Date: 2026-11-02. Mode: Strict TDD. Verdict: PASS WITH WARNINGS (0 CRITICAL, 4 WARNING, 5 SUGGESTION). OK to proceed to PR 1d.

## Evidence (run by verifier, PowerShell)
- `cargo test --workspace`: 65 (niva) + 160 (niva-core) passed, 0 failed.
- `cargo clippy --workspace --all-targets -- -D warnings`: exit 0. `cargo clippy --release -p niva -- -D warnings`: clean. `cargo fmt --check`: exit 0.
- `npm test` (app/): 3 files, 13 tests passed.
- Tasks 1.14-1.20 are [x] and match the code on disk. GUI not launched.

## 1. Runner contract: PASS
- Every `Speak` arms the watchdog first, token = speak id (`effect_runner.rs:157-164`, `166-178`); length rule max(5 s, 80 ms/char) tested (`:436`, `:402`, `:419`).
- Disarm paths: `observe` of SpeakDone/TtsFailed of the same id only (`:98-102`, test `:450`, `:462`), `StopSpeaking` (`:119`, test `:472`), newer Speak (`:167`, test `:481`).
- Timers echo `Watchdog{kind, token}` (`:131-145`); all 9 durations tested at d-1ms/d (`:298`); restart replaces (`:320`); cancel by kind (`:347`); reducer-started SpeakWatchdog ignored (`:373`).
- Chime -> ChimeDone (`:120-126`, test `:502`); Held never completes (`:517`).
- "Deadline at StartTimer arrival": `sleep_until(deadline)` with deadline computed before spawn (`:138-140`, `:169-170`). The paused-clock tests call `advance` right after `run`, before the task is first polled, so a first-poll deadline would fire late (this is what produced the 8 RED failures). Test is discriminating.
- W2 end to end: `dispatcher.rs:332` (Held, confirm prompt never ends -> 5 s watchdog -> reducer synthesizes speak_done -> 10 s confirm timer -> Cancelled). Reducer side (`state_machine.rs:1090-1096`) maps SpeakWatchdog to `on_speak_done`. Guaranteed.
- No task leaks: aborts on replace/cancel/disarm, `Drop` calls `cancel_all` (`:193-197`). Finished handles stay in the Vec/Option until replaced; bounded (<= 9 timers + 1 watchdog), harmless.

## 2. Stubs: PASS
- Speak/Chime synthesize Done in Instant mode; Listen, CancelListen, ForceEndOfSpeech, StartRequest, RunTool, CancelRequest, EnqueueReminder, DrainReminders are logged (`effect_runner.rs:127`, test `:539`). Reducer timers (no-speech 6 s, LLM 15 s, tool, follow-up, stt) bound every state waiting on them. No stuck path found; Held mode is bounded by watchdog (5 s+) and alert cap (30 s, test `dispatcher.rs:358`).

## 3. Debug isolation: PASS
- `mod debug` (`lib.rs:2-3`), `invoke_handler` with `debug_dispatch` (`lib.rs:103-104`), `open_window` (`:141-142`), `Input::Reset/SetMedia`, `MediaMode::Held` are all cfg-gated. Release clippy clean (no dead code).
- `capabilities/debug.json` scoped to window `debug`; `default.json` (island) lacks `allow-debug-dispatch`.
- Permission generated in every profile (`build.rs:5-7`): harmless in release. With no `invoke_handler` registered, any `invoke("debug_dispatch")` fails with command-not-found, and the window `debug` is never created. No release debug path exposed.
- Note: `DebugWindow.tsx`/`events.json` are still in the release JS bundle (inert, see S2).

## 4. UI events: PASS
- `niva://view` shape tested against design JSON (`ui_adapter.rs:531`): state kebab names, caption, confirm{id=request id, prompt, expiresAt only after Confirm timer}, reminder{id,text}; optional fields omitted. `niva://island {visible}` (`:564`). `niva://cursor {x,y}` window-relative logical px (`cursor.rs`). Order island:true, view, island:false tested (`:303`, `:314`). Views published only on change (`:324`).
- Generation guard (`lib.rs:47-49, 54, 63-72`) is logically correct (show bumps, hide captures gen+1, delayed thread hides only if unchanged) but see W2.

## Findings

### CRITICAL
None.

### WARNING
- W1 `lib.rs:153-155`: tray label is "Show / hide Niva"; island-shell spec requires at least "Show island" and "Quit". Rename to include "Show island" (or amend the spec). Also in release the entry sends `Shortcut`, which under accepted CO1 shows the error pose instead of a plain show (consistent with design, but not "show island").
- W2 `lib.rs:44-76`: the show/hide generation guard (the delayed-hide fix) has no automated test; the logic is glue in `TauriHost`. Extract to a small testable type (counter + `should_hide(gen)`), as done for `CursorTracker`.
- W3 `debug.rs` / `dispatcher.rs:116-123` (Reset race): `Reset` replaces state with `State::default()`, which restarts id allocators. Events already queued from the old numbering (a fired timer, a synthesized SpeakDone) may match a new id 1 and be accepted. Debug-only, but can produce confusing debug sessions. Mitigate by draining pending inputs on reset or keeping allocators monotonic.
- W4 TDD quality (item 5): `cursor.rs` and ui/dispatcher/debug tests were not RED in isolation; `CursorPoller` start/stop thread logic (`cursor.rs:61-106`) and `TauriUiAdapter` have zero tests (only the pure parts do). Test quality of the pure parts is good (boundary values, negative origins, scaled displays, JSON shape, stale resolver, reset leaves no timers). Process deviation recorded; no behavioural defect found.

### SUGGESTION
- S1 `cursor.rs:94`: `win.emit` goes to all webviews (debug window also receives 30 Hz cursor events); use `emit_to(ISLAND_LABEL, ...)`.
- S2 Release bundle includes the debug window JS (`main.tsx:3`); lazy `import()` guarded by `import.meta.env.DEV` would drop it. Task 1.26 should still assert the Rust strings are absent (already checked manually per apply note).
- S3 `cursor.rs`: still cursor produces no events (dedup); fine and documented, but design says "30 Hz while visible"; add a design line.
- S4 Reminder alert ends at once in release under stubs (`ChimeDone`/`SpeakDone` synthesized). Consistent with design Host-runner note and mascot spec is only observable in Held mode; no conflict until slice 4, but 1.26/1.30 smoke tests must not expect a visible alert in release.
- S5 Hide thread per `hide_island` is a raw `thread::spawn` sleeping 250 ms; fine now, could be a tokio sleep.

## Deviations review (item 6)
Only the tray label (W1) conflicts with a spec. Release alert ending immediately, English caption stub, `expiresAt` timing, placeholder reminder `id: 0`, and Reset race (W3) are documented and do not violate specs.

## Next
`sdd-apply` not required; address W1/W2 cheaply in this PR or carry to 1d. Recommended: sdd-archive path continues with PR 1d.
