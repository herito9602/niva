import { describe, expect, it } from "vitest";

// The mascot is drawn procedurally: no module under src/mascot or src/state may
// reference an image or audio file (spec: no image assets).
const sources = import.meta.glob(["./*.{ts,tsx}", "../state/*.{ts,tsx}", "!./*.test.{ts,tsx}", "!../state/*.test.{ts,tsx}"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const ASSET = /["'`][^"'`\n]*\.(png|jpe?g|svg|gif|webp|mp3|wav)(\?[^"'`\n]*)?["'`]/i;

describe("mascot and state modules", () => {
  it("scans the expected source files", () => {
    const names = Object.keys(sources);
    expect(names.some((n) => n.endsWith("/robot.ts"))).toBe(true);
    expect(names.some((n) => n.endsWith("/MascotCanvas.tsx"))).toBe(true);
    expect(names.some((n) => n.endsWith("/viewStore.ts"))).toBe(true);
  });

  it("import no image or media assets", () => {
    for (const [name, text] of Object.entries(sources)) {
      expect(ASSET.test(text), `${name} references an asset file`).toBe(false);
    }
  });

  it("the asset pattern catches the usual import forms", () => {
    expect(ASSET.test('import logo from "./logo.png";')).toBe(true);
    expect(ASSET.test('const u = new URL("../a/b.svg", import.meta.url)')).toBe(true);
    expect(ASSET.test("import('./chime.wav')")).toBe(true);
    expect(ASSET.test('import { x } from "./geometry";')).toBe(false);
  });
});
