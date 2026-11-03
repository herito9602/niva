// The Niva robot: spring animation state, blink scheduler, gaze maths and the
// Canvas 2D drawing. Everything except `drawRobot` is pure and unit tested.
//
// Original design, drawn from scratch with primitives only (no image assets):
// a rounded-square head with a dark visor, two capsule eyes, small side
// "ear" plates, a bulb antenna and two mitt hands on short arms. Its accent
// colour follows the state. Gestures (scratching, thumbs-up, typing on a small
// laptop), happy arc eyes and dizzy spiral eyes are blended by spring weights.

import {
  armCurve,
  burstSegments,
  HAND_SCALE,
  handParts,
  handShape,
  type HandShape,
  type Part,
  laptopGeometry,
  SHOULDERS,
  starPoints,
  vibeSegments,
} from "./geometry";
import {
  PAN_HANDLE,
  PAN_HIT_Y,
  PAN_R,
  PAN_RY,
  clangPop,
  handPositions,
  impactAlpha,
  panCloseness,
  panPose,
  panVibe,
  questionBob,
  shockwaves,
  spiralAngle,
  spiralPoints,
  waveAlphas,
  type HandWeights,
  type Hands,
  type Vec,
} from "./gesture";
import { POSE_KEYS, type PoseParams } from "./pose";
import { criticalDamping, stepSpring, type SpringConfig, type SpringState } from "./spring";

type PoseKey = (typeof POSE_KEYS)[number];

export interface Rig {
  pose: Record<PoseKey, SpringState>;
  gazeX: SpringState;
  gazeY: SpringState;
  /** Bounce cycles, accumulated so a frequency change never jumps (in [0, 1)). */
  phase: number;
}

const bouncy = (stiffness: number, damping: number): SpringConfig => ({ stiffness, damping });
const settled = (stiffness: number): SpringConfig => ({
  stiffness,
  damping: criticalDamping(stiffness),
});

// Per-parameter feel: shape and motion parameters overshoot a little (alive),
// colours, amplitudes and blend weights settle without overshoot (no flashing,
// no negative weights).
const SPRING_BY_KEY: Record<PoseKey, SpringConfig> = {
  eyeOpen: bouncy(260, 22),
  eyeWidth: bouncy(220, 20),
  eyeSlant: bouncy(200, 16),
  headTilt: bouncy(170, 13),
  headLift: bouncy(200, 14),
  scale: bouncy(240, 15),
  antennaLean: bouncy(120, 7),
  antennaWobble: settled(80),
  glow: settled(90),
  breath: settled(60),
  bounceAmp: settled(120),
  bounceHz: settled(120),
  accentR: settled(70),
  accentG: settled(70),
  accentB: settled(70),
  shakeAmp: settled(80),
  wobbleAmp: settled(80),
  handScratch: settled(110),
  handThumbs: settled(110),
  handTyping: settled(110),
  handRaise: settled(110),
  eyeArcs: settled(130),
  eyeSpiral: settled(130),
  propLaptop: settled(100),
  handCup: settled(110),
  propQuestion: settled(100),
  propWaves: settled(100),
  handPans: settled(110),
  propImpact: settled(120),
  headShift: settled(90),
  lookX: settled(120),
  lookY: settled(120),
};
const GAZE_SPRING = bouncy(180, 20);

const rest = (value: number): SpringState => ({ value, velocity: 0 });

export function createRig(initial: PoseParams): Rig {
  const pose = {} as Record<PoseKey, SpringState>;
  for (const key of POSE_KEYS) pose[key] = rest(initial[key]);
  return { pose, gazeX: rest(0), gazeY: rest(0), phase: 0 };
}

const MAX_STEP = 0.1;

export function stepRig(
  rig: Rig,
  target: PoseParams,
  gaze: { x: number; y: number },
  dt: number,
): Rig {
  const pose = {} as Record<PoseKey, SpringState>;
  for (const key of POSE_KEYS) {
    pose[key] = stepSpring(rig.pose[key], target[key], SPRING_BY_KEY[key], dt);
  }
  const hz = rig.pose.bounceHz.value;
  const advance = dt > 0 && Number.isFinite(dt) && hz > 0 ? hz * Math.min(dt, MAX_STEP) : 0;
  return {
    pose,
    gazeX: stepSpring(rig.gazeX, gaze.x, GAZE_SPRING, dt),
    gazeY: stepSpring(rig.gazeY, gaze.y, GAZE_SPRING, dt),
    phase: (rig.phase + advance) % 1,
  };
}

// --- Blinking -------------------------------------------------------------

export const BLINK_MIN_S = 2;
export const BLINK_MAX_S = 6;
export const BLINK_DURATION = 0.16;

export interface Blinker {
  /** Eye closure at `now` (seconds): 0 open .. 1 shut. Call once per frame. */
  amount(now: number): number;
  nextBlinkAt(): number;
}

/** Random blinks every 2-6 s; the RNG ([0, 1)) and the clock are injected. */
export function createBlinker(rng: () => number, start: number): Blinker {
  const interval = () => BLINK_MIN_S + rng() * (BLINK_MAX_S - BLINK_MIN_S);
  let next = start + interval();
  let blinkStart = -1;

  return {
    amount(now) {
      if (now >= next) {
        blinkStart = next;
        next = next + BLINK_DURATION + interval();
        // After a long gap (hidden tab) do not replay missed blinks.
        if (next <= now) next = now + interval();
      }
      if (blinkStart < 0) return 0;
      const p = (now - blinkStart) / BLINK_DURATION;
      if (p >= 1) {
        blinkStart = -1;
        return 0;
      }
      return p < 0.5 ? 2 * p : 2 - 2 * p;
    },
    nextBlinkAt: () => next,
  };
}

// --- Gaze -----------------------------------------------------------------

/** Unit-circle vector from the mascot centre toward the cursor. Magnitude
 *  grows with distance and saturates at `range` logical pixels. Non-finite
 *  input gives a neutral gaze. */
export function lookVector(
  cursor: { x: number; y: number } | null,
  center: { x: number; y: number },
  range: number,
): { x: number; y: number } {
  if (!cursor || !(range > 0)) return { x: 0, y: 0 };
  const dx = cursor.x - center.x;
  const dy = cursor.y - center.y;
  if (!Number.isFinite(dx) || !Number.isFinite(dy)) return { x: 0, y: 0 };
  const dist = Math.hypot(dx, dy);
  if (dist === 0) return { x: 0, y: 0 };
  const k = Math.min(1, dist / range) / dist;
  return { x: dx * k, y: dy * k };
}

// --- Frame ----------------------------------------------------------------

export interface Frame {
  pose: PoseParams;
  gaze: { x: number; y: number };
  eyeOpen: number;
  offsetX: number;
  offsetY: number;
  tilt: number;
  scale: number;
  antennaAngle: number;
  hands: Hands;
  spiralAngle: number;
  /** Bobbing of the thinking question mark. */
  question: { dy: number; rot: number };
  /** Opacity of the three sound-wave arcs while listening. */
  waves: number[];
  /** How close the frying pans are (0..1) and the clang flash (0..1). */
  panClose: number;
  impact: number;
  /** Shockwave rings, the CLANG word, pan vibration, and the static "!" shown
   *  instead of all of it under reduced motion (0 or 1). */
  shock: Array<{ radius: number; alpha: number }>;
  clang: { alpha: number; scale: number; rot: number; side: -1 | 1 };
  vibe: { alpha: number; dx: number };
  alertMark: number;
}

export function currentPose(rig: Rig): PoseParams {
  const pose = {} as PoseParams;
  for (const key of POSE_KEYS) pose[key] = rig.pose[key].value;
  return pose;
}

/** Chuckle (success) and dizzy wobble (error) have fixed rates; only their
 *  amplitudes are springs. */
const CHUCKLE_HZ = 6.5;
const WOBBLE_HZ = 2.4;

/** Applies the time-based layers on top of the spring values. `motion` is 1
 *  normally and 0 under prefers-reduced-motion (no breathing, bouncing, sway,
 *  chuckle, wobble, scratching, typing or spiral rotation; pose transitions,
 *  blinks and gaze still play). */
export function composeFrame(rig: Rig, t: number, blink: number, motion: number): Frame {
  const pose = currentPose(rig);
  const tau = Math.PI * 2;
  const weight = Math.min(1, Math.max(0, pose.propImpact));
  const hit = impactAlpha(t, motion) * weight;
  const vibe = panVibe(t, motion);
  const clang = clangPop(t, motion);
  const hop = -Math.abs(Math.sin(Math.PI * rig.phase)) * pose.bounceAmp;
  const breath = Math.sin((tau * t) / 3.6) * pose.breath;
  const sway = Math.sin(tau * 1.7 * t) * pose.antennaWobble;
  return {
    pose,
    gaze: {
      x: Math.min(1, Math.max(-1, rig.gazeX.value + pose.lookX)),
      y: Math.min(1, Math.max(-1, rig.gazeY.value + pose.lookY)),
    },
    eyeOpen: pose.eyeOpen * (1 - blink),
    offsetX: Math.sin(tau * CHUCKLE_HZ * t) * pose.shakeAmp * motion + pose.headShift + vibe.dx * weight + 0, // + 0 turns -0 into 0
    offsetY: pose.headLift + hop * motion + 4 * hit,
    tilt: pose.headTilt + Math.sin(tau * WOBBLE_HZ * t) * pose.wobbleAmp * motion + 0.08 * hit * clang.side + 0,
    scale: pose.scale * (1 + breath * motion),
    antennaAngle: pose.antennaLean + sway * motion,
    hands: handPositions(
      {
        scratch: pose.handScratch,
        thumbs: pose.handThumbs,
        typing: pose.handTyping,
        raise: pose.handRaise,
        cup: pose.handCup,
        pans: pose.handPans,
      },
      t,
      motion,
    ),
    question: questionBob(t, motion),
    waves: waveAlphas(t, motion),
    panClose: panCloseness(t, motion),
    impact: hit,
    shock: shockwaves(t, motion).map((s) => ({ radius: s.radius, alpha: s.alpha * weight })),
    clang: { ...clang, alpha: clang.alpha * weight },
    vibe: { alpha: vibe.alpha * weight, dx: vibe.dx * weight },
    alertMark: (1 - motion) * weight,
    spiralAngle: spiralAngle(t, motion),
  };
}

export function accentCss(pose: PoseParams, alpha: number): string {
  const c = (v: number) => Math.round(Math.min(255, Math.max(0, v)));
  return `rgba(${c(pose.accentR)}, ${c(pose.accentG)}, ${c(pose.accentB)}, ${alpha})`;
}

// --- Canvas plumbing ------------------------------------------------------

/** Backing-store side in device pixels for a CSS size. */
export function backingStoreSize(size: number, dpr: number): number {
  const ratio = Number.isFinite(dpr) && dpr > 0 ? dpr : 1;
  return Math.round(size * ratio);
}

/** `getContext('2d')` that logs one warning if the context is unavailable. */
export function makeContextGetter(warn: (message: string) => void = console.warn) {
  let warned = false;
  return (canvas: {
    getContext: (id: "2d") => CanvasRenderingContext2D | null;
  }): CanvasRenderingContext2D | null => {
    const ctx = canvas.getContext("2d");
    if (!ctx && !warned) {
      warned = true;
      warn("[niva] mascot: 2d canvas context unavailable, the mascot is not drawn");
    }
    return ctx;
  };
}

// --- Drawing --------------------------------------------------------------

/** Design space is DESIGN_SIZE units square with the head centred a little
 *  low so the antenna, hands and laptop fit; `size` is the canvas side in
 *  CSS pixels. */
export const DESIGN_SIZE = 150;
const ORIGIN = { x: DESIGN_SIZE / 2, y: 66 };
export const MAX_EYE_TRAVEL = 4;

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));

function handWeights(pose: PoseParams): HandWeights {
  return {
    scratch: clamp01(pose.handScratch),
    thumbs: clamp01(pose.handThumbs),
    typing: clamp01(pose.handTyping),
    raise: clamp01(pose.handRaise),
    cup: clamp01(pose.handCup),
    pans: clamp01(pose.handPans),
  };
}

// One outline colour and weight for every prop, so the shapes read together.
const OUTLINE = "#0b0e1c";
const OUTLINE_W = 2.2;
const LAPTOP = laptopGeometry();

function pathPart(ctx: CanvasRenderingContext2D, p: Part): void {
  ctx.save();
  ctx.translate(p.cx, p.cy);
  ctx.rotate(p.angle);
  ctx.roundRect(-p.w / 2, -p.h / 2, p.w, p.h, p.r);
  ctx.restore();
}

/** Thin smooth arm with a soft elbow and a small round shoulder joint. */
function drawArm(ctx: CanvasRenderingContext2D, from: Vec, to: Vec, side: -1 | 1): void {
  const c = armCurve(from, to, side);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.beginPath();
  ctx.moveTo(c.from.x, c.from.y);
  ctx.quadraticCurveTo(c.ctrl.x, c.ctrl.y, c.to.x, c.to.y);
  ctx.strokeStyle = OUTLINE;
  ctx.lineWidth = 5.2;
  ctx.stroke();
  ctx.strokeStyle = "#7d86a3";
  ctx.lineWidth = 3;
  ctx.stroke();

  ctx.beginPath();
  ctx.arc(from.x, from.y, 3.8, 0, Math.PI * 2);
  ctx.fillStyle = "#5a648f";
  ctx.fill();
  ctx.strokeStyle = OUTLINE;
  ctx.lineWidth = 1.8;
  ctx.stroke();
}

/** Hand: one rounded mitten with a thumb nub (and an index finger when
 *  pointing). Slate fill with a soft highlight, a dark outline and a thin
 *  accent rim. */
function drawHand(ctx: CanvasRenderingContext2D, pos: Vec, shape: HandShape, pose: PoseParams): void {
  const parts = handParts(shape);
  const k = HAND_SCALE * shape.size;
  const build = () => {
    ctx.beginPath();
    parts.forEach((p) => pathPart(ctx, p));
  };
  ctx.save();
  ctx.translate(pos.x, pos.y);
  ctx.scale(shape.side * k, k);
  ctx.rotate(-shape.lean);
  ctx.lineJoin = "round";

  build();
  ctx.strokeStyle = OUTLINE;
  ctx.lineWidth = (OUTLINE_W * 2) / k;
  ctx.stroke();

  const fill = ctx.createLinearGradient(0, -shape.bodyH / 2, 0, shape.bodyH / 2);
  fill.addColorStop(0, "#8e9acb");
  fill.addColorStop(1, "#66719f");
  ctx.fillStyle = fill;
  ctx.fill();

  // Soft highlight on the upper inner side of the body.
  ctx.fillStyle = "rgba(255, 255, 255, 0.28)";
  ctx.beginPath();
  ctx.ellipse(-shape.bodyW * 0.17, -shape.bodyH * 0.2, shape.bodyW * 0.17, shape.bodyH * 0.13, -0.5, 0, Math.PI * 2);
  ctx.fill();

  build();
  ctx.strokeStyle = accentCss(pose, 0.8);
  ctx.lineWidth = 1 / k;
  ctx.stroke();
  ctx.restore();
}

/** Back of an open laptop, facing the robot: a small slate lid with an accent
 *  top edge and a ring emblem, in front of a wider deck strip. */
function drawLaptop(ctx: CanvasRenderingContext2D, pose: PoseParams, weight: number): void {
  const g = LAPTOP;
  const w = clamp01(weight);
  ctx.save();
  ctx.globalAlpha = w;
  ctx.translate(0, (1 - w) * 16);
  ctx.lineJoin = "round";
  ctx.strokeStyle = OUTLINE;
  ctx.lineWidth = OUTLINE_W;

  const deck = ctx.createLinearGradient(0, g.base.y, 0, g.base.y + g.base.h);
  deck.addColorStop(0, "#4c5586");
  deck.addColorStop(1, "#262b4a");
  ctx.fillStyle = deck;
  ctx.beginPath();
  ctx.roundRect(g.base.x, g.base.y, g.base.w, g.base.h, g.base.r);
  ctx.fill();
  ctx.stroke();

  const panel = ctx.createLinearGradient(0, g.lid.y, 0, g.lid.y + g.lid.h);
  panel.addColorStop(0, "#464f7d");
  panel.addColorStop(1, "#2c3252");
  ctx.fillStyle = panel;
  ctx.beginPath();
  ctx.roundRect(g.lid.x, g.lid.y, g.lid.w, g.lid.h, g.lid.r);
  ctx.fill();
  ctx.stroke();

  // Accent light leaking over the top edge of the lid.
  ctx.lineCap = "round";
  ctx.strokeStyle = accentCss(pose, 0.9);
  ctx.lineWidth = 1.8;
  ctx.shadowColor = accentCss(pose, 0.9);
  ctx.shadowBlur = 5;
  ctx.beginPath();
  ctx.moveTo(g.lid.x + g.lid.r + 1, g.lid.y + 0.5);
  ctx.lineTo(g.lid.x + g.lid.w - g.lid.r - 1, g.lid.y + 0.5);
  ctx.stroke();
  ctx.shadowBlur = 0;

  ctx.strokeStyle = accentCss(pose, 0.6);
  ctx.lineWidth = 1.8;
  ctx.beginPath();
  ctx.arc(g.emblem.cx, g.emblem.cy, g.emblem.r, 0, Math.PI * 2);
  ctx.stroke();
  ctx.restore();
}

/** "?" above the right side of the head while thinking, in the accent colour. */
function drawQuestion(ctx: CanvasRenderingContext2D, pose: PoseParams, frame: Frame): void {
  const w = clamp01(pose.propQuestion);
  if (w < 0.01) return;
  ctx.save();
  ctx.globalAlpha = w;
  ctx.translate(31, -43 + frame.question.dy);
  ctx.rotate(frame.question.rot);
  ctx.strokeStyle = accentCss(pose, 1);
  ctx.fillStyle = accentCss(pose, 1);
  ctx.lineWidth = 3.2;
  ctx.lineCap = "round";
  ctx.shadowColor = accentCss(pose, 0.9);
  ctx.shadowBlur = 6;
  ctx.beginPath();
  ctx.arc(0, -5, 5.2, Math.PI * 1.1, Math.PI * 2.55);
  ctx.lineTo(0, 3);
  ctx.stroke();
  ctx.beginPath();
  ctx.arc(0, 9, 1.9, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();
}

/** Three sound-wave arcs reaching the cupped hand from the outside. */
function drawWaves(ctx: CanvasRenderingContext2D, pose: PoseParams, frame: Frame): void {
  const w = clamp01(pose.propWaves);
  if (w < 0.01) return;
  const hand = frame.hands.right;
  ctx.save();
  ctx.strokeStyle = accentCss(pose, 1);
  ctx.lineWidth = 2;
  ctx.lineCap = "round";
  ctx.shadowColor = accentCss(pose, 0.9);
  ctx.shadowBlur = 4;
  [21, 16, 11].forEach((radius, i) => {
    ctx.globalAlpha = w * frame.waves[i];
    ctx.beginPath();
    ctx.arc(hand.x, hand.y, radius, -0.85, 0.85);
    ctx.stroke();
  });
  ctx.restore();
}

/** Two frying pans seen from the side, held above the head one per hand. Each
 *  is a shallow dark-metal dish with an accent rim, an elliptical opening
 *  showing the inside, and a long straight handle that runs down and outward
 *  to the mitten gripping its end. They swing together and clang. */
function drawPans(ctx: CanvasRenderingContext2D, pose: PoseParams, frame: Frame): void {
  const w = clamp01(pose.handPans);
  if (w < 0.01) return;
  const depth = 6.5;
  ctx.save();
  ctx.globalAlpha = w;
  ctx.lineJoin = "round";
  ctx.lineCap = "round";
  for (const side of [-1, 1] as const) {
    const p = panPose(side, frame.panClose);
    ctx.save();
    ctx.translate(p.center.x - side * frame.vibe.dx * 0.6, p.center.y);
    ctx.scale(side, 1);
    ctx.rotate(p.angle);
    // Handle: a long steel stick from the rim to the grip, with two rivets.
    const hx = PAN_R - 3;
    const hw = PAN_HANDLE - PAN_R + 5;
    ctx.beginPath();
    ctx.roundRect(hx, -1.1, hw, 4.4, 2.2);
    ctx.fillStyle = "#a3acd0";
    ctx.fill();
    ctx.strokeStyle = OUTLINE;
    ctx.lineWidth = OUTLINE_W;
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(hx + 4, -0.1);
    ctx.lineTo(hx + hw - 6, -0.1);
    ctx.strokeStyle = "rgba(255, 255, 255, 0.5)";
    ctx.lineWidth = 1;
    ctx.stroke();
    ctx.fillStyle = "#3a3f5e";
    for (const rx of [PAN_R + 1.5, PAN_R + 6]) {
      ctx.beginPath();
      ctx.arc(rx, 1.1, 0.9, 0, Math.PI * 2);
      ctx.fill();
    }
    // Dish: curved lower body under the rim.
    ctx.beginPath();
    ctx.moveTo(-PAN_R, 0);
    ctx.bezierCurveTo(-PAN_R * 0.95, depth, -PAN_R * 0.5, depth + PAN_RY, 0, depth + PAN_RY);
    ctx.bezierCurveTo(PAN_R * 0.5, depth + PAN_RY, PAN_R * 0.95, depth, PAN_R, 0);
    ctx.closePath();
    const metal = ctx.createLinearGradient(0, 0, 0, depth + PAN_RY);
    metal.addColorStop(0, "#9099c0");
    metal.addColorStop(1, "#3b4068");
    ctx.fillStyle = metal;
    ctx.fill();
    ctx.strokeStyle = OUTLINE;
    ctx.lineWidth = OUTLINE_W;
    ctx.stroke();
    // Rim opening: dark inside, bright accent edge.
    ctx.beginPath();
    ctx.ellipse(0, 0, PAN_R, PAN_RY, 0, 0, Math.PI * 2);
    const inside = ctx.createLinearGradient(0, -PAN_RY, 0, PAN_RY);
    inside.addColorStop(0, "#0c0e1a");
    inside.addColorStop(1, "#3c4270");
    ctx.fillStyle = inside;
    ctx.fill();
    ctx.strokeStyle = OUTLINE;
    ctx.lineWidth = OUTLINE_W + 1.6;
    ctx.stroke();
    ctx.shadowColor = accentCss(pose, 0.9);
    ctx.shadowBlur = 5;
    ctx.strokeStyle = accentCss(pose, 1);
    ctx.lineWidth = 2;
    ctx.stroke();
    ctx.shadowBlur = 0;
    // Highlight on the inside back wall, and a glint on the lower body.
    ctx.beginPath();
    ctx.ellipse(0, 0.3, PAN_R - 3, PAN_RY - 1.9, 0, Math.PI * 1.1, Math.PI * 1.9);
    ctx.strokeStyle = "rgba(255, 255, 255, 0.6)";
    ctx.lineWidth = 1.2;
    ctx.stroke();
    ctx.beginPath();
    ctx.moveTo(-PAN_R * 0.55, depth * 0.8 + 1.5);
    ctx.lineTo(-PAN_R * 0.2, depth + PAN_RY - 2);
    ctx.strokeStyle = "rgba(255, 255, 255, 0.28)";
    ctx.lineWidth = 1.1;
    ctx.stroke();
    // Vibration ticks right after the hit.
    if (frame.vibe.alpha > 0.02) {
      ctx.globalAlpha = w * frame.vibe.alpha;
      ctx.strokeStyle = accentCss(pose, 1);
      ctx.lineWidth = 1.6;
      ctx.beginPath();
      for (const s of vibeSegments(frame.vibe.alpha)) {
        ctx.moveTo(s.from.x, s.from.y - 1);
        ctx.lineTo(s.to.x, s.to.y - 1);
      }
      ctx.stroke();
    }
    ctx.restore();
  }
  ctx.restore();
}

/** Bold star burst, shockwave rings and noise lines at the point where the
 *  rim edges meet, above the head centre. The rings and lines (`back`) sit
 *  behind the pans, the star in front of them. */
function drawImpact(
  ctx: CanvasRenderingContext2D,
  pose: PoseParams,
  frame: Frame,
  layer: "back" | "star",
): void {
  const center = { x: 0, y: PAN_HIT_Y };
  ctx.save();
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.shadowColor = accentCss(pose, 1);
  ctx.shadowBlur = 8;
  // Rings expanding outward and fading.
  ctx.strokeStyle = accentCss(pose, 1);
  for (const ring of layer === "back" ? frame.shock : []) {
    if (ring.alpha < 0.02) continue;
    ctx.globalAlpha = ring.alpha;
    ctx.lineWidth = 1 + 2.4 * ring.alpha;
    ctx.beginPath();
    ctx.arc(center.x, center.y, ring.radius, 0, Math.PI * 2);
    ctx.stroke();
  }
  if (frame.impact >= 0.02) {
    ctx.globalAlpha = Math.min(1, frame.impact * 1.4);
    // Noise lines.
    ctx.lineWidth = 2.4;
    ctx.beginPath();
    for (const seg of layer === "back" ? burstSegments(center, frame.impact, 12) : []) {
      ctx.moveTo(seg.from.x, seg.from.y);
      ctx.lineTo(seg.to.x, seg.to.y);
    }
    ctx.stroke();
    if (layer === "back") {
      ctx.restore();
      return;
    }
    // Star: accent body with a bright core.
    const star = starPoints(center, frame.impact, 8);
    const trace = (pts: Vec[], scale: number) => {
      ctx.beginPath();
      pts.forEach((pt, i) => {
        const x = center.x + (pt.x - center.x) * scale;
        const y = center.y + (pt.y - center.y) * scale;
        if (i === 0) ctx.moveTo(x, y);
        else ctx.lineTo(x, y);
      });
      ctx.closePath();
    };
    trace(star, 1);
    ctx.fillStyle = accentCss(pose, 1);
    ctx.fill();
    ctx.shadowBlur = 0;
    ctx.strokeStyle = OUTLINE;
    ctx.lineWidth = 1.2;
    ctx.stroke();
    trace(star, 0.5);
    ctx.fillStyle = "rgba(255, 245, 252, 0.95)";
    ctx.fill();
  }
  ctx.restore();
}

/** Comic "CLANG!" pop beside the pans, in design space (not tilted with the
 *  head): bold accent letters with a dark outline. It pops at each hit,
 *  scales up and fades, alternating side and a slight rotation. */
function drawClang(ctx: CanvasRenderingContext2D, pose: PoseParams, frame: Frame): void {
  const c = frame.clang;
  if (c.alpha < 0.02) return;
  ctx.save();
  ctx.globalAlpha = c.alpha;
  ctx.translate(ORIGIN.x + c.side * 48, 17);
  ctx.rotate(c.rot);
  ctx.scale(c.scale, c.scale);
  ctx.font = '900 11px "Segoe UI", system-ui, sans-serif';
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.lineJoin = "round";
  ctx.miterLimit = 2;
  ctx.lineWidth = 3.4;
  ctx.strokeStyle = OUTLINE;
  ctx.strokeText("CLANG!", 0, 0);
  ctx.shadowColor = accentCss(pose, 1);
  ctx.shadowBlur = 5;
  ctx.fillStyle = accentCss(pose, 1);
  ctx.fillText("CLANG!", 0, 0);
  ctx.restore();
}

/** Small still "!" for reduced motion: the alert without any movement. */
function drawAlertMark(ctx: CanvasRenderingContext2D, pose: PoseParams, frame: Frame): void {
  if (frame.alertMark < 0.02) return;
  ctx.save();
  ctx.globalAlpha = frame.alertMark;
  ctx.font = '900 16px "Segoe UI", system-ui, sans-serif';
  ctx.textAlign = "center";
  ctx.textBaseline = "middle";
  ctx.lineJoin = "round";
  ctx.lineWidth = 3.2;
  ctx.strokeStyle = OUTLINE;
  ctx.strokeText("!", ORIGIN.x + 50, 22);
  ctx.fillStyle = accentCss(pose, 1);
  ctx.fillText("!", ORIGIN.x + 50, 22);
  ctx.restore();
}

/** The screen faces the robot: its light tints the visor from below. */
function drawSpill(ctx: CanvasRenderingContext2D, pose: PoseParams, weight: number): void {
  const s = LAPTOP.spill;
  const tint = ctx.createLinearGradient(0, s.bottomY, 0, s.topY);
  tint.addColorStop(0, accentCss(pose, 0.85 * clamp01(weight)));
  tint.addColorStop(1, accentCss(pose, 0));
  ctx.save();
  ctx.beginPath();
  ctx.roundRect(-27, -20, 54, 36, 15);
  ctx.clip();
  ctx.fillStyle = tint;
  ctx.fillRect(-s.halfWidth - 1, s.topY, 2 * s.halfWidth + 2, s.bottomY - s.topY);
  ctx.restore();
}

function drawEyes(ctx: CanvasRenderingContext2D, frame: Frame): void {
  const { pose } = frame;
  const arcs = clamp01(pose.eyeArcs);
  const spiral = clamp01(pose.eyeSpiral);
  const capsule = clamp01(1 - arcs - spiral);
  const startle = 1 + 0.2 * frame.impact; // eyes pop wider on each hit
  const w = 9 * pose.eyeWidth * startle;
  const h = Math.max(2.4, 17 * frame.eyeOpen * startle);
  const spiralShape = spiralPoints(2.1, 7.5, 40, 0);

  ctx.fillStyle = accentCss(pose, 1);
  ctx.strokeStyle = accentCss(pose, 1);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.shadowColor = accentCss(pose, 0.9);
  ctx.shadowBlur = pose.glow * 9;

  for (const side of [-1, 1]) {
    ctx.save();
    ctx.translate(side * 12 + frame.gaze.x * MAX_EYE_TRAVEL, -2 + frame.gaze.y * (MAX_EYE_TRAVEL * 0.6));

    if (capsule > 0.01) {
      ctx.save();
      ctx.globalAlpha = capsule;
      ctx.rotate(side * pose.eyeSlant);
      ctx.beginPath();
      ctx.roundRect(-w / 2, -h / 2, w, h, Math.min(w, h) / 2);
      ctx.fill();
      ctx.restore();
    }
    if (arcs > 0.01) {
      // Upturned "^" arc: a happy closed eye.
      ctx.save();
      ctx.globalAlpha = arcs;
      ctx.lineWidth = 3.4;
      ctx.beginPath();
      ctx.moveTo(-5.5, 3);
      ctx.quadraticCurveTo(0, -5.5, 5.5, 3);
      ctx.stroke();
      ctx.restore();
    }
    if (spiral > 0.01) {
      // Dizzy spiral; the two eyes turn in opposite directions.
      ctx.save();
      ctx.globalAlpha = spiral;
      ctx.rotate(side * frame.spiralAngle);
      ctx.lineWidth = 2;
      ctx.beginPath();
      spiralShape.forEach((p, i) => (i === 0 ? ctx.moveTo(p.x, p.y) : ctx.lineTo(p.x, p.y)));
      ctx.stroke();
      ctx.restore();
    }
    ctx.restore();
  }
  ctx.shadowBlur = 0;
}

export function drawRobot(ctx: CanvasRenderingContext2D, frame: Frame, size: number): void {
  const { pose } = frame;
  const left = handShape(handWeights(pose), -1);
  const right = handShape(handWeights(pose), 1);
  ctx.save();
  ctx.clearRect(0, 0, size, size);
  ctx.scale(size / DESIGN_SIZE, size / DESIGN_SIZE);
  ctx.save();
  ctx.translate(ORIGIN.x + frame.offsetX, ORIGIN.y + frame.offsetY);
  ctx.rotate(frame.tilt);
  ctx.scale(frame.scale, frame.scale);

  // Antenna: stem pivots at the top of the head, bulb glows in the accent.
  ctx.save();
  ctx.translate(0, -31);
  ctx.rotate(frame.antennaAngle);
  ctx.strokeStyle = "#7d86a3";
  ctx.lineWidth = 2.4;
  ctx.lineCap = "round";
  ctx.beginPath();
  ctx.moveTo(0, 0);
  ctx.lineTo(0, -14);
  ctx.stroke();
  ctx.shadowColor = accentCss(pose, 1);
  ctx.shadowBlur = 4 + pose.glow * 10;
  ctx.fillStyle = accentCss(pose, 0.55 + pose.glow * 0.45);
  ctx.beginPath();
  ctx.arc(0, -17.5, 4.2, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();

  // Side plates.
  ctx.fillStyle = "#2a2f4a";
  ctx.beginPath();
  ctx.roundRect(-40, -8, 7, 18, 3);
  ctx.roundRect(33, -8, 7, 18, 3);
  ctx.fill();

  // Head.
  const body = ctx.createLinearGradient(0, -32, 0, 28);
  body.addColorStop(0, "#464e73");
  body.addColorStop(1, "#262b47");
  ctx.fillStyle = body;
  ctx.strokeStyle = "rgba(255, 255, 255, 0.14)";
  ctx.lineWidth = 1;
  ctx.beginPath();
  ctx.roundRect(-34, -31, 68, 59, 21);
  ctx.fill();
  ctx.stroke();

  // Visor.
  ctx.fillStyle = "#090b14";
  ctx.beginPath();
  ctx.roundRect(-27, -20, 54, 36, 15);
  ctx.fill();

  if (pose.propLaptop > 0.01) drawSpill(ctx, pose, pose.propLaptop);
  drawEyes(ctx, frame);

  if (pose.propLaptop > 0.01) drawLaptop(ctx, pose, pose.propLaptop);
  drawQuestion(ctx, pose, frame);
  drawWaves(ctx, pose, frame);
  drawImpact(ctx, pose, frame, "back");
  drawPans(ctx, pose, frame);
  drawArm(ctx, SHOULDERS.left, frame.hands.left, -1);
  drawArm(ctx, SHOULDERS.right, frame.hands.right, 1);
  drawHand(ctx, frame.hands.left, left, pose);
  drawHand(ctx, frame.hands.right, right, pose);
  drawImpact(ctx, pose, frame, "star");
  ctx.restore();
  drawClang(ctx, pose, frame);
  drawAlertMark(ctx, pose, frame);
  ctx.restore();
}
