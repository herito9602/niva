import { useCallback, useEffect, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { VIEW_EVENT, type ViewPayload } from "../events";
import { ALL_STATES, EVENT_BUTTONS, STATE_SCRIPTS, type DebugEvent } from "./events";
import "./debug.css";

// Debug builds only: the host creates this window and registers `debug_dispatch`.
async function dispatch(event: DebugEvent): Promise<void> {
  try {
    await invoke("debug_dispatch", { event });
  } catch (err) {
    console.error("debug_dispatch failed", event, err);
  }
}

export default function DebugWindow() {
  const [view, setView] = useState<ViewPayload | null>(null);
  const [voiceReady, setVoiceReady] = useState(true);
  const [hold, setHold] = useState(true);

  useEffect(() => {
    const sub = listen<ViewPayload>(VIEW_EVENT, (e) => setView(e.payload));
    return () => {
      sub.then((un) => un());
    };
  }, []);

  // Hold the stubbed audio by default so a state stays up long enough to look at.
  useEffect(() => {
    void dispatch({ type: "set_media", held: hold });
  }, [hold]);

  const goTo = useCallback(async (state: (typeof ALL_STATES)[number]) => {
    for (const event of STATE_SCRIPTS[state]) {
      if (event.type === "voice_ready") setVoiceReady(Boolean(event.ready));
      await dispatch(event);
    }
  }, []);

  const toggleVoice = useCallback(async () => {
    const ready = !voiceReady;
    setVoiceReady(ready);
    await dispatch({ type: "voice_ready", ready });
  }, [voiceReady]);

  return (
    <main className="debug">
      <h1>Niva debug</h1>

      <section className="current">
        <span className="state" data-state={view?.state ?? "idle"}>
          {view?.state ?? "idle"}
        </span>
        <span className="caption">{view?.caption ?? ""}</span>
        {view?.confirm && <span className="extra">confirm: {view.confirm.prompt}</span>}
        {view?.reminder && <span className="extra">reminder: {view.reminder.text}</span>}
      </section>

      <section>
        <h2>Go to state (resets first)</h2>
        <div className="row">
          {ALL_STATES.map((state) => (
            <button key={state} onClick={() => goTo(state)}>
              {state}
            </button>
          ))}
        </div>
      </section>

      <section>
        <h2>Voice and media</h2>
        <div className="row">
          <button onClick={toggleVoice}>Voice: {voiceReady ? "ready (click to bring down)" : "down (click to ready)"}</button>
          <label>
            <input type="checkbox" checked={hold} onChange={(e) => setHold(e.target.checked)} />
            Hold audio (no instant SpeakDone / ChimeDone)
          </label>
          <button onClick={() => dispatch({ type: "shortcut" })}>Shortcut</button>
          <button onClick={() => dispatch({ type: "reset" })}>Reset</button>
        </div>
      </section>

      <section>
        <h2>Events</h2>
        <div className="grid">
          {EVENT_BUTTONS.map((b) => (
            <button key={b.label} onClick={() => dispatch(b.event)}>
              {b.label}
            </button>
          ))}
        </div>
      </section>
    </main>
  );
}
