import { describe, expect, it } from "vitest";
import type { ViewPayload } from "../events";
import { MAX_DEBUG_TEXT, appendLog, describeView, prepareText } from "./log";

const view = (over: Partial<ViewPayload>): ViewPayload => ({ state: "idle", ...over });

describe("prepareText", () => {
  it("trims the text and drops a blank one", () => {
    expect(prepareText("  open notepad \n")).toBe("open notepad");
    expect(prepareText("   ")).toBeNull();
    expect(prepareText("")).toBeNull();
  });

  it("caps the text at the host's limit, in characters", () => {
    expect(MAX_DEBUG_TEXT).toBe(500);
    const long = "á".repeat(MAX_DEBUG_TEXT + 40);
    expect(Array.from(prepareText(long) ?? "")).toHaveLength(MAX_DEBUG_TEXT);
  });
});

describe("appendLog", () => {
  it("appends and keeps only the newest entries", () => {
    expect(appendLog(["a"], "b", 5)).toEqual(["a", "b"]);
    expect(appendLog(["a", "b", "c"], "d", 3)).toEqual(["b", "c", "d"]);
  });

  it("does not mutate the previous log", () => {
    const before = ["a"];
    appendLog(before, "b", 5);
    expect(before).toEqual(["a"]);
  });
});

describe("describeView", () => {
  it("reports a new caption with its state", () => {
    expect(describeView(null, view({ state: "success", caption: "Opened notepad." }))).toBe(
      "success: Opened notepad.",
    );
  });

  it("reports a confirmation prompt once, as a confirmation", () => {
    const asked = view({
      state: "confirmation-needed",
      caption: "Close notepad.exe?",
      confirm: { id: 3, prompt: "Close notepad.exe?" },
    });
    expect(describeView(null, asked)).toBe("confirm: Close notepad.exe?");
    const withDeadline = view({
      ...asked,
      confirm: { id: 3, prompt: "Close notepad.exe?", expiresAt: 5000 },
    });
    expect(describeView(asked, withDeadline)).toBeNull();
  });

  it("says nothing when nothing new is shown", () => {
    const same = view({ state: "success", caption: "Done." });
    expect(describeView(same, same)).toBeNull();
    expect(describeView(null, view({ state: "listening" }))).toBeNull();
  });

  it("reports the same caption again when a new reply repeats it in a new state", () => {
    const a = view({ state: "success", caption: "Done." });
    const b = view({ state: "idle" });
    expect(describeView(a, b)).toBeNull();
    expect(describeView(b, a)).toBe("success: Done.");
  });
});
