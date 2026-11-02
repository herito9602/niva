# Island Shell Specification

## Purpose

The floating window, tray icon and shortcut through which the user sees and invokes Niva.

## Requirements

### Requirement: Island window
The system MUST show a transparent, always-on-top island at the top-center of the primary display.

#### Scenario: Island placement
- GIVEN Niva is running
- WHEN the island is shown
- THEN it is horizontally centered at the top of the primary display and above normal windows
- AND its background is transparent outside the island shape

### Requirement: Tray icon
The system MUST provide a tray icon with at least "Show island" and "Quit" entries.

#### Scenario: Quit from tray
- GIVEN Niva is running
- WHEN the user chooses "Quit" in the tray menu
- THEN the island and tray icon disappear and the process exits

### Requirement: Global shortcut
The system MUST toggle activation on Ctrl+Shift+Space from any application.

#### Scenario: Shortcut shows island
- GIVEN the island is hidden and another app has focus
- WHEN the user presses Ctrl+Shift+Space
- THEN the island is shown and Niva starts listening

### Requirement: Show/hide lifecycle
The system MUST hide the island when an interaction ends and show it again on the next activation.

#### Scenario: Auto-hide
- GIVEN a command has completed and Niva has finished replying
- WHEN the idle period elapses
- THEN the island is hidden and the tray icon remains

## Assumptions to confirm
- Auto-hide idle period: 3 s settle after the reply (design State Machine, success/error rows).
- Behavior on multi-monitor setups is unspecified; v1 uses the primary display.
