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

#### Scenario: Fuzzy name (PR 2d-a)
- GIVEN the Start menu has "Visual Studio Code", "Calculator" and "Notepad"
- WHEN the model names "studio code", "calculater" (a typo) or "notepad" in capitals
- THEN the matching app starts (exact name first, then prefix, whole word, the name inside a longer phrase, and a typo of one letter, two from eight letters, from five letters up); a query under four letters matches only a whole name or a whole word of a name, never a prefix; a query of fewer than two letters, an uninstaller or maintenance entry (never started, not even when asked for by name: `open_app` has no confirmation, and only launchable apps are indexed) and a name nothing is close to start nothing

#### Scenario: The index is still loading (PR 2d-a)
- GIVEN the Start menu index has not been built yet (it is built in the background at start-up)
- WHEN the user asks to open an app
- THEN nothing starts and Niva says the app list is still loading, without waiting for the index

#### Scenario: The name is data, never a command (PR 2d-a)
- GIVEN a model name such as `notepad & calc.exe`, `"; Start-Process calc; "` or a path
- WHEN the tool runs
- THEN the text is only compared with the names of the Start menu, and only the AppID of the matching entry is handed to Windows; no part of the model's text reaches a shell, a command line or a path

#### Scenario: Returns without waiting for the app (PR 2d-a)
- GIVEN the app takes seconds to show its window
- WHEN Windows has accepted the launch
- THEN the tool returns at once (the tool timer is 5 s); it never waits for the window or the process

#### Scenario: Launch refused (PR 2d-a)
- GIVEN Windows refuses to start the matched entry
- WHEN the tool returns
- THEN Niva says it failed to open the app (it does not say it opened it)

### Requirement: Built-in app names in Spanish and English (PR 2d-a)
The system MUST open a built-in Windows app by its Spanish or its English name, whichever language the Start menu uses on this PC. The alias applies to the WHOLE name the model gave (after dropping one leading article and one trailing `app` or `aplicacion`), never to a word inside a longer name, and never reaches an uninstaller or maintenance entry.

#### Scenario: Both directions
- GIVEN the Start menu has "Calculator" and "Configuración" (or, on another PC, "Calculadora" and "Settings")
- WHEN the model names "calculadora", "la calculadora" or "settings" (or "calculator" and "configuración")
- THEN the entry in the menu's own language starts, and "settings" prefers "Configuración" over "WSL Settings"

#### Scenario: A longer name is not an alias
- GIVEN the Start menu has "Camera" and "Configuración de cámara Logitech"
- WHEN the model names "camara logitech"
- THEN the Logitech entry starts, not Windows Camera

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

#### Scenario: Raise and lower (PR 2d-a)
- GIVEN current volume 95
- WHEN the user asks to raise it by 10
- THEN the volume is 100 and Niva reports that it was clamped; a change that lands inside 0-100 is not reported as clamped

#### Scenario: Mute and unmute (PR 2d-a)
- GIVEN current volume 40, not muted
- WHEN the user asks to mute, then to unmute
- THEN the device is muted with the level still 40, then unmuted at 40

#### Scenario: Raising a muted device unmutes it (PR 2d-a, like the Windows slider)
- GIVEN the device is muted
- WHEN the user asks for volume 30, or to raise it
- THEN the level is set and the device is unmuted, so the change is audible; lowering the volume, or setting 0, leaves the mute alone

#### Scenario: The reply is what the device says (PR 2d-a)
- GIVEN a device that moves in coarse steps (a request for 31 lands on 30)
- WHEN the tool returns
- THEN the reply states the level read back from the device; a device that does not take the write (more than 2 points from the request, or the wrong mute state) is a failure, never a success

#### Scenario: No audio device (PR 2d-a)
- GIVEN there is no audio output device
- WHEN the user asks to change the volume
- THEN nothing is written and Niva says it did not work

### Requirement: Truthful results
Each action's reply MUST report the real outcome, including failure.

#### Scenario: Failure
- GIVEN an app fails to launch
- WHEN the tool returns the failure
- THEN Niva says it failed and the mascot shows error/confused

### Requirement: Running processes (PR 2d-a)
The system MUST list the running processes with their executable name and whether each belongs to the user's session. A process whose session cannot be read MUST count as outside the user's session.

#### Scenario: Own process
- GIVEN the app is running in the user's session
- WHEN the processes are listed
- THEN it appears with its executable name and as a process of the user's session

### Requirement: Risky actions are not available in this release (PR 2d-a)
Until the island can answer a confirmation (PR 2d-b), a release build MUST NOT close apps or control power. A request that includes `close_app` or `power_action` MUST end in a truthful "not available yet" reply, BEFORE any confirmation prompt, and nothing in the same request may run. The Windows adapter's `close_app` and `power` MUST NOT act even if called. Debug builds keep the dry-run PC for these actions.

#### Scenario: Risky request in the release
- GIVEN the release build and a model response that closes an app or asks for lock, sleep, restart or shutdown
- WHEN the response is validated
- THEN Niva says it cannot close apps or control the power of the PC yet, shows no confirmation, and Windows is not touched

#### Scenario: Risky call among safe ones
- GIVEN a response with "open notepad", a volume change and a `close_app` call
- WHEN it is validated
- THEN none of the three runs and Niva gives the same reply

#### Scenario: Safe requests are unaffected
- GIVEN the release build
- WHEN the user asks to open an app or change the volume
- THEN the action runs for real and the reply states its real outcome

## Assumptions to confirm
- Clamping out-of-range volume is an assumption.
- Raising the volume of a muted device also unmutes it (Windows slider behaviour; PR 2d-a).
- Window management, media control and other PC actions are not in scope.
