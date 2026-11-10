import { describe, expect, it } from "vitest";
import { ATTACK_TAU_MS, BAR_COUNT, LEVEL_STALE_MS, RELEASE_TAU_MS, frame, type Smoothed } from "./bars";
import { LOOKS } from "./looks";
import type { LevelSample } from "./levelStore";

const flat = (v: number, at: number): LevelSample => ({ src: "mic", bands: Array(7).fill(v), at });
const flatTts = (v: number, at: number): LevelSample => ({ src: "tts", bands: Array(7).fill(v), at });
const spread = (bars: number[]) => Math.max(...bars) - Math.min(...bars);

/** Runs `frame` on a 60 Hz fake clock; `levelAt` supplies the level of each tick. */
function run(from: number, to: number, levelAt: (t: number) => LevelSample | null) {
  const out: { t: number; bars: number[] }[] = [];
  let prev: Smoothed | undefined;
  for (let t = from; t <= to; t += 1000 / 60) {
    prev = frame(t, LOOKS.listening, levelAt(t), { prev });
    out.push({ t, bars: prev.bars });
  }
  return out;
}

/** Time after `stepAt` at which bar 3 has covered half of its way to the new level. */
function halfwayLag(ticks: { t: number; bars: number[] }[], stepAt: number, rising: boolean): number {
  const settled = ticks.filter((k) => k.t < stepAt).at(-1)!.bars[3];
  const target = rising ? 1 : 0;
  const mid = (settled + target) / 2;
  const after = ticks.filter((k) => k.t >= stepAt);
  for (let i = 1; i < after.length; i++) {
    const p = after[i - 1].bars[3];
    const c = after[i].bars[3];
    if (rising ? c >= mid : c <= mid) {
      const f = (mid - p) / (c - p);
      return after[i - 1].t + f * (after[i].t - after[i - 1].t) - stepAt;
    }
  }
  throw new Error("the bar never reached the halfway point");
}

describe("bars.frame", () => {
  it("returns seven values in 0..1 for every look", () => {
    for (const look of Object.values(LOOKS)) {
      for (const t of [0, 123, 5000, 99999]) {
        const { bars } = frame(t, look, null, { startedAt: 0 });
        expect(bars).toHaveLength(BAR_COUNT);
        for (const b of bars) {
          expect(b).toBeGreaterThanOrEqual(0);
          expect(b).toBeLessThanOrEqual(1);
        }
      }
    }
  });

  it("attack_faster_than_release", () => {
    expect(ATTACK_TAU_MS).toBe(25);
    expect(RELEASE_TAU_MS).toBe(110);
    const up = frame(25, LOOKS.listening, flat(100, 20), { prev: { at: 0, bars: Array(7).fill(0) } });
    const down = frame(25, LOOKS.listening, flat(0, 20), { prev: { at: 0, bars: Array(7).fill(1) } });
    const rise = up.bars[0];
    const fall = 1 - down.bars[0];
    expect(rise).toBeGreaterThan(fall * 2);
  });

  it("falls_back_to_simulation_after_150ms_without_levels", () => {
    expect(LEVEL_STALE_MS).toBe(150);
    // A silent but fresh real level is flat; once it is stale the simulation moves.
    expect(spread(frame(1000, LOOKS.listening, flat(0, 900), {}).bars)).toBeLessThan(1e-9);
    const later = [1200, 1330, 1470, 1610, 1790].map((t) => frame(t, LOOKS.listening, flat(0, 900), {}).bars);
    expect(later.some((b) => spread(b) > 0.05)).toBe(true);
  });

  it("keeps the real level at exactly the stale limit", () => {
    expect(spread(frame(1050, LOOKS.listening, flat(0, 900), {}).bars)).toBeLessThan(1e-9);
  });

  it("returns_to_real_levels_when_levels_resume", () => {
    const real = frame(2000, LOOKS.listening, flat(100, 1990), {}).bars;
    for (const b of real) expect(b).toBeCloseTo(1, 5);
    expect(frame(2000, LOOKS.listening, null, {}).bars).not.toEqual(real);
  });

  it("wave_phase_travels_left_to_right_1200ms", () => {
    const a = frame(300, LOOKS.thinking, null, {}).bars;
    // Bar i+1 shows what bar i showed 0.8 rad of phase earlier: the wave moves to the right.
    const shift = (0.8 / (2 * Math.PI)) * 1200;
    const b = frame(300 + shift, LOOKS.thinking, null, {}).bars;
    for (let i = 0; i < 6; i++) expect(b[i + 1]).toBeCloseTo(a[i], 5);
    // The underlying sine repeats every 1200 ms.
    const c = frame(300 + 1200, LOOKS.thinking, null, {}).bars;
    for (let i = 0; i < 7; i++) expect(c[i]).toBeCloseTo(a[i], 5);
    // And it is not a still picture.
    expect(b).not.toEqual(a);
  });

  it("levels_ignored_in_thinking_and_confirmation", () => {
    for (const look of [LOOKS.thinking, LOOKS.confirmation]) {
      expect(frame(500, look, flat(100, 499), {}).bars).toEqual(frame(500, look, null, {}).bars);
    }
  });

  it("a level of the wrong source is ignored by the look", () => {
    expect(frame(500, LOOKS.listening, flatTts(100, 499), {}).bars).toEqual(
      frame(500, LOOKS.listening, null, {}).bars,
    );
  });

  it("reduced_motion_simulation_does_not_animate", () => {
    const a = frame(1000, LOOKS.listening, null, { reduced: true }).bars;
    const b = frame(1337, LOOKS.listening, null, { reduced: true }).bars;
    expect(a).toEqual(b);
    for (const look of Object.values(LOOKS)) {
      const x = frame(10, look, null, { reduced: true, startedAt: 0 }).bars;
      const y = frame(2222, look, null, { reduced: true, startedAt: 0 }).bars;
      expect(x).toEqual(y);
      expect(x).toEqual([...look.silhouette]);
    }
  });

  it("fresh real levels still move the bars under reduced motion", () => {
    const real = frame(1000, LOOKS.speaking, flatTts(100, 995), { reduced: true }).bars;
    expect(real[0]).toBeCloseTo(1, 5);
  });

  it("fake_clock_lag_mean_at_most_50ms_max_at_most_100ms", () => {
    // Step the real level up and then down on a 60 Hz clock. Lag is the time the bar
    // needs to cover half of the step (display quantization is the manual 240 fps check).
    const stepAt = 1000;
    const up = run(0, 2000, (t) => flat(t < stepAt ? 0 : 100, t));
    const down = run(0, 2000, (t) => flat(t < stepAt ? 100 : 0, t));
    const rise = halfwayLag(up, stepAt, true);
    const fall = halfwayLag(down, stepAt, false);
    expect((rise + fall) / 2).toBeLessThanOrEqual(50);
    expect(Math.max(rise, fall)).toBeLessThanOrEqual(100);
    expect(rise).toBeLessThan(fall);
  });

  it("success bounces once and settles, error keeps a low jitter", () => {
    const settled = frame(5000, LOOKS.success, null, { startedAt: 0 }).bars;
    const early = frame(150, LOOKS.success, null, { startedAt: 0 }).bars;
    expect(Math.max(...early)).toBeGreaterThan(Math.max(...settled));
    expect(frame(9000, LOOKS.success, null, { startedAt: 0 }).bars).toEqual(settled);
    const err = frame(700, LOOKS.error, null, { startedAt: 0 }).bars;
    expect(Math.max(...err)).toBeLessThan(0.4);
  });

  it("reminder pulses outward from the centre", () => {
    const a = frame(0, LOOKS.reminder, null, { startedAt: 0 }).bars;
    expect(a[2]).toBeCloseTo(a[4], 9);
    expect(a[0]).toBeCloseTo(a[6], 9);
    const peakTime = (i: number) => {
      let best = -1;
      let bestT = 0;
      for (let t = 0; t < 1000; t += 5) {
        const v = frame(t, LOOKS.reminder, null, { startedAt: 0 }).bars[i];
        if (v > best) {
          best = v;
          bestT = t;
        }
      }
      return bestT;
    };
    // The centre bar peaks before the outer bar of the same cycle does.
    expect(peakTime(3)).toBeLessThan(peakTime(0));
  });
});
