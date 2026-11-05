// The pure parts of the confirmation buttons, so they can be tested without a
// DOM: what the island reports to the host, the size the buttons are drawn
// with, and the timer that arms Confirm.

import { CONFIRM_ARM_MS } from "./confirmCommands";
import { BUTTON_HEIGHT, BUTTON_WIDTH, confirmButtonRects, type Box, type Layout } from "./layout";

/** The size of each button, from the same constants the reported rectangles use. */
export const buttonSize = { width: BUTTON_WIDTH, height: BUTTON_HEIGHT };

/**
 * The page keeps the buttons dimmed a little longer than the host's guard. The
 * host starts its half second when the first report arrives, a moment after the
 * buttons mount, so with this slack the page never offers a Confirm the host
 * would still drop. The host stays the authority: this is only what is shown.
 */
export const ARM_SLACK_MS = 150;
export const DIM_MS = CONFIRM_ARM_MS + ARM_SLACK_MS;

export interface Timers {
  set: (fn: () => void, ms: number) => unknown;
  clear: (handle: unknown) => void;
}

const realTimers: Timers = {
  set: (fn, ms) => setTimeout(fn, ms),
  clear: (handle) => clearTimeout(handle as ReturnType<typeof setTimeout>),
};

/** Calls `onArmed` once after the dim period. Returns the function that cancels it. */
export function createArmTimer(onArmed: () => void, delayMs: number = DIM_MS, timers: Timers = realTimers): () => void {
  const handle = timers.set(onArmed, delayMs);
  return () => timers.clear(handle);
}

interface ReportInput {
  confirmId: number | undefined;
  buttonsOn: boolean;
  layout: Layout;
  stageWidth: number;
}

/**
 * What to tell the host about the buttons, or null when there is no
 * confirmation. Not showing the buttons reports an empty list, which releases
 * the capture.
 */
export function reportFor({ confirmId, buttonsOn, layout, stageWidth }: ReportInput): { id: number; areas: Box[] } | null {
  if (confirmId === undefined) return null;
  if (!buttonsOn) return { id: confirmId, areas: [] };
  const { cancel, confirm } = confirmButtonRects(layout, stageWidth);
  return { id: confirmId, areas: [cancel, confirm] };
}
