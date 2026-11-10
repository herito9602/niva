// The notch's waveform: one solid shape mirrored around the horizontal center
// line, whose top edge is a polyline of WAVE_POINTS points. Everything here is
// pure or takes its collaborators (clock, rng, canvas context) as arguments, so
// vitest can drive it with a seeded rng and a recording context. The loop in
// loop.ts owns the state between frames.

import type { LevelSample } from "./levelStore";
import type { Look } from "./looks";

/** Points on the top edge. */
export const WAVE_POINTS = 30;
/** New random targets this often. */
export const RETARGET_MS = 70;
/** Per frame: how far a point moves toward its target. */
export const FOLLOW = 0.45;
/** Per frame level smoothing: fast up, slow down. */
export const LEVEL_ATTACK = 0.4;
export const LEVEL_RELEASE = 0.1;
/** Spike height multiplier: amp = 1 + level * AMP_GAIN. */
export const AMP_GAIN = 24;
/** Silence is a thin line of this half-height (drawing units). */
export const MIN_HALF = 0.6;
/** Keeps a spike this far from the edge of the drawing space. */
const EDGE_MARGIN = 1.5;
/** Level of the states without audio: they look alive but calm. */
export const BREATH_LEVEL = 0.05;
/** A real level older than this is stale and the simulated voice takes over. */
export const LEVEL_STALE_MS = 150;
/** The drawing space; the canvas scales it to its pixel size. */
export const SPACE_W = 236;
export const SPACE_H = 60;
/** Where the spikes start and end, and where the flat tails end (fractions of the width). */
const SPAN_FROM = 0.07;
const SPAN_TO = 0.93;
const TAIL_FROM = 0.028;
const TAIL_TO = 0.972;
/** Height of the static shape under reduced motion, before the level scales it. */
const STATIC_SHAPE = 0.6;
/** Below this the level is at rest and a reduced-motion loop may stop. */
const REST_LEVEL = 0.001;

export type Rng = () => number;

export interface WaveState {
  /** Current relative height of each point. */
  cur: number[];
  /** Where each point is heading. */
  tgt: number[];
  /** Smoothed level, 0..1. */
  lvl: number;
  /** Clock reading of the next re-target. */
  retargetAt: number;
}

/** Small seeded generator (mulberry32) for deterministic tests. */
export function seededRng(seed: number): Rng {
  let a = seed >>> 0;
  return () => {
    a = (a + 0x6d2b79f5) >>> 0;
    let t = a;
    t = Math.imul(t ^ (t >>> 15), t | 1);
    t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
    return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
  };
}

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));
const uOf = (i: number) => i / (WAVE_POINTS - 1);

/** Bell centered on the notch: the middle spikes are the tallest. */
export const envelope = (u: number) => Math.exp(-(((u - 0.5) / 0.22) ** 2));

export function createWave(): WaveState {
  return {
    cur: Array(WAVE_POINTS).fill(0),
    tgt: Array(WAVE_POINTS).fill(0),
    lvl: 0,
    retargetAt: Number.NEGATIVE_INFINITY,
  };
}

/** Whether `stepWave` would pick new targets at `nowMs`. */
export const retargetsDue = (s: WaveState, nowMs: number) => nowMs >= s.retargetAt;

/** Syllable-like bursts with gaps: the simulated voice envelope (0..1). */
export function voiceEnvelope(nowMs: number): number {
  const t = nowMs / 1000;
  const word = Math.max(0, Math.sin(t * 5.3));
  const syllable = 0.62 + 0.38 * Math.sin(t * 21);
  const phrase = Math.max(0, Math.sin(t * 0.9 + 0.4) * 1.6 + 0.35);
  return clamp01(Math.pow(word, 0.7) * syllable * Math.min(1, phrase));
}

/**
 * The level the shape should reach (0..1). Speaking follows a fresh real level
 * of Niva's voice (mean of the bands), else the simulated voice. Every other look,
 * listening included, has no audio and breathes. Under reduced motion nothing is simulated
 * and nothing breathes, but a real level still counts: it is information.
 */
export function targetLevel(look: Look, sample: LevelSample | null, nowMs: number, reduced: boolean): number {
  if (!look.source) return reduced ? 0 : BREATH_LEVEL;
  if (sample && sample.src === look.source && nowMs - sample.at <= LEVEL_STALE_MS) {
    return clamp01(sample.bands.reduce((a, b) => a + b, 0) / sample.bands.length / 100);
  }
  return reduced ? 0 : voiceEnvelope(nowMs);
}

/**
 * The next state. Full motion: every RETARGET_MS new random targets, every frame
 * each point closes 45% of the gap. Reduced motion: no randomness, the shape is
 * the static envelope. Either way the level is smoothed (fast up, slow down).
 */
export function stepWave(s: WaveState, nowMs: number, level: number, rng: Rng, reduced: boolean): WaveState {
  const lvl = s.lvl + (level - s.lvl) * (level > s.lvl ? LEVEL_ATTACK : LEVEL_RELEASE);
  if (reduced) {
    const still = s.cur.map((_, i) => envelope(uOf(i)) * STATIC_SHAPE);
    return { cur: still, tgt: still, lvl, retargetAt: s.retargetAt };
  }
  let tgt = s.tgt;
  let retargetAt = s.retargetAt;
  if (retargetsDue(s, nowMs)) {
    tgt = s.tgt.map((_, i) => {
      const r = rng();
      return envelope(uOf(i)) * (0.2 + 0.8 * r * r * 1.3);
    });
    retargetAt = nowMs + RETARGET_MS;
  }
  const cur = s.cur.map((c, i) => c + (tgt[i] - c) * FOLLOW);
  return { cur, tgt, lvl, retargetAt };
}

/** Half-height of the shape at each point, in drawing units. */
export function halfHeights(s: WaveState): number[] {
  const amp = 1 + s.lvl * AMP_GAIN;
  const max = SPACE_H / 2 - EDGE_MARGIN;
  return s.cur.map((c) => Math.min(max, Math.max(MIN_HALF, c * amp)));
}

export interface Point {
  x: number;
  y: number;
}

/**
 * The closed outline, in drawing units: left tail, the top edge left to right,
 * right tail, then the mirrored bottom edge right to left.
 */
export function outline(s: WaveState): Point[] {
  const cy = SPACE_H / 2;
  const h = halfHeights(s);
  const xs = h.map((_, i) => (SPAN_FROM + (SPAN_TO - SPAN_FROM) * uOf(i)) * SPACE_W);
  return [
    { x: TAIL_FROM * SPACE_W, y: cy },
    ...h.map((v, i) => ({ x: xs[i], y: cy - v })),
    { x: TAIL_TO * SPACE_W, y: cy },
    ...h.map((_, i) => ({ x: xs[h.length - 1 - i], y: cy + h[h.length - 1 - i] })),
  ];
}

/** The slice of CanvasRenderingContext2D the waveform needs. */
export interface Ctx2D {
  fillStyle: string | CanvasGradient | CanvasPattern;
  lineJoin: CanvasLineJoin;
  miterLimit: number;
  setTransform(a: number, b: number, c: number, d: number, e: number, f: number): void;
  clearRect(x: number, y: number, w: number, h: number): void;
  beginPath(): void;
  moveTo(x: number, y: number): void;
  lineTo(x: number, y: number): void;
  closePath(): void;
  fill(): void;
}

/** Draws the waveform into a canvas of `pxW` x `pxH` device pixels. */
export function drawWave(ctx: Ctx2D, s: WaveState, color: string, pxW: number, pxH: number): void {
  ctx.setTransform(pxW / SPACE_W, 0, 0, pxH / SPACE_H, 0, 0);
  ctx.clearRect(0, 0, SPACE_W, SPACE_H);
  const [first, ...rest] = outline(s);
  ctx.beginPath();
  ctx.moveTo(first.x, first.y);
  for (const p of rest) ctx.lineTo(p.x, p.y);
  ctx.closePath();
  ctx.fillStyle = color;
  ctx.lineJoin = "miter";
  ctx.miterLimit = 3;
  ctx.fill();
}

export interface CanvasSize {
  width: number;
  height: number;
}

/**
 * Sizes the canvas backing store for the device pixel ratio (CSS size stays
 * `cssW` x `cssH`) and returns it in device pixels.
 */
export function fitCanvas(canvas: CanvasSize, cssW: number, cssH: number, dpr: number): { w: number; h: number } {
  const ratio = dpr > 0 ? dpr : 1;
  const w = Math.round(cssW * ratio);
  const h = Math.round(cssH * ratio);
  if (canvas.width !== w) canvas.width = w;
  if (canvas.height !== h) canvas.height = h;
  return { w, h };
}

/** Whether a reduced-motion loop has nothing left to draw once this state is on screen. */
export const atRest = (s: WaveState) => s.lvl < REST_LEVEL;
