# PC Actions Specification

## Purpose

Voice-controlled app and audio actions on the Windows PC.

## Requirements

### Requirement: Open app
The system MUST open an installed app by name.

#### Scenario: Open installed app
- GIVEN "Notepad" is installed
- WHEN the user asks to open Notepad
- THEN Notepad starts and Niva confirms in the request language

#### Scenario: Unknown app
- GIVEN no installed app matches the name
- WHEN the user asks to open it
- THEN nothing starts and Niva says it could not find the app

### Requirement: Close app
The system MUST close a running app by name, subject to action-safety confirmation.

#### Scenario: Close not running
- GIVEN the named app is not running
- WHEN the user asks to close it
- THEN Niva says it is not running and asks nothing further

### Requirement: Volume and mute
The system MUST set volume to a level, raise/lower it, and mute/unmute.

#### Scenario: Set volume
- GIVEN current volume 60
- WHEN the user asks for volume 30
- THEN system volume is 30

#### Scenario: Out of range
- GIVEN the request "volume 150"
- WHEN the tool runs
- THEN volume is clamped to 100 (assumption) and Niva reports it

### Requirement: Truthful results
Each action's reply MUST report the real outcome, including failure.

#### Scenario: Failure
- GIVEN an app fails to launch
- WHEN the tool returns the failure
- THEN Niva says it failed and the mascot shows error/confused

## Assumptions to confirm
- Clamping out-of-range volume is an assumption.
- Window management, media control and other PC actions are not in scope.
