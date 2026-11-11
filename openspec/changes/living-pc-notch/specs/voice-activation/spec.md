# Voice Activation Specification (delta against niva-mvp)

Baseline: `openspec/changes/niva-mvp/specs/voice-activation/spec.md`. Unchanged: everything not listed here.

## ADDED Requirements

### Requirement: Activation plays an acknowledgement cue
When Niva is activated by the wake word or the shortcut and the voice is available, the host MUST play a short acknowledgement cue before the command listen opens, so the user knows he was heard without looking at the screen. The cue is two soft rising notes (C5 then G5) lasting at most 260 ms and no louder than the reminder chime in energy. The microphone MUST open when the cue ends, and the no-speech window MUST count from that moment. Speech uttered between the activation and the opening of the listen MUST NOT be lost. A listen that follows a question Niva spoke (follow-up question, confirmation) MUST NOT play the cue.

#### Scenario: Wake word is acknowledged
- GIVEN the voice is available and Niva is idle
- WHEN the wake word is detected
- THEN the acknowledgement cue plays, and when it ends the command listen opens

#### Scenario: Shortcut is acknowledged
- GIVEN the voice is available
- WHEN the user presses Ctrl+Shift+Space to activate
- THEN the same cue plays before the command listen opens

#### Scenario: Speaking right after the wake word
- GIVEN the wake word was just detected and the cue is playing
- WHEN the user starts the command before the cue ends
- THEN the sidecar keeps that audio and the late listen replays it, so the command is transcribed whole

#### Scenario: Cue does not become a turn
- GIVEN the acknowledgement cue is playing
- WHEN the microphone captures it
- THEN no speech start is reported for it, because the listen opens after the cue ends

#### Scenario: Cancelled before the cue ends
- GIVEN the cue is playing and the listen has not opened
- WHEN the activation is cancelled (shortcut again, or a newer listen)
- THEN the pending listen never opens

#### Scenario: Follow-up question has no cue
- GIVEN Niva asked a follow-up or confirmation question aloud
- WHEN the answer listen opens after the question
- THEN no acknowledgement cue plays and the listen opens at once

#### Scenario: Voice down gives no cue
- GIVEN the voice is not available
- WHEN the user activates Niva
- THEN the voice-not-ready error and its chime apply, and no acknowledgement cue plays
