import { useEffect, useSyncExternalStore } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { createConfirmActions } from "./island-ui/confirmCommands";
import { Island } from "./island-ui/Island";
import { bindHostEvents, createViewStore, type ViewState } from "./state/viewStore";
import "./styles.css";

const store = createViewStore();

function useViewState<T>(select: (state: ViewState) => T): T {
  return useSyncExternalStore(store.subscribe, () => select(store.getState()));
}

const getCursor = () => store.getState().cursor;

// The island's two host commands (capability `island-actions`).
const actions = createConfirmActions(invoke);

export default function App() {
  const phase = useViewState((s) => s.phase);
  const view = useViewState((s) => s.view);

  useEffect(() => bindHostEvents(store, listen), []);

  return <Island phase={phase} view={view} getCursor={getCursor} actions={actions} />;
}
