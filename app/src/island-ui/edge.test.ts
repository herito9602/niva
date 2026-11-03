import { describe, expect, it } from "vitest";
import { EDGE_IN_MS, EDGE_OUT_MS } from "./choreography";
import {
  EDGE_FILLET,
  EDGE_INTRO_SETTLE_MS,
  EDGE_NOTCH_W,
  EDGE_RESIZE_SETTLE_MS,
  edgeIntro,
  edgeOut,
  edgeResize,
  edgeSettled,
} from "./edge";
import { LINE_HEIGHT, MAX_ISLAND_WIDTH, SHADOW_ROOM_X, SPRING_OVERSHOOT, computeLayout } from "./layout";
import { shapePath } from "./shape";

const SHORT = computeLayout({ lineWidth: 90, blockHeight: LINE_HEIGHT });
const TWO = computeLayout({ lineWidth: 900, blockHeight: 2 * LINE_HEIGHT });
const TALL = computeLayout({ lineWidth: 3000, blockHeight: 8 * LINE_HEIGHT });
const LAYOUTS = [SHORT, TWO, TALL];
const CX = 320;
const nums = (d: string) => (d.match(/-?\d+(?:\.\d+)?/g) ?? []).map(Number);

describe("edgeSettled", () => {
  it("is the island hanging from the edge: fillets at the top", () => {
    const s = edgeSettled(SHORT);
    expect(s.bw).toBe(SHORT.width / 2);
    expect(s.h).toBe(SHORT.height);
    expect(s.F).toBe(EDGE_FILLET);
    expect(EDGE_FILLET).toBeGreaterThanOrEqual(8);
    expect(EDGE_FILLET).toBeLessThanOrEqual(12);
  });
});

describe("edgeIntro", () => {
  it("starts with no height, at the notch width", () => {
    const s = edgeIntro(0, SHORT);
    expect(s.h).toBe(0);
    expect(s.bw * 2).toBe(EDGE_NOTCH_W);
  });

  it("ends exactly on the settled shape within the declared duration", () => {
    for (const l of LAYOUTS) {
      expect(edgeIntro(EDGE_INTRO_SETTLE_MS, l)).toEqual(edgeSettled(l));
      expect(edgeIntro(EDGE_INTRO_SETTLE_MS * 3, l)).toEqual(edgeSettled(l));
    }
    expect(EDGE_IN_MS).toBeGreaterThanOrEqual(EDGE_INTRO_SETTLE_MS);
    // The owner liked this speed (kept as in Batch 23): do not slow it down.
    expect(EDGE_IN_MS).toBe(900);
    expect(EDGE_INTRO_SETTLE_MS).toBeGreaterThanOrEqual(700);
    expect(EDGE_INTRO_SETTLE_MS).toBeLessThanOrEqual(750);
  });

  it("is always flush with the top edge and its fillets stay sane", () => {
    for (const l of LAYOUTS) {
      for (let t = 0; t <= EDGE_INTRO_SETTLE_MS; t += 8) {
        const s = edgeIntro(t, l);
        for (const v of Object.values(s)) expect(Number.isFinite(v)).toBe(true);
        expect(s.h).toBeGreaterThanOrEqual(0);
        expect(s.bw).toBeGreaterThan(0);
        expect(s.F).toBeGreaterThanOrEqual(0);
        expect(s.F).toBeLessThanOrEqual(EDGE_FILLET);
        expect(s.F).toBeLessThanOrEqual(s.h / 2 + 1e-9);
        expect(s.r).toBeLessThanOrEqual(s.bw + 1e-9);
        if (s.h < 3) continue;
        const v = nums(shapePath(s, CX));
        // first and last points sit on y = 0, nothing is above it or past the fillets
        expect(v[1]).toBe(0);
        expect(v[v.length - 1]).toBe(0);
        for (let i = 0; i < v.length; i += 2) {
          expect(v[i]).toBeGreaterThanOrEqual(CX - s.bw - s.F - 0.02);
          expect(v[i]).toBeLessThanOrEqual(CX + s.bw + s.F + 0.02);
          expect(v[i + 1]).toBeGreaterThanOrEqual(-0.02);
          expect(v[i + 1]).toBeLessThanOrEqual(s.h + 0.02);
        }
      }
    }
  });

  it("grows down and out at the same time", () => {
    const s = edgeIntro(150, SHORT);
    const tall = s.h / SHORT.height;
    const wide = (s.bw * 2 - EDGE_NOTCH_W) / (SHORT.width - EDGE_NOTCH_W);
    expect(tall).toBeGreaterThan(0.1);
    expect(tall).toBeLessThan(0.95);
    expect(wide).toBeGreaterThan(0.03);
    expect(wide).toBeLessThan(0.95);
  });

  it("climbs steadily to its target, overshooting by 3 to 6 percent", () => {
    for (const l of [SHORT, TWO]) {
      let maxH = 0;
      let maxW = 0;
      let reached = false;
      let prev = 0;
      for (let t = 0; t <= EDGE_INTRO_SETTLE_MS; t += 2) {
        const s = edgeIntro(t, l);
        if (!reached) {
          expect(s.h).toBeGreaterThanOrEqual(prev - 1e-9);
          prev = s.h;
          if (s.h >= l.height) reached = true;
        }
        maxH = Math.max(maxH, s.h);
        maxW = Math.max(maxW, s.bw * 2);
      }
      expect((maxH - l.height) / l.height).toBeGreaterThanOrEqual(0.03);
      expect((maxH - l.height) / l.height).toBeLessThanOrEqual(0.06);
      expect((maxW - l.width) / (l.width - EDGE_NOTCH_W)).toBeGreaterThanOrEqual(0.03);
      expect((maxW - l.width) / (l.width - EDGE_NOTCH_W)).toBeLessThanOrEqual(0.06);
    }
  });

  it("stays inside the window budget", () => {
    for (const l of LAYOUTS) {
      for (let t = 0; t <= EDGE_INTRO_SETTLE_MS; t += 4) {
        const s = edgeIntro(t, l);
        expect(s.h).toBeLessThanOrEqual(l.height + SPRING_OVERSHOOT);
        expect(s.bw * 2).toBeLessThanOrEqual(MAX_ISLAND_WIDTH + SHADOW_ROOM_X);
      }
    }
  });

  it("is safe with a bad clock", () => {
    expect(edgeIntro(Number.NaN, SHORT)).toEqual(edgeIntro(0, SHORT));
    expect(edgeIntro(-9, SHORT)).toEqual(edgeIntro(0, SHORT));
  });
});

describe("edgeOut", () => {
  it("starts on the settled shape and retracts into the edge within the declared time", () => {
    for (const l of LAYOUTS) {
      expect(edgeOut(0, l)).toEqual(edgeSettled(l));
      const end = edgeOut(EDGE_OUT_MS, l);
      expect(end.h).toBeLessThan(1);
      expect(Math.abs(end.bw * 2 - EDGE_NOTCH_W)).toBeLessThan(2);
      expect(edgeOut(EDGE_OUT_MS * 3, l)).toEqual(end);
    }
  });

  it("retracts steadily with no bounce, flush with the edge", () => {
    for (const l of [SHORT, TALL]) {
      let prevH = Infinity;
      let prevW = Infinity;
      for (let t = 0; t <= EDGE_OUT_MS; t += 4) {
        const s = edgeOut(t, l);
        expect(s.h).toBeGreaterThanOrEqual(0);
        expect(s.h).toBeLessThanOrEqual(prevH + 0.5);
        expect(s.bw * 2).toBeLessThanOrEqual(prevW + 0.5);
        prevH = s.h;
        prevW = s.bw * 2;
        expect(s.F).toBeLessThanOrEqual(s.h / 2 + 1e-9);
      }
    }
  });

  it("waits for the content to fade before the shape moves", () => {
    const s = edgeOut(EDGE_OUT_MS * 0.1, SHORT);
    expect(SHORT.height - s.h).toBeLessThan(2);
  });

  it("keeps the exit speed the owner liked: 380 ms, nearly gone about 200 ms in", () => {
    expect(EDGE_OUT_MS).toBe(380);
    for (const l of [SHORT, TALL]) {
      expect(edgeOut(200, l).h / l.height).toBeLessThan(0.2);
      expect(edgeOut(100, l).h / l.height).toBeGreaterThan(0.7);
    }
  });
});

describe("edgeResize", () => {
  const from = edgeSettled(SHORT);

  it("starts where the island is and lands exactly on the new size", () => {
    expect(edgeResize(from, TALL, 0)).toEqual(from);
    expect(edgeResize(from, TALL, EDGE_RESIZE_SETTLE_MS)).toEqual(edgeSettled(TALL));
  });

  it("stays anchored at the top with a small overshoot", () => {
    let maxW = 0;
    for (let t = 0; t <= EDGE_RESIZE_SETTLE_MS; t += 4) {
      const s = edgeResize(from, TWO, t);
      maxW = Math.max(maxW, s.bw * 2);
      expect(s.h).toBeGreaterThan(0);
    }
    expect(maxW).toBeGreaterThan(TWO.width);
    expect((maxW - TWO.width) / (TWO.width - SHORT.width)).toBeLessThan(0.07);
  });

  it("does nothing when the size is the same", () => {
    expect(edgeResize(from, SHORT, 100)).toEqual(from);
  });
});
