import { describe, expect, it } from "vitest";
import { ALL_STATES, type StateName, type ViewPayload } from "../events";
import { LOOKS, viewToLook } from "./looks";

const view = (state: StateName, extra: Partial<ViewPayload> = {}): ViewPayload => ({
  state,
  ...extra,
});

describe("look table", () => {
  it("every_state_has_look_with_design_color", () => {
    const expected: Record<string, string> = {
      listening: "#378ADD",
      thinking: "#7F77DD",
      speaking: "#5DCAA5",
      confirmation: "#EF9F27",
      success: "#97C459",
      error: "#E24B4A",
      reminder: "#F0997B",
    };
    expect(Object.keys(LOOKS).sort()).toEqual(Object.keys(expected).sort());
    for (const [name, color] of Object.entries(expected)) {
      expect(LOOKS[name as keyof typeof LOOKS].color.toLowerCase()).toBe(color.toLowerCase());
    }
    // Every non-idle state of the host maps to one of the seven looks.
    for (const state of ALL_STATES.filter((s) => s !== "idle")) {
      expect(viewToLook(view(state), false), state).not.toBeNull();
    }
  });

  it("reduced_motion_silhouettes_pairwise_distinct", () => {
    const looks = Object.values(LOOKS);
    expect(looks).toHaveLength(7);
    const keys = looks.map((l) => {
      expect(l.silhouette).toHaveLength(7);
      return l.silhouette.join(",");
    });
    expect(new Set(keys).size).toBe(7);
  });

  it("silhouettes are normalized to 0..1", () => {
    for (const look of Object.values(LOOKS)) {
      for (const h of look.silhouette) {
        expect(h).toBeGreaterThanOrEqual(0);
        expect(h).toBeLessThanOrEqual(1);
      }
    }
  });

  it("acting_uses_thinking_look", () => {
    expect(viewToLook(view("acting"), false)).toBe(LOOKS.thinking);
    expect(viewToLook(view("thinking"), false)).toBe(LOOKS.thinking);
  });

  it("countdown_uses_confirmation_look", () => {
    const countdown = view("acting", { countdown: { endsAt: 0, seconds: 10, text: "Sleeping in {n}" } });
    expect(viewToLook(countdown, false)).toBe(LOOKS.confirmation);
    expect(viewToLook(view("confirmation-needed"), false)).toBe(LOOKS.confirmation);
  });

  it("speaking_overrides_phase_look_and_reverts", () => {
    const success = view("success");
    expect(viewToLook(success, true)).toBe(LOOKS.speaking);
    expect(viewToLook(success, false)).toBe(LOOKS.success);
  });

  it("speaking flag defaults to the payload and a missing flag is false", () => {
    expect(viewToLook(view("success", { speaking: true }))).toBe(LOOKS.speaking);
    expect(viewToLook(view("success"))).toBe(LOOKS.success);
  });

  it("idle_is_hidden", () => {
    expect(viewToLook(view("idle"), false)).toBeNull();
    expect(viewToLook(view("idle"), true)).toBeNull();
    expect(viewToLook(null, false)).toBeNull();
  });

  it("reminder_alert_look_and_no_text", () => {
    const reminder = view("reminder-alert", { caption: "Call mom", reminder: { id: 1, text: "Call mom" } });
    const look = viewToLook(reminder, false);
    expect(look).toBe(LOOKS.reminder);
    expect(JSON.stringify(look)).not.toContain("Call mom");
  });

  it("the announced name is the state name, never reply text", () => {
    const look = viewToLook(view("success", { caption: "Opening Notepad" }), false);
    expect(look?.label).toBe("Done");
    for (const l of Object.values(LOOKS)) expect(l.label).toMatch(/^[A-Za-z ]+$/);
  });
});

describe("notch assets", () => {
  it("notch_has_no_image_assets", () => {
    // The notch is drawn shapes only. Walk app/src and app/public for image files.
    const images = {
      ...import.meta.glob("/src/**/*.{png,jpg,jpeg,svg,webp,gif}"),
      ...import.meta.glob("/public/**/*.{png,jpg,jpeg,svg,webp,gif}"),
    };
    expect(Object.keys(images)).toEqual([]);
  });

  it("the old mascot and island modules are gone", () => {
    const old = import.meta.glob(["/src/mascot/**", "/src/island-ui/**", "/src/island.ts", "/src/island.test.ts"]);
    expect(Object.keys(old)).toEqual([]);
  });
});
