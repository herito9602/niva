// The notch's bar heights. `frame` is pure: it takes the clock, the look, the
// latest level and the previous output, and returns the next output. The
// animation loop in Notch.tsx owns the previous value and writes the result to
// the DOM through refs.

import type { LevelSample } from "./levelStore";
import type { Look } from "./looks";

export const BAR_COUNT = 7;
/** Time constants of the smoothing: bars rise fast and fall slowly. */
export const ATTACK_TAU_MS = 25;
export const RELEASE_TAU_MS = 110;
/** A real level older than this is stale and the simulation takes over. */
export const LEVEL_STALE_MS = 150;
/** Period of the thinking wave's underlying sine. */
export const WAVE_PERIOD_MS = 1200;

export interface Smoothed {
  /** Clock reading of this output. */
  at: number;
  bars: number[];
}

export interface FrameOptions {
  prev?: Smoothed;
  /** The system asks for reduced motion: static silhouettes, no simulation. */
  reduced?: boolean;
  /** When the current look started (the success bounce is relative to it). */
  startedAt?: number;
}

/** Floor of a live bar, so a quiet bar stays a visible dot (0 is reserved for a gap). */
const LIVE_FLOOR = 0.04;
/** Voice-like weight of each band in the simulation: the middle bands carry more. */
const VOICE_WEIGHTS = [0.7, 0.92, 1, 0.85, 0.62, 0.42, 0.28];
const TAU = 2 * Math.PI;

const clamp01 = (v: number) => Math.min(1, Math.max(0, v));

/** Syllable-like bursts with gaps: the simulated voice envelope (0..1). */
function voiceEnvelope(t: number): number {
  const word = Math.max(0, Math.sin(t * 5.3));
  const syllable = 0.62 + 0.38 * Math.sin(t * 21);
  const phrase = Math.max(0, Math.sin(t * 0.9 + 0.4) * 1.6 + 0.35);
  return clamp01(Math.pow(word, 0.7) * syllable * Math.min(1, phrase));
}

function simulateVoice(nowMs: number): number[] {
  const t = nowMs / 1000;
  const env = voiceEnvelope(t);
  return VOICE_WEIGHTS.map((w, i) => {
    const wobble = 0.5 + 0.5 * Math.sin(t * (9 + i * 1.7) + i * 2.1);
    return clamp01(0.1 + 0.9 * env * w * (0.55 + 0.45 * wobble));
  });
}

function targetFor(nowMs: number, look: Look, level: LevelSample | null, opts: FrameOptions): number[] {
  const t = nowMs / 1000;
  switch (look.motion) {
    case "levels": {
      const usable = level && level.src === look.source && nowMs - level.at <= LEVEL_STALE_MS;
      if (usable) return level.bands.map((b) => Math.max(LIVE_FLOOR, b / 100));
      return opts.reduced ? [...look.silhouette] : simulateVoice(nowMs);
    }
    case "wave": {
      const w = TAU / WAVE_PERIOD_MS;
      return Array.from({ length: BAR_COUNT }, (_, i) => 0.15 + 0.85 * Math.sin(nowMs * w - i * 0.8) ** 2);
    }
    case "pulse": {
      const v = 0.3 + 0.4 * (0.5 + 0.5 * Math.sin(t * 3.2));
      return Array(BAR_COUNT).fill(v);
    }
    case "bounce": {
      const since = nowMs - (opts.startedAt ?? nowMs);
      return Array.from({ length: BAR_COUNT }, (_, i) => {
        const s = clamp01((since - i * 30) / 600);
        return 0.7 + 0.3 * Math.sin(s * Math.PI);
      });
    }
    case "jitter":
      return Array.from({ length: BAR_COUNT }, (_, i) => 0.16 + 0.1 * (0.5 + 0.5 * Math.sin(t * 37 + i * 2.3)));
    case "outward":
      return Array.from({ length: BAR_COUNT }, (_, i) => {
        const d = Math.abs(i - 3);
        return 0.2 + 0.8 * (0.5 + 0.5 * Math.sin(t * 6 - d * 1.1));
      });
  }
}

/**
 * Bar heights (0..1) for `nowMs`. Fresh real levels drive the listening and
 * speaking looks; without them (or when stale) a simulation does, except under
 * reduced motion, where every look is its static silhouette. Smoothing is
 * asymmetric: `ATTACK_TAU_MS` going up, `RELEASE_TAU_MS` going down.
 */
export function frame(
  nowMs: number,
  look: Look,
  level: LevelSample | null,
  opts: FrameOptions = {},
): Smoothed {
  const reducedStatic = opts.reduced && look.motion !== "levels";
  const target = reducedStatic ? [...look.silhouette] : targetFor(nowMs, look, level, opts);
  const prev = opts.prev;
  if (!prev) return { at: nowMs, bars: target };
  const dt = Math.min(1000, Math.max(0, nowMs - prev.at));
  const bars = target.map((goal, i) => {
    const from = prev.bars[i];
    const tau = goal > from ? ATTACK_TAU_MS : RELEASE_TAU_MS;
    return clamp01(from + (goal - from) * (1 - Math.exp(-dt / tau)));
  });
  return { at: nowMs, bars };
}
