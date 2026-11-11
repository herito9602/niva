import { describe, expect, it, vi } from "vitest";
import { ISLAND_EVENT, LEVEL_EVENT, VIEW_EVENT } from "../events";
import { bindIsland } from "./bindEvents";
import { createViewStore, type Listen } from "../state/viewStore";
import { bindLevelEvents } from "./levelEvents";
import { createLevelStore } from "./levelStore";
import { viewToLook, type Look } from "./looks";
import { LEVEL_STALE_MS, targetLevel } from "./wave";

type Handler = (event: { payload: unknown }) => void;

/** Fake of Tauri's `listen`: records handlers, lets a test fire events or fail one name. */
function fakeListen(failFor?: string) {
  const handlers = new Map<string, Handler>();
  const unlisten = vi.fn();
  const listen: Listen = (name, handler) => {
    if (name === failFor) return Promise.reject(new Error("no such event"));
    handlers.set(name, handler as Handler);
    return Promise.resolve(unlisten);
  };
  const fire = (name: string, payload: unknown) => handlers.get(name)?.({ payload });
  return { listen, fire, unlisten, handlers };
}

const BANDS = [0, 12, 40, 55, 30, 8, 100];

describe("niva://level", () => {
  it("is the event the host emits", () => {
    expect(LEVEL_EVENT).toBe("niva://level");
  });

  it("level_event_updates_store", () => {
    const store = createLevelStore();
    const { listen, fire } = fakeListen();
    bindLevelEvents(store, listen, () => 1234);
    fire(LEVEL_EVENT, { src: "tts", bands: BANDS });
    expect(store.latest("tts")).toEqual({ src: "tts", bands: BANDS, at: 1234 });
  });

  it("stamps each level with the animation loop's clock (performance.now), not Date.now", () => {
    const store = createLevelStore();
    const { listen, fire } = fakeListen();
    bindLevelEvents(store, listen); // default clock
    const before = performance.now();
    fire(LEVEL_EVENT, { src: "tts", bands: BANDS });
    const after = performance.now();
    const at = store.latest("tts")?.at ?? NaN;
    expect(at).toBeGreaterThanOrEqual(before);
    expect(at).toBeLessThanOrEqual(after);
    expect(Math.abs(at - Date.now())).toBeGreaterThan(1e6); // the two clocks are far apart
  });

  it("a level stamped now is fresh to the wave on the very next frame, and stale after 150 ms", () => {
    const store = createLevelStore();
    const { listen, fire } = fakeListen();
    bindLevelEvents(store, listen);
    fire(LEVEL_EVENT, { src: "tts", bands: [100, 100, 100, 100, 100, 100, 100] });
    const speaking = viewToLook({ state: "listening", speaking: true }) as Look;
    const sample = store.forLook(speaking);
    expect(sample).not.toBeNull();
    const now = performance.now();
    expect(targetLevel(speaking, sample, now, false)).toBe(1); // the real level
    expect(targetLevel(speaking, sample, now + LEVEL_STALE_MS + 50, false)).not.toBe(1); // simulation took over
  });

  it("mic_source_ignored", () => {
    const store = createLevelStore();
    const { listen, fire } = fakeListen();
    bindLevelEvents(store, listen, () => 1);
    fire(LEVEL_EVENT, { src: "mic", bands: BANDS });
    expect(store.latest("tts")).toBeNull();
  });

  it("drops malformed payloads without throwing", () => {
    const store = createLevelStore();
    const { listen, fire } = fakeListen();
    bindLevelEvents(store, listen, () => 1);
    for (const payload of [null, undefined, 5, "x", {}, { src: "tts" }, { src: "tts", bands: [1, 2] }, { src: "tts", bands: Array(7).fill(101) }]) {
      expect(() => fire(LEVEL_EVENT, payload)).not.toThrow();
    }
    expect(store.latest("tts")).toBeNull();
  });

  it("listening_ignores_levels", () => {
    const store = createLevelStore();
    const { listen, fire } = fakeListen();
    bindLevelEvents(store, listen, () => 1);
    fire(LEVEL_EVENT, { src: "tts", bands: BANDS });
    // Only the speaking look reads a stream; listening, thinking and the rest read none.
    for (const state of ["listening", "thinking", "acting", "confirmation-needed", "success", "error", "reminder-alert"] as const) {
      const look = viewToLook({ state }) as Look;
      expect(look.source, state).toBeNull();
      expect(store.forLook(look), state).toBeNull();
    }
    const speaking = viewToLook({ state: "listening", speaking: true }) as Look;
    expect(store.forLook(speaking)?.bands).toEqual(BANDS);
  });

  it("unbinds the listener once it resolved", async () => {
    const store = createLevelStore();
    const { listen, unlisten } = fakeListen();
    const unbind = bindLevelEvents(store, listen);
    unbind();
    await Promise.resolve();
    await Promise.resolve();
    expect(unlisten).toHaveBeenCalledTimes(1);
  });

  it("missing_level_stream_leaves_other_view_updates_untouched", async () => {
    const view = createViewStore();
    const levels = createLevelStore();
    const onError = vi.fn();
    const { listen, fire } = fakeListen(LEVEL_EVENT);
    bindIsland(view, levels, listen, () => 1, onError);
    await Promise.resolve();
    await Promise.resolve();
    expect(onError).toHaveBeenCalledTimes(1); // the level stream failed once...
    fire(VIEW_EVENT, { state: "thinking" });
    fire(ISLAND_EVENT, { visible: true });
    expect(view.getState().view).toEqual({ state: "thinking" }); // ...and the view store carries on
    expect(view.getState().phase).toBe("in");
    expect(levels.latest("tts")).toBeNull();
  });

  it("the island binds both streams and unbinds both", async () => {
    const view = createViewStore();
    const levels = createLevelStore();
    const { listen, fire, unlisten } = fakeListen();
    const unbind = bindIsland(view, levels, listen, () => 7);
    fire(LEVEL_EVENT, { src: "tts", bands: BANDS });
    fire(VIEW_EVENT, { state: "listening", speaking: true });
    expect(levels.latest("tts")?.at).toBe(7);
    expect(view.getState().view?.speaking).toBe(true);
    unbind();
    for (let i = 0; i < 5; i++) await Promise.resolve();
    expect(unlisten).toHaveBeenCalledTimes(3);
  });
});
