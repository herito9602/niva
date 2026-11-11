import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { ISLAND_EVENT, VIEW_EVENT, type ViewPayload } from "../events";
import { LOOKS, viewToLook } from "../notch/looks";
import { ACTING_MIN_MS, bindHostEvents, createViewStore, isSpeaking, type Listen } from "./viewStore";

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
    expect(createViewStore().getState()).toEqual({ view: null, phase: "hidden" });
  });

  it("stores the latest view as received", () => {
    const store = createViewStore();
    const view: ViewPayload = { state: "thinking" };
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

  it("has no cursor state or cursor event (the window is always click-through)", () => {
    const store = createViewStore();
    expect(store).not.toHaveProperty("setCursor");
    expect(store.getState()).not.toHaveProperty("cursor");
  });

  it("notifies subscribers on change and stops after unsubscribe", () => {
    const store = createViewStore();
    const listener = vi.fn();
    const off = store.subscribe(listener);
    store.setIsland(true);
    expect(listener).toHaveBeenCalledTimes(1);
    off();
    store.setIsland(false);
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

describe("the payload the notch reads", () => {
  it("view_store_parses_trimmed_payload", () => {
    // The payload exactly as the host serializes it (design L5): no reply text,
    // no labels, no countdown numbers.
    const wire = '{"state":"confirmation-needed","speaking":false,"confirm":{"id":4,"expiresAt":1700,"chord":true}}';
    const store = createViewStore();
    const { listen, fire } = fakeListen();
    bindHostEvents(store, listen);
    fire(VIEW_EVENT, JSON.parse(wire));
    const view = store.getState().view;
    expect(view?.state).toBe("confirmation-needed");
    expect(view?.confirm).toEqual({ id: 4, expiresAt: 1700, chord: true });
    expect(isSpeaking(view)).toBe(false);
    expect(viewToLook(view)).toBe(LOOKS.confirmation);
    // A reminder carries a count only; the look has no text to draw.
    fire(VIEW_EVENT, JSON.parse('{"state":"reminder-alert","speaking":false,"reminder":{"count":3}}'));
    expect(store.getState().view?.reminder).toEqual({ count: 3 });
    expect(viewToLook(store.getState().view)).toBe(LOOKS.reminder);
    // The countdown is a state of its own.
    fire(VIEW_EVENT, JSON.parse('{"state":"countdown","speaking":false}'));
    expect(viewToLook(store.getState().view)).toBe(LOOKS.confirmation);
  });

  it("the speaking flag from the host reaches the look and drops again", () => {
    const store = createViewStore();
    const { listen, fire } = fakeListen();
    bindHostEvents(store, listen);
    fire(VIEW_EVENT, { state: "success", speaking: false });
    expect(viewToLook(store.getState().view)).toBe(LOOKS.success);
    fire(VIEW_EVENT, { state: "success", speaking: true });
    expect(viewToLook(store.getState().view)).toBe(LOOKS.speaking);
    fire(VIEW_EVENT, { state: "success", speaking: false });
    expect(viewToLook(store.getState().view)).toBe(LOOKS.success);
  });

  it("missing_speaking_defaults_false", () => {
    expect(isSpeaking({ state: "success" })).toBe(false);
    expect(isSpeaking(null)).toBe(false);
    expect(isSpeaking({ state: "success", speaking: false })).toBe(false);
    expect(isSpeaking({ state: "success", speaking: true })).toBe(true);
    expect(viewToLook({ state: "success" })).toBe(LOOKS.success);
  });
});

describe("the minimum time on screen for acting", () => {
  const acting: ViewPayload = { state: "acting" };
  const success: ViewPayload = { state: "success" };
  const shown = (store: ReturnType<typeof createViewStore>) => store.getState().view;

  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("quick_action_holds_acting_900ms_before_result", () => {
    const store = createViewStore();
    store.setView(acting);
    vi.advanceTimersByTime(40);
    store.setView(success);
    expect(shown(store)).toBe(acting);
    vi.advanceTimersByTime(ACTING_MIN_MS - 41);
    expect(shown(store)).toBe(acting);
    vi.advanceTimersByTime(1);
    expect(shown(store)).toBe(success);
  });

  it("holds an error the same way", () => {
    const store = createViewStore();
    const error: ViewPayload = { state: "error" };
    store.setView(acting);
    store.setView(error);
    expect(shown(store)).toBe(acting);
    vi.advanceTimersByTime(ACTING_MIN_MS);
    expect(shown(store)).toBe(error);
  });

  it("adds nothing when the action already took longer", () => {
    const store = createViewStore();
    store.setView(acting);
    vi.advanceTimersByTime(ACTING_MIN_MS + 1);
    store.setView(success);
    expect(shown(store)).toBe(success);
  });

  it("does not restart the minimum when another acting view arrives", () => {
    const store = createViewStore();
    store.setView(acting);
    vi.advanceTimersByTime(ACTING_MIN_MS - 100);
    store.setView({ state: "acting", speaking: false });
    store.setView(success);
    vi.advanceTimersByTime(100);
    expect(shown(store)).toBe(success);
  });

  it.each<ViewPayload>([
    { state: "reminder-alert", reminder: { count: 1 } },
    { state: "confirmation-needed", confirm: { id: 7, chord: false } },
    { state: "idle" },
    { state: "listening" },
  ])("shows $state at once", (view) => {
    const store = createViewStore();
    store.setView(acting);
    store.setView(view);
    expect(shown(store)).toBe(view);
  });

  it("reminder_during_hold_shows_at_once_and_drops_held_result", () => {
    const store = createViewStore();
    const alert: ViewPayload = { state: "reminder-alert", reminder: { count: 1 } };
    store.setView(acting);
    store.setView(success);
    store.setView(alert);
    vi.advanceTimersByTime(ACTING_MIN_MS * 2);
    expect(shown(store)).toBe(alert);
  });

  it("countdown_is_never_held", () => {
    const store = createViewStore();
    const countdown: ViewPayload = { state: "countdown" };
    store.setView(countdown);
    expect(shown(store)).toBe(countdown);
    store.setView(success);
    expect(shown(store)).toBe(success);
  });

  it("a countdown that follows acting cancels the minimum", () => {
    const store = createViewStore();
    const countdown: ViewPayload = { state: "countdown" };
    store.setView(acting);
    store.setView(countdown);
    store.setView(success);
    expect(shown(store)).toBe(success);
  });

  it("shows a confirmation at once, even while acting is still held", () => {
    const store = createViewStore();
    store.setView(acting);
    vi.advanceTimersByTime(40);
    const confirmation: ViewPayload = {
      state: "confirmation-needed",
      confirm: { id: 7, chord: false },
    };
    store.setView(confirmation);
    expect(shown(store)).toBe(confirmation);
    // And it stays: the held-result timer of an earlier view must not replace it.
    vi.advanceTimersByTime(ACTING_MIN_MS * 2);
    expect(shown(store)).toBe(confirmation);
  });

  it("a result that needs no hold is not delayed by an earlier acting", () => {
    const store = createViewStore();
    store.setView({ state: "idle" });
    store.setView(success);
    expect(shown(store)).toBe(success);
  });
});

describe("bindHostEvents", () => {
  it("routes the two host events into the store", () => {
    const store = createViewStore();
    const { listen, fire } = fakeListen();
    bindHostEvents(store, listen);
    fire(VIEW_EVENT, { state: "acting" });
    fire(ISLAND_EVENT, { visible: true });
    expect(store.getState()).toEqual({
      view: { state: "acting" },
      phase: "in",
    });
  });

  it("unlistens both subscriptions after they resolved", async () => {
    const store = createViewStore();
    const { listen, unlisten } = fakeListen();
    const unbind = bindHostEvents(store, listen);
    await flush();
    expect(unlisten).not.toHaveBeenCalled();
    unbind();
    await flush();
    expect(unlisten).toHaveBeenCalledTimes(2);
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
    expect(unlisten).toHaveBeenCalledTimes(2);
  });

  it("reports a rejected listen instead of leaving an unhandled rejection", async () => {
    const store = createViewStore();
    const onError = vi.fn();
    const listen: Listen = () => Promise.reject(new Error("no ipc"));
    const unbind = bindHostEvents(store, listen, onError);
    await flush();
    expect(onError).toHaveBeenCalledTimes(2);
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
    expect(onError).toHaveBeenCalledTimes(2);
  });
});
