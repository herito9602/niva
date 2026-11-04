import { useEffect, useLayoutEffect, useRef, useState } from "react";
import type { CursorPayload, ViewPayload } from "../events";
import { displayLabel, islandLabel, type SlidePhase } from "../island";
import { MascotCanvas } from "../mascot/MascotCanvas";
import { useNow } from "../state/useNow";
import { EDGE_IN_MS, EDGE_OUT_MS, SWAP_FADE_MS, planTextChange } from "./choreography";
import { EDGE_INTRO_SETTLE_MS, EDGE_RESIZE_SETTLE_MS, edgeIntro, edgeOut, edgeResize, edgeSettled } from "./edge";
import { COMPACT_LAYOUT, LINE_HEIGHT, MASCOT_SIZE, MAX_TEXT_WIDTH, computeLayout, type Layout } from "./layout";
import { pageOffsets, scrollFrame } from "./scroll";
import { shapePath, type Shape } from "./shape";

interface Props {
  phase: SlidePhase;
  view: ViewPayload | null;
  /** Read each frame by the mascot (cursor events are not React state). */
  getCursor: () => CursorPayload | null;
  /** Overrides prefers-reduced-motion (dev preview only). */
  reducedMotion?: boolean;
  /** Freezes the long-message scroll clock at this time in ms (dev preview only). */
  scrollClockMs?: number;
  /** Freezes the shape at this time of the current phase, in ms (dev preview only). */
  shapeClockMs?: number;
}

interface Shown {
  text: string;
  layout: Layout;
}

function usePrefersReducedMotion(override: boolean | undefined): boolean {
  const [system, setSystem] = useState(() => window.matchMedia("(prefers-reduced-motion: reduce)").matches);
  useEffect(() => {
    const query = window.matchMedia("(prefers-reduced-motion: reduce)");
    const onChange = () => setSystem(query.matches);
    query.addEventListener("change", onChange);
    return () => query.removeEventListener("change", onChange);
  }, []);
  return override ?? system;
}

/**
 * Drives the island's SVG path and the box that clips its content from
 * `edge.ts`. Writes to the DOM directly: the shape changes every frame.
 */
function useShapeDriver(
  phase: SlidePhase,
  layout: Layout,
  reduced: boolean,
  shapeClockMs: number | undefined,
  refs: { stage: React.RefObject<HTMLDivElement | null>; path: React.RefObject<SVGPathElement | null>; box: React.RefObject<HTMLDivElement | null> },
) {
  const current = useRef<Shape | null>(null);
  const lastPhase = useRef<SlidePhase>("hidden");
  const introStart = useRef(0);
  const outStart = useRef(0);
  const resize = useRef<{ from: Shape; start: number } | null>(null);

  useEffect(() => {
    const cx = (refs.stage.current?.clientWidth || 640) / 2;
    const paint = (shape: Shape | null) => {
      current.current = shape;
      // A shape with no height yet (or any more) is not drawn.
      const drawn = shape && shape.h >= 1 ? shape : null;
      refs.path.current?.setAttribute("d", drawn ? shapePath(drawn, cx) : "");
      const box = refs.box.current;
      if (!box) return;
      if (!drawn || !shape) {
        box.style.visibility = "hidden";
        return;
      }
      box.style.visibility = "visible";
      box.style.left = `${cx - shape.bw}px`;
      box.style.width = `${2 * shape.bw}px`;
      box.style.height = `${shape.h}px`;
      box.style.clipPath = `inset(0 0 0 0 round 0 0 ${shape.r}px ${shape.r}px)`;
    };
    const settled = edgeSettled(layout);
    const was = lastPhase.current;
    lastPhase.current = phase;
    const now = () => performance.now();

    if (phase === "hidden") {
      paint(null);
      return;
    }
    if (reduced) {
      paint(settled);
      return;
    }
    if (shapeClockMs !== undefined) {
      paint(phase === "in" ? edgeIntro(shapeClockMs, layout) : edgeOut(shapeClockMs, layout));
      return;
    }

    let raf = 0;
    const loop = (frame: (t: number) => boolean) => {
      const tick = () => {
        if (frame(now())) raf = requestAnimationFrame(tick);
      };
      tick();
    };
    if (phase === "out") {
      if (was !== "out") outStart.current = now();
      loop((t) => {
        const e = t - outStart.current;
        paint(e >= EDGE_OUT_MS ? null : edgeOut(e, layout));
        return e < EDGE_OUT_MS;
      });
    } else {
      if (was !== "in") {
        introStart.current = now();
        resize.current = null;
      } else if (now() - introStart.current >= EDGE_INTRO_SETTLE_MS && current.current) {
        // A new message after the intro: spring from where the shape is now.
        resize.current = { from: current.current, start: now() };
      }
      loop((t) => {
        const e = t - introStart.current;
        if (e < EDGE_INTRO_SETTLE_MS) {
          paint(edgeIntro(e, layout));
          return true;
        }
        const r = resize.current;
        if (r && t - r.start < EDGE_RESIZE_SETTLE_MS) {
          paint(edgeResize(r.from, layout, t - r.start));
          return true;
        }
        paint(settled);
        return false;
      });
    }
    return () => cancelAnimationFrame(raf);
  }, [phase, layout, reduced, shapeClockMs, refs.stage, refs.path, refs.box]);
}

export function Island({ phase, view, getCursor, reducedMotion, scrollClockMs, shapeClockMs }: Props) {
  const target = islandLabel(view);
  // Only ticks while a sleep countdown is on screen; the pill is laid out for
  // `target`, which does not change with the seconds.
  const now = useNow(Boolean(view?.countdown));
  const reduced = usePrefersReducedMotion(reducedMotion);
  const stageRef = useRef<HTMLDivElement>(null);
  const pathRef = useRef<SVGPathElement>(null);
  const boxRef = useRef<HTMLDivElement>(null);

  const lineProbe = useRef<HTMLSpanElement>(null);
  const blockProbe = useRef<HTMLDivElement>(null);
  const prevPhase = useRef<SlidePhase>("hidden");
  const [shown, setShown] = useState<Shown>({ text: "", layout: COMPACT_LAYOUT });
  const [fading, setFading] = useState(false);

  // Measure the wanted message and decide how it replaces the one on screen.
  // The pill resizes at the moment the text swaps, so size and text move together.
  useLayoutEffect(() => {
    const before = prevPhase.current;
    prevPhase.current = phase;
    if (phase === "out") return;

    const layout = computeLayout({
      lineWidth: lineProbe.current?.getBoundingClientRect().width ?? 0,
      blockHeight: blockProbe.current?.getBoundingClientRect().height ?? LINE_HEIGHT,
    });
    const plan = planTextChange({ shownText: shown.text, target, phase, prevPhase: before });

    if (plan === "crossfade") {
      setFading(true);
      const id = window.setTimeout(() => {
        setShown({ text: target, layout });
        setFading(false);
      }, SWAP_FADE_MS);
      return () => window.clearTimeout(id);
    }
    setFading(false);
    if (plan === "swap-now" || shown.layout.width !== layout.width || shown.layout.height !== layout.height) {
      setShown({ text: plan === "swap-now" ? target : shown.text, layout });
    }
  }, [phase, target, shown.text, shown.layout.width, shown.layout.height]);

  const { layout } = shown;
  const refs = { stage: stageRef, path: pathRef, box: boxRef };
  useShapeDriver(phase, layout, reduced, shapeClockMs, refs);
  const scrolls = layout.scrollDistance > 0;
  const scrollerRef = useRef<HTMLDivElement>(null);

  // Long message: slow scroll with pauses (or paged fades under reduced motion).
  useEffect(() => {
    const el = scrollerRef.current;
    if (!el) return;
    const info = { distance: layout.scrollDistance, offsets: pageOffsets(layout.scrollDistance, layout.textHeight) };
    const apply = (elapsed: number) => {
      const f = scrollFrame(elapsed, info, reduced);
      el.style.transform = f.offset === 0 ? "" : `translateY(${-f.offset}px)`;
      el.style.opacity = f.opacity === 1 ? "" : String(f.opacity);
    };
    if (scrollClockMs !== undefined) {
      apply(scrollClockMs);
      return;
    }
    // While the island leaves, the text stays where it is.
    if (phase === "out") return;
    apply(0);
    if (!scrolls || phase !== "in") return;
    // The reading pause starts once the intro has finished.
    const start = performance.now() + EDGE_IN_MS;
    let raf = 0;
    const tick = () => {
      apply(performance.now() - start);
      raf = requestAnimationFrame(tick);
    };
    raf = requestAnimationFrame(tick);
    return () => cancelAnimationFrame(raf);
  }, [shown.text, layout.scrollDistance, layout.textHeight, scrolls, phase, reduced, scrollClockMs]);

  const content = (
    <>
      <MascotCanvas view={view} getCursor={getCursor} active={phase === "in"} size={MASCOT_SIZE} reducedMotion={reducedMotion} />
      <div
        className={`label${fading ? " fading" : ""}${scrolls ? " scrolls" : ""}`}
        style={{ width: layout.textWidth, height: layout.textHeight }}
      >
        <div ref={scrollerRef} className="txt scroller">
          {displayLabel(view, shown.text, now)}
        </div>
      </div>
    </>
  );
  return (
    <div ref={stageRef} className="stage">
      <div className={["island", phase, reduced ? "rm" : ""].filter(Boolean).join(" ")}>
        <svg className="island-shape" aria-hidden="true">
          <path ref={pathRef} />
        </svg>
        <div ref={boxRef} className="island-clip">
          <div className="content" style={{ width: layout.width }}>
            {content}
          </div>
        </div>
      </div>
      <div className="probe txt" aria-hidden="true">
        <span ref={lineProbe} className="probe-line">
          {target}
        </span>
        <div ref={blockProbe} style={{ width: MAX_TEXT_WIDTH }}>
          {target}
        </div>
      </div>
    </div>
  );
}
