import { describe, expect, it } from "vitest";
import { EDGE_IN_MS, EDGE_OUT_MS, SWAP_FADE_MS, planTextChange } from "./choreography";
// @ts-expect-error node builtin, no @types/node in this project (same as vite.config.ts)
import { readFileSync } from "node:fs";

const css: string = readFileSync(new URL("../styles.css", import.meta.url), "utf8");

describe("durations", () => {
  it("are the ones the owner chose (appear 900 ms, exit 380 ms)", () => {
    expect(EDGE_IN_MS).toBe(900);
    expect(EDGE_OUT_MS).toBe(380);
  });
});

describe("styles.css stays in sync with the constants", () => {
  it("declares the same durations", () => {
    expect(css).toContain(`--in-ms: ${EDGE_IN_MS}ms`);
    expect(css).toContain(`--out-ms: ${EDGE_OUT_MS}ms`);
    expect(css).toContain(`--swap-ms: ${SWAP_FADE_MS}ms`);
  });
});

describe("planTextChange", () => {
  const base = { shownText: "Listening…", target: "Listening…", phase: "in", prevPhase: "in" } as const;

  it("keeps the text when nothing changed", () => {
    expect(planTextChange(base)).toBe("keep");
  });

  it("cross-fades when the message changes while the island is shown", () => {
    expect(planTextChange({ ...base, target: "Done" })).toBe("crossfade");
  });

  it("swaps at once when the island is appearing", () => {
    expect(planTextChange({ ...base, target: "Done", prevPhase: "hidden" })).toBe("swap-now");
    expect(planTextChange({ ...base, target: "Done", prevPhase: "out" })).toBe("swap-now");
    expect(planTextChange({ ...base, target: "Done", phase: "hidden", prevPhase: "hidden" })).toBe("swap-now");
  });

  it("freezes the text while the island leaves, so the idle label never flashes", () => {
    expect(planTextChange({ ...base, target: "Niva", phase: "out" })).toBe("keep");
  });
});
