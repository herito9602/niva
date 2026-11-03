import { describe, expect, it } from "vitest";
import { ALL_STATES, type ViewPayload } from "./events";
import { islandLabel, phaseForIsland } from "./island";

const view = (state: ViewPayload["state"], caption?: string): ViewPayload => ({ state, caption });

describe("islandLabel", () => {
  it("prefers the caption", () => {
    expect(islandLabel(view("success", "Opening Notepad"))).toBe("Opening Notepad");
  });

  it("falls back to a label per state", () => {
    const labels = ALL_STATES.map((s) => islandLabel(view(s)));
    expect(new Set(labels).size).toBe(8);
    expect(islandLabel(view("listening"))).toBe("Listening…");
    expect(islandLabel(view("thinking"))).toBe("Thinking…");
  });

  it("shows the listening label before any view arrives", () => {
    expect(islandLabel(null)).toBe("Listening…");
  });
});

describe("phaseForIsland", () => {
  it("slides in when visible and out when hidden", () => {
    expect(phaseForIsland(true)).toBe("in");
    expect(phaseForIsland(false)).toBe("out");
  });
});
