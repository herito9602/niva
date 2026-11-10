// The notch's animation loop, free of React so it can be tested with a fake
// requestAnimationFrame. Frames only draw on the canvas: nothing here can
// trigger a React render.

import type { LevelStore } from "./levelStore";
import type { Look } from "./looks";
import { atRest, createWave, drawWave, stepWave, targetLevel, type Ctx2D, type Rng, type WaveState } from "./wave";

export const REDUCED_QUERY = "(prefers-reduced-motion: reduce)";

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

/**
 * Whether the loop must keep running after this frame. A static line (reduced
 * motion, look without audio) needs no loop once the level is at rest. The level
 * look (speaking) keeps running under reduced motion on purpose: it
 * shows the real voice level, which is information rather than decoration.
 */
export function keepsRunning(reduced: boolean, look: Look, next: WaveState): boolean {
  return !(reduced && look.source === null && atRest(next));
}

export interface WaveLoopOptions {
  ctx: Ctx2D;
  /** Canvas size in device pixels, read every frame (it follows the DPR). */
  size: () => { w: number; h: number };
  getLook: () => Look;
  levels: LevelStore;
  reduced: boolean;
  /** Randomness of the spikes; `Math.random` in production, seeded in tests. */
  rng: Rng;
  /** Wave state that survives loop restarts. */
  memory: { wave?: WaveState };
  env: FrameEnv;
}

/** Starts the waveform loop and returns its stop function. */
export function startWaveLoop(o: WaveLoopOptions): () => void {
  return startFrameLoop((now) => {
    const look = o.getLook();
    const level = targetLevel(look, o.levels.forLook(look), now, o.reduced);
    const next = stepWave(o.memory.wave ?? createWave(), now, level, o.rng, o.reduced);
    o.memory.wave = next;
    const { w, h } = o.size();
    drawWave(o.ctx, next, look.color, w, h);
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

export interface DisplayWindowLike {
  devicePixelRatio: number;
  addEventListener: (type: "resize", cb: () => void) => void;
  removeEventListener: (type: "resize", cb: () => void) => void;
  matchMedia?: (query: string) => Pick<MediaQueryLike, "addEventListener" | "removeEventListener">;
}

/**
 * Calls `onChange` when the window is resized or the device pixel ratio changes
 * (for example when the window moves to another monitor). The DPR has no event
 * of its own: a `(resolution: Ndppx)` query fires once when the ratio leaves N,
 * so it is re-armed for the new ratio each time. Returns the unsubscribe.
 */
export function watchDisplayMetrics(win: DisplayWindowLike | undefined, onChange: () => void): () => void {
  if (!win) return () => {};
  win.addEventListener("resize", onChange);
  let disarm = () => {};
  let closed = false;
  const arm = () => {
    const mq = win.matchMedia?.(`(resolution: ${win.devicePixelRatio}dppx)`);
    if (!mq) return;
    const handler = () => {
      mq.removeEventListener("change", handler);
      if (closed) return;
      onChange();
      arm();
    };
    mq.addEventListener("change", handler);
    disarm = () => mq.removeEventListener("change", handler);
  };
  arm();
  return () => {
    closed = true;
    win.removeEventListener("resize", onChange);
    disarm();
  };
}

/**
 * Draws the remembered wave once, re-fitting the canvas first. For when the
 * loop is stopped (static line) and the canvas size changed under it.
 */
export function redrawWave(o: Pick<WaveLoopOptions, "ctx" | "size" | "getLook" | "memory">): void {
  const { w, h } = o.size();
  drawWave(o.ctx, o.memory.wave ?? createWave(), o.getLook().color, w, h);
}
