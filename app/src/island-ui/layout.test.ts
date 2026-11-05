import { describe, expect, it } from "vitest";
import {
  ACTIONS_GAP,
  ACTIONS_WIDTH,
  BORDER,
  BUTTON_GAP,
  BUTTON_HEIGHT,
  BUTTON_WIDTH,
  CHROME_X,
  GAP,
  MASCOT_SIZE,
  PAD_X,
  TEXT_PAD_Y,
  LINE_HEIGHT,
  MAX_ISLAND_HEIGHT,
  MAX_ISLAND_WIDTH,
  MAX_LINES,
  MAX_TEXT_WIDTH,
  SCROLL_END_PAD,
  MIN_ISLAND_HEIGHT,
  MIN_ISLAND_WIDTH,
  computeLayout,
  confirmButtonRects,
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
    expect(l.height).toBe(MIN_ISLAND_HEIGHT);
    expect(l.scrollDistance).toBe(0); // six lines fit the mascot's height; only buttons add more
  });

  it("caps the height at six lines and scrolls the rest", () => {
    const l = computeLayout({ lineWidth: 3000, blockHeight: 8 * LINE_HEIGHT });
    expect(l.lines).toBe(8);
    expect(l.visibleLines).toBe(MAX_LINES);
    expect(l.height).toBe(MIN_ISLAND_HEIGHT);
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

describe("computeLayout with the confirmation buttons", () => {
  const short = { lineWidth: 90, blockHeight: LINE_HEIGHT };

  it("is unchanged when there are no buttons", () => {
    const plain = computeLayout(short);
    expect(computeLayout({ ...short, actions: false })).toEqual(plain);
    expect(plain.actionsHeight).toBe(0);
  });

  it("makes room for the buttons' row under the text", () => {
    const l = computeLayout({ ...short, actions: true });
    expect(l.actionsHeight).toBe(ACTIONS_GAP + BUTTON_HEIGHT);
    expect(l.textWidth).toBeGreaterThanOrEqual(ACTIONS_WIDTH);
    expect(l.width).toBe(CHROME_X + l.textWidth);
  });

  it("keeps a compact confirmation as tall as the mascot", () => {
    // One line of text plus the row is 60 px of column: the mascot still decides.
    expect(computeLayout({ ...short, actions: true }).height).toBe(MIN_ISLAND_HEIGHT);
    expect(computeLayout({ lineWidth: 900, blockHeight: 4 * LINE_HEIGHT, actions: true }).height).toBe(
      MIN_ISLAND_HEIGHT,
    );
  });

  it("grows for five and six lines of text and stops at the budget", () => {
    const row = ACTIONS_GAP + BUTTON_HEIGHT;
    const chrome = 2 * TEXT_PAD_Y + 2 * BORDER;
    const five = computeLayout({ lineWidth: 2000, blockHeight: 5 * LINE_HEIGHT, actions: true });
    expect(five.height).toBe(5 * LINE_HEIGHT + row + chrome);
    const six = computeLayout({ lineWidth: 2000, blockHeight: 6 * LINE_HEIGHT, actions: true });
    expect(six.height).toBe(MAX_ISLAND_HEIGHT);
    expect(six.height).toBe(6 * LINE_HEIGHT + row + chrome);
    const eight = computeLayout({ lineWidth: 3000, blockHeight: 8 * LINE_HEIGHT, actions: true });
    expect(eight.height).toBe(MAX_ISLAND_HEIGHT);
    expect(eight.scrollDistance).toBeGreaterThan(0);
  });

  it("never lets the island exceed the window budget", () => {
    for (const blockHeight of [LINE_HEIGHT, 3 * LINE_HEIGHT, 6 * LINE_HEIGHT, 9 * LINE_HEIGHT]) {
      for (const actions of [false, true]) {
        const l = computeLayout({ lineWidth: 5000, blockHeight, actions });
        expect(l.height).toBeLessThanOrEqual(MAX_ISLAND_HEIGHT);
        expect(l.width).toBeLessThanOrEqual(MAX_ISLAND_WIDTH);
      }
    }
  });
});

describe("confirmButtonRects", () => {
  const STAGE = 640;

  it("puts both buttons inside the island, under the text, in the text column", () => {
    const l = computeLayout({ lineWidth: 90, blockHeight: LINE_HEIGHT, actions: true });
    const { cancel, confirm } = confirmButtonRects(l, STAGE);
    const left = STAGE / 2 - l.width / 2;
    const column = left + PAD_X + MASCOT_SIZE + GAP;
    expect(confirm.x + confirm.width).toBe(column + l.textWidth);
    expect(cancel.x + cancel.width + BUTTON_GAP).toBe(confirm.x);
    expect(cancel.x).toBeGreaterThanOrEqual(column);
    for (const r of [cancel, confirm]) {
      expect(r.width).toBe(BUTTON_WIDTH);
      expect(r.height).toBe(BUTTON_HEIGHT);
      expect(r.y).toBeGreaterThan(0);
      expect(r.y + r.height).toBeLessThanOrEqual(l.height);
    }
    expect(cancel.y).toBe(confirm.y);
  });

  it("centers the text and the row together in the island's height", () => {
    const l = computeLayout({ lineWidth: 90, blockHeight: LINE_HEIGHT, actions: true });
    const { confirm } = confirmButtonRects(l, STAGE);
    const column = l.textHeight + l.actionsHeight;
    expect(confirm.y).toBe((l.height - column) / 2 + l.textHeight + ACTIONS_GAP);
  });

  it("follows the message when it wraps: more lines, a taller island, a lower row", () => {
    const one = computeLayout({ lineWidth: 90, blockHeight: LINE_HEIGHT, actions: true });
    const six = computeLayout({ lineWidth: 2000, blockHeight: 6 * LINE_HEIGHT, actions: true });
    const a = confirmButtonRects(one, STAGE).confirm;
    const b = confirmButtonRects(six, STAGE).confirm;
    expect(b.y).toBeGreaterThan(a.y);
    expect(b.y + b.height).toBeLessThanOrEqual(six.height);
    // A wider island keeps the right edge of the row at the right edge of the text.
    expect(b.x + b.width).toBe(STAGE / 2 - six.width / 2 + PAD_X + MASCOT_SIZE + GAP + six.textWidth);
  });

  it("stays inside the window at the widest island", () => {
    const wide = computeLayout({ lineWidth: 2000, blockHeight: 2 * LINE_HEIGHT, actions: true });
    const { cancel, confirm } = confirmButtonRects(wide, STAGE);
    expect(cancel.x).toBeGreaterThan(0);
    expect(confirm.x + confirm.width).toBeLessThan(STAGE);
  });
});
