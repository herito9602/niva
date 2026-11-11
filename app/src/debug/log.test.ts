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
  it("reports a state change", () => {
    expect(describeView(null, view({ state: "success" }))).toBe("state: success");
    expect(describeView(view({ state: "success" }), view({ state: "idle" }))).toBe("state: idle");
  });

  it("reports when Niva's voice becomes audible and when it stops", () => {
    const quiet = view({ state: "success", speaking: false });
    const loud = view({ state: "success", speaking: true });
    expect(describeView(quiet, loud)).toBe("state: success (speaking)");
    expect(describeView(loud, quiet)).toBe("state: success");
  });

  it("says nothing when nothing changed, and never any reply text", () => {
    const same = view({ state: "success" });
    expect(describeView(same, same)).toBeNull();
    const asked = view({ state: "confirmation-needed", confirm: { id: 3, chord: false } });
    expect(describeView(asked, view({ ...asked, confirm: { id: 3, chord: false, expiresAt: 5000 } }))).toBeNull();
  });
});
