import { describe, expect, it, vi } from "vitest";
import { CURSOR_EVENT, ISLAND_EVENT, VIEW_EVENT, type ViewPayload } from "../events";
import { bindHostEvents, createViewStore, type Listen } from "./viewStore";

type Handler = (event: { payload: unknown }) => void;

/** Lets queued promise callbacks (a few chained `then`s) run. */
const flush = async () => {
  for (let i = 0; i < 5; i++) await Promise.resolve();
};

/** Fake of Tauri's `listen`: records handlers and lets a test fire events. */
function fakeListen() {
  const handlers = new Map<string, Handler>();
  const unlisten = vi.fn();
  const listen: Listen = (name, handler) => {
    handlers.set(name, handler as Handler);
    return Promise.resolve(unlisten);
  };
  const fire = (name: string, payload: unknown) => handlers.get(name)?.({ payload });
  return { listen, fire, unlisten, handlers };
}

describe("createViewStore", () => {
  it("starts empty and hidden", () => {
    expect(createViewStore().getState()).toEqual({ view: null, phase: "hidden", cursor: null });
  });

  it("stores the latest view as received", () => {
    const store = createViewStore();
    const view: ViewPayload = { state: "thinking", caption: "Which one?" };
    store.setView(view);
    expect(store.getState().view).toBe(view);
    store.setView({ state: "idle" });
    expect(store.getState().view).toEqual({ state: "idle" });
  });

  it("maps island visibility to a slide phase", () => {
    const store = createViewStore();
    store.setIsland(true);
    expect(store.getState().phase).toBe("in");
    store.setIsland(false);
    expect(store.getState().phase).toBe("out");
  });

  it("stores the cursor", () => {
    const store = createViewStore();
    store.setCursor({ x: 10, y: -4 });
    expect(store.getState().cursor).toEqual({ x: 10, y: -4 });
  });

  it("notifies subscribers on change and stops after unsubscribe", () => {
    const store = createViewStore();
    const listener = vi.fn();
    const off = store.subscribe(listener);
    store.setCursor({ x: 1, y: 2 });
    expect(listener).toHaveBeenCalledTimes(1);
    off();
    store.setCursor({ x: 3, y: 4 });
    expect(listener).toHaveBeenCalledTimes(1);
  });

  it("replaces the state object on every change (snapshot identity)", () => {
    const store = createViewStore();
    const before = store.getState();
    store.setIsland(true);
    expect(store.getState()).not.toBe(before);
    expect(before.phase).toBe("hidden");
  });
});

describe("bindHostEvents", () => {
  it("routes the three host events into the store", () => {
    const store = createViewStore();
    const { listen, fire } = fakeListen();
    bindHostEvents(store, listen);
    fire(VIEW_EVENT, { state: "acting" });
    fire(ISLAND_EVENT, { visible: true });
    fire(CURSOR_EVENT, { x: 5, y: 6 });
    expect(store.getState()).toEqual({
      view: { state: "acting" },
      phase: "in",
      cursor: { x: 5, y: 6 },
    });
  });

  it("unlistens all three subscriptions after they resolved", async () => {
    const store = createViewStore();
    const { listen, unlisten } = fakeListen();
    const unbind = bindHostEvents(store, listen);
    await flush();
    expect(unlisten).not.toHaveBeenCalled();
    unbind();
    await flush();
    expect(unlisten).toHaveBeenCalledTimes(3);
  });

  it("unbinding before listen resolves still unlistens once they do", async () => {
    const store = createViewStore();
    const unlisten = vi.fn();
    const resolvers: Array<(un: () => void) => void> = [];
    const listen: Listen = () => new Promise((resolve) => resolvers.push(resolve));
    const unbind = bindHostEvents(store, listen);
    unbind(); // nothing has resolved yet
    await flush();
    expect(unlisten).not.toHaveBeenCalled();
    resolvers.forEach((resolve) => resolve(unlisten)); // the late resolution
    await flush();
    expect(unlisten).toHaveBeenCalledTimes(3);
  });

  it("reports a rejected listen instead of leaving an unhandled rejection", async () => {
    const store = createViewStore();
    const onError = vi.fn();
    const listen: Listen = () => Promise.reject(new Error("no ipc"));
    const unbind = bindHostEvents(store, listen, onError);
    await flush();
    expect(onError).toHaveBeenCalledTimes(3);
    unbind(); // must not throw or reject
    await flush();
  });

  it("reports an unlisten that throws", async () => {
    const store = createViewStore();
    const onError = vi.fn();
    const listen: Listen = () =>
      Promise.resolve(() => {
        throw new Error("already gone");
      });
    const unbind = bindHostEvents(store, listen, onError);
    await flush();
    unbind();
    await flush();
    expect(onError).toHaveBeenCalledTimes(3);
  });
});
