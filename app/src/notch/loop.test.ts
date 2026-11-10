// @ts-expect-error node builtin, no @types/node in this project (same as vite.config.ts)
import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import { LOOKS } from "./looks";
import {
  keepsRunning,
  readReducedMotion,
  REDUCED_QUERY,
  startFrameLoop,
  startWaveLoop,
  watchReducedMotion,
  type FrameEnv,
  type MediaQueryLike,
} from "./loop";
import { createLevelStore } from "./levelStore";
import { createWave, seededRng, stepWave, type Ctx2D } from "./wave";

/** A manual rAF: callbacks queue up and run only when the test says so. */
function fakeEnv() {
  let clock = 0;
  let nextId = 1;
  const pending = new Map<number, () => void>();
  const request = vi.fn((cb: () => void) => {
    const id = nextId++;
    pending.set(id, cb);
    return id;
  });
  const cancel = vi.fn((id: number) => void pending.delete(id));
  const env: FrameEnv = { request, cancel, now: () => clock };
  /** Runs the queued frame(s), advancing the clock by `dt`. */
  const step = (dt = 16) => {
    clock += dt;
    const cbs = [...pending.values()];
    pending.clear();
    cbs.forEach((cb) => cb());
  };
  return { env, request, cancel, step, pendingCount: () => pending.size };
}

/** A canvas context that records its calls. */
function fakeCtx() {
  const calls: string[] = [];
  const ctx: Ctx2D = {
    fillStyle: "",
    lineJoin: "round",
    miterLimit: 10,
    setTransform: () => void calls.push("setTransform"),
    clearRect: () => void calls.push("clearRect"),
    beginPath: () => void calls.push("beginPath"),
    moveTo: () => void calls.push("moveTo"),
    lineTo: () => void calls.push("lineTo"),
    closePath: () => void calls.push("closePath"),
    fill: () => void calls.push("fill"),
  };
  return { ctx, calls, fills: () => calls.filter((c) => c === "fill").length };
}

describe("startFrameLoop", () => {
  it("runs one step per frame while the step asks to continue", () => {
    const f = fakeEnv();
    const step = vi.fn(() => true);
    startFrameLoop(step, f.env);
    expect(f.pendingCount()).toBe(1);
    f.step();
    f.step();
    f.step();
    expect(step).toHaveBeenCalledTimes(3);
    expect(f.pendingCount()).toBe(1);
  });

  it("stop cancels the pending frame and schedules nothing more", () => {
    const f = fakeEnv();
    const step = vi.fn(() => true);
    const stop = startFrameLoop(step, f.env);
    f.step();
    stop();
    expect(f.cancel).toHaveBeenCalledTimes(1);
    expect(f.pendingCount()).toBe(0);
    f.step();
    expect(step).toHaveBeenCalledTimes(1);
    expect(f.request).toHaveBeenCalledTimes(2);
  });

  it("a stop called from inside a step does not reschedule", () => {
    const f = fakeEnv();
    let stop = () => {};
    stop = startFrameLoop(() => {
      stop();
      return true;
    }, f.env);
    f.step();
    expect(f.pendingCount()).toBe(0);
  });

  it("stops by itself when the step returns false", () => {
    const f = fakeEnv();
    startFrameLoop(() => false, f.env);
    f.step();
    expect(f.pendingCount()).toBe(0);
  });

  it("a restart (hide then show) leaves exactly one live loop", () => {
    const f = fakeEnv();
    const step = vi.fn(() => true);
    const stopFirst = startFrameLoop(step, f.env);
    f.step();
    stopFirst();
    startFrameLoop(step, f.env);
    expect(f.pendingCount()).toBe(1);
    step.mockClear();
    f.step();
    expect(step).toHaveBeenCalledTimes(1);
    expect(f.pendingCount()).toBe(1);
  });
});

describe("startWaveLoop", () => {
  const baseOpts = (f: ReturnType<typeof fakeEnv>, c: ReturnType<typeof fakeCtx>, reduced = false, look = LOOKS.thinking) => ({
    ctx: c.ctx,
    size: () => ({ w: 118, h: 30 }),
    getLook: () => look,
    levels: createLevelStore(),
    reduced,
    rng: seededRng(5),
    memory: {} as { wave?: ReturnType<typeof createWave> },
    env: f.env,
  });

  it("draws the waveform in the look color on every frame", () => {
    const f = fakeEnv();
    const c = fakeCtx();
    startWaveLoop(baseOpts(f, c, false, LOOKS.error));
    f.step(100);
    f.step(100);
    f.step(100);
    expect(c.fills()).toBe(3);
    expect(c.ctx.fillStyle).toBe(LOOKS.error.color);
  });

  it("is free of React: frames can only touch the canvas", () => {
    const src: string = readFileSync(new URL("./loop.ts", import.meta.url), "utf8");
    expect(src).not.toMatch(/from\s+["']react/);
    expect(src).not.toMatch(/useState|setState|useReducer/);
  });

  it("keeps the wave memory across restarts", () => {
    const f = fakeEnv();
    const opts = baseOpts(f, fakeCtx());
    const stop = startWaveLoop(opts);
    f.step();
    expect(opts.memory.wave?.cur).toHaveLength(30);
    expect(opts.memory.wave?.lvl).toBeGreaterThan(0);
    stop();
    expect(opts.memory.wave).toBeDefined();
  });

  it("uses the injected rng, so a seeded run is reproducible", () => {
    const run = () => {
      const f = fakeEnv();
      const opts = baseOpts(f, fakeCtx());
      startWaveLoop(opts);
      for (let i = 0; i < 10; i++) f.step();
      return opts.memory.wave;
    };
    expect(run()).toEqual(run());
  });

  it("reads the level of the look from the store", () => {
    const f = fakeEnv();
    const opts = baseOpts(f, fakeCtx(), false, LOOKS.listening);
    opts.levels.push("mic", Array(7).fill(100), 0);
    startWaveLoop(opts);
    f.step(16);
    // The clock is 16 and the sample is fresh: the smoothed level moved 40% toward 1.
    expect(opts.memory.wave?.lvl).toBeCloseTo(0.4, 9);
  });

  it("under reduced motion a non-level look draws one flat line and the loop stops", () => {
    const f = fakeEnv();
    const c = fakeCtx();
    const opts = baseOpts(f, c, true, LOOKS.confirmation);
    startWaveLoop(opts);
    f.step();
    expect(c.fills()).toBe(1);
    expect(f.pendingCount()).toBe(0);
    expect(f.request).toHaveBeenCalledTimes(1);
    expect(opts.memory.wave?.lvl).toBe(0);
  });

  it("without reduced motion a non-level look keeps looping", () => {
    const f = fakeEnv();
    startWaveLoop(baseOpts(f, fakeCtx(), false, LOOKS.confirmation));
    for (let i = 0; i < 5; i++) f.step();
    expect(f.pendingCount()).toBe(1);
  });

  it("level looks keep running under reduced motion by design", () => {
    for (const look of [LOOKS.listening, LOOKS.speaking]) {
      const f = fakeEnv();
      startWaveLoop(baseOpts(f, fakeCtx(), true, look));
      for (let i = 0; i < 20; i++) f.step();
      expect(f.pendingCount()).toBe(1);
    }
  });
});

describe("keepsRunning", () => {
  const settled = (lvl: number) => ({ ...createWave(), lvl });

  it("stops only for reduced motion + non-level look + level at rest", () => {
    expect(keepsRunning(true, LOOKS.thinking, settled(0))).toBe(false);
    expect(keepsRunning(false, LOOKS.thinking, settled(0))).toBe(true);
    expect(keepsRunning(true, LOOKS.listening, settled(0))).toBe(true);
    // The level is still decaying from the previous look: keep going until it rests.
    expect(keepsRunning(true, LOOKS.thinking, settled(0.5))).toBe(true);
  });

  it("settles after a decay from a loud look", () => {
    const rng = seededRng(1);
    let s = { ...createWave(), lvl: 1 };
    let frames = 0;
    while (keepsRunning(true, LOOKS.thinking, s) && frames < 500) {
      s = stepWave(s, frames * 16, 0, rng, true);
      frames++;
    }
    expect(frames).toBeLessThan(500);
  });
});

describe("reduced motion preference", () => {
  const fakeMedia = (matches: boolean) => {
    const listeners = new Set<() => void>();
    const mq: MediaQueryLike & { matches: boolean } = {
      matches,
      addEventListener: vi.fn((_t, cb) => void listeners.add(cb)),
      removeEventListener: vi.fn((_t, cb) => void listeners.delete(cb)),
    };
    const matchMedia = vi.fn((_q: string) => mq);
    return { mq, matchMedia, fire: () => listeners.forEach((cb) => cb()), listeners };
  };

  it("reads the system preference through the reduced-motion query", () => {
    const m = fakeMedia(true);
    expect(readReducedMotion({ matchMedia: m.matchMedia })).toBe(true);
    expect(m.matchMedia).toHaveBeenCalledWith(REDUCED_QUERY);
    expect(REDUCED_QUERY).toBe("(prefers-reduced-motion: reduce)");
    expect(readReducedMotion({ matchMedia: fakeMedia(false).matchMedia })).toBe(false);
  });

  it("defaults to full motion without a window or matchMedia", () => {
    expect(readReducedMotion(undefined)).toBe(false);
    expect(readReducedMotion({})).toBe(false);
    expect(watchReducedMotion(undefined, () => {})).toBeTypeOf("function");
  });

  it("reports now, follows changes, and unsubscribes", () => {
    const m = fakeMedia(false);
    const seen: boolean[] = [];
    const off = watchReducedMotion({ matchMedia: m.matchMedia }, (r) => seen.push(r));
    expect(seen).toEqual([false]);
    m.mq.matches = true;
    m.fire();
    expect(seen).toEqual([false, true]);
    off();
    expect(m.mq.removeEventListener).toHaveBeenCalledTimes(1);
    expect(m.listeners.size).toBe(0);
    m.fire();
    expect(seen).toEqual([false, true]);
  });
});
