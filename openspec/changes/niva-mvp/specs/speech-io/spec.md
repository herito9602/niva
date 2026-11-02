# Speech I/O Specification

## Purpose

Turning user speech into text and Niva replies into streamed speech, fast enough to feel conversational.

## Requirements

### Requirement: Speech-to-text
The system MUST transcribe the captured Spanish or English utterance into text.

#### Scenario: Transcribe command
- GIVEN a captured utterance in Spanish or English
- WHEN transcription runs
- THEN the text of the utterance is passed to the assistant core

### Requirement: Streaming speech output
In v1, "streaming" means TTS chunk streaming of a complete reply: the system MUST begin audio playback as soon as the first synthesized chunk of an already complete reply is ready, without waiting for the rest of the audio. LLM-to-TTS sentence streaming (speaking while the LLM is still generating) is deferred. The system MUST use one distinct voice per supported language.

#### Scenario: Streamed start
- GIVEN the assistant has a complete reply text
- WHEN the first synthesized audio chunk is ready
- THEN audio playback starts without waiting for the rest of the audio

#### Scenario: Voice per language
- GIVEN one Spanish reply and one English reply
- WHEN each is spoken
- THEN the Spanish reply uses the Spanish voice and the English reply the English voice

### Requirement: Response latency
The average time from end of user speech to start of reply (audio or visible response) MUST be under 2 seconds on a PC with a GPU, measured over the action-path commands of the eval set (commands answered with a template reply, such as open app, volume, reminders). Search-path commands (web search plus summary) are measured and reported separately and are outside this average.

#### Scenario: Latency measurement
- GIVEN the eval set run on a GPU PC with the assistant warmed up
- WHEN end-of-speech to reply-start is measured per command
- THEN the mean is below 2.0 s

### Requirement: Unintelligible or empty speech
The system MUST NOT run any action when transcription is empty.

#### Scenario: Empty transcript
- GIVEN the user activated Niva but said nothing intelligible
- WHEN transcription returns empty text
- THEN no tool runs and the mascot shows error/confused

## Assumptions to confirm
- "Start of reply" for latency means first audio or first visible text; proposal's spike figure (3.75 s) was the baseline. Chosen metric is a design/test-plan item.
- PCs without a GPU are out of scope (answer to question 2); no latency target applies there.
- Percentile (p95) latency target is not stated; only the average.
