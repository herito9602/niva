import { useEffect, useRef, useState } from "react";
import type { ViewPayload } from "../events";
import type { SlidePhase } from "../state/viewStore";
import { levelStore as sharedLevels, type LevelStore } from "./levelStore";
import { holdLook, viewToLook, type Look } from "./looks";
import { readReducedMotion, redrawWave, startWaveLoop, watchDisplayMetrics, watchReducedMotion } from "./loop";
import { NOTCH_H, NOTCH_W } from "./motion";
import { fitCanvas, type WaveState } from "./wave";
import "./notch.css";

function useReducedMotion(): boolean {
  const win = typeof window !== "undefined" ? window : undefined;
  const [reduced, setReduced] = useState(() => readReducedMotion(win));
  useEffect(() => watchReducedMotion(win, setReduced), [win]);
  return reduced;
}

export interface NotchProps {
  view: ViewPayload | null;
  phase: SlidePhase;
  levels?: LevelStore;
}

/**
 * The notch: a black shape hanging from the top edge with one mirrored spike
 * waveform on a canvas. One rAF loop draws it, so no React render happens per
 * frame. The only text is a visually hidden status region with the state name;
 * reply text never reaches this component.
 */
export function Notch({ view, phase, levels = sharedLevels }: NotchProps) {
  const current = viewToLook(view);
  const lastLook = useRef<Look | null>(null);
  // While it slides out the view is already idle: keep the last look on screen.
  const look = holdLook(current, lastLook.current);
  if (current) lastLook.current = current;

  const reduced = useReducedMotion();
  const canvasEl = useRef<HTMLCanvasElement>(null);
  const lookRef = useRef(look);
  const waveMemory = useRef<{ wave?: WaveState }>({});
  const active = phase !== "hidden";

  useEffect(() => {
    lookRef.current = look;
  }, [look]);

  useEffect(() => {
    const canvas = canvasEl.current;
    const ctx = canvas?.getContext("2d");
    if (!active || !canvas || !ctx) return;
    return startWaveLoop({
      ctx,
      size: () => fitCanvas(canvas, NOTCH_W, NOTCH_H, window.devicePixelRatio),
      getLook: () => lookRef.current,
      levels,
      reduced,
      rng: Math.random,
      memory: waveMemory.current,
      env: {
        request: (cb) => requestAnimationFrame(cb),
        cancel: (h) => cancelAnimationFrame(h),
        now: () => performance.now(),
      },
    });
  }, [active, reduced, look.name, levels]);

  // A stopped loop (static line) does not follow the DPR or a resize by itself:
  // redraw once on each change so the backing store stays sharp.
  useEffect(() => {
    const canvas = canvasEl.current;
    const ctx = canvas?.getContext("2d");
    if (!active || !canvas || !ctx) return;
    return watchDisplayMetrics(window, () =>
      redrawWave({
        ctx,
        size: () => fitCanvas(canvas, NOTCH_W, NOTCH_H, window.devicePixelRatio),
        getLook: () => lookRef.current,
        memory: waveMemory.current,
      }),
    );
  }, [active]);

  return (
    <div className="stage">
      <div className={`notch ${phase}`} data-look={look.name} style={{ width: NOTCH_W, height: NOTCH_H }}>
        <canvas className="wave" ref={canvasEl} aria-hidden="true" />
        <span className="sr-only" role="status">
          {active ? look.label : ""}
        </span>
      </div>
    </div>
  );
}
