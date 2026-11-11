// Names and payloads of the Tauri events the Rust host emits (design: "Tauri events").

export const VIEW_EVENT = "niva://view";
export const ISLAND_EVENT = "niva://island";
/** The audio level of Niva's voice, ~30 a second while it speaks. Not part of the view. */
export const LEVEL_EVENT = "niva://level";

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

/**
 * Every state the host publishes. `ALL_STATES` lists the ones the debug window
 * can script; the sleep countdown has a state of its own and no script.
 */
export type StateName = (typeof ALL_STATES)[number] | "countdown";

/**
 * The open confirmation. The notch draws no text and no buttons: it carries the
 * id the confirm keys answer with, the deadline once the question was spoken,
 * and whether only the Ctrl+Shift+Enter chord can confirm.
 */
export interface ConfirmView {
  id: number;
  expiresAt?: number;
  /** True when only the Ctrl+Shift+Enter chord can confirm (restart, shutdown). */
  chord?: boolean;
}

/**
 * Payload of `niva://view` (design L5): the state, whether Niva's voice is
 * audible, and the details of a confirmation or a reminder. It never carries
 * reply text, labels or countdown numbers.
 */
export interface ViewPayload {
  state: StateName;
  /** True while Niva's voice is audible. A payload without it means false. */
  speaking?: boolean;
  confirm?: ConfirmView;
  /** How many reminders merged into the alert. */
  reminder?: { count: number };
}

/** Payload of `niva://level`: seven integers 0..100. Only Niva's own voice ("tts"). */
export interface LevelPayload {
  src: string;
  bands: number[];
}

export interface IslandPayload {
  visible: boolean;
}

