import { describe, expect, it } from "vitest";
import { EDGE_IN_MS, EDGE_OUT_MS } from "./choreography";
import { edgeIntro, edgeOut, edgeSettled } from "./edge";
import { LINE_HEIGHT, computeLayout } from "./layout";
import { shapePath, type Shape } from "./shape";

const SHORT = computeLayout({ lineWidth: 90, blockHeight: LINE_HEIGHT });
const TWO = computeLayout({ lineWidth: 900, blockHeight: 2 * LINE_HEIGHT });
const TALL = computeLayout({ lineWidth: 3000, blockHeight: 8 * LINE_HEIGHT });
const LAYOUTS = [SHORT, TWO, TALL];
const CX = 320;
const nums = (d: string) => (d.match(/-?\d+(?:\.\d+)?/g) ?? []).map(Number);

describe("shapePath", () => {
  it("draws the settled pill from fillet to fillet along the top edge", () => {
    const s = edgeSettled(SHORT);
    const d = shapePath(s, CX);
    expect(d.startsWith("M")).toBe(true);
    const n = nums(d);
    expect(n[0]).toBeCloseTo(CX - s.bw - s.F, 1);
    expect(n[1]).toBe(0);
    expect(n[n.length - 2]).toBeCloseTo(CX + s.bw + s.F, 1);
    expect(n[n.length - 1]).toBe(0);
  });

  it("stays inside its own box for every moment of the animation", () => {
    for (const l of LAYOUTS) {
      const frames: Shape[] = [];
      for (let t = 0; t <= EDGE_IN_MS; t += 16) frames.push(edgeIntro(t, l));
      for (let t = 0; t <= EDGE_OUT_MS; t += 16) frames.push(edgeOut(t, l));
      for (const s of frames) {
        const d = shapePath(s, CX);
        expect(d).not.toMatch(/NaN|Infinity/);
        // The path repairs tiny sizes (at least 1 px wide, 2 px tall, 0.5 px fillets).
        const reach = Math.max(1, s.bw) + Math.max(0.5, s.F);
        const v = nums(d);
        for (let i = 0; i < v.length; i += 2) {
          expect(v[i]).toBeGreaterThanOrEqual(CX - reach - 0.02);
          expect(v[i]).toBeLessThanOrEqual(CX + reach + 0.02);
          expect(v[i + 1]).toBeGreaterThanOrEqual(-0.02);
          expect(v[i + 1]).toBeLessThanOrEqual(Math.max(2, s.h) + 0.02);
        }
      }
    }
  });

  it("repairs absurd parameters instead of producing a broken path", () => {
    const bad: Shape[] = [
      { bw: -3, h: -9, F: 99, r: 400 },
      { bw: Number.NaN, h: 10, F: 1, r: 1 },
      { bw: 10, h: 2, F: 12, r: 30 },
    ];
    for (const s of bad) {
      const d = shapePath(s, CX);
      expect(d).not.toMatch(/NaN|Infinity/);
      expect(nums(d).every(Number.isFinite)).toBe(true);
    }
  });
});
