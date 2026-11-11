// @ts-expect-error node builtin, no @types/node in this project (same as vite.config.ts)
import { readdirSync, readFileSync } from "node:fs";
import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";
import type { ViewPayload } from "../events";
import { Notch } from "./Notch";
import { holdLook, LOOKS } from "./looks";
import { NOTCH_H, NOTCH_IN_MS, NOTCH_OUT_MS, NOTCH_W, REDUCED_FADE_MS } from "./motion";

const css: string = readFileSync(new URL("./notch.css", import.meta.url), "utf8");

const render = (view: ViewPayload | null, phase: "hidden" | "in" | "out" = "in") =>
  renderToStaticMarkup(createElement(Notch, { view, phase }));

// A payload from before the trim, still carrying text: the notch must draw none of it.
const reply = {
  state: "success",
  caption: "Opening Notepad",
  confirm: { id: 3, prompt: "Close Spotify?", confirmLabel: "Confirm", cancelLabel: "Cancel" },
  reminder: { id: 1, text: "Call mom" },
} as unknown as ViewPayload;

describe("Notch markup", () => {
  it("renders_one_waveform_canvas_in_118x30_shell", () => {
    const html = render({ state: "listening" });
    expect(html.match(/<canvas /g)).toHaveLength(1);
    expect(html).not.toContain('class="bar"');
    expect(html).toContain(`width:${NOTCH_W}px`);
    expect(html).toContain(`height:${NOTCH_H}px`);
    expect(NOTCH_W).toBe(118);
    expect(NOTCH_H).toBe(30);
  });

  it("status_region_has_state_name_and_no_reply_text", () => {
    const html = render(reply);
    expect(html).toMatch(/<span class="sr-only" role="status">Done<\/span>/);
    for (const text of ["Opening Notepad", "Close Spotify", "Call mom", "Confirm", "Cancel"]) {
      expect(html).not.toContain(text);
    }
    expect(render({ state: "listening" })).toContain(">Listening<");
    expect(render({ state: "success", speaking: true })).toContain(">Speaking<");
  });

  it("no_buttons_and_no_text_in_markup", () => {
    const html = render({ ...reply, state: "confirmation-needed" });
    expect(html).not.toMatch(/<(button|input|a|img|svg|p|h1|h2)\b/);
    // The only text node is the visually hidden state name.
    const text = html.replace(/<[^>]*>/g, "");
    expect(text).toBe("Confirmation needed");
  });

  it("marks the look on the shell and leaves the color to the canvas", () => {
    expect(render({ state: "thinking" })).toContain('data-look="thinking"');
    expect(render({ state: "error" })).toContain('data-look="error"');
    expect(render({ state: "error" })).not.toContain("--bar");
  });

  it("shows the exit and the hidden phases through a class", () => {
    expect(render({ state: "listening" }, "in")).toContain("notch in");
    expect(render({ state: "idle" }, "out")).toContain("notch out");
    expect(render(null, "hidden")).toContain("notch hidden");
  });

  it("an island that is shown before any view arrives looks like listening", () => {
    expect(render(null, "in")).toContain(">Listening<");
  });

  it("css_mirrors_motion_constants", () => {
    const ms = (name: string) => Number(new RegExp(`--${name}:\\s*(\\d+)ms`).exec(css)?.[1]);
    expect(ms("in-ms")).toBe(NOTCH_IN_MS);
    expect(ms("out-ms")).toBe(NOTCH_OUT_MS);
    expect(ms("fade-ms")).toBe(REDUCED_FADE_MS);
    expect(css).toContain("cubic-bezier(0.34, 1.56, 0.64, 1)");
    expect(css).toContain("left: calc(50% - 59px)");
    expect(NOTCH_W / 2).toBe(59);
  });
});

describe("Notch idle fallback", () => {
  it("holds the previous look while it slides out", () => {
    expect(holdLook(null, LOOKS.error)).toBe(LOOKS.error);
    expect(holdLook(LOOKS.thinking, LOOKS.error)).toBe(LOOKS.thinking);
    expect(holdLook(null, null)).toBe(LOOKS.listening);
  });

  it("an idle view sliding out keeps the label of the look it already has", () => {
    const html = render({ state: "idle" }, "out");
    // Nothing was shown before, so the fallback is listening, not an empty notch.
    expect(html).toContain(`data-look="listening"`);
    expect(html).toContain(">Listening<");
  });
});

describe("Notch never confirms by itself", () => {
  const walk = (dir: URL): string[] =>
    readdirSync(dir, { withFileTypes: true }).flatMap((e: { name: string; isDirectory(): boolean }) =>
      e.isDirectory() ? walk(new URL(`${e.name}/`, dir)) : [new URL(e.name, dir).href],
    );

  it("no UI source invokes report_confirm_buttons or answer_confirmation", () => {
    const files = walk(new URL("../", import.meta.url)).filter(
      (f) => /\.(ts|tsx)$/.test(f) && !/\.test\.(ts|tsx)$/.test(f),
    );
    expect(files.length).toBeGreaterThan(10);
    for (const f of files) {
      const text: string = readFileSync(new URL(f), "utf8");
      expect(text, f).not.toMatch(/report_confirm_buttons|answer_confirmation/);
    }
  });
});
