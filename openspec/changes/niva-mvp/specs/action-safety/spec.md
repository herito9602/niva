# Action Safety Specification

## Purpose

A gate enforced in application code that classifies actions as risky or safe and requires user confirmation for risky ones.

## Requirements

### Requirement: Risk classification
Closing an app and power/lock/sleep actions MUST be classified risky. Opening apps, volume/mute, web search and reminders MUST be classified safe.

#### Scenario: Safe action
- GIVEN the request "volume to 30"
- WHEN the tool is called
- THEN it executes with no confirmation

### Requirement: Confirmation for risky actions
A risky action MUST NOT execute until the user explicitly confirms. Enforcement MUST be in code, independent of the model's output or prompt. Across the eval set, 0 risky actions may run without confirmation.

#### Scenario: Confirmed
- GIVEN "close Chrome" and the mascot in confirmation-needed state
- WHEN the user says yes
- THEN Chrome is closed

#### Scenario: Denied
- GIVEN a pending confirmation
- WHEN the user says no
- THEN the action does not run

#### Scenario: Model bypass attempt
- GIVEN a model that calls the close tool as if already confirmed
- WHEN the call reaches the gate
- THEN the gate still requires user confirmation

### Requirement: Timeout cancels
If no answer arrives within the confirmation timeout, the action MUST be cancelled.

#### Scenario: No answer
- GIVEN a pending confirmation
- WHEN the timeout elapses with no answer
- THEN the action does not run and the user is told it was cancelled

### Requirement: Ambiguous answer
An answer that is not clearly affirmative MUST NOT be treated as confirmation.

#### Scenario: Unclear reply
- GIVEN a pending confirmation
- WHEN the user says something unrelated
- THEN the action is not executed

### Requirement: Substring matching for destructive targets
Destructive app matching MUST use substring matching only, never fuzzy matching.

#### Scenario: Near-miss name
- GIVEN running apps "Notepad" and "Notion"
- WHEN the user asks to close "Notpad"
- THEN no app is closed and the user is told no match was found

## Assumptions to confirm
- The timeout duration is not stated (design/user to set).
- Confirmation by shortcut/click as well as voice is not specified; spec only requires an explicit affirmative.
- If one substring matches several running apps, behavior is unspecified (flagged; safest: ask which, never close all).
