# Island Shell Specification

## Purpose

The floating window, tray icon and shortcut through which the user sees and invokes Niva.

## Requirements

### Requirement: Island window
The system MUST show a transparent, always-on-top island at the top-center of the primary display.

#### Scenario: Island placement
- GIVEN Niva is running
- WHEN the island is shown
- THEN it is horizontally centered at the top of the primary display and above normal windows
- AND its background is transparent outside the island shape

### Requirement: Island fits its message (owner request 2026-11-03)
The island MUST grow to fit the message it shows and MUST NOT truncate it. A short message keeps a compact pill; a longer one makes the pill wider (up to about 560 px) and then taller (up to 6 lines). A message longer than 6 lines MUST scroll slowly and smoothly inside the island so all of it can be read, pausing at the start and at the end and looping gently. Messages of about 400 characters (search summaries) plus confirmations MUST fit this behavior. The island window MUST be large enough for the largest island plus the animation overshoot and the soft shadow.

#### Scenario: Short message
- GIVEN the island shows "Opening Notepad"
- THEN it is a compact one-line pill

#### Scenario: Message that wraps
- GIVEN a message of two to six lines
- WHEN it is shown
- THEN the pill is as wide as the maximum allows and as tall as the text needs, and no text is cut or ellipsized

#### Scenario: Message longer than six lines
- GIVEN a message of about 400 characters
- WHEN it is shown
- THEN the pill stops growing at six lines
- AND the text scrolls slowly, rests at the first and at the last line, and loops
- AND the last line is fully readable during the rest at the end

#### Scenario: Reduced motion
- GIVEN the system asks for reduced motion
- WHEN a message longer than six lines is shown
- THEN the text does not move: it is shown one page of six lines at a time, the pages changing through a short fade, so all of it can still be read

#### Scenario: Message changes while shown
- GIVEN the island is shown
- WHEN a new message arrives
- THEN the old text fades out, the pill resizes smoothly in width and height with no jump, and the new text fades in

### Requirement: Click-through (owner request 2026-11-03)
The island window MUST NOT block mouse input to the applications under it, including the transparent area around the island. While the island has no clickable control, the window MUST ignore the cursor entirely. When the island gets buttons (PR 2d confirm buttons), only the buttons' rectangles MAY capture the mouse (host-side hit-testing with the cursor poller); the rest MUST stay click-through. This is done in the Rust host; the island webview gets no new permission.

#### Scenario: Click on an app under the island window
- GIVEN the island is shown over another app
- WHEN the user clicks on the part of that app covered by the island window
- THEN the click reaches that app

### Requirement: Edge extension animation (owner request 2026-11-03; chosen by the owner on 2026-11-03 among three prototypes)
The island MUST look like the top edge of the screen extending downward, like the notch of a laptop. Its black shape is always flush with the top edge of the window (y = 0): it never floats and never drops as a detached pill. Its top corners merge into the screen edge with small outward concave fillets (inverse radius of about 8 to 12 px) and its bottom corners are rounded. Appear: it grows down and out from a small notch at the top edge; height goes from 0 to the island height and width from the notch width to the island width, both following overlapping damped springs with a soft overshoot of about 3 to 6 percent; only the bottom edge and the sides move, there is no vertical translation. The mascot and the text then fade in with a slight upward drift and a short blur-to-sharp, the text just after the mascot. Disappear: the content fades out first, then the shape retracts up into the edge (height to 0, width to the notch width) with near-critical damping and no bounce, within a bounded time (`EDGE_OUT_MS`, 380 ms; the appear takes 900 ms). The pace is the one the owner approved: a slower version was tried and rejected, so time to read is provided by the reading time below, not by slowing the animation. When the message changes the size springs to the new size, still anchored at the top. With reduced motion, both are a simple fade of the settled shape. The host MUST NOT hide the window before the exit has finished.

#### Scenario: Appear
- GIVEN the island is hidden
- WHEN the island is shown
- THEN a small notch at the top edge grows down and out to the message size, always joined to the edge through concave fillets
- AND the content fades in last

#### Scenario: Disappear
- GIVEN the island is shown
- WHEN the island is hidden
- THEN the content fades out, the shape retracts up into the top edge, and the window hides after the exit time

### Requirement: The island stays visible long enough to read the full message (owner request 2026-11-03)
After a final reply (success, error or cancel), the island MUST stay visible long enough to read the whole message before the exit starts: at least `clamp(1500 ms + 55 ms per character, 2500 ms, 12000 ms)` after the reply is shown, and never less than the usual 3 s settle after the speech ends. When speech is played, the island hides at the later of the two moments. Starting a new activation, the shortcut or a barge-in during that time works as usual and cancels the pending hide. Confirmation prompts and follow-up questions are not affected: they stay until answered or until their own timeout. The 12 s cap covers every reply up to the reply bound of `assistant-core` (about 400 characters): for that length the text scrolls once and its last line stays readable before the hide.

#### Scenario: Short reply
- GIVEN the reply is "Opening Notepad"
- WHEN it is shown
- THEN the island hides 3 s after the speech ends, as before

#### Scenario: Long reply
- GIVEN the reply is a search summary of about 400 characters
- WHEN it is shown and there is no speech yet
- THEN the island stays up for 12 s (the cap) while the text scrolls, and then hides

#### Scenario: Speech shorter than the reading time
- GIVEN a 150-character reply whose speech ends after 4 s
- WHEN the speech ends
- THEN the island hides at 9.75 s after the reply was shown, not 3 s after the speech

#### Scenario: New activation during the hold
- GIVEN a long reply is still being held
- WHEN the user presses the shortcut
- THEN Niva starts listening and the old hide never fires

### Requirement: Acting stays visible (owner request 2026-11-04)

The island MUST keep the plain `acting` state on screen for at least 900 ms before it shows a `success` or `error` result. This is display pacing only: the action MUST NOT wait for it and the reducer's phase MUST NOT change. Any other view (a reminder alert, a confirmation, a cancel) MUST show at once and drop a held result. The sleep countdown is never held.

#### Scenario: A quick action

- GIVEN an action that finishes 40 ms after `acting` appeared
- WHEN the result arrives
- THEN the mascot stays in `acting` until 900 ms have passed, and then the result shows

#### Scenario: A reminder during the hold

- GIVEN a result held behind `acting`
- WHEN a reminder alert fires
- THEN the alert shows at once and the held result never appears

### Requirement: Tray icon
The system MUST provide a tray icon with a show/hide toggle for the island, a "Type a command" entry and a "Quit" entry.

#### Scenario: Quit from tray
- GIVEN Niva is running
- WHEN the user chooses "Quit" in the tray menu
- THEN the island and tray icon disappear and the process exits

### Requirement: Type a command window (task 2.5, PR 2d-a)
The system MUST provide a small window, opened from the tray entry "Type a command", with a text box. Enter MUST submit the text as a typed command and clear the box; the answer appears on the island as usual. The window MUST be able to do one thing only: submit text. It MUST NOT be able to produce a confirmation answer, a click, a transcript or any other event, to listen to or emit events, or to control windows. Typed text MUST NOT count as a confirmation answer.

#### Scenario: Submit with Enter
- GIVEN the window is open and the user typed "open notepad"
- WHEN the user presses Enter
- THEN the text reaches the assistant as a typed command, the box is empty again and the island shows the reply

#### Scenario: Blank or long text
- GIVEN the box is empty or only spaces
- WHEN the user presses Enter
- THEN nothing is sent; text longer than 500 characters is cut to 500

#### Scenario: Open twice
- GIVEN the window is already open
- WHEN the user chooses "Type a command" again
- THEN the same window is brought to the front

#### Scenario: Least privilege
- GIVEN the window's capability file
- WHEN it is checked
- THEN it grants one permission (the text command) to the window "text" only, the island's capability is unchanged and cannot call that command, and the content security policy stays strict

#### Scenario: Typed text is not a confirmation
- GIVEN a confirmation is open
- WHEN the user types "yes" in the window
- THEN the typed text is ignored: confirmations are answered by a click or a spoken answer only

### Requirement: Global shortcut
The system MUST toggle activation on Ctrl+Shift+Space from any application.

#### Scenario: Shortcut shows island
- GIVEN the island is hidden and another app has focus
- WHEN the user presses Ctrl+Shift+Space
- THEN the island is shown and Niva starts listening

### Requirement: Show/hide lifecycle
The system MUST hide the island when an interaction ends and show it again on the next activation.

#### Scenario: Auto-hide
- GIVEN a command has completed and Niva has finished replying
- WHEN the idle period elapses (and the message has had its reading time)
- THEN the island is hidden and the tray icon remains

### Requirement: Countdown display
While a sleep countdown runs, the island MUST show the seconds remaining, computed from a deadline in the view (not from one event per second), with the existing acting pose and the existing message-fit layout. It MUST work with reduced motion and MUST stay click-through (no buttons).

#### Scenario: Seconds go down
- GIVEN the view carries a countdown that ends at time T
- WHEN time passes
- THEN the island shows the whole seconds left until T, and the pill does not resize while the number changes

#### Scenario: Reduced motion
- GIVEN the system asks for reduced motion
- WHEN the countdown runs
- THEN the number still counts down (it is text, not motion)

## Assumptions to confirm
- Auto-hide idle period: 3 s settle after the speech, stretched to the reading time of long replies (design "Reading time").
- Behavior on multi-monitor setups is unspecified; v1 uses the primary display.
