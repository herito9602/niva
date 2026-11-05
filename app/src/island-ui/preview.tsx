// Dev-only preview page (app/island-preview.html). It is not imported by the
// app entry, so the production build never sees it.
//
// URL parameters (all optional):
//   msg=short|two|six|long|confirm   message to show              (default short)
//   then=<msg>                       switch to this message later (shows the resize)
//   seq=in|out                       play the appear, or appear then disappear
//   t=<ms>                           freeze every animation at this time of the current step
//   scroll=<ms>                      freeze the long-message scroll clock
//   reduced=1                        force reduced motion
//   ui=0                             hide the controls (clean screenshots)

import { StrictMode, useCallback, useEffect, useMemo, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import type { ViewPayload } from "../events";
import type { SlidePhase } from "../island";
import "../styles.css";
import { SWAP_FADE_MS } from "./choreography";
import { Island } from "./Island";

const SAMPLES: Record<string, string> = {
  short: "Opening Notepad",
  confirm: "Close Spotify.exe?",
  two: "I found two Spotify processes running. Which one should I close: Spotify.exe or SpotifyHelper.exe?",
  six:
    "Here is what I found about the Rust 2024 edition: it changes how temporaries drop in tail expressions, " +
    "makes unsafe extern blocks mandatory, reserves the gen keyword, and tightens the never type fallback. " +
    "Migrating is mostly automatic with cargo fix, and the compiler tells you where it cannot decide.",
  long:
    "Rust 2024 is the largest edition so far. Tail-expression temporaries now drop before locals, if-let " +
    "scrutinee temporaries drop before the else block, unsafe extern blocks are required, and the gen " +
    "keyword is reserved for generators. The never type falls back to itself instead of the unit type, " +
    "and several lints move from warn to deny. Run cargo fix --edition to migrate; review the few spots " +
    "it flags manually. Source: the Rust blog and the edition guide.",
};

const params = new URLSearchParams(window.location.search);
const num = (key: string) => {
  const raw = params.get(key);
  return raw !== null && Number.isFinite(Number(raw)) ? Number(raw) : undefined;
};
const sample = (key: string | null, fallback: string) => (key && SAMPLES[key] ? key : fallback);

const FREEZE_MS = num("t");
const SCROLL_MS = num("scroll");
const SEQ = params.get("seq");
const THEN = params.get("then");
const SHOW_UI = params.get("ui") !== "0";

// The confirm sample shows the buttons too (they do nothing here: no host).
const asView = (key: string): ViewPayload =>
  key === "confirm"
    ? {
        state: "confirmation-needed",
        caption: SAMPLES[key],
        confirm: { id: 1, prompt: SAMPLES[key], confirmLabel: "Confirm", cancelLabel: "Cancel" },
      }
    : { state: "success", caption: SAMPLES[key] };

/**
 * Pauses animations at `ms` (screenshots). With `onlyTransitions`, the keyframe
 * animations (the intro) are finished instead, so a resize can be captured on
 * its own (headless virtual time makes frame callbacks unreliable, so nothing
 * here waits for a frame).
 */
function freezeAt(ms: number, onlyTransitions = false) {
  // getAnimations() flushes style, so the animation of the last change exists.
  document.getAnimations().forEach((a) => {
    if (onlyTransitions && !(a instanceof CSSTransition)) {
      a.finish();
      return;
    }
    a.pause();
    a.currentTime = ms;
  });
}

function Preview() {
  const [phase, setPhase] = useState<SlidePhase>("hidden");
  const [msg, setMsg] = useState(sample(params.get("msg"), "short"));
  const [reduced, setReduced] = useState(params.get("reduced") === "1");
  // Freeze the shape at this time of the current phase.
  const [shapeClock, setShapeClock] = useState<number | undefined>(SEQ === "out" || THEN ? undefined : FREEZE_MS);
  const timers = useRef<number[]>([]);
  const after = useCallback((ms: number, fn: () => void) => {
    timers.current.push(window.setTimeout(fn, ms));
  }, []);
  const view = useMemo(() => asView(msg), [msg]);

  const appear = useCallback(() => {
    setPhase("hidden");
    after(80, () => setPhase("in"));
  }, [after]);

  // Scripted runs for the screenshots.
  useEffect(() => {
    const frozen = FREEZE_MS !== undefined;
    after(150, () => {
      setPhase("in");
      if (SEQ === "out") {
        after(1200, () => {
          setPhase("out");
          if (frozen) setShapeClock(FREEZE_MS);
          if (frozen) after(20, () => freezeAt(FREEZE_MS));
        });
      } else if (THEN && SAMPLES[THEN]) {
        after(1200, () => {
          // Headless virtual time never ends the intro by itself; end it so the
          // resize starts from the settled pill, as it does for a real user.
          document.getAnimations().forEach((a) => a.finish());
          after(30, () => {
            setMsg(THEN);
            if (frozen) after(SWAP_FADE_MS + 20, () => freezeAt(FREEZE_MS, true));
          });
        });
      } else if (frozen) {
        after(20, () => freezeAt(FREEZE_MS));
      }
    });
    const pending = timers.current;
    return () => pending.forEach((id) => window.clearTimeout(id));
  }, [after]);

  return (
    <div className="desk">
      <style>{`
        .desk { position: fixed; inset: 0; overflow: hidden; background: linear-gradient(135deg, #5b73b8, #b58bd0 55%, #efb487); font-family: "Segoe UI", system-ui, sans-serif; }
        .win { position: relative; width: 640px; height: 200px; margin: 0 auto; overflow: hidden; outline: 1px dashed rgba(255,255,255,.35); }
        .win #island-host { width: 100%; height: 100%; }
        .controls { position: absolute; top: 224px; left: 0; right: 0; display: flex; flex-wrap: wrap; gap: 8px; justify-content: center; padding: 0 16px; }
        .controls button, .controls label { font: 13px "Segoe UI", system-ui, sans-serif; color: #101226; background: rgba(255,255,255,.8); border: 0; border-radius: 6px; padding: 6px 10px; cursor: pointer; }
        .controls button.on { background: #101226; color: #fff; }
        .note { position: absolute; top: 270px; width: 100%; text-align: center; color: rgba(255,255,255,.9); font-size: 12px; }
      `}</style>
      <div className="win" id="island-host">
        <Island
          phase={phase}
          view={view}
          getCursor={() => null}
          reducedMotion={reduced || undefined}
          scrollClockMs={SCROLL_MS}
          shapeClockMs={shapeClock}
        />
      </div>
      {SHOW_UI && (
        <>
          <div className="controls">
            <button onClick={() => { setShapeClock(undefined); appear(); }}>Replay appear</button>
            <button onClick={() => { setShapeClock(undefined); setPhase("out"); }}>Disappear</button>
            {Object.keys(SAMPLES).map((key) => (
              <button key={key} className={key === msg ? "on" : ""} onClick={() => setMsg(key)}>
                {key}
              </button>
            ))}
            <label>
              <input type="checkbox" checked={reduced} onChange={(e) => setReduced(e.target.checked)} /> Reduced motion
            </label>
          </div>
          <div className="note">The dashed box is the island window (640 x 200): nothing may be cut by it.</div>
        </>
      )}
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Preview />
  </StrictMode>,
);
