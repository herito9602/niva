import { describe, expect, it } from "vitest";

// The island window ignores the mouse entirely (host side, `click_through.rs`),
// which is only correct while nothing in the island is clickable. PR 2d's
// confirm buttons must come with host hit-testing; this test makes adding a
// handler without that a conscious decision instead of a silent dead button.
const sources = import.meta.glob(["./*.tsx", "../App.tsx", "../mascot/MascotCanvas.tsx", "!./preview.tsx"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

// Any interaction handler (onClick, onKeyDown, onPointerEnter, onMouseOver,
// onTouchStart, ...), an interactive role or focus order, or a native control.
const HANDLER =
  /\bon(Click|DoubleClick|ContextMenu|Wheel|Key\w*|Pointer\w*|Mouse\w*|Touch\w*|Drag\w*|Focus|Blur)\b|\brole\s*=\s*["']?button|\btabIndex\b|<(button|a|input|select|textarea)\b/;

describe("island sources", () => {
  it("scans the island component and the mascot it renders", () => {
    expect(Object.keys(sources).some((n) => n.endsWith("/Island.tsx"))).toBe(true);
    expect(Object.keys(sources).some((n) => n.endsWith("/MascotCanvas.tsx"))).toBe(true);
  });

  it("have no clickable control, so the window can ignore the mouse", () => {
    for (const [name, text] of Object.entries(sources)) {
      expect(HANDLER.test(text), `${name} adds a clickable control: add host hit-testing first`).toBe(false);
    }
  });

  it("the pattern catches the usual forms", () => {
    expect(HANDLER.test("<div onClick={go} />")).toBe(true);
    expect(HANDLER.test("<button>Confirm</button>")).toBe(true);
    for (const form of [
      "<div onKeyDown={f} />",
      "<div onPointerEnter={f} />",
      "<div onPointerMove={f} />",
      "<div onMouseOver={f} />",
      "<div onTouchStart={f} />",
      '<div role="button" />',
      "<div tabIndex={0} />",
      "<textarea />",
    ]) {
      expect(HANDLER.test(form), form).toBe(true);
    }
  });

  it("does not flag plain markup", () => {
    expect(HANDLER.test('<div className="island" role="presentation" />')).toBe(false);
    expect(HANDLER.test("const online = once(onceDone);")).toBe(false);
  });
});
