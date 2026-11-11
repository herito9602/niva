// Timing and size of the notch. The host reads this file (island_contract_tests.rs)
// to tie its window hide delay to the exit animation, so keep the constants plain
// numeric literals.

export const NOTCH_W = 210;
export const NOTCH_H = 36;
/** Drop-in: translateY(-100%) to 0 with an overshoot curve. */
export const NOTCH_IN_MS = 320;
/** Drop-out: ease-in back above the screen edge. */
export const NOTCH_OUT_MS = 200;
/** The host hides the window this long after the island leaves: exit plus slack. */
export const HIDE_DELAY_MS = 240;
/** Reduced motion: state changes and show/hide are a fade of this length. */
export const REDUCED_FADE_MS = 120;
