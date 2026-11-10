import { useEffect, useRef, useState, type CSSProperties } from "react";
import type { ViewPayload } from "../events";
import type { SlidePhase } from "../state/viewStore";
import { BAR_COUNT, frame, type Smoothed } from "./bars";
import { levelStore as sharedLevels, type LevelStore } from "./levelStore";
import { LOOKS, viewToLook, type Look } from "./looks";
import { NOTCH_H, NOTCH_W } from "./motion";
import "./notch.css";

/** A bar at level 0 is a dot this fraction of the full bar; exactly 0 is a gap. */
const MIN_SCALE = 0.25;
const GAP_BELOW = 0.01;
const REDUCED_QUERY = "(prefers-reduced-motion: reduce)";

const barScale = (v: number) => (v < GAP_BELOW ? 0 : MIN_SCALE + (1 - MIN_SCALE) * v);

function useReducedMotion(): boolean {
  const query = () => (typeof window !== "undefined" && window.matchMedia ? window.matchMedia(REDUCED_QUERY) : null);
  const [reduced, setReduced] = useState(() => query()?.matches ?? false);
  useEffect(() => {
    const mq = query();
    if (!mq) return;
    const onChange = () => setReduced(mq.matches);
    onChange();
    mq.addEventListener("change", onChange);
    return () => mq.removeEventListener("change", onChange);
  }, []);
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
  if (current) lastLook.current = current;
  // While it slides out the view is already idle: keep the last look on screen.
  const look = current ?? lastLook.current ?? LOOKS.listening;

  const reduced = useReducedMotion();
  const barsEl = useRef<HTMLDivElement>(null);
  const lookRef = useRef(look);
  const startedAt = useRef(0);
  const smoothed = useRef<Smoothed | undefined>(undefined);
  const active = phase !== "hidden";

  useEffect(() => {
    lookRef.current = look;
    startedAt.current = performance.now();
  }, [look]);

  useEffect(() => {
    const root = barsEl.current;
    if (!active || !root) return;
    const bars = Array.from(root.children) as HTMLElement[];
    let raf = 0;
    const tick = () => {
      const now = performance.now();
      const l = lookRef.current;
      const next = frame(now, l, levels.forLook(l), {
        prev: smoothed.current,
        reduced,
        startedAt: startedAt.current,
      });
      smoothed.current = next;
      next.bars.forEach((v, i) => {
        const el = bars[i];
        if (!el) return;
        el.style.transform = `scaleY(${barScale(v).toFixed(3)})`;
        el.style.opacity = (0.55 + 0.45 * v).toFixed(3);
      });
      // A static silhouette needs no loop, only the write above.
      if (!(reduced && l.motion !== "levels" && settled(next, l))) raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
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

/** The smoothed bars have reached the silhouette (within a hair). */
function settled(s: Smoothed, look: Look): boolean {
  return s.bars.every((v, i) => Math.abs(v - look.silhouette[i]) < 0.002);
}
