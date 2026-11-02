# Web Search Specification

## Purpose

Answer user questions from web results, spoken as a short summary.

## Requirements

### Requirement: Spoken summary
The system MUST fetch search results as text and speak a short summary in the request language. Merely opening a browser MUST NOT satisfy this requirement.

#### Scenario: Question answered
- GIVEN internet access and the question "who won the last Champions League?"
- WHEN the search tool runs
- THEN Niva speaks a short summary derived from fetched results

### Requirement: Free source
The search source MUST NOT require a paid service.

#### Scenario: No paid dependency
- GIVEN a fresh install with no API payment configured
- WHEN a search is requested
- THEN the search succeeds

### Requirement: Safe action
Web search MUST execute without confirmation.

#### Scenario: No prompt
- GIVEN a search request
- WHEN the tool is called
- THEN no confirmation is requested

### Requirement: Offline or empty results
The system MUST tell the user when the search failed or returned nothing, and MUST NOT invent an answer.

#### Scenario: Offline
- GIVEN no internet connection
- WHEN the user asks a search question
- THEN Niva says it cannot search now and the mascot shows error/confused

### Requirement: Never mixed with a risky call
A response that contains `web_search` together with a risky call MUST be rejected at validation, with an error template reply and nothing executed.

#### Scenario: Search plus close
- GIVEN one model response with `web_search` and `close_app`
- WHEN the response is validated
- THEN no tool runs, no confirmation is requested, and the error template is spoken

## Assumptions to confirm
- "Short" is answered by design D4/Safety Gate: the summary is capped (e.g. 400 characters); the exact cap is tuned in tests.
- Source citation by voice is not required.
- Search source: picked by spike S3 (design D5); the spec example may change if no free source answers it.
