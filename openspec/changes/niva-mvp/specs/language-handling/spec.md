# Language Handling Specification

## Purpose

Niva supports Spanish and English, detecting the language of each request and replying in it.

## Requirements

### Requirement: Per-request detection
The system MUST detect the language (Spanish or English) of each request independently.

#### Scenario: Language switch
- GIVEN the previous request was in Spanish
- WHEN the next request is in English
- THEN the reply to the second request is in English

### Requirement: Reply language match
The reply language MUST match the request language in at least 95% of eval commands.

#### Scenario: Eval run
- GIVEN an eval set of Spanish and English commands
- WHEN every reply language is checked
- THEN at least 95% match their request language

### Requirement: Language independent of tool
Tool names and app names that are English (e.g. "Spotify") MUST NOT change the reply language.

#### Scenario: Spanish request, English app
- GIVEN the request "abrí Spotify"
- WHEN Niva replies
- THEN the reply is in Spanish

### Requirement: Voice follows reply language
The voice used for speaking MUST be the one for the reply language.

#### Scenario: English reply voice
- GIVEN an English request
- WHEN Niva speaks
- THEN the English voice is used

## Assumptions to confirm
- Requests in other languages are unspecified. Assumption: treated as best-effort Spanish/English; not tested.
- Mixed-language utterances (code-switching) are not covered.
