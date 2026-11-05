/**
 * Longest command the "Type a command" window sends, in characters. The host
 * caps it at the same size (`MAX_TEXT_CHARS` in `text_window.rs`).
 */
export const MAX_COMMAND_CHARS = 500;

/** The text to send for what was typed: trimmed, capped, or null if blank. */
export function prepareCommand(raw: string): string | null {
  const text = raw.trim();
  if (text === "") return null;
  return Array.from(text).slice(0, MAX_COMMAND_CHARS).join("");
}

/** The only command this window may call (its capability grants nothing else). */
export const SUBMIT_COMMAND = "submit_text";

/** What a submit attempt came to. */
export type SubmitOutcome = "sent" | "failed" | "blank" | "busy";

/** The shape of Tauri's `invoke` this window needs (injected so it can be tested). */
export type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

/**
 * Sends commands to the host one at a time. A repeated Enter (or a click on
 * Send) while the previous command is still being handed over is ignored, so a
 * command is never sent twice. The guard is released whether the send worked or
 * not, so a failure can be retried.
 */
export function createSubmitter(invoke: Invoke): (raw: string) => Promise<SubmitOutcome> {
  let inFlight = false;
  return async (raw) => {
    if (inFlight) return "busy";
    const command = prepareCommand(raw);
    if (command === null) return "blank";
    inFlight = true;
    try {
      await invoke(SUBMIT_COMMAND, { text: command });
      return "sent";
    } catch (err) {
      console.error("could not send the command", err);
      return "failed";
    } finally {
      inFlight = false;
    }
  };
}

/**
 * What the window shows after a submit: clear the box and the error hint after
 * a send, keep the text and show the hint after a failure, and leave everything
 * as it is for a blank command or a repeated Enter (`null`).
 */
export function viewAfter(outcome: SubmitOutcome): { clear: boolean; failed: boolean } | null {
  switch (outcome) {
    case "sent":
      return { clear: true, failed: false };
    case "failed":
      return { clear: false, failed: true };
    default:
      return null;
  }
}
