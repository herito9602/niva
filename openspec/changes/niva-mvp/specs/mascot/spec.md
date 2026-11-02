# Mascot Specification

## Purpose

An original procedural robot character whose appearance reflects the assistant state.

## Requirements

### Requirement: Eight visible states
The mascot MUST render distinct, visibly different looks for: idle, listening, thinking, acting, success, error/confused, confirmation-needed, reminder-alert.

#### Scenario: State change
- GIVEN the mascot is in any state
- WHEN the assistant state changes to another of the eight states
- THEN the mascot transitions to the new look with animation, without a hard cut

#### Scenario: Idle life
- GIVEN the state is idle
- WHEN time passes with no input
- THEN the mascot breathes and blinks

### Requirement: State driven only by assistant state
The mascot MUST derive its look solely from the current assistant state, so a debug switcher can drive every state without AI or voice.

#### Scenario: Debug switcher
- GIVEN a debug build with the switcher
- WHEN a tester selects each of the 8 states
- THEN each state's look is displayed

### Requirement: Original, asset-free character
The mascot MUST NOT use image assets, nor the Coucou/"Mochi" character, name or media.

#### Scenario: Asset audit
- GIVEN the built app
- WHEN its bundled resources are inspected
- THEN no raster/vector character images are present for the mascot

## Assumptions to confirm
- "Original design review" (proposal risk) is a human sign-off, not an automated check.
