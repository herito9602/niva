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
The average time from end of user speech to start of reply (audio or visible response) MUST be under 2.6 seconds on a PC with a GPU, measured over the action-path commands of the eval set (commands answered with a template reply, such as open app, volume, reminders). Search-path commands (web search plus summary) are measured and reported separately and are outside this average.

#### Scenario: Latency measurement
- GIVEN the eval set run on a GPU PC with the assistant warmed up
- WHEN end-of-speech to reply-start is measured per command
- THEN the mean is below 2.6 s

### Requirement: Response latency tail
The 95th percentile of the time from end of user speech to start of reply MUST be at most 3.0 seconds, measured over the same action-path commands as the mean requirement, with real microphone audio (task 3.5). The current estimate from measured components is about 2.71 s at p95 (VAD 400 ms unmeasured + STT 203 ms + LLM 1110 ms + tool 100 ms + TTS 900 ms).

#### Scenario: Tail measurement
- GIVEN the action-path commands of the eval set spoken through a real microphone on a GPU PC with the assistant warmed up
- WHEN end-of-speech to reply-start is measured per command
- THEN the 95th percentile is at most 3.0 s

### Requirement: Unintelligible or empty speech
The system MUST NOT run any action when transcription is empty.

#### Scenario: Empty transcript
- GIVEN the user activated Niva but said nothing intelligible
- WHEN transcription returns empty text
- THEN no tool runs and the mascot shows error/confused

## Assumptions to confirm
- Latency metric: end of speech to first audio (`speak_started`); spike baseline 3.75 s (design Latency budget and Metric logging).
- PCs without a GPU are out of scope (answer to question 2); v1 targets GPU PCs (design D1), no latency target applies elsewhere.
- Percentiles: the < 2.6 s target (raised from 2.5 s on 2026-11-05 after the VAD measurement, owner decision) is a mean over action-path cases and the end-to-end p95 goal is ≤ 3.0 s (owner decision, round 3). Per-stage gates: STT p95 ≤ 400 ms (S1) and LLM tool-choice mean ≤ 800 ms on the action path (E1). The earlier "LLM p95 ≤ 700 ms" gate is retired (design, Slice 0 results).
