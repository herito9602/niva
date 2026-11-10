# Notch Specification

## Purpose

A small black notch at the top center of the screen whose equalizer bars move with the sound and change color by assistant state. It replaces the `mascot` capability (niva-mvp). Niva answers by voice only: the notch shows no reply text and no buttons.

## ADDED Requirements

### Requirement: Notch geometry and bars
The notch MUST be a black shape of 118x30 px, flush with the top edge of the screen, with rounded bottom corners of about 14 px and 7 rounded vertical bars. The notch MUST be built from drawn shapes only (no image assets).

#### Scenario: Notch dimensions
- GIVEN the notch is shown in any state
- WHEN its rendered size is measured
- THEN it is 118x30 px with 7 bars, and its top edge is at y = 0 of the window

#### Scenario: Asset audit
- GIVEN the built app
- WHEN its bundled resources are inspected
- THEN no raster or vector character images are present for the notch
- AND the distinct-silhouette test in `looks.ts` passes (see "Non-color cue and reduced motion")

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

### Requirement: Palette and motion per state
The notch MUST show exactly one look per state, with a color and a motion that tells the state apart without color:

| State | Color | Motion (non-color cue) |
|-------|-------|------------------------|
| listening | #378ADD | bars follow the microphone level |
| thinking | #7F77DD | a wave travels across the bars |
| speaking | #5DCAA5 | bars follow the voice level |
| confirmation-needed | #EF9F27 | slow pulse |
| success | #97C459 | bounce, then hide |
| error | #E24B4A | low jitter, then hide |
| reminder-alert | #F0997B | outward pulse |

The `acting` phase MUST use the thinking look. The `countdown` phase MUST use the confirmation-needed look (amber pulse). While text-to-speech audio plays, the speaking look MUST override the look of the current phase.

#### Scenario: Each state has its look
- GIVEN a debug build with the state switcher
- WHEN a tester selects each state in the table
- THEN the notch shows the color and motion listed for it

#### Scenario: Acting and countdown reuse looks
- GIVEN the phase is acting, or a sleep countdown is running
- WHEN the notch is observed
- THEN it shows the thinking wave for acting and the amber pulse for the countdown

#### Scenario: Speaking overrides the phase look
- GIVEN the phase is success and TTS is playing
- WHEN the notch is observed
- THEN it shows the speaking look and bars driven by the voice level
- AND when the speech ends it shows the look of the phase

### Requirement: Non-color cue and reduced motion
Each state MUST be distinguishable without color. Under the operating system's reduced-motion setting, looping motion (wave, pulse, bounce, jitter, simulated bars) MUST become a static per-state bar silhouette that is different for every state, while state changes still transition with a short fade.

#### Scenario: Grayscale
- GIVEN the notch rendered in grayscale
- WHEN each of the seven states is shown
- THEN every state can be told apart from the others by bar motion or silhouette
- AND a vitest in `looks.ts` asserts that all seven reduced-motion silhouettes are pairwise distinct

#### Scenario: Reduced motion
- GIVEN the system asks for reduced motion
- WHEN each state is shown
- THEN no looping motion plays, each state shows its own static silhouette, and the colors still apply

### Requirement: Level-driven bars
While listening, the bar heights MUST follow the microphone level. While speaking, they MUST follow the TTS voice level. The levels come from the sidecar `level` message (see `speech-io`) and MUST reach the notch without passing through the state reducer. Bars MUST track the audible sound with a mean lag of 50 ms or less and no sample above 100 ms, for both microphone and TTS, on built-in or wired output (Bluetooth is excluded because its latency is not reported).

#### Scenario: Listening follows the microphone
- GIVEN the notch is listening and `level` messages arrive
- WHEN the user speaks louder
- THEN the bars rise accordingly with a mean lag of 50 ms or less and no sample above 100 ms
- AND a fake-clock unit test of the bar smoothing checks this lag in addition to the manual 240 fps measurement

#### Scenario: Speaking follows the voice
- GIVEN Niva is speaking and `level` messages arrive
- WHEN the voice gets louder
- THEN the bars rise in step with the audible voice (same lag bound, on built-in or wired output)

#### Scenario: Levels do not drive other states
- GIVEN the phase is thinking or confirmation-needed
- WHEN `level` messages arrive
- THEN the bars keep the motion of that state

### Requirement: Simulation fallback
If the notch is in listening or speaking and receives no `level` message for more than 150 ms, it MUST drive the bars from a UI-only simulation, and MUST return to real levels as soon as `level` messages arrive again. Under reduced motion the simulation MUST NOT animate.

#### Scenario: Levels stop
- GIVEN the notch is listening with real levels
- WHEN 150 ms pass without a `level` message
- THEN the bars continue to move from the simulation

#### Scenario: Levels resume
- GIVEN the bars run on the simulation
- WHEN a `level` message arrives
- THEN the bars follow the real level again

### Requirement: Success and error auto-hide
After the success or the error look, the notch MUST settle and hide on its own, with no reading hold. The hide MUST NOT start before the speech of the reply has ended, and MUST wait for the settle time (1.5 s, down from 3 s) after it. A new activation MUST cancel a pending hide.

#### Scenario: Success hides
- GIVEN an action finished and Niva has spoken the reply
- WHEN the settle time (1.5 s, down from 3 s) elapses
- THEN the notch shows the success bounce and then hides

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
- THEN it shows only bars and color, and the reply is heard

#### Scenario: A confirmation
- GIVEN a confirmation is pending
- WHEN the notch is observed
- THEN it shows the amber pulse and no prompt text and no Cancel or Confirm button

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
When a reminder is alerting, the notch MUST show the reminder-alert look (#F0997B outward pulse) and no text. Under reduced motion it MUST show the static reminder silhouette.

#### Scenario: Reminder alert
- GIVEN a reminder falls due
- WHEN the alert starts
- THEN the notch shows the coral outward pulse and no text

## Notes
- Design fixes the static silhouettes ("Notch looks") and confirms the level bound (L14).
