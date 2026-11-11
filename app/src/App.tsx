import { useEffect, useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { bindIsland } from "./notch/bindEvents";
import { Notch } from "./notch/Notch";
import { levelStore } from "./notch/levelStore";
import { createViewStore, type ViewState } from "./state/viewStore";
import "./styles.css";

const store = createViewStore();

function useViewState<T>(select: (state: ViewState) => T): T {
  return useSyncExternalStore(store.subscribe, () => select(store.getState()));
}

export default function App() {
  const phase = useViewState((s) => s.phase);
  const view = useViewState((s) => s.view);

  useEffect(() => bindIsland(store, levelStore, listen), []);

  // The notch is click-through and has no buttons: nothing here talks to the host.
  return <Notch phase={phase} view={view} />;
}
