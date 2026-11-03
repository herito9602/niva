// Frontend mirror of what the Rust core publishes. The reducer lives in
// `niva-core`; this store only keeps the latest payload of each host event.

import { CURSOR_EVENT, ISLAND_EVENT, VIEW_EVENT } from "../events";
import type { CursorPayload, IslandPayload, ViewPayload } from "../events";
import { phaseForIsland, type SlidePhase } from "../island";

export interface ViewState {
  view: ViewPayload | null;
  phase: SlidePhase;
  cursor: CursorPayload | null;
}

export interface ViewStore {
  getState(): ViewState;
  subscribe(listener: () => void): () => void;
  setView(view: ViewPayload): void;
  setIsland(visible: boolean): void;
  setCursor(cursor: CursorPayload): void;
}

export function createViewStore(): ViewStore {
  let state: ViewState = { view: null, phase: "hidden", cursor: null };
  const listeners = new Set<() => void>();

  // A fresh object per change keeps `getState` usable as a React snapshot.
  const update = (patch: Partial<ViewState>) => {
    state = { ...state, ...patch };
    listeners.forEach((l) => l());
  };

  return {
    getState: () => state,
    subscribe(listener) {
      listeners.add(listener);
      return () => {
        listeners.delete(listener);
      };
    },
    setView: (view) => update({ view }),
    setIsland: (visible) => update({ phase: phaseForIsland(visible) }),
    setCursor: (cursor) => update({ cursor }),
  };
}

/** Shape of Tauri's `listen`, so tests can inject a fake. */
export type Listen = <T>(
  event: string,
  handler: (event: { payload: T }) => void,
) => Promise<() => void>;

/** Feeds the three host events into the store. Returns the unbind function. */
export function bindHostEvents(
  store: ViewStore,
  listen: Listen,
  onError: (error: unknown) => void = (e) => console.warn("[niva] host event binding:", e),
): () => void {
  // A failed `listen` is reported once and becomes a no-op unlisten.
  const track = (p: Promise<() => void>) =>
    p.catch((e) => {
      onError(e);
      return () => {};
    });
  const subs = [
    track(listen<ViewPayload>(VIEW_EVENT, (e) => store.setView(e.payload))),
    track(listen<IslandPayload>(ISLAND_EVENT, (e) => store.setIsland(e.payload.visible))),
    track(listen<CursorPayload>(CURSOR_EVENT, (e) => store.setCursor(e.payload))),
  ];
  return () => {
    // Unlistening waits for the subscription to resolve, so an early unbind
    // (React strict mode) never leaks a listener.
    subs.forEach((p) => p.then((un) => un()).catch(onError));
  };
}
