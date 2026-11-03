# Verify Report: niva-mvp PR 1b (reducer, tasks 1.6-1.13)

Branch feat/1b-reducer (uncommitted tree). Mode: openspec, Strict TDD.
Files: crates/niva-core/src/state_machine.rs (+ state_machine/{tests,tests_activation,tests_input,tests_tools,tests_speech,tests_confirm,coverage}.rs).

## Evidence
- cargo test -p niva-core (PowerShell): 120 passed, 0 failed.
- cargo clippy -p niva-core --all-targets -- -D warnings: exit 0. cargo fmt --check: exit 0.
- Purity grep over crates/niva-core: only std::time::Duration (a value type, state_machine.rs:10). No Instant/SystemTime/tokio/async/fs/net/OS calls. Cargo.toml has zero dependencies.
- Tasks 1.6-1.13 are marked done and match the code state.

## Verdict: PASS WITH WARNINGS (0 CRITICAL, 5 WARNING, 6 SUGGESTION)

## 1. Matrix vs design
All 22 event kinds x 8 states were compared with the design.md transition matrix. No cell disagrees in behavior. Differences are only deviations the implementer documented:
- Reminder-alert end (matrix row SpeakDone/alert and the Reminder-alert end paragraph): design ends the alert at the later of chime/speech done, capped at 30 s. Code ends it only on click or the 30 s cap (state_machine.rs:888-890, test a_finished_alert_speech_does_not_end_the_alert). See W3.
- STT timeout after end of speech has no duration in design; code uses 10 s and arms it only when the 15 s cap forces the end (:206, :911-920). See W4.
- One-turn follow-up enforced in the reducer (:850-853), consistent with the design Follow-up turn section. OK.
Other cells verified by reading code: Wake (:594), Shortcut (:613), Transcript/NoSpeech/EmptyTranscript (:645-685), TextSubmitted (:742), ReminderDue/Dismiss (:715-740), ToolCalls/ToolsDone/Reply/Failure/AskFollowUp (:788-860), SpeakDone per kind (:864), Watchdog (:895), VoiceReadinessChanged (:938, CO4 included), Accept/Reject (:693-713, :969).

## 2. The resolved ambiguities
None changes spec-level behavior (the specs are silent on alert end, STT timeout and follow-up limits), so no spec sign-off is needed. Items needing a design.md update or owner awareness:
1. Effect::StartRequest, Effect::ForceEndOfSpeech, request ids on LLM events (ToolCalls/ToolsDone/Reply/Failure/AskFollowUp carry req), SpeechStart{id}, ReminderDue{count}, ToolVerdict, ToolsOutcome, RequestInput, Template, TimerKind SttTimeout/FollowUp/AlertEnd: CONFIRMED, design.md State Machine (Effects list, around line 72) and the event list must be updated. Safe and consistent (the request-id stale guard extends the stale-id guard).
2. ToolCalls verdict: design gap; document the contract (service decides RunSafe / Confirm / Invalid; mixed safe+risky = RunSafe then ToolsDone RiskyRemaining).
3. Alert never ends on speech done: user-visible (alert stays up to 30 s unless clicked). Design-gap level; tell the owner. W3.
4. Reducer-enforced one-turn: safe, matches design.
5. 10 s STT timeout: invented number; document, keep tunable. W4.
6-7. SpeakStarted no-op; TtsFailed equals SpeakDone: match design.
8. Wake in success/error ignored while speaking or voice down: matches design.
9. Voice down while confirming: matches design matrix.
10-11. Queue / StopSpeaking on alert exit: match design.
12. Template ids: fine, but D4 should mention the id set.

## 3. Safety invariants
- Risky never proceeds without Accept: RunTool is emitted from only two places: on_tool_calls RunSafe (:793-797) and accept() (:693-701). accept() is reachable only from the ConfirmationNeeded phase via Accept click or a whole-utterance Yes. TextSubmitted, Wake and Reply are ignored in ConfirmationNeeded. Fails safe if request_id is None (no RunTool; Tool timer ends in error). Residual trust: the reducer believes the service ToolVerdict; a service that labels a risky call RunSafe bypasses the reducer. The type-level Approved gate (task 2.2) is the real guard. S1.
- Confirmation bounded by a timer: the 10 s Confirm timer starts only at SpeakDone/TtsFailed/synthesized done (:873-879, per design). The bound depends on the runner per-Speak watchdog, which is not reducer state. W1, W2.
- Power restart/shutdown click-only: the reducer honors click_only from the verdict (no Listen at :876; voice-down sets it at :956; voice-up never reopens it). Tests exist. The flag source is the service. S2.
- Voice-down never emits Listen: all Listen emitters (enter_listening via activate, Wake, SpeakDone AskFollowUp) check voice_available; confirm Listen is guarded by voice_available and not click_only; the reissue (:689) has no explicit guard but needs a live listen_id, which voice-down clears. S3.

## 4. Purity
PASS. See Evidence. Id allocation is a deterministic counter.

## 5. Test quality
Tests assert ordered effects with payloads and next state, not call counts. coverage.rs is exhaustive by construction: kind_of, row, fixture, sample are exhaustive matches, EventKind::ALL is macro-generated, rows are [Expect; 8], state invariants run after every cell, and an Ignored cell must leave the state bit-identical. Limits: the coverage test checks effect kinds only (payloads are checked in the topic files) and uses one canonical fixture per cell (voice-down variants live in topic tests). Acceptable.

## Findings
### WARNING
- W1 state_machine.rs:304-306, 896-901: Watchdog{SpeakWatchdog} carries no speak id, so a late watchdog of speak A can end speak B (premature settle, early confirm listen). The stale-id guard cannot apply. Fix: add an id, or require the runner to cancel it on every SpeakDone/new Speak, with a test; document in design.
- W2 :781-786, 873: the guarantee that a confirmation is never pending forever depends on the runner always synthesizing SpeakDone (the watchdog is runner-owned, not in State.timers). Document as a runner contract, test it in task 1.14; consider starting the Confirm timer at entry (design currently says at speak_done).
- W3 :888-890: alert deviates from the design Reminder-alert end rule. Update design.md and tell the owner.
- W4 :206, 918: invented 10 s STT timeout, armed only on a forced end of speech. Document and make tunable.
- W5 :717-723: ReminderDue always emits Speak, but fullscreen-policy says MUST NOT speak. The reducer has no fullscreen input; the service/runner must suppress speech (or ReminderDue should carry a silent flag). Decide before slice 4 and add to design.

### SUGGESTION
- S1 Treat ToolVerdict as a trust boundary: enforce with Approved<T> in 2.2 and add a service test that a risky call can never yield RunSafe.
- S2 Defense in depth: in on_transcript (:651) ignore a Yes when confirm_click_only.
- S3 Guard reissue_confirm_listen (:689) with voice_available and not click_only.
- S4 Update design.md State Machine (Effects, Events, timers) and tasks.md 1.13 ("17 rows" is now 22 kinds because VoiceReadinessChanged, Accept/Reject and SpeakDone/TtsFailed are split).
- S5 on_watchdog removes the timer for non-matching (phase, kind) pairs without emitting CancelTimer (:903-934); harmless, add a comment or test.
- S6 Voice down in success/error emits StopSpeaking toward a dead sidecar (:959); harmless, the runner should no-op.

## Next
No blocker for PR 1b. Apply W1-W5 as design.md edits (plus the optional W1 fix) before or alongside PR 1c. PR 1b is about 3650 lines, far over the 400-line budget, so it needs size:exception or the per-commit split from the apply note.
