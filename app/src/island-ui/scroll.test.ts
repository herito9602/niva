import { describe, expect, it } from "vitest";
import { DEFAULT_SCROLL, pageOffsets, pagedFrame, scrollCycleMs, scrollFrame, scrollOffset } from "./scroll";

const C = DEFAULT_SCROLL;
const downMs = (distance: number) => Math.max(C.minTravelMs, (distance / C.pxPerSec) * 1000);

describe("scrollOffset", () => {
  it("stays at the start during the first pause", () => {
    expect(scrollOffset(0, 80, C)).toBe(0);
    expect(scrollOffset(C.startPauseMs - 1, 80, C)).toBe(0);
  });

  it("moves forward, never backwards, and reaches the end exactly", () => {
    let last = 0;
    for (let t = C.startPauseMs; t <= C.startPauseMs + downMs(80); t += 25) {
      const o = scrollOffset(t, 80, C);
      expect(o).toBeGreaterThanOrEqual(last);
      last = o;
    }
    expect(scrollOffset(C.startPauseMs + downMs(80), 80, C)).toBe(80);
  });

  it("holds the end during the second pause", () => {
    const end = C.startPauseMs + downMs(80);
    expect(scrollOffset(end + 10, 80, C)).toBe(80);
    expect(scrollOffset(end + C.endPauseMs - 1, 80, C)).toBe(80);
  });

  it("returns smoothly to the start and loops", () => {
    const cycle = scrollCycleMs(80, C);
    expect(scrollOffset(cycle, 80, C)).toBe(0);
    for (const t of [0, 1234, 4000, 6500, 9100]) {
      expect(scrollOffset(t + cycle, 80, C)).toBeCloseTo(scrollOffset(t, 80, C), 6);
    }
    let prev = scrollOffset(0, 80, C);
    for (let t = 0; t < cycle; t += 16) {
      const o = scrollOffset(t, 80, C);
      expect(Math.abs(o - prev)).toBeLessThan(6);
      prev = o;
    }
  });

  it("scrolls slowly: the top speed stays near a gentle pace", () => {
    let max = 0;
    let prev = scrollOffset(0, 200, C);
    const down = C.startPauseMs + downMs(200);
    for (let t = 16; t < down; t += 16) {
      const o = scrollOffset(t, 200, C);
      max = Math.max(max, ((o - prev) / 16) * 1000);
      prev = o;
    }
    expect(max).toBeLessThan(C.pxPerSec * 2);
  });

  it("is inert without distance or with a bad clock", () => {
    expect(scrollOffset(5000, 0, C)).toBe(0);
    expect(scrollOffset(Number.NaN, 80, C)).toBe(0);
    expect(scrollOffset(-100, 80, C)).toBe(0);
    expect(scrollOffset(Number.POSITIVE_INFINITY, 80, C)).toBe(0);
  });

  it("never leaves [0, distance]", () => {
    for (let t = 0; t < 40000; t += 37) {
      const o = scrollOffset(t, 120, C);
      expect(o).toBeGreaterThanOrEqual(0);
      expect(o).toBeLessThanOrEqual(120);
    }
  });
});

describe("pageOffsets", () => {
  it("is a single page when everything fits", () => {
    expect(pageOffsets(0, 120)).toEqual([0]);
  });

  it("pages by a full viewport and ends on the last lines", () => {
    expect(pageOffsets(45, 120)).toEqual([0, 45]);
    expect(pageOffsets(165, 120)).toEqual([0, 120, 165]);
    expect(pageOffsets(120, 120)).toEqual([0, 120]);
    expect(pageOffsets(240, 120)).toEqual([0, 120, 240]);
  });
});

describe("pagedFrame (reduced motion)", () => {
  const dwell = C.pageDwellMs;
  const fade = C.pageFadeMs;

  it("shows one page solid", () => {
    expect(pagedFrame(12345, [0], C)).toEqual({ offset: 0, opacity: 1 });
  });

  it("starts on the first page at full opacity", () => {
    expect(pagedFrame(0, [0, 40], C)).toEqual({ offset: 0, opacity: 1 });
  });

  it("fades out, switches page while invisible, fades in", () => {
    const outMid = pagedFrame(dwell + fade / 2, [0, 40], C);
    expect(outMid.offset).toBe(0);
    expect(outMid.opacity).toBeGreaterThan(0);
    expect(outMid.opacity).toBeLessThan(1);
    const inMid = pagedFrame(dwell + fade + fade / 2, [0, 40], C);
    expect(inMid.offset).toBe(40);
    expect(inMid.opacity).toBeGreaterThan(0);
    expect(inMid.opacity).toBeLessThan(1);
    expect(pagedFrame(dwell + 2 * fade + 100, [0, 40], C)).toEqual({ offset: 40, opacity: 1 });
  });

  it("covers every page and wraps around", () => {
    const seen = new Set<number>();
    for (let t = 0; t < 60000; t += 50) {
      const f = pagedFrame(t, [0, 120, 160], C);
      seen.add(f.offset);
      expect(f.opacity).toBeGreaterThanOrEqual(0);
      expect(f.opacity).toBeLessThanOrEqual(1);
    }
    expect([...seen].sort((a, b) => a - b)).toEqual([0, 120, 160]);
  });

  it("is safe with a bad clock", () => {
    expect(pagedFrame(Number.NaN, [0, 40], C)).toEqual({ offset: 0, opacity: 1 });
  });
});

describe("scrollFrame", () => {
  it("uses the smooth scroll normally and the paged fade under reduced motion", () => {
    const info = { distance: 80, offsets: [0, 80] };
    const smooth = scrollFrame(C.startPauseMs + 2000, info, false, C);
    expect(smooth.opacity).toBe(1);
    expect(smooth.offset).toBeGreaterThan(0);
    const reduced = scrollFrame(C.pageDwellMs + C.pageFadeMs / 2, info, true, C);
    expect(reduced.opacity).toBeLessThan(1);
  });

  it("is still when there is nothing to scroll", () => {
    const none = { distance: 0, offsets: [0] };
    expect(scrollFrame(9999, none, false, C)).toEqual({ offset: 0, opacity: 1 });
    expect(scrollFrame(9999, none, true, C)).toEqual({ offset: 0, opacity: 1 });
  });
});
