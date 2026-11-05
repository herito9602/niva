import { describe, expect, it } from "vitest";

// The island window ignores the mouse everywhere except over the confirmation
// buttons (host side: `click_through.rs` and `confirm_buttons.rs`). That is only
// correct while every clickable thing in the island is one of those buttons and
// the host is told where they are. This test makes adding any other control, or
// a button the host does not know about, a conscious decision instead of a
// silent dead click.
const sources = import.meta.glob(["./*.tsx", "../App.tsx", "../mascot/MascotCanvas.tsx", "!./preview.tsx"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

// Any interaction handler (onClick, onKeyDown, onPointerEnter, onMouseOver,
// onTouchStart, ...), an interactive role or focus order, or a native control.
const HANDLER =
  /\bon(Click|DoubleClick|ContextMenu|Wheel|Key\w*|Pointer\w*|Mouse\w*|Touch\w*|Drag\w*|Focus|Blur)\b|\brole\s*=\s*["']?button|\btabIndex\b|<(button|a|input|select|textarea)\b/;

/** The one file allowed to hold clickable controls: the two confirmation buttons. */
const BUTTONS_FILE = "/ConfirmButtons.tsx";

const named = (suffix: string) => {
  const entry = Object.entries(sources).find(([name]) => name.endsWith(suffix));
  expect(entry, `${suffix} is scanned`).toBeDefined();
  return entry![1];
};

describe("island sources", () => {
  it("scans the island, the mascot it renders and the buttons", () => {
    expect(Object.keys(sources).some((n) => n.endsWith("/Island.tsx"))).toBe(true);
    expect(Object.keys(sources).some((n) => n.endsWith("/MascotCanvas.tsx"))).toBe(true);
    expect(Object.keys(sources).some((n) => n.endsWith(BUTTONS_FILE))).toBe(true);
  });

  it("have no clickable control outside the confirmation buttons", () => {
    for (const [name, text] of Object.entries(sources)) {
      if (name.endsWith(BUTTONS_FILE)) continue;
      expect(HANDLER.test(text), `${name} adds a clickable control: add host hit-testing first`).toBe(false);
    }
  });

  it("the confirmation buttons are exactly two, and only react to a click", () => {
    const text = named(BUTTONS_FILE);
    expect(text.match(/<button\b/g)?.length).toBe(2);
    const handlers = [...text.matchAll(/\bon[A-Z]\w*\s*=/g)].map((m) => m[0].replace(/\s*=$/, ""));
    expect(handlers).toEqual(["onClick", "onClick"]);
    expect(/\b(tabIndex|role)\b/.test(text)).toBe(false);
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
