import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { createLevelStore } from "../notch/levelStore";
import { CLICK_LEN_MS, CLICK_PERIOD_MS, startLevelTest } from "./levelTest";

describe("debug level test", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("level_test_injects_a_click_every_500ms", () => {
    expect(CLICK_PERIOD_MS).toBe(500);
    const store = createLevelStore();
    const stop = startLevelTest(store, "tts", () => Date.now());
    const loud: number[] = [];
    const start = Date.now();
    for (let t = 0; t < 1900; t += 10) {
      vi.advanceTimersByTime(10);
      const sample = store.latest("tts");
      if (sample && sample.bands[0] === 100) loud.push(Date.now() - start);
    }
    stop();
    // Four clicks in 1.9 seconds, each CLICK_LEN_MS long, 500 ms apart.
    const starts = loud.filter((t, i) => i === 0 || t - loud[i - 1] > 50);
    expect(starts).toHaveLength(4);
    expect(starts[1] - starts[0]).toBeGreaterThanOrEqual(CLICK_PERIOD_MS - 40);
    expect(starts[1] - starts[0]).toBeLessThanOrEqual(CLICK_PERIOD_MS + 40);
    expect(loud.length * 10).toBeGreaterThanOrEqual(4 * (CLICK_LEN_MS - 40));
  });

  it("keeps the level fresh between clicks so the simulation never takes over", () => {
    const store = createLevelStore();
    const stop = startLevelTest(store, "tts", () => Date.now());
    for (let i = 0; i < 40; i++) {
      vi.advanceTimersByTime(25);
      const sample = store.latest("tts")!;
      expect(Date.now() - sample.at).toBeLessThan(100);
    }
    stop();
  });

  it("feeds only the tts source (never a mic level) and stops when asked", () => {
    const store = createLevelStore();
    const stop = startLevelTest(store, "tts", () => Date.now());
    vi.advanceTimersByTime(100);
    expect(store.latest("tts")).not.toBeNull();
    const last = store.latest("tts");
    stop();
    vi.advanceTimersByTime(500);
    expect(store.latest("tts")).toBe(last);
  });
});
