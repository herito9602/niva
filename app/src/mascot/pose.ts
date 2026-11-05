// Target look per assistant state (mascot spec: eight visible states, driven
// only by the state). The springs in `robot.ts` move toward these numbers.

import { ALL_STATES, type StateName, type ViewPayload } from "../events";

export interface PoseParams {
  /** Eye height as a fraction of full height (0 closed .. 1 wide). */
  eyeOpen: number;
  /** Eye width multiplier. */
  eyeWidth: number;
  /** Eye tilt in radians; positive lowers the inner corners (focused). */
  eyeSlant: number;
  /** Head tilt in radians. */
  headTilt: number;
  /** Vertical head offset in drawing units (negative = up). */
  headLift: number;
  scale: number;
  /** Strength of the eye glow. */
  glow: number;
  /** Idle breathing amplitude (fraction of scale). */
  breath: number;
  /** Bounce amplitude in drawing units and its frequency in Hz. */
  bounceAmp: number;
  bounceHz: number;
  accentR: number;
  accentG: number;
  accentB: number;
  /** Horizontal chuckle amplitude (units) and head wobble amplitude (radians). */
  shakeAmp: number;
  wobbleAmp: number;
  /** Gesture, eye-style and prop blend weights (0..1); see `GESTURES`. */
  handScratch: number;
  handThumbs: number;
  handTyping: number;
  handRaise: number;
  eyeArcs: number;
  eyeSpiral: number;
  propLaptop: number;
  handCup: number;
  propQuestion: number;
  propWaves: number;
  /** Sideways head shift in drawing units. */
  headShift: number;
  /** Eye glance added to the cursor gaze (-1..1; negative = left / up). */
  lookX: number;
  lookY: number;
  handPans: number;
  propImpact: number;
  /** Mouth blend weights (0..1), one per `MouthMode`; see `GESTURES`. */
  mouthSmile: number;
  mouthOpen: number;
  mouthTilted: number;
  mouthFlat: number;
  mouthWavy: number;
  mouthHappy: number;
  mouthFrown: number;
  mouthShout: number;
}

export const POSE_KEYS = [
  "eyeOpen",
  "eyeWidth",
  "eyeSlant",
  "headTilt",
  "headLift",
  "scale",
  "glow",
  "breath",
  "bounceAmp",
  "bounceHz",
  "accentR",
  "accentG",
  "accentB",
  "shakeAmp",
  "wobbleAmp",
  "handScratch",
  "handThumbs",
  "handTyping",
  "handRaise",
  "eyeArcs",
  "eyeSpiral",
  "propLaptop",
  "handCup",
  "propQuestion",
  "propWaves",
  "headShift",
  "lookX",
  "lookY",
  "handPans",
  "propImpact",
  "mouthSmile",
  "mouthOpen",
  "mouthTilted",
  "mouthFlat",
  "mouthWavy",
  "mouthHappy",
  "mouthFrown",
  "mouthShout",
] as const satisfies readonly (keyof PoseParams)[];

/** The discrete part of a look (owner feedback 2026-11-03). It is blended as
 *  0/1 weights in the pose so the springs cross-fade between gestures. */
export type HandsMode = "rest" | "scratch" | "thumbsUp" | "typing" | "raised" | "cupEar" | "pans";
export type EyesMode = "capsule" | "happyArcs" | "spiral";
export type MouthMode =
  | "smile"
  | "open"
  | "tilted"
  | "flat"
  | "wavy"
  | "happy"
  | "frown"
  | "shout";
export type PropMode = "none" | "laptop" | "question" | "waves" | "impact";

export interface Gesture {
  hands: HandsMode;
  eyes: EyesMode;
  mouth: MouthMode;
  prop: PropMode;
}

const calm: Gesture = { hands: "rest", eyes: "capsule", mouth: "smile", prop: "none" };

export const GESTURES: Record<StateName, Gesture> = {
  idle: calm,
  listening: { hands: "cupEar", eyes: "capsule", mouth: "open", prop: "waves" },
  thinking: { hands: "scratch", eyes: "capsule", mouth: "tilted", prop: "question" },
  acting: { hands: "typing", eyes: "capsule", mouth: "flat", prop: "laptop" },
  "confirmation-needed": { hands: "raised", eyes: "capsule", mouth: "wavy", prop: "none" },
  success: { hands: "thumbsUp", eyes: "happyArcs", mouth: "happy", prop: "none" },
  error: { hands: "rest", eyes: "spiral", mouth: "frown", prop: "none" },
  "reminder-alert": { hands: "pans", eyes: "capsule", mouth: "shout", prop: "impact" },
};

type OptionalKey = "shakeAmp" | "wobbleAmp" | "headShift" | "lookX" | "lookY";

type NumericPose = Omit<
  PoseParams,
  | "handScratch"
  | "handThumbs"
  | "handTyping"
  | "handRaise"
  | "eyeArcs"
  | "eyeSpiral"
  | "propLaptop"
  | "handCup"
  | "propQuestion"
  | "propWaves"
  | "handPans"
  | "propImpact"
  | "mouthSmile"
  | "mouthOpen"
  | "mouthTilted"
  | "mouthFlat"
  | "mouthWavy"
  | "mouthHappy"
  | "mouthFrown"
  | "mouthShout"
  | OptionalKey
> &
  Partial<Pick<PoseParams, OptionalKey>>;

function withGesture(base: NumericPose, g: Gesture): PoseParams {
  return {
    shakeAmp: 0,
    wobbleAmp: 0,
    headShift: 0,
    lookX: 0,
    lookY: 0,
    ...base,
    handScratch: g.hands === "scratch" ? 1 : 0,
    handThumbs: g.hands === "thumbsUp" ? 1 : 0,
    handTyping: g.hands === "typing" ? 1 : 0,
    handRaise: g.hands === "raised" ? 1 : 0,
    eyeArcs: g.eyes === "happyArcs" ? 1 : 0,
    eyeSpiral: g.eyes === "spiral" ? 1 : 0,
    propLaptop: g.prop === "laptop" ? 1 : 0,
    handCup: g.hands === "cupEar" ? 1 : 0,
    propQuestion: g.prop === "question" ? 1 : 0,
    propWaves: g.prop === "waves" ? 1 : 0,
    handPans: g.hands === "pans" ? 1 : 0,
    propImpact: g.prop === "impact" ? 1 : 0,
    mouthSmile: g.mouth === "smile" ? 1 : 0,
    mouthOpen: g.mouth === "open" ? 1 : 0,
    mouthTilted: g.mouth === "tilted" ? 1 : 0,
    mouthFlat: g.mouth === "flat" ? 1 : 0,
    mouthWavy: g.mouth === "wavy" ? 1 : 0,
    mouthHappy: g.mouth === "happy" ? 1 : 0,
    mouthFrown: g.mouth === "frown" ? 1 : 0,
    mouthShout: g.mouth === "shout" ? 1 : 0,
  };
}

const BASE_POSES: Record<StateName, NumericPose> = {
  // Calm, half-lidded, cool cyan; breathes.
  idle: {
    eyeOpen: 0.85, eyeWidth: 1, eyeSlant: 0, headTilt: 0, headLift: 0, scale: 1,
    glow: 0.35, breath: 0.025,
    bounceAmp: 0, bounceHz: 0, accentR: 90, accentG: 205, accentB: 255,
  },
  // Wide attentive eyes, head leaning in, teal.
  listening: {
    eyeOpen: 1, eyeWidth: 1.15, eyeSlant: 0, headTilt: 0.18, headLift: -2, scale: 1.05,
    glow: 0.8, breath: 0.015,
    bounceAmp: 0, bounceHz: 0, accentR: 70, accentG: 235, accentB: 190,
    headShift: 2, lookX: -0.5, lookY: -0.3,
  },
  // Narrowed eyes, head cocked, violet.
  thinking: {
    eyeOpen: 0.5, eyeWidth: 0.95, eyeSlant: 0.2, headTilt: -0.14, headLift: 0, scale: 1,
    glow: 0.6, breath: 0.02,
    bounceAmp: 0, bounceHz: 0, accentR: 181, accentG: 123, accentB: 255,
    lookX: -0.6, lookY: -0.7,
  },
  // Focused, quick small bounce, blue.
  acting: {
    eyeOpen: 0.75, eyeWidth: 1, eyeSlant: 0.3, headTilt: 0, headLift: 0, scale: 1,
    glow: 0.7, breath: 0.01,
    bounceAmp: 2, bounceHz: 3.2, accentR: 70, accentG: 140, accentB: 255,
  },
  // Round questioning eyes, head tilted, gold.
  "confirmation-needed": {
    eyeOpen: 1, eyeWidth: 0.9, eyeSlant: -0.3, headTilt: 0.2, headLift: -1, scale: 1.03,
    glow: 0.85, breath: 0.015,
    bounceAmp: 1, bounceHz: 1.2, accentR: 255, accentG: 205, accentB: 70,
  },
  // Happy squint, big hop, green.
  success: {
    eyeOpen: 0.3, eyeWidth: 1.2, eyeSlant: 0, headTilt: -0.04, headLift: -3, scale: 1.07,
    glow: 1, breath: 0.01,
    bounceAmp: 4, bounceHz: 2.4, accentR: 110, accentG: 230, accentB: 100, shakeAmp: 1,
  },
  // Droopy: eyes sad-slanted, head sunk, red.
  error: {
    eyeOpen: 0.7, eyeWidth: 0.85, eyeSlant: -0.45, headTilt: -0.22, headLift: 3, scale: 0.95,
    glow: 0.5, breath: 0.01,
    bounceAmp: 0, bounceHz: 0, accentR: 255, accentG: 85, accentB: 85, wobbleAmp: 0.07,
  },
  // Alarm: wide startled eyes, magenta. The head sits low so both pans fit
  // above it. The hit
  // itself kicks the head, so no constant bounce.
  "reminder-alert": {
    eyeOpen: 1, eyeWidth: 1.3, eyeSlant: -0.1, headTilt: 0, headLift: 14, scale: 1,
    glow: 1, breath: 0,
    bounceAmp: 0, bounceHz: 0, accentR: 255, accentG: 80, accentB: 200,
  },
};

export const POSES = Object.fromEntries(
  ALL_STATES.map((s) => [s, withGesture(BASE_POSES[s], GESTURES[s])]),
) as Record<StateName, PoseParams>;

/** The mascot follows the view state 1:1. While an AskFollowUp question is
 *  spoken the host still reports `thinking`, so the thinking pose stays. */
export function poseForView(view: ViewPayload | null): PoseParams {
  return view ? POSES[view.state] : POSES.idle;
}
