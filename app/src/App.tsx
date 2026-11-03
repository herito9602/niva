import { useEffect, useSyncExternalStore } from "react";
import { listen } from "@tauri-apps/api/event";
import { islandLabel } from "./island";
import { MascotCanvas } from "./mascot/MascotCanvas";
import { bindHostEvents, createViewStore, type ViewState } from "./state/viewStore";
import "./styles.css";

const store = createViewStore();

function useViewState<T>(select: (state: ViewState) => T): T {
  return useSyncExternalStore(store.subscribe, () => select(store.getState()));
}

const getCursor = () => store.getState().cursor;

export default function App() {
  const phase = useViewState((s) => s.phase);
  const view = useViewState((s) => s.view);

  useEffect(() => bindHostEvents(store, listen), []);

  return (
    <div className="stage">
      <div className={`island ${phase}`}>
        <MascotCanvas view={view} getCursor={getCursor} active={phase === "in"} />
        <span className="label">{islandLabel(view)}</span>
      </div>
    </div>
  );
}
