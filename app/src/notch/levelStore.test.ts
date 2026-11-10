import { describe, expect, it } from "vitest";
import { LOOKS } from "./looks";
import { createLevelStore } from "./levelStore";

const bands = (v: number) => Array.from({ length: 7 }, () => v);

describe("levelStore", () => {
  it("stores_latest_levels_with_timestamp", () => {
    const store = createLevelStore();
    expect(store.push("mic", bands(10), 100)).toBe(true);
    expect(store.push("mic", bands(60), 133)).toBe(true);
    expect(store.latest("mic")).toEqual({ src: "mic", bands: bands(60), at: 133 });
    expect(store.latest("tts")).toBeNull();
  });

  it("rejects_wrong_length_or_range_bands", () => {
    const store = createLevelStore();
    expect(store.push("mic", [1, 2, 3, 4, 5, 6], 1)).toBe(false);
    expect(store.push("mic", [...bands(1), 8], 1)).toBe(false);
    expect(store.push("mic", bands(101), 1)).toBe(false);
    expect(store.push("mic", bands(-1), 1)).toBe(false);
    expect(store.push("mic", bands(Number.NaN), 1)).toBe(false);
    expect(store.push("mic", bands(1.5), 1)).toBe(false);
    expect(store.latest("mic")).toBeNull();
  });

  it("rejects an unknown source", () => {
    const store = createLevelStore();
    expect(store.push("radio" as never, bands(5), 1)).toBe(false);
  });

  it("a rejected message keeps the previous good one", () => {
    const store = createLevelStore();
    store.push("mic", bands(40), 5);
    store.push("mic", bands(500), 6);
    expect(store.latest("mic")?.at).toBe(5);
  });

  it("source_filter_mic_for_listening_tts_for_speaking", () => {
    const store = createLevelStore();
    store.push("mic", bands(20), 1);
    store.push("tts", bands(80), 2);
    expect(store.forLook(LOOKS.listening)?.src).toBe("mic");
    expect(store.forLook(LOOKS.speaking)?.src).toBe("tts");
    expect(store.forLook(LOOKS.thinking)).toBeNull();
    expect(store.forLook(LOOKS.confirmation)).toBeNull();
  });

  it("stored bands are a copy", () => {
    const store = createLevelStore();
    const input = bands(30);
    store.push("mic", input, 1);
    input[0] = 99;
    expect(store.latest("mic")?.bands[0]).toBe(30);
  });
});
