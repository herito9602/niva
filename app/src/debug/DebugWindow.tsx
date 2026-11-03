import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { VIEW_EVENT, type ViewPayload } from "../events";
import { ALL_STATES, EVENT_BUTTONS, STATE_SCRIPTS, answerEvent, rawEvent, type DebugEvent } from "./events";
import {
  DEV_LOG_EVENT,
  MAX_DEBUG_TEXT,
  appendLog,
  describeView,
  prepareText,
  type DevLogPayload,
} from "./log";
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
  const [text, setText] = useState("");
  const [log, setLog] = useState<string[]>([]);
  const lastView = useRef<ViewPayload | null>(null);

  useEffect(() => {
    const subs = [
      listen<ViewPayload>(VIEW_EVENT, (e) => {
        const line = describeView(lastView.current, e.payload);
        lastView.current = e.payload;
        setView(e.payload);
        if (line) setLog((old) => appendLog(old, line));
      }),
      // What the dry-run PC and the dev LLM report (debug builds only).
      listen<DevLogPayload>(DEV_LOG_EVENT, (e) => setLog((old) => appendLog(old, e.payload.line))),
    ];
    return () => {
      for (const sub of subs) sub.then((un) => un());
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

  // A typed command goes in as TextSubmitted, like the release text window will.
  const send = useCallback(async () => {
    const command = prepareText(text);
    if (command === null) return;
    setText("");
    setLog((old) => appendLog(old, `you: ${command}`));
    await dispatch({ type: "text_submitted", text: command });
  }, [text]);

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
        <h2>Type a command</h2>
        <form
          className="row"
          onSubmit={(e) => {
            e.preventDefault();
            void send();
          }}
        >
          <input
            type="text"
            className="command"
            value={text}
            maxLength={MAX_DEBUG_TEXT}
            placeholder="open notepad, close spotify, volume 30, remind me to call mom..."
            onChange={(e) => setText(e.target.value)}
            autoFocus
          />
          <button type="submit">Send</button>
        </form>
        {view?.state === "confirmation-needed" && view.confirm && (
          <div className="row confirm">
            <button
              className="yes"
              onClick={() => dispatch(answerEvent("accept", view.confirm!.id))}
            >
              Confirm (click)
            </button>
            <button onClick={() => dispatch(answerEvent("reject", view.confirm!.id))}>
              Cancel (click)
            </button>
            <span className="extra">Restart and shutdown can only be confirmed by click.</span>
          </div>
        )}
      </section>

      <section>
        <h2>Activity</h2>
        <ol className="log">
          {log.map((line, i) => (
            <li key={i}>{line}</li>
          ))}
        </ol>
        <button onClick={() => setLog([])}>Clear</button>
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
            <button key={b.label} onClick={() =>
                dispatch(
                  rawEvent(b.event, view?.state === "confirmation-needed" ? view.confirm?.id : undefined),
                )
              }>
              {b.label}
            </button>
          ))}
        </div>
      </section>
    </main>
  );
}
