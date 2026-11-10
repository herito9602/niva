# Island Shell Specification (delta against niva-mvp)

Baseline: `openspec/changes/niva-mvp/specs/island-shell/spec.md`. Unchanged: "Tray icon" and "Global shortcut".

## MODIFIED Requirements

### Requirement: Island window
The system MUST show a transparent, non-focusable, always-on-top window of about 200x70 px at the top center of the primary display, holding the notch. The notch MUST be horizontally centered and flush with the top edge. The background outside the notch MUST be transparent.
(Previously: a transparent, always-on-top island of 640x232 px sized for text, mascot and buttons.)

#### Scenario: Island placement
- GIVEN Niva is running
- WHEN the notch is shown
- THEN the window is about 200x70 px, horizontally centered at the top of the primary display and above normal windows
- AND it is transparent and does not take focus

### Requirement: Click-through (owner request 2026-11-03)
The window MUST ignore the mouse everywhere, always, including the notch and the transparent area. There MUST be no region that captures the mouse, no cursor poller and no hit-test in the host, and no webview permission to change click-through.
(Previously: the confirmation buttons' rectangles could capture the mouse, driven by a cursor poller and hit-testing.)

#### Scenario: Click on an app under the window
- GIVEN the notch is shown over another app
- WHEN the user clicks on the part of that app covered by the window
- THEN the click reaches that app

#### Scenario: No capture in any state
- GIVEN a confirmation is pending
- WHEN the user clicks anywhere in the window area
- THEN the click reaches the app below and no answer is produced

### Requirement: Acting stays visible (owner request 2026-11-04)
The notch MUST keep the plain `acting` look (the thinking wave) on screen for at least 900 ms before it shows a `success` or `error` result. This is display pacing only: the action MUST NOT wait for it and the reducer's phase MUST NOT change. Any other view (a reminder alert, a confirmation, a cancel) MUST show at once and drop a held result. The sleep countdown is never held.
(Previously worded for the mascot; behavior unchanged.)

#### Scenario: A quick action
- GIVEN an action that finishes 40 ms after `acting` appeared
- WHEN the result arrives
- THEN the notch stays in the acting look until 900 ms have passed, and then the result shows

#### Scenario: A reminder during the hold
- GIVEN a result held behind `acting`
- WHEN a reminder alert fires
- THEN the alert shows at once and the held result never appears

### Requirement: Type a command window (task 2.5, PR 2d-a)
Unchanged except for two points. The answer to a typed command is spoken and shown only as the notch look, not as text on the island. A confirmation is answered by voice, Esc, Ctrl+Shift+Space or (restart and shutdown only) Ctrl+Shift+Enter; typed text MUST NOT count as a confirmation answer.
(Previously: "the answer appears on the island as usual" and "answered by a click or a spoken answer only".)

#### Scenario: Submit with Enter
- GIVEN the window is open and the user typed "open notepad"
- WHEN the user presses Enter
- THEN the text reaches the assistant as a typed command, the box is empty again and Niva answers by voice with the notch showing its look

#### Scenario: Typed text is not a confirmation
- GIVEN a confirmation is open
- WHEN the user types "yes" in the window
- THEN the typed text is ignored

### Requirement: Show/hide lifecycle
The system MUST hide the notch when an interaction ends and show it again on the next activation. The hide MUST happen after the speech has ended plus the settle time (1.5 s, down from 3 s) only, with no reading time. The host MUST NOT hide the window before the notch's exit animation has finished, and the hide delay MUST be tied to that exit duration by a contract test.
(Previously: the idle period was stretched to the reading time of the message.)

#### Scenario: Auto-hide
- GIVEN a command has completed and Niva has finished replying
- WHEN the settle time (1.5 s, down from 3 s) elapses
- THEN the notch plays its exit, the window is hidden after the exit time, and the tray icon remains

#### Scenario: Long reply
- GIVEN a reply of about 400 characters that is spoken
- WHEN the speech ends
- THEN the settle time (1.5 s) starts at the end of the speech, with no extra hold, and the reply is spoken in full

## REMOVED Requirements

### Requirement: Island fits its message (owner request 2026-11-03)
Reason: no text is shown; the notch has a fixed size.
Migration: `notch`, "Notch geometry and waveform", and "No reply text and no buttons". Long replies are spoken in full; barge-in still stops them.

### Requirement: Edge extension animation (owner request 2026-11-03; chosen by the owner on 2026-11-03 among three prototypes)
Reason: the growing edge shape with concave fillets belongs to the text island.
Migration: `notch`, "Idle is hidden" (spring drop-in) and `island-shell`, "Show/hide lifecycle" (exit time before hide).

### Requirement: The island stays visible long enough to read the full message (owner request 2026-11-03)
Reason: there is no message to read; hold time was only for reading.
Migration: `island-shell`, "Show/hide lifecycle" (settle only), and `notch`, "Success and error auto-hide".

### Requirement: Confirmation buttons (PR 2d-b)
Reason: the notch has no buttons and is always click-through.
Migration: confirmations are answered by voice, Esc, Ctrl+Shift+Space and, for restart and shutdown, Ctrl+Shift+Enter (`action-safety`). The buttons and their host guard are removed only after the Ctrl+Shift+Enter channel exists (see `action-safety`, "Restart and shutdown always keep a confirmation channel").

### Requirement: Countdown display
Reason: no numbers are shown.
Migration: the sleep countdown shows the amber pulse (`notch`, "Palette and motion per state") and is announced by voice (`action-safety`, "Sleep countdown").

## Notes
- Design L4 fixes the window at 200x70 px.
