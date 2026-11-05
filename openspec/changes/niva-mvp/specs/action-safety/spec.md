# Action Safety Specification

## Purpose

A gate enforced in application code that classifies actions as risky or safe and requires user confirmation for risky ones.

## Requirements

### Requirement: Risk classification
Closing an app and power/lock/sleep actions MUST be classified risky. Opening apps, volume/mute, web search and reminders MUST be classified safe.

#### Scenario: Safe action
- GIVEN the request "volume to 30"
- WHEN the tool is called
- THEN it executes with no confirmation

### Requirement: Confirmation for risky actions
A risky action MUST NOT execute until the user explicitly confirms. Enforcement MUST be in code, independent of the model's output or prompt. Across the eval set, 0 risky actions may run without confirmation.

#### Scenario: Confirmed
- GIVEN "close Chrome" and the mascot in confirmation-needed state
- WHEN the user says yes
- THEN Chrome is closed

#### Scenario: Denied
- GIVEN a pending confirmation
- WHEN the user says no
- THEN the action does not run

#### Scenario: Model bypass attempt
- GIVEN a model that calls the close tool as if already confirmed
- WHEN the call reaches the gate
- THEN the gate still requires user confirmation

### Requirement: Timeout cancels
If no answer arrives within the confirmation timeout, the action MUST be cancelled.

#### Scenario: No answer
- GIVEN a pending confirmation
- WHEN the timeout elapses with no answer
- THEN the action does not run and the user is told it was cancelled

### Requirement: Ambiguous answer
An answer that is not clearly affirmative MUST NOT be treated as confirmation.

#### Scenario: Unclear reply
- GIVEN a pending confirmation
- WHEN the user says something unrelated
- THEN the action is not executed

### Requirement: Substring matching for destructive targets
Destructive app matching MUST use substring matching only, never fuzzy matching.

#### Scenario: Near-miss name
- GIVEN running apps "Notepad" and "Notion"
- WHEN the user asks to close "Notpad"
- THEN no app is closed and the user is told no match was found

### Requirement: Sleep countdown
After the user accepts a sleep confirmation (by voice or by click), the system MUST NOT suspend the PC at once. It MUST announce and show a 10 s countdown, and it MUST call the OS only when the countdown ends and nobody cancelled it. Any keyboard or mouse input MUST cancel the countdown (detected by polling the time of the last input, never by a keyboard hook and never by reading which key). The shortcut Ctrl+Shift+Space MUST cancel it too. Restart and shutdown MUST stay click-only with no countdown, and lock MUST have none. The approval of a cancelled countdown MUST NOT be usable again.

#### Scenario: Accepted and left alone
- GIVEN the confirmation "Put the PC to sleep?" and the user says yes
- WHEN 10 s pass with no input
- THEN Niva says "Suspending in 10 seconds. Press a key or move the mouse to cancel." at the start, the OS suspend call happens once at the end, and not before

#### Scenario: Cancelled by input
- GIVEN a running countdown and its 1 s grace period is over
- WHEN the user presses a key or moves the mouse
- THEN the countdown stops, Niva says "Suspend cancelled." (not "done") and the OS is never called

#### Scenario: Cancelled by the shortcut
- GIVEN a running countdown
- WHEN the user presses Ctrl+Shift+Space
- THEN the countdown stops, Niva says "Suspend cancelled." and the OS is never called

#### Scenario: The accepting input does not cancel
- GIVEN the user accepted by clicking, and the mouse jitters during the next second
- WHEN the countdown starts
- THEN that input does not cancel it (a baseline is taken at the start and input is ignored for a 1 s grace period)

#### Scenario: Cancel and deadline race
- GIVEN a cancel and the end of the countdown reach the system together
- WHEN the system handles them
- THEN exactly one wins: the one handled first. A cancel handled first means no OS call, ever. A deadline handled first means the call is released to the worker and a later countdown cancel (input, debug window) is ignored. A shortcut that arrives after the deadline, while the reducer is already acting on it, ends the request instead of being ignored, and may stop the sleep if the worker has not released it yet; that is the fail-safe direction

#### Scenario: Stale deadline
- GIVEN a countdown that was cancelled, reset or replaced by a new request
- WHEN its old timer fires
- THEN nothing happens

#### Scenario: Activity cannot be read (fail closed)
- GIVEN the host cannot read the last-input time when the countdown starts or while it runs
- WHEN that happens
- THEN the PC is not suspended and Niva says it could not check for input, so it did not suspend

#### Scenario: Host never confirms it is watching (fail closed)
- GIVEN a sleep was accepted
- WHEN the host does not confirm that it watches input within 5 s
- THEN the 10 s deadline never starts, the PC is not suspended and Niva says so

#### Scenario: Polling only while counting down
- GIVEN no countdown is running
- WHEN the system is idle, or the countdown just ended (finished, cancelled or failed)
- THEN nothing polls the last-input time

#### Scenario: Other power actions unchanged
- GIVEN a restart, shutdown or lock request
- WHEN the user confirms
- THEN there is no countdown (restart and shutdown only by click, as before)

#### Scenario: Reminder during the countdown
- GIVEN a running countdown
- WHEN a reminder falls due
- THEN it is queued and shown after the countdown ends, not over it

### Requirement: The release asks for confirmations (PR 2d-b; replaces "a build that cannot confirm does not ask" of PR 2d-a)
Now that the island's buttons answer a confirmation (below) and the real `close_app` and power adapters ship, the release MUST show the confirmation for a risky request like any other build, and MUST act only after a Confirm click that passes the host's guard. There is no build-time switch that refuses risky requests any more; the type-level gate (`Approved<T>`) is unchanged.

#### Scenario: The release asks before it acts
- GIVEN the release build and the request "close Spotify" (or "shut down the PC")
- WHEN the model proposes the risky call
- THEN the confirmation state is entered with the Confirm and Cancel buttons, and nothing is closed or powered before a Confirm click

#### Scenario: Restart and shutdown stay click-only
- GIVEN a restart or shutdown confirmation
- WHEN the user answers by voice
- THEN the action does not run

### Requirement: The island's click answers a confirmation (PR 2d-b)
A click on the island's Confirm or Cancel MUST reach the core only as `Accept { id }` or `Reject { id }`, through one host command that carries the confirmation id and the choice and nothing else (no text, no action name). The host MUST drop an answer whose id is not the confirmation on screen, a second answer to the same confirmation, and a Confirm that arrives before the island has reported its buttons (the first non-empty report of their rectangles for that confirmation, sent once they are mounted) or less than 500 ms after that report; this MUST be enforced in the host, not only in the page, on an injectable clock, so with no report a Confirm can never pass. The host MUST also reject a rectangle report as a whole if it has more than two rectangles or one bigger than a button. This command is the only way a click reaches the core in a release build, and nothing reachable from the model or typed text may produce one. A Cancel MUST always be honoured. A dropped answer MUST NOT be an error and MUST NOT change anything. The release webview MUST hold this command and the report of the buttons' rectangles in a capability of its own, and no debug-only command.

#### Scenario: A click as the island drops in
- GIVEN the island reported its buttons 200 ms ago
- WHEN a Confirm click arrives
- THEN nothing happens, and a Confirm after 500 ms is accepted

#### Scenario: A slow show
- GIVEN the confirmation was published but the island has not reported its buttons
- WHEN a Confirm click arrives, however late
- THEN it is dropped; a Cancel click still cancels

#### Scenario: Cancel at once
- GIVEN the buttons appeared 200 ms ago
- WHEN a Cancel click arrives
- THEN the confirmation is cancelled

#### Scenario: Double click and stale click
- GIVEN a confirmation was answered, or replaced by the next prompt
- WHEN another click arrives for it
- THEN it is dropped and the reducer state does not change

## Assumptions to confirm
- Timeout duration: 10 s (design "Confirmation flow"; user-confirmed).
- Confirmation by shortcut/click as well as voice is not specified; spec only requires an explicit affirmative.
- Several apps matching one substring: ask which one, never close all (design Safety Gate, `close_app` matching).
