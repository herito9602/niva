// Dev-only preview page (app/mascot-preview.html): every state at 1x and 3x.
// It is not imported by the app entry, so the production build never sees it.

import { StrictMode, useRef, useState } from "react";
import { createRoot } from "react-dom/client";
import { ALL_STATES, type CursorPayload } from "../events";
import { MascotCanvas } from "./MascotCanvas";

const SIZE = 96;

// `?state=thinking` shows one state large, for close-up checks.
const only = new URLSearchParams(window.location.search).get("state");
const ONLY_STATE = ALL_STATES.find((s) => s === only);
// `?t=0.25` freezes the animation clock (seconds) to capture an exact moment.
const tParam = new URLSearchParams(window.location.search).get("t");
const FIXED_T = tParam !== null && Number.isFinite(Number(tParam)) ? Number(tParam) : undefined;

function Preview() {
  const [reduced, setReduced] = useState(false);
  const cursor = useRef<CursorPayload | null>(null);

  return (
    <div
      onPointerMove={(e) => {
        cursor.current = { x: e.clientX, y: e.clientY };
      }}
    >
      <header>
        <h1>Niva mascot preview</h1>
        <label>
          <input type="checkbox" checked={reduced} onChange={(e) => setReduced(e.target.checked)} />{" "}
          Reduced motion
        </label>
        <span className="scale">Move the pointer: the eyes follow it.</span>
      </header>
      <div className="grid">
        {(ONLY_STATE ? [ONLY_STATE] : ALL_STATES).map((state) => (
          <section className="cell" key={state}>
            <h2>{state}</h2>
            <div className="row">
              {!ONLY_STATE && (
                <MascotCanvas
                  view={{ state }}
                  getCursor={() => cursor.current}
                  active
                  size={SIZE}
                  reducedMotion={reduced}
                  timeOverride={FIXED_T}
                />
              )}
              <MascotCanvas
                view={{ state }}
                getCursor={() => cursor.current}
                active
                size={ONLY_STATE ? SIZE * 5 : SIZE * 3}
                reducedMotion={reduced}
                timeOverride={FIXED_T}
              />
            </div>
            <div className="scale">1x ({SIZE}px, as on the island) and 3x</div>
          </section>
        ))}
      </div>
    </div>
  );
}

createRoot(document.getElementById("root")!).render(
  <StrictMode>
    <Preview />
  </StrictMode>,
);
