import { describe, expect, it } from "vitest";
import { ALL_STATES, type StateName, type ViewPayload } from "../events";
import { LOOKS, viewToLook } from "./looks";

/** Every state the host can publish: the debug scenes plus the sleep countdown. */
const HOST_STATES: StateName[] = [...ALL_STATES, "countdown"];

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
    for (const state of HOST_STATES.filter((s) => s !== "idle")) {
      expect(viewToLook(view(state), false), state).not.toBeNull();
    }
  });

  it("only_speaking_is_level_driven_and_listening_is_calm", () => {
    const driven = Object.values(LOOKS)
      .filter((l) => l.source !== null)
      .map((l) => [l.name, l.source]);
    expect(driven).toEqual([["speaking", "tts"]]);
    expect(LOOKS.listening.source).toBeNull();
    expect(LOOKS.listening.color).toBe("#378ADD");
  });

  it("a look is only a color, a source and a state name: no motion or silhouette", () => {
    for (const look of Object.values(LOOKS)) {
      expect(Object.keys(look).sort()).toEqual(["color", "label", "name", "source"]);
    }
  });

  it("acting_uses_thinking_look", () => {
    expect(viewToLook(view("acting"), false)).toBe(LOOKS.thinking);
    expect(viewToLook(view("thinking"), false)).toBe(LOOKS.thinking);
  });

  it("countdown_uses_confirmation_look", () => {
    expect(viewToLook(view("countdown"), false)).toBe(LOOKS.confirmation);
    expect(viewToLook(view("confirmation-needed"), false)).toBe(LOOKS.confirmation);
  });

  it("countdown_payload_maps_to_confirmation_look", () => {
    // The payload as the host sends it: its own state name, no numbers, no text.
    const payload = JSON.parse('{"state":"countdown","speaking":false}') as ViewPayload;
    expect(viewToLook(payload)).toBe(LOOKS.confirmation);
    // Niva announcing the countdown is audible: the speaking look wins, then it reverts.
    const announcing = JSON.parse('{"state":"countdown","speaking":true}') as ViewPayload;
    expect(viewToLook(announcing)).toBe(LOOKS.speaking);
  });

  it("speaking_flag_from_payload_overrides_phase_look", () => {
    // Straight from the host's JSON, with no second argument: the flag alone decides.
    for (const state of HOST_STATES.filter((s) => s !== "idle")) {
      const speaking = JSON.parse(`{"state":"${state}","speaking":true}`) as ViewPayload;
      const quiet = JSON.parse(`{"state":"${state}","speaking":false}`) as ViewPayload;
      expect(viewToLook(speaking), state).toBe(LOOKS.speaking);
      expect(viewToLook(quiet), state).not.toBe(LOOKS.speaking);
    }
    // The speaking look is driven by the voice level; no phase look is.
    expect(LOOKS.speaking.source).toBe("tts");
    expect(LOOKS.speaking.color).toBe("#5DCAA5");
    // The idle notch is hidden even if a stale flag arrives with it.
    expect(viewToLook({ state: "idle", speaking: true })).toBeNull();
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
    const reminder = view("reminder-alert", { reminder: { count: 2 } });
    const look = viewToLook(reminder, false);
    expect(look).toBe(LOOKS.reminder);
    expect(JSON.stringify(look)).not.toContain("count");
  });

  it("the announced name is the state name, never reply text", () => {
    const look = viewToLook(view("success"), false);
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
