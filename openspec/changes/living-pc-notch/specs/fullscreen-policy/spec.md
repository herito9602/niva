# Fullscreen Policy Specification (delta against niva-mvp)

Baseline: `openspec/changes/niva-mvp/specs/fullscreen-policy/spec.md`. Unchanged: "Explicit activation always shows", "No unsolicited interruptions", "Fullscreen exit" (read "island" as "notch").

## MODIFIED Requirements

### Requirement: Reminder over fullscreen
A due reminder MUST show the notch with the reminder-alert look (coral outward pulse) and play a soft chime, and MUST NOT speak and MUST NOT show any text. The reminder's content is not presented during fullscreen.
(Previously: the island showed reminder-alert; no text was spoken.)

#### Scenario: Reminder during video
- GIVEN a fullscreen video is active
- WHEN a reminder falls due
- THEN the notch appears with the coral pulse, a soft chime plays, no speech is produced and no text is shown

#### Scenario: Notch cannot draw over exclusive fullscreen
- GIVEN an exclusive-mode fullscreen app over which the notch cannot be drawn [verify, detection spike in design L16]
- WHEN a reminder falls due or the user activates Niva by shortcut or wake word
- THEN the achievable behavior applies: a soft chime for the reminder, and the same host chime (`audio_cue`) when listening starts, with no visual notch

## Notes
- The reminder's content is not presented anywhere during fullscreen: no text, no tray item. Speaking a missed reminder after fullscreen ends is deferred (BACKLOG).
