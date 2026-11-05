import { describe, expect, it } from "vitest";
import { ALL_STATES, type ViewPayload } from "../events";
import {
  GESTURES,
  POSE_KEYS,
  POSES,
  poseForView,
  type PoseParams,
} from "./pose";

describe("gesture table (owner feedback 2026-11-03)", () => {
  it("maps every state to the requested hands, eyes and prop", () => {
    expect(GESTURES.thinking).toEqual({ hands: "scratch", eyes: "capsule", mouth: "tilted", prop: "question" });
    expect(GESTURES.listening).toEqual({ hands: "cupEar", eyes: "capsule", mouth: "open", prop: "waves" });
    expect(GESTURES.success).toEqual({ hands: "thumbsUp", eyes: "happyArcs", mouth: "happy", prop: "none" });
    expect(GESTURES.error).toEqual({ hands: "rest", eyes: "spiral", mouth: "frown", prop: "none" });
    expect(GESTURES.acting).toEqual({ hands: "typing", eyes: "capsule", mouth: "flat", prop: "laptop" });
    expect(GESTURES["confirmation-needed"].hands).toBe("raised");
    expect(GESTURES["reminder-alert"]).toEqual({ hands: "pans", eyes: "capsule", mouth: "shout", prop: "impact" });
    for (const s of ["idle"] as const) {
      expect(GESTURES[s]).toEqual({ hands: "rest", eyes: "capsule", mouth: "smile", prop: "none" });
    }
    expect(Object.keys(GESTURES).sort()).toEqual([...ALL_STATES].sort());
  });

  it("gives every state its expected mouth, as exactly one 0/1 weight", () => {
    const expected = {
      idle: "smile",
      listening: "open",
      thinking: "tilted",
      acting: "flat",
      "confirmation-needed": "wavy",
      success: "happy",
      error: "frown",
      "reminder-alert": "shout",
    } as const;
    const keys = {
      smile: "mouthSmile", open: "mouthOpen", tilted: "mouthTilted", flat: "mouthFlat",
      wavy: "mouthWavy", happy: "mouthHappy", frown: "mouthFrown", shout: "mouthShout",
    } as const;
    for (const state of ALL_STATES) {
      expect(GESTURES[state].mouth).toBe(expected[state]);
      const weights = Object.values(keys).map((k) => POSES[state][k]);
      expect(weights.reduce((a, b) => a + b, 0)).toBe(1);
      expect(POSES[state][keys[expected[state]]]).toBe(1);
    }
  });

  it("encodes the gesture as 0/1 weights in the numeric pose", () => {
    for (const state of ALL_STATES) {
      const g = GESTURES[state];
      const p = POSES[state];
      expect(p.handScratch).toBe(g.hands === "scratch" ? 1 : 0);
      expect(p.handThumbs).toBe(g.hands === "thumbsUp" ? 1 : 0);
      expect(p.handTyping).toBe(g.hands === "typing" ? 1 : 0);
      expect(p.handRaise).toBe(g.hands === "raised" ? 1 : 0);
      expect(p.eyeArcs).toBe(g.eyes === "happyArcs" ? 1 : 0);
      expect(p.eyeSpiral).toBe(g.eyes === "spiral" ? 1 : 0);
      expect(p.propLaptop).toBe(g.prop === "laptop" ? 1 : 0);
      expect(p.handCup).toBe(g.hands === "cupEar" ? 1 : 0);
      expect(p.propQuestion).toBe(g.prop === "question" ? 1 : 0);
      expect(p.propWaves).toBe(g.prop === "waves" ? 1 : 0);
      expect(p.handPans).toBe(g.hands === "pans" ? 1 : 0);
      expect(p.propImpact).toBe(g.prop === "impact" ? 1 : 0);
    }
  });

  it("keeps the thinking gesture while an AskFollowUp question is spoken", () => {
    const asking: ViewPayload = { state: "thinking", caption: "Which Notepad window?" };
    // poseForView is the production path the mascot uses.
    expect(poseForView(asking)).toBe(POSES.thinking);
    expect(poseForView(asking).handScratch).toBe(1);
    expect(poseForView(null)).toBe(POSES.idle);
  });

  it("makes thinking glance up toward the scratching hand and listening glance away", () => {
    const thinking = POSES["thinking"];
    expect(thinking.lookX).toBeLessThan(0); // the hand is on the left
    expect(thinking.lookY).toBeLessThan(0); // up
    expect(thinking.headTilt).toBeLessThan(0); // tipped toward the left hand
    const listening = POSES["listening"];
    expect(listening.lookX).toBeLessThan(0); // the cupping hand is on the right
    expect(listening.headShift).toBeGreaterThan(0);
    for (const s of ALL_STATES.filter((x) => x !== "thinking" && x !== "listening")) {
      expect(POSES[s].lookX).toBe(0);
      expect(POSES[s].headShift).toBe(0);
    }
  });

  it("has the dizzy wobble only on error", () => {
    expect(POSES["error"].wobbleAmp).toBeGreaterThan(0);
    for (const s of ALL_STATES.filter((x) => x !== "error")) {
      expect(POSES[s].wobbleAmp).toBe(0);
    }
  });
});

const distance = (a: PoseParams, b: PoseParams) =>
  Math.sqrt(POSE_KEYS.reduce((sum, k) => sum + (a[k] - b[k]) ** 2, 0));

describe("pose table", () => {
  it("has a complete, finite pose for every one of the 8 states", () => {
    expect(ALL_STATES).toHaveLength(8);
    for (const state of ALL_STATES) {
      const pose = POSES[state];
      for (const key of POSE_KEYS) expect(Number.isFinite(pose[key])).toBe(true);
    }
    expect(Object.keys(POSES).sort()).toEqual([...ALL_STATES].sort());
  });

  it("keeps channel values in their valid ranges", () => {
    for (const state of ALL_STATES) {
      const p = POSES[state];
      expect(p.eyeOpen).toBeGreaterThanOrEqual(0);
      expect(p.eyeOpen).toBeLessThanOrEqual(1);
      expect(p.glow).toBeGreaterThanOrEqual(0);
      expect(p.glow).toBeLessThanOrEqual(1);
      for (const c of [p.accentR, p.accentG, p.accentB]) {
        expect(c).toBeGreaterThanOrEqual(0);
        expect(c).toBeLessThanOrEqual(255);
      }
      expect(p.scale).toBeGreaterThan(0);
    }
  });

  it("gives every state a visibly different look", () => {
    for (let i = 0; i < ALL_STATES.length; i++) {
      for (let j = i + 1; j < ALL_STATES.length; j++) {
        const d = distance(POSES[ALL_STATES[i]], POSES[ALL_STATES[j]]);
        expect(d, `${ALL_STATES[i]} vs ${ALL_STATES[j]}`).toBeGreaterThan(40);
      }
    }
  });
});

describe("poseForView", () => {
  it("follows the view state 1:1", () => {
    for (const state of ALL_STATES) {
      expect(poseForView({ state })).toBe(POSES[state]);
    }
  });

  it("shows the idle pose before any view arrives", () => {
    expect(poseForView(null)).toBe(POSES["idle"]);
  });

  it("keeps the thinking pose while an AskFollowUp question is spoken", () => {
    // The reducer keeps state `thinking` (with the question as caption) until
    // the follow-up listen opens; only then does the view become `listening`.
    const asking: ViewPayload = { state: "thinking", caption: "Which Notepad window?" };
    expect(poseForView(asking)).toBe(POSES["thinking"]);
    expect(poseForView({ state: "listening" })).toBe(POSES["listening"]);
  });

  it("ignores caption, confirm and reminder fields", () => {
    const view: ViewPayload = {
      state: "confirmation-needed",
      caption: "Close Notepad?",
      confirm: { id: 3, prompt: "Close Notepad?", confirmLabel: "Confirm", cancelLabel: "Cancel" },
    };
    expect(poseForView(view)).toBe(POSES["confirmation-needed"]);
  });

  it("makes room above the head for the raised pans", () => {
    const alert = POSES["reminder-alert"];
    expect(alert.headLift).toBeGreaterThanOrEqual(10); // head sits low in the canvas
    expect(alert.scale).toBeLessThanOrEqual(1.02);
    expect(alert.eyeOpen).toBe(1);
    expect(alert.eyeWidth).toBeGreaterThanOrEqual(1.25); // wide alarmed eyes
  });
});
