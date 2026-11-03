# Assistant Core Specification

## Purpose

Turns a text request into tool calls and a reply, behind a swappable LLM provider.

## Requirements

### Requirement: Swappable LLM provider
The core MUST depend on an `LlmProvider` port and MUST ship with a local Ollama adapter. Replacing the adapter MUST NOT require changes to tool, safety or UI logic.

#### Scenario: Provider swap
- GIVEN a test double implementing the port
- WHEN the core runs a request with the double instead of the Ollama adapter
- THEN the same request pipeline and safety behavior apply

### Requirement: Single-response tool calling
The core MUST take one LLM response per request, validate its tool calls before executing any of them, run the validated tools, and produce a localized template reply from the real tool results. Tool results are not fed back to the LLM; the only second LLM call is the no-tools `web_search` summary. Tool results MUST reflect what actually happened (no "dry run" placeholders), and the reply MUST report them truthfully. If the single response is incomplete for the request, the reply covers only what ran.

#### Scenario: Open app
- GIVEN the request "open Notepad"
- WHEN the model calls the open-app tool
- THEN the tool executes, returns a real success or failure result, and the template reply reflects it

#### Scenario: Validation before execution
- GIVEN a model response with an invalid argument or an unresolvable target
- WHEN the core validates the response
- THEN nothing is executed and an error template reply is produced

#### Scenario: Results are not fed back
- GIVEN the model response contained safe tool calls that all finished
- WHEN the core produces the reply
- THEN the reply is a localized template and no further LLM call is made (except the no-tools `web_search` summary)

#### Scenario: web_search mixed with a risky call
- GIVEN one model response containing `web_search` and a risky call (`close_app` or `power_action`)
- WHEN the core validates the response
- THEN the whole response is rejected, nothing is executed (including its safe calls), and an error template reply is produced

### Requirement: Tool selection accuracy
The core MUST select the correct tool for at least 90% of eval commands (Spanish and English).

#### Scenario: Eval run
- GIVEN the eval set with expected tools
- WHEN every command is run
- THEN at least 90% call the expected tool

### Requirement: No unrequested actions
The core MUST NOT execute actions the user did not request.

#### Scenario: Chat only
- GIVEN the request "how are you?"
- WHEN the model replies
- THEN no tool is executed

### Requirement: Warm-up
The system MUST warm up the model at startup so the first command is not slower than later ones beyond the latency target.

#### Scenario: First command
- GIVEN Niva just started and warm-up completed
- WHEN the first command is issued
- THEN its latency meets the speech-io latency requirement

### Requirement: Provider unavailable
The core MUST signal an error state to the user when the provider is unreachable.

#### Scenario: LLM down
- GIVEN the Ollama service is not running
- WHEN a request is made
- THEN no action runs and the mascot shows error/confused

### Requirement: Replies are short (owner decision 2026-11-03)
Every spoken or shown reply MUST be at most about 400 characters, the same bound as the web-search summary. For LLM free-text replies the core MUST enforce it twice: the system prompt asks for short replies, and a hard cap cuts anything longer (at a word boundary, with an ellipsis) before it is spoken or shown. Templates and search summaries already respect the bound.

#### Scenario: Chatty model
- GIVEN the model answers a chat request with 1500 characters
- WHEN the reply is spoken and shown
- THEN it is cut to at most about 400 characters, so the island can show and scroll all of it within its reading time

## Assumptions to confirm
- Product LLM model is an open decision; criteria apply to whichever is chosen (qwen3.5:2b is known to fail them).
- Conversation memory across requests is not in scope; each request is independent (assumed).
- There is no tool loop: design D4 fixes one LLM response per request, so no iteration cap applies (resolved).
