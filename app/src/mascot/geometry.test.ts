import { describe, expect, it } from "vitest";
import {
  armCurve,
  burstSegments,
  HAND_SCALE,
  handParts,
  handShape,
  laptopGeometry,
  starPoints,
  vibeSegments,
} from "./geometry";
import { HAND_REST, handPositions, type HandWeights } from "./gesture";

const NONE: HandWeights = { scratch: 0, thumbs: 0, typing: 0, raise: 0, cup: 0, pans: 0 };

/** Signed distance from a point to the head's rounded-rectangle outline. */
function headOutlineDistance(p: { x: number; y: number }): number {
  const c = { x: 0, y: -1.5 };
  const half = { x: 34, y: 29.5 };
  const r = 21;
  const qx = Math.abs(p.x - c.x) - (half.x - r);
  const qy = Math.abs(p.y - c.y) - (half.y - r);
  return Math.hypot(Math.max(qx, 0), Math.max(qy, 0)) + Math.min(Math.max(qx, qy), 0) - r;
}
const ALL_MODES: HandWeights[] = [
  NONE,
  { ...NONE, cup: 1 },
  { ...NONE, pans: 1 },
  { ...NONE, scratch: 1 },
  { ...NONE, thumbs: 1 },
  { ...NONE, typing: 1 },
  { ...NONE, raise: 1 },
];
/** Where the drawn index finger ends: the tip of the finger part that
 *  `handParts` returns, through the same transform `drawHand` applies
 *  (translate, scale(side * k, k), rotate(-lean)). */
function drawnFingertip(pos: { x: number; y: number }, shape: ReturnType<typeof handShape>) {
  const finger = handParts(shape)[2];
  const k = HAND_SCALE * shape.size;
  const lx = finger.cx;
  const ly = finger.cy - finger.h / 2;
  const phi = -shape.lean;
  const x = lx * Math.cos(phi) - ly * Math.sin(phi);
  const y = lx * Math.sin(phi) + ly * Math.cos(phi);
  return { x: pos.x + x * k * shape.side, y: pos.y + y * k };
}
const finite = (...n: number[]) => n.every(Number.isFinite);

describe("armCurve", () => {
  it("bends outward, away from the body centre", () => {
    const left = armCurve({ x: -30, y: 20 }, { x: -40, y: 36 }, -1);
    const right = armCurve({ x: 30, y: 20 }, { x: 40, y: 36 }, 1);
    expect(left.ctrl.x).toBeLessThan((left.from.x + left.to.x) / 2);
    expect(right.ctrl.x).toBeGreaterThan((right.from.x + right.to.x) / 2);
  });

  it("bends more for a longer arm and not at all for a zero-length one", () => {
    const short = armCurve({ x: 30, y: 20 }, { x: 34, y: 26 }, 1);
    const long = armCurve({ x: 30, y: 20 }, { x: 15, y: 70 }, 1);
    const off = (c: ReturnType<typeof armCurve>) =>
      Math.hypot(c.ctrl.x - (c.from.x + c.to.x) / 2, c.ctrl.y - (c.from.y + c.to.y) / 2);
    expect(off(long)).toBeGreaterThan(off(short));
    const none = armCurve({ x: 5, y: 5 }, { x: 5, y: 5 }, 1);
    expect(none.ctrl).toEqual({ x: 5, y: 5 });
  });

});

describe("handShape (one mitten, no separate fingers)", () => {
  it("rests as a relaxed mitten hanging down with a small thumb nub", () => {
    for (const side of [-1, 1] as const) {
      const s = handShape(NONE, side);
      expect(s.pointLen).toBe(0);
      expect(s.bodyH).toBeGreaterThan(s.bodyW); // hangs, taller than wide
      expect(Math.sin(s.thumbAngle)).toBeGreaterThan(0); // nub points down
      expect(s.thumbLen).toBeLessThan(8);
    }
  });

  it("closes into a fist with a thick thumb straight up for the thumbs-up", () => {
    const s = handShape({ ...NONE, thumbs: 1 }, 1);
    expect(s.pointLen).toBe(0);
    expect(Math.abs(s.thumbAngle + Math.PI / 2)).toBeLessThan(0.05);
    expect(s.thumbLen).toBeGreaterThanOrEqual(13);
    expect(s.thumbWidth).toBeGreaterThanOrEqual(7);
  });

  it("opens flat and taller for the raised hand", () => {
    const raised = handShape({ ...NONE, raise: 1 }, 1);
    expect(raised.bodyH).toBeGreaterThanOrEqual(18);
    expect(raised.bodyH).toBeGreaterThan(handShape(NONE, 1).bodyH);
  });

  it("points one index finger straight down for the head scratch", () => {
    const s = handShape({ ...NONE, scratch: 1 }, -1);
    expect(s.pointLen).toBeGreaterThanOrEqual(10);
    expect(s.lean).toBeGreaterThan(2.7); // turned about half a turn: finger down
    expect(s.lean).toBeLessThan(3.4);
    const tip = drawnFingertip({ x: 0, y: 0 }, s);
    expect(tip.y).toBeGreaterThan(12); // the finger hangs below the hand
    expect(Math.abs(tip.x)).toBeLessThan(5);
  });

  it("cups the open mitten behind the ear for listening, right hand only", () => {
    const s = handShape({ ...NONE, cup: 1 }, 1);
    expect(s.pointLen).toBe(0);
    expect(s.bodyH).toBeGreaterThanOrEqual(16);
    expect(s.bodyW).toBeLessThan(handShape({ ...NONE, raise: 1 }, 1).bodyW);
    expect(Math.cos(s.thumbAngle)).toBeLessThan(0); // thumb toward the head
    expect(s.lean).toBeLessThan(0); // opened outward, away from the head
    expect(handShape({ ...NONE, cup: 1 }, -1)).toEqual(handShape(NONE, -1));
  });

  it("only applies a gesture to the hand that performs it", () => {
    expect(handShape({ ...NONE, thumbs: 1 }, -1)).toEqual(handShape(NONE, -1));
    expect(handShape({ ...NONE, scratch: 1 }, 1)).toEqual(handShape(NONE, 1));
  });

  it("keeps every hand about the size of an eye, never dominating the head", () => {
    for (const w of ALL_MODES) {
      for (const side of [-1, 1] as const) {
        const s = handShape(w, side);
        expect(s.bodyW * s.size).toBeLessThanOrEqual(16);
        expect(s.bodyH * s.size).toBeLessThanOrEqual(20);
      }
    }
  });

  it("stays finite for any blend", () => {
    for (const a of [0, 0.4, 1]) {
      for (const b of [0, 0.7, 1]) {
        for (const side of [-1, 1] as const) {
          const s = handShape({ scratch: a, thumbs: b, typing: a, raise: b, cup: a, pans: b }, side);
          expect(
            finite(s.bodyW, s.bodyH, s.pointLen, s.thumbAngle, s.thumbLen, s.thumbWidth, s.thumbBase, s.lean, s.size),
          ).toBe(true);
        }
      }
    }
  });
});

describe("handParts", () => {
  it("is a body and a thumb, plus one finger only when pointing", () => {
    expect(handParts(handShape(NONE, 1))).toHaveLength(2);
    expect(handParts(handShape({ ...NONE, thumbs: 1 }, 1))).toHaveLength(2);
    expect(handParts(handShape({ ...NONE, scratch: 1 }, -1))).toHaveLength(3);
  });

  it("is finite and compact in every mode", () => {
    for (const w of ALL_MODES) {
      for (const p of handParts(handShape(w, 1))) {
        expect(finite(p.cx, p.cy, p.w, p.h, p.r, p.angle)).toBe(true);
        expect(p.w).toBeGreaterThan(0);
        expect(p.h).toBeGreaterThan(0);
        expect(Math.hypot(p.cx, p.cy)).toBeLessThan(26);
      }
    }
  });

  it("puts a thick thumb clearly above the thumbs-up fist", () => {
    const [body, thumb] = handParts(handShape({ ...NONE, thumbs: 1 }, 1));
    expect(thumb.cy).toBeLessThan(body.cy - 8);
    expect(thumb.w).toBeGreaterThanOrEqual(7);
    expect(Math.abs(thumb.angle + Math.PI)).toBeLessThan(0.05); // vertical
  });

  it("draws the pointing finger thin, long and above the closed body", () => {
    const [body, , finger] = handParts(handShape({ ...NONE, scratch: 1 }, -1));
    expect(finger.h).toBeGreaterThanOrEqual(10);
    expect(finger.w).toBeLessThan(6);
    expect(finger.cy).toBeLessThan(body.cy - 8);
  });
});

describe("pans", () => {
  it("grips with a closed mitten on both hands", () => {
    for (const side of [-1, 1] as const) {
      const s = handShape({ ...NONE, pans: 1 }, side);
      expect(s.pointLen).toBe(0);
      expect(s.bodyH).toBeLessThanOrEqual(13);
    }
  });
});

describe("burstSegments", () => {
  const c = { x: 0, y: 26 };

  it("is empty with no impact", () => {
    expect(burstSegments(c, 0, 12)).toEqual([]);
  });

  it("radiates short outward noise lines that grow with the impact", () => {
    const strong = burstSegments(c, 1, 12);
    const weak = burstSegments(c, 0.3, 12);
    expect(strong).toHaveLength(12);
    for (const s of strong) {
      for (const v of [s.from.x, s.from.y, s.to.x, s.to.y]) expect(Number.isFinite(v)).toBe(true);
      const r0 = Math.hypot(s.from.x - c.x, s.from.y - c.y);
      const r1 = Math.hypot(s.to.x - c.x, s.to.y - c.y);
      expect(r1).toBeGreaterThan(r0);
      expect(r1).toBeLessThanOrEqual(24.01); // stays inside the canvas above the head
    }
    const len = (g: typeof strong) => Math.hypot(g[0].to.x - g[0].from.x, g[0].to.y - g[0].from.y);
    expect(len(strong)).toBeGreaterThan(len(weak));
  });
});

describe("starPoints", () => {
  const c = { x: 0, y: -56 };

  it("is empty with no impact", () => {
    expect(starPoints(c, 0, 8)).toEqual([]);
  });

  it("alternates tips and notches around the contact point, bigger when stronger", () => {
    const strong = starPoints(c, 1, 8);
    const weak = starPoints(c, 0.3, 8);
    expect(strong).toHaveLength(16);
    const radius = (p: { x: number; y: number }) => Math.hypot(p.x - c.x, p.y - c.y);
    strong.forEach((p, i) => {
      expect(Number.isFinite(p.x) && Number.isFinite(p.y)).toBe(true);
      expect(radius(p)).toBeLessThanOrEqual(16.01);
      if (i % 2 === 0) expect(radius(p)).toBeGreaterThan(radius(strong[i + 1]));
    });
    expect(Math.max(...strong.map(radius))).toBeGreaterThan(Math.max(...weak.map(radius)));
    expect(Math.max(...strong.map(radius))).toBeGreaterThanOrEqual(14);
  });
});

describe("vibeSegments", () => {
  it("is empty at rest and draws short ticks around the pan after a hit", () => {
    expect(vibeSegments(0)).toEqual([]);
    const ticks = vibeSegments(1);
    expect(ticks.length).toBeGreaterThanOrEqual(3);
    for (const s of ticks) {
      for (const v of [s.from.x, s.from.y, s.to.x, s.to.y]) expect(Number.isFinite(v)).toBe(true);
      expect(Math.hypot(s.to.x - s.from.x, s.to.y - s.from.y)).toBeGreaterThan(1);
      expect(Math.hypot(s.to.x - s.from.x, s.to.y - s.from.y)).toBeLessThan(10);
      // above the pan's rim in its local frame (y up is negative)
      expect(s.from.y).toBeLessThan(0);
    }
  });
});

describe("drawn fingertip", () => {
  it("touches the head outline at the upper left while scratching", () => {
    const w: HandWeights = { ...NONE, scratch: 1 };
    for (const t of [0, 0.03, 0.06, 0.1, 0.15, 0.2, 0.24]) {
      const hand = handPositions(w, t, 1).left;
      const tip = drawnFingertip(hand, handShape(w, -1));
      expect(Math.abs(headOutlineDistance(tip))).toBeLessThanOrEqual(3);
      expect(tip.y).toBeLessThan(-18); // crown or upper side
      expect(tip.x).toBeLessThan(-8);
      expect(tip.x).toBeGreaterThan(-32);
    }
  });

  it("is mirrored for the right hand", () => {
    const shape = { ...handShape({ ...NONE, scratch: 1 }, -1), side: 1 as const };
    const l = drawnFingertip({ x: 0, y: 0 }, { ...shape, side: -1 });
    const r = drawnFingertip({ x: 0, y: 0 }, shape);
    expect(r.x).toBeCloseTo(-l.x, 6);
    expect(r.y).toBeCloseTo(l.y, 6);
  });
});

describe("laptopGeometry (back of the lid, deck wider than the lid)", () => {
  const g = laptopGeometry();

  it("keeps the chin visible: the lid sits below the head", () => {
    expect(g.lid.y).toBeGreaterThanOrEqual(28);
    expect(g.lid.w).toBeLessThanOrEqual(50); // smaller than the head's 68
  });

  it("has a deck strip protruding on both sides of the lid, in front of its foot", () => {
    expect(g.base.x).toBeLessThanOrEqual(g.lid.x - 12);
    expect(g.base.x + g.base.w).toBeGreaterThanOrEqual(g.lid.x + g.lid.w + 12);
    expect(g.base.y).toBeLessThan(g.lid.y + g.lid.h);
    expect(g.base.y + g.base.h).toBeGreaterThan(g.lid.y + g.lid.h);
  });

  it("puts a small emblem inside the lid", () => {
    const e = g.emblem;
    expect(e.r).toBeGreaterThan(1);
    expect(e.cx - e.r).toBeGreaterThan(g.lid.x);
    expect(e.cx + e.r).toBeLessThan(g.lid.x + g.lid.w);
    expect(e.cy - e.r).toBeGreaterThan(g.lid.y);
    expect(e.cy + e.r).toBeLessThan(g.lid.y + g.lid.h);
  });

  it("spills the screen light upward from the lid onto the visor", () => {
    const s = g.spill;
    expect(s.bottomY).toBe(g.lid.y);
    expect(s.topY).toBeLessThan(s.bottomY - 10);
    expect(s.topY).toBeGreaterThanOrEqual(-20);
    expect(s.halfWidth).toBeLessThanOrEqual(27);
  });

  it("has no viewer-facing code lines or keys", () => {
    expect(g).not.toHaveProperty("codeLines");
    expect(g).not.toHaveProperty("keys");
  });

  it("fits the drawing space", () => {
    expect(Math.abs(g.base.x)).toBeLessThan(48);
    expect(g.base.y + g.base.h).toBeLessThan(75);
  });

  it("taps with the hands on the protruding deck edges, clear of the lid", () => {
    const w: HandWeights = { ...NONE, typing: 1 };
    const lidHalf = g.lid.w / 2;
    for (const t of [0, 0.06, 0.12, 0.18, 0.3]) {
      const h = handPositions(w, t, 1);
      for (const [hand, side] of [
        [h.left, -1],
        [h.right, 1],
      ] as const) {
        const body = handShape(w, side);
        const half = (body.bodyW * body.size) / 2;
        expect(Math.abs(hand.x) - half).toBeGreaterThan(lidHalf); // beside the lid
        expect(Math.abs(hand.x) + half).toBeLessThan(g.base.x * -1 + 0.01); // over the deck
        expect(hand.y).toBeGreaterThanOrEqual(g.base.y - 10);
        expect(hand.y).toBeLessThanOrEqual(g.base.y + 1);
      }
    }
    expect(HAND_REST.right.y).toBeLessThan(g.base.y); // resting hands hang above the deck
  });
});
