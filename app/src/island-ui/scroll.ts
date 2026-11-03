// Timing of the long-message scroll. Pure functions of elapsed time, so the
// rAF loop only samples them and tests can check any moment.

export interface ScrollConfig {
  /** Rest at the first line before moving. */
  startPauseMs: number;
  /** Rest at the last line before going back. */
  endPauseMs: number;
  /** Reading pace on the way down. */
  pxPerSec: number;
  /** The way down never takes less than this, however short the distance. */
  minTravelMs: number;
  /** The way back is this many times faster than the way down. */
  returnFactor: number;
  /** Reduced motion: how long a page of text stays on screen. */
  pageDwellMs: number;
  /** Reduced motion: fade between two pages. */
  pageFadeMs: number;
}

export const DEFAULT_SCROLL: ScrollConfig = {
  startPauseMs: 1800,
  endPauseMs: 2200,
  pxPerSec: 26,
  minTravelMs: 1200,
  returnFactor: 3,
  pageDwellMs: 4500,
  pageFadeMs: 220,
};

export interface ScrollFrame {
  offset: number;
  opacity: number;
}

const usable = (t: number) => (Number.isFinite(t) && t > 0 ? t : 0);
const easeInOutSine = (p: number) => 0.5 - 0.5 * Math.cos(Math.PI * p);

function travelMs(distance: number, c: ScrollConfig): number {
  return Math.max(c.minTravelMs, (distance / c.pxPerSec) * 1000);
}

function returnMs(distance: number, c: ScrollConfig): number {
  return Math.max(c.minTravelMs / c.returnFactor, travelMs(distance, c) / c.returnFactor);
}

/** Length of one loop: pause, down, pause, back. */
export function scrollCycleMs(distance: number, c: ScrollConfig = DEFAULT_SCROLL): number {
  return c.startPauseMs + travelMs(distance, c) + c.endPauseMs + returnMs(distance, c);
}

/** Offset in px (0..distance) after `elapsedMs`; loops forever. */
export function scrollOffset(elapsedMs: number, distance: number, c: ScrollConfig = DEFAULT_SCROLL): number {
  if (!(distance > 0)) return 0;
  const cycle = scrollCycleMs(distance, c);
  let t = usable(elapsedMs) % cycle;
  if (t < c.startPauseMs) return 0;
  t -= c.startPauseMs;
  const down = travelMs(distance, c);
  if (t < down) return distance * easeInOutSine(t / down);
  t -= down;
  if (t < c.endPauseMs) return distance;
  t -= c.endPauseMs;
  const back = returnMs(distance, c);
  return distance * (1 - easeInOutSine(Math.min(1, t / back)));
}

/**
 * Offsets of the pages a long message is split into when motion is reduced:
 * one viewport at a time, the last page ending on the last line.
 */
export function pageOffsets(distance: number, viewport: number): number[] {
  const offsets = [0];
  for (let o = viewport; viewport > 0 && o < distance; o += viewport) offsets.push(o);
  if (distance > 0) offsets.push(distance);
  return offsets;
}

/** Reduced motion: no movement, the text changes page through a short fade. */
export function pagedFrame(elapsedMs: number, offsets: number[], c: ScrollConfig = DEFAULT_SCROLL): ScrollFrame {
  const n = offsets.length;
  if (n <= 1) return { offset: offsets[0] ?? 0, opacity: 1 };
  const period = c.pageDwellMs + 2 * c.pageFadeMs;
  // Shifted so the first page starts at full opacity.
  const t = (usable(elapsedMs) + c.pageFadeMs) % (n * period);
  const page = Math.floor(t / period);
  const local = t - page * period;
  const fade = c.pageFadeMs;
  if (local < fade) return { offset: offsets[page], opacity: local / fade };
  if (local < fade + c.pageDwellMs) return { offset: offsets[page], opacity: 1 };
  return { offset: offsets[page], opacity: 1 - (local - fade - c.pageDwellMs) / fade };
}

export interface ScrollInfo {
  distance: number;
  offsets: number[];
}

export function scrollFrame(
  elapsedMs: number,
  info: ScrollInfo,
  reduced: boolean,
  c: ScrollConfig = DEFAULT_SCROLL,
): ScrollFrame {
  if (!(info.distance > 0)) return { offset: 0, opacity: 1 };
  return reduced ? pagedFrame(elapsedMs, info.offsets, c) : { offset: scrollOffset(elapsedMs, info.distance, c), opacity: 1 };
}
