// The notch's animation loop, free of React so it can be tested with a fake
// requestAnimationFrame. Frames only write to the bar elements' style: nothing
// here can trigger a React render.

import { frame, type Smoothed } from "./bars";
import type { LevelStore } from "./levelStore";
import type { Look } from "./looks";

/** A bar at level 0 is a dot this fraction of the full bar; exactly 0 is a gap. */
const MIN_SCALE = 0.25;
const GAP_BELOW = 0.01;
export const REDUCED_QUERY = "(prefers-reduced-motion: reduce)";

export const barScale = (v: number) => (v < GAP_BELOW ? 0 : MIN_SCALE + (1 - MIN_SCALE) * v);

export interface FrameEnv {
  request: (cb: () => void) => number;
  cancel: (handle: number) => void;
  now: () => number;
}

/**
 * Calls `step` once per frame while it returns true. The returned function
 * stops the loop: it cancels the pending frame and never schedules another,
 * even if a step is still running.
 */
export function startFrameLoop(step: (now: number) => boolean, env: FrameEnv): () => void {
  let stopped = false;
  let handle = 0;
  const tick = () => {
    if (stopped) return;
    if (step(env.now()) && !stopped) handle = env.request(tick);
  };
  handle = env.request(tick);
  return () => {
    stopped = true;
    env.cancel(handle);
  };
}

/** The smoothed bars have reached the silhouette (within a hair). */
export function settled(s: Smoothed, look: Look): boolean {
  return s.bars.every((v, i) => Math.abs(v - look.silhouette[i]) < 0.002);
}

/**
 * Whether the loop must keep running after this frame. A static silhouette
 * (reduced motion, non-level look) needs no loop once reached. The level looks
 * (listening, speaking) keep running under reduced motion on purpose: they show
 * the real voice level, which is information rather than decoration.
 */
export function keepsRunning(reduced: boolean, look: Look, next: Smoothed): boolean {
  return !(reduced && look.motion !== "levels" && settled(next, look));
}

export interface BarElement {
  style: { transform: string; opacity: string };
}

export interface BarLoopOptions {
  bars: readonly BarElement[];
  getLook: () => Look;
  getStartedAt: () => number;
  levels: LevelStore;
  reduced: boolean;
  /** Smoothing memory that survives loop restarts. */
  memory: { smoothed?: Smoothed };
  env: FrameEnv;
}

/** Starts the bar loop and returns its stop function. */
export function startBarLoop(o: BarLoopOptions): () => void {
  return startFrameLoop((now) => {
    const look = o.getLook();
    const next = frame(now, look, o.levels.forLook(look), {
      prev: o.memory.smoothed,
      reduced: o.reduced,
      startedAt: o.getStartedAt(),
    });
    o.memory.smoothed = next;
    next.bars.forEach((v, i) => {
      const el = o.bars[i];
      if (!el) return;
      el.style.transform = `scaleY(${barScale(v).toFixed(3)})`;
      el.style.opacity = (0.55 + 0.45 * v).toFixed(3);
    });
    return keepsRunning(o.reduced, look, next);
  }, o.env);
}

export interface MediaQueryLike {
  matches: boolean;
  addEventListener: (type: "change", cb: () => void) => void;
  removeEventListener: (type: "change", cb: () => void) => void;
}

export interface WindowLike {
  matchMedia?: (query: string) => MediaQueryLike;
}

/** Current reduced-motion preference; false when matchMedia is unavailable. */
export function readReducedMotion(win: WindowLike | undefined): boolean {
  return win?.matchMedia?.(REDUCED_QUERY).matches ?? false;
}

/** Calls `onChange(reduced)` now and on every preference change; returns the unsubscribe. */
export function watchReducedMotion(win: WindowLike | undefined, onChange: (reduced: boolean) => void): () => void {
  const mq = win?.matchMedia?.(REDUCED_QUERY);
  if (!mq) return () => {};
  const handler = () => onChange(mq.matches);
  handler();
  mq.addEventListener("change", handler);
  return () => mq.removeEventListener("change", handler);
}
