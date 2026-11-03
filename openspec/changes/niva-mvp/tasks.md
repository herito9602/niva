# Tasks: Niva MVP

Sources: `proposal.md`, `design.md` (incl. "Judgment Day carry-over"), `specs/*/spec.md` (11 capabilities). `spikes/` stays untouched, reference only.

## Review Workload Forecast

| Field | Value |
|-------|-------|
| Estimated changed lines | S1 ~1800-2400 (incl. ~600 copied spike lines), S2 ~2500-3200, S3 ~1500-2000, S4 ~1500-2000, S5 ~800-1200 |
| 400-line budget risk | High (every slice, S1 included) |
| Chained PRs recommended | Yes |
| Suggested split | S1: PR 1a scaffold -> 1b reducer -> 1c host -> 1d mascot; S2-S5 split further at apply time |
| Delivery strategy | ask-on-risk |
| Chain strategy | pending (no git repo yet; slice 1 creates it) |

Decision needed before apply: Yes
Chained PRs recommended: Yes
Chain strategy: pending
400-line budget risk: High

### Suggested Work Units

| Unit | Goal | Likely PR | Notes |
|------|------|-----------|-------|
| 1a | Git, workspace, copied app, test runners | PR 1 | Mostly copied spike code; candidate for `size:exception` |
| 1b | `niva-core` reducer + full matrix tests | PR 2 | Pure Rust, independently verifiable |
| 1c | Host effect runner, UiPort, cursor, debug window | PR 3 | Needs 1b |
| 1d | Mascot canvas, poses, vitest | PR 4 | Needs 1c events; can be reviewed in parallel with 1c |
| 2-5 | One slice each, subdivided at apply time | PR 5+ | Chain/base rule is set once the strategy is chosen |

Decision required: chain strategy (`stacked-to-main` or `feature-branch-chain`) or `size:exception`, asked after PR 1a creates the repo.

## Conventions

- Run cargo/Tauri only from PowerShell, never Git Bash (Git's `link` shadows MSVC `link.exe`).
- Copy the spike excluding `node_modules`, `dist`, `target`.
- From 1.6 on, reducer work is test-first: RED (matrix cell as failing test), GREEN, REFACTOR.
- Carry-over items: CO1 -> 1.14-1.15; CO4 -> 1.11 and 1.12; CO2 -> 2.6; CO3 -> 3.2. Minor items are tasks 1.12, 2.7, 2.8, 2.9.

## Slice 1: Foundation, reducer, island, mascot

### Phase 1.A: Scaffold (PR 1a)

- [x] 1.1 `git init` in `D:\trabajo\ProyectoAsistente`; create `.gitignore` (`node_modules`, `target`, `dist`, `.venv`). Done: `git status` shows no ignored dirs.
- [x] 1.2 Create root `Cargo.toml` workspace (members `crates/niva-core`, `app/src-tauri`) with `[profile.release]` moved from the spike. Done: `cargo metadata` (PowerShell) succeeds.
- [x] 1.3 Copy `spikes/island` to `app/` with `robocopy /XD node_modules dist target`. Rename product to "Niva" (`productName`, window/tray titles, bundle identifier); remove the member-level `[profile.release]`. Done: no "Niva Island" in `app/`; `spikes/` unchanged.
- [x] 1.4 Create `crates/niva-core` (`lib.rs`, empty `state_machine.rs`, `ports.rs`) with one smoke test. Done: `cargo test -p niva-core` passes.
- [x] 1.5 Add `vitest` to `app/package.json` with `npm test` script and one smoke test. Done: `npm test` passes; `npm run build` and `cargo tauri dev` still launch the island.

### Phase 1.B: Reducer, test-first (PR 1b) in `crates/niva-core/src/state_machine.rs`

- [x] 1.6 Define `State` (8 states, `voice_available` flag default `false` (set only by `VoiceReadinessChanged{ready}`), pending slot, active listen/speak ids), `Event`, `Effect`, `ReplyKind` enums and `reduce` signature returning unchanged state. Done: compiles; no logic yet.
- [x] 1.7 RED+GREEN: state-table rows (idle/listening/thinking/acting/confirmation/success/error/alert entry and exit). Spec: `assistant-core`, `island-shell` lifecycle.
- [x] 1.8 RED+GREEN: matrix rows Wake, Shortcut, SpeechStart for all 8 states, including voice-down branches. Spec: `voice-activation`.
- [x] 1.9 RED+GREEN: Transcript, stale-id drop (all id-carrying events), NoSpeech, EmptyTranscript, TextSubmitted rows. Spec: `speech-io`, `language-handling`.
- [x] 1.10 RED+GREEN: ToolCalls, ToolsDone (incl. `web_search` re-entry to thinking), Reply/Failure, AskFollowUp rows. Spec: `assistant-core`, `web-search`.
- [x] 1.11 RED+GREEN: SpeakStarted/SpeakDone/TtsFailed rows per `ReplyKind` (Final, ConfirmPrompt, AskFollowUp, Alert); watchdog-timeout row for every state. CO4: `VoiceReadinessChanged{ready:false}` while `listening` -> error plus error-template `Speak` effect (skipped by runner when voice is down) so the 3 s settle starts. Spec: `speech-io`.
- [x] 1.12 RED+GREEN: Accept/Reject, confirm unclear reissue (new id, timer not restarted), ReminderDue/queue/merge, Dismiss, `VoiceReadinessChanged{ready}` row (true and false) in all 8 states. Minor: Shortcut-as-Reject during prompt emits `StopSpeaking`. Spec: `action-safety`, `reminders`, `island-shell`.
- [x] 1.13 REFACTOR + coverage check: script or test iterating every (event, state) cell, failing if a cell has no assertion. Done: `cargo test -p niva-core` green, all 23 event kinds x 8 states covered (the original 17 rows grew with the VoiceReadinessChanged, Accept/Reject and SpeakDone/TtsFailed splits and ChimeDone).

### Phase 1.C: Host runner and debug (PR 1c) in `app/src-tauri/src/`

- [x] 1.14 CO1: `effect_runner.rs` minimal runner: `StartTimer`/`CancelTimer` (tokio tasks feeding `Watchdog` events), settle 3 s, `ShowIsland`/`HideIsland`, `Chime` as no-op log, other effects logged. Test: runner unit test with paused tokio time (settle fires at 3 s). Runner contract (W2): arm a speak watchdog for every `Speak` (token = speak id) so a `Speak` always ends in `SpeakDone`/`TtsFailed`/watchdog, and test it; `StartTimer{kind, token}` must be echoed back in `Watchdog{kind, token}`; `Chime{id}` must report `ChimeDone{id}`.
- [x] 1.15 CO1 decision: reducer default `voice_available=false`; debug builds dispatch `VoiceReadinessChanged{ready:true}` at startup (the real supervisor arrives in slice 3); release slice 1 keeps it false, so shortcut shows the `error` pose (accepted until slice 3). Test: reducer test for both paths; document in `design.md` note.
- [x] 1.16 `ui_adapter.rs`: `TauriUiAdapter` emitting `niva://view` and `niva://island` from reducer output. Done: events visible in webview console.
- [x] 1.17 `cursor.rs`: `GetCursorPos` poller at 30 Hz while visible, emits `niva://cursor` window-relative. Done: coordinates logged on move.
- [x] 1.18 Wire the spike's shortcut and tray to dispatch `Shortcut` into the reducer through the runner. Done: shortcut shows/hides island via state.
- [x] 1.19 `debug.rs` (`cfg(debug_assertions)`): create debug window in code, `debug_dispatch{event}` command, `capabilities-debug/debug.json` scoped to that window label (moved out of `capabilities/` in Batch 5 so release never loads it). Done: command absent in release build (checked in 1.26).
- [x] 1.20 `app/src/debug/DebugWindow.tsx`: one button per event, per-state shortcut buttons, voice ready/down toggle. Done: clicking each button changes the state shown.

### Phase 1.D: Mascot (PR 1d) in `app/src/`

- [x] 1.21 `state/viewStore.ts` consuming `niva://view`/`niva://island`/`niva://cursor`; vitest for reducer-free store updates.
- [x] 1.22 `mascot/spring.ts` damped spring + vitest (convergence, no overshoot blowup).
- [x] 1.23 `mascot/pose.ts` pose table for all 8 states + vitest (every state has a pose; thinking pose kept during AskFollowUp). Spec: `mascot`.
- [x] 1.24 `mascot/robot.ts` + `MascotCanvas.tsx`: procedural head, capsule eyes, antenna, breathing, blinks 2-6 s, cursor-following eyes; no image assets. Done: visual check of 8 states via debug window. (code done; owner visual check pending)
- [x] 1.25 Look review of mascot against the proposal's "derivative" risk (no Coucou code, shapes, name, media). Done: noted in PR. (code and originality note done; owner look review pending)

### Phase 1.E: Slice 1 done check

- [x] 1.26 (a) Dev: `cargo test`, `npm test` pass and all 8 states visible from the debug window. (b) Release: `cargo tauri build` from PowerShell, run the exe, shortcut shows island (error pose), no debug window, no `debug_dispatch` (`npm run check:release` in `app/`). Note: under the slice-1 stubs the release reminder alert ends instantly, so this smoke test must not expect a visible alert.
  - Done 2026-11-03: `cargo test --workspace` 86 + 161 and `npm test` 149 passed on main; 8 states checked by the owner in the debug window during PR 1d. Release built with `npm run tauri build -- --no-bundle` (no installer, so WiX/NSIS are not downloaded yet; installer deferred to packaging), `npm run check:release` ok. Owner ran the exe: island drops with the error pose and "Voice is not ready" caption, no debug window. The smoke test found the caption overflowing the pill (`white-space: nowrap`); fixed in `app/src/styles.css` (wraps to 2 lines, then ellipsis) and re-checked by the owner on a rebuilt exe.

## Slice 2: Assistant core, tools, text input

- [ ] 2.1 `ports.rs`: `LlmProvider`, `PcActions`, `Clock`, `UiPort` plus fakes; `replies.rs` es/en templates (tests for every D4 template case).
- [ ] 2.2 `safety.rs` + `tools.rs`: registry (Safe/Risky), `Approved<T>` crate-private constructors, substring matcher, protected list, min length 3. Tests: risky never runs without `Approved`, power restart/shutdown click-only. Spec: `action-safety`.
- [ ] 2.3 `assistant.rs` tool loop with fake LLM: validation before execution, pending slot, confirmation flow, whole-utterance yes/no matcher, timeouts. Spec: `assistant-core`.
- [ ] 2.4 Adapters `ollama.rs` (health, backoff, warm-up, `/api/ps` slow mode), `win_apps.rs`, `win_volume.rs`; `#[ignore]` integration tests. Spec: `pc-actions`.
- [ ] 2.5 Debug window text box and release "Type a command" window (`textwindow/TextWindow.tsx`, `text_window.rs`, `capabilities/text-window.json`, tray entry). Spec: `island-shell`, `language-handling` text heuristic.
- [ ] 2.6 CO2: RED test then rule for `web_search` mixed with a risky call in one response. Precedence: reject the mix at validation with an error template, nothing executed.
- [ ] 2.7 Minor test: `confirm.expiresAt` absent until `speak_done`, set after.
- [ ] 2.8 Minor test: second ambiguity in a `close_app` follow-up fails after one turn (same as reminders).
- [ ] 2.9 Minor: remove or demote the "6 LLM steps" cap (single-response-only) and fix the design text.
- [ ] 2.10 `eval/commands.yaml` (~40 cases) and `bin/niva-eval.rs`; run E1 and record gates. Decide the product LLM.

## Slice 3: Voice sidecar

- [ ] 3.1 Spike S1 (STT GPU) and S2 (TTS): record results in `spikes/` notes outside existing spike dirs or `docs/`.
- [ ] 3.2 CO3: RED tests then `SidecarError{code}` row: listening -> error with code template; confirmation-needed -> click-only; list which codes set `voice_available=false` (`MIC_MISSING`, `MIC_BUSY`, `CUDA_OOM`, `MODEL_LOAD`, `PROTOCOL`, `INTERNAL`; `TTS_FAILED` stays `TtsFailed`).
- [ ] 3.3 `voice/` uv project: `protocol.py`, `wake.py`, `vad.py`, `stt.py`, `tts.py`, `main.py`; pytest for protocol and VAD fixtures. Spec: `voice-activation`, `speech-io`.
- [ ] 3.4 `adapters/voice_sidecar.rs`: JSON lines, restart counter, OOM downgrade, degraded mode, "Restart voice" tray entry; `audio_cue.rs` rodio adapter.
- [ ] 3.5 Per-stage latency log; manual E2E checklist with real microphone audio for the < 2.5 s action-path mean and the end-to-end p95 <= 3.0 s goal (estimate ~2.71 s at p95: VAD 400 unmeasured + STT 203 + LLM 1110 + tool 100 + TTS 900).

## Slice 4: Reminders and fullscreen

- [ ] 4.1 `reminders.rs` time resolution (tests first: past time -> tomorrow, past date/non-positive delay rejected, DST) and `sqlite_reminders.rs` (temp-file tests). Spec: `reminders`.
- [ ] 4.2 Scheduler (sleep to `min(next_due, 60 s)`), queue/coalesce, `fired_at` after show, missed-on-start, autostart plugin.
- [ ] 4.3 Spike S5, then `win_fullscreen.rs` per truth table; alert over fullscreen with chime, re-alert once after exit. Spec: `fullscreen-policy`.

## Slice 5: Search, wake word, installer

- [ ] 5.1 Spike S3, then `adapters/search.rs` behind `SearchProvider`; injection-safe summary call (no tools, capped). Spec: `web-search`.
- [ ] 5.2 Spike S4: train "hey niva", apply threshold procedure, ship only if it passes. Check stock-model license.
- [ ] 5.3 Installer: decide sidecar packaging, bundle sidecar, `cargo tauri build` smoke test on a clean PC. Spec: `voice-activation`.

## Delivery decision (2026-11-02)

Owner chose chained delivery for slice 1: 4 PRs (1a scaffold, 1b reducer, 1c host, 1d mascot), `chain_strategy: stacked-to-main`, `delivery_strategy: ask-on-risk`. PR 1a is a bootstrap exception (it creates the repo).

## Apply progress

### Batch 1: Phase 1.A Scaffold (PR 1a), tasks 1.1-1.5 done (2026-11-02)

- Repo: `git init` + `.gitignore` only; nothing committed, no branches/remotes. `spikes/` untouched.
- Workspace root `Cargo.toml` (resolver 3, members `crates/niva-core`, `app/src-tauri`, `[profile.release]` moved here). `Cargo.lock` moved from `app/src-tauri` to the root.
- `app/` copied from `spikes/island` (robocopy `/XD node_modules dist target`). Renames: productName "Niva", identifier `dev.niva.app`, npm package `niva`, Rust package `niva`, lib `niva_lib`, `index.html` title "Niva". Window label `island` and the `island:*` events are kept (spike behaviour unchanged). Spike `RESULTS.md` not copied; `app/README.md` rewritten.
- `crates/niva-core`: `lib.rs` (smoke test), empty `state_machine.rs` and `ports.rs`.
- `app/`: `vitest` devDependency, `npm test` = `vitest run`, `src/smoke.test.ts`.
- Environment note: the PowerShell tool was not available; cargo/npm ran via `powershell.exe -NoProfile` invoked from Bash.
- Done checks: `cargo metadata` ok; `cargo test -p niva-core` 1 passed; `npm install`, `npm test` (1 passed), `npm run build` ok; `cargo build -p niva` (debug) ok; `target\debug\niva.exe` launched 4 s then killed, no process left. `cargo tauri dev` not run (build + brief launch only).
- Proposed commit plan (conventional, no attribution):
  1. `chore: initialize repository with gitignore and cargo workspace`
  2. `feat(core): add niva-core crate skeleton with smoke test`
  3. `feat(app): import island spike as app and rename product to Niva`
  4. `test(app): add vitest runner with smoke test`
  (`.atl/`, `docs/`, `openspec/`, `spikes/` are pre-existing untracked content; decide separately.)

### Batch 2: Phase 1.B Reducer (PR 1b), tasks 1.6-1.13 done (2026-11-02)

- Branch `feat/1b-reducer`, nothing committed. Placeholder smoke test removed from `lib.rs`.
- Files: `crates/niva-core/src/state_machine.rs` (types, `reduce`, handlers, `classify_confirmation`), tests in `state_machine/{tests,tests_activation,tests_input,tests_tools,tests_speech,tests_confirm,coverage}.rs`. `lib.rs` lost its smoke test.
- Strict TDD: each task started with a failing test run (1.6 and 1.7 failed to compile on the missing API; 1.8 15 failed, 1.9 11 failed, 1.10 13 failed, 1.11 19 failed, 1.12 14 failed), then the minimal code. 1.13 REFACTOR: `cargo fmt -p niva-core` plus the coverage test.
- Design: `transition()` is the single door between states; it cancels the leaving state's timers, open listen, speech (and the alert chime) and the in-flight request unless the next state continues it. Reducer-owned timers are tracked in `State.timers`; a `Watchdog` for a timer that is no longer running is a stale fire and is ignored.
- Coverage (1.13): `coverage.rs` has one `[Expect; 8]` row per `EventKind` (22 kinds = the matrix rows with VoiceReadinessChanged split in down/up, Accept/Reject and SpeakDone/TtsFailed split), 176 cells all asserted, plus state invariants after every cell. Adding an `Event` variant, an `EventKind` or a `Phase` breaks the build; verified by deleting a row (E0004), adding a variant (E0004) and flipping an expectation (test fails).
- Done checks: `cargo test -p niva-core` 120 passed; `cargo clippy -p niva-core --all-targets -- -D warnings` clean.
- Matrix ambiguities resolved (safest choice), also commented in code:
  1. Effect set extended: `StartRequest{req,input}` (reducer allocates request ids; late LLM results are dropped by id) and `ForceEndOfSpeech{id}` (max-utterance cap). Neither was in the design list.
  2. `ToolCalls` carries a service verdict (`RunSafe | Confirm{prompt,click_only} | Invalid`); the "missing argument / ambiguity" outcome is the separate `AskFollowUp` event, as in the matrix.
  3. Reminder alert ends only by click or the 30 s cap, never by `SpeakDone` alone (the matrix says "later of chime/speech done, capped 30 s", but the chime has no done event and a skipped/synthesized speak would hide the reminder at once).
  4. One follow-up turn is enforced in the reducer (`followup_turn`): an `AskFollowUp` raised while serving a follow-up fails the request (error template). Matches "anything still missing after that one turn ends the request"; makes task 2.8 reducer-level.
  5. STT timeout after a forced end of speech uses 10 s (`TimerKind::SttTimeout`); the design gives no duration. The matrix has no end-of-speech event, so the STT timer only starts when the 15 s cap forces the end.
  6. `SpeakStarted` is a no-op in every state (only the stale-id check applies).
  7. `TtsFailed` and the synthesized speak watchdog behave exactly like `SpeakDone` (`Watchdog{SpeakWatchdog}` with an active speak id).
  8. `Wake` in success/error is ignored while a speak is active (self-trigger guard) and when voice is down; in alert it needs voice.
  9. Voice down while confirming drops the confirm listen id without a `CancelListen` (the sidecar is down) and marks the confirmation click-only; voice up never reopens it.
  10. Queued reminders: `ReminderDue` outside idle/alert emits `EnqueueReminder{count}`; every entry to idle emits `DrainReminders` after `HideIsland`.
  11. Leaving the alert always emits `StopSpeaking` (it also stops the chime).
  12. Text templates are ids (`Template`), rendered by the host; LLM replies and prompts travel as `Say::Text`.
- Size: ~3650 lines added (about 700 production, 2900 tests), far over the 400-line budget (forecast "High"); needs `size:exception` or per-commit review as below.
- Proposed commit plan (conventional, no attribution), tests with code in each unit:
  1. `feat(core): add reducer types and state-table transitions` (1.6, 1.7, remove smoke test)
  2. `feat(core): handle wake, shortcut and speech start with the voice guard` (1.8)
  3. `feat(core): handle transcripts, no-speech and text input` (1.9)
  4. `feat(core): handle tool calls, replies and follow-up questions` (1.10)
  5. `feat(core): handle speak completion, watchdogs and voice loss while listening` (1.11)
  6. `feat(core): handle confirmation, reminders, dismiss and voice readiness` (1.12)
  7. `test(core): assert every event x state cell of the matrix` (1.13, fmt)
  (The work is in one working tree; splitting needs `git add -p`. Alternatively squash to commits 1, 6-style groups.)

### Batch 3: PR 1b review fixes (2026-11-02)

- Owner-approved fixes after the verify and reliability reviews; nothing committed. Strict TDD: new tests written first (compile error on the missing `Timer`/`token`/`ChimeDone`/`silent` API = RED), then the code (GREEN), then `cargo fmt` (REFACTOR).
- Reducer: speak watchdog bound to the speak id (`Watchdog{kind, token}`; token = speak id for `SpeakWatchdog`); timer generations (`State.timers: Vec<Timer{kind, token}>`, `StartTimer{kind, token}`, stale fires dropped in `is_stale`); alert end = later of chime done and speech done (new `Event::ChimeDone{id}`, `Effect::Chime{id}`, `State.chime_id`), 30 s cap or click; `ReminderDue{count, silent}` (silent = chime only); `ToolCalls` while summarizing = `Invalid`; click-only confirmations ignore spoken answers (S2) and the confirm reissue needs `voice_available && !click_only` (S3); exhaustive matches in `on_no_speech`, `on_empty_transcript`, `on_watchdog` (kind x phase) and `on_speak_done` (by reply kind); `alert_count` saturates, `count == 0` ignored; `debug_assert!` on accept without a request.
- Tests: new `tests_races.rs` (id/generation guards, double events, watchdog kind x phase sub-table, voice-down sequences, silent reminders, alert end, S2/S3, determinism) and `tests_scenarios.rs` (3 end-to-end sequences with invariants after every step); fixtures `prompt_playing`, `question_playing`, `waiting_for_text`, `alert_playing` and `assert_invariants` moved to `tests.rs` with id constants; coverage gained the `ChimeDone` row (23 kinds, 184 cells).
- Docs: `design.md` gained the "Reducer contract" section plus edits to the effects list, matrix rows (ReminderDue, ChimeDone), alert end, speak watchdog (runner contract, W2) and follow-up turn (second AskFollowUp -> error). Batch 2 note 3 (alert never ends on speech alone) is superseded by the alert end rule above.
- Follow-up (coordinator decision): `ToolCalls` while summarizing now fails with the search template (`SearchUnavailable`, via `fail_request`) per D4, test first; the phase-guard handlers (`on_speech_start`, `on_accept`, `on_reject`, `on_dismiss`, `on_tool_calls`, `on_tools_done`, `on_reply`, `on_failure`, `on_ask_follow_up`) are now exhaustive matches; the inner phase checks of `on_speak_done` stay `if`s (nested under the reply-kind match, a 7-phase list would hurt readability).
- Done checks: `cargo test -p niva-core` 160 passed (was 120), clippy `-D warnings` clean, `cargo fmt --check` clean.

### Batch 4: Phase 1.C Host (PR 1c), tasks 1.14-1.20 done (2026-11-02)

- Branch `feat/1c-host` (from `main` after PR #1); nothing committed. Strict TDD for the logic, glue verified by build plus a real dev run.
- New in `app/src-tauri/src/`: `dispatcher.rs` (one `Input` queue -> `reduce` -> UI events -> runner; `Core` is synchronous and testable, `run` is the async loop; `startup_events`), `effect_runner.rs` (`EffectRunner`, `Host`, `MediaMode`, `speak_watchdog_duration`), `ui_adapter.rs` (`ViewModel`, `UiEvent`, `UiPort`, `TauriUiAdapter`, English `template_caption` stub), `cursor.rs` (`window_relative`, `CursorTracker`, `CursorPoller`), `debug.rs` (cfg(debug_assertions): `DebugEvent`, `debug_dispatch`, `open_window`). `lib.rs` rewired: shortcut and tray (double-click and a "Show / hide Niva" item) send `Event::Shortcut` through the dispatcher; the spike's direct show/hide is now the `TauriHost` behind `ShowIsland`/`HideIsland` (a generation counter stops a delayed hide from hiding a re-shown window).
- Also: `build.rs` declares `debug_dispatch` in the app manifest (generates `allow-debug-dispatch` in every profile; `capabilities/debug.json` targets only window `debug`; generated `permissions/autogenerated/` is gitignored); `Cargo.toml` gained `niva-core` and `tokio` (rt, time, sync, macros; dev: test-util). No other dependency. Frontend: `src/events.ts` (event names and payload types), `src/island.ts` (label, mode, slide phase; spike visuals kept), `App.tsx` now listens to `niva://view|island|cursor` (and logs them to the webview console), `main.tsx` picks `DebugWindow` for `?window=debug`, `src/debug/{DebugWindow.tsx,debug.css,events.ts,events.json}`. `events.json` (buttons and per-state scripts) is shared: vitest checks its shape, a Rust test parses every entry as a `DebugEvent` and replays each script through the real reducer to its state. Design note added to `design.md` ("Host runner (added in PR 1c)").
- No change to `niva-core` (no API addition needed). `TimerKind` lacks `Hash`, so the runner keeps timers in a `Vec`.
- TDD evidence: effect_runner (RED = first run failed to compile on the missing API, then 8 timing failures caught the "deadline taken at first poll" bug, fixed with `sleep_until` computed at `StartTimer`); ui_adapter, dispatcher, debug and the vitest files were written test-first but not run on their own before the implementation existed (RED only as compile failure of the whole crate); `cursor.rs` tests and implementation were written in one edit (not strictly test-first, honest note).
- Stubs (logged, documented in design.md): `Speak` has no TTS -> `SpeakDone` synthesized at once (`MediaMode::Instant`) while the speak watchdog is armed first; `Chime` -> `ChimeDone` at once; `Listen`/`CancelListen`/`ForceEndOfSpeech`/`StartRequest`/`RunTool`/`CancelRequest`/`EnqueueReminder`/`DrainReminders` only log. Consequence: in release a reminder alert ends immediately (visible only in the debug window's Held mode).
- Debug extras beyond the task text: `Reset` (fresh idle state keeping the voice flag, cancels all timers) and `SetMedia{held}` (Held = nothing synthesized, so states stay visible; the window defaults to Held), both `cfg(debug_assertions)`.
- Done checks (PowerShell): `cargo test --workspace` 65 (niva) + 160 (niva-core) = 225 passed; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo fmt --check` clean; `npm test` 3 files / 13 tests passed; `npm run build` ok.
- Dev run (not interactive-by-hand): vite + `target\debug\niva.exe` with WebView2 remote debugging; a CDP script clicked every "go to state" button in the debug window and read the state back: idle, listening, thinking, acting, confirmation-needed (caption "Close Notepad?"), success, error, reminder-alert (reminder "1 reminder due") all reached; Shortcut/Transcript/Reply buttons worked; the island console logged `niva://view`, `niva://island` and `niva://cursor` (coordinates). No panics on stderr; processes killed afterwards. The real global shortcut keypress and the tray clicks were NOT exercised by hand (they use the same `send_event` path).
- Release: `cargo build --release -p niva` ok, no warnings; `debug_dispatch`, `window=debug` and `Niva debug` are absent from `target\release\niva.exe` (present in the debug exe); launched 5 s: single `island` webview, no debug window, no stderr output; process killed. `cargo tauri build` (bundle) not run (that is 1.26).
- Size: about 3,300 lines added (Rust ~2,640 incl. ~1,500 test lines, TS/JSON/CSS ~420); far over 400, same `size:exception` situation as PR 1b.
- Proposed commit plan (conventional, no attribution):
  1. `feat(app): add effect runner with timers, speak watchdog and chime completion` (1.14: `effect_runner.rs`, tokio deps, minimal `ui_adapter::template_caption`)
  2. `feat(app): add dispatcher and startup voice readiness policy` (1.15: `dispatcher.rs`, design.md note)
  3. `feat(app): publish reducer output as niva view and island events` (1.16: `ui_adapter.rs`, `events.ts`, `island.ts`, `App.tsx`)
  4. `feat(app): poll the cursor while the island is visible` (1.17: `cursor.rs`)
  5. `feat(app): route shortcut and tray through the reducer` (1.18: `lib.rs`)
  6. `feat(app): add debug window and debug_dispatch command for debug builds` (1.19, 1.20: `debug.rs`, `build.rs`, `capabilities/debug.json`, `src/debug/*`, `main.tsx`, `.gitignore`)
  (Commits 1-3 share `ui_adapter.rs`/`lib.rs`; splitting needs `git add -p`, or squash to 3 commits: runner+dispatcher, ui+cursor+wiring, debug.)

### Batch 5: PR 1c review fixes (verify-report-1c W1-W4, S1-S5 plus risk review) (2026-11-02)

- Merged on top of Batch 4; nothing committed. Strict TDD for logic (test written and run RED first only for the `State::reset_from` core change; the host-side tests below were written together with their extraction, as in Batch 4, and are honest about it).
- Security: island capability is now `core:event:allow-listen` + `core:event:allow-unlisten` only (verified: the island JS uses only `listen`); debug capability moved to `capabilities-debug/debug.json` (listen, unlisten, `allow-debug-dispatch`). `build.rs` keys on `CARGO_CFG_DEBUG_ASSERTIONS` (`cfg!` in a build script describes the script, `PROFILE` ignores overrides): debug loads `capabilities*/` and declares `debug_dispatch`; release loads only `capabilities/` and points the app permissions glob at a non-existent directory. Reason for the last part (found by checking the binary, not by reading code): `permissions/autogenerated/allow-debug-dispatch` written by any earlier debug build is globbed into the release ACL, so the first release exe still contained `debug-dispatch`. `app/scripts/check-release-binary.mjs` (`npm run check:release`) fails if the release exe contains `debug_dispatch` or `debug-dispatch`; `capabilities_tests.rs` asserts the capability contents (island: no global-shortcut, emit, debug, window, `core:default`), that `capabilities/` holds only the island file, and that the build script's release branch has no debug pieces.
- CSP set in `tauri.conf.json` (`csp` strict; `devCsp` also allows Vite http/ws on 1420). Verified with a real WebView2 session over CDP: debug build (island and debug window render, no CSP console errors, island is denied `debug_dispatch`, `plugin:event|emit`, `global-shortcut|register` and external fetch) and a `--features tauri/custom-protocol` release build (renders from `tauri.localhost`, stylesheet loaded, no console errors, `debug_dispatch` command not found). Plain `cargo build --release` without that feature still loads `devUrl`; a real bundle is `cargo tauri build` (1.26).
- Events are targeted (`emit_to`): `event_targets` sends everything to `island`, and `niva://view` also to `debug` in debug builds only; `TauriUiAdapter` now takes an `EmitTo` trait object (fake in tests: targets, payloads, release has no debug target, one failing window does not block the other). Cursor poller split into `CursorSource`/`PollControl`/`poll_once`/`poll_loop` (tests: still cursor and failed samples emit nothing, double start, stop/restart generations, loop exits on stop).
- `debug.rs`: `Resolver` returns `Option<Event>` (no `expect`; `None` is a no-op, tested), free-form text capped at 500 chars (tested), SECURITY comment on `DebugEvent` (never add tool-execution or confirmation variants reachable from a release webview; Accept/Reject only from native paths).
- `lib.rs`: shortcut registration failure is logged and no longer aborts setup (`shortcut_status`, tested); delayed-hide guard extracted to `visibility.rs::HideGuard` (tests: re-show during the delay cancels the hide, only the latest hide wins, clones share the counter). Tray item renamed "Show / hide island" (W1); `island-shell` spec amended to "a show/hide toggle for the island and a Quit entry"; noted in design.md.
- `niva-core`: `State::reset_from(&previous)` keeps voice flag and the id/token allocators; debug `Reset` uses it (tests in core and dispatcher: ids after a reset are greater than before). This is a small public API addition to `niva-core` (W3).
- `main.tsx`: `DebugWindow` is imported lazily behind `import.meta.env.DEV`; the production bundle has no debug chunk (single JS file, no `debug_dispatch` string in `dist`).
- Docs: design.md "Host runner" gained debug-command security, least-privilege/CSP, tray, cursor dedup (S3) and Reset bullets; tasks 1.19 path updated; 1.26 notes that the release reminder alert ends instantly under stubs (S4). S5 (raw `thread::spawn` for the hide delay) left as is.
- Done checks (PowerShell): `cargo test --workspace` 86 (niva) + 161 (niva-core) passed; `cargo clippy --workspace --all-targets -- -D warnings` clean; `cargo clippy --release -p niva -- -D warnings` clean; `cargo fmt --check` clean; `npm test` 13 passed; `npm run build` ok; `cargo build --release -p niva` ok and `npm run check:release` ok (no `debug_dispatch`/`debug-dispatch` in `targetelease
iva.exe`). No processes left running.
- Updated commit plan (replaces Batch 4's step 6; capability files moved):
  1. `feat(app): add effect runner with timers, speak watchdog and chime completion` (1.14)
  2. `feat(app): add dispatcher and startup voice readiness policy` (1.15; includes `State::reset_from` use in `Reset`, so commit `niva-core` `reset_from` + test here or as its own `feat(core): add State::reset_from keeping id allocators`)
  3. `feat(app): publish reducer output as targeted niva view and island events` (1.16: `ui_adapter.rs`, `events.ts`, `island.ts`, `App.tsx`)
  4. `feat(app): poll the cursor while the island is visible` (1.17: `cursor.rs`)
  5. `feat(app): route shortcut and tray through the reducer` (1.18: `lib.rs`, `visibility.rs`, tray label, spec wording)
  6. `feat(app): add debug window and debug_dispatch command for debug builds` (1.19, 1.20: `debug.rs`, `build.rs`, `capabilities-debug/`, `src/debug/*`, lazy import in `main.tsx`, `.gitignore`)
  7. `feat(app): least-privilege capabilities, CSP and release binary check` (`capabilities/default.json`, `tauri.conf.json`, `capabilities_tests.rs`, `scripts/check-release-binary.mjs`, `package.json`)
  8. `docs(openspec): record PR 1c verify report, review fixes and design notes`
  (Files are shared across steps; splitting needs `git add -p`, or squash to 4 commits: runner+dispatcher(+core), ui+cursor+wiring, debug, security+docs.)

### Batch 6: Phase 1.D Mascot (PR 1d), tasks 1.21-1.25 done in code (2026-11-02)

- Branch `feat/1d-mascot`; nothing committed. Strict TDD: the four test files (`viewStore`, `spring`, `pose`, `robot`) were written first and run RED (all four failed on the missing modules, 13 old tests green), then the modules were written and went GREEN on the first run. Not strictly one test at a time; `MascotCanvas.tsx` and `drawRobot` are glue verified only by `tsc` and build.
- New in `app/src/`: `state/viewStore.ts` (`createViewStore` with plain setters, `subscribe`/`getState` snapshot, `bindHostEvents(store, listen)` with an injectable `Listen`; no reducer logic, the Rust core owns state), `mascot/spring.ts` (`stepSpring`, `criticalDamping`; semi-implicit Euler with 1/240 s sub-steps, dt clamped to 0.1 s so a hidden tab is stable), `mascot/pose.ts` (`POSES` for the 8 states, `poseForView`; null view = idle; AskFollowUp keeps `thinking` because the host keeps that state), `mascot/robot.ts` (rig of one spring per pose key plus gaze, `createBlinker(rng, start)` 2-6 s, `lookVector`, `composeFrame`, `drawRobot`), `mascot/MascotCanvas.tsx` (rAF loop, only while the island phase is `in`, DPR-aware). `App.tsx` now reads the store through `useSyncExternalStore` (cursor is read per frame, not React state) and the spike's console logging is gone. The spike orb CSS and `islandMode` (dead after the orb) were removed with their test.
- Reduced motion: the spec does not mention it; implemented as a cheap extra. Under `prefers-reduced-motion` breathing, bounce and antenna sway are off; pose transitions, blinks and gaze still play. The loop reads the media query each frame, so changing the OS setting applies live.
- Capabilities/CSP untouched (the island still only listens). No new dependency, no network, no asset (Canvas 2D primitives only; `dist` has one JS and one CSS file).
- Gaze geometry: the cursor payload is window-relative logical px and the webview fills the window, so the canvas centre comes from `getBoundingClientRect()`; during the drop-in animation the eyes are measured against the moving canvas, which is harmless.
- Known limitation: `bounceHz` is a spring parameter and the hop is `sin(pi * hz * t)`, so a hz change between two bouncing states (success -> reminder-alert) causes a small phase jump inside the ~0.2 s transition. Accepted; an integrated phase would fix it if the owner sees it.
- Done checks: `npm test` 7 files / 49 tests (37 new in 4 files: viewStore 8, spring 7, pose 7, robot 15; the old 13 became 12 after dropping the `islandMode` test); `npm run build` ok; `cargo test --workspace` 86 (niva) + 161 (niva-core) passed; `cargo clippy --workspace --all-targets -- -D warnings` clean. `cargo tauri build` not run (1.26).
- Not done: nobody has looked at the 8 poses yet. The owner must run the debug build, click the 8 state buttons and judge the look (1.24 visual check) and originality (1.25 review). Pose numbers in `pose.ts` and the shape constants in `drawRobot` are the knobs.
- Size: about 905 lines added and 71 removed (`git diff --stat` on tracked files: 20+/71-; new files 884 lines of which 365 are tests). Over 400, same `size:exception` situation as 1b/1c.
- Originality note (for the PR, task 1.25): the mascot was designed and written from scratch in this change; no code, shapes, proportions, names or media from Coucou (github.com/louis-cfm/coucou) or Hitodama were read, fetched or copied. Its identity: a rounded-square head (68 x 59 units, 21 radius) with a dark visor panel, two glowing capsule eyes that tilt and squash, two small side plates and a bulb-tipped antenna, drawn with Canvas 2D primitives and a cool slate body. Each of the 8 states has its own accent colour (cyan idle, teal listening, violet thinking, blue acting, gold confirmation, green success, red error, magenta alert) plus eye shape, head tilt, antenna behaviour and bounce, so state is readable by colour alone. Only the generic technique (damped springs and procedural drawing, noted in design.md) is shared with Coucou; the springs are textbook physics and the table of poses, palette and geometry are ours. Final sign-off is the owner's human look review.
- Proposed commit plan (conventional, no attribution):
  1. `feat(app): add view store fed by the niva host events` (1.21: `state/viewStore.ts` + test)
  2. `feat(app): add damped spring and per-state pose table for the mascot` (1.22, 1.23: `spring.ts`, `pose.ts` + tests)
  3. `feat(app): draw the procedural Niva robot on the island` (1.24, 1.25: `robot.ts`, `MascotCanvas.tsx`, `App.tsx`, `island.ts`, `island.test.ts`, `styles.css`, this note)

### Batch 7: PR 1d owner feedback (hands and gestures) and verify-report-1d fixes (2026-11-03)

- Nothing committed; branch `feat/1d-mascot`. Strict TDD: tests for `gesture.ts`, the gesture table, composeFrame additions, non-finite guards, bounce phase, canvas helpers and viewStore rejection paths were written first and run RED (18 failing tests plus the missing `gesture` module), then the code went GREEN. The dev app was left running by the owner and was not touched.
- Owner visuals. `mascot/gesture.ts` (new, pure): `handPositions(weights, t, motion)` blends rest, scratch (left hand at the upper left of the head, 3 strokes/s), thumbs-up (right hand beside the head), typing (both hands over the keyboard, 4 taps/s, half a cycle apart) and raised (right hand) by weight; `spiralAngle`, `spiralPoints`. Every loop has a 1 s period; `motion = 0` freezes them. `pose.ts`: `GESTURES` per state (`hands: rest|scratch|thumbsUp|typing|raised`, `eyes: capsule|happyArcs|spiral`, `prop: none|laptop`), encoded as 0/1 weights in the numeric pose (`handScratch`, `handThumbs`, `handTyping`, `handRaise`, `eyeArcs`, `eyeSpiral`, `propLaptop`) so the same springs cross-fade; plus `shakeAmp` (success chuckle, 6.5 Hz) and `wobbleAmp` (error head wobble, 2.4 Hz); `gestureForState`/`gestureForView` (thinking keeps its gesture during AskFollowUp). `robot.ts`: arms and mitt hands, thumb, laptop deck with keys (slides in with its weight), "^ ^" arcs, rotating spiral eyes (opposite directions), frame gets `offsetX`, `tilt`, `hands`, `spiralAngle`. Mapping: thinking = scratch; success = thumbs-up + arcs + chuckle; error = spirals + wobble; acting = laptop + typing; confirmation-needed = raised hand; idle, listening, reminder-alert = rest.
- Reduced motion: scratching, typing, thumb bob, hand sway, chuckle, wobble, spiral rotation, bounce, breathing and antenna sway are all static; blends, blinks and gaze still play.
- Drawing space grew from 100 to 120 units and the default canvas from 64 to 72 px (hands, laptop and the alert hop were clipping; the old design already clipped the antenna by about 6 units at the reminder-alert peak).
- Review fixes. W1: `MascotCanvas` re-sizes the backing store (`backingStoreSize`) from a `ResizeObserver` and a re-armed `(resolution: Ndppx)` media query, both cleaned up on unmount, and re-applies the context transform. W2: `makeContextGetter` warns once if `getContext('2d')` is null (tested). W3: the early-unbind test now unbinds before `listen` resolves and checks the late unlisten (3 calls). W4: `bindHostEvents(store, listen, onError)` catches rejected `listen` and throwing unlisten (tested). S1: `stepSpring` ignores non-finite targets and `lookVector` returns a neutral gaze for non-finite cursor, centre or range (tested). S2: the bounce phase is accumulated in the rig (`phase`, cycles mod 1) instead of `bounceHz * t` (tested for no jump when the frequency changes). S6: 1.24 and 1.25 still say the owner's visual check is pending, now also covering the new hands and gestures.
- Docs: `specs/mascot/spec.md` gained the requirement "Hands and per-state gestures (owner request, 2026-11-03)" with two scenarios; `design.md` Mascot section describes the gestures.
- Done checks: `npm test` 8 files / 78 tests (was 49: viewStore 11, spring 8, pose 11, gesture 10 new file, robot 26, island 4, plus smoke/debug files); `npm run build` ok; `cargo test --workspace` 86 + 161 passed; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- Size: mascot and state sources plus tests are now about 1650 lines in 15 new files (702 of them tests) plus 62+/77- on tracked files (`git diff --stat`). Same `size:exception` situation.
- Risks: the pose numbers, hand positions and the laptop geometry have never been seen on screen (no canvas available in vitest); the owner must check all 8 states in the running dev app. Arms are drawn behind the head from fixed shoulders at (+-23, 20), so a long reach (thumbs-up, raised) is a straight line; revisit if it looks stiff.
- Updated commit plan (replaces Batch 6's):
  1. `feat(app): add view store fed by the niva host events` (1.21: `viewStore.ts` + test, including the rejection handling)
  2. `feat(app): add damped spring and per-state pose table for the mascot` (1.22, 1.23: `spring.ts`, `pose.ts` + tests)
  3. `feat(app): add hand gesture and dizzy-eye maths for the mascot` (`gesture.ts` + test; gesture table and weights in `pose.ts` if split with `git add -p`)
  4. `feat(app): draw the procedural Niva robot with hands, laptop and gestures on the island` (1.24, 1.25: `robot.ts`, `MascotCanvas.tsx`, `App.tsx`, `island.ts`, `island.test.ts`, `styles.css`)
  5. `docs(openspec): record mascot gestures, PR 1d progress and verify report` (`spec.md`, `design.md`, `tasks.md`, `verify-report-1d.md`)

### Batch 8: PR 1d owner look feedback (hands, laptop, preview page) (2026-11-03)

- Owner: concept liked, look not ("hands should be better defined, not little white dots; the laptop it types on more defined"). Nothing committed. The owner's dev app was not running when I looked (port 1420 not listening), so for the visual check I started my own Vite on port 1431 and screenshotted the preview page with headless Edge; that process was stopped afterwards. This was the first time the poses were actually seen, and the screenshots drove three fixes (below).
- Strict TDD for the pure logic: `geometry.test.ts` (19 tests, new) and additions to `gesture.test.ts` (`typingPress`) and `robot.test.ts` (`frame.presses`) were written first and run RED (missing `geometry` module, 3 failing), then the code went GREEN. `geometry.ts` passed on its first run. Drawing is glue, verified by screenshots.
- `mascot/geometry.ts` (new, pure): `armCurve` (quadratic with an outward elbow bend), `armPolygon` (tapered tube outline), `handShape` (per-hand blend of rest, scratch, thumbs-up, raised and typing shapes: finger extension, thumb angle and length, lean), `handParts` (palm, three fingers, thumb as rotated rounded rects), `fingertip`, `laptopGeometry` (lid, lit screen with code lines, hinge, trapezoid deck, 3 x 8 key grid in perspective) and `keyUnder`. `HAND_SCALE` 1.3.
- `robot.ts` drawing is now thin glue over those shapes with one outline colour and weight: hands are slate with a light-to-dark shade, a dark outline and the state accent as an inner rim that also separates the fingers; the thumbs-up is a fist (knuckle bumps) with a tall thumb; the scratching hand sits on the head with fingers visible; arms are tapered tubes with a soft bend and a round shoulder joint, drawn over the head and the laptop; the laptop is an open laptop seen from the front with a lit accent-tinted screen and code lines, a hinge, a trapezoid deck with keys, and the typing hands on the deck with the pressed key lit in the accent (`frame.presses`, zero under reduced motion).
- Fixes found by looking at the screenshots: the thumb was hidden behind the knuckles (thumb length 12 -> 18 for the thumbs-up); the typing hands were clipped by the canvas bottom (laptop made 4 units shorter, hands moved up, typing fingers shortened so the fingertip lands on the first key row); the 3x preview overflowed its grid cell (cell min width 450 px).
- Size and layout: drawing space 150 units, canvas 96 px (was 120 units, 72 px). Island padding 14/24 -> 6/22 px so the pill is 110 px high; with the 6 px margin and the drop-in overshoot (cubic-bezier peak 1.098 over a 160% travel, about 17 px) the bottom reaches about 133 px, inside the 140 px window. Not checked inside the real Tauri window.
- Preview page: `app/mascot-preview.html` + `app/src/mascot/preview.tsx`; all 8 states at 96 px (the island size) and 288 px on a dark background, animated, labelled, with a reduced-motion checkbox (`MascotCanvas` got an optional `reducedMotion` override) and the eyes following the pointer. Served by the dev server at http://localhost:1420/mascot-preview.html. It is not imported by `main.tsx` and Vite builds only `index.html`: `npm run build` output (`dist/`: index.html, one JS, one CSS, icons) contains no preview (searched for its title and file name).
- Docs: `specs/mascot/spec.md` hands requirement now states the readability expectations (mitt with thumb and fingers, outline, accent rim, tapered arms, fist thumbs-up, laptop with lit screen, hinge, deck, key highlight); `design.md` Mascot section describes `geometry.ts`, the 150-unit/96 px layout and the preview page.
- Done checks: `npm test` 9 files / 99 tests (was 78); `npm run build` ok, preview excluded from `dist/`; `cargo test --workspace` 86 + 161 passed; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- Size: mascot, state and preview sources plus tests are about 2,370 lines in 20 new files, plus 81+/78- on tracked files (`git diff --stat`). Same `size:exception` situation.
- Risks: still no look sign-off from the owner. At the 96 px island size the hands are about 12 px wide with visible fingers but small details (laptop code lines, key press) are barely visible there; they are meant for the 3x view. The typing hands cover most of the keys. The arm is a flat tube, no real elbow joint.
- Updated commit plan (replaces Batch 7's):
  1. `feat(app): add view store fed by the niva host events` (1.21)
  2. `feat(app): add damped spring and per-state pose table for the mascot` (1.22, 1.23: `spring.ts`, `pose.ts` + tests)
  3. `feat(app): add hand gesture and dizzy-eye maths for the mascot` (`gesture.ts` + test)
  4. `feat(app): add arm, hand and laptop geometry for the mascot` (`geometry.ts` + test)
  5. `feat(app): draw the procedural Niva robot with hands, laptop and gestures on the island` (1.24, 1.25: `robot.ts`, `MascotCanvas.tsx`, `App.tsx`, `island.ts`, `island.test.ts`, `styles.css`)
  6. `feat(app): add a dev-only mascot preview page` (`mascot-preview.html`, `preview.tsx`)
  7. `docs(openspec): record mascot gestures and look, PR 1d progress and verify report` (`spec.md`, `design.md`, `tasks.md`, `verify-report-1d.md`)

### Batch 9: PR 1d owner look feedback 2 (fist, relaxed hands, laptop from behind) (2026-11-03)

- Owner: "I liked the previous version better, but adjust these details." Spiral eyes, "^ ^" eyes and the head scratch kept as they were. Nothing committed. The owner's dev app (port 1420) was left alone; screenshots came from my own Vite on port 1432 (stopped afterwards).
- Tests first: `geometry.test.ts` was rewritten (hand shapes, hand parts, laptop-from-behind geometry, typing hands on the lid edge), `robot.test.ts` adjusted (acting hands now at the lid top; key presses removed). Honest note: this time I did not run the tests once before the code, I wrote tests and code in the same pass and ran them GREEN (100 tests); the geometry module changed API, so the old tests could not have passed.
- Thumbs-up (success): the hand is now a closed fist, a rounded block with three horizontal knuckle rows (the finger parts turn into stacked rows through a new `curl` parameter), and a thick thumb (8.8 wide, 17 long) straight up from the top; the accent rim separates the rows and the thumb. It reads at 1x.
- Resting hands (idle, listening, reminder-alert, the idle hand in thinking, success, error, confirmation-needed): `down` parameter, fingers hang down and the thumb points down and inward, size 0.72 of a working hand (about 28% smaller), positions closer to the body ((+-37, 34) instead of (+-41, 38)). `HAND_SCALE` stays 1.3 and per-hand `size` carries the difference, so the scratching hand and the raised hand (confirmation-needed, deliberately open and up) are unchanged.
- Laptop (acting): seen from behind its lid. A slate back panel (62 x 32 at y 20..52) with a subtle accent top edge and a small ring emblem (own design, no logo), on a base slab; no code lines or keys on the viewer side. The screen's light tints the visor from below (`spill` gradient clipped to the visor, strength follows the `propLaptop` weight). Typing hands (fingers draped down over the top edge of the lid, size 0.85) sit at the lid's top edge at (+-17, 19) and dip 2.5 units alternately. Removed with the old front view: key grid, `keyUnder`, `fingertip`, `Frame.presses`, key press highlight.
- Simplification: net fewer details than Batch 8 (no keys, no code lines, one outline weight, no per-key highlight).
- Self-check from headless screenshots of all 8 states at 1x and 3x: the thumbs-up, the relaxed hands and the back-of-laptop read clearly at 3x; at 1x (96 px) the thumb, the small hanging hands and the lid with the visor tint are recognisable but small. The raised hand of confirmation-needed still looks like an open hand with fingers up; that is the gesture, not a rest hand. Screenshots (not part of the repo): `C:\Users\herit\AppData\Local\Temp\claude\D--trabajo-ProyectoAsistente\a37a8ce7-68e3-463c-91e9-c7b6cd1b7df8\scratchpad\mascot-batch9\preview-all-1.png` and `preview-all-2.png` (two animation moments).
- Docs: `specs/mascot/spec.md` hands requirement now describes the fist thumbs-up, relaxed resting hands and the laptop seen from behind with the visor tint; `design.md` geometry description updated.
- Done checks: `npm test` 9 files / 100 tests (geometry 21, robot 26, gesture 11, pose 11, spring 8, viewStore 11, island 4, others); `npm run build` ok and the preview is still excluded from `dist/`; `cargo test --workspace` 86 + 161 passed; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- Size: mascot, state and preview sources plus tests are about 2,320 lines in 20 new files, plus 103+/78- on tracked files (`git diff --stat`). Same `size:exception` situation.
- Commit plan unchanged from Batch 8 (7 commits: view store; spring and pose; gesture maths; geometry; robot drawing on the island; dev preview page; docs).

### Batch 10: PR 1d owner look feedback 3 (simple mittens, one-finger scratch, laptop with side deck) (2026-11-03)

- Owner: dislikes the multi-finger hands; the head scratch did not read; "the classic one with one finger, like when thinking". Nothing committed. The owner's dev app (1420) was left alone; screenshots came from my own Vite on port 1433 (stopped afterwards).
- Strict TDD with a real RED pass: `geometry.test.ts` was rewritten first (13 failing against the old geometry), and the `robot.test.ts` acting/thinking positions were changed first (1 failing); then the code went GREEN (102 tests).
- Hands: the paw look is gone. Each hand is one rounded mitten plus a thumb nub (`HandShape`: `bodyW`, `bodyH`, `pointLen`, thumb angle, length, width and base, `lean`, `size`; `HAND_SCALE` 1.1, bodies at most 16 x 20 units, about the size of an eye). Modes blended by weight: relaxed hanging (rest, typing), closed fist with a thick thumb straight up (thumbs-up, kept), open flat taller mitten (raised, confirmation-needed) and pointing (thinking). Drawing: slate gradient fill, soft highlight, dark outline, thin accent rim. Arms are thin smooth curves (one 3-unit stroke over a dark outline) from small round shoulder joints. Removed: finger parts, knuckle rows, `armPolygon` and `HAND_SCALE` finger geometry.
- Thinking: the left mitten sits beside the head at (-45, -17) tipped 1.35 rad toward it with ONE extended index finger whose tip lands on the head's upper-left near the temple (`fingertip`, tested to stay inside x -34..-25, y -30..-8 through the whole stroke). The stroke is 4 Hz, +-4 units vertically and +-1.5 horizontally (about +-2.5 px at 1x). The head still tilts toward the hand (thinking pose tilt -0.14). The `?`/thought dots were left out to keep it clean. The pose keeps `thinking` during AskFollowUp as before.
- Laptop (acting): a small lid (44 x 20 at y 31, so the whole face and chin stay visible) with the accent top edge and the ring emblem, in front of a deck strip 80 wide that protrudes 18 units on each side; the visor glow tint is kept (stronger now, since the lid is lower). The hands tap on the protruding deck edges at (+-31, 42), 2.5 units of dip, alternately, clear of the lid (tested). Typing mitten is the relaxed one.
- Preview page: `?state=thinking` shows one state at 5x for close-ups (dev only).
- Self-check from headless screenshots: the mittens read as simple hands at 1x and 3x and no longer dominate the head; the thumbs-up shows a fist and a thumb up; the raised hand is an open mitten; the one-finger scratch touches the temple and the two close-ups show the hand at different stroke positions; acting reads as a robot with a laptop in front and hands on the deck. At 1x the finger scratch motion is about 2 px and the thumb is a small stick; both are recognisable but small. Screenshots (not in the repo): `C:\Users\herit\AppData\Local\Temp\claude\D--trabajo-ProyectoAsistente\a37a8ce7-68e3-463c-91e9-c7b6cd1b7df8\scratchpad\mascot-batch10\grid-8-states.png`, `thinking-closeup-a.png`, `thinking-closeup-b.png`.
- Kept unchanged: spiral eyes, "^ ^" eyes, colours, head, antenna, reduced-motion behaviour (hand loops and presses are static when motion is 0), view store, review fixes.
- Docs: `specs/mascot/spec.md` hands requirement rewritten for the mittens, the one-finger scratch and the laptop with side deck; `design.md` geometry description updated.
- Done checks: `npm test` 9 files / 102 tests (geometry 23, robot 26, gesture 11, pose 11, spring 8, viewStore 11, island 4, others); `npm run build` ok, preview still excluded from `dist/`; `cargo test --workspace` 86 + 161 passed; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- Size: about 2,330 lines in 20 new files (sources and tests) plus 117+/78- on tracked files. Same `size:exception` situation.
- Commit plan unchanged (7 commits: view store; spring and pose; gesture maths; geometry; robot drawing on the island; dev preview page; docs).

### Batch 11: PR 1d owner look feedback 4 (overhead one-finger scratch, listening cups the ear) (2026-11-03)

- Owner approved mittens, laptop and thumbs-up; asked for two changes. Nothing committed. Screenshots from my own Vite on port 1434 (stopped afterwards); the owner's 1420 was not touched.
- Strict TDD with a real RED pass: tests for the new gestures were written first and 15 failed (missing `cup`, `questionBob`, `waveAlphas`, `headShift`, `lookX/lookY`, new fingertip geometry); then the code went GREEN (110 tests).
- Thinking: the scratching hand now rises ABOVE the head on the top-left (hand at about (-20, -46)) with the index finger turned to point DOWN (`lean` 3.0), the fingertip on the crown outline (tested: within 3 units of the head's rounded-rect outline through the whole stroke, using a signed-distance helper). The scratch is a 4 Hz, +-3.5 unit stroke along the outline tangent. The head tilts toward the hand (tilt -0.14, as before), the eyes glance up-left (`lookX` -0.6, `lookY` -0.7 added to the cursor gaze) and a "?" in the accent colour bobs (1 Hz, +-2 units, tiny rotation) near the top-right (`propQuestion`). The antenna lean was changed from -0.1 to 0.25 so it does not collide with the raised hand. Kept during AskFollowUp as before.
- Listening: new gesture `cupEar`. The right hand rises to just outside the side plate (about (47, -2)), an open mitten cupped behind the ear (narrow and tall, thumb toward the head, opened outward), the head tilts +0.18 rad toward that side with a +2 unit shift, the eyes glance away (`lookX` -0.5, `lookY` -0.3), and three accent arcs (`propWaves`) pulse inward toward the hand. The left hand rests. Tests: listening maps to cupEar/waves, hand near the side plate, tilt and shift signs match the hand side, the glance is on the opposite side, bounded and finite, static under reduced motion.
- Model: `Gesture` hands mode gains `cupEar`, prop gains `question` and `waves`; new numeric pose keys `handCup`, `propQuestion`, `propWaves`, `headShift`, `lookX`, `lookY` (springs, so changes cross-fade); `HandWeights` gains `cup`; pure `questionBob` and `waveAlphas` in `gesture.ts`; `Frame` gains `question` and `waves`.
- Self-check from headless screenshots: the thinking close-ups show the raised hand with a downward finger on the crown, the "?" and the up-left glance; listening shows the cupped hand with the arcs, the leaning head and the sideways glance. At 1x (96 px) the "?" and the sound arcs are visible; the finger on the crown is a small detail, and the 4 Hz stroke is about +-2 px. Screenshots (not in the repo): `C:\Users\herit\AppData\Local\Temp\claude\D--trabajo-ProyectoAsistente\a37a8ce7-68e3-463c-91e9-c7b6cd1b7df8\scratchpad\mascot-batch11\grid-8-states.png`, `thinking-closeup-a.png`, `thinking-closeup-b.png`, `listening-closeup.png`.
- Docs: `specs/mascot/spec.md` (thinking and listening gestures) and `design.md` updated.
- Done checks: `npm test` 9 files / 110 tests (geometry 24, robot 27, gesture 16, pose 12, spring 8, viewStore 11, island 4, others); `npm run build` ok, preview still excluded from `dist/`; `cargo test --workspace` 86 + 161 passed; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- Size: about 2,590 lines in 20 new files (sources and tests) plus 132+/78- on tracked files. Same `size:exception` situation. Commit plan unchanged (7 commits).

### Batch 12: PR 1d owner look feedback 5 (reminder-alert bangs two frying pans) (2026-11-03)

- Owner: listening accepted; thinking accepted for now but "not fully convinced" (recorded in `docs/BACKLOG.md`, Island / desktop section). New request: reminder-alert bangs two frying pans. Nothing committed. Screenshots from my own Vite on port 1435 (stopped afterwards); the owner's 1420 was not touched.
- Strict TDD with a real RED pass: tests first (14 failing: missing `pans` weight, `panCloseness`, `impactAlpha`, `panCenter`, `panTilt`, `burstSegments`, gesture table entries, frame `impact`), then GREEN (121 tests).
- Model: hands mode `pans` (both hands grip, closed mitten with the thumb over the handle) and prop `impact`; pose keys `handPans` and `propImpact`; reminder-alert maps to both. Pure functions: `panCloseness(t, motion)` (0 apart and raised, 1 touching, 2 Hz so the period divides the 1 s loop), `impactAlpha` (1 at the instant the pans meet, gone after 70 ms, so a hit shows as about 4 frames), `panCenter`/`panTilt` (pan disc inward of the hand, lifted and tilted on the outer swing) and `burstSegments` (8 short accent lines). Tests: the pans meet within 2 units of rim contact at the hit and are clearly apart in between; the impact is only above 0.5 while the pans are together; everything is bounded and finite; under reduced motion the hands are static, pans held up and apart, impact 0 and no head kick.
- Look: a pan is a dark metal disc with an accent (magenta) rim, a dark dish, a small highlight and a long handle (30 units) that the mitten grips at the outer end; they swing together in front of the chin (clear of the eyes), `PAN_R` 11. On each hit: star burst from the meeting point and a head kick (2 units down, 0.05 rad tilt, only while the flash lasts). The constant 5 Hz bounce of the old alert pose was removed (bounce 0), since the hit itself is now the bounce; eyes (wide, magenta) and the antenna shake are unchanged. No sound was added; the soft chime policy is unchanged.
- Preview: `?t=0.255` freezes the animation clock (dev preview only, `MascotCanvas` prop `timeOverride`) to capture the hit moment; `?state=` as before.
- Self-check from headless screenshots: the pans read as pans with handles at 3x and 5x; at 1x (96 px) they read as two magenta rings with handles and mittens in front of the chin; the burst reads at 5x and is small at 1x. Screenshots (not in the repo): `C:\Users\herit\AppData\Local\Temp\claude\D--trabajo-ProyectoAsistente\a37a8ce7-68e3-463c-91e9-c7b6cd1b7df8\scratchpad\mascot-batch12\grid-8-states.png`, `reminder-closeup-apart.png`, `reminder-closeup-hit.png`.
- Docs: `specs/mascot/spec.md` (reminder-alert bullet) and `design.md` updated.
- Done checks: `npm test` 9 files / 121 tests (geometry 29, robot 28, gesture 21, pose 12, spring 8, viewStore 11, island 4, others); `npm run build` ok, preview still excluded from `dist/`; `cargo test --workspace` 86 + 161 passed; `cargo clippy --workspace --all-targets -- -D warnings` clean.
- Size: about 2,940 lines in 20 new files (sources and tests) plus 146+/78- on tracked files. Same `size:exception` situation. Commit plan unchanged (7 commits).

### Batch 13 (owner look feedback 6: loud frying pans) (2026-11-03)

- Owner: the face-on pans looked like magnifying glasses; the alert must make a LOT of noise (the point of the message is to alert, the noisier the better); moving the pans above the head was approved.
- Strict TDD with a real RED pass: tests rewritten first (15 failing: missing `panPose`, `shockwaves`, `clangPop`, `panVibe`, `starPoints`, `vibeSegments`, frame `shock`/`clang`/`vibe`/`alertMark`, new pose values), then GREEN (128 tests).
- Pans: side view above the head. A shallow dish (curved lower body, elliptical rim opening with magenta rim and inner highlight, dark metal) plus a long straight handle (`PAN_HANDLE` 39 from the pan centre, `PAN_R` 13) down and outward to the mitten. `panPose(side, d)` gives tip, centre, hand and tilt: steep and wound up when apart, flatter when together, inner rim edges meeting at x = 0, y = -56 above the head centre. Old `panTilt`/`panCenter` removed. `handPositions` now returns the handle ends.
- Noise: `impactAlpha` (120 ms), `shockwaves` (2 rings, 300 ms, second delayed 70 ms), `clangPop` (400 ms, scale 0.6 to 1.3, fade, side and rotation alternate per hit), `panVibe` (220 ms, ticks plus jitter); `burstSegments` (12 lines, within 24 units) and `starPoints` (8-point star, up to 16 units); head kick 4 units down, 0.08 rad tilt (alternating) and a sideways shake; eyes pop 20% wider on the hit. Drawing order: rings and lines behind the pans, star in front, CLANG in design space.
- Pose: reminder-alert head lowered 14 units, scale 1, antenna laid over (lean 1.25) so it does not collide, eyes wider (1.3) and slightly worried. Reduced motion: pans held raised and apart, zero impact, rings, CLANG, vibration and shake, plus a still "!" (`Frame.alertMark`). No audio added.
- Test change outside the alert: the bounce-phase test switched its second state from reminder-alert to acting, because the alert's pose change (head lowered) is not what that test is about.
- Self-check from headless screenshots (own Vite on port 1436, stopped afterwards): at apart and at 5x the pans read as frying pans (dish, rim, long handle); at the hit the star, rings, noise lines and CLANG are loud; everything fits the canvas at 1x and in the 3x cell. Screenshots (not in the repo): `C:UsersheritAppDataLocalTempclaudeD--trabajo-ProyectoAsistentea37a8ce7-68e3-463c-91e9-c7b6cd1b7df8scratchpadmascot-batch13` (`grid-8-states.png`, `reminder-apart.png`, `reminder-hit.png`, `reminder-1x.png`).
- Docs: `specs/mascot/spec.md` (reminder-alert bullet) and `design.md` updated.
- Done checks: `npm test` 9 files / 128 tests (geometry 30, robot 29, gesture 25, pose 13, spring 8, viewStore 11, debug events 7, island 4, smoke 1); `npm run build` ok, `mascot-preview` absent from `dist/`; `cargo test --workspace` 86 + 161 passed; `cargo clippy --workspace --all-targets -- -D warnings` clean.

### Batch 14 (final review fixes) (2026-11-03)
- W1: `design.md` Mascot section said hands are drawn 1.3x; the code (`HAND_SCALE`) is 1.1x. Fixed. Scanned the design and spec mascot wording: the other numbers (150-unit space for 96 px, 140 px window, 6 px margin, 2 Hz pans, 120 ms flash, 4 Hz scratch, 0.15-0.25 rad listening lean, head lowered 14) match the code.
- W2: added `drawRobot.smoke.test.ts` (hand-written recording 2D context; 8 states x reduced motion on/off x 10 times including the pan hit; asserts no throw, finite numeric arguments and properties, balanced save/restore), `noAssets.test.ts` (no image or audio imports in `src/mascot` or `src/state`, via `import.meta.glob` raw) and `MascotCanvas.lifecycle.test.ts` (no jsdom or testing-library installed, so React hooks are stubbed with `vi.mock`; on unmount rAF is cancelled, the ResizeObserver disconnects and the matchMedia listener is removed; nothing starts while inactive). Mutation check: removing a `restore()` in `robot.ts` made 2 smoke tests fail, then it was restored.
- W3: `robot.ts` never recomputed the fingertip (the finger is a `handParts` part placed by the hand position), so there was no production caller to rewire. `fingertip` and `quadPoint` (the arm is stroked with `quadraticCurveTo`) were removed. The head-scratch test now checks the drawn finger: the tip of the finger part from `handParts` through the transform `drawHand` applies. `poseForState`, `gestureForState` and `gestureForView` were removed; tests use `POSES[state]`, and the AskFollowUp rule is tested through the production `poseForView`. No visual change.
- S: `preview.tsx` ignores `?t=` values that are not finite and the mis-indented `timeOverride` is fixed.
- Done checks: `npm test` 12 files / 149 tests; `npm run build` ok; `cargo test --workspace` 86 + 161 passed; `cargo clippy --workspace --all-targets -- -D warnings` clean.
