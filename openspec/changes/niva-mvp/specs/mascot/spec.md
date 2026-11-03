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

### Requirement: Hands and per-state gestures (owner request, 2026-11-03)
The mascot MUST have two hands drawn procedurally, each with a rest pose in every state and animated through the same springs as the rest of the look. The look MUST add these gestures, driven only by the assistant state:
- thinking: one hand scratches the side of the head in a loop while the other rests; the gesture stays during an AskFollowUp question (the state is still `thinking`).
- success: one hand gives a thumbs-up; the eyes become happy closed arcs ("^ ^") and the head chuckles with a small rhythmic shake.
- error: the eyes become rotating red spirals (dizzy) with a slight head wobble.
- acting: a small laptop is drawn in front of the body and both hands tap its keyboard alternately.
- confirmation-needed: one hand is held up (optional flourish); idle keeps both hands at rest.
- reminder-alert (owner request, 2026-11-03; reworked 2026-11-03 after owner feedback 6, "loud frying pans"): the robot holds two frying pans above its head, one in each raised hand. Each pan is seen from the side as a shallow dish of dark metal with an elliptical opening (accent rim, highlighted inside) and a long straight handle running down and outward to the mitten gripping its end, clearly a stick and not a ring. The pans swing together over the head, tilting toward each other, and clang about twice a second with the rim edges meeting above the head centre; the antenna is laid over to the side so it does not collide. The noise is visual and must be big: at each hit a bold accent star burst, one or two expanding shockwave rings that fade out, short noise lines radiating outward, a comic "CLANG!" word in bold accent letters with a dark outline that pops, scales up, fades and alternates side and slight rotation, vibration ticks on the pans, a stronger head kick and shake, and wide alarmed eyes that pop wider on the hit. Everything stays inside the 96 px canvas. Under reduced motion the pans are held raised, still and apart over the head, with no burst, rings, CLANG text or shake (a static small "!" is shown). Visual only: the reminder's soft chime policy is unchanged and no clang sound is added.
Gesture changes MUST blend smoothly (no hard cut) and no image assets are used. Owner look feedback 2026-11-03: the hands MUST be simple, bold cartoon-robot hands: a single smooth rounded mitten (no separate fingers) with a small thumb nub, slate fill with a soft highlight, a dark outline and a thin accent rim, about the size of an eye, on thin smooth curved arms from small shoulder joints. Resting hands are relaxed mittens hanging down. The thumbs-up MUST be a closed fist with a thick, clearly separate thumb pointing straight up. The raised hand (confirmation-needed) is an open flat mitten, palm toward the viewer. The thinking gesture is the classic one-finger head scratch (owner, 2026-11-03): one hand rises ABOVE the head on the top-left with ONE index finger pointing DOWN onto the crown, the fingertip touching the head outline and scratching along it a few units at about 4 Hz; the head tips toward the hand, the eyes glance up toward it, and a small accent-coloured "?" bobs near the opposite top side (static under reduced motion); it stays during an AskFollowUp question. The listening gesture (owner, 2026-11-03): one hand rises to the side of the head and cups behind the ear at the side plate (open mitten, thumb toward the head), the head leans 0.15-0.25 rad toward that side with a small sideways shift so the ear points outward, the eyes glance away from the hand, and two or three accent arcs (sound waves) pulse inward toward the hand (static under reduced motion); the other hand rests. The acting laptop is seen from behind its lid: a small slate lid below the chin (the face stays visible) with a subtle accent top edge and a small ring emblem (no brand logos, no code lines) in front of a deck strip wider than the lid; the screen's light tints the robot's visor from below so the screen is known to face the robot; the hands tap alternately on the protruding deck edges at the sides, not over the lid.

#### Scenario: Gesture per state
- GIVEN a debug build with the switcher
- WHEN a tester selects thinking, success, error and acting
- THEN they see, respectively, a scratching hand, a thumbs-up with arc eyes and a chuckle, spiral eyes with a wobble, and a laptop with typing hands

#### Scenario: Reduced motion
- GIVEN the OS requests reduced motion
- WHEN any state is shown
- THEN looping gestures (scratching, typing, chuckle, wobble, spiral rotation, bounce, breathing) are static; state changes still transition and the character still blinks and follows the cursor

## Assumptions to confirm
- "Original design review" (proposal risk) is a human sign-off, not an automated check.
