# Reminders Specification (delta against niva-mvp)

Baseline: `openspec/changes/niva-mvp/specs/reminders/spec.md`. Unchanged: "Create reminder", "List reminders", "Persistence".

## MODIFIED Requirements

### Requirement: Fire reminder
At the due time the system MUST alert the user with the notch in the reminder-alert look and a soft chime, and MUST speak the reminder. The reminder text MUST NOT be shown. Over a fullscreen app, `fullscreen-policy` applies instead (chime and pulse, no speech).
(Previously: the reminder text was shown and spoken, with the mascot in reminder-alert.)

#### Scenario: Fires at due time
- GIVEN a reminder due now and no fullscreen app
- WHEN the due time arrives
- THEN the notch shows the coral pulse, a soft chime plays, and the reminder is spoken
- AND no reminder text appears on screen

#### Scenario: Speaking overrides the pulse
- GIVEN a reminder alert is showing and Niva starts speaking it
- WHEN the speech plays
- THEN the notch shows the speaking look

#### Scenario: The alert ends with the speech
- GIVEN a reminder is being spoken
- WHEN the speech ends
- THEN the coral pulse does not resume, the alert is over, and the notch settles (1.5 s) and hides

## Notes
- The 30 s alert timer is unchanged and bounds a reminder that is never spoken (fullscreen).
