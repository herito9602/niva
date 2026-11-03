import { describe, expect, it } from "vitest";
import { criticalDamping, stepSpring, type SpringConfig, type SpringState } from "./spring";

const run = (from: SpringState, target: number, cfg: SpringConfig, seconds: number, dt: number) => {
  let s = from;
  let peak = s.value;
  for (let t = 0; t < seconds; t += dt) {
    s = stepSpring(s, target, cfg, dt);
    peak = Math.max(peak, s.value);
  }
  return { s, peak };
};

const bouncy: SpringConfig = { stiffness: 220, damping: 14 };

describe("stepSpring", () => {
  it("converges to the target at rest", () => {
    const { s } = run({ value: 0, velocity: 0 }, 1, bouncy, 3, 1 / 60);
    expect(s.value).toBeCloseTo(1, 3);
    expect(Math.abs(s.velocity)).toBeLessThan(1e-3);
  });

  it("an underdamped spring overshoots but stays bounded", () => {
    const { peak } = run({ value: 0, velocity: 0 }, 1, bouncy, 3, 1 / 60);
    expect(peak).toBeGreaterThan(1);
    expect(peak).toBeLessThan(1.5);
  });

  it("a critically damped spring does not overshoot", () => {
    const cfg = { stiffness: 220, damping: criticalDamping(220) };
    const { peak, s } = run({ value: 0, velocity: 0 }, 1, cfg, 3, 1 / 60);
    expect(peak).toBeLessThanOrEqual(1 + 1e-6);
    expect(s.value).toBeCloseTo(1, 3);
  });

  it("stays at rest on the target", () => {
    const s = stepSpring({ value: 2, velocity: 0 }, 2, bouncy, 1 / 60);
    expect(s).toEqual({ value: 2, velocity: 0 });
  });

  it("is stable with a huge dt (tab was hidden)", () => {
    const s = stepSpring({ value: 0, velocity: 0 }, 1, { stiffness: 900, damping: 30 }, 60);
    expect(Number.isFinite(s.value)).toBe(true);
    expect(Math.abs(s.value)).toBeLessThan(2);
  });

  it("ignores zero, negative and non-finite dt", () => {
    const start = { value: 0.3, velocity: 1 };
    expect(stepSpring(start, 1, bouncy, 0)).toEqual(start);
    expect(stepSpring(start, 1, bouncy, -1)).toEqual(start);
    expect(stepSpring(start, 1, bouncy, Number.NaN)).toEqual(start);
  });

  it("ignores a non-finite target", () => {
    const start = { value: 0.3, velocity: 1 };
    for (const bad of [Number.NaN, Infinity, -Infinity]) {
      expect(stepSpring(start, bad, bouncy, 1 / 60)).toEqual(start);
    }
  });

  it("gives similar results at 60 Hz and 144 Hz", () => {
    const a = run({ value: 0, velocity: 0 }, 1, bouncy, 0.5, 1 / 60).s.value;
    const b = run({ value: 0, velocity: 0 }, 1, bouncy, 0.5, 1 / 144).s.value;
    expect(Math.abs(a - b)).toBeLessThan(0.03);
  });
});
