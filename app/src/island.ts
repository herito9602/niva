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

const COUNTDOWN_PLACEHOLDER = "{n}";

function fillSeconds(text: string, seconds: number): string {
  return text.split(COUNTDOWN_PLACEHOLDER).join(String(seconds));
}

/** Whole seconds until `endsAt`, never below zero. */
export function countdownLeft(endsAt: number, nowMs: number): number {
  return Math.max(0, Math.ceil((endsAt - nowMs) / 1000));
}

/**
 * The message the pill is laid out for. During a sleep countdown it is the text
 * with the widest number, so the pill does not resize (and the text does not
 * crossfade) while the seconds go down; see `displayLabel`.
 */
export function islandLabel(view: ViewPayload | null): string {
  if (!view) return STATE_LABELS.listening;
  if (view.countdown) return fillSeconds(view.countdown.text, view.countdown.seconds);
  return view.caption ?? STATE_LABELS[view.state];
}

/**
 * The text drawn right now: when the message on screen is the countdown text,
 * its number is the seconds left until the deadline in the view. Anything
 * else on screen (an older message still fading out) is drawn as it is.
 */
export function displayLabel(view: ViewPayload | null, shownText: string, nowMs: number): string {
  const countdown = view?.countdown;
  if (!countdown || shownText !== fillSeconds(countdown.text, countdown.seconds)) return shownText;
  const left = Math.min(countdown.seconds, countdownLeft(countdown.endsAt, nowMs));
  return fillSeconds(countdown.text, left);
}

export function phaseForIsland(visible: boolean): SlidePhase {
  return visible ? "in" : "out";
}
