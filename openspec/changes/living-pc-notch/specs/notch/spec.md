# Notch Specification

## Purpose

A small black notch at the top center of the screen whose mirrored spike waveform moves with the sound and changes color by assistant state. It replaces the `mascot` capability (niva-mvp). Niva answers by voice only: the notch shows no reply text and no buttons.

## ADDED Requirements

### Requirement: Notch geometry and waveform
The notch MUST be a black shape of 118x30 px, flush with the top edge of the screen, with rounded bottom corners of about 14 px. Inside it MUST draw one solid shape, mirrored around the horizontal center line, spanning 7% to 93% of the notch width with flat tails to about 2.8% and 97.2% on the center line, whose top edge is a polyline of 30 points with sharp (mitered) corners. The notch MUST be built from drawn shapes only (no image assets).

#### Scenario: Notch dimensions
- GIVEN the notch is shown in any state
- WHEN its rendered size is measured
- THEN it is 118x30 px with one waveform canvas, and its top edge is at y = 0 of the window

#### Scenario: Silence
- GIVEN the level is 0
- WHEN the waveform is drawn
- THEN it is an almost flat thin line on the center line (half-height about 0.6 of 30 drawing units)

#### Scenario: Asset audit
- GIVEN the built app
- WHEN its bundled resources are inspected
- THEN no raster or vector character images are present for the notch

### Requirement: Idle is hidden
In the idle phase the notch MUST NOT be visible. It MUST appear on activation and MUST hide again when the interaction ends.

#### Scenario: Idle
- GIVEN the assistant is idle
- WHEN the screen is observed
- THEN no notch is visible

#### Scenario: Summoned
- GIVEN the notch is hidden
- WHEN the user activates Niva by shortcut or wake word
- THEN the notch drops in from the top center with a spring motion and shows the listening look

### Requirement: Color per state
The notch MUST show exactly one color per state, and the color alone tells the states apart (owner decision 2026-11-10: the per-state motion/silhouette non-color cue is removed; the voice and the hidden status text remain the non-color channels):

| State | Color |
|-------|-------|
| listening | #378ADD |
| thinking | #7F77DD |
| speaking | #5DCAA5 |
| confirmation-needed | #EF9F27 |
| success | #97C459 |
| error | #E24B4A |
| reminder-alert | #F0997B |

The `acting` phase MUST use the thinking color. The `countdown` phase MUST use the confirmation-needed color. While text-to-speech audio plays, the speaking look MUST override the look of the current phase. The shape fill MUST be the state color, opaque. A visually hidden `role="status"` region MUST announce the state name.

#### Scenario: Each state has its color
- GIVEN a debug build with the state switcher
- WHEN a tester selects each state in the table
- THEN the waveform is filled with the color listed for it

#### Scenario: Acting and countdown reuse colors
- GIVEN the phase is acting, or a sleep countdown is running
- WHEN the notch is observed
- THEN it shows the thinking color for acting and the amber color for the countdown

#### Scenario: Speaking overrides the phase look
- GIVEN the phase is success and TTS is playing
- WHEN the notch is observed
- THEN it shows the speaking color and a waveform driven by the voice level
- AND when the speech ends it shows the look of the phase

#### Scenario: Status announcement
- GIVEN a screen reader is running
- WHEN the state changes
- THEN the hidden status region announces the state name and never reply text

### Requirement: Waveform motion and reduced motion
Every ~70 ms the waveform MUST pick new random targets for its points, shaped by a bell-curve envelope centered on the notch (`env(u) = exp(-((u-0.5)/0.22)^2)`), and every frame each point MUST move 45% of the way to its target. The spike height MUST scale with the smoothed level (`amp = 1 + level*24`). States without audio (every state except speaking, listening included) MUST use level 0 plus a tiny breathing level (about 0.05), so the line stays near flat but alive. Under the operating system's reduced-motion setting there MUST be no random re-targeting: the notch MUST draw a static shape scaled by the smoothed level (a static flat line at level 0), while state changes still transition with a short fade.

#### Scenario: Breathing
- GIVEN the state is thinking
- WHEN the notch is observed
- THEN the waveform is a near-flat line with tiny spikes in the thinking color

#### Scenario: Reduced motion
- GIVEN the system asks for reduced motion
- WHEN a state without audio is shown
- THEN a static flat line is drawn in the state color and no loop keeps running

#### Scenario: Reduced motion with voice
- GIVEN the system asks for reduced motion and the notch is speaking
- WHEN the level changes
- THEN the static shape (a flat line at level 0) scales with the level and does not re-target randomly

### Requirement: Level-driven waveform
Only while speaking MUST the spike height follow a level, the TTS voice level (the wave moves with Niva's voice, not with the user: owner decision 2026-11-10). While listening the waveform MUST NOT be level-driven: it shows the same calm breathing line as thinking, in the listening color, with no microphone level and no simulated voice. The level comes from the sidecar `level` message (see `speech-io`) and MUST reach the notch without passing through the state reducer. The shape amplitude MUST rise with the audible sound with a mean lag of 50 ms or less and no sample above 100 ms (the decay is intentionally slower and is not bounded), for the TTS level, on built-in or wired output (Bluetooth is excluded because its latency is not reported).

#### Scenario: Listening is calm
- GIVEN the notch is listening
- WHEN the user speaks, or any `level` message arrives
- THEN the waveform keeps the calm breathing line in the listening color and does not follow any level or simulated voice

#### Scenario: Speaking follows the voice
- GIVEN Niva is speaking and `level` messages arrive
- WHEN the voice gets louder
- THEN the spikes grow in step with the audible voice (onset mean lag 50 ms or less and no sample above 100 ms, the decay is intentionally slower; on built-in or wired output)
- AND a fake-clock unit test of the level smoothing checks this lag in addition to the manual 240 fps measurement

#### Scenario: Levels do not drive other states
- GIVEN the phase is listening, thinking or confirmation-needed
- WHEN `level` messages arrive
- THEN the waveform keeps its breathing line

### Requirement: Simulation fallback
If the notch is speaking and receives no `level` message for more than 150 ms, it MUST drive the waveform level from a UI-only simulated voice envelope, and MUST return to real levels as soon as `level` messages arrive again. Under reduced motion the simulation MUST NOT animate.

#### Scenario: Levels stop
- GIVEN the notch is speaking with real levels
- WHEN 150 ms pass without a `level` message
- THEN the waveform continues to move from the simulation

#### Scenario: Levels resume
- GIVEN the waveform runs on the simulation
- WHEN a `level` message arrives
- THEN the waveform follows the real level again

### Requirement: Success and error auto-hide
After the success or the error look, the notch MUST settle and hide on its own, with no reading hold. The hide MUST NOT start before the speech of the reply has ended, and MUST wait for the settle time (1.5 s, down from 3 s) after it. A new activation MUST cancel a pending hide.

#### Scenario: Success hides
- GIVEN an action finished and Niva has spoken the reply
- WHEN the settle time (1.5 s, down from 3 s) elapses
- THEN the notch shows the success color and then hides

#### Scenario: Error hides
- GIVEN a command failed
- WHEN the error look has been shown and the settle time (1.5 s) elapses
- THEN the notch hides

#### Scenario: Activation during settle
- GIVEN the notch is settling after a reply
- WHEN the user presses Ctrl+Shift+Space
- THEN the pending hide is cancelled and the notch shows listening

### Requirement: No reply text and no buttons
The notch MUST NOT show reply text, labels, countdown numbers, confirmation prompts or buttons. Niva MUST answer by voice only. The view the host sends to the notch MUST NOT carry the reply text.

#### Scenario: A reply
- GIVEN Niva replies "Opening Notepad"
- WHEN the notch is observed
- THEN it shows only the waveform and its color, and the reply is heard

#### Scenario: A confirmation
- GIVEN a confirmation is pending
- WHEN the notch is observed
- THEN it shows the amber waveform and no prompt text and no Cancel or Confirm button

### Requirement: Always click-through
The notch window MUST ignore the mouse everywhere, in every state, and MUST NOT take focus.

#### Scenario: Click on the notch
- GIVEN the notch is shown over another app
- WHEN the user clicks where the notch is drawn
- THEN the click reaches the app below

### Requirement: Voice down is signalled
When voice is unavailable (microphone error or sidecar down), the notch MUST show the error look and the host MUST play a chime. No text MUST be shown.

#### Scenario: Sidecar down
- GIVEN the sidecar is not running
- WHEN the user activates Niva by shortcut or typed command
- THEN the notch shows the error look, the host plays a chime (`audio_cue`, no sidecar involved), and the notch hides after the settle time (1.5 s)

### Requirement: Reminder look
When a reminder is alerting, the notch MUST show the reminder-alert look (#F0997B waveform) and no text. Under reduced motion it MUST show the static line.

#### Scenario: Reminder alert
- GIVEN a reminder falls due
- WHEN the alert starts
- THEN the notch shows the coral waveform and no text

## Notes
- Design fixes the waveform rendering (L1, L2, "Notch looks") and confirms the level bound (L14). Visual reworked on 2026-11-10 by owner decision (supersedes the 7-bar version).
