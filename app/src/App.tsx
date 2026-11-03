import { useEffect, useState } from "react";
import { listen } from "@tauri-apps/api/event";
import {
  CURSOR_EVENT,
  ISLAND_EVENT,
  VIEW_EVENT,
  type CursorPayload,
  type IslandPayload,
  type ViewPayload,
} from "./events";
import { islandLabel, islandMode, phaseForIsland, type SlidePhase } from "./island";
import "./styles.css";

export default function App() {
  const [phase, setPhase] = useState<SlidePhase>("hidden");
  const [view, setView] = useState<ViewPayload | null>(null);

  useEffect(() => {
    // Every event is also logged so the host wiring is visible in the webview
    // console (PR 1d replaces this with the view store).
    const subs = [
      listen<ViewPayload>(VIEW_EVENT, (e) => {
        console.log(VIEW_EVENT, e.payload);
        setView(e.payload);
      }),
      listen<IslandPayload>(ISLAND_EVENT, (e) => {
        console.log(ISLAND_EVENT, e.payload);
        setPhase(phaseForIsland(e.payload.visible));
      }),
      listen<CursorPayload>(CURSOR_EVENT, (e) => {
        console.log(CURSOR_EVENT, e.payload.x, e.payload.y);
      }),
    ];
    return () => {
      subs.forEach((p) => p.then((un) => un()));
    };
  }, []);

  return (
    <div className="stage">
      <div className={`island ${phase} ${islandMode(view?.state)}`}>
        <div className="orb" />
        <span className="label">{islandLabel(view)}</span>
      </div>
    </div>
  );
}
