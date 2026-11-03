import type { StateName, ViewPayload } from "./events";

export type SlidePhase = "hidden" | "in" | "out";

// Fallback text while the reducer has not sent a caption.
const STATE_LABELS: Record<StateName, string> = {
  idle: "Niva",
  listening: "Listening…",
  thinking: "Thinking…",
  acting: "Working…",
  "confirmation-needed": "Confirm?",
  success: "Done",
  error: "Something went wrong",
  "reminder-alert": "Reminder",
};

export function islandLabel(view: ViewPayload | null): string {
  if (!view) return STATE_LABELS.listening;
  return view.caption ?? STATE_LABELS[view.state];
}

export function phaseForIsland(visible: boolean): SlidePhase {
  return visible ? "in" : "out";
}
