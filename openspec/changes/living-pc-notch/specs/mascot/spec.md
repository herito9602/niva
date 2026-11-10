# Mascot Specification (delta against niva-mvp)

Baseline: `openspec/changes/niva-mvp/specs/mascot/spec.md`. The whole capability is removed; the `notch` capability replaces it.

## REMOVED Requirements

### Requirement: Eight visible states
Reason: the robot character no longer exists; the product pivots to a living-PC notch. This supersedes niva-mvp's success criterion "The mascot visibly shows all 8 states".
Migration: the `notch` requirement "Palette and motion per state" defines seven looks. `acting` reuses the thinking look and `countdown` the amber pulse; idle is hidden.

### Requirement: State driven only by assistant state
Reason: the mascot component is deleted.
Migration: the notch derives its look from the assistant phase (plus the speaking flag and level messages). The debug window keeps its 8-state switcher, now driving the notch (`notch`, "Palette and motion per state").

### Requirement: Original, asset-free character
Reason: there is no character any more.
Migration: `notch`, "Notch geometry and waveform", keeps the no-image-assets rule (scenario "Asset audit").

### Requirement: Hands and per-state gestures (owner request, 2026-11-03)
Reason: hands, laptop, pans and other gestures belong to the removed robot.
Migration: per-state meaning is carried by color and bar motion (`notch`, "Palette and motion per state"). Reduced-motion behavior moves to `notch`, "Non-color cue and reduced motion".

### Requirement: Character look (owner request, 2026-11-04, changed 2026-11-05 after the originality review; supersedes the look notes above where they differ)
Reason: the head, ears, screen and mouth designs belong to the removed robot, and the originality review no longer applies.
Migration: none; the notch has no face. Gaze following the cursor is dropped together with the cursor poller (`island-shell`, "Click-through").
