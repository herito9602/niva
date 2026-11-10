import { useCallback, useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
import { VIEW_EVENT, type ViewPayload } from "../events";
import { Notch } from "../notch/Notch";
import { levelStore, type LevelSrc } from "../notch/levelStore";
import { viewToLook } from "../notch/looks";
import { useNow } from "../state/useNow";
import { ALL_STATES, EVENT_BUTTONS, STATE_SCRIPTS, answerEvent, rawEvent, type DebugEvent } from "./events";
import {
  DEV_LOG_EVENT,
  MAX_DEBUG_TEXT,
  appendLog,
  countdownLeft,
  describeView,
  prepareText,
  type DevLogPayload,
} from "./log";
import { startLevelTest } from "./levelTest";
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
  const [levelSrc, setLevelSrc] = useState<LevelSrc | null>(null);
  const lastView = useRef<ViewPayload | null>(null);
  const now = useNow(Boolean(view?.countdown));

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

  // The level test feeds this window's own level store, which the preview notch reads.
  useEffect(() => {
    if (!levelSrc) return;
    return startLevelTest(levelStore, levelSrc);
  }, [levelSrc]);

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

  // While the level test runs the preview shows the look that level drives.
  const previewView: ViewPayload | null = levelSrc
    ? { state: "listening", speaking: true }
    : view;
  const previewLook = viewToLook(previewView);

  return (
    <main className="debug">
      <h1>Niva debug</h1>

      <section>
        <h2>Notch preview</h2>
        <div className="notch-preview">
          <Notch view={previewView} phase={previewLook ? "in" : "hidden"} />
        </div>
        <div className="row">
          <span className="extra">Level test, Niva's voice only (a click every 500 ms, for filming at 240 fps):</span>
          {([null, "tts"] as const).map((src) => (
            <button key={src ?? "off"} aria-pressed={levelSrc === src} onClick={() => setLevelSrc(src)}>
              {src ?? "off"}
            </button>
          ))}
        </div>
      </section>

      <section className="current">
        <span className="state" data-state={view?.state ?? "idle"}>
          {view?.state ?? "idle"}
        </span>
        <span className="caption">{view?.caption ?? ""}</span>
        {view?.confirm && <span className="extra">confirm: {view.confirm.prompt}</span>}
        {view?.reminder && <span className="extra">reminder: {view.reminder.text}</span>}
        {view?.countdown && (
          <span className="extra">
            sleep countdown: {countdownLeft(view.countdown.endsAt, now)} s left (the PC is dry-run and never sleeps)
          </span>
        )}
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
        {view?.countdown && (
          <div className="row confirm">
            <button onClick={() => dispatch({ type: "countdown_cancel" })}>Cancel countdown</button>
            <span className="extra">Any key or mouse input cancels it too, after 1 s.</span>
          </div>
        )}
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
