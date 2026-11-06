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

### Requirement: Voice failure never decides for the user
When the voice sidecar reports an unusable-voice error (no microphone, microphone busy, GPU out of memory, models not loaded, protocol error or internal error), the system MUST mark voice unavailable and MUST NOT return to voice use until the voice supervisor reports it ready again. A turn that was listening MUST end in the error state with a message that names what happened, in the language of the current request (the last used language if there is none). A pending confirmation MUST stay pending, answerable by click only; an error MUST NOT confirm, reject or cancel it. A sleep countdown MUST keep running and stay cancellable by input and the shortcut; an error MUST NOT start the sleep early or cancel it silently. An action in progress, a thinking turn and a reminder alert MUST NOT be interrupted. A failed speech (`TTS_FAILED`) is not one of these errors and does not make voice unavailable.

#### Scenario: Microphone busy while listening
- GIVEN Niva is listening
- WHEN the sidecar reports that the microphone is in use by another app
- THEN the listen ends, Niva shows an error that says so and offers the text window
- AND the next activation by shortcut or wake word says voice is not ready

#### Scenario: Voice fails during a confirmation
- GIVEN Niva waits for a yes or no on closing an app
- WHEN the sidecar reports an error
- THEN the question stays open with the buttons, a spoken answer is no longer accepted, and the 10 s timeout still cancels it

#### Scenario: Voice fails during the sleep countdown
- GIVEN the 10 s sleep countdown is running
- WHEN the sidecar reports an error
- THEN the countdown continues and any key, mouse movement or the shortcut still cancels it

## Assumptions to confirm
- Slice 3 uses a stock wake word model; the custom "Hey Niva" model lands in slice 5. The criteria above apply to the custom model.
- Listening timeout: 6 s with no speech, then a 15 s utterance cap after `speech_start` (design State Machine, listening row).
