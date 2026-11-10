import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { ViewPayload } from "../events";
import type { SlidePhase } from "../state/viewStore";
import { BAR_COUNT, type Smoothed } from "./bars";
import { levelStore as sharedLevels, type LevelStore } from "./levelStore";
import { holdLook, viewToLook, type Look } from "./looks";
import { readReducedMotion, startBarLoop, watchReducedMotion } from "./loop";
import { NOTCH_H, NOTCH_W } from "./motion";
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
 * The notch: a black shape hanging from the top edge with seven bars. One rAF
 * loop writes `transform` and `opacity` of the bars through refs, so no React
 * render happens per frame. The only text is a visually hidden status region
 * with the state name; reply text never reaches this component.
 */
export function Notch({ view, phase, levels = sharedLevels }: NotchProps) {
  const current = viewToLook(view);
  const lastLook = useRef<Look | null>(null);
  // While it slides out the view is already idle: keep the last look on screen.
  const look = holdLook(current, lastLook.current);
  if (current) lastLook.current = current;

  const reduced = useReducedMotion();
  const barsEl = useRef<HTMLDivElement>(null);
  const lookRef = useRef(look);
  const startedAt = useRef(0);
  const smoothedMemory = useRef<{ smoothed?: Smoothed }>({});
  const active = phase !== "hidden";

  useEffect(() => {
    lookRef.current = look;
    startedAt.current = performance.now();
  }, [look]);

  useEffect(() => {
    const root = barsEl.current;
    if (!active || !root) return;
    return startBarLoop({
      bars: Array.from(root.children) as HTMLElement[],
      getLook: () => lookRef.current,
      getStartedAt: () => startedAt.current,
      levels,
      reduced,
      memory: smoothedMemory.current,
      env: {
        request: (cb) => requestAnimationFrame(cb),
        cancel: (h) => cancelAnimationFrame(h),
        now: () => performance.now(),
      },
    });
  }, [active, reduced, look.name, levels]);

  return (
    <div className="stage">
      <div
        className={`notch ${phase}`}
        data-look={look.name}
        style={{ width: NOTCH_W, height: NOTCH_H, "--bar": look.color } as CSSProperties}
      >
        <div className="bars" ref={barsEl}>
          {Array.from({ length: BAR_COUNT }, (_, i) => (
            <span className="bar" key={i} />
          ))}
        </div>
        <span className="sr-only" role="status">
          {active ? look.label : ""}
        </span>
      </div>
    </div>
  );
}
