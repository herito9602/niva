// Names and payloads of the Tauri events the Rust host emits (design: "Tauri events").

export const VIEW_EVENT = "niva://view";
export const ISLAND_EVENT = "niva://island";
export const CURSOR_EVENT = "niva://cursor";

export const ALL_STATES = [
  "idle",
  "listening",
  "thinking",
  "acting",
  "confirmation-needed",
  "success",
  "error",
  "reminder-alert",
] as const;

export type StateName = (typeof ALL_STATES)[number];

/**
 * The open confirmation. The island shows two buttons with these labels (in the
 * request's language) and answers with `id`, never with a number of its own.
 */
export interface ConfirmView {
  id: number;
  prompt: string;
  confirmLabel: string;
  cancelLabel: string;
  expiresAt?: number;
}

export interface ViewPayload {
  state: StateName;
  caption?: string;
  confirm?: ConfirmView;
  reminder?: { id: number; text: string };
  /**
   * A sleep countdown (shown with the `acting` pose). The island counts down
   * from `endsAt` (epoch ms); the host sends no event per second. `text` is the
   * localized message and `{n}` stands for the whole seconds left; `seconds` is
   * the length of the countdown, i.e. the widest number.
   */
  countdown?: { endsAt: number; seconds: number; text: string };
}

export interface IslandPayload {
  visible: boolean;
}

/** Logical pixels relative to the island window's top-left corner. */
export interface CursorPayload {
  x: number;
  y: number;
}
