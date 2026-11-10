# Speech I/O Specification (delta against niva-mvp)

Baseline: `openspec/changes/niva-mvp/specs/speech-io/spec.md`. All other requirements are unchanged.

## MODIFIED Requirements

### Requirement: Unintelligible or empty speech
The system MUST NOT run any action when transcription is empty.
(Previously: "the mascot shows error/confused".)

#### Scenario: Empty transcript
- GIVEN the user activated Niva but said nothing intelligible
- WHEN transcription returns empty text
- THEN no tool runs and the notch shows the error look and then hides

## ADDED Requirements

### Requirement: Audio level message
While the sidecar is listening or speaking, it MUST send a `level` message to the host at about 30 Hz (a tolerance of 20 to 40 Hz). The message MUST carry 7 integer bands of 0 to 100 and MUST NOT carry audio samples, transcript text or any other audio content. While listening, the bands MUST come from the microphone frames. While speaking, they MUST come from the TTS audio as it is written to the output device. The sidecar MUST NOT send `level` when it is neither listening nor speaking, and MUST stop within one period of the listen or speech ending.

#### Scenario: Level while listening
- GIVEN a listen is open
- WHEN the user speaks for one second
- THEN `level` messages arrive at 20 to 40 per second, each with 7 values in 0..100

#### Scenario: Level while speaking
- GIVEN a reply is being spoken
- WHEN audio is written to the output device
- THEN `level` messages arrive at 20 to 40 per second with the voice bands, aligned to the audible sound as `notch` requires (mean lag 50 ms or less, no sample above 100 ms, on built-in or wired output)

#### Scenario: Idle sends nothing
- GIVEN the sidecar is ready and not listening or speaking
- WHEN time passes
- THEN no `level` message is sent

#### Scenario: No audio content
- GIVEN a captured `level` message
- WHEN its fields are inspected
- THEN it holds only the 7 band values and no samples or text

### Requirement: Level messages bypass the state machine
The host MUST forward `level` messages straight to the notch window and MUST NOT pass them through the core reducer, and the host MUST ignore a malformed `level` message (wrong band count or out-of-range value) without error. A missing `level` stream MUST NOT block, delay or change any other message.

#### Scenario: Flood does not touch the reducer
- GIVEN 30 `level` messages per second
- WHEN they arrive during a listen
- THEN no reducer event is created for them

#### Scenario: Malformed level
- GIVEN a `level` message with 6 bands
- WHEN the host receives it
- THEN it is dropped and nothing else changes

### Requirement: Speaking flag in the reducer
The reducer MUST expose `audible_speak`, set when `SpeakStarted` arrives with an id equal to the current speak id, and cleared wherever the speak id clears. The view's `speaking` flag MUST be true exactly while `audible_speak` is set.

#### Scenario: audible_speak follows the audible speech
- GIVEN a speak id is current and synthesis is running
- WHEN `SpeakStarted` arrives with the matching id
- THEN `audible_speak` is set and `speaking` is true
- AND when the speak id clears, `audible_speak` is cleared

#### Scenario: Mismatched SpeakStarted
- GIVEN a speak id is current
- WHEN `SpeakStarted` arrives with a different id
- THEN `audible_speak` is not set
