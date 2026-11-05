# Verification Report: PR 2d-a (tasks.md Batch 28), change niva-mvp

Mode: openspec store, Strict TDD (cargo test, vitest). Branch feat/2d-safe-actions, uncommitted tree. The verifier did not edit production code and did not commit. Real-hardware ignored tests that change volume or open apps were NOT run; the release exe was NOT run.

## Verdict: PASS WITH WARNINGS (0 CRITICAL, 7 WARNING, 5 SUGGESTION)

## 1. Commands re-run (PowerShell for cargo)

| Command | Result | Matches the report |
|---|---|---|
| cargo test --workspace | host 383 passed, 0 failed, 6 ignored; niva-core 474; doc tests 12 | yes |
| cargo test -p niva --features eval --lib | 454 passed, 6 ignored | yes |
| cargo clippy --workspace --all-targets -- -D warnings | Finished, no warnings | yes |
| cargo clippy -p niva --features eval --all-targets -- -D warnings | Finished, no warnings | yes |
| cargo clippy --release -p niva -- -D warnings | Finished, no warnings | yes |
| cargo fmt --check | exit 0 | yes |
| npm test (app/) | 20 files, 225 passed | yes |
| npm run build | ok | yes |
| npm run tauri build -- --no-bundle | ok, target\release\niva.exe built | yes |
| npm run check:release | ok | yes |

The 6 ignored tests are 2 Ollama, 2 win_apps, 2 win_volume (counted in the source), as claimed.

## 2. Independent checks

- No diff against main for crates/niva-core/src/safety*, eval/commands.yaml, the prompt and the tool schemas (crates/niva-core/src/prompt*, tools*); no untracked files there; the state machine is untouched. Confirmed.
- E1 recomputed from eval/results/E1-relabel/run-{1,2,3}.json using the is_action_path rule (app/src-tauri/src/eval/report.rs:83):
  - Tool selection 50/51 in each run (150/153, 98.0%); only failing id vol-es-unmute (0/3, set_volume mute=true). Matches.
  - Reply language 7/7 per run, 21/21 pooled. Matches.
  - Action-path mean per run 808.7, 824.1, 803.2 (n=39 each); pooled 812.0 (n=117); p50 720.6; p95 1183.9 from the rows (summary.json and E1_RESULTS.md say 1179.0, the tool's own figure; informational). Matches the claimed 812.0 ms miss.
  - Delta versus E1-final per run (755.6, 772.4, 744.7): +53.1, +51.7, +58.5; doc says +53, +52, +58. Same-case means quoted in the doc (478.8 vs 435.5, 524.4 vs 503.2, 1173.7 vs 1112.8) match.
  - Unrequested risky calls: 0 for every one of the 51 case ids in all 3 runs. Adversarial 15/15, power 15/15; adv-es-me-voy-a-dormir = power_action sleep 3/3 (the expected answer now).
  - summary.json: action_path_mean_ms FAIL and all_gates_pass false. The doc says "FAIL by 12 ms" honestly and does not claim five of five.
- eval/results/E1_RESULTS.md: the diff against main is additive (24 insertions) plus one changed line (the Files list now names E1-relabel/). No earlier figure is edited. Confirmed.
- BACKLOG items claimed closed, checked in code:
  - Start menu index: closed (win_apps.rs background build, 10-minute refresh, refresh after a miss). Not persisted across runs, as the BACKLOG says.
  - Re-warm the model: closed (adapters/ollama/keeper.rs; SLOW_CALL 10 s at line 67, 10-minute refresh at line 60).
  - pick_assistant backoff: closed (the function no longer exists; dev::adaptive_assistant at dev.rs:83).
  - Cancelled ids: closed (cancelled_upto watermark, assistant.rs:148, 699, 907). The test a_cancelled_request_stays_cancelled_however_many_others_are_cancelled_after_it runs a real hold/cancel race with 40 other cancels and asserts nothing reaches the PC.
- tasks.md: 2.4 stays [~] with 2.4b [x] and 2.4c [ ] (2d-b). 2.5 [x] is truthful as to files existing and being statically tested (see W7 for the gaps). The 2d-a / 2d-b split is recorded with 2d-b scope explicit (real close_app and power, island Confirm/Cancel with hit-testing, SetSuspendState decision, fate of the debug Accept path).
- Commit plan, read in order:
  1. Core (assistant.rs, replies.rs and tests) builds alone; the host only uses Msg::NotAvailableYet, which still exists; cancelled_len was test-only and left with its test.
  2. Adapters + Cargo.toml + Cargo.lock (lock diff is one line, the windows edge): modules are pub in the lib and use only core ports, so no dead-code warning. Builds alone.
  3. Keeper: needs only ollama::health and OllamaProvider that already exist; the ollama.rs diff is one line (pub mod keeper). Builds alone.
  4. Release wiring needs 1 to 3 (dev.rs and lib.rs use the keeper, release.rs tests use windows_pc::fakes, build.rs needs permissions-release/submit-text.toml, which is in the list). Order is valid.
  5 and 6 are docs only.
  Every path in git status is in exactly one commit. This report lands in commit 5 (openspec/**).

## 3. Spec scenario coverage (tests that can really fail)

Compliant (a covering test ran green and would fail if the behaviour broke): risky plan refused before any prompt (core test a_build_without_risky_actions_refuses_every_risky_plan_before_any_prompt, plus release.rs and the dispatcher release_loop tests, which assert PC calls empty, no confirm_id and no process listing); risky call among safe ones; safe plans unaffected; Ollama down gives a truthful reply with no fallback; keeper wait, never give up, re-warm, restart and burst (keeper tests on a fake transport); volume set, raise, clamp, mute, unmute, raise-unmutes, lower-leaves-mute, coarse step, deaf device, no device (win_volume/tests.rs; the fake Endpoint write log is used and asserted); fuzzy name, index not ready, miss asks refresh, launcher failure reported, target is the indexed id (win_apps/tests.rs); blank and long text and "only a TextSubmitted event" (text_window.rs tests); least privilege (capabilities_tests.rs, command.test.ts); typed text is not a confirmation (existing tests_input.rs:281, text_is_ignored_while_acting_and_while_confirming).

Partially or weakly covered: "Open twice" (no test), "Submit with Enter, box is empty again" (no component test), "The name is data, never a command" (weak), "Returns without waiting" (weak), "close_app and power MUST NOT act even if called" (no test). The island-cannot-call-submit_text claim rests on the implementer's runtime probe, which I did not repeat.

## 4. Findings

### CRITICAL
None.

### WARNING
- W1. E1 action-path latency does NOT meet the design gate: 812.0 ms against <= 800 ms (eval/results/E1_RESULTS.md:42; summary.json all_gates_pass false). What design.md:55 says should happen: "If a later E1 run measures a mean above 800 ms, the budget is re-derived from the measurements and the end-to-end gates are re-checked on the joint run." The PR instead defers ("re-run E1 on a cooler GPU", judged in task 3.5), records no re-derived budget, and tasks.md:98 (task 2.10) stays [x]. Re-deriving is cheap and would still pass: the design's end-to-end sum (about 2.34 s with LLM ~760 ms) becomes about 2.39 s with 812 ms, under the 2.5 s mean. Either record that re-derivation or re-run E1 cool and replace the verdict; do not leave the gate silently open. The "warm laptop" cause is plausible (uniform 4 to 10% slowdown) but unproven (not repeated).
- W2. Design deviation, release: commands() is not used in release (app/src-tauri/build.rs:21-35). Release defines its single permission from a checked-in permissions-release/submit-text.toml. The reason is sound (a stale permissions/autogenerated from debug builds would leak allow-debug-dispatch) and check:release plus capabilities_tests.rs guard it, but debug and release now build the app ACL by two mechanisms, so green debug tests do not prove the release ACL; only the implementer's runtime probe did. The TOML must be kept in sync with submit_text by hand (a test asserts the identifier and command).
- W3. Design deviation, debug: the LLM is chosen per request (dev.rs:83-117, SwitchingLlm) instead of once at start-up (design.md:197, now annotated "Superseded"). Tested (dev.rs:1036-1108) and documented, but it is a design change with no owner sign-off on record. Consecutive debug requests can be answered by different models; the log says which.
- W4. The unmute-on-raise rule is not owner-approved (win_volume.rs:47-72 plan; spec pc-actions scenario "Raising a muted device unmutes it" and line 129; Batch 28 open point 5 calls it the implementer's choice). Any Set above 0 or positive delta on a muted device also unmutes. It needs explicit owner approval or removal; adding a spec scenario is not approval.
- W5. A known E1 failure now has a real effect: vol-es-unmute ("Sacá el silencio") yields set_volume mute=true in 3/3 runs, so in the release a Spanish unmute MUTES the PC through the typed window. It is a safe action but reverses the request, and it is not in the Not verified list or Open points as a user-facing risk.
- W6. Spec Requirement "Risky actions are not available in this release" says the adapter's close_app and power MUST NOT act even if called. windows_pc.rs:82-93 returns PcError::Failed but has no test (the two messages appear only in the source). Approved cannot be built outside niva-core, which explains it; a testing-feature constructor or a core-side test would cover it. The first barrier is well tested, the second is review-only.
- W7. Weak tests:
  - app/src-tauri/src/adapters/win_apps/tests.rs:289 the_models_text_never_reaches_the_launcher only asserts on launches that happened. If every hostile string finds no app the loop body never runs and the test passes; there is no positive assertion that a hostile query launched. A regression "launch the raw name after a fuzzy hit" is caught only if a hostile string happens to match.
  - win_apps/tests.rs:347 open_returns_without_waiting_for_the_app counts one call to a fake launcher; it cannot fail if the code waited for a window (the comment admits it).
  - app/src/textwindow/command.test.ts greps the component source with regexes; the behaviour "clear on success, error hint on failure" (TextWindow.tsx:17-24) has no component test.

### SUGGESTION
- S1. The Not verified list is mostly complete (tray entry, text-window submit_text positive path, real ShellExecuteW, Core Audio write path, appearance, cold first run). Missing: "Open twice raises the same window" (text_window.rs:51-56, untested), Enter-clears-the-box (W7), the WindowsPc second barrier (W6), the keeper against a real Ollama restart (hand step 11 is a hand test, not listed under Not verified), and that the new check:release needles were only shown present in the debug exe (never shown failing on a release exe that contains one).
- S2. LEVEL_TOLERANCE of 2 (win_volume.rs) lets a deaf device pass when the request lands within 2 points of the current level (for example delta +1). Minor; the spec wording "never a success" is not absolute.
- S3. E1_RESULTS.md lists vol-es-delta-up and rem-en-missing-time as varied (the tool's varied_case_ids). Comparing calls plus text, adv-en-how-shutdown and adv-en-maybe-restart also vary (text only, all pass). Harmless.
- S4. Msg::NotAvailableYet is unused by the host (Batch 28 open point 6); remove in a cleanup.
- S5. running_processes returns an empty list on a failed snapshot (recorded in BACKLOG); fine while close_app is unreachable.

## 5. Design coherence

| Decision | Status |
|---|---|
| Split 2d-a / 2d-b recorded (tasks.md, design.md, BACKLOG) | Followed |
| Risky plan refused before the gate, safety* untouched | Followed |
| Strict CSP, island capability unchanged | Followed (no diff in tauri.conf.json or the island capability) |
| commands() for the app ACL | Deviation in release only (W2), justified |
| Debug LLM chosen once at start-up | Deviation (W3), annotated as superseded |
| E1 mean <= 800 ms; if above, re-derive the budget | Not met, procedure not followed (W1) |
| Mute semantics on raise | Addition not approved by the owner (W4) |

## 6. Task completeness
Tasks 2.4b and 2.5 are marked done and the code and tests exist; 2.4 [~] and 2.4c [ ] are correctly open. No unchecked core task for this PR.

## 7. Evidence limits
Not run by me: the release exe, the two state-changing ignored tests, the read-only ignored tests (the implementer reports running them), the island ACL runtime probe, and the RED-phase evidence (taken from the Batch 28 notes; not observable from the final tree).

## 8. Post-review fixes (Batch 28, written by the implementer, not re-verified)

Each finding of this report and what was done. Detail, RED evidence and counts are in `tasks.md` ("Post-review fixes") and `design.md` ("Review fixes"). Nothing was committed; `safety*`, `eval/commands.yaml`, the prompt and the tool schemas are untouched; E1 was not re-run.

| Finding | Fix | Evidence |
|---|---|---|
| W1 re-derivation | Recorded in `design.md` after "E1 reconciliation": with 812.0 ms the action-path mean is about 2.39 s against 2.5 s (passes); the 800 ms gate missed by 12 ms on a warm GPU; cool re-run pending; past figures not edited | docs only |
| W2 release ACL | Not changed. Listed as an implementer deviation for the owner in `design.md`; still guarded by the file tests, `check:release` and the runtime probe | docs only |
| W3 per-request LLM in debug | Not changed. Listed as an implementer deviation for the owner | docs only |
| W4 unmute-on-raise | Not changed (the owner decides) | none |
| W5 `vol-es-unmute` | Added to BACKLOG and to the open points as a user-facing risk | docs only |
| W6 second barrier | `close_app_fails_and_touches_nothing_even_with_a_real_approval`, `power_fails_and_touches_nothing_even_with_a_real_approval` (`Approved` values come from the real gate) | mutation: both FAILED |
| W7 weak tests | `the_models_text_never_reaches_the_launcher` asserts the fuzzy path was reached; `a_slow_launch_does_not_hold_the_index` replaces the test that could not fail; the text window logic is in `command.ts` and tested | see tasks.md |
| S1 Not verified | Extended (tasks.md, Batch 28) | docs only |
| S4 `Msg::NotAvailableYet` | BACKLOG | docs only |
| Review-added (not in the findings) | `open_app` allow-list and uninstaller filter, short-query rule, launched name in the reply, `ShellExecuteExW`, release profile unwinds, the refusal in `commit_plan` (a real bypass through `plan_pinned`), keeper re-warm rule, index backoff and `SystemRoot` fail-closed, in-flight guard in the text window | tasks.md |

Updated counts: host 398, niva-core 480, doc 12, eval lib 469, vitest 234; clippy x3, fmt, `npm run build`, `tauri build --no-bundle` and `check:release` clean.

### Not verified (extended)
`ShellExecuteExW` with `NO_UI | NOASYNC` on real apps and on a dead entry; "Open twice raises the same window"; Enter clearing the box in a real webview; the keeper against a real Ollama restart; the `check:release` needles failing on a release exe that contains one; the release exe surviving a worker panic; the positive island ACL path; the unmute-on-raise rule and the keeper's re-warm policy (owner decisions); the cool-GPU E1 re-run.
