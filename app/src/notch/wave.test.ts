import { describe, expect, it } from "vitest";
import { LOOKS } from "./looks";
import type { LevelSample } from "./levelStore";
import {
  AMP_GAIN,
  BREATH_LEVEL,
  createWave,
  drawWave,
  envelope,
  fitCanvas,
  halfHeights,
  LEVEL_ATTACK,
  LEVEL_RELEASE,
  LEVEL_STALE_MS,
  MIN_HALF,
  outline,
  retargetsDue,
  RETARGET_MS,
  seededRng,
  SPACE_H,
  SPACE_W,
  stepWave,
  targetLevel,
  WAVE_POINTS,
  type Ctx2D,
  type WaveState,
} from "./wave";

const sample = (src: "mic" | "tts", v: number, at: number): LevelSample => ({ src: src as "tts", bands: Array(7).fill(v), at });
const u = (i: number) => i / (WAVE_POINTS - 1);

describe("wave constants and envelope", () => {
  it("uses the owner's prototype constants", () => {
    expect(WAVE_POINTS).toBe(30);
    expect(RETARGET_MS).toBe(70);
    expect(LEVEL_ATTACK).toBe(0.4);
    expect(LEVEL_RELEASE).toBe(0.1);
    expect(AMP_GAIN).toBe(24);
    expect(MIN_HALF).toBe(0.6);
    expect(BREATH_LEVEL).toBe(0.05);
    expect(LEVEL_STALE_MS).toBe(150);
    expect([SPACE_W, SPACE_H]).toEqual([236, 60]);
  });

  it("envelope is a bell centered on the notch", () => {
    expect(envelope(0.5)).toBe(1);
    expect(envelope(0.5 - 0.22)).toBeCloseTo(Math.exp(-1), 9);
    expect(envelope(0.2)).toBeCloseTo(envelope(0.8), 12);
    expect(envelope(0)).toBeLessThan(0.01);
  });

  it("the seeded rng is deterministic and in [0, 1)", () => {
    const a = seededRng(7);
    const b = seededRng(7);
    const xs = Array.from({ length: 50 }, () => a());
    expect(xs).toEqual(Array.from({ length: 50 }, () => b()));
    for (const x of xs) {
      expect(x).toBeGreaterThanOrEqual(0);
      expect(x).toBeLessThan(1);
    }
    expect(seededRng(8)()).not.toBe(seededRng(7)());
  });
});

describe("stepWave", () => {
  it("starts flat", () => {
    const s = createWave();
    expect(s.cur).toHaveLength(30);
    expect(s.cur.every((v) => v === 0)).toBe(true);
    expect(s.lvl).toBe(0);
  });

  it("re-targets with the injected rng using env(u) * (0.2 + 0.8 r^2 1.3) and follows at 0.45", () => {
    const draws = Array.from({ length: 30 }, (_, i) => (i + 1) / 31);
    let n = 0;
    const s = stepWave(createWave(), 1000, 0, () => draws[n++], false);
    expect(n).toBe(30);
    draws.forEach((r, i) => {
      const tgt = envelope(u(i)) * (0.2 + 0.8 * r * r * 1.3);
      expect(s.tgt[i]).toBeCloseTo(tgt, 12);
      expect(s.cur[i]).toBeCloseTo(tgt * 0.45, 12);
    });
  });

  it("re-targets only every 70 ms, while cur keeps closing in on tgt", () => {
    const rng = seededRng(1);
    let s = stepWave(createWave(), 0, 0, rng, false);
    const first = s.tgt;
    s = stepWave(s, 16, 0, rng, false);
    s = stepWave(s, 69, 0, rng, false);
    expect(s.tgt).toEqual(first);
    expect(retargetsDue(s, 69)).toBe(false);
    expect(Math.abs(s.cur[15] - first[15])).toBeLessThan(Math.abs(first[15] * 0.45 ** 2) + 1e-12);
    expect(retargetsDue(s, 70)).toBe(true);
    s = stepWave(s, 70, 0, rng, false);
    expect(s.tgt).not.toEqual(first);
  });

  it("same seed, same frames, same shape", () => {
    const run = () => {
      const rng = seededRng(42);
      let s: WaveState = createWave();
      for (let t = 0; t < 1000; t += 16) s = stepWave(s, t, 0.5, rng, false);
      return s;
    };
    expect(run()).toEqual(run());
  });

  it("smooths the level fast going up and slowly going down", () => {
    const up = stepWave(createWave(), 0, 1, () => 0.5, false);
    expect(up.lvl).toBeCloseTo(0.4, 12);
    const down = stepWave({ ...createWave(), lvl: 1 }, 0, 0, () => 0.5, false);
    expect(down.lvl).toBeCloseTo(0.9, 12);
  });

  it("reduced motion never calls the rng and shows a static shape", () => {
    const rng = () => {
      throw new Error("rng must not be used under reduced motion");
    };
    const a = stepWave(createWave(), 0, 0.5, rng, true);
    let s = a;
    for (let t = 16; t < 2000; t += 16) s = stepWave({ ...s, lvl: a.lvl }, t, 0.5, rng, true);
    expect(s.cur).toEqual(a.cur);
    // Biggest in the middle, symmetric.
    expect(a.cur[14]).toBeGreaterThan(a.cur[3]);
    expect(a.cur[3]).toBeCloseTo(a.cur[26], 12);
  });
});

describe("halfHeights and outline", () => {
  it("level 0 and a flat state is a flat thin line", () => {
    const h = halfHeights(createWave());
    expect(h.every((v) => v === MIN_HALF)).toBe(true);
  });

  it("half heights are clamp(cur * (1 + level 24), 0.6, H/2 - 1.5)", () => {
    const s: WaveState = { ...createWave(), cur: Array(30).fill(0.1), lvl: 0.5 };
    s.cur[0] = 0.01;
    s.cur[1] = 10;
    const h = halfHeights(s);
    expect(h[0]).toBe(0.6);
    expect(h[1]).toBe(SPACE_H / 2 - 1.5);
    expect(h[2]).toBeCloseTo(0.1 * 13, 12);
  });

  it("the outline is mirrored, spans 7%..93% with flat tails at 2.8%/97.2%", () => {
    const s: WaveState = { ...createWave(), cur: Array.from({ length: 30 }, (_, i) => 0.05 + i * 0.01), lvl: 0.2 };
    const pts = outline(s);
    const cy = SPACE_H / 2;
    // tail, 30 top, tail, 30 bottom
    expect(pts).toHaveLength(62);
    expect(pts[0]).toEqual({ x: 0.028 * SPACE_W, y: cy });
    expect(pts[31]).toEqual({ x: 0.972 * SPACE_W, y: cy });
    const top = pts.slice(1, 31);
    expect(top[0].x).toBeCloseTo(0.07 * SPACE_W, 9);
    expect(top[29].x).toBeCloseTo(0.93 * SPACE_W, 9);
    const h = halfHeights(s);
    top.forEach((p, i) => expect(p.y).toBeCloseTo(cy - h[i], 9));
    const bottom = pts.slice(32);
    bottom.forEach((p, k) => {
      const i = 29 - k;
      expect(p.x).toBeCloseTo(top[i].x, 9);
      expect(p.y).toBeCloseTo(cy + h[i], 9);
    });
  });
});

describe("targetLevel", () => {
  it("speaking follows a fresh tts level; listening ignores any level and breathes", () => {
    expect(targetLevel(LOOKS.listening, sample("mic", 60, 900), 1000, false)).toBe(BREATH_LEVEL);
    expect(targetLevel(LOOKS.listening, sample("tts", 60, 900), 1000, false)).toBe(BREATH_LEVEL);
    expect(targetLevel(LOOKS.speaking, sample("tts", 25, 900), 1000, false)).toBeCloseTo(0.25, 12);
  });

  it("a level exactly at the stale limit is still real; a silent real level is 0", () => {
    expect(targetLevel(LOOKS.speaking, sample("tts", 0, 850), 1000, false)).toBe(0);
  });

  it("falls back to the simulated voice when the level is stale, missing or of the wrong source", () => {
    const sims = [1000, 1210, 1430, 1650, 1870, 2090].flatMap((t) => [
      targetLevel(LOOKS.speaking, null, t, false),
      targetLevel(LOOKS.speaking, sample("tts", 0, 100), t, false),
      targetLevel(LOOKS.speaking, sample("mic", 100, t), t, false),
    ]);
    expect(Math.max(...sims)).toBeGreaterThan(0.2);
    for (const v of sims) {
      expect(v).toBeGreaterThanOrEqual(0);
      expect(v).toBeLessThanOrEqual(1);
    }
    // The simulation has gaps.
    expect(Math.min(...sims)).toBeLessThan(0.05);
  });

  it("the other looks ignore levels and breathe at about 0.05", () => {
    for (const look of [LOOKS.listening, LOOKS.thinking, LOOKS.confirmation, LOOKS.success, LOOKS.error, LOOKS.reminder]) {
      expect(targetLevel(look, sample("mic", 100, 999), 1000, false)).toBe(BREATH_LEVEL);
    }
  });

  it("under reduced motion the simulation and the breathing are off, real levels still count", () => {
    expect(targetLevel(LOOKS.speaking, null, 1000, true)).toBe(0);
    expect(targetLevel(LOOKS.listening, null, 1000, true)).toBe(0);
    expect(targetLevel(LOOKS.thinking, null, 1000, true)).toBe(0);
    expect(targetLevel(LOOKS.speaking, sample("tts", 50, 990), 1000, true)).toBeCloseTo(0.5, 12);
  });
});

describe("level lag", () => {
  /**
   * Time after the step until the smoothed level covered half of it. The
   * smoothing constants are per frame, exactly as in the owner-approved
   * prototype, so motion scales with the refresh rate by design.
   */
  function halfLag(rising: boolean, hz = 60): number {
    const rng = seededRng(3);
    const dt = 1000 / hz;
    let s: WaveState = { ...createWave(), lvl: rising ? 0 : 1 };
    const goal = rising ? 1 : 0;
    for (let k = 1; k < 400; k++) {
      s = stepWave(s, k * dt, goal, rng, false);
      if (rising ? s.lvl >= 0.5 : s.lvl <= 0.5) return k * dt;
    }
    throw new Error("never reached the halfway point");
  }

  it("the onset (rise) lag is at most 50 ms on a 60 Hz clock", () => {
    // Bound of design L14: mean <= 50 ms, max <= 100 ms. It applies to the onset only.
    expect(halfLag(true)).toBeLessThanOrEqual(50);
  });

  it("the onset bound holds at 60 Hz and only gets shorter at higher refresh rates", () => {
    const lags = [60, 90, 120, 144, 240].map((hz) => halfLag(true, hz));
    expect(lags[0]).toBeLessThanOrEqual(50);
    for (let i = 1; i < lags.length; i++) expect(lags[i]).toBeLessThanOrEqual(lags[i - 1]);
  });

  it("the decay is deliberately slower than the onset and still under 120 ms", () => {
    expect(halfLag(false)).toBeGreaterThan(halfLag(true));
    expect(halfLag(false)).toBeLessThanOrEqual(120);
  });
});

/** Records the canvas calls so the drawing can be asserted without a browser. */
function fakeCtx() {
  const calls: string[] = [];
  const ctx: Ctx2D & Record<string, unknown> = {
    fillStyle: "",
    lineJoin: "round",
    miterLimit: 10,
    setTransform: (...a: number[]) => void calls.push(`setTransform(${a.map((n) => +n.toFixed(4)).join(",")})`),
    clearRect: (...a: number[]) => void calls.push(`clearRect(${a.join(",")})`),
    beginPath: () => void calls.push("beginPath"),
    moveTo: (x: number, y: number) => void calls.push(`moveTo(${x.toFixed(2)},${y.toFixed(2)})`),
    lineTo: (x: number, y: number) => void calls.push(`lineTo(${x.toFixed(2)},${y.toFixed(2)})`),
    closePath: () => void calls.push("closePath"),
    fill: () => void calls.push("fill"),
  };
  return { ctx, calls };
}

describe("drawWave", () => {
  it("fills one closed mitered polygon with the opaque state color, scaled to the canvas", () => {
    const { ctx, calls } = fakeCtx();
    drawWave(ctx, createWave(), "#378ADD", 236, 60);
    expect(ctx.fillStyle).toBe("#378ADD");
    expect(ctx.lineJoin).toBe("miter");
    expect(ctx.miterLimit).toBe(3);
    expect(calls[0]).toBe("setTransform(1,0,0,1,0,0)");
    expect(calls.filter((c) => c === "fill")).toHaveLength(1);
    expect(calls.filter((c) => c.startsWith("moveTo"))).toHaveLength(1);
    expect(calls.filter((c) => c.startsWith("lineTo"))).toHaveLength(61);
    expect(calls.indexOf("closePath")).toBeLessThan(calls.indexOf("fill"));
  });

  it("scales the 236x60 drawing space to a DPR-sized canvas", () => {
    const { ctx, calls } = fakeCtx();
    drawWave(ctx, createWave(), "#fff", 236 * 1.5, 60 * 1.5);
    expect(calls[0]).toBe("setTransform(1.5,0,0,1.5,0,0)");
  });
});

describe("fitCanvas", () => {
  it("sizes the backing store by the device pixel ratio and reports it", () => {
    const canvas = { width: 0, height: 0 };
    expect(fitCanvas(canvas, 118, 30, 2)).toEqual({ w: 236, h: 60 });
    expect(canvas).toEqual({ width: 236, height: 60 });
    expect(fitCanvas(canvas, 118, 30, 1.25)).toEqual({ w: 148, h: 38 });
    expect(fitCanvas(canvas, 118, 30, 0)).toEqual({ w: 118, h: 30 });
  });
});
