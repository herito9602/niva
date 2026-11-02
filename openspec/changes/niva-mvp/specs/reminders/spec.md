# Reminders Specification

## Purpose

Create, list and fire local reminders.

## Requirements

### Requirement: Create reminder
The system MUST create a reminder from a spoken request containing a message and a time, without confirmation.

#### Scenario: Create
- GIVEN the request "remind me in 10 minutes to call Ana"
- WHEN the tool runs
- THEN a reminder is stored for 10 minutes from now and Niva confirms the time

#### Scenario: Missing time
- GIVEN "remind me to call Ana" with no time
- WHEN the tool runs
- THEN no reminder is created and Niva asks for the time
- AND the next utterance (one follow-up turn) completes the pending reminder

### Requirement: List reminders
The system MUST list pending reminders on request.

#### Scenario: List
- GIVEN two pending reminders
- WHEN the user asks "what reminders do I have?"
- THEN Niva reports both with their times

### Requirement: Fire reminder
At the due time the system MUST alert the user: mascot in reminder-alert state and island shown.

#### Scenario: Fires at due time
- GIVEN a reminder due now and no fullscreen app
- WHEN the due time arrives
- THEN the island shows reminder-alert, plays a soft chime, and the reminder text is shown and spoken

### Requirement: Persistence
Pending reminders MUST survive an app restart.

#### Scenario: Restart
- GIVEN a pending reminder due in 1 hour
- WHEN Niva restarts
- THEN the reminder still fires at its due time

## Assumptions to confirm
- Spoken reading of the reminder when no fullscreen app is active is assumed from the "no spoken text over fullscreen" answer; confirm.
- Reminders missed while Niva was closed: behavior unspecified (assumed fire on next start; confirm).
- Cancel/edit reminders is not in the proposal and is out of scope.
- Recurring reminders are out of scope.
