import type { ViewPayload } from "../events";

/** Longest text the debug window sends (the host caps it at the same size). */
export const MAX_DEBUG_TEXT = 500;

/** Event the dry-run adapters use to report what they would have done. */
export const DEV_LOG_EVENT = "niva://dev-log";

export interface DevLogPayload {
  line: string;
}

/** The text to send for a typed command: trimmed, capped, or null if blank. */
export function prepareText(raw: string): string | null {
  const text = raw.trim();
  if (text === "") return null;
  return Array.from(text).slice(0, MAX_DEBUG_TEXT).join("");
}

/** `log` plus `entry`, keeping only the newest `max` entries. */
export function appendLog(log: readonly string[], entry: string, max = 60): string[] {
  return [...log, entry].slice(-max);
}

/**
 * The log line a view change deserves: a new state, or the voice becoming
 * audible in the same state. The view carries no reply text; what Niva says
 * arrives as its own dev-log line from the host (debug builds).
 */
export function describeView(prev: ViewPayload | null, next: ViewPayload): string | null {
  const speaking = next.speaking === true;
  if (next.state === prev?.state && speaking === (prev?.speaking === true)) return null;
  return `state: ${next.state}${speaking ? " (speaking)" : ""}`;
}
