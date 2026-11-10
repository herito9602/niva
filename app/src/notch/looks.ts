// What the notch shows for each state: a color, a kind of motion and a static
// bar silhouette for reduced motion. Every silhouette differs, so the states can
// be told apart without color. No text lives here except the state name that a
// screen reader announces.

import type { ViewPayload } from "../events";

export type LookName =
  | "listening"
  | "thinking"
  | "speaking"
  | "confirmation"
  | "success"
  | "error"
  | "reminder";

export type MotionKind = "levels" | "wave" | "pulse" | "bounce" | "jitter" | "outward";

export interface Look {
  name: LookName;
  /** Bar color. */
  color: string;
  motion: MotionKind;
  /** Which sidecar stream drives the bars, when there is one. */
  source: "mic" | "tts" | null;
  /** Static bar heights (0..1) under reduced motion. */
  silhouette: readonly number[];
  /** The state name announced by the status region. Never reply text. */
  label: string;
}

/** Design silhouettes are written on a 0..8 scale. */
const sil = (...heights: number[]): readonly number[] => heights.map((h) => h / 8);

export const LOOKS: Record<LookName, Look> = {
  listening: {
    name: "listening",
    color: "#378ADD",
    motion: "levels",
    source: "mic",
    silhouette: sil(2, 4, 6, 8, 6, 4, 2),
    label: "Listening",
  },
  thinking: {
    name: "thinking",
    color: "#7F77DD",
    motion: "wave",
    source: null,
    silhouette: sil(1, 2, 3, 4, 5, 6, 7),
    label: "Thinking",
  },
  speaking: {
    name: "speaking",
    color: "#5DCAA5",
    motion: "levels",
    source: "tts",
    silhouette: sil(5, 2, 5, 2, 5, 2, 5),
    label: "Speaking",
  },
  confirmation: {
    name: "confirmation",
    color: "#EF9F27",
    motion: "pulse",
    source: null,
    silhouette: sil(1, 1, 8, 1, 8, 1, 1),
    label: "Confirmation needed",
  },
  success: {
    name: "success",
    color: "#97C459",
    motion: "bounce",
    source: null,
    silhouette: sil(7, 7, 7, 7, 7, 7, 7),
    label: "Done",
  },
  error: {
    name: "error",
    color: "#E24B4A",
    motion: "jitter",
    source: null,
    silhouette: sil(2, 2, 2, 0, 2, 2, 2),
    label: "Error",
  },
  reminder: {
    name: "reminder",
    color: "#F0997B",
    motion: "outward",
    source: null,
    silhouette: sil(8, 6, 3, 1, 3, 6, 8),
    label: "Reminder",
  },
};

function phaseLook(view: ViewPayload): Look | null {
  switch (view.state) {
    case "idle":
      return null;
    case "listening":
      return LOOKS.listening;
    case "thinking":
      return LOOKS.thinking;
    case "acting":
      // A sleep countdown is shown as a confirmation pulse; plain acting is the thinking wave.
      return view.countdown ? LOOKS.confirmation : LOOKS.thinking;
    case "confirmation-needed":
      return LOOKS.confirmation;
    case "success":
      return LOOKS.success;
    case "error":
      return LOOKS.error;
    case "reminder-alert":
      return LOOKS.reminder;
  }
}

/**
 * The look for a view, or `null` when nothing should show (idle). While the
 * voice is audible the speaking look overrides the phase look. A missing
 * `speaking` flag is false.
 */
export function viewToLook(
  view: ViewPayload | null,
  speaking: boolean = view?.speaking === true,
): Look | null {
  if (!view) return null;
  const phase = phaseLook(view);
  if (!phase) return null;
  return speaking ? LOOKS.speaking : phase;
}

/**
 * The look to draw: the current one, else the last one shown (the view is
 * already idle while the notch slides out), else listening.
 */
export function holdLook(current: Look | null, last: Look | null): Look {
  return current ?? last ?? LOOKS.listening;
}
