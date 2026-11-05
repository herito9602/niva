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

### Requirement: Free source, the user's own key
The search source (Tavily, spike S3) MUST NOT require payment: it works on the provider's free plan with a key the user creates for themselves. The key MUST belong to the user (an environment variable `NIVA_TAVILY_KEY`), MUST NOT be shipped inside the app (the provider's terms forbid sharing a key), and MUST NOT appear in any log, error, reply, URL or test. Without a key, Niva MUST say so and nothing else may break.

#### Scenario: No payment needed
- GIVEN the user's own free-plan key is configured
- WHEN a search is requested
- THEN the search succeeds and nothing was paid

#### Scenario: No key
- GIVEN no key is configured
- WHEN a search is requested
- THEN no request leaves the machine, Niva says "Web search needs a key. See the setup note." (in the request language), the mascot shows error, and every other command still works

#### Scenario: Key rejected or quota used up
- GIVEN the source answers 401 or 403, or a quota or rate-limit status (429, 432, 433)
- WHEN a search is requested
- THEN Niva says the key was rejected, or that the quota is used up, and does not say "offline"

### Requirement: What leaves the machine
A search MUST send only the search query (and the user's key as the credential) to the one fixed HTTPS endpoint, over a connection that follows no redirects. No other user data MAY be sent. The query MUST travel in the request body, never in a URL. Fetched text MUST be treated as untrusted data: it only reaches the no-tools summary call, quoted, and never a tool.

#### Scenario: Only the query
- GIVEN a spoken question
- WHEN the search runs
- THEN the request body holds the query (cut to 400 characters) and fixed options, and nothing else from the user

### Requirement: Summary knows today's date
The no-tools summary call MUST be given today's local date from the system clock, and MUST be told to answer in the request language, as plain text, and to ignore or mark results dated in the past when the question is about the future. The date MUST NOT come from the results.

#### Scenario: Next eclipse
- GIVEN the question "when is the next solar eclipse?" and results that mention eclipses already past
- WHEN the summary is requested
- THEN the request carries today's date from the clock and the instruction about past dates

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
- Search source: Tavily, picked by spike S3 and approved by the owner (design D5); keyless sources failed. Setting the key up for friends belongs to the installer work (`docs/BACKLOG.md`).
