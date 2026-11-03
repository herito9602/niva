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

- [ ] 1.14 CO1: `effect_runner.rs` minimal runner: `StartTimer`/`CancelTimer` (tokio tasks feeding `Watchdog` events), settle 3 s, `ShowIsland`/`HideIsland`, `Chime` as no-op log, other effects logged. Test: runner unit test with paused tokio time (settle fires at 3 s). Runner contract (W2): arm a speak watchdog for every `Speak` (token = speak id) so a `Speak` always ends in `SpeakDone`/`TtsFailed`/watchdog, and test it; `StartTimer{kind, token}` must be echoed back in `Watchdog{kind, token}`; `Chime{id}` must report `ChimeDone{id}`.
- [ ] 1.15 CO1 decision: reducer default `voice_available=false`; debug builds dispatch `VoiceReadinessChanged{ready:true}` at startup (the real supervisor arrives in slice 3); release slice 1 keeps it false, so shortcut shows the `error` pose (accepted until slice 3). Test: reducer test for both paths; document in `design.md` note.
- [ ] 1.16 `ui_adapter.rs`: `TauriUiAdapter` emitting `niva://view` and `niva://island` from reducer output. Done: events visible in webview console.
- [ ] 1.17 `cursor.rs`: `GetCursorPos` poller at 30 Hz while visible, emits `niva://cursor` window-relative. Done: coordinates logged on move.
- [ ] 1.18 Wire the spike's shortcut and tray to dispatch `Shortcut` into the reducer through the runner. Done: shortcut shows/hides island via state.
- [ ] 1.19 `debug.rs` (`cfg(debug_assertions)`): create debug window in code, `debug_dispatch{event}` command, `capabilities/debug.json` scoped to that window label. Done: command absent in release build (checked in 1.26).
- [ ] 1.20 `app/src/debug/DebugWindow.tsx`: one button per event, per-state shortcut buttons, voice ready/down toggle. Done: clicking each button changes the state shown.

### Phase 1.D: Mascot (PR 1d) in `app/src/`

- [ ] 1.21 `state/viewStore.ts` consuming `niva://view`/`niva://island`/`niva://cursor`; vitest for reducer-free store updates.
- [ ] 1.22 `mascot/spring.ts` damped spring + vitest (convergence, no overshoot blowup).
- [ ] 1.23 `mascot/pose.ts` pose table for all 8 states + vitest (every state has a pose; thinking pose kept during AskFollowUp). Spec: `mascot`.
- [ ] 1.24 `mascot/robot.ts` + `MascotCanvas.tsx`: procedural head, capsule eyes, antenna, breathing, blinks 2-6 s, cursor-following eyes; no image assets. Done: visual check of 8 states via debug window.
- [ ] 1.25 Look review of mascot against the proposal's "derivative" risk (no Coucou code, shapes, name, media). Done: noted in PR.

### Phase 1.E: Slice 1 done check

- [ ] 1.26 (a) Dev: `cargo test`, `npm test` pass and all 8 states visible from the debug window. (b) Release: `cargo tauri build` from PowerShell, run the exe, shortcut shows island (error pose), no debug window, no `debug_dispatch`.

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
