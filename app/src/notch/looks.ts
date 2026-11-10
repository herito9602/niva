// What the notch shows for each state: a color, and which audio stream (if any)
// drives the waveform. Color alone tells the states apart (owner decision). No
// text lives here except the state name that a screen reader announces.

import type { ViewPayload } from "../events";

export type LookName =
  | "listening"
  | "thinking"
  | "speaking"
  | "confirmation"
  | "success"
  | "error"
  | "reminder";

export interface Look {
  name: LookName;
  /** Fill color of the waveform: the only thing that tells the states apart. */
  color: string;
  /** Which sidecar stream drives the spike height; null means no audio (flat, breathing line). */
  source: "mic" | "tts" | null;
  /** The state name announced by the status region. Never reply text. */
  label: string;
}

export const LOOKS: Record<LookName, Look> = {
  listening: { name: "listening", color: "#378ADD", source: "mic", label: "Listening" },
  thinking: { name: "thinking", color: "#7F77DD", source: null, label: "Thinking" },
  speaking: { name: "speaking", color: "#5DCAA5", source: "tts", label: "Speaking" },
  confirmation: { name: "confirmation", color: "#EF9F27", source: null, label: "Confirmation needed" },
  success: { name: "success", color: "#97C459", source: null, label: "Done" },
  error: { name: "error", color: "#E24B4A", source: null, label: "Error" },
  reminder: { name: "reminder", color: "#F0997B", source: null, label: "Reminder" },
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
      // A sleep countdown is shown in the confirmation color; plain acting is the thinking color.
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
