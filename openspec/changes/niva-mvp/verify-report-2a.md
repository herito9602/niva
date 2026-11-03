# Verify Report: PR 2a (safety core), change niva-mvp

Branch feat/2a-safety, uncommitted. Date 2026-11-03. Mode: openspec, strict TDD.
Scope: task 2.1 (ports only, partial), task 2.2 (done), Batch 15, new review rules in openspec/config.yaml.

## Verdict

PASS WITH WARNINGS. CRITICAL 0, WARNING 6, SUGGESTION 7. Nothing blocks the 2a merge; W1, W2 and W4 should be fixed or consciously accepted first.

## Evidence

| Check | Result |
|---|---|
| cargo test --workspace (PowerShell) | host 86 passed, niva-core 233 passed, 4 doc tests (compile_fail) passed |
| cargo clippy --workspace --all-targets -- -D warnings | clean |
| cargo fmt --check | clean (exit 0) |
| Line counts in Batch 15 | match wc -l (ports 105, tools 437, safety 289, matcher 103, testing 283) |
| Test counts in Batch 15 | match (233 = 161 + 72) |
| TDD RED pass ("63 failed, 169 passed") | Not independently verifiable: nothing is committed, so there is no history. Substitute evidence below. |
| Mutation check (scratch copy outside the repo, source untouched) | 8 of 8 compilable mutants killed: click-only check removed, confirmation-id check removed, accepted check removed, re-resolve equality removed, protected check removed, session filter removed, min length 3 to 1, delta range check removed. 2 more mutants did not compile (non-exhaustive match) and are not counted. |

The gate tests are strong. TDD ordering cannot be proven, but the final suite detects each safety-relevant regression tried.

## Scenario-to-test table

| Spec scenario | Where covered | Status |
|---|---|---|
| action-safety / Safe action ("volume to 30") | safety::tests::a_safe_call_validates_without_confirmation | Covered (gate). Execution by adapter in 2d. |
| Confirmed | an_explicit_click_approves_a_close, a_voice_yes_approves_a_close_lock_or_sleep, a_confirmed_close_runs_exactly_once_with_the_confirmed_executable | Covered at the gate. "User says yes" parsing is the reducer's classify_confirmation plus 2b wiring. |
| Denied | a_denied_answer_does_not_approve, only_a_matching_accepted_answer_ever_reaches_the_risky_port_methods | Covered. Reducer side in state_machine/tests_confirm.rs:53. |
| Model bypass attempt | a_model_claiming_confirmation_still_only_gets_a_pending_request plus 4 compile_fail doc tests (safety.rs:17-47) | Covered; the type-level guarantee is the real proof. Routing every call through the gate end to end is 2b (2.3). |
| Timeout cancels | Reducer: state_machine/tests_speech.rs:256, tests_confirm.rs | Deferred by design to the slice 1 reducer (done) and 2b wiring (Batch 15 point 5). Accepted. |
| Ambiguous answer / unclear reply | Gate: a_denied_answer_does_not_approve (anything not accepted). Reducer: classify_confirmation (state_machine.rs:565, tests_confirm.rs:112-137) | Covered. See S4. |
| Near-miss name ("Notpad") | a_near_miss_name_matches_nothing, a_fuzzy_neighbour_is_never_returned | Covered. "User is told" is a 2b template. |
| Several apps match: ask which | several_distinct_executables_are_returned_as_candidates, an_exact_name_does_not_beat_a_longer_one, close_app_with_several_matches_returns_the_candidates_not_a_request | Covered. |
| Risk classification | tools::tests::close_app_and_power_action_are_risky, every_other_tool_is_safe, parse tests | Covered, but see W5. |
| pc-actions / Open app, Unknown app | Port only (PcError::NotFound, NotReady) | Deferred to 2d (adapter) and 2b (templates). |
| Close not running | an_app_that_stopped_before_the_click_is_reported_not_running, ToolError::NoMatch | Covered at the gate; see S5. |
| Set volume 60 to 30 | set_volume_sets_a_level plus FakePc | Parse covered; real volume is 2d. |
| Out of range 150 clamped and reported | a_level_above_100_is_clamped_and_reported, a_negative_level_is_clamped_to_zero | Covered. |
| Truthful failure | PcError::Failed, CloseOutcome::StillRunning test | Port shape and one test; reply and mascot in 2b/2d. |

## Tool registry versus spec, design and E1

- Risky: close_app, power_action. Safe: open_app, set_volume, create_reminder, list_reminders, web_search. Matches the spec and design "Safety Gate".
- Wire and arg names match eval/commands.yaml exactly: open_app{name}, close_app{name}, set_volume{level|delta|mute}, create_reminder{text, delay_minutes, at_local_time, date}, list_reminders{}, web_search{query}, power_action{action: lock|sleep|restart|shutdown}. No rename.
- Versus the spike (spikes/pc-control/spike_pc_control.py:47-96): same names for the five tools it has. delta, reminders and list_reminders are product additions from the design. null is treated as absent, so the spike's required-nullable shape still parses.
- One value-format mismatch with E1 (W1).

## Judgment of the Batch 15 deviations

1. Sync ports: acceptable. niva-core has no runtime, the reducer already drops late results by request id, and a blocking adapter on a worker thread fits. Cost: an in-flight Ollama call cannot be cancelled, only ignored. Revisit in 2c as the batch says. design.md should say it (W4).
2. running_processes() instead of resolve_close_target: acceptable and better. The protected list, session filter and matcher live in the core and an adapter cannot skip them. design.md:41 and design.md:181 (the adapter re-resolves via PcActions.resolve_close_target) are now wrong: re-resolve is SafetyGate::execute_close (safety.rs:267). Must be updated (W4).
3. Delta rejected instead of clamped: acceptable. The spec only clamps level, and the design schema bounds delta to -100..100, so rejection is schema validation. See S3 for the friendlier alternative. Document it in design.md:184.

## Findings

### CRITICAL

None.

### WARNING

**W1. at_local_time format is stricter than what E1 accepts.** crates/niva-core/src/tools.rs:401-406 (parse_clock_time) and tools/tests.rs:398 accept only two-digit HH:MM and assert that "9" is rejected. eval/commands.yaml:195 and :216 define the correct model answer as one_of ["21:00", "21:00:00", "21"] and ["18:00", "18:00:00", "18"]. Failure scenario: the user says "remind me at 6 to call the bank", qwen emits at_local_time "18". E1 scores it correct, but the product returns InvalidArgument{at_local_time, BadFormat} and the user gets an error. E1 accuracy (96.1%) therefore overstates real behavior. Fix: accept H, HH, HH:MM and HH:MM:SS (seconds 00 only), or tighten the E1 matcher to the same grammar and re-run in 2c. Decide before 2c.

**W2. RiskyRequest is Clone, so one confirmation can mint two Approved values.** safety.rs:159 derives Clone; the doc at safety.rs:102 says one confirmation authorizes one execution, but the request can be cloned and approved twice. safety/tests.rs:410 itself calls request.clone() and approves both. Failure scenario: a future assistant loop clones the pending request (for example to keep a copy in the pending slot) and approves on both a click and a late voice "yes": the app is closed or the PC powered twice for one confirmation. The Approved type guarantee does not cover this. Fix: drop Clone from RiskyRequest (and from Validated and RiskyKind if nothing needs it) and have the test build two requests.

**W3. PR composition conflicts with the new openspec/config.yaml rules.** The rules (config.yaml:46-49) say safety code goes in its own PR, "never mixed with bulk code or prose", and "each PR from slice 2 on ends in something the owner can try by hand". This diff carries, besides safety: ports.rs plus ports/testing.rs fakes (283 lines), host edits in app/src-tauri/src/ui_adapter.rs and dispatcher.rs, and proposed commit 2 puts tasks.md and config.yaml (the rules themselves) in the same PR. Nothing is wired, so there is nothing to try by hand. The ports and the UiPort move are defensible (the risky port methods need Approved, and Batch 15 explains why it is one commit), but the prose is separable. Fix: land the config.yaml rules and the progress notes in their own commit or PR, or record an explicit owner exception in the tasks.md "Delivery decision". The "nothing to try by hand" part is implied by the owner-approved split; record it as an accepted exception.

**W4. design.md is stale after the deviations.** Not updated: design.md:41 (port list: resolve_close_target is now running_processes; ports are synchronous; UiPort is generic UiPort<E>), design.md:181 (Binding: re-resolve happens in SafetyGate::execute_close in the core, and Approved<CloseApp> binds query plus one exe plus confirmation id, not a Candidates set), design.md:184 (delta out of range is rejected, level is clamped). Failure scenario: 2b/2c/2d implementers follow the design and write an adapter with resolve_close_target, duplicating or bypassing the core matcher. Update design.md in or right after this PR.

**W5. ToolName::risk() and Risk are dead in production and a second source of truth.** tools.rs:83 risk() is read only by tests (tools/tests.rs:36-44). The runtime classification is the shape of parse_call's result (ParsedCall::Safe versus Close/Power). Nothing ties the two together. Failure scenario: someone adds a tool, marks it Risk::Risky in risk() but parses it into ParsedCall::Safe; tests pass and the tool runs unconfirmed. Fix: add a test that for each ToolName a minimal valid call parses to Safe iff risk() is Safe, or derive the class from ParsedCall and delete risk().

**W6. Tests that only exercise fakes, and fakes shipped in the library.** ports/testing.rs:209-226 (fake_llm_replays_its_script_and_records_requests) and :271-282 (fake_clock_and_ui_behave) test the fakes themselves and no product behavior; :228-241 mostly does too. :243-269 duplicates safety::tests (a_confirmed_close_runs_exactly_once...). Also ports.rs:14 "pub mod testing;" compiles the fakes into the production crate, and they have no consumer until 2b/2c (unused public surface today). Failure scenario: none functional; maintenance cost and a larger niva-core API for nothing. Fix: delete the fakes-only tests (keep the safety-relevant one) and gate the module behind cfg(any(test, feature = "testing")) when niva-eval needs it.

### SUGGESTION

**S1. Protected list is exactly the design list, but thin.** safety/matcher.rs:16-33. Not protected: ApplicationFrameHost (the design itself uses it to close UWP apps), RuntimeBroker, fontdrvhost, audiodg, taskmgr, System, Registry, TextInputHost. Scenario: "close frame" matches ApplicationFrameHost.exe alone; after the user's yes, every UWP window is told to close. The confirmation names the exact exe, which limits the damage. Consider extending the design list.

**S2. ASCII-only case folding in two comparisons.** matcher.rs:88 (dedupe) and safety.rs:274 (re-resolve) use eq_ignore_ascii_case, while matching uses Unicode lower-casing. Scenario: "Ñandú.exe" and "ñandú.exe" (the same file on Windows) become two candidates, so a spurious Ambiguous. Use to_lowercase for both.

**S3. Consider clamping delta like level.** tools.rs:343-346. "Turn the volume way up" with delta 150 yields an error, while level 150 succeeds. VolumeResult.clamped (ports.rs:77) already documents delta clamping. Either keep rejection (and document it) or clamp and report.

**S4. Two "Answer" concepts and an inconsistent note.** safety::Answer (struct) and state_machine::Answer (Yes/No/Unclear, state_machine.rs:551) share a name. Batch 15 point 4 says the whole-utterance yes/no matcher is task 2.3, but classify_confirmation already exists (state_machine.rs:565). Rename one (for example Verdict) and fix the tasks.md note so 2b does not write a second matcher.

**S5. NoMatch covers both "not running" and "near-miss".** ToolError::NoMatch (tools.rs:228). pc-actions "Close not running" wants "is not running" and action-safety "Near-miss" wants "no match found". The core cannot tell them apart. Fine if 2b uses one wording; otherwise add a distinct case.

**S6. Prepare 2c for the spike's set_volume shape and JSON numbers.** The spike schema marks both level and mute required (spike_pc_control.py:75). A model that fills both (level 0, mute true for "mute") hits Conflicting (tools.rs:334-336). Make all three optional in the production schema and keep a test for it. The host decoder must send integral JSON numbers as Int (30.0 becomes Float and then WrongType).

**S7. Minor API and docs notes.** MIN_QUERY_CHARS and is_protected are re-exported publicly (safety.rs:56) but used only inside the crate and tests. ArgValue::Float exists only to be rejected (Batch 15 point 10, documented). Batch 15 should say the TDD RED evidence is self-reported.

## Config rules check (openspec/config.yaml)

- Safety code isolated from bulk code: mostly (see W3).
- One concern per PR: ports plus gate is one concern (what makes the gate real); acceptable.
- Production and test lines reported separately: yes, Batch 15 does it (1,129 production, 1,176 tests) and the numbers check out.

## Next recommended

sdd-apply for a small fix batch (W1 decision, W2, W4, W5, W6; W3 as a commit-split decision by the owner), then re-verify 2a and proceed to PR 2b. No CRITICAL issue.
