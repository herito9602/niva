import { describe, expect, it } from "vitest";
import { LOOKS } from "./looks";
import { createLevelStore } from "./levelStore";

const bands = (v: number) => Array.from({ length: 7 }, () => v);

describe("levelStore", () => {
  it("stores_latest_levels_with_timestamp", () => {
    const store = createLevelStore();
    expect(store.latest("tts")).toBeNull();
    expect(store.push("tts", bands(10), 100)).toBe(true);
    expect(store.push("tts", bands(60), 133)).toBe(true);
    expect(store.latest("tts")).toEqual({ src: "tts", bands: bands(60), at: 133 });
  });

  it("rejects_wrong_length_or_range_bands", () => {
    const store = createLevelStore();
    expect(store.push("tts", [1, 2, 3, 4, 5, 6], 1)).toBe(false);
    expect(store.push("tts", [...bands(1), 8], 1)).toBe(false);
    expect(store.push("tts", bands(101), 1)).toBe(false);
    expect(store.push("tts", bands(-1), 1)).toBe(false);
    expect(store.push("tts", bands(Number.NaN), 1)).toBe(false);
    expect(store.push("tts", bands(1.5), 1)).toBe(false);
    expect(store.latest("tts")).toBeNull();
  });

  it("rejects an unknown source", () => {
    const store = createLevelStore();
    expect(store.push("radio" as never, bands(5), 1)).toBe(false);
    // No microphone level ever enters the store.
    expect(store.push("mic" as never, bands(5), 1)).toBe(false);
  });

  it("a rejected message keeps the previous good one", () => {
    const store = createLevelStore();
    store.push("tts", bands(40), 5);
    store.push("tts", bands(500), 6);
    expect(store.latest("tts")?.at).toBe(5);
  });

  it("source_filter_tts_for_speaking_only", () => {
    const store = createLevelStore();
    store.push("tts", bands(80), 2);
    expect(store.forLook(LOOKS.listening)).toBeNull();
    expect(store.forLook(LOOKS.speaking)?.src).toBe("tts");
    expect(store.forLook(LOOKS.thinking)).toBeNull();
    expect(store.forLook(LOOKS.confirmation)).toBeNull();
  });

  it("stored bands are a copy", () => {
    const store = createLevelStore();
    const input = bands(30);
    store.push("tts", input, 1);
    input[0] = 99;
    expect(store.latest("tts")?.bands[0]).toBe(30);
  });
});
