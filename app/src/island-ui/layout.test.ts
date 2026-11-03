import { describe, expect, it } from "vitest";
import {
  CHROME_X,
  LINE_HEIGHT,
  MAX_ISLAND_HEIGHT,
  MAX_ISLAND_WIDTH,
  MAX_LINES,
  MAX_TEXT_WIDTH,
  SCROLL_END_PAD,
  MIN_ISLAND_HEIGHT,
  MIN_ISLAND_WIDTH,
  computeLayout,
} from "./layout";

describe("computeLayout", () => {
  it("keeps a short single line as a compact pill", () => {
    const l = computeLayout({ lineWidth: 90, blockHeight: LINE_HEIGHT });
    expect(l.lines).toBe(1);
    expect(l.height).toBe(MIN_ISLAND_HEIGHT);
    expect(l.width).toBe(CHROME_X + l.textWidth);
    expect(l.textWidth).toBeGreaterThanOrEqual(90);
    expect(l.scrollDistance).toBe(0);
  });

  it("never goes below the minimum width", () => {
    expect(computeLayout({ lineWidth: 4, blockHeight: LINE_HEIGHT }).width).toBe(MIN_ISLAND_WIDTH);
  });

  it("treats a line that exactly fills the text area as one line", () => {
    const l = computeLayout({ lineWidth: MAX_TEXT_WIDTH, blockHeight: LINE_HEIGHT });
    expect(l.lines).toBe(1);
    expect(l.width).toBe(MAX_ISLAND_WIDTH);
  });

  it("wraps at the maximum width and counts lines from the wrapped height", () => {
    const l = computeLayout({ lineWidth: 900, blockHeight: 2 * LINE_HEIGHT });
    expect(l.lines).toBe(2);
    expect(l.width).toBe(MAX_ISLAND_WIDTH);
    expect(l.textWidth).toBe(MAX_TEXT_WIDTH);
    expect(l.height).toBe(MIN_ISLAND_HEIGHT);
  });

  it("tolerates sub-pixel line heights when counting lines", () => {
    expect(computeLayout({ lineWidth: 900, blockHeight: 3 * LINE_HEIGHT - 0.4 }).lines).toBe(3);
    expect(computeLayout({ lineWidth: 900, blockHeight: 3 * LINE_HEIGHT + 0.4 }).lines).toBe(3);
  });

  it("grows taller up to six lines without scrolling", () => {
    const l = computeLayout({ lineWidth: 2000, blockHeight: MAX_LINES * LINE_HEIGHT });
    expect(l.lines).toBe(6);
    expect(l.visibleLines).toBe(6);
    expect(l.height).toBe(MAX_ISLAND_HEIGHT);
    expect(l.scrollDistance).toBe(0);
  });

  it("caps the height at six lines and scrolls the rest", () => {
    const l = computeLayout({ lineWidth: 3000, blockHeight: 8 * LINE_HEIGHT });
    expect(l.lines).toBe(8);
    expect(l.visibleLines).toBe(MAX_LINES);
    expect(l.height).toBe(MAX_ISLAND_HEIGHT);
    expect(l.textHeight).toBe(MAX_LINES * LINE_HEIGHT);
    // The extra pad keeps the last line clear of the fading bottom edge.
    expect(l.scrollDistance).toBe(2 * LINE_HEIGHT + SCROLL_END_PAD);
  });

  it("never exceeds the window budget, whatever the input", () => {
    for (const blockHeight of [0, 1, 500, 1e6, Number.NaN, Number.POSITIVE_INFINITY, -50]) {
      for (const lineWidth of [0, 50, 1e6, Number.NaN, -3]) {
        const l = computeLayout({ lineWidth, blockHeight });
        expect(l.width).toBeLessThanOrEqual(MAX_ISLAND_WIDTH);
        expect(l.width).toBeGreaterThanOrEqual(MIN_ISLAND_WIDTH);
        expect(l.height).toBeLessThanOrEqual(MAX_ISLAND_HEIGHT);
        expect(l.height).toBeGreaterThanOrEqual(MIN_ISLAND_HEIGHT);
        expect(l.lines).toBeGreaterThanOrEqual(1);
        expect(l.scrollDistance).toBeGreaterThanOrEqual(0);
        expect(Number.isFinite(l.scrollDistance)).toBe(true);
      }
    }
  });
});
