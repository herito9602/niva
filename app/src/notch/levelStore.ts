// Latest audio levels, kept outside React: they arrive up to ~30 times a second
// and only the notch's animation loop reads them. A level never goes through the
// view store or the host reducer.

import type { Look } from "./looks";

/** The wire carries this many bands per sample. */
export const BAND_COUNT = 7;

/** Only Niva's own voice: no microphone level is ever sent or stored. */
export type LevelSrc = "tts";

export interface LevelSample {
  src: LevelSrc;
  /** BAND_COUNT integers, 0..100. */
  bands: number[];
  /** Clock reading (ms, the animation loop's clock) when it arrived. */
  at: number;
}

export interface LevelStore {
  /** Keeps the sample when it is well formed. Returns whether it did. */
  push(src: LevelSrc, bands: readonly number[], at: number): boolean;
  /** The latest sample of one source. */
  latest(src: LevelSrc): LevelSample | null;
  /** The latest sample of the stream that drives `look` (null for looks with none). */
  forLook(look: Look): LevelSample | null;
}

const isBand = (v: unknown): v is number => Number.isInteger(v) && (v as number) >= 0 && (v as number) <= 100;

export function createLevelStore(): LevelStore {
  const samples: Record<LevelSrc, LevelSample | null> = { tts: null };
  return {
    push(src, bands, at) {
      if (src !== "tts") return false;
      if (bands.length !== BAND_COUNT || !bands.every(isBand)) return false;
      samples[src] = { src, bands: [...bands], at };
      return true;
    },
    latest: (src) => samples[src],
    forLook: (look) => (look.source ? samples[look.source] : null),
  };
}

/** The store the notch reads and the debug window and (later) the host feed. */
export const levelStore = createLevelStore();
