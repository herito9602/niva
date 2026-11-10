// @ts-expect-error node builtin, no @types/node in this project (same as vite.config.ts)
import { readFileSync } from "node:fs";
import { describe, expect, it, vi } from "vitest";
import { LOOKS } from "./looks";
import {
  keepsRunning,
  readReducedMotion,
  REDUCED_QUERY,
  startBarLoop,
  startFrameLoop,
  watchReducedMotion,
  type BarElement,
  type FrameEnv,
  type MediaQueryLike,
} from "./loop";
import { createLevelStore } from "./levelStore";
import { frame } from "./bars";

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

const makeBars = (): BarElement[] =>
  Array.from({ length: 7 }, () => ({ style: { transform: "", opacity: "" } }));

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

describe("startBarLoop", () => {
  const baseOpts = (f: ReturnType<typeof fakeEnv>, bars: BarElement[], reduced = false, look = LOOKS.thinking) => ({
    bars,
    getLook: () => look,
    getStartedAt: () => 0,
    levels: createLevelStore(),
    reduced,
    memory: {},
    env: f.env,
  });

  it("writes transform and opacity to the bar elements every frame", () => {
    const f = fakeEnv();
    const bars = makeBars();
    startBarLoop(baseOpts(f, bars));
    f.step(100);
    for (const b of bars) {
      expect(b.style.transform).toMatch(/^scaleY\(\d\.\d{3}\)$/);
      expect(b.style.opacity).toMatch(/^\d\.\d{3}$/);
    }
    const before = bars.map((b) => b.style.transform).join();
    f.step(100);
    expect(bars.map((b) => b.style.transform).join()).not.toBe(before);
  });

  it("is free of React: frames can only touch element styles", () => {
    const src: string = readFileSync(new URL("./loop.ts", import.meta.url), "utf8");
    expect(src).not.toMatch(/from\s+["']react/);
    expect(src).not.toMatch(/useState|setState|useReducer/);
  });

  it("keeps the smoothing memory across restarts", () => {
    const f = fakeEnv();
    const opts = baseOpts(f, makeBars());
    const stop = startBarLoop(opts);
    f.step();
    const at = opts.memory as { smoothed?: { at: number } };
    expect(at.smoothed?.at).toBeGreaterThan(0);
    stop();
    expect(at.smoothed).toBeDefined();
  });

  it("under reduced motion a non-level look reaches its silhouette and the loop stops", () => {
    const f = fakeEnv();
    const bars = makeBars();
    startBarLoop(baseOpts(f, bars, true, LOOKS.confirmation));
    // First frame is already the silhouette (no previous output to smooth from).
    f.step();
    expect(f.pendingCount()).toBe(0);
    expect(f.request).toHaveBeenCalledTimes(1);
    const sil = LOOKS.confirmation.silhouette;
    expect(bars[2].style.opacity).toBe((0.55 + 0.45 * sil[2]).toFixed(3));
    expect(bars[0].style.transform).toBe(`scaleY(${(0.25 + 0.75 * sil[0]).toFixed(3)})`);
  });

  it("an error look gap (silhouette 0) is written as scaleY(0) under reduced motion", () => {
    const f = fakeEnv();
    const bars = makeBars();
    startBarLoop(baseOpts(f, bars, true, LOOKS.error));
    f.step();
    expect(bars[3].style.transform).toBe("scaleY(0.000)");
    expect(f.pendingCount()).toBe(0);
  });

  it("without reduced motion a non-level look keeps looping", () => {
    const f = fakeEnv();
    startBarLoop(baseOpts(f, makeBars(), false, LOOKS.confirmation));
    for (let i = 0; i < 5; i++) f.step();
    expect(f.pendingCount()).toBe(1);
  });

  it("level looks keep running under reduced motion by design", () => {
    for (const look of [LOOKS.listening, LOOKS.speaking]) {
      const f = fakeEnv();
      startBarLoop(baseOpts(f, makeBars(), true, look));
      // Even once the bars sit on the silhouette (no live level), the loop lives on.
      for (let i = 0; i < 20; i++) f.step();
      expect(f.pendingCount()).toBe(1);
    }
  });
});

describe("keepsRunning", () => {
  const onSilhouette = (look: typeof LOOKS.thinking) => frame(0, look, null, { reduced: true });

  it("stops only for reduced motion + non-level look + settled", () => {
    expect(keepsRunning(true, LOOKS.thinking, onSilhouette(LOOKS.thinking))).toBe(false);
    expect(keepsRunning(false, LOOKS.thinking, onSilhouette(LOOKS.thinking))).toBe(true);
    expect(keepsRunning(true, LOOKS.listening, onSilhouette(LOOKS.listening))).toBe(true);
    // Not yet at the silhouette: keep going until it arrives.
    expect(keepsRunning(true, LOOKS.thinking, onSilhouette(LOOKS.success))).toBe(true);
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
