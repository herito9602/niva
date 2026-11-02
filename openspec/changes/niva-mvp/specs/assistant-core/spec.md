# Assistant Core Specification

## Purpose

Turns a text request into tool calls and a reply, behind a swappable LLM provider.

## Requirements

### Requirement: Swappable LLM provider
The core MUST depend on an `LlmProvider` port and MUST ship with a local Ollama adapter. Replacing the adapter MUST NOT require changes to tool, safety or UI logic.

#### Scenario: Provider swap
- GIVEN a test double implementing the port
- WHEN the core runs a request with the double instead of the Ollama adapter
- THEN the same tool loop and safety behavior apply

### Requirement: Tool-calling loop
The core MUST let the model call tools, feed back each tool's real result, and produce a final reply. Tool results MUST reflect what actually happened (no "dry run" placeholders).

#### Scenario: Open app
- GIVEN the request "open Notepad"
- WHEN the model calls the open-app tool
- THEN the tool executes, returns a real success or failure result, and the reply reflects it

#### Scenario: Loop ends
- GIVEN a tool returned its result
- WHEN the model has nothing further to call
- THEN the loop ends and the final reply is produced

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

## Assumptions to confirm
- Product LLM model is an open decision; criteria apply to whichever is chosen (qwen3.5:2b is known to fail them).
- Conversation memory across requests is not in scope; each request is independent (assumed).
- Maximum tool-loop iterations is a design item.
