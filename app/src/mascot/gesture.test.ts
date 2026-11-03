import { describe, expect, it } from "vitest";
import { ALL_STATES } from "../events";
import {
  HAND_REST,
  LOOP_PERIOD,
  CLANG_SECONDS,
  IMPACT_SECONDS,
  PAN_HANDLE,
  PAN_R,
  SHOCK_DELAY,
  SHOCK_SECONDS,
  clangPop,
  panPose,
  panVibe,
  shockwaves,
  SPIRAL_PERIOD,
  handPositions,
  impactAlpha,
  panCloseness,
  spiralAngle,
  questionBob,
  spiralPoints,
  typingPress,
  waveAlphas,
  type HandWeights,
} from "./gesture";
import { POSES } from "./pose";

const weightsOf = (state: (typeof ALL_STATES)[number]): HandWeights => {
  const p = POSES[state];
  return {
    scratch: p.handScratch,
    thumbs: p.handThumbs,
    typing: p.handTyping,
    raise: p.handRaise,
    cup: p.handCup,
    pans: p.handPans,
  };
};

const NONE: HandWeights = { scratch: 0, thumbs: 0, typing: 0, raise: 0, cup: 0, pans: 0 };
const times = Array.from({ length: 400 }, (_, i) => i * 0.0173);

describe("handPositions", () => {
  it("rests at the rest positions with no gesture and no motion", () => {
    const h = handPositions(NONE, 3.7, 0);
    expect(h.left).toEqual(HAND_REST.left);
    expect(h.right).toEqual(HAND_REST.right);
  });

  it("is finite and inside the drawing box for every state, with and without motion", () => {
    for (const state of ALL_STATES) {
      for (const motion of [0, 1]) {
        for (const t of times) {
          const h = handPositions(weightsOf(state), t, motion);
          for (const v of [h.left, h.right]) {
            expect(Number.isFinite(v.x) && Number.isFinite(v.y)).toBe(true);
            expect(Math.abs(v.x)).toBeLessThanOrEqual(62);
            expect(v.y).toBeGreaterThanOrEqual(-55);
            expect(v.y).toBeLessThanOrEqual(75);
          }
        }
      }
    }
  });

  it("is periodic with the loop period", () => {
    for (const state of ALL_STATES) {
      for (const t of [0.1, 0.37, 0.8]) {
        const a = handPositions(weightsOf(state), t, 1);
        const b = handPositions(weightsOf(state), t + LOOP_PERIOD * 3, 1);
        expect(b.left.x).toBeCloseTo(a.left.x, 6);
        expect(b.left.y).toBeCloseTo(a.left.y, 6);
        expect(b.right.x).toBeCloseTo(a.right.x, 6);
        expect(b.right.y).toBeCloseTo(a.right.y, 6);
      }
    }
  });

  it("is static under reduced motion", () => {
    for (const state of ALL_STATES) {
      const a = handPositions(weightsOf(state), 0.13, 0);
      const b = handPositions(weightsOf(state), 0.71, 0);
      expect(b).toEqual(a);
    }
  });

  it("raises the left hand above the head to scratch while the right rests", () => {
    const lefts = times.map((t) => handPositions(weightsOf("thinking"), t, 1).left);
    for (const p of lefts) {
      expect(p.y).toBeLessThan(-38); // above the crown, not beside the head
      expect(p.x).toBeLessThan(-10);
      expect(p.x).toBeGreaterThan(-35);
    }
    const xs = lefts.map((p) => p.x);
    expect(Math.max(...xs) - Math.min(...xs)).toBeGreaterThan(5); // a quick stroke
    const right = handPositions(weightsOf("thinking"), 0.4, 1).right;
    expect(Math.abs(right.x - HAND_REST.right.x)).toBeLessThan(1);
    expect(Math.abs(right.y - HAND_REST.right.y)).toBeLessThan(2);
  });

  it("types with both hands alternately", () => {
    const w = weightsOf("acting");
    let leftDownRightUp = false;
    let rightDownLeftUp = false;
    for (const t of times) {
      const h = handPositions(w, t, 1);
      const base = handPositions(w, 0, 0);
      const dl = h.left.y - base.left.y;
      const dr = h.right.y - base.right.y;
      if (dl > 2 && dr < 0.5) leftDownRightUp = true;
      if (dr > 2 && dl < 0.5) rightDownLeftUp = true;
    }
    expect(leftDownRightUp && rightDownLeftUp).toBe(true);
  });

  it("raises the right hand for a thumbs-up and for the question", () => {
    for (const s of ["success", "confirmation-needed"] as const) {
      const h = handPositions(weightsOf(s), 0.25, 1);
      expect(h.right.y).toBeLessThan(HAND_REST.right.y - 20);
    }
  });
});

describe("typingPress", () => {
  it("is in [0, 1], alternates between the hands and is zero without motion", () => {
    let leftOnly = false;
    let rightOnly = false;
    for (const t of times) {
      for (const side of [-1, 1] as const) {
        const p = typingPress(t, side, 1);
        expect(p).toBeGreaterThanOrEqual(0);
        expect(p).toBeLessThanOrEqual(1);
        expect(typingPress(t, side, 0)).toBe(0);
      }
      if (typingPress(t, -1, 1) > 0.5 && typingPress(t, 1, 1) === 0) leftOnly = true;
      if (typingPress(t, 1, 1) > 0.5 && typingPress(t, -1, 1) === 0) rightOnly = true;
    }
    expect(leftOnly && rightOnly).toBe(true);
  });
});

describe("cupEar (listening)", () => {
  it("brings the right hand to the side plate and leaves the left at rest", () => {
    for (const t of times) {
      const h = handPositions(weightsOf("listening"), t, 1);
      expect(h.right.x).toBeGreaterThan(40); // just outside the side plate (x 33..40)
      expect(h.right.x).toBeLessThan(54);
      expect(Math.abs(h.right.y)).toBeLessThan(10);
      expect(Math.abs(h.left.x - HAND_REST.left.x)).toBeLessThan(1);
    }
  });

  it("tips the head toward the hand's side and glances away from it", () => {
    const hand = handPositions(weightsOf("listening"), 0, 1).right;
    const pose = POSES["listening"];
    expect(Math.sign(pose.headTilt)).toBe(Math.sign(hand.x));
    expect(Math.sign(pose.headShift)).toBe(Math.sign(hand.x));
    expect(pose.headTilt).toBeGreaterThanOrEqual(0.15);
    expect(pose.headTilt).toBeLessThanOrEqual(0.25);
    expect(pose.lookX * hand.x).toBeLessThan(0); // opposite side
  });
});

describe("pans (reminder-alert)", () => {
  const w = weightsOf("reminder-alert");
  const HEAD_TOP = -31; // top of the head outline
  const HIT = 0.25; // first hit; then every half second

  it("holds both pans above the head, apart or together", () => {
    for (const t of times) {
      const d = panCloseness(t, 1);
      expect(d).toBeGreaterThanOrEqual(0);
      expect(d).toBeLessThanOrEqual(1);
      for (const side of [-1, 1] as const) {
        const p = panPose(side, d);
        for (const v of [p.center.x, p.center.y, p.tip.x, p.tip.y, p.hand.x, p.hand.y, p.angle]) {
          expect(Number.isFinite(v)).toBe(true);
        }
        expect(p.center.y).toBeLessThan(HEAD_TOP - 10);
        expect(p.tip.y).toBeLessThan(HEAD_TOP - 10);
        expect(Math.sign(p.center.x)).toBe(side);
        expect(Math.abs(p.hand.x)).toBeLessThanOrEqual(62);
        expect(p.hand.y).toBeGreaterThan(-55);
        // the handle runs down and outward from the pan to the hand
        expect(p.hand.y).toBeGreaterThan(p.center.y + 8);
        expect(Math.abs(p.hand.x)).toBeGreaterThan(Math.abs(p.center.x));
      }
    }
  });

  it("connects each pan to its mitten with a long straight handle", () => {
    for (const t of times) {
      const d = panCloseness(t, 1);
      const h = handPositions(w, t, 1);
      for (const [side, hand] of [[-1, h.left], [1, h.right]] as const) {
        const p = panPose(side, d);
        expect(hand.x).toBeCloseTo(p.hand.x, 6);
        expect(hand.y).toBeCloseTo(p.hand.y, 6);
        expect(Math.hypot(p.hand.x - p.center.x, p.hand.y - p.center.y)).toBeCloseTo(PAN_HANDLE, 6);
      }
    }
    expect(PAN_HANDLE - PAN_R).toBeGreaterThanOrEqual(2 * PAN_R); // a stick, not a ring
  });

  it("brings the rim edges together above the head centre at the hit", () => {
    expect(panCloseness(HIT, 1)).toBeCloseTo(1, 6);
    expect(panCloseness(0, 1)).toBeCloseTo(0, 6);
    const l = panPose(-1, panCloseness(HIT, 1));
    const r = panPose(1, panCloseness(HIT, 1));
    expect(l.tip.x).toBeCloseTo(0, 6);
    expect(r.tip.x).toBeCloseTo(0, 6);
    expect(l.tip.y).toBeCloseTo(r.tip.y, 6);
    expect(l.tip.y).toBeLessThan(HEAD_TOP);
    const apartL = panPose(-1, 0);
    const apartR = panPose(1, 0);
    expect(apartR.tip.x - apartL.tip.x).toBeGreaterThan(30);
    // tilted toward each other: the pan turns from steep to flatter as it swings in
    expect(l.angle).toBeLessThan(apartL.angle);
  });

  it("hits two to three times a second (period divides the loop period)", () => {
    for (const t of [0.05, 0.2, 0.6]) {
      expect(panCloseness(t + 0.5, 1)).toBeCloseTo(panCloseness(t, 1), 6);
      expect(impactAlpha(t + 0.5, 1)).toBeCloseTo(impactAlpha(t, 1), 6);
      expect(shockwaves(t + 0.5, 1)[0].alpha).toBeCloseTo(shockwaves(t, 1)[0].alpha, 6);
    }
    const hits = times.filter((t, i) => i > 0 && impactAlpha(t, 1) > 0.95).length;
    expect(hits).toBeGreaterThan(0);
  });

  it("flashes the impact only around the hits, bounded and finite", () => {
    for (const t of times) {
      const a = impactAlpha(t, 1);
      expect(Number.isFinite(a)).toBe(true);
      expect(a).toBeGreaterThanOrEqual(0);
      expect(a).toBeLessThanOrEqual(1);
      if (a > 0.5) expect(panCloseness(t, 1)).toBeGreaterThan(0.8);
    }
    expect(impactAlpha(HIT, 1)).toBeCloseTo(1, 6);
    expect(impactAlpha(HIT + 0.2, 1)).toBe(0);
    expect(impactAlpha(HIT + IMPACT_SECONDS * 2, 1)).toBe(0);
  });

  it("expands two shockwave rings that fade out", () => {
    for (const t of times) {
      const rings = shockwaves(t, 1);
      expect(rings).toHaveLength(2);
      for (const r of rings) {
        expect(Number.isFinite(r.radius) && Number.isFinite(r.alpha)).toBe(true);
        expect(r.alpha).toBeGreaterThanOrEqual(0);
        expect(r.alpha).toBeLessThanOrEqual(1);
        expect(r.radius).toBeGreaterThanOrEqual(0);
        expect(r.radius).toBeLessThanOrEqual(24);
      }
    }
    const early = shockwaves(HIT + 0.02, 1);
    const later = shockwaves(HIT + 0.15, 1);
    expect(early[0].alpha).toBeGreaterThan(0.3);
    expect(later[0].radius).toBeGreaterThan(early[0].radius);
    expect(later[0].alpha).toBeLessThan(early[0].alpha);
    // the second ring trails the first; between hits nothing is visible
    expect(shockwaves(HIT + 0.1, 1)[1].radius).toBeLessThan(shockwaves(HIT + 0.1, 1)[0].radius);
    expect(shockwaves(HIT + SHOCK_SECONDS + SHOCK_DELAY + 0.01, 1).every((r) => r.alpha === 0)).toBe(true);
  });

  it("pops a CLANG word at each hit: scales up, fades, alternates rotation", () => {
    for (const t of times) {
      const c = clangPop(t, 1);
      for (const v of [c.alpha, c.scale, c.rot]) expect(Number.isFinite(v)).toBe(true);
      expect(c.alpha).toBeGreaterThanOrEqual(0);
      expect(c.alpha).toBeLessThanOrEqual(1);
      expect(c.scale).toBeGreaterThanOrEqual(0);
      expect(c.scale).toBeLessThanOrEqual(1.6);
      expect(Math.abs(c.rot)).toBeLessThanOrEqual(0.2);
    }
    const start = clangPop(HIT + 0.01, 1);
    const end = clangPop(HIT + CLANG_SECONDS * 0.9, 1);
    expect(start.alpha).toBeGreaterThan(0.9);
    expect(end.alpha).toBeLessThan(start.alpha);
    expect(end.scale).toBeGreaterThan(start.scale);
    expect(clangPop(HIT + CLANG_SECONDS + 0.01, 1).alpha).toBe(0);
    // the next hit tips it the other way and moves it to the other side
    const next = clangPop(HIT + 0.5 + 0.01, 1);
    expect(Math.sign(next.rot)).toBe(-Math.sign(start.rot));
    expect(next.side).toBe(-start.side);
  });

  it("vibrates the pans right after the hit and settles", () => {
    for (const t of times) {
      const v = panVibe(t, 1);
      expect(Number.isFinite(v.alpha) && Number.isFinite(v.dx)).toBe(true);
      expect(v.alpha).toBeGreaterThanOrEqual(0);
      expect(v.alpha).toBeLessThanOrEqual(1);
      expect(Math.abs(v.dx)).toBeLessThanOrEqual(2);
    }
    expect(panVibe(HIT + 0.01, 1).alpha).toBeGreaterThan(0.8);
    expect(panVibe(HIT + 0.3, 1)).toEqual({ alpha: 0, dx: 0 });
  });

  it("is static and silent under reduced motion: pans held raised apart, no effects", () => {
    for (const t of times) {
      expect(impactAlpha(t, 0)).toBe(0);
      expect(panCloseness(t, 0)).toBe(0);
      expect(shockwaves(t, 0).every((r) => r.alpha === 0)).toBe(true);
      expect(clangPop(t, 0).alpha).toBe(0);
      expect(panVibe(t, 0)).toEqual({ alpha: 0, dx: 0 });
      expect(handPositions(w, t, 0)).toEqual(handPositions(w, 0.5, 0));
    }
    const apart = panPose(-1, panCloseness(0.3, 0));
    expect(apart.center.y).toBeLessThan(HEAD_TOP - 10);
    expect(panPose(1, 0).tip.x - panPose(-1, 0).tip.x).toBeGreaterThan(30);
  });
});

describe("questionBob", () => {
  it("is bounded, periodic and static under reduced motion", () => {
    for (const t of times) {
      const b = questionBob(t, 1);
      expect(Number.isFinite(b.dy) && Number.isFinite(b.rot)).toBe(true);
      expect(Math.abs(b.dy)).toBeLessThanOrEqual(3);
      expect(Math.abs(b.rot)).toBeLessThanOrEqual(0.2);
      const later = questionBob(t + LOOP_PERIOD * 2, 1);
      expect(later.dy).toBeCloseTo(b.dy, 6);
      expect(later.rot).toBeCloseTo(b.rot, 6);
      expect(questionBob(t, 0)).toEqual({ dy: 0, rot: 0 });
    }
    expect(questionBob(0.1, 1).dy).not.toBeCloseTo(questionBob(0.35, 1).dy, 3);
  });
});

describe("waveAlphas", () => {
  it("gives three bounded, periodic pulses that arrive from the outside in", () => {
    for (const t of times) {
      const a = waveAlphas(t, 1);
      expect(a).toHaveLength(3);
      for (const v of a) {
        expect(Number.isFinite(v)).toBe(true);
        expect(v).toBeGreaterThanOrEqual(0);
        expect(v).toBeLessThanOrEqual(1);
      }
      const later = waveAlphas(t + LOOP_PERIOD, 1);
      a.forEach((v, i) => expect(later[i]).toBeCloseTo(v, 6));
    }
    // the outer arc (index 0) peaks before the inner one (index 2)
    const peak = (i: number) =>
      times.slice(0, 58).reduce((best, t) => (waveAlphas(t, 1)[i] > waveAlphas(best, 1)[i] ? t : best), 0);
    expect(peak(0)).toBeLessThan(peak(2));
  });

  it("is static under reduced motion", () => {
    expect(waveAlphas(0.2, 0)).toEqual(waveAlphas(3.7, 0));
    for (const v of waveAlphas(0.2, 0)) expect(v).toBeGreaterThan(0);
  });
});

describe("spiralAngle", () => {
  it("is bounded and periodic", () => {
    for (const t of times) {
      const a = spiralAngle(t, 1);
      expect(a).toBeGreaterThanOrEqual(0);
      expect(a).toBeLessThan(Math.PI * 2 + 1e-9);
      expect(spiralAngle(t + SPIRAL_PERIOD * 2, 1)).toBeCloseTo(a, 6);
    }
  });

  it("rotates over time and stops under reduced motion", () => {
    expect(spiralAngle(0.2, 1)).not.toBeCloseTo(spiralAngle(0.4, 1), 3);
    expect(spiralAngle(0.2, 0)).toBe(spiralAngle(5.5, 0));
  });
});

describe("spiralPoints", () => {
  it("stays inside its radius, starts at the centre and is finite", () => {
    const pts = spiralPoints(2.2, 7, 40, 1.3);
    expect(pts).toHaveLength(41);
    expect(pts[0]).toEqual({ x: 0, y: 0 });
    for (const p of pts) {
      expect(Number.isFinite(p.x) && Number.isFinite(p.y)).toBe(true);
      expect(Math.hypot(p.x, p.y)).toBeLessThanOrEqual(7 + 1e-9);
    }
    expect(Math.hypot(pts[40].x, pts[40].y)).toBeCloseTo(7, 6);
  });
});
