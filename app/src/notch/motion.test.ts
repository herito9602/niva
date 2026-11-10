import { describe, expect, it } from "vitest";
import {
  HIDE_DELAY_MS,
  NOTCH_H,
  NOTCH_OUT_MS,
  NOTCH_W,
  REDUCED_FADE_MS,
} from "./motion";

describe("notch motion constants", () => {
  it("notch_is_118_by_30", () => {
    expect(NOTCH_W).toBe(118);
    expect(NOTCH_H).toBe(30);
  });

  it("hide_delay_within_exit_plus_100", () => {
    expect(HIDE_DELAY_MS).toBeGreaterThanOrEqual(NOTCH_OUT_MS);
    expect(HIDE_DELAY_MS).toBeLessThanOrEqual(NOTCH_OUT_MS + 100);
  });

  it("reduced_motion_fade_below_hide_delay", () => {
    expect(REDUCED_FADE_MS).toBe(120);
    expect(REDUCED_FADE_MS).toBeLessThan(HIDE_DELAY_MS);
  });
});
