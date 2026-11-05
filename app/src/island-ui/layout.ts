// Size maths of the island: the pill hugs its message, up to a maximum, and a
// message longer than six lines scrolls instead of growing further.
//
// The browser does the text layout (so wrapping always matches what is drawn);
// this module only turns two measurements into the pill geometry. The numbers
// below are mirrored in styles.css, and the ones named "window budget" are
// checked against tauri.conf.json by a Rust test (`island_contract_tests.rs`),
// which reads this file: keep them as plain numeric literals.

export const LINE_HEIGHT = 20;
export const MAX_LINES = 6;
export const MASCOT_SIZE = 124;
export const PAD_X = 24;
export const PAD_Y = 8;
export const TEXT_PAD_Y = 10;
export const GAP = 14;
export const BORDER = 1;
/** Room under the last line of a scrolling message, so the fading edge never dims it. */
export const SCROLL_END_PAD = 5;
/** Extra pixels on a measured line, so sub-pixel rounding never wraps it. */
const TEXT_SLACK = 2;

// Window budget (logical px).
export const MAX_ISLAND_WIDTH = 560;
export const MAX_ISLAND_HEIGHT = 142;
/** Room for the height spring to overshoot. */
export const SPRING_OVERSHOOT = 14;
/** Room for the soft shadow. */
export const SHADOW_ROOM_X = 40;
export const SHADOW_ROOM_Y = 28;

/** Pixels around the text: borders, side padding, mascot and the gap after it. */
export const CHROME_X = 2 * (PAD_X + BORDER) + MASCOT_SIZE + GAP;
export const MAX_TEXT_WIDTH = MAX_ISLAND_WIDTH - CHROME_X;
export const MIN_ISLAND_WIDTH = 208;
export const MIN_ISLAND_HEIGHT = MASCOT_SIZE + 2 * PAD_Y + 2 * BORDER;

/** What the browser reports for a message. */
export interface Measured {
  /** Width of the whole message on one line. */
  lineWidth: number;
  /** Height of the message wrapped at `MAX_TEXT_WIDTH`. */
  blockHeight: number;
}

export interface Layout {
  width: number;
  height: number;
  /** Width of the text column. */
  textWidth: number;
  /** Height of the visible text window (at most six lines). */
  textHeight: number;
  /** Lines the whole message takes. */
  lines: number;
  visibleLines: number;
  /** Pixels the text has to travel to show its last line (0 = fits). */
  scrollDistance: number;
}

const finite = (n: number, fallback: number) => (Number.isFinite(n) ? n : fallback);

export function computeLayout(m: Measured): Layout {
  const lineWidth = Math.max(0, finite(m.lineWidth, 0));
  const blockHeight = finite(m.blockHeight, LINE_HEIGHT);

  const single = lineWidth <= MAX_TEXT_WIDTH;
  const minText = MIN_ISLAND_WIDTH - CHROME_X;
  const textWidth = single
    ? Math.min(MAX_TEXT_WIDTH, Math.max(minText, Math.ceil(lineWidth) + TEXT_SLACK))
    : MAX_TEXT_WIDTH;

  const lines = single ? 1 : Math.max(1, Math.round(blockHeight / LINE_HEIGHT));
  const visibleLines = Math.min(lines, MAX_LINES);
  const textHeight = visibleLines * LINE_HEIGHT;
  const height = Math.min(
    MAX_ISLAND_HEIGHT,
    Math.max(MIN_ISLAND_HEIGHT, textHeight + 2 * TEXT_PAD_Y + 2 * BORDER),
  );

  return {
    width: CHROME_X + textWidth,
    height,
    textWidth,
    textHeight,
    lines,
    visibleLines,
    scrollDistance: lines > visibleLines ? (lines - visibleLines) * LINE_HEIGHT + SCROLL_END_PAD : 0,
  };
}

/** Shown before the first measurement. */
export const COMPACT_LAYOUT: Layout = computeLayout({ lineWidth: 90, blockHeight: LINE_HEIGHT });
