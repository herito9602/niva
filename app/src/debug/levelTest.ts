// Debug "level test": feeds the notch's level store with a click every 500 ms
// (full level for a moment, then silence) so the waveform lag can be filmed at 240 fps.
// The level stays fresh between clicks, so the UI's simulation never takes over.

import type { LevelSrc, LevelStore } from "../notch/levelStore";

export const CLICK_PERIOD_MS = 500;
export const CLICK_LEN_MS = 100;
export const PUSH_EVERY_MS = 25;

/** Starts the click train for Niva's voice (tts); there is no microphone source. Returns the stop function. */
export function startLevelTest(
  store: Pick<LevelStore, "push">,
  src: LevelSrc,
  now: () => number = () => performance.now(),
): () => void {
  const t0 = now();
  const id = setInterval(() => {
    const t = now();
    const loud = (t - t0) % CLICK_PERIOD_MS < CLICK_LEN_MS;
    store.push(src, Array(7).fill(loud ? 100 : 0), t);
  }, PUSH_EVERY_MS);
  return () => clearInterval(id);
}
