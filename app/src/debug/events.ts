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

/** One button per reducer event (several for events with variants). */
export const EVENT_BUTTONS: EventButton[] = data.buttons;

/**
 * Events that drive the reducer into each state from a fresh reset. Shared
 * with the Rust tests, which parse every entry as a `DebugEvent`.
 */
export const STATE_SCRIPTS = data.scripts as Record<StateName, DebugEvent[]>;
