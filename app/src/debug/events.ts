import data from "./events.json";
import { ALL_STATES, type StateName } from "../events";

export { ALL_STATES };

/** A message for the Rust `debug_dispatch` command (the `DebugEvent` enum). */
export interface DebugEvent {
  type: string;
  [field: string]: unknown;
}

export interface EventButton {
  label: string;
  event: DebugEvent;
}

/**
 * The message a Confirm / Cancel button sends: it carries the id of the
 * confirmation the view showed, so a click that arrives after that prompt was
 * replaced by the next one is dropped by the reducer.
 */
export function answerEvent(type: "accept" | "reject", confirmId: number): DebugEvent {
  return { type, id: confirmId };
}

/**
 * What a raw event button sends. Accept / Reject carry the id of the
 * confirmation shown now (`undefined` when none is open: id 0, which no
 * prompt ever has), so the click can never land on a later prompt.
 */
export function rawEvent(event: DebugEvent, openConfirmId: number | undefined): DebugEvent {
  if (event.type !== "accept" && event.type !== "reject") return event;
  return { ...event, id: openConfirmId ?? 0 };
}

/** One button per reducer event (several for events with variants). */
export const EVENT_BUTTONS: EventButton[] = data.buttons;

/**
 * Events that drive the reducer into each state from a fresh reset. Shared
 * with the Rust tests, which parse every entry as a `DebugEvent`.
 */
export const STATE_SCRIPTS = data.scripts as Record<StateName, DebugEvent[]>;
