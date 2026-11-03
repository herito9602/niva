# Verify report: PR 2b (assistant loop), change `niva-mvp`

Date: 2026-11-03. Branch `feat/2b-assistant` (uncommitted). Mode: openspec, Strict TDD active.
Scope: tasks 2.1 (replies), 2.3, 2.5 (debug half), 2.6, 2.7, 2.8.

Verdict: PASS WITH WARNINGS. 0 CRITICAL, 5 WARNING, 7 SUGGESTION.

## Command evidence (PowerShell for cargo)

| Command | Result |
|---|---|
| `cargo test --workspace` | exit 0: host 121, niva-core 315, doc tests 6, all pass |
| `cargo clippy --workspace --all-targets -- -D warnings` | exit 0 |
| `cargo fmt --check` | exit 0 |
| `npm test` (app/) | 13 files, 157 tests pass |
| `npm run build` (app/) | ok |
| `node scripts/check-release-binary.mjs target/debug/niva.exe` | fails, listing all 7 needles (positive control works) |
| `node scripts/check-release-binary.mjs target/release/niva.exe` | ok. The release exe (12:28) is newer than the last source edit (12:26) |

Not run, as instructed: `tauri build`, the GUI. Strict TDD RED evidence is self-reported in Batch 17 and cannot be re-observed; the report admits the nine "checklist" dispatcher tests and the `ui_adapter` follow-up test were written after the code.

## Scenario-to-test table

| Spec scenario | Covering test(s) | Status |
|---|---|---|
| assistant-core / Provider swap | every `assistant::tests` case runs on `FakeLlm` behind `LlmProvider`, same pipeline and gate | COMPLIANT |
| Open app | `open_app_runs_the_tool_and_replies_with_the_real_result`, `a_failed_open_is_reported_truthfully_as_an_error` | COMPLIANT |
| Validation before execution | `an_invalid_call_rejects_the_whole_response_and_nothing_executes`, `an_unknown_tool_runs_nothing` | COMPLIANT (but see W5: past-time reminders) |
| Results are not fed back | `open_app_runs...` asserts `llm.requests().len() == 1`; `without_a_search_provider...` | COMPLIANT |
| web_search mixed with a risky call | `web_search_mixed_with_a_risky_call_is_rejected_and_nothing_runs`, `the_mix_is_rejected_whatever_the_order_of_the_calls` | COMPLIANT |
| Tool selection accuracy (90%) | none | DEFERRED to PR 2c (task 2.10, `niva-eval`), recorded in tasks.md |
| Chat only | `chit_chat_replies_with_the_models_text_and_runs_nothing` | COMPLIANT |
| Warm-up | none | DEFERRED to PR 2c (task 2.4, `ollama.rs`) |
| LLM down | `the_provider_being_down_is_an_error_and_nothing_runs` (Unavailable and Timeout) | COMPLIANT |
| language-handling / Language switch | `a_tie_without_a_pending_turn_uses_the_fixed_default_not_the_previous_language`, `detection_is_per_text_with_no_memory`, `the_reply_language_follows_the_request_not_the_app_name` | COMPLIANT |
| Reply language match (95% eval) | none | DEFERRED to PR 2c eval |
| Spanish request, English app | `the_reply_language_follows_the_request_not_the_app_name`, `accented_or_spanish_punctuation_decides_even_with_an_english_app_name`, host `a_typed_command_runs_and_the_island_shows_the_template_reply` | COMPLIANT |
| Voice follows reply language | none (`assistant.lang()` is exposed) | DEFERRED to slice 3 (TTS) |
| action-safety / Safe action | `volume_results_are_read_back_and_a_clamp_is_reported` | COMPLIANT |
| Confirmed | `close_app_asks_first_and_runs_only_after_a_click`, `a_spoken_yes_confirms_a_close` | COMPLIANT |
| Denied | `a_rejected_click_cancels_and_tells_the_user`, `an_unclear_spoken_answer_does_not_confirm_and_a_no_cancels` | COMPLIANT |
| Model bypass attempt | `a_model_that_claims_the_user_already_confirmed_still_has_to_ask`, `running_the_tools_without_an_answer_executes_nothing` | COMPLIANT |
| Timeout cancels | `no_answer_in_time_cancels_the_action` (assistant), host timeout test | COMPLIANT |
| Unclear reply | `an_unclear_spoken_answer_does_not_confirm...`, `only_clicks_and_the_open_listens_transcript_answer_a_confirmation` | COMPLIANT |
| Near-miss name | `a_near_miss_and_a_too_short_name_close_nothing` | COMPLIANT |
| island-shell / text input (debug) | `log.test.ts` (8) plus host `assistant_loop` tests through `TextSubmitted` | COMPLIANT for the debug half; release window DEFERRED to PR 2d (task 2.5 is `[~]`) |

## D4 and PR 2a contract checks

| Check | Result |
|---|---|
| One LLM response; only second call is the no-tools summary | OK. `llm.requests().len()` asserted 1 or 2 |
| Validation of the whole response before ANY execution | OK. `plan_calls` (assistant.rs:294-348) validates every call and resolves every risky target before `ToolCalls`; `running_processes()` is read-only. "volume 30 and close spotify": validated whole, then `RunSafe` (volume), then `RiskyRemaining`, then Confirm; test `safe_calls_run_first_then_each_risky_call_is_confirmed_in_order` pins `pc.calls()` before the click. A bad second call rejects the first (test at tests.rs:328) |
| Template replies; LLM text only for chit-chat and summary | OK |
| Fresh request id per response, retries included | OK. Ids come from the reducer; `every_response_gets_a_fresh_request_id_retries_included` |
| Per-call index into `SafetyGate::validate` | OK (assistant.rs:317); `safe_calls_run_first...` shows the first answer cannot approve the second call |
| `classify_confirmation` reused | OK. One yes-token list exists (state_machine.rs:568); `voice_answer` calls it (assistant.rs:73) |
| `Channel::Click` only from native click paths | Partly. The only producer is `click_answer` (assistant.rs:58). It is fed by any `Event::Accept/Reject`, which in debug builds also come from the debug webview. See W3 |
| Timeouts | LLM 15 s and confirm 10 s tested through the loop; tool 5 s only at reducer level (S5) |
| Release safety | `dev` is `cfg(debug_assertions)` (lib.rs:2-5); `AssistantRequests` and `Spawn` are `any(test, debug_assertions)`; `Unavailable` is the release path; the app `Cargo.toml` does not enable `testing` and no dev-dependency does; release exe passes the extended check |

## Findings

### CRITICAL

None.

### WARNING

**W1. Replies drop results that already happened (batch 17 open point 7: a spec violation, not only a misleading reply).**
- Spec, assistant-core "Single-response tool calling": tool results MUST reflect what actually happened, the reply MUST report them truthfully, and if the single response is incomplete for the request the reply covers only what ran. "Cancelled." after the volume changed, or after Spotify was closed, is not a truthful report of what ran.
- Evidence: the reject, timeout and Shortcut paths end in the reducer Template::Cancelled and never see Plan.lines. The test safe_calls_run_first_then_each_risky_call_is_confirmed_in_order asserts r.last() == "Cancelled." (assistant/tests.rs:596-597) right after Spotify was really closed and the volume set, so the test pins the false reply.
- Same defect on the summary path: summarize returns search_unavailable and discards context.lines (assistant.rs:567).
- Failure scenarios:
  - "volume 30 and close spotify", then Cancel: the user hears "Cancelled." and the volume is 30.
  - "open notepad and search rust" with the summary call failing: Notepad opens, the reply is "I can't search right now."
- The design contradicts itself: design.md:128 says one template reply "covering the outcome", while design.md:130 item 6 says Reject ends with the cancelled template. The safety side holds (nothing unconfirmed runs).
- Fix options: carry the text on the cancel path (a Reject with text, or a CancelledText), or amend spec and design to accept the loss. I recommend carrying the text. Not CRITICAL because no unconfirmed action runs.

**W2. A stale response can overwrite the live plan (single Session.plan slot, no currency check).**
- assistant.rs:338 (plan_calls stores unconditionally), 430 (run_tools puts the plan back) and 546-551 (summarize takes the search context) write the session without checking that req is still the reducer's current request. start_request runs on its own thread per request and the adapters are blocking (open point 10).
- Failure scenario:
  1. A: "close notepad"; the LLM hangs. At 15 s the timer fires, the reducer errors and sends CancelRequest(A).
  2. B: "open notepad"; B's response arrives, plan B is stored, and the reducer goes to Acting.
  3. A's blocked chat() finally returns, and plan_calls(A) sets session.plan = plan A, which destroys plan B.
  4. run_tools(B) gets take_plan(B) = None and returns nothing, so B hangs until the 5 s tool timer and ends in "Something went wrong."
  5. If B is a risky request, the Confirm click is recorded against A's confirmation id and nothing runs.
- The gate refuses a mismatched id, so nothing unsafe runs. This is an availability and correctness bug, found by inspection (no test reproduces it).
- No test covers the overlap. an_llm_that_never_answers_times_out_and_a_late_answer_is_dropped (assistant/tests.rs:863) has no second request in flight.
- Fix: the session should remember the current req (set in plan_request) and drop writes from any other req; add the overlap test.

**W3. The debug webview can produce Channel::Click, which the design forbids.**
- design.md:194 says Channel::Click may only be produced by the native click handler (host requirement; never the model, typed text or the debug webview).
- PR 2b: dispatcher.rs:105 calls observe for every event. The debug window button "Confirm (click)" (DebugWindow.tsx:112) dispatches Event::Accept, which becomes Channel::Click. For restart and shutdown, ClickRequired is then satisfied by the debug webview. Batch 17 point 4 states that click confirmation works from the debug window only.
- Today it is harmless: debug builds use DryRunPc. When PR 2d wires real win_* adapters in debug builds, the debug window can approve a real shutdown.
- Fix: amend design.md:194 to say debug builds only, and keep power actions dry in debug builds, or have the PR 2d host decide explicitly.

**W4. PR composition: safety code is mixed into the bulk PR (config.yaml tasks rule).**
- crates/niva-core/src/safety/matcher.rs:128-136 (+13 production lines, +65 test lines in safety/tests.rs) is a matcher change. The rule says safety-critical code (matchers, Approved<T>, protected list) goes in its own PR, never mixed with bulk code or prose. This PR also carries about 2,100 production lines and design.md and tasks.md prose.
- Recommendation: split it out. Commit 1 of the plan is already isolated, so cut a branch from main with matcher.rs, safety/tests.rs and the design.md sentence about the exact file name (line 195), open it as PR 2b-0 and merge it first. Rebase feat/2b-assistant on top.
- Also ask for a security-minded review of assistant.rs:58-93 and 510-541 (answer recording and spending, the code that decides what Approved is created from) in the main PR. It is orchestration, not a gate, so a separate PR is not required.
- The matcher change itself is sound. It only narrows (candidates.len() > 1, then an exact key match), and protected and out-of-session processes are filtered before it. The tests an_exact_name_never_widens_what_a_substring_would_match and the pick-survives-re-resolve test pin this.

**W5. Past-time reminders are caught at execution, not at validation.**
- design.md:126 lists "past date" under Validation before execution, and the spec says an invalid argument means nothing is executed. In run_safe (assistant.rs:473-475) the check is ReminderError::InvalidTime from the store, at execution time.
- Failure scenario: the response [open notepad, reminder yesterday 9:00] opens Notepad, then fails the reminder. The reply says only the time error (see also W1). a_past_time_is_rejected_by_the_store_with_a_future_time_message (assistant/tests.rs:317) tests only a single call.
- This is acknowledged as slice 4 (open point 3), but tasks.md does not list it as a slice 4 obligation or a spec deviation. Add a slice 4 task: resolve the time with the Clock at validation.

### SUGGESTION

- **S1 (open point 8: conformant).** Stopping the safe calls at the first failure matches "the reply covers only what ran" and D4; nothing in the spec requires continuing. Optional: record it in the spec as intended, because later independent safe calls (for example a volume change after a failed open) are skipped.
- **S2 (open point 2: OK).** FailureText has a row in coverage.rs (192 cells, passing), is in the stale-id guard (state_machine.rs:649-652), and is tested for thinking/acting, for a stale req and for every other phase (tests_tools.rs, failure_text_*). One doc gap: the transition matrix row "Reply / Failure" (design.md:111) does not name FailureText; it appears only in the contract list (line 150).
- **S3.** Two web_search calls in one response: found is overwritten (assistant.rs:491), so the first search runs and is silently dropped. Reject duplicates at validation, or run only the first.
- **S4.** summary_user (assistant.rs:709-715) delimits the untrusted snippets with triple double quotes. A snippet containing them closes the block early. The no-tools call and the 400-character cap bound the damage, but escape or strip the delimiter.
- **S5.** The 5 s tool timeout is tested only in the reducer. Add one assistant-level test with a blocking PcActions and a late result, so "late result dropped by id" is shown through the real loop (tied to W2).
- **S6.** check-release-binary.mjs is a denylist of strings, so it will not catch a new dev-only string unless someone adds it, nor the testing feature leaking in (fakes carry no distinctive literal). Add a CI line such as cargo tree -p niva -e features -i niva-core that fails on testing, and keep the debug-exe positive control (it works today) as a script test.
- **S7.** dev.rs is about 550 production lines plus about 230 test lines for throwaway adapters that PR 2c and 2d replace. The dev.rs tests exercise only the fakes. The host assistant_loop tests (real reducer, real assistant, deterministic inline spawn) are the valuable ones. Accept dev.rs because of the try-by-hand rule, but do not add more tests for it.

Other notes (no action needed): the TS maxLength counts UTF-16 units while prepareText counts code points. Reply language outside a request defaults to English, not the system locale (open point 5, recorded). Reset does not call Requests::cancel, but request ids stay monotonic, so a leftover plan is inert. Strict TDD RED evidence is self-reported.

## Design coherence

- D3/D4, Validation before execution, Confirmation flow, Reducer contract, Safety Gate and the new Assistant loop section match the code except for W1 (design.md:128 vs 130), W3 (design.md:194) and W5 (design.md:126).
- No dead code found: every Msg variant is used, no allow(dead_code), no TODO or todo! in production files.

## Completeness

Tasks 2.1, 2.3, 2.6, 2.7 and 2.8 are [x], 2.5 is [~] (release window, capability and tray pending in 2d). All match the code state. No unchecked task of this PR is outstanding.

## Recommendation

Next: sdd-apply for a small remediation batch (W2 with its test, W1 text on the cancel and summary-failure paths, design amendments for W3 and W5, matcher split for W4), then re-verify the delta. No CRITICAL issue blocks the PR itself; archive stays pending for the rest of slice 2.
