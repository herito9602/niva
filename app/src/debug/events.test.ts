import { describe, expect, it } from "vitest";
import { ALL_STATES, EVENT_BUTTONS, STATE_SCRIPTS, answerEvent, rawEvent } from "./events";

// The 22 message types of the Rust `DebugEvent` that map to reducer events
// (the 23rd kind, VoiceReadinessChanged, has two buttons: ready and down).
const EVENT_TYPES = [
  "wake",
  "shortcut",
  "speech_start",
  "transcript",
  "no_speech",
  "empty_transcript",
  "text_submitted",
  "reminder_due",
  "tool_calls",
  "tools_done",
  "reply",
  "failure",
  "ask_follow_up",
  "accept",
  "reject",
  "speak_started",
  "speak_done",
  "tts_failed",
  "watchdog",
  "chime_done",
  "voice_ready",
  "dismiss",
];

describe("debug event buttons", () => {
  it("offers at least one button for every reducer event", () => {
    const offered = new Set(EVENT_BUTTONS.map((b) => b.event.type));
    expect(EVENT_TYPES).toHaveLength(22);
    for (const type of EVENT_TYPES) {
      expect(offered.has(type), type).toBe(true);
    }
  });

  it("has unique labels", () => {
    const labels = EVENT_BUTTONS.map((b) => b.label);
    expect(new Set(labels).size).toBe(labels.length);
  });

  it("offers voice ready and voice down", () => {
    const voice = EVENT_BUTTONS.filter((b) => b.event.type === "voice_ready").map(
      (b) => b.event.ready === true,
    );
    expect(voice.sort()).toEqual([false, true]);
  });
});

describe("confirm buttons", () => {
  it("send back the id of the confirmation they were rendered for", () => {
    expect(answerEvent("accept", 12)).toEqual({ type: "accept", id: 12 });
    expect(answerEvent("reject", 12)).toEqual({ type: "reject", id: 12 });
  });
});

describe("raw accept / reject buttons", () => {
  const accept = { type: "accept" };
  it("send the id of the confirmation shown when one is open", () => {
    expect(rawEvent(accept, 7)).toEqual({ type: "accept", id: 7 });
    expect(rawEvent({ type: "reject" }, 7)).toEqual({ type: "reject", id: 7 });
  });
  it("send an id that never matches when none is open", () => {
    expect(rawEvent(accept, undefined)).toEqual({ type: "accept", id: 0 });
  });
  it("leave every other event alone", () => {
    const wake = { type: "wake" };
    expect(rawEvent(wake, 7)).toBe(wake);
  });
});

describe("state scripts", () => {
  it("has a script for each of the 8 states", () => {
    expect(ALL_STATES).toEqual([
      "idle",
      "listening",
      "thinking",
      "acting",
      "confirmation-needed",
      "success",
      "error",
      "reminder-alert",
    ]);
    for (const state of ALL_STATES) {
      expect(STATE_SCRIPTS[state].length, state).toBeGreaterThan(0);
    }
  });

  it("every script starts from a reset", () => {
    for (const state of ALL_STATES) {
      expect(STATE_SCRIPTS[state][0], state).toEqual({ type: "reset" });
    }
  });

  it("drives the reducer along the documented paths", () => {
    const types = (s: (typeof ALL_STATES)[number]) => STATE_SCRIPTS[s].map((e) => e.type);
    expect(types("idle")).toEqual(["reset"]);
    expect(types("listening")).toEqual(["reset", "voice_ready", "shortcut"]);
    expect(types("thinking")).toEqual(["reset", "voice_ready", "shortcut", "transcript"]);
    expect(types("acting")).toEqual([...types("thinking"), "tool_calls"]);
    expect(types("confirmation-needed")).toEqual([...types("thinking"), "tool_calls"]);
    expect(types("success")).toEqual([...types("thinking"), "reply"]);
    expect(types("error")).toEqual([...types("thinking"), "failure"]);
    expect(types("reminder-alert")).toEqual(["reset", "voice_ready", "reminder_due"]);
    expect(STATE_SCRIPTS.acting[STATE_SCRIPTS.acting.length - 1]).toEqual({ type: "tool_calls", verdict: "run_safe" });
    expect(STATE_SCRIPTS["confirmation-needed"][STATE_SCRIPTS["confirmation-needed"].length - 1]).toEqual({
      type: "tool_calls",
      verdict: "confirm",
    });
  });

  it("always announces the voice as ready before using it", () => {
    for (const state of ALL_STATES.filter((s) => s !== "idle")) {
      const ready = STATE_SCRIPTS[state].find((e) => e.type === "voice_ready");
      expect(ready, state).toEqual({ type: "voice_ready", ready: true });
    }
  });
});
