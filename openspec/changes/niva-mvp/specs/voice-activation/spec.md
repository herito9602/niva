# Voice Activation Specification

## Purpose

How a user starts an interaction: local wake word "Hey Niva" or the shortcut.

## Requirements

### Requirement: Local wake word
The system MUST detect "Hey Niva" locally, without sending audio off the device before activation.

#### Scenario: Wake word heard
- GIVEN Niva is running and idle
- WHEN the user says "Hey Niva"
- THEN the island is shown and Niva starts listening

### Requirement: Shortcut activation
The system MUST start listening on the shortcut, equivalent to the wake word.

#### Scenario: Shortcut works without wake word
- GIVEN the wake word detector is not firing
- WHEN the user presses Ctrl+Shift+Space
- THEN Niva starts listening

### Requirement: False activation limits
The system MUST produce 0 false activations in 10 minutes of idle (silence) and SHOULD produce at most 1 in 30 minutes of mixed music, video and conversation.

#### Scenario: Idle soak
- GIVEN a quiet room and Niva idle for 10 minutes
- WHEN the test ends
- THEN the false-activation count is 0

#### Scenario: Noisy soak
- GIVEN music, video and talk playing for 30 minutes without "Hey Niva"
- WHEN the test ends
- THEN the false-activation count is at most 1

### Requirement: End of speech
The system MUST use voice activity detection to decide the user finished speaking.

#### Scenario: Silence ends capture
- GIVEN Niva is listening and the user spoke a command
- WHEN the user stops speaking
- THEN capture ends and the command is processed

## Assumptions to confirm
- Slice 3 uses a stock wake word model; the custom "Hey Niva" model lands in slice 5. The criteria above apply to the custom model.
- Behavior when no speech follows activation (listening timeout) is unspecified; flagged for design.
