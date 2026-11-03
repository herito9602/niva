import type { StateName, ViewPayload } from "./events";

export type SlidePhase = "hidden" | "in" | "out";
export type IslandMode = "listening" | "thinking";

// Fallback text while the reducer has not sent a caption. The mascot (PR 1d)
// replaces the spike's orb; this keeps the spike's label until then.
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

/** The spike island only has two looks (orb colour and pulse speed). */
export function islandMode(state: StateName | undefined): IslandMode {
  return state === "thinking" || state === "acting" ? "thinking" : "listening";
}

export function phaseForIsland(visible: boolean): SlidePhase {
  return visible ? "in" : "out";
}
