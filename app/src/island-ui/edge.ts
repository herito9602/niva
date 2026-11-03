// The island's choreography: it is the top edge of the screen growing
// downward, like the notch of a laptop (the owner's choice, 2026-11-03). The
// shape is always flush with the top of the window (y = 0), its top corners
// flare out into the edge with concave fillets, its bottom corners are
// rounded. It never floats: on appear only the bottom edge moves down and the
// sides move out, driven by damped springs that overlap in time; on exit it
// retracts up into the edge, with no bounce.
//
// Pure functions of time, so the rAF loop only samples them and the tests can
// check any moment. `shapePath` (shape.ts) draws the result.

import { EDGE_OUT_MS } from "./choreography";
import type { Layout } from "./layout";
import type { Shape } from "./shape";
import { settleMs, springK } from "./springs";

/** Width of the small notch it grows from and retracts to. */
export const EDGE_NOTCH_W = 96;
/** Radius of the concave fillets where the shape meets the top edge. */
export const EDGE_FILLET = 10;
/** Radius of the bottom corners. */
const EDGE_RADIUS = 34;

interface Channel {
  omega: number;
  zeta: number;
  delay: number;
}

// Intro: the height leads, the width follows 40 ms later (both soft, 3-6 % overshoot).
const IN_H: Channel = { omega: 16, zeta: 0.68, delay: 0 };
const IN_W: Channel = { omega: 13, zeta: 0.72, delay: 40 };
// Exit: content fades first (about 70 ms), then near-critical damping: no bounce.
// The owner tried a slower version and preferred this speed (2026-11-03).
const OUT_H: Channel = { omega: 28, zeta: 0.95, delay: 70 };
const OUT_W: Channel = { omega: 28, zeta: 0.95, delay: 90 };
// Message change: same springs, no delays.
const RE_H: Channel = { ...IN_H, delay: 0 };
const RE_W: Channel = { ...IN_W, delay: 0 };

const EPS = 0.002;
const settleOf = (c: Channel) => c.delay + settleMs(c.omega, c.zeta, EPS);
export const EDGE_INTRO_SETTLE_MS = Math.max(settleOf(IN_H), settleOf(IN_W));
export const EDGE_RESIZE_SETTLE_MS = Math.max(settleOf(RE_H), settleOf(RE_W));

type Pill = Pick<Layout, "width" | "height">;

const usable = (t: number) => (Number.isFinite(t) && t > 0 ? t : 0);
const along = (from: number, to: number, c: Channel, t: number) =>
  from + (to - from) * springK(t - c.delay, c.omega, c.zeta);

/** A shape of the given size, hanging from the edge, with fillets that fit it. */
function frame(h: number, w: number): Shape {
  const height = Math.max(0, h);
  const bw = Math.max(1, w / 2);
  return {
    bw,
    h: height,
    F: Math.min(EDGE_FILLET, height / 2),
    r: Math.min(EDGE_RADIUS, height / 2, bw),
  };
}

export function edgeSettled(l: Pill): Shape {
  return frame(l.height, l.width);
}

export function edgeIntro(tMs: number, l: Pill): Shape {
  const t = usable(tMs);
  if (t >= EDGE_INTRO_SETTLE_MS) return edgeSettled(l);
  return frame(along(0, l.height, IN_H, t), along(EDGE_NOTCH_W, l.width, IN_W, t));
}

/** Back into the edge and gone within `EDGE_OUT_MS`. */
export function edgeOut(tMs: number, l: Pill): Shape {
  const t = Math.min(usable(tMs), EDGE_OUT_MS);
  return frame(along(l.height, 0, OUT_H, t), along(l.width, EDGE_NOTCH_W, OUT_W, t));
}

/** The island on screen springing to the size of a new message, still anchored at the top. */
export function edgeResize(from: Shape, l: Pill, tMs: number): Shape {
  const t = usable(tMs);
  if (t >= EDGE_RESIZE_SETTLE_MS) return edgeSettled(l);
  return frame(along(from.h, l.height, RE_H, t), along(from.bw * 2, l.width, RE_W, t));
}
