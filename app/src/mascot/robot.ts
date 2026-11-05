// The Niva robot: spring animation state, blink scheduler, gaze maths and the
// Canvas 2D drawing. Everything except `drawRobot` is pure and unit tested.
//
// Original design, drawn from scratch with primitives only (no image assets):
// a floating coral squircle head (a soft rounded square, wider than tall) with a
// large dark glossy screen framed by a glowing ring in the state's accent
// colour, an ear on each side, soft cyan capsule eyes, a mouth that follows the
// state and two floating white mitten hands (no arms). Gestures
// (scratching, thumbs-up, typing on a small laptop), happy arc eyes and dizzy
// spiral eyes are blended by spring weights.

import {
  burstSegments,
  HEAD,
  HAND_SCALE,
  handLayout,
  handShape,
  type HandShape,
  laptopGeometry,
  SCRATCH_TILT,
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
  glow: settled(90),
  breath: settled(60),
  bounceAmp: settled(120),
  bounceHz: settled(120),
  accentR: settled(70),
  accentG: settled(70),
  accentB: settled(70),
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
  mouthSmile: settled(130),
  mouthOpen: settled(130),
  mouthTilted: settled(130),
  mouthFlat: settled(130),
  mouthWavy: settled(130),
  mouthHappy: settled(130),
  mouthFrown: settled(130),
  mouthShout: settled(130),
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

/** The dizzy wobble (error) has a fixed rate; only its amplitude is a spring. */
const WOBBLE_HZ = 2.4;

/** Applies the time-based layers on top of the spring values. `motion` is 1
 *  normally and 0 under prefers-reduced-motion (no breathing, bouncing, sway,
 *  wobble, scratching, typing or spiral rotation; pose transitions,
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
  return {
    pose,
    gaze: {
      x: Math.min(1, Math.max(-1, rig.gazeX.value + pose.lookX)),
      y: Math.min(1, Math.max(-1, rig.gazeY.value + pose.lookY)),
    },
    eyeOpen: pose.eyeOpen * (1 - blink),
    offsetX: pose.headShift + vibe.dx * weight + 0, // + 0 turns -0 into 0
    offsetY: pose.headLift + hop * motion + 4 * hit,
    tilt: pose.headTilt + Math.sin(tau * WOBBLE_HZ * t) * pose.wobbleAmp * motion + 0.08 * hit * clang.side + 0,
    scale: pose.scale * (1 + breath * motion),
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
 *  low so the props, hands and laptop fit; `size` is the canvas side in
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

// --- Head, screen and ears ---------------------------------------------------

/** The screen: an oval about 72% of the head width, so the coral shell shows. */
const SCREEN = { cx: 0, cy: 0, rx: 24.5, ry: 19.5 };
const SHELL_EDGE = "rgba(120, 36, 22, 0.8)";
/** Eyes are always this soft cyan-blue; the ring carries the state colour. */
const EYE_RGB = "125, 220, 255";
const eyeCss = (alpha: number) => `rgba(${EYE_RGB}, ${alpha})`;

function screenPath(ctx: CanvasRenderingContext2D, inset = 0): void {
  ctx.beginPath();
  ctx.ellipse(SCREEN.cx, SCREEN.cy, SCREEN.rx - inset, SCREEN.ry - inset, 0, 0, Math.PI * 2);
}

/** Coral shell shading for the shape just traced: lit from the top left, darker
 *  toward the lower edge; the ears use a slightly darker tone. */
function fillShell(ctx: CanvasRenderingContext2D, r: number, ear: boolean): void {
  const base = ctx.createRadialGradient(-r * 0.32, -r * 0.42, r * 0.08, 0, 0, r * 1.25);
  base.addColorStop(0, ear ? "#ff917a" : "#ffb8a3");
  base.addColorStop(0.5, ear ? "#e5634a" : "#ff7a5c");
  base.addColorStop(1, ear ? "#ae3a25" : "#cc452b");
  ctx.fillStyle = base;
  ctx.fill();
}

/** One ear: a plain rounded rectangle, taller than wide, attached to the side
 *  of the head (drawn behind the shell, so only its outer part shows) in a
 *  slightly darker coral. It never changes colour, and it is part of the head:
 *  it tilts with the face and never moves on its own. */
function drawEar(ctx: CanvasRenderingContext2D, side: -1 | 1): void {
  const w = 10;
  const h = 19;
  ctx.save();
  ctx.translate(side * (HEAD.halfW + 0.5), 3);
  ctx.beginPath();
  ctx.roundRect(-w / 2, -h / 2, w, h, 4.2);
  fillShell(ctx, h / 2, true);
  ctx.strokeStyle = SHELL_EDGE;
  ctx.lineWidth = 0.9;
  ctx.stroke();
  ctx.restore();
}

/** The head shell with its soft highlight, the screen and the accent ring. */
function drawBody(ctx: CanvasRenderingContext2D, pose: PoseParams): void {
  ctx.beginPath();
  ctx.roundRect(-HEAD.halfW, -HEAD.halfH, 2 * HEAD.halfW, 2 * HEAD.halfH, HEAD.radius);
  fillShell(ctx, 36, false);
  ctx.strokeStyle = SHELL_EDGE;
  ctx.lineWidth = 0.9;
  ctx.stroke();

  // Small soft highlight at the top left.
  ctx.save();
  ctx.translate(-18, -24);
  ctx.rotate(-0.6);
  ctx.scale(1, 0.42);
  const hl = ctx.createRadialGradient(0, 0, 0, 0, 0, 8.5);
  hl.addColorStop(0, "rgba(255, 255, 255, 0.95)");
  hl.addColorStop(1, "rgba(255, 255, 255, 0)");
  ctx.fillStyle = hl;
  ctx.beginPath();
  ctx.arc(0, 0, 9.5, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();

  // Screen: near-black glass with a faint sheen across the upper left.
  screenPath(ctx);
  const glass = ctx.createLinearGradient(0, SCREEN.cy - SCREEN.ry, 0, SCREEN.cy + SCREEN.ry);
  glass.addColorStop(0, "#101830");
  glass.addColorStop(1, "#04060d");
  ctx.fillStyle = glass;
  ctx.fill();
  ctx.save();
  screenPath(ctx);
  ctx.clip();
  ctx.translate(-9, -16);
  ctx.rotate(-0.28);
  const sheen = ctx.createLinearGradient(0, -7, 0, 7);
  sheen.addColorStop(0, "rgba(255, 255, 255, 0.16)");
  sheen.addColorStop(1, "rgba(255, 255, 255, 0)");
  ctx.fillStyle = sheen;
  ctx.beginPath();
  ctx.ellipse(0, 0, 20, 7, 0, 0, Math.PI * 2);
  ctx.fill();
  ctx.restore();

  // Ring in the state's accent colour: a wide faint halo, then the bright line.
  // A dark bezel keeps the ring legible on the coloured shell.
  screenPath(ctx, -1.5);
  ctx.lineJoin = "round";
  ctx.strokeStyle = "rgba(6, 9, 18, 0.92)";
  ctx.lineWidth = 4.5;
  ctx.stroke();
  screenPath(ctx, 1);
  ctx.strokeStyle = accentCss(pose, 0.22);
  ctx.lineWidth = 4.5;
  ctx.stroke();
  ctx.shadowColor = accentCss(pose, 0.9);
  ctx.shadowBlur = 3 + pose.glow * 3;
  ctx.strokeStyle = accentCss(pose, 0.85 + 0.15 * pose.glow);
  ctx.lineWidth = 2;
  ctx.stroke();
  ctx.shadowBlur = 0;
}

// --- Hands ----------------------------------------------------------------

/** Smooth egg centred on the origin, narrower at the top (-y). */
function eggPath(ctx: CanvasRenderingContext2D, w: number, h: number): void {
  const hw = w / 2;
  const hh = h / 2;
  ctx.moveTo(0, -hh);
  ctx.bezierCurveTo(hw * 0.8, -hh, hw, -hh * 0.55, hw, hh * 0.05);
  ctx.bezierCurveTo(hw, hh * 0.62, hw * 0.7, hh, 0, hh);
  ctx.bezierCurveTo(-hw * 0.7, hh, -hw, hh * 0.62, -hw, hh * 0.05);
  ctx.bezierCurveTo(-hw, -hh * 0.55, -hw * 0.8, -hh, 0, -hh);
  ctx.closePath();
}

/** A floating mitten: a smooth white egg with no fingers. The head scratch
 *  stretches it into a teardrop whose tip touches the crown, and thumbs-up
 *  grows a short rounded thumb nub on the inner side. */
function drawHand(
  ctx: CanvasRenderingContext2D,
  pos: Vec,
  shape: HandShape,
  pose: PoseParams,
  thumb: number,
): void {
  const k = HAND_SCALE * shape.size;
  // Scratching keeps a small round paddle, shifted toward the crown so its tip
  // reaches it, and tilted about that tip so it rests on the shell.
  const { w, h, cy, point, pivotY } = handLayout(shape);
  const build = () => {
    ctx.beginPath();
    ctx.save();
    ctx.translate(0, cy);
    eggPath(ctx, w, h);
    ctx.restore();
    if (thumb > 0.02) {
      // Short rounded nub rising from the top of the fist, toward the body.
      const len = 10 * thumb;
      ctx.roundRect(-w * 0.4, -shape.bodyH / 2 - len + 4, 7.2, len + 3, 3.6);
    }
  };
  ctx.save();
  ctx.translate(pos.x, pos.y);
  ctx.scale(shape.side * k, k);
  ctx.rotate(-shape.lean);
  if (point > 0) {
    ctx.translate(0, pivotY);
    ctx.rotate(SCRATCH_TILT * point);
    ctx.translate(0, -pivotY);
  }
  ctx.lineJoin = "round";

  // Outlines first: the fill then hides the inner half and any inner contour.
  build();
  ctx.strokeStyle = "rgba(115, 132, 182, 0.8)";
  ctx.lineWidth = 2.4 / k;
  ctx.stroke();
  ctx.strokeStyle = accentCss(pose, 0.6);
  ctx.lineWidth = 1.6 / k;
  ctx.stroke();
  const r = Math.max(w, h) / 2;
  const g = ctx.createRadialGradient(-w * 0.22, cy - h * 0.22, 0.5, 0, cy, r * 1.15);
  g.addColorStop(0, "#ffffff");
  g.addColorStop(0.45, "#f1f4fa");
  g.addColorStop(0.8, "#d3dbec");
  g.addColorStop(1, "#aab7d6");
  ctx.fillStyle = g;
  build();
  ctx.fill();

  // Soft glint.
  ctx.fillStyle = "rgba(255, 255, 255, 0.85)";
  ctx.beginPath();
  ctx.ellipse(-w * 0.17, cy - h * 0.18, w * 0.13, h * 0.1, -0.5, 0, Math.PI * 2);
  ctx.fill();
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
    metal.addColorStop(0, "#e2e7fa");
    metal.addColorStop(1, "#8d97c4");
    ctx.fillStyle = metal;
    ctx.fill();
    ctx.strokeStyle = OUTLINE;
    ctx.lineWidth = OUTLINE_W;
    ctx.stroke();
    // Rim opening: dark inside, bright accent edge.
    ctx.beginPath();
    ctx.ellipse(0, 0, PAN_R, PAN_RY, 0, 0, Math.PI * 2);
    const inside = ctx.createLinearGradient(0, -PAN_RY, 0, PAN_RY);
    inside.addColorStop(0, "#2a3158");
    inside.addColorStop(1, "#6a74a8");
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
  const starC = { x: 0, y: PAN_HIT_Y - 4 };
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
    const star = starPoints(starC, frame.impact, 8);
    const trace = (pts: Vec[], scale: number) => {
      scale *= 0.62;
      ctx.beginPath();
      pts.forEach((pt, i) => {
        const x = starC.x + (pt.x - starC.x) * scale;
        const y = starC.y + (pt.y - starC.y) * scale;
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
  tint.addColorStop(0, accentCss(pose, 0.7 * clamp01(weight)));
  tint.addColorStop(1, accentCss(pose, 0));
  ctx.save();
  screenPath(ctx);
  ctx.clip();
  ctx.fillStyle = tint;
  ctx.fillRect(-s.halfWidth - 1, s.topY, 2 * s.halfWidth + 2, s.bottomY - s.topY);
  ctx.restore();
}

/** Mouth under the eyes, in the eye colour; one shape per `MouthMode`, each
 *  drawn at its spring weight so a state change cross-fades. It follows the
 *  eyes' look offset at a smaller travel. */
function drawMouth(ctx: CanvasRenderingContext2D, frame: Frame): void {
  const { pose } = frame;
  ctx.save();
  ctx.translate(frame.gaze.x * MAX_EYE_TRAVEL * 0.6, 11 + frame.gaze.y * (MAX_EYE_TRAVEL * 0.3));
  ctx.scale(0.8, 0.8);
  ctx.strokeStyle = eyeCss(1);
  ctx.fillStyle = eyeCss(1);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  ctx.lineWidth = 2.4;
  const shapes: Array<[number, () => void]> = [
    [pose.mouthSmile, () => {
      ctx.beginPath();
      ctx.moveTo(-5, -1);
      ctx.quadraticCurveTo(0, 4, 5, -1);
      ctx.stroke();
    }],
    [pose.mouthOpen, () => {
      ctx.beginPath();
      ctx.ellipse(0, 0.5, 3.2, 3.8, 0, 0, Math.PI * 2);
      ctx.fill();
    }],
    [pose.mouthTilted, () => {
      ctx.beginPath();
      ctx.moveTo(-4.5, 2);
      ctx.lineTo(4.5, -1.5);
      ctx.stroke();
    }],
    [pose.mouthFlat, () => {
      ctx.beginPath();
      ctx.moveTo(-4.5, 0.5);
      ctx.lineTo(4.5, 0.5);
      ctx.stroke();
    }],
    [pose.mouthWavy, () => {
      ctx.lineWidth = 2.1;
      ctx.beginPath();
      ctx.moveTo(-6, 0.5);
      ctx.bezierCurveTo(-4.5, -2.5, -2.5, -2.5, -1, 0.5);
      ctx.bezierCurveTo(0.5, 3.5, 2.5, 3.5, 4, 0.5);
      ctx.bezierCurveTo(4.8, -0.8, 5.5, -1, 6, -0.8);
      ctx.stroke();
    }],
    [pose.mouthHappy, () => {
      // Wide open smile: a filled half-moon.
      ctx.beginPath();
      ctx.moveTo(-7, -2);
      ctx.quadraticCurveTo(0, -0.5, 7, -2);
      ctx.quadraticCurveTo(5.5, 7, 0, 7);
      ctx.quadraticCurveTo(-5.5, 7, -7, -2);
      ctx.closePath();
      ctx.fill();
    }],
    [pose.mouthFrown, () => {
      ctx.beginPath();
      ctx.moveTo(-5, 3);
      ctx.quadraticCurveTo(0, -2, 5, 3);
      ctx.stroke();
    }],
    [pose.mouthShout, () => {
      ctx.beginPath();
      ctx.ellipse(0, 1.5, 6, 5.2, 0, 0, Math.PI * 2);
      ctx.fill();
    }],
  ];
  for (const [weight, draw] of shapes) {
    const w = clamp01(weight);
    if (w < 0.02) continue;
    ctx.globalAlpha = w;
    draw();
  }
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
  ctx.strokeStyle = eyeCss(1);
  ctx.lineCap = "round";
  ctx.lineJoin = "round";
  const halo = 0.3 + 0.3 * pose.glow;

  for (const side of [-1, 1]) {
    ctx.save();
    ctx.translate(side * 12 + frame.gaze.x * MAX_EYE_TRAVEL, -2 + frame.gaze.y * (MAX_EYE_TRAVEL * 0.6));

    if (capsule > 0.01) {
      ctx.save();
      ctx.globalAlpha = capsule;
      ctx.rotate(side * pose.eyeSlant);
      // Cheap glow: a wider, faint capsule behind the bright one.
      const gr = w / 2 + 8;
      ctx.save();
      ctx.scale(1, (h / 2 + 8) / gr);
      const glow = ctx.createRadialGradient(0, 0, 0, 0, 0, gr);
      glow.addColorStop(0, eyeCss(halo));
      glow.addColorStop(0.5, eyeCss(halo * 0.45));
      glow.addColorStop(1, eyeCss(0));
      ctx.fillStyle = glow;
      ctx.beginPath();
      ctx.arc(0, 0, gr, 0, Math.PI * 2);
      ctx.fill();
      ctx.restore();
      const lit = ctx.createLinearGradient(0, -h / 2, 0, h / 2);
      lit.addColorStop(0, "#d8f6ff");
      lit.addColorStop(0.55, "#7fdcff");
      lit.addColorStop(1, "#4fb2ff");
      ctx.fillStyle = lit;
      ctx.beginPath();
      ctx.roundRect(-w / 2, -h / 2, w, h, Math.min(w, h) / 2);
      ctx.fill();
      ctx.restore();
    }
    if (arcs > 0.01) {
      // Upturned "^" arc: a happy closed eye.
      ctx.save();
      ctx.globalAlpha = arcs;
      ctx.lineWidth = 3.6;
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

  drawMouth(ctx, frame);
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

  // Ears behind the shell, fixed to the head.
  drawEar(ctx, -1);
  drawEar(ctx, 1);
  drawBody(ctx, pose);

  if (pose.propLaptop > 0.01) drawSpill(ctx, pose, pose.propLaptop);
  drawEyes(ctx, frame);

  if (pose.propLaptop > 0.01) drawLaptop(ctx, pose, pose.propLaptop);
  drawQuestion(ctx, pose, frame);
  drawWaves(ctx, pose, frame);
  drawImpact(ctx, pose, frame, "back");
  drawPans(ctx, pose, frame);
  drawHand(ctx, frame.hands.left, left, pose, 0);
  drawHand(ctx, frame.hands.right, right, pose, clamp01(pose.handThumbs));
  drawImpact(ctx, pose, frame, "star");
  ctx.restore();
  drawClang(ctx, pose, frame);
  drawAlertMark(ctx, pose, frame);
  ctx.restore();
}
