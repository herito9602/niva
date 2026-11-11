// The owner's live complaint: "the wave doesn't move when Niva speaks". These
// tests cross the host boundary: a payload exactly as `niva://view` carries it
// goes through the view store and the look table down to the level that drives
// the waveform, with no real TTS level (slice 5) yet.

import { describe, expect, it } from "vitest";
import { VIEW_EVENT } from "../events";
import { bindHostEvents, createViewStore, type Listen } from "../state/viewStore";
import { createLevelStore } from "./levelStore";
import { LOOKS, viewToLook } from "./looks";
import { targetLevel } from "./wave";

type Handler = (event: { payload: unknown }) => void;

function host() {
  const store = createViewStore();
  let handler: Handler = () => {};
  const listen: Listen = (name, h) => {
    if (name === VIEW_EVENT) handler = h as Handler;
    return Promise.resolve(() => {});
  };
  bindHostEvents(store, listen);
  return { store, send: (wire: string) => handler({ payload: JSON.parse(wire) }) };
}

/** The wave level over three seconds of frames, with no TTS level in the store. */
function levelsOver3s(state: ReturnType<typeof host>["store"]) {
  const look = viewToLook(state.getState().view);
  if (!look) throw new Error("nothing to show");
  const store = createLevelStore();
  const out: number[] = [];
  for (let t = 0; t < 3000; t += 16) out.push(targetLevel(look, store.forLook(look), t, false));
  return { look, out };
}

describe("speaking from the host", () => {
  it("speaking_true_switches_to_the_teal_look_and_moves_the_wave", () => {
    const { store, send } = host();
    send('{"state":"success","speaking":false}');
    const quiet = levelsOver3s(store);
    expect(quiet.look).toBe(LOOKS.success);
    expect(Math.max(...quiet.out)).toBeLessThan(0.1); // the calm breathing line

    send('{"state":"success","speaking":true}');
    const talking = levelsOver3s(store);
    expect(talking.look).toBe(LOOKS.speaking);
    expect(talking.look.color).toBe("#5DCAA5");
    // The simulated voice rises and falls: it is not the flat line of the phase look.
    expect(Math.max(...talking.out)).toBeGreaterThan(0.3);
    expect(new Set(talking.out).size).toBeGreaterThan(20);
  });

  it("speaking_overrides_every_phase_color_and_the_phase_returns_when_it_ends", () => {
    const { store, send } = host();
    for (const state of ["success", "error", "thinking", "reminder-alert", "countdown", "confirmation-needed"]) {
      send(`{"state":"${state}","speaking":true}`);
      expect(viewToLook(store.getState().view), state).toBe(LOOKS.speaking);
      send(`{"state":"${state}","speaking":false}`);
      expect(viewToLook(store.getState().view), state).not.toBe(LOOKS.speaking);
    }
  });

  it("listening_stays_the_calm_line_even_if_a_flag_is_missing", () => {
    const { store, send } = host();
    send('{"state":"listening","speaking":false}');
    const listening = levelsOver3s(store);
    expect(listening.look).toBe(LOOKS.listening);
    expect(Math.max(...listening.out)).toBeLessThan(0.1);
  });
});
