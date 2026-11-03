import { describe, expect, it } from "vitest";
import { ALL_STATES } from "../events";
import { POSES } from "./pose";
import { composeFrame, createRig, drawRobot, stepRig } from "./robot";

// A hand-written recording stand-in for CanvasRenderingContext2D, with only the
// members `drawRobot` uses. A missing member throws, so the smoke test also
// notices when drawing starts using something the stub does not know.

interface Recorder {
  calls: string[];
  badNumbers: string[];
  depth: number;
  minDepth: number;
}

function makeContext(): { ctx: CanvasRenderingContext2D; rec: Recorder } {
  const rec: Recorder = { calls: [], badNumbers: [], depth: 0, minDepth: 0 };
  const checkNumbers = (name: string, args: unknown[]) => {
    for (const a of args) {
      if (typeof a === "number" && !Number.isFinite(a)) rec.badNumbers.push(`${name}(${String(a)})`);
    }
  };
  const target: Record<string, unknown> = {};
  const record = (name: string) => (...args: unknown[]) => {
    rec.calls.push(name);
    checkNumbers(name, args);
  };
  for (const name of [
    "clearRect", "scale", "translate", "rotate", "setTransform", "beginPath", "closePath",
    "moveTo", "lineTo", "quadraticCurveTo", "bezierCurveTo", "arc", "ellipse", "roundRect",
    "fill", "stroke", "clip", "fillRect", "strokeText", "fillText",
  ]) {
    target[name] = record(name);
  }
  target.save = () => {
    rec.calls.push("save");
    rec.depth += 1;
  };
  target.restore = () => {
    rec.calls.push("restore");
    rec.depth -= 1;
    rec.minDepth = Math.min(rec.minDepth, rec.depth);
  };
  target.createLinearGradient = (...args: unknown[]) => {
    rec.calls.push("createLinearGradient");
    checkNumbers("createLinearGradient", args);
    return {
      addColorStop: (offset: number, color: string) => {
        checkNumbers("addColorStop", [offset]);
        if (typeof color !== "string") rec.badNumbers.push("addColorStop(color)");
      },
    };
  };
  // Numeric state properties must stay finite too.
  for (const prop of ["lineWidth", "globalAlpha", "shadowBlur", "miterLimit"]) {
    let value = 0;
    Object.defineProperty(target, prop, {
      get: () => value,
      set: (v: number) => {
        checkNumbers(prop, [v]);
        value = v;
      },
    });
  }
  const ctx = new Proxy(target, {
    get(obj, key: string) {
      if (key in obj) return obj[key];
      // Plain style properties (fillStyle, font, lineCap...) read back as unset.
      return undefined;
    },
    set(obj, key: string, v) {
      // Only style-like properties may be assigned without a stub entry.
      if (typeof v === "number") checkNumbers(key, [v]);
      obj[key] = v;
      return true;
    },
  });
  return { ctx: ctx as unknown as CanvasRenderingContext2D, rec };
}

// Several moments, including the pan hit (about 0.25 s into each half second).
const TIMES = [0, 0.04, 0.12, 0.25, 0.26, 0.31, 0.5, 0.75, 1.37, 12.3];

describe("drawRobot with a recording canvas", () => {
  for (const reducedMotion of [false, true]) {
    for (const state of ALL_STATES) {
      it(`${state} (${reducedMotion ? "reduced" : "full"} motion): no errors, finite numbers, balanced save/restore`, () => {
        let rig = createRig(POSES.idle);
        // A few frames of springs from idle so blended poses are exercised too.
        for (let i = 0; i < 12; i++) rig = stepRig(rig, POSES[state], { x: 0.4, y: -0.3 }, 1 / 60);
        const settled = createRig(POSES[state]);
        for (const r of [rig, settled]) {
          for (const t of TIMES) {
            const { ctx, rec } = makeContext();
            const frame = composeFrame(r, t, 0.3, reducedMotion ? 0 : 1);
            expect(() => drawRobot(ctx, frame, 96)).not.toThrow();
            expect(rec.badNumbers).toEqual([]);
            expect(rec.depth).toBe(0);
            expect(rec.minDepth).toBe(0);
            expect(rec.calls.length).toBeGreaterThan(20);
          }
        }
      });
    }
  }

  it("draws the hit effects only with full motion on the alert state", () => {
    const count = (motion: number, t: number) => {
      const { ctx, rec } = makeContext();
      drawRobot(ctx, composeFrame(createRig(POSES["reminder-alert"]), t, 0, motion), 96);
      return rec.calls.filter((c) => c === "fillText" || c === "strokeText").length;
    };
    expect(count(1, 0.26)).toBeGreaterThan(0); // CLANG
    expect(count(0, 0.26)).toBe(2); // only the static "!"
  });
});
