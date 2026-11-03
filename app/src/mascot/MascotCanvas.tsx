import { useEffect, useRef } from "react";
import type { CursorPayload, ViewPayload } from "../events";
import { poseForView } from "./pose";
import {
  backingStoreSize,
  composeFrame,
  createBlinker,
  createRig,
  drawRobot,
  lookVector,
  makeContextGetter,
  stepRig,
} from "./robot";

interface Props {
  view: ViewPayload | null;
  /** Read each frame; cursor events are not React state (30 Hz). */
  getCursor: () => CursorPayload | null;
  /** The loop only runs while the island is on screen. */
  active: boolean;
  size?: number;
  /** Overrides prefers-reduced-motion (the dev preview toggles it). */
  reducedMotion?: boolean;
  /** Freezes the animation clock at this time in seconds (dev preview only). */
  timeOverride?: number;
}

/** Cursor distance (logical px) at which the eyes reach full travel. */
const GAZE_RANGE = 160;

const getContext2d = makeContextGetter();

export function MascotCanvas({ view, getCursor, active, size = 96, reducedMotion, timeOverride }: Props) {
  const canvasRef = useRef<HTMLCanvasElement>(null);
  // The loop reads the latest props through refs so it never restarts for them.
  const targetRef = useRef(poseForView(view));
  targetRef.current = poseForView(view);
  const getCursorRef = useRef(getCursor);
  getCursorRef.current = getCursor;
  const reducedRef = useRef(reducedMotion);
  reducedRef.current = reducedMotion;
  const timeRef = useRef(timeOverride);
  timeRef.current = timeOverride;

  useEffect(() => {
    const canvas = canvasRef.current;
    if (!active || !canvas) return;
    const ctx = getContext2d(canvas);
    if (!ctx) return;

    // Sizes the backing store for the current devicePixelRatio. Resizing a
    // canvas resets its context state, so the transform is set again.
    const configure = () => {
      const px = backingStoreSize(size, window.devicePixelRatio);
      if (canvas.width !== px) canvas.width = px;
      if (canvas.height !== px) canvas.height = px;
      ctx.setTransform(px / size, 0, 0, px / size, 0, 0);
    };
    configure();

    // The ratio changes when the window moves to another monitor or the user
    // zooms; a resolution media query fires once per change, so re-arm it.
    let dprQuery: MediaQueryList | null = null;
    const onDprChange = () => {
      configure();
      watchDpr();
    };
    const watchDpr = () => {
      dprQuery?.removeEventListener("change", onDprChange);
      dprQuery = window.matchMedia(`(resolution: ${window.devicePixelRatio}dppx)`);
      dprQuery.addEventListener("change", onDprChange);
    };
    watchDpr();
    const resizeObserver = new ResizeObserver(configure);
    resizeObserver.observe(canvas);

    const reduced = window.matchMedia("(prefers-reduced-motion: reduce)");
    const isReduced = () => reducedRef.current ?? reduced.matches;
    let rig = createRig(targetRef.current);
    let blinker: ReturnType<typeof createBlinker> | null = null;
    let last = 0;
    let raf = 0;

    const frame = (nowMs: number) => {
      const now = nowMs / 1000;
      const dt = last === 0 ? 0 : now - last;
      last = now;
      blinker ??= createBlinker(Math.random, now);

      // Window-relative centre of the canvas (the webview fills the window).
      const box = canvas.getBoundingClientRect();
      const center = { x: box.left + box.width / 2, y: box.top + box.height / 2 };
      const gaze = lookVector(getCursorRef.current(), center, GAZE_RANGE);

      rig = stepRig(rig, targetRef.current, gaze, dt);
      const clock = timeRef.current ?? now;
      drawRobot(ctx, composeFrame(rig, clock, blinker.amount(now), isReduced() ? 0 : 1), size);
      raf = requestAnimationFrame(frame);
    };
    raf = requestAnimationFrame(frame);

    return () => {
      cancelAnimationFrame(raf);
      resizeObserver.disconnect();
      dprQuery?.removeEventListener("change", onDprChange);
    };
  }, [active, size]);

  return <canvas ref={canvasRef} className="mascot" style={{ width: size, height: size }} />;
}
