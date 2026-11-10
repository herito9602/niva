# Action Safety Specification (delta against niva-mvp)

Baseline: `openspec/changes/niva-mvp/specs/action-safety/spec.md`. Unchanged: "Risk classification", "Timeout cancels", "Ambiguous answer", "Substring matching for destructive targets". The type-level gate (`Approved<T>`) is unchanged. This delta is safety-critical and ships in its own PR (slice 1).

## MODIFIED Requirements

### Requirement: Confirmation for risky actions
A risky action MUST NOT execute until the user explicitly confirms. Enforcement MUST be in code, independent of the model's output or prompt. Across the eval set, 0 risky actions may run without confirmation. A pending confirmation is shown by the notch's amber pulse and announced by voice; there is no on-screen prompt or button.
(Previously: the mascot in confirmation-needed state.)

#### Scenario: Confirmed
- GIVEN "close Chrome" and the notch in the confirmation look
- WHEN the user says yes
- THEN Chrome is closed

#### Scenario: Denied
- GIVEN a pending confirmation
- WHEN the user says no, presses Esc, or presses Ctrl+Shift+Space
- THEN the action does not run

#### Scenario: Model bypass attempt
- GIVEN a model that calls the close tool as if already confirmed
- WHEN the call reaches the gate
- THEN the gate still requires user confirmation

### Requirement: Sleep countdown
After the user accepts a sleep confirmation by voice, the system MUST NOT suspend the PC at once. It MUST announce by voice a 10 s countdown (the notch shows the amber pulse, no number), and it MUST call the OS only when the countdown ends and nobody cancelled it. Any keyboard or mouse input MUST cancel the countdown (detected by polling the time of the last input, never by a keyboard hook and never by reading which key). The shortcut Ctrl+Shift+Space MUST cancel it too. Restart and shutdown MUST have no countdown and MUST be confirmed only by Ctrl+Shift+Enter (see below); lock MUST have none. The approval of a cancelled countdown MUST NOT be usable again.
(Previously: accepted by voice or by click; restart and shutdown click-only; the countdown seconds were shown.)

The scenarios "Accepted and left alone", "Cancelled by input", "Cancelled by the shortcut", "The accepting input does not cancel" (read: the accepting spoken answer or key does not cancel it, through the same baseline and 1 s grace period), "Cancel and deadline race", "Stale deadline", "Activity cannot be read (fail closed)", "Host never confirms it is watching (fail closed)", "Polling only while counting down" and "Reminder during the countdown" are unchanged.

#### Scenario: Other power actions unchanged
- GIVEN a restart, shutdown or lock request
- WHEN the user confirms
- THEN there is no countdown (restart and shutdown only by Ctrl+Shift+Enter)

### Requirement: The release asks for confirmations (PR 2d-b; replaces "a build that cannot confirm does not ask" of PR 2d-a)
The release MUST enter the confirmation state for a risky request like any other build and MUST act only after a valid confirmation answer: a spoken yes for close and sleep, or the Ctrl+Shift+Enter chord for restart and shutdown. There is no build-time switch that refuses risky requests.
(Previously: a Confirm click that passes the host's guard.)

#### Scenario: The release asks before it acts
- GIVEN the release build and the request "close Spotify" (or "shut down the PC")
- WHEN the model proposes the risky call
- THEN the confirmation state is entered with the amber pulse and a spoken prompt, and nothing is closed or powered before a valid confirmation

#### Scenario: Restart and shutdown never confirm by speech
- GIVEN a restart or shutdown confirmation
- WHEN the user answers by voice, or ambient speech contains "yes" or similar
- THEN the action does not run and the confirmation stays pending until Esc, Ctrl+Shift+Space, the chord or the timeout

## REMOVED Requirements

### Requirement: The island's click answers a confirmation (PR 2d-b)
Reason: the notch is always click-through and has no buttons, so a click can no longer reach the core.
Migration: the host command for click answers, the rectangles report, the 500 ms button guard and the `island-actions` capability are removed, but ONLY after the Ctrl+Shift+Enter channel (below) has landed. Replacements: "Esc cancels a pending confirmation", "Ctrl+Shift+Enter confirms restart and shutdown", "Confirmation shortcuts exist only while pending".

## ADDED Requirements

### Requirement: Esc cancels a pending confirmation
While a confirmation is pending, pressing Esc MUST cancel it, producing `Reject { id }` for that confirmation, and the action MUST NOT run. This applies to every confirmation kind.

#### Scenario: Esc cancels
- GIVEN a pending confirmation to close an app
- WHEN the user presses Esc
- THEN the confirmation is cancelled, the action does not run and Niva says it was cancelled

#### Scenario: Esc with nothing pending
- GIVEN no confirmation is pending
- WHEN the user presses Esc in any app
- THEN Niva does not react and the key reaches the focused app

### Requirement: Ctrl+Shift+Enter confirms restart and shutdown
While a restart or shutdown confirmation is pending, pressing Ctrl+Shift+Enter within the confirmation window (10 s) MUST confirm it, producing `Accept { id }` for that confirmation. The chord MUST carry only the id of the confirmation on screen. Speech MUST NOT confirm restart or shutdown (including ambient speech). The chord MUST NOT confirm any other kind of confirmation. Ctrl+Shift+Space MUST still reject a pending confirmation.

#### Scenario: Chord confirms shutdown
- GIVEN a pending "Shut down the PC?" confirmation
- WHEN the user presses Ctrl+Shift+Enter within 10 s
- THEN the shutdown is requested

#### Scenario: Chord on another kind
- GIVEN a pending "Close Spotify?" confirmation
- WHEN the user presses Ctrl+Shift+Enter
- THEN nothing is confirmed and the action does not run

#### Scenario: Ambient speech
- GIVEN a pending restart confirmation and a TV in the room saying "yes, do it"
- WHEN the sidecar transcribes it
- THEN the restart does not run

#### Scenario: Ctrl+Shift+Space rejects
- GIVEN a pending restart or shutdown confirmation
- WHEN the user presses Ctrl+Shift+Space
- THEN the confirmation is rejected and the action does not run

#### Scenario: Stale or repeated chord
- GIVEN a confirmation was answered or replaced by the next prompt
- WHEN a chord press arrives for the old id
- THEN it is dropped and the reducer state does not change

### Requirement: Confirmation shortcuts exist only while pending
The Esc shortcut MUST be registered only while a confirmation is pending. The Ctrl+Shift+Enter shortcut MUST be registered only while a restart or shutdown confirmation is pending. The chord MUST be registered only after the prompt has been spoken and the 10 s window runs, and a press within 500 ms of arming MUST be ignored. Both MUST be unregistered on every exit: confirm, reject (voice, Esc or Ctrl+Shift+Space), timeout, replacement by another confirmation of a kind that no longer matches, the island being hidden (`Island{visible:false}`), and app quit. A shortcut event carrying a stale confirmation id MUST be dropped. A failed registration of the chord MUST reject that confirmation (fail closed) and set the tray tooltip "Confirm key unavailable"; a failed registration of Esc MUST be logged and MUST NOT end the confirmation.

#### Scenario: Registration lifecycle
- GIVEN no confirmation is pending
- WHEN a restart confirmation is entered
- THEN Esc is registered at once
- AND Ctrl+Shift+Enter is registered only once the prompt has been spoken and the 10 s window runs
- AND on confirm, reject, timeout or quit both are unregistered

#### Scenario: Close confirmation registers Esc only
- GIVEN a "close app" confirmation is entered
- WHEN the shortcuts are inspected
- THEN Esc is registered and Ctrl+Shift+Enter is not

#### Scenario: Chord pressed before the prompt ends is dropped
- GIVEN a restart confirmation whose prompt is still being spoken
- WHEN the user presses Ctrl+Shift+Enter
- THEN nothing is confirmed (the chord is not registered yet) and the action does not run

#### Scenario: Chord pressed within 500 ms of arming is ignored
- GIVEN the chord was registered less than 500 ms ago
- WHEN the user presses Ctrl+Shift+Enter
- THEN the press is ignored and the confirmation stays pending

#### Scenario: Leak check on every exit path
- GIVEN each exit path (accept, reject by voice, Esc, Ctrl+Shift+Space, timeout, quit)
- WHEN the confirmation ends by that path
- THEN no confirmation shortcut remains registered

#### Scenario: Hidden island and quit unregister
- GIVEN a confirmation shortcut is registered
- WHEN the island is hidden (`Island{visible:false}`) or the app quits
- THEN the host unregisters both shortcuts

#### Scenario: Stale confirmation id
- GIVEN a shortcut event carrying the id of a confirmation that was answered or replaced
- WHEN the host receives it
- THEN it is dropped and the reducer state does not change

#### Scenario: Registration fails (chord)
- GIVEN the OS refuses to register Ctrl+Shift+Enter at the late registration point
- WHEN a shutdown confirmation is waiting for its chord
- THEN the confirmation is rejected (Niva says "Cancelled"), nothing runs and the tray tooltip is set to "Confirm key unavailable" (fail closed)

#### Scenario: Registration fails (Esc)
- GIVEN the OS refuses to register Esc
- WHEN a confirmation is entered
- THEN the failure is logged and the confirmation continues
- AND voice, the timeout and Ctrl+Shift+Space still work

### Requirement: Restart and shutdown always keep a confirmation channel
Restart and shutdown MUST NOT be reachable without a working confirmation channel. The click channel (buttons, host click command, `island-actions` capability, button guard) MUST NOT be removed in any release before Ctrl+Shift+Enter is implemented and tested in that release. If the chord cannot be registered, the request MUST be cancelled (fail closed); the system MUST NOT fall back to speech or to auto-confirm.

#### Scenario: Slice order
- GIVEN the change is delivered in slices
- WHEN the slice that removes the buttons and `confirm_buttons.rs` is applied
- THEN the Ctrl+Shift+Enter channel and its lifecycle tests are already in the same or an earlier release

#### Scenario: Chord slice is never reverted alone
- GIVEN the buttons have been removed
- WHEN a rollback is planned
- THEN the chord slice is not reverted on its own: the buttons are removed only after the chord has shipped, and once removed the chord stays

## Assumptions to confirm
- The confirmation window for the chord is the existing 10 s timeout.
