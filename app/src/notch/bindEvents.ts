// Everything the island window listens to: the view and island events (through
// the view store) and the audio level (straight into the level store).

import { bindHostEvents, type Listen, type ViewStore } from "../state/viewStore";
import { bindLevelEvents } from "./levelEvents";
import type { LevelStore } from "./levelStore";

/**
 * Binds both streams. They are independent: a level stream that fails to bind
 * only costs the real waveform (the notch falls back to its simulation) and
 * never touches the view updates.
 */
export function bindIsland(
  view: ViewStore,
  levels: Pick<LevelStore, "push">,
  listen: Listen,
  now: () => number = () => performance.now(),
  onError: (error: unknown) => void = (e) => console.warn("[niva] host event binding:", e),
): () => void {
  const unbindView = bindHostEvents(view, listen, onError);
  const unbindLevels = bindLevelEvents(levels, listen, now, onError);
  return () => {
    unbindView();
    unbindLevels();
  };
}
