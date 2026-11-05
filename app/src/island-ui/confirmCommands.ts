// The island's two commands to the host (capability `island-actions`). Each is
// as narrow as it can be: the answer carries a confirmation id and a choice,
// the report carries rectangles. The host decides what an answer is worth
// (`confirm_buttons.rs`): it drops an answer for another confirmation, a second
// answer, and a Confirm in the first half second. The names below are the Rust
// function names; a contract test (`island_contract_tests.rs`) checks them.

import type { Box } from "./layout";

export const ANSWER_COMMAND = "answer_confirmation";
export const REPORT_COMMAND = "report_confirm_buttons";

/**
 * How long the buttons stay dimmed, and Confirm inactive, after they appear.
 * The host ignores a Confirm for the same time (`ARM_MS` in `confirm_buttons.rs`);
 * a contract test keeps the two equal.
 */
export const CONFIRM_ARM_MS = 500;

/** The shape of Tauri's `invoke` (injected so it can be tested). */
export type Invoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;

/** What an answer attempt came to. "repeated" is a click on a question already answered. */
export type AnswerOutcome = "sent" | "failed" | "repeated";

export interface ConfirmActions {
  /** Answers the confirmation `id`: `accept` is Confirm, otherwise Cancel. */
  answer(id: number, accept: boolean): Promise<AnswerOutcome>;
  /** Tells the host where the buttons of confirmation `id` are (empty: none). */
  report(id: number, areas: Box[]): Promise<void>;
  /**
   * Drops the memory of the last report. Call it whenever the confirmation on
   * screen changes (or goes away): the host forgets its rectangles then too, and
   * an identical report must be sent again, not skipped.
   */
  forget(): void;
}

const sameAreas = (a: Box[], b: Box[]) =>
  a.length === b.length && a.every((r, i) => r.x === b[i].x && r.y === b[i].y && r.width === b[i].width && r.height === b[i].height);

/**
 * A confirmation is answered once. A second click on it (a double click, or
 * Cancel after Confirm) is dropped here, and the host drops it again; a failed
 * hand-over frees the question so the click can be repeated.
 */
export function createConfirmActions(invoke: Invoke): ConfirmActions {
  const taken = new Set<number>();
  let lastReport: { id: number; areas: Box[] } | null = null;

  return {
    async answer(id, accept) {
      if (taken.has(id)) return "repeated";
      taken.add(id);
      try {
        await invoke(ANSWER_COMMAND, { id, accept });
        return "sent";
      } catch (err) {
        taken.delete(id);
        console.error("could not answer the confirmation", err);
        return "failed";
      }
    },

    forget() {
      lastReport = null;
    },

    async report(id, areas) {
      if (lastReport && lastReport.id === id && sameAreas(lastReport.areas, areas)) return;
      lastReport = { id, areas };
      try {
        await invoke(REPORT_COMMAND, { id, areas });
      } catch (err) {
        lastReport = null;
        console.warn("could not report the buttons", err);
      }
    },
  };
}
