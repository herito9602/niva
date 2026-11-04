import { describe, expect, it } from "vitest";
import { ALL_STATES, type ViewPayload } from "./events";
import { countdownLeft, displayLabel, islandLabel, phaseForIsland } from "./island";

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

describe("sleep countdown label", () => {
  const countdown = { endsAt: 20_000, seconds: 10, text: "Suspending in {n} s. Press a key or move the mouse to cancel." };
  const counting: ViewPayload = { state: "acting", caption: "Suspending in 10 seconds. Press a key or move the mouse to cancel.", countdown };

  it("lays the pill out with the widest number, so it never resizes while counting", () => {
    expect(islandLabel(counting)).toBe("Suspending in 10 s. Press a key or move the mouse to cancel.");
    // The layout text does not depend on the time.
    expect(islandLabel({ ...counting })).toBe(islandLabel(counting));
  });

  it("counts whole seconds from the deadline and never goes below zero", () => {
    expect(countdownLeft(20_000, 10_000)).toBe(10);
    expect(countdownLeft(20_000, 10_001)).toBe(10);
    expect(countdownLeft(20_000, 11_000)).toBe(9);
    expect(countdownLeft(20_000, 19_001)).toBe(1);
    expect(countdownLeft(20_000, 20_000)).toBe(0);
    expect(countdownLeft(20_000, 25_000)).toBe(0);
  });

  it("shows the seconds left in the text on screen", () => {
    const shown = islandLabel(counting);
    expect(displayLabel(counting, shown, 10_000)).toBe("Suspending in 10 s. Press a key or move the mouse to cancel.");
    expect(displayLabel(counting, shown, 14_200)).toBe("Suspending in 6 s. Press a key or move the mouse to cancel.");
    expect(displayLabel(counting, shown, 19_500)).toBe("Suspending in 1 s. Press a key or move the mouse to cancel.");
  });

  it("never shows more than the length of the countdown, even with a clock that runs behind", () => {
    expect(displayLabel(counting, islandLabel(counting), 0)).toBe("Suspending in 10 s. Press a key or move the mouse to cancel.");
  });

  it("leaves the text alone when it is not the countdown text (another message is on screen)", () => {
    expect(displayLabel(counting, "Done.", 15_000)).toBe("Done.");
    expect(displayLabel(null, "Done.", 15_000)).toBe("Done.");
    expect(displayLabel(view("success", "Done."), "Done.", 15_000)).toBe("Done.");
  });

  it("falls back to the caption when the view has no countdown yet", () => {
    expect(islandLabel({ state: "acting", caption: "Working" })).toBe("Working");
    expect(islandLabel({ state: "acting" })).toBe("Working…");
  });
});
