import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

// No jsdom or testing-library in this project, so React is replaced by a tiny
// hook stub: `useRef` returns a plain box and `useEffect` records its callback.
// The component function is then called directly and its effect run by hand.
const hooks = vi.hoisted(() => ({
  refs: [] as Array<{ current: unknown }>,
  effects: [] as Array<() => void | (() => void)>,
  refIndex: 0,
}));

vi.mock("react", () => ({
  useRef: (initial: unknown) => {
    const i = hooks.refIndex++;
    hooks.refs[i] ??= { current: initial };
    return hooks.refs[i];
  },
  useEffect: (fn: () => void | (() => void)) => {
    hooks.effects.push(fn);
  },
}));

import { MascotCanvas } from "./MascotCanvas";

function stubWindow() {
  const query = {
    matches: false,
    addEventListener: vi.fn(),
    removeEventListener: vi.fn(),
  };
  const observer = { observe: vi.fn(), disconnect: vi.fn() };
  const ctx = new Proxy({ setTransform: vi.fn() } as Record<string, unknown>, {
    get: (obj, key: string) => obj[key] ?? vi.fn(() => ({ addColorStop: vi.fn() })),
    set: () => true,
  });
  const canvas = {
    width: 0,
    height: 0,
    getContext: () => ctx,
    getBoundingClientRect: () => ({ left: 0, top: 0, width: 96, height: 96 }),
  };
  vi.stubGlobal("window", { devicePixelRatio: 2, matchMedia: vi.fn(() => query) });
  vi.stubGlobal(
    "ResizeObserver",
    class {
      observe = observer.observe;
      disconnect = observer.disconnect;
    },
  );
  const raf = vi.fn(() => 42);
  const caf = vi.fn();
  vi.stubGlobal("requestAnimationFrame", raf);
  vi.stubGlobal("cancelAnimationFrame", caf);
  return { query, observer, canvas, raf, caf };
}

function mount(canvas: unknown, active: boolean) {
  hooks.refs.length = 0;
  hooks.effects.length = 0;
  hooks.refIndex = 0;
  MascotCanvas({ view: { state: "idle" }, getCursor: () => null, active });
  hooks.refs[0].current = canvas; // the <canvas> ref, attached after render
  return hooks.effects[0]();
}

describe("MascotCanvas lifecycle", () => {
  beforeEach(() => vi.restoreAllMocks());
  afterEach(() => vi.unstubAllGlobals());

  it("releases rAF, the ResizeObserver and the matchMedia listeners on unmount", () => {
    const env = stubWindow();
    const cleanup = mount(env.canvas, true) as () => void;
    expect(typeof cleanup).toBe("function");
    expect(env.raf).toHaveBeenCalled();
    expect(env.observer.observe).toHaveBeenCalledWith(env.canvas);
    expect(env.query.addEventListener).toHaveBeenCalledWith("change", expect.any(Function));

    cleanup();
    expect(env.caf).toHaveBeenCalledWith(42);
    expect(env.observer.disconnect).toHaveBeenCalledTimes(1);
    const added = env.query.addEventListener.mock.calls.map((c) => c[1]);
    const removed = env.query.removeEventListener.mock.calls.map((c) => c[1]);
    expect(removed).toContain(added[added.length - 1]);
  });

  it("starts nothing while inactive", () => {
    const env = stubWindow();
    expect(mount(env.canvas, false)).toBeUndefined();
    expect(env.raf).not.toHaveBeenCalled();
    expect(env.observer.observe).not.toHaveBeenCalled();
  });
});
