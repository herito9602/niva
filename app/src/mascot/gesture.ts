// Pure gesture maths for the robot's hands and dizzy eyes. Positions are in
// the drawing space of `drawRobot` (units, origin at the head centre, y down).
// Every loop has a period of one second (or a divisor of it), so the
// functions are periodic in `LOOP_PERIOD`. `motion` is 1 normally and 0 under
// prefers-reduced-motion, which freezes the loops into static poses.

export interface Vec {
  x: number;
  y: number;
}

/** Blend weights (0..1) of the hand gestures, taken from the pose springs. */
export interface HandWeights {
  scratch: number;
  thumbs: number;
  typing: number;
  raise: number;
  cup: number;
  pans: number;
}

export interface Hands {
  left: Vec;
  right: Vec;
}

export const LOOP_PERIOD = 1;
export const SPIRAL_PERIOD = 0.9;

const TAU = Math.PI * 2;

export const HAND_REST: Hands = {
  left: { x: -37, y: 33 },
  right: { x: 37, y: 33 },
};

const sin = (cycles: number) => Math.sin(TAU * cycles);

function rest(t: number, motion: number): Hands {
  // A barely visible sway so a resting hand is not frozen.
  return {
    left: { x: HAND_REST.left.x, y: HAND_REST.left.y + 0.8 * motion * sin(t) },
    right: { x: HAND_REST.right.x, y: HAND_REST.right.y + 0.8 * motion * sin(t + 0.25) },
  };
}

/** Left hand above the head on the top-left, its index finger pointing down
 *  onto the crown, scratching along the head surface about four times a
 *  second. The stroke runs along the outline's tangent there. */
function scratchLeft(t: number, motion: number): Vec {
  const stroke = 3.5 * motion * sin(4 * t);
  return { x: -19.7 + 0.94 * stroke, y: -45.8 - 0.33 * stroke };
}

/** Right hand cupped behind the ear, just outside the side plate, adjusting
 *  gently. */
function cupRight(t: number, motion: number): Vec {
  return { x: 47 + 0.8 * motion * sin(t), y: -2 };
}

/** Frying pans (reminder-alert), seen from the side and held above the head.
 *  The rim opening is an ellipse of half-width `PAN_R`; the long handle runs
 *  from the pan centre down and outward to the mitten, `PAN_HANDLE` units from
 *  the centre. Each pan swings from steep and wound up (apart) to flatter with
 *  the inner rim edges meeting above the head centre. */
export const PAN_R = 13;
export const PAN_RY = 5.5;
export const PAN_HANDLE = 39;
/** The pans meet this many times a second (divides the loop period). */
const PAN_HZ = 2;
/** The first hit; then one every `1 / PAN_HZ` seconds. */
const FIRST_HIT = 0.25;
const PAN_ANGLE_APART = 0.8;
const PAN_ANGLE_HIT = 0.3;
/** Distance of each inner rim edge from the head centre line when apart. */
const TIP_GAP = 20;
/** Height of the contact point above the head centre, and how much higher the
 *  wound-up pans are lifted. */
export const PAN_HIT_Y = -56;
const TIP_LIFT = 6;

/** How close the pans are: 0 apart (and held raised), 1 touching. Zero under
 *  reduced motion, so the pans stay up and apart. */
export function panCloseness(t: number, motion: number): number {
  return motion * (0.5 - 0.5 * Math.cos(TAU * PAN_HZ * t)) + 0;
}

export interface PanPose {
  /** Inner rim edge, the point that meets the other pan. */
  tip: Vec;
  /** Middle of the rim opening. */
  center: Vec;
  /** Where the mitten grips the end of the handle. */
  hand: Vec;
  /** Handle angle below the horizontal in radians (the pan tilts with it). */
  angle: number;
}

/** One pan for closeness `d` (0 apart .. 1 touching). `side` -1 is the left
 *  pan; the handle points outward and down from its centre. */
export function panPose(side: -1 | 1, d: number): PanPose {
  const k = Math.min(1, Math.max(0, d));
  const angle = PAN_ANGLE_APART + (PAN_ANGLE_HIT - PAN_ANGLE_APART) * k;
  const dir = { x: side * Math.cos(angle), y: Math.sin(angle) };
  const tip = { x: side * TIP_GAP * (1 - k), y: PAN_HIT_Y - TIP_LIFT * (1 - k) };
  const center = { x: tip.x + dir.x * PAN_R, y: tip.y + dir.y * PAN_R };
  const hand = { x: center.x + dir.x * PAN_HANDLE, y: center.y + dir.y * PAN_HANDLE };
  return { tip, center, hand, angle };
}

/** Hand positions while banging the pans: gripping the handle ends. */
function panHands(t: number, motion: number): Hands {
  const d = panCloseness(t, motion);
  return { left: panPose(-1, d).hand, right: panPose(1, d).hand };
}

/** Seconds since the last hit, in [0, 1 / PAN_HZ). */
function sinceHit(t: number): number {
  const period = 1 / PAN_HZ;
  return (((t - FIRST_HIT) % period) + period) % period;
}

/** Which hit this is (alternates parity), for the side and tilt of the CLANG. */
function hitParity(t: number): -1 | 1 {
  const n = Math.floor((t - FIRST_HIT) * PAN_HZ + 1e-9);
  return n % 2 === 0 ? 1 : -1;
}

export const IMPACT_SECONDS = 0.12;
export const SHOCK_SECONDS = 0.3;
export const SHOCK_DELAY = 0.07;
export const CLANG_SECONDS = 0.4;
const VIBE_SECONDS = 0.22;
const SHOCK_R0 = 5;
const SHOCK_R1 = 23;

/** Flash of the clang (star and noise lines): 1 at the instant the pans meet,
 *  gone after 120 ms; zero under reduced motion. */
export function impactAlpha(t: number, motion: number): number {
  if (motion === 0) return 0;
  return Math.max(0, 1 - sinceHit(t) / IMPACT_SECONDS);
}

/** Two shockwave rings expanding from the contact point and fading; the second
 *  trails the first. All invisible between hits and under reduced motion. */
export function shockwaves(t: number, motion: number): Array<{ radius: number; alpha: number }> {
  const since = motion === 0 ? Infinity : sinceHit(t);
  return [0, SHOCK_DELAY].map((delay) => {
    const p = (since - delay) / SHOCK_SECONDS;
    if (!(p >= 0 && p < 1)) return { radius: 0, alpha: 0 };
    return { radius: SHOCK_R0 + (SHOCK_R1 - SHOCK_R0) * (1 - (1 - p) ** 2), alpha: (1 - p) ** 1.5 };
  });
}

/** The comic "CLANG!" word: pops at each hit, scales up while fading out, and
 *  tips the other way (and sits on the other side) on every second hit. */
export function clangPop(
  t: number,
  motion: number,
): { alpha: number; scale: number; rot: number; side: -1 | 1 } {
  const side = hitParity(t);
  const p = motion === 0 ? 1 : sinceHit(t) / CLANG_SECONDS;
  if (!(p < 1)) return { alpha: 0, scale: 0, rot: 0, side };
  return {
    alpha: p < 0.55 ? 1 : 1 - (p - 0.55) / 0.45,
    scale: 0.6 + 0.7 * (1 - (1 - p) ** 3),
    rot: side * 0.14,
    side,
  };
}

/** Vibration of the pans right after a hit: strength of the vibration ticks
 *  and a small sideways jitter in units. Settles within about 0.2 s. */
export function panVibe(t: number, motion: number): { alpha: number; dx: number } {
  if (motion === 0) return { alpha: 0, dx: 0 };
  const since = sinceHit(t);
  if (since >= VIBE_SECONDS) return { alpha: 0, dx: 0 };
  const alpha = 1 - since / VIBE_SECONDS;
  return { alpha, dx: 1.6 * alpha * Math.sin(TAU * 30 * since) + 0 };
}

/** Key press strength in [0, 1] of one typing hand: four taps a second, the
 *  two hands half a cycle apart. Zero when motion is off. */
export function typingPress(t: number, side: -1 | 1, motion: number): number {
  return motion === 0 ? 0 : Math.max(0, sin(4 * t + (side === -1 ? 0 : 0.5)));
}

/** Both hands on the protruding deck edges beside the laptop lid (the deck's
 *  top is at y = 49), each dipping a little on its tap. */
function typingHands(t: number, motion: number): Hands {
  return {
    left: { x: -31, y: 42 + 2.5 * typingPress(t, -1, motion) },
    right: { x: 31, y: 42 + 2.5 * typingPress(t, 1, motion) },
  };
}

function thumbsRight(t: number, motion: number): Vec {
  return { x: 45, y: 2 + 1.5 * motion * sin(t) };
}

function raisedRight(t: number, motion: number): Vec {
  return { x: 43, y: -10 + 1.2 * motion * sin(t) };
}

/** Blends the gestures over the rest pose by weight, so a state change moves
 *  the hands smoothly instead of cutting. */
export function handPositions(w: HandWeights, t: number, motion: number): Hands {
  const base = rest(t, motion);
  const scratch = scratchLeft(t, motion);
  const typing = typingHands(t, motion);
  const thumbs = thumbsRight(t, motion);
  const raised = raisedRight(t, motion);
  const cup = cupRight(t, motion);
  const pans = panHands(t, motion);
  return {
    left: {
      x:
        base.left.x +
        w.scratch * (scratch.x - base.left.x) +
        w.typing * (typing.left.x - base.left.x) +
        w.pans * (pans.left.x - base.left.x),
      y:
        base.left.y +
        w.scratch * (scratch.y - base.left.y) +
        w.typing * (typing.left.y - base.left.y) +
        w.pans * (pans.left.y - base.left.y),
    },
    right: {
      x:
        base.right.x +
        w.thumbs * (thumbs.x - base.right.x) +
        w.raise * (raised.x - base.right.x) +
        w.cup * (cup.x - base.right.x) +
        w.pans * (pans.right.x - base.right.x) +
        w.typing * (typing.right.x - base.right.x),
      y:
        base.right.y +
        w.thumbs * (thumbs.y - base.right.y) +
        w.raise * (raised.y - base.right.y) +
        w.cup * (cup.y - base.right.y) +
        w.pans * (pans.right.y - base.right.y) +
        w.typing * (typing.right.y - base.right.y),
    },
  };
}

/** Rotation of the dizzy spiral eyes in [0, 2pi); 0 when motion is off. */
export function spiralAngle(t: number, motion: number): number {
  if (motion === 0) return 0;
  const phase = (t / SPIRAL_PERIOD) % 1;
  return TAU * (phase < 0 ? phase + 1 : phase);
}

/** Archimedean spiral from the centre out to `radius`, `steps + 1` points. */
export function spiralPoints(turns: number, radius: number, steps: number, rotation: number): Vec[] {
  const pts: Vec[] = [];
  for (let i = 0; i <= steps; i++) {
    const f = i / steps;
    const a = rotation + TAU * turns * f;
    pts.push({ x: radius * f * Math.cos(a), y: radius * f * Math.sin(a) });
  }
  return pts;
}

/** The thinking "?" bobbing above the right side of the head, once a second.
 *  Static under reduced motion. */
export function questionBob(t: number, motion: number): { dy: number; rot: number } {
  return { dy: 2 * motion * sin(t) + 0, rot: 0.12 * motion * sin(t + 0.25) + 0 };
}

const STATIC_WAVES = [0.65, 0.5, 0.35];

/** Opacity of the three sound-wave arcs reaching the cupped hand, outermost
 *  first; the pulse travels inward once a second. Static under reduced motion. */
export function waveAlphas(t: number, motion: number): number[] {
  return STATIC_WAVES.map((still, i) => {
    const pulse = 0.2 + 0.75 * (0.5 + 0.5 * sin(t - i / 3));
    return still + motion * (pulse - still);
  });
}
