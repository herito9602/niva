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
 * The log line a view change deserves: a new caption (a reply, a question, an
 * error) or a new confirmation prompt. Nothing for changes that only move the
 * confirmation deadline or show no text.
 */
export function describeView(prev: ViewPayload | null, next: ViewPayload): string | null {
  const prompt = next.confirm?.prompt;
  if (prompt && prompt !== prev?.confirm?.prompt) return `confirm: ${prompt}`;
  if (next.confirm) return null;
  if (next.caption && (next.caption !== prev?.caption || next.state !== prev?.state)) {
    return `${next.state}: ${next.caption}`;
  }
  return null;
}

/** Whole seconds until `endsAt` (epoch ms), never below zero. */
export function countdownLeft(endsAt: number, nowMs: number): number {
  return Math.max(0, Math.ceil((endsAt - nowMs) / 1000));
}
