# Fullscreen Policy Specification

## Purpose

Rules for when Niva may appear over a fullscreen app.

## Requirements

### Requirement: Explicit activation always shows
An explicit wake word or shortcut MUST show the island even over a fullscreen app.

#### Scenario: Shortcut over game
- GIVEN a fullscreen app is active
- WHEN the user presses Ctrl+Shift+Space
- THEN the island is shown and Niva listens

### Requirement: No unsolicited interruptions
While a fullscreen app is active, Niva MUST NOT show the island on its own except for reminders.

#### Scenario: Non-reminder event
- GIVEN a fullscreen app is active
- WHEN a non-reminder background event occurs
- THEN the island stays hidden

### Requirement: Reminder over fullscreen
A due reminder MUST show the island visually (reminder-alert) and play a soft chime, and MUST NOT speak text.

#### Scenario: Reminder during video
- GIVEN a fullscreen video is active
- WHEN a reminder falls due
- THEN the island appears with reminder-alert, a soft chime plays, and no speech is produced

#### Scenario: Island cannot draw over exclusive fullscreen
- GIVEN an exclusive-mode fullscreen app over which the island cannot be drawn [verify, slice 4 spike]
- WHEN a reminder falls due or the user activates Niva by shortcut or wake word
- THEN the achievable behavior applies: a soft chime for the reminder, and an audio cue (chime) when listening starts, with no visual island

### Requirement: Fullscreen exit
After the fullscreen app ends, normal behavior MUST resume.

#### Scenario: Back to normal
- GIVEN fullscreen has ended
- WHEN a reminder falls due
- THEN the standard (spoken) reminder behavior applies

## Assumptions to confirm
- How fullscreen is detected (exclusive vs borderless) is design's; criteria assume both count.
- Replies to explicit activation over fullscreen (spoken vs visual only) are unspecified; assumed normal spoken reply.
- Only reminders count as "important events" in v1.
