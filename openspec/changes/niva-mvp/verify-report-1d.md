# Verify report (final): niva-mvp PR 1d (mascot), tasks 1.21-1.25

Branch `feat/1d-mascot`, uncommitted. Strict TDD mode. Fresh-context re-verification after Batches 6-13. Aesthetics not judged (owner approved the look).

## Evidence
- `npm test` (app/): 9 files, 128 tests passed.
- `npm run build` (app/): tsc + vite OK; dist has only index.html and one JS/CSS bundle. No mascot-preview.html in dist; the bundle has no preview text, and `URLSearchParams`/`?state=`/`?t=` are read only in `src/mascot/preview.tsx`, which only `mascot-preview.html` loads. `MascotCanvas` exposes the `timeOverride`/`reducedMotion` props, but the app never passes them.
- `cargo test --workspace` (PowerShell): 86 + 161 passed, 0 failed (others 0 tests).
- `cargo clippy --workspace --all-targets -- -D warnings` (PowerShell): finished with no warnings or errors.
- `git diff --stat -- app/src-tauri app/package.json app/package-lock.json`: empty. No capability, CSP or dependency change.
- Lifecycle read-through of `MascotCanvas.tsx`: cleanup cancels rAF, disconnects the ResizeObserver and removes the DPR matchMedia listener; the loop only runs while `active` (`phase === "in"`). `drawRobot` ctx.save/restore are balanced (18 saves, 19 restores, the extra one is an early-return branch at robot.ts:635).
- `viewStore.bindHostEvents`: rejected `listen`, early unbind before resolution and throwing unlisten are all handled and tested.

## Spec coverage
| Requirement / scenario | Implementation | Test |
|---|---|---|
| 8 visibly different states, no hard cut | pose.ts `POSES`, robot.ts springs | pose.test, robot.test (converges without hard cut) |
| Idle breathes and blinks (2-6 s) | composeFrame, createBlinker | robot.test |
| State driven only by assistant state | pose.ts `poseForView` | pose.test |
| Thinking kept during AskFollowUp | host keeps `thinking`; `poseForView` ignores caption | pose.test |
| Gestures per state (scratch, thumbs-up + arcs + chuckle, spiral + wobble, laptop + typing, raised hand, cup ear, pans) | gesture.ts, geometry.ts, pose.ts `GESTURES` | gesture.test, geometry.test, pose.test, robot.test |
| Reminder-alert pans, CLANG, bounded, zero under reduced motion, static "!" | gesture.ts, robot.ts | gesture.test, robot.test (lines 169-190) |
| Reduced motion: loops static, blink and gaze still work | composeFrame `motion` factor | robot.test:134 |
| No image assets | primitives only, no image imports (rg for `drawImage`/`new Image`/image imports: none) | none (review only) |
| Debug switcher / preview | existing debug window + dev `mascot-preview.html` | manual |

## Findings

### CRITICAL
None.

### WARNING
1. `openspec/changes/niva-mvp/design.md` (Mascot section, "hands are drawn 1.3x their local geometry") vs `app/src/mascot/geometry.ts:8` `HAND_SCALE = 1.1`. Scenario: a later contributor trusts the design, sets the scale to 1.3, and the hands overshoot the owner-approved size. Fix the design wording to 1.1.
2. `app/src/mascot/robot.ts:306+` (`drawRobot`) and `app/src/mascot/MascotCanvas.tsx` have no test (old W5, still open). Scenario: a NaN in any new geometry term, or a regression that leaves a rAF running after unmount, passes tsc and the 128 tests. The spec scenario "Asset audit" is also review-only. Add a fake-canvas smoke test of `drawRobot` over all 8 poses and reduced motion (no throw, only finite numeric arguments) plus a test that `src/` has no image imports/`drawImage`.
3. `app/src/mascot/geometry.ts:40` (`quadPoint`) and `:193` (`fingertip`) are exported but used only by tests, and design.md cites `fingertip` as how the scratch fingertip lands on the head, while `robot.ts:370` recomputes the placement itself. Scenario: a leftover from the iterations; the tested helper can drift from what is actually drawn, so its tests give false confidence. Either draw through `fingertip` or delete both and their tests.

### SUGGESTION
1. `app/src/mascot/preview.tsx:15`: `Number("Infinity")` is not NaN, so `?t=Infinity` gives `FIXED_T = Infinity` and `composeFrame` yields NaN transforms (blank canvas). Dev only; use `Number.isFinite`.
2. `app/src/mascot/preview.tsx:48`: `timeOverride` is mis-indented (cosmetic).
3. `app/src/mascot/MascotCanvas.tsx:23-25`: dev-only props (`reducedMotion`, `timeOverride`) live on the production component. Harmless (the app never sets them, and `?t=` is unreachable from index.html), but they widen the prod API.
4. `app/src/mascot/pose.ts:217-225`: `poseForState`, `gestureForState`, `gestureForView` are not used by production code (only tests, and `poseForView` is the only prod entry). Candidates for inlining or removal.
5. `app/src/mascot/robot.ts` (composeFrame offsetX/tilt): the `+ 0` trick to normalise -0 relies on an inline comment; a small `noNegZero` helper would be clearer.
6. Old S3/S4/S5/S6 remain open (see below); all cosmetic or process.

## Previous findings
| Id | Status |
|---|---|
| W1 DPR change not handled | FIXED: `configure()` + re-armed `(resolution: Ndppx)` query + ResizeObserver, all cleaned up |
| W2 silent null 2d context | FIXED: `makeContextGetter` warns once, tested (robot.test:269) |
| W3 early-unbind path untested | FIXED: deferred-resolver test (viewStore.test "unbinding before listen resolves...") |
| W4 unhandled `listen` rejection | FIXED: `track().catch(onError)` and `.catch(onError)` on unlisten, both tested |
| W5 untested glue / asset audit | STILL OPEN (now WARNING 2 above) |
| S1 NaN/Infinity guards | FIXED: `stepSpring` ignores non-finite target, `lookVector` guards, tests added (robot.test:240-255, spring.test) |
| S2 bounce phase jump | FIXED: phase accumulated in `stepRig`, tested (robot.test:216) |
| S3 injectable RNG prop | STILL OPEN (cosmetic; `createBlinker` itself takes the RNG and is tested) |
| S4 stale cursor after hide | STILL OPEN (harmless) |
| S5 loop stops at `out`, freezes during slide-up | STILL OPEN (acceptable) |
| S6 1.24/1.25 checked while owner review pending | Acknowledged: owner has now approved the look; originality sign-off remains the owner's call |

## Strict TDD
Pure modules (spring, pose, gesture, geometry, robot maths, viewStore) are tested behaviour-first with injected RNG/clock and fixed dt; deterministic. Glue (`drawRobot`, `MascotCanvas`) is the only untested layer.

## Verdict
0 CRITICAL, 3 WARNING, 6 SUGGESTION. No blocker. Fix the design wording (W1) and decide on the dead helpers (W3); a smoke test (W2) is recommended before archive.


## Resolution
- W1 fixed: `design.md` now says hands are drawn 1.1x (`HAND_SCALE`); the other mascot numbers in design and spec were checked against the code.
- W2 fixed: `drawRobot` smoke test over all 8 states, reduced motion on/off and the hit moment (no throw, finite numbers, balanced save/restore); asset-import test for `src/mascot` and `src/state`; `MascotCanvas` unmount lifecycle test (React hooks stubbed, since jsdom and testing-library are not installed).
- W3 fixed: `fingertip`, `quadPoint`, `poseForState`, `gestureForState` and `gestureForView` removed (no production callers). The scratch-tip test now measures the drawn finger part; the AskFollowUp rule is tested through `poseForView`.
- S fixed: `preview.tsx` ignores non-finite `?t=`; indentation fixed.
- Re-verified: `npm test` 149 passed, `npm run build`, `cargo test --workspace`, `cargo clippy --workspace --all-targets -- -D warnings`.
