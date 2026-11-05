// Pure geometry for the robot's hands and laptop, in the drawing space
// of `drawRobot` (units, origin at the head centre, y down). The drawing code
// only strokes and fills what these functions return.

import { PAN_R, type HandWeights, type Vec } from "./gesture";

/** Hands are drawn this much larger than their local geometry. */
export const HAND_SCALE = 1.1;

// --- Hands ----------------------------------------------------------------

/** Look of one hand: a single rounded mitten with a thumb nub, and an index
 *  finger only when pointing. Local frame: fingers up, thumb on the -x side
 *  (toward the body centre once the left hand is mirrored), body centred on
 *  the hand position. */
export interface HandShape {
  side: -1 | 1;
  bodyW: number;
  bodyH: number;
  /** Length of the extended index finger; 0 = none. */
  pointLen: number;
  /** Thumb direction in radians (canvas angles: -pi/2 is straight up). */
  thumbAngle: number;
  thumbLen: number;
  thumbWidth: number;
  /** Where the thumb starts on the inner side, as a share of the body height
   *  from the centre (negative = toward the top). */
  thumbBase: number;
  /** The whole hand tips toward the thumb side by this many radians. */
  lean: number;
  size: number;
}

type ShapeCore = Omit<HandShape, "side">;

// Relaxed mitten hanging down, small thumb nub pointing down and inward.
const SHAPE_REST: ShapeCore = {
  bodyW: 12, bodyH: 15, pointLen: 0, thumbAngle: 2.2, thumbLen: 5, thumbWidth: 5,
  thumbBase: -0.1, lean: 0, size: 1,
};
// Closed fist with a thick thumb straight up.
const SHAPE_THUMBS: ShapeCore = {
  bodyW: 14, bodyH: 13, pointLen: 0, thumbAngle: -Math.PI / 2, thumbLen: 15, thumbWidth: 7.5,
  thumbBase: -0.4, lean: 0, size: 1,
};
// Open flat mitten, palm toward the viewer.
const SHAPE_RAISED: ShapeCore = {
  bodyW: 15, bodyH: 19, pointLen: 0, thumbAngle: -2.6, thumbLen: 8, thumbWidth: 5.6,
  thumbBase: 0.15, lean: -0.12, size: 1,
};
// Closed mitten held above the head, one index finger pointing down at it.
const SHAPE_POINT: ShapeCore = {
  bodyW: 12, bodyH: 12, pointLen: 11, thumbAngle: 2.4, thumbLen: 4.5, thumbWidth: 5,
  thumbBase: 0.1, lean: 3.0, size: 1,
};
// Open mitten cupped behind the ear: narrow and tall, thumb toward the head,
// opened outward.
const SHAPE_CUP: ShapeCore = {
  bodyW: 10, bodyH: 18, pointLen: 0, thumbAngle: -2.9, thumbLen: 7, thumbWidth: 5,
  thumbBase: -0.15, lean: -0.3, size: 1,
};
// Gripping a pan handle: closed mitten, thumb wrapped over the top.
const SHAPE_PANS: ShapeCore = {
  bodyW: 12, bodyH: 12, pointLen: 0, thumbAngle: -1.6, thumbLen: 5, thumbWidth: 5,
  thumbBase: -0.2, lean: 0, size: 1,
};
// Typing uses the relaxed mitten.
const SHAPE_TYPING: ShapeCore = { ...SHAPE_REST, bodyW: 13, bodyH: 12 };

const SHAPE_KEYS = [
  "bodyW",
  "bodyH",
  "pointLen",
  "thumbAngle",
  "thumbLen",
  "thumbWidth",
  "thumbBase",
  "lean",
  "size",
] as const satisfies readonly (keyof ShapeCore)[];

/** Blends the gesture shapes over the rest shape. Pointing (the head scratch)
 *  belongs to the left hand, thumbs-up, raised and the cupped ear to the right, typing to both. */
export function handShape(w: HandWeights, side: -1 | 1): HandShape {
  const parts: Array<[number, ShapeCore]> =
    side === -1
      ? [
          [w.scratch, SHAPE_POINT],
          [w.pans, SHAPE_PANS],
          [w.typing, SHAPE_TYPING],
        ]
      : [
          [w.thumbs, SHAPE_THUMBS],
          [w.raise, SHAPE_RAISED],
          [w.cup, SHAPE_CUP],
          [w.pans, SHAPE_PANS],
          [w.typing, SHAPE_TYPING],
        ];
  const shape = { side } as HandShape;
  for (const key of SHAPE_KEYS) {
    shape[key] =
      SHAPE_REST[key] + parts.reduce((sum, [weight, s]) => sum + weight * (s[key] - SHAPE_REST[key]), 0);
  }
  return shape;
}

/** Outline of the head: a soft rounded square, wider than tall, centred on the
 *  origin of the drawing space. `drawRobot` fills this shape and the scratching
 *  hand's tip is placed on it. */
export const HEAD = { halfW: 34, halfH: 30, radius: 22 } as const;

/** Signed distance from `p` to the head outline: negative inside, positive outside. */
export function headOutlineDistance(p: Vec): number {
  const qx = Math.abs(p.x) - (HEAD.halfW - HEAD.radius);
  const qy = Math.abs(p.y) - (HEAD.halfH - HEAD.radius);
  return Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - HEAD.radius;
}

/** How far the scratching paddle reaches past its body, per unit of pointing
 *  length; chosen so the tip rests on the head outline. */
const SCRATCH_REACH = 0.84;

/** Tilt of the scratching paddle about its tip, in radians (at full pointing). */
export const SCRATCH_TILT = 0.9;

export interface HandLayout {
  /** Egg size and its centre along the local y axis. */
  w: number;
  h: number;
  cy: number;
  /** 0..1: how far the hand is in the pointing (scratch) pose. */
  point: number;
  /** Pivot of the scratch tilt, local y. */
  pivotY: number;
}

/** The mitten's local layout. While pointing it shrinks a little, shifts toward
 *  the crown so its far end reaches it, and tilts about `pivotY`. */
export function handLayout(shape: HandShape): HandLayout {
  const point = Math.min(1, shape.pointLen / 8);
  const w = shape.bodyW * (1.12 - 0.12 * point);
  const h = shape.bodyH * (1 - 0.12 * point);
  const tipY = -shape.bodyH / 2 - shape.pointLen * SCRATCH_REACH;
  return { w, h, cy: point * (tipY + h / 2), point, pivotY: tipY * point };
}

/** Where the far end of the drawn mitten is in drawing space, through the same
 *  transform `drawHand` applies: translate, scale(side * k, k), rotate(-lean),
 *  then the scratch tilt about the pivot. While scratching it rests on the crown. */
export function handTip(pos: Vec, shape: HandShape): Vec {
  const l = handLayout(shape);
  const k = HAND_SCALE * shape.size;
  let x = 0;
  let y = l.cy - l.h / 2;
  if (l.point > 0) {
    const t = SCRATCH_TILT * l.point;
    const dy = y - l.pivotY;
    x = -dy * Math.sin(t);
    y = l.pivotY + dy * Math.cos(t);
  }
  const phi = -shape.lean;
  return {
    x: pos.x + (x * Math.cos(phi) - y * Math.sin(phi)) * k * shape.side,
    y: pos.y + (x * Math.sin(phi) + y * Math.cos(phi)) * k,
  };
}

// --- Impact effects ---------------------------------------------------------

/** Short noise lines radiating from `center`, longer and nearer for a stronger
 *  impact; none when there is no impact. They stay within 24 units of the
 *  centre, which is what the canvas has above the head. */
export function burstSegments(
  center: Vec,
  alpha: number,
  count: number,
): Array<{ from: Vec; to: Vec }> {
  if (!(alpha > 0)) return [];
  const inner = 17 + 2 * (1 - alpha);
  const outer = inner + 3 + 4 * alpha;
  const segments: Array<{ from: Vec; to: Vec }> = [];
  for (let i = 0; i < count; i++) {
    const a = ((i + 0.5) / count) * Math.PI * 2;
    const c = Math.cos(a);
    const s = Math.sin(a);
    segments.push({
      from: { x: center.x + c * inner, y: center.y + s * inner },
      to: { x: center.x + c * outer, y: center.y + s * outer },
    });
  }
  return segments;
}

/** Outline of the bold star burst at the contact point: `spikes` tips with a
 *  notch between each (so `2 * spikes` points), up to 16 units across at full
 *  strength; none when there is no impact. */
export function starPoints(center: Vec, alpha: number, spikes: number): Vec[] {
  if (!(alpha > 0)) return [];
  const tip = 8 + 8 * Math.min(1, alpha);
  const notch = tip * 0.45;
  const points: Vec[] = [];
  for (let i = 0; i < spikes * 2; i++) {
    const a = (i / (spikes * 2)) * Math.PI * 2 - Math.PI / 2;
    const r = i % 2 === 0 ? tip : notch;
    points.push({ x: center.x + r * Math.cos(a), y: center.y + r * Math.sin(a) });
  }
  return points;
}

/** Vibration ticks around a pan right after a hit, in the pan's local frame
 *  (handle toward +x, opening up). Three short ticks arcing above the rim,
 *  longer while the vibration is strong; none at rest. */
export function vibeSegments(alpha: number): Array<{ from: Vec; to: Vec }> {
  if (!(alpha > 0)) return [];
  const inner = PAN_R + 3.5;
  const outer = inner + 2.5 + 3.5 * Math.min(1, alpha);
  return [-2.2, -1.57, -0.94].map((a) => ({
    from: { x: inner * Math.cos(a), y: inner * 0.62 * Math.sin(a) },
    to: { x: outer * Math.cos(a), y: outer * 0.62 * Math.sin(a) },
  }));
}

// --- Laptop ---------------------------------------------------------------

export interface Rect {
  x: number;
  y: number;
  w: number;
  h: number;
}

export interface LaptopGeometry {
  /** Back of the lid, facing the viewer (the screen faces the robot). */
  lid: Rect & { r: number };
  /** Deck strip, wider than the lid and protruding on both sides. */
  base: Rect & { r: number };
  /** Our own small emblem on the lid: a ring. */
  emblem: { cx: number; cy: number; r: number };
  /** Where the screen light spills onto the visor: a soft vertical gradient
   *  from the lid's top edge (`bottomY`) fading out at `topY`. */
  spill: { topY: number; bottomY: number; halfWidth: number };
}

/** An open laptop seen from behind its lid, in front of the robot's lower
 *  body: a small lid below the chin and a deck wider than the lid, whose
 *  protruding edges are where the hands tap. */
export function laptopGeometry(): LaptopGeometry {
  const lid = { x: -22, y: 31, w: 44, h: 20, r: 4 };
  return {
    lid,
    base: { x: -40, y: 49, w: 80, h: 9, r: 3.5 },
    emblem: { cx: 0, cy: lid.y + lid.h / 2, r: 4 },
    spill: { topY: -6, bottomY: lid.y, halfWidth: 26 },
  };
}
