// Feeds `niva://level` into the level store. Levels bypass the view store and
// the host reducer: they arrive ~30 times a second and only the animation loop
// reads them.

import { LEVEL_EVENT, type LevelPayload } from "../events";
import type { Listen } from "../state/viewStore";
import type { LevelStore } from "./levelStore";

/**
 * Subscribes to the level event. Each sample is stamped with `now()`, which must be
 * the animation loop's clock (`performance.now`): the wave compares it with the
 * time of its own frames to tell a fresh level from a stale one (150 ms).
 * A malformed payload or a source other than "tts" is dropped by the store.
 * Returns the unbind function.
 */
export function bindLevelEvents(
  store: Pick<LevelStore, "push">,
  listen: Listen,
  now: () => number = () => performance.now(),
  onError: (error: unknown) => void = (e) => console.warn("[niva] level event binding:", e),
): () => void {
  const sub = listen<LevelPayload>(LEVEL_EVENT, (e) => {
    const payload = e.payload;
    if (payload && Array.isArray(payload.bands)) store.push(payload.src as "tts", payload.bands, now());
  }).catch((e) => {
    onError(e);
    return () => {};
  });
  return () => {
    sub.then((un) => un()).catch(onError);
  };
}
