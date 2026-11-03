import { describe, expect, it, vi } from "vitest";
import { POSE_KEYS, POSES } from "./pose";
import {
  accentCss,
  backingStoreSize,
  BLINK_DURATION,
  makeContextGetter,
  composeFrame,
  createBlinker,
  createRig,
  lookVector,
  stepRig,
} from "./robot";
import { waveAlphas } from "./gesture";

const still = { x: 0, y: 0 };

describe("stepRig", () => {
  it("starts exactly on the initial pose", () => {
    const rig = createRig(POSES["idle"]);
    expect(composeFrame(rig, 0, 0, 0).pose).toEqual(POSES["idle"]);
  });

  it("converges to a new pose without a hard cut", () => {
    let rig = createRig(POSES["idle"]);
    const target = POSES["success"];
    rig = stepRig(rig, target, still, 1 / 60);
    const first = composeFrame(rig, 0, 0, 0).pose;
    // one frame later it has started moving but is not there yet
    expect(first.accentG).not.toBe(POSES["idle"].accentG);
    expect(Math.abs(first.scale - target.scale)).toBeGreaterThan(0.005);
    for (let i = 0; i < 240; i++) rig = stepRig(rig, target, still, 1 / 60);
    const settled = composeFrame(rig, 0, 0, 0).pose;
    for (const key of POSE_KEYS) expect(settled[key]).toBeCloseTo(target[key], 2);
  });

  it("eases the gaze toward the target", () => {
    let rig = createRig(POSES["idle"]);
    for (let i = 0; i < 240; i++) rig = stepRig(rig, POSES["idle"], { x: 1, y: -1 }, 1 / 60);
    const frame = composeFrame(rig, 0, 0, 0);
    expect(frame.gaze.x).toBeCloseTo(1, 2);
    expect(frame.gaze.y).toBeCloseTo(-1, 2);
  });
});

describe("createBlinker", () => {
  const seq = (values: number[]) => {
    let i = 0;
    return () => values[i++ % values.length];
  };

  it("does not blink before the first interval (>= 2 s)", () => {
    const b = createBlinker(() => 0, 0);
    expect(b.nextBlinkAt()).toBe(2);
    expect(b.amount(1.99)).toBe(0);
  });

  it("draws every interval inside 2-6 s", () => {
    expect(createBlinker(() => 0, 0).nextBlinkAt()).toBe(2);
    expect(createBlinker(() => 0.999999, 0).nextBlinkAt()).toBeCloseTo(6, 3);
    const b = createBlinker(seq([0.5, 0.1, 0.9]), 10);
    const starts: number[] = [];
    for (let t = 10; t < 60; t += 0.01) {
      b.amount(t);
      const next = b.nextBlinkAt();
      if (starts[starts.length - 1] !== next) starts.push(next);
    }
    expect(starts.length).toBeGreaterThan(5);
    for (let i = 1; i < starts.length; i++) {
      const gap = starts[i] - starts[i - 1];
      expect(gap).toBeGreaterThanOrEqual(2 + BLINK_DURATION - 1e-6);
      expect(gap).toBeLessThanOrEqual(6 + BLINK_DURATION + 1e-6);
    }
  });

  it("closes fully mid-blink and reopens", () => {
    const b = createBlinker(() => 0, 0);
    expect(b.amount(2 + BLINK_DURATION / 2)).toBeCloseTo(1, 5);
    expect(b.amount(2 + BLINK_DURATION + 0.01)).toBe(0);
  });

  it("survives a long frame gap without stalling", () => {
    const b = createBlinker(() => 0, 0);
    expect(b.amount(100)).toBe(0);
    expect(b.nextBlinkAt()).toBeGreaterThan(100);
  });
});

describe("lookVector", () => {
  const center = { x: 100, y: 50 };

  it("is neutral without a cursor", () => {
    expect(lookVector(null, center, 120)).toEqual({ x: 0, y: 0 });
  });

  it("points toward the cursor, scaled by distance", () => {
    const near = lookVector({ x: 130, y: 50 }, center, 120);
    expect(near.y).toBeCloseTo(0, 5);
    expect(near.x).toBeGreaterThan(0);
    expect(near.x).toBeLessThan(1);
    const far = lookVector({ x: 1000, y: 50 }, center, 120);
    expect(far.x).toBeCloseTo(1, 5);
  });

  it("never exceeds the unit circle", () => {
    const v = lookVector({ x: -500, y: 900 }, center, 120);
    expect(Math.hypot(v.x, v.y)).toBeLessThanOrEqual(1 + 1e-9);
    expect(v.x).toBeLessThan(0);
    expect(v.y).toBeGreaterThan(0);
  });

  it("is neutral with the cursor on the centre", () => {
    expect(lookVector(center, center, 120)).toEqual({ x: 0, y: 0 });
  });
});

describe("composeFrame", () => {
  const rig = createRig(POSES["success"]);

  it("closes the eyes with the blink", () => {
    const open = composeFrame(rig, 0, 0, 1).eyeOpen;
    const shut = composeFrame(rig, 0, 1, 1).eyeOpen;
    expect(shut).toBe(0);
    expect(open).toBeGreaterThan(0);
  });

  it("moves with breathing and bounce when motion is full", () => {
    const later = stepRig(rig, POSES["success"], still, 0.1);
    const a = composeFrame(rig, 0.1, 0, 1);
    const b = composeFrame(later, 0.2, 0, 1);
    expect(a.offsetY).not.toBe(b.offsetY);
  });

  it("drops breathing, bounce, shake and wobble under reduced motion", () => {
    for (const state of ["success", "error", "thinking", "acting", "listening", "reminder-alert"] as const) {
      const r = createRig(POSES[state]);
      for (const t of [0, 0.1, 0.37, 1.2]) {
        const f = composeFrame(r, t, 0, 0);
        expect(f.offsetY).toBe(r.pose.headLift.value);
        expect(f.offsetX).toBe(r.pose.headShift.value);
        expect(f.question).toEqual({ dy: 0, rot: 0 });
        expect(f.impact).toBe(0);
        expect(f.panClose).toBe(0);
        expect(f.clang.alpha).toBe(0);
        expect(f.shock.every((r) => r.alpha === 0)).toBe(true);
        expect(f.vibe).toEqual({ alpha: 0, dx: 0 });
        expect(f.waves).toEqual(waveAlphas(0, 0));
        expect(f.scale).toBe(r.pose.scale.value);
        expect(f.antennaAngle).toBe(r.pose.antennaLean.value);
        expect(f.tilt).toBe(r.pose.headTilt.value);
        expect(f.spiralAngle).toBe(0);
      }
    }
  });

  it("chuckles on success (small rhythmic horizontal shake)", () => {
    const xs = [0.01, 0.03, 0.05, 0.07, 0.09, 0.11].map((t) => composeFrame(rig, t, 0, 1).offsetX);
    expect(Math.max(...xs) - Math.min(...xs)).toBeGreaterThan(0.3);
    for (const x of xs) expect(Math.abs(x)).toBeLessThan(3);
  });

  it("wobbles the head and spins the spiral on error", () => {
    const err = createRig(POSES["error"]);
    const tilts = [0, 0.1, 0.2, 0.3].map((t) => composeFrame(err, t, 0, 1).tilt);
    expect(Math.max(...tilts) - Math.min(...tilts)).toBeGreaterThan(0.02);
    expect(composeFrame(err, 0.2, 0, 1).spiralAngle).not.toBe(composeFrame(err, 0.4, 0, 1).spiralAngle);
  });

  it("kicks the head on each pan hit and is calm between hits and under reduced motion", () => {
    const alert = createRig(POSES["reminder-alert"]);
    const hit = composeFrame(alert, 0.25, 0, 1);
    const apart = composeFrame(alert, 0, 0, 1);
    expect(hit.impact).toBeCloseTo(1, 6);
    expect(apart.impact).toBe(0);
    expect(hit.offsetY - alert.pose.headLift.value).toBeGreaterThan(3);
    expect(hit.clang.alpha).toBeGreaterThan(0.9);
    expect(hit.shock[0].alpha).toBeGreaterThan(0.5);
    expect(Math.abs(hit.tilt - alert.pose.headTilt.value)).toBeGreaterThan(0.05);
    expect(apart.clang.alpha).toBeLessThan(hit.clang.alpha);
    expect(composeFrame(createRig(POSES["idle"]), 0.25, 0, 1).clang.alpha).toBe(0);
    expect(composeFrame(alert, 0.25, 0, 0).impact).toBe(0);
    expect(composeFrame(createRig(POSES["idle"]), 0.25, 0, 1).impact).toBe(0);
  });

  it("shows a static alert mark only under reduced motion", () => {
    const alert = createRig(POSES["reminder-alert"]);
    expect(composeFrame(alert, 0.3, 0, 0).alertMark).toBe(1);
    expect(composeFrame(alert, 0.3, 0, 1).alertMark).toBe(0);
    expect(composeFrame(createRig(POSES["idle"]), 0.3, 0, 0).alertMark).toBe(0);
  });

  it("glances up toward the scratching hand and away from the cupped ear", () => {
    const think = composeFrame(createRig(POSES["thinking"]), 0.2, 0, 1);
    expect(think.gaze.x).toBeLessThan(0);
    expect(think.gaze.y).toBeLessThan(0);
    const listen = composeFrame(createRig(POSES["listening"]), 0.2, 0, 1);
    expect(listen.gaze.x).toBeLessThan(0);
    expect(listen.offsetX).toBeGreaterThan(0);
    expect(listen.tilt).toBeGreaterThan(0.15);
    expect(listen.waves).toHaveLength(3);
  });

  it("puts the scratching hand above the head and the typing hands on the deck edges", () => {
    const think = composeFrame(createRig(POSES["thinking"]), 0.2, 0, 1);
    expect(think.hands.left.y).toBeLessThan(-38);
    expect(think.hands.left.x).toBeLessThan(-10);
    const act = composeFrame(createRig(POSES["acting"]), 0.2, 0, 1);
    for (const hand of [act.hands.left, act.hands.right]) {
      expect(hand.y).toBeGreaterThan(36);
      expect(hand.y).toBeLessThan(50);
      expect(Math.abs(hand.x)).toBeGreaterThan(26); // beside the lid, not over it
    }
  });
});

describe("bounce phase", () => {
  it("accumulates, so a frequency change never jumps the hop", () => {
    let r = createRig(POSES["success"]);
    const hz = (state: "success" | "acting") => POSES[state];
    let prev = composeFrame(r, 0, 0, 1).offsetY;
    let worst = 0;
    for (let i = 0; i < 120; i++) {
      r = stepRig(r, hz(i < 30 ? "success" : "acting"), still, 1 / 60);
      const y = composeFrame(r, i / 60, 0, 1).offsetY;
      worst = Math.max(worst, Math.abs(y - prev));
      prev = y;
    }
    // with bounce amplitude <= 5 and hz <= 5, one 60 Hz frame moves < 2 units
    expect(worst).toBeLessThan(2);
  });

  it("does not advance without bounce frequency or with a bad dt", () => {
    const r = createRig(POSES["idle"]);
    expect(stepRig(r, POSES["idle"], still, 1).phase).toBe(0);
    const b = createRig(POSES["success"]);
    expect(stepRig(b, POSES["success"], still, Number.NaN).phase).toBe(0);
  });
});

describe("non-finite input", () => {
  it("lookVector ignores a non-finite cursor, centre or range", () => {
    const c = { x: 10, y: 10 };
    expect(lookVector({ x: Number.NaN, y: 3 }, c, 100)).toEqual({ x: 0, y: 0 });
    expect(lookVector({ x: Infinity, y: 3 }, c, 100)).toEqual({ x: 0, y: 0 });
    expect(lookVector({ x: 1, y: 3 }, { x: Number.NaN, y: 0 }, 100)).toEqual({ x: 0, y: 0 });
    expect(lookVector({ x: 1, y: 3 }, c, 0)).toEqual({ x: 0, y: 0 });
  });

  it("stepRig keeps every value finite when the gaze target is not", () => {
    let r = createRig(POSES["idle"]);
    r = stepRig(r, POSES["idle"], { x: Number.NaN, y: Infinity }, 1 / 60);
    const f = composeFrame(r, 0, 0, 1);
    expect(Number.isFinite(f.gaze.x) && Number.isFinite(f.gaze.y)).toBe(true);
  });
});

describe("backingStoreSize", () => {
  it("scales the CSS size by the device pixel ratio", () => {
    expect(backingStoreSize(72, 1)).toBe(72);
    expect(backingStoreSize(72, 1.5)).toBe(108);
    expect(backingStoreSize(72, 2.5)).toBe(180);
  });

  it("falls back to 1 for a bad ratio", () => {
    for (const bad of [0, -1, Number.NaN, Infinity]) expect(backingStoreSize(72, bad)).toBe(72);
  });
});

describe("makeContextGetter", () => {
  it("returns the context when available", () => {
    const ctx = {} as CanvasRenderingContext2D;
    const warn = vi.fn();
    expect(makeContextGetter(warn)({ getContext: () => ctx })).toBe(ctx);
    expect(warn).not.toHaveBeenCalled();
  });

  it("warns once when the 2d context is unavailable", () => {
    const warn = vi.fn();
    const get = makeContextGetter(warn);
    expect(get({ getContext: () => null })).toBeNull();
    expect(get({ getContext: () => null })).toBeNull();
    expect(warn).toHaveBeenCalledTimes(1);
  });
});

describe("accentCss", () => {
  it("rounds and clamps channels", () => {
    const pose = { ...POSES["idle"], accentR: 300.4, accentG: -5, accentB: 10.6 };
    expect(accentCss(pose, 0.5)).toBe("rgba(255, 0, 11, 0.5)");
  });
});
