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
The 95th percentile of the time from end of user speech to start of reply MUST be at most 3.0 seconds, measured over the same action-path commands as the mean requirement, with real microphone audio (task 3.5). The current estimate from measured components is about 2.85 s at p95 (VAD 532 ms computed, see spikes/vad/RESULTS.md + STT 203 ms + LLM 1110 ms + tool 100 ms + TTS 900 ms).

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

### Requirement: Continuous microphone capture
The sidecar MUST keep one microphone stream open from `ready` until it stops or the stream fails. It MUST NOT close or reopen the stream between the wake word and the command, or between turns. While the stream is open, audio read during a turn or during speech output MAY be ignored, but MUST NOT cause a capture gap when listening starts.

#### Scenario: Command right after the wake word
- GIVEN the sidecar is ready
- WHEN the user says "Hey Niva, open Spotify" in one breath
- THEN the transcript contains the whole command, with no words lost between the wake word and the command

#### Scenario: Stream failure
- GIVEN the microphone stream is open
- WHEN the stream fails
- THEN the sidecar reports a `MIC_*` error and does not stay silently deaf

### Requirement: Audio before speech onset
The sidecar MUST keep a short rolling buffer of recent audio in memory and include it in the utterance sent to STT, so the first syllable is not cut by the detection delay. The buffer MUST stay in memory only and MUST be overwritten as it rolls.

#### Scenario: Soft first syllable
- GIVEN a listen is open
- WHEN the user starts speaking before the VAD confirms speech
- THEN the audio sent to STT starts before the confirmed onset

### Requirement: Discard reasons
Every wake detection or utterance that does not become a request MUST record a reason code in a local log, so a silent failure can be told apart from the user saying nothing. The log MUST NOT contain audio or transcript text.

#### Scenario: Ignored wake word
- GIVEN Niva is speaking
- WHEN the wake word is detected and ignored
- THEN the log records the reason code and no audio or text

#### Scenario: Empty utterance
- GIVEN a listen is open
- WHEN transcription returns empty text
- THEN the log records the reason code and no audio or text

### Requirement: Truthful speech start
The sidecar MUST send `speak_started` only after the first non-silent audio chunk has been written to the output device, not when synthesis is requested. If no audio reaches the device, it MUST report `TTS_FAILED` for that speak id.

#### Scenario: Synthesis works but the output device fails
- GIVEN a reply is synthesized
- WHEN writing to the output device fails
- THEN `speak_started` is not sent and `TTS_FAILED` is reported for that speak id

## Assumptions to confirm
- Latency metric: end of speech to first audio (`speak_started`); spike baseline 3.75 s (design Latency budget and Metric logging).
- PCs without a GPU are out of scope (answer to question 2); v1 targets GPU PCs (design D1), no latency target applies elsewhere.
- Percentiles: the < 2.6 s target (raised from 2.5 s on 2026-11-05 after the VAD measurement, owner decision) is a mean over action-path cases and the end-to-end p95 goal is ≤ 3.0 s (owner decision, round 3). Per-stage gates: STT p95 ≤ 400 ms (S1) and LLM tool-choice mean ≤ 800 ms on the action path (E1). The earlier "LLM p95 ≤ 700 ms" gate is retired (design, Slice 0 results).
