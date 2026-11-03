// Phase logic of the island's animation. The shape is driven by springs in
// edge.ts; the content fades live in styles.css (as fractions of --in-ms /
// --out-ms); this module owns the durations and the decisions around them.
// `EDGE_OUT_MS` is also read by a Rust test (`island_contract_tests.rs`): the
// host hides the window `HIDE_DELAY_MS` after `niva://island {visible:false}`,
// so it must never be shorter than the exit animation. Keep the durations as
// plain numeric literals, and in sync with the CSS (a test checks that).

import type { SlidePhase } from "../island";

/** The shape has settled and the content has finished fading in. */
export const EDGE_IN_MS = 900;
/** The shape is back in the edge. */
export const EDGE_OUT_MS = 380;
/** Fade of the old text before a new message replaces it. */
export const SWAP_FADE_MS = 130;

export type TextPlan = "keep" | "swap-now" | "crossfade";

interface TextChange {
  shownText: string;
  target: string;
  phase: SlidePhase;
  prevPhase: SlidePhase;
}

/**
 * What to do when the message the host wants may differ from the one on screen.
 * While the island leaves, the text is frozen: the reducer goes back to idle
 * right after hiding and that label must not flash during the exit.
 */
export function planTextChange({ shownText, target, phase, prevPhase }: TextChange): TextPlan {
  if (phase === "out" || target === shownText) return "keep";
  return prevPhase === "in" ? "crossfade" : "swap-now";
}
