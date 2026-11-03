# Verify report: island PR (branch `feat/island-notch`, Batches 20-24)

Mode: openspec, Strict TDD active. RED evidence is claimed in tasks.md Batches 20-24; it cannot be re-run on finished code, so only GREEN was verified. Scope: uncommitted diff plus untracked files.

## Execution

| Command | Result |
|---|---|
| `cargo test --workspace` (PowerShell) | pass: host 148, niva-core 358, doc tests 6, 0 failed |
| `cargo clippy --workspace --all-targets -- -D warnings` | clean |
| `cargo fmt --check` | clean |
| `npm test` (app/) | pass: 19 files, 212 tests |
| `npm run build` (app/) | pass; `dist/` = index.html, one JS, one CSS, two svg. No `island-preview`, no preview string. |

No real sleeps in the new tests: Rust uses `start_paused`, vitest tests are pure functions of time.

## Verdict

0 CRITICAL, 5 WARNING, 7 SUGGESTION. Nothing blocks archive; the warnings are hardening and test-gap items.

## Scenario to test table

| Requirement / scenario | Implementation | Test | Status |
|---|---|---|---|
| Fits: short message, compact pill | `layout.ts` computeLayout | `layout.test.ts` "keeps a short single line", "never goes below the minimum width" | covered (pure) |
| Fits: wraps 2-6 lines, nothing cut | `layout.ts`, `.txt overflow-wrap:anywhere` | `layout.test.ts` "wraps at the maximum width", "grows taller up to six lines" | covered (pure); real browser measurement untested (W5) |
| Fits: more than 6 lines scrolls, rests, loops, last line readable | `scroll.ts`, `Island.tsx:196-208` | `scroll.test.ts` (start pause, reaches end exactly, end pause, loop, speed) | covered (pure); rAF wiring untested (W4) |
| Reduced motion pages with fade | `scroll.ts` pagedFrame, `Island.tsx` | `scroll.test.ts` pagedFrame suite | covered (pure) |
| Message changes while shown | `planTextChange`, crossfade in `Island.tsx:130-137`, `edgeResize` | `choreography.test.ts` planTextChange, `edge.test.ts` edgeResize | partial: decision and spring tested, component effect not |
| Window large enough (640x200) | `tauri.conf.json` | `island_contract_tests.rs` fits / not wasteful | covered, reads TS constants |
| Click-through: window ignores cursor | `click_through.rs`, `lib.rs:51-53, 68, 191` | `click_through.rs` 7 tests (decision only) | decision covered; startup and per-show wiring untested (W3) |
| Click-through: no webview permission | capability not in the diff; CSP unchanged (only width/height changed in tauri.conf.json) | `capabilities_tests.rs` exact-permission equality (pre-existing) | covered |
| Guard against clickable island elements | `clickThrough.test.ts` | 3 tests | covered, narrow (S6) |
| Edge: flush to y=0, fillets, rounded bottom | `shape.ts`, `edge.ts` | `shape.test.ts`, `edge.test.ts` edgeSettled / edgeIntro flush | covered |
| Edge: appear springs, 3-6 % overshoot, no translation, content last | `edge.ts`, CSS `content-in` | `edge.test.ts` (overshoot band, grows down and out); CSS timing visual only | covered (shape) |
| Edge: disappear 380 ms, no bounce, host does not hide early | `edgeOut`, `HIDE_DELAY_MS=540` | `edge.test.ts` edgeOut (4 tests), `island_contract_tests.rs` delay >= EDGE_OUT_MS and <= +250 | covered |
| Reduced motion: fade of the settled shape | `Island.tsx:77-80`, CSS `.rm` | none | NOT tested (W4) |
| Reading: short reply 3 s after speech | `reading.rs`, `effect_runner.rs:168` | `a_short_final_reply_settles_after_3_s_as_before`, dispatcher `a_short_reply_still_settles_after_3_s_with_the_reading_rule` | covered |
| Reading: long reply 12 s cap, no speech | same | `a_long_final_reply_stays_for_its_reading_time`, dispatcher `a_long_reply_stays_up_long_enough_to_read_it` | covered |
| Reading: 150 chars, speech 4 s, hide at 9.75 s | same | `speech_that_ends_before_the_reading_time_waits_for_the_rest` | covered exactly |
| Reading: new activation during hold cancels hide | reducer CancelTimer + `drop_timer` | dispatcher `a_shortcut_during_the_reading_hold_barges_in_and_the_old_hold_is_gone` | covered |
| Reading: prompts and follow-ups unaffected | `speak()` sets the hold only for `ReplyKind::Final` | `a_confirmation_prompt_never_gets_a_reading_hold` (ConfirmPrompt only; AskFollowUp and Alert untested) | partial |
| Reading: applied once | `reading.take()` | `the_reading_time_belongs_to_one_reply_only` | covered, but see W1 |
| Auto-hide after the reading time | same | same | covered |

## Findings

### CRITICAL

None.

### WARNING

**W1. The reading hold is consumed by any timer start and is not cleared by `StopSpeaking`.**
`app/src-tauri/src/effect_runner.rs:168` evaluates `self.reading.take()` inside the match on `(kind, ...)`, so it runs for every `start_timer` call, not only `Settle`. Line 145 (`Effect::StopSpeaking`) does not clear it either. Today the code is correct only by coincidence: after a barge-in the reducer starts `NoSpeech` or `Llm`, which swallows the stale record. Failure scenario (slice 3, real TTS, speech held): a long Final reply is spoken, and the reducer starts any non-Settle timer between that `Speak` and its `SpeakDone`. The hold is silently dropped and the 400-character reply hides 3 s after the speech instead of 12 s after it was shown. The reverse case: a record that survives because no timer starts keeps the `shown_at` of an old reply, and a later unrelated `Settle` is stretched wrongly. No test pins either path (the only "once" test uses two Settles back to back).
Fix: take the hold only when `kind == Settle`, key it by speak id, and clear it on `StopSpeaking`. Tests: final long Speak, then a NoSpeech timer start, then Settle must still use the hold; Speak, StopSpeaking, Settle must be a plain 3 s.

**W2. A reply of roughly 800+ characters is hidden before its last line has scrolled into view.**
`crates/niva-core/src/reading.rs:19` caps the hold at 12 s, and the plain LLM reply path is uncapped (`crates/niva-core/src/assistant.rs:378`; only the search summary is cut at 400 by `SUMMARY_MAX_CHARS`, line 39). The scroll clock starts 900 ms after the intro (`Island.tsx:200`), rests 1.8 s, then travels at 26 px/s. A 1500-character answer is about 28 lines: scrollDistance = 22 lines x 20 px + 5 = 445 px, travel about 17 s. The island hides at 12 s with most of the text unseen, which violates "MUST NOT truncate ... so all of it can be read" for any reply the model writes at that length. The design says "scrolls past it anyway" but the spec requirement has no length bound. For about 400 characters the timeline is fine (end rest at roughly 4.4 to 6.6 s).
Fix (pick one): cap plain replies like the summary; or add the scroll cycle to the hold for scrolling texts; or document the bound in the spec.

**W3. The startup and per-show click-through calls are untested and their errors are swallowed.**
`app/src-tauri/src/lib.rs:52` ignores the result of `win.set_ignore_cursor_events(...)`, called at `lib.rs:68` (every show) and `lib.rs:191` (startup). No test asserts that `show_island` or setup invoke it, and a failure is invisible. Failure scenario: if the call errors (window not ready, platform refusal), the 640x200 transparent window silently swallows every click on the top-centre strip of the screen, the exact regression the requirement forbids, with nothing in the log. "Ignores the cursor at startup and on every show" is only a manual open point (Batch 20, open point 1).
Fix: log the error at minimum; put the call behind a small closure or trait so a test can assert it runs on show and at setup.

**W4. No component-level test for `Island.tsx`; lifecycle claims rest on code review.**
`app/src/island-ui/Island.tsx` (245 lines: rAF loops, crossfade timer, matchMedia listener, measurement) has no test because vitest runs without a DOM. Verified by reading only: the shape loop stops on settle (`:106-124` returns false) and on cleanup (`:127`); the scroll loop stops on `out` and on unmount (`:196-208`); the crossfade `setTimeout` is cleared (`:135`); the matchMedia listener is removed (`:34`); nothing is created while hidden. There is no `ResizeObserver`, so nothing to leak. Review found no leak, but a regression would pass the suite. Reduced motion for the shape and for paged text is untested too.
Fix: add a jsdom or happy-dom test with a fake rAF that mounts `<Island>`, drives `in` then `out`, and asserts the rAF queue is empty after settle and after unmount.

**W5. Text measurement depends on the browser and is unverified for a hidden window, emoji and CJK.**
`Island.tsx:98-101` measures two probes while the island window is `visible:false` on first use (Batch 20 open point 1 flags it). `layout.ts:66` derives the line count as `round(blockHeight / 20)`. With `line-height: 20px`, a line with emoji or CJK glyphs from a fallback font can grow its line box by 1 to 3 px in Chromium, so a 14-line message can measure 14.6 lines and round to 15. The label (same inflated lines) then needs a different scrollDistance than computed and the last line can be clipped by the fade. The very-long-word case is handled (`overflow-wrap:anywhere` on probe and label through `.txt`), and fonts are system fonts (Segoe UI), so a font-loading race does not apply. `preview.tsx` has no emoji or CJK sample.
Fix: measure the real label (`scrollHeight`) instead of inferring from a probe, or add emoji and CJK samples to the preview and check them by hand.

### SUGGESTION

**S1.** `Island.tsx:58`: the `clientWidth ?? 640` fallback does not guard a width of 0 from a hidden webview; the shape would be centred at x=0 for that frame. Use `||`.

**S2.** `Island.tsx:114` plus the crossfade `setShown` creates a new `layout` object for every text swap even when width and height are equal, so the effect restarts a no-op `edgeResize` rAF loop for about 0.7 s. Skip it when the size is equal.

**S3.** Re-show during the exit: the intro restarts from h=0 while the shape is partly retracted, so it pops back to the notch. Accepted in Batch 20 open point 5; a spring from the current shape (like `edgeResize`) would remove it.

**S4.** `layout.ts` says its numbers are mirrored in styles.css (`PAD_X` 24, `GAP` 14, `LINE_HEIGHT` 20, font 15px) but only the three durations are checked by a test (`choreography.test.ts`). Changing the CSS padding or line-height would silently break the line count and the window budget. Add a test that reads those literals from `styles.css`.

**S5.** Exports used only inside their own module: `TextPlan`, `PAD_X`, `PAD_Y`, `TEXT_PAD_Y`, `GAP`, `BORDER`, `Measured`, `ScrollConfig`, `ScrollFrame`, `ScrollInfo`, `SHADOW_ROOM_Y`. Harmless. No leftover `droplet`, `ISLAND_STYLE`, `NOTCH_` or `DROPLET_` identifiers in source, CSS or tests (they appear only in the historical Batch 21-23 text of tasks.md, correct as history). No orphan CSS class: every selector maps to a class in `Island.tsx` or `MascotCanvas.tsx`.

**S6.** The click guard (`clickThrough.test.ts:13`) misses `onKeyDown`, `onPointerEnter`, `onPointerOver`, `onTouch*`, `role="button"`, `tabIndex`, and does not scan `mascot/MascotCanvas.tsx`, which renders inside the island. Widen the pattern to any `on` + capital handler with an allow-list, and include the mascot folder.

**S7.** Design choice, host-side Settle stretch versus a reducer change. Assessment: defensible. The reducer stays pure and untouched, token and stale-timer rules are unchanged (the old Settle is aborted by `drop_timer`; a dispatcher test shows a barge-in cancels it), and the runner already owns the speak-watchdog duration. The cost is a hidden coupling: a one-shot mutable side channel (the next Settle after a Final Speak) instead of data carried by the effect, which is exactly what W1 exposes, and the rule cannot be seen from `niva-core` tests (half of the contract lives in `reading.rs`, half in the runner). A cleaner variant is to key the hold by speak id, or to let `StartTimer` carry a minimum remaining time computed by the reducer. Not worth changing now if W1 is fixed. Also, `shown_at` is the moment the `Speak` effect ran, not the first painted frame (Batch 24 open point 2): fine today, revisit when TTS lands.

## Checks that passed (no finding)

- Click-through: `apply_click_through` runs in setup and in every `show_island` (`lib.rs:68, 191`). `capabilities/default.json` is not in the diff and the CSP string is unchanged (only width and height changed in `tauri.conf.json`). `capabilities_tests.rs` pins the island permissions to listen and unlisten with no window permission, so the webview cannot call `set_ignore_cursor_events`.
- Reading time arithmetic: floor 2.5 s, cap 12 s, overflow-safe (`checked_mul`), monotonic, `settle_delay` never below 3 s. Replies of 28 or more characters can exceed 3 s by up to 40 ms, consistent with the design note that 27 or fewer keep the plain 3 s.
- Stale timers: `start_timer` aborts the previous task of the same kind; tokens are reducer-owned and untouched. Confirmation, follow-up and alert replies leave no hold because `speak()` overwrites it with None for every non-Final kind.
- Animation: `HIDE_DELAY_MS` (540) >= `EDGE_OUT_MS` (380) and within +250; the contract test reads `EDGE_OUT_MS` from `choreography.ts` and the window constants from `layout.ts`; the CSS durations are pinned to the TS constants.
- Window budget: 560+80 = 640 wide; 142+14+28 = 184 <= 200 tall; the width overshoot (about 9 px per side) plus the 10 px fillet stays inside the 40 px side room.
- Auto-scroll for 400 characters (about 8 lines): 0.9 s intro, 1.8 s rest, 1.7 s travel, 2.2 s end rest, all inside the 12 s hold.
- Preview page: `island-preview.html` is not a build input; `dist/` has no preview file or string.
- Test quality: behaviour-first names, deterministic, no real sleeps, no randomness.

## Recommendation

`next_recommended`: sdd-apply for W1 and W3 (small, one test each) and an owner decision on W2; then sdd-archive. A manual desktop check is still pending (Batch 20 open point 1, Batch 24 open point 4).

## Resolution (Batch 25, 2026-11-03)

| Finding | Outcome |
|---|---|
| W1 | Fixed. Hold keyed to the final speak id, consumed only by Settle, cleared by StopSpeaking and replaced by a new Speak. Two RED tests plus one pin. |
| W2 | Decided by spec: replies are at most about 400 characters (assistant-core "Replies are short"); island-shell states the reading time covers that bound. Implementation is task 2.4a in PR 2c. |
| W3 | Fixed. `IslandWindow` seam; startup and every show tested; a refusal is logged with `[niva]` and the show continues. |
| S1 | Fixed (`||`). |
| S6 | Fixed. Wider pattern, mascot scanned, positive and negative pattern tests. |
| W4, W5, S2, S3, S4, S5, S7 | Skipped, recorded as open points in tasks.md Batch 25. |

Re-run: cargo test host 154, niva-core 358, doc 6; npm test 213; clippy, fmt, build clean; release build and check:release ok.
