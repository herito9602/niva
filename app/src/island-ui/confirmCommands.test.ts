import { describe, expect, it, vi } from "vitest";
import { ANSWER_COMMAND, CONFIRM_ARM_MS, REPORT_COMMAND, createConfirmActions } from "./confirmCommands";

const BOXES = [
  { x: 300, y: 90, width: 84, height: 32 },
  { x: 392, y: 90, width: 84, height: 32 },
];

describe("the island's two commands", () => {
  it("are the ones its capability grants, and the dim period is half a second", () => {
    expect(ANSWER_COMMAND).toBe("answer_confirmation");
    expect(REPORT_COMMAND).toBe("report_confirm_buttons");
    expect(CONFIRM_ARM_MS).toBe(500);
  });
});

describe("answering", () => {
  it("sends only the confirmation id and the choice", async () => {
    const invoke = vi.fn().mockResolvedValue(undefined);
    const { answer } = createConfirmActions(invoke);
    expect(await answer(7, true)).toBe("sent");
    expect(invoke).toHaveBeenCalledTimes(1);
    expect(invoke).toHaveBeenCalledWith(ANSWER_COMMAND, { id: 7, accept: true });
    expect(Object.keys(invoke.mock.calls[0][1] as object).sort()).toEqual(["accept", "id"]);
  });

  it("answers a confirmation once: a second click on it, either button, is dropped", async () => {
    const invoke = vi.fn().mockResolvedValue(undefined);
    const { answer } = createConfirmActions(invoke);
    expect(await answer(7, true)).toBe("sent");
    expect(await answer(7, true)).toBe("repeated");
    expect(await answer(7, false)).toBe("repeated");
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("drops a click that arrives while the first is still being handed over", async () => {
    let finish: () => void = () => {};
    const invoke = vi.fn().mockReturnValue(new Promise<void>((resolve) => (finish = resolve)));
    const { answer } = createConfirmActions(invoke);
    const first = answer(7, true);
    expect(await answer(7, true)).toBe("repeated");
    finish();
    expect(await first).toBe("sent");
    expect(invoke).toHaveBeenCalledTimes(1);
  });

  it("a new confirmation can be answered", async () => {
    const invoke = vi.fn().mockResolvedValue(undefined);
    const { answer } = createConfirmActions(invoke);
    await answer(7, true);
    expect(await answer(8, false)).toBe("sent");
    expect(invoke).toHaveBeenLastCalledWith(ANSWER_COMMAND, { id: 8, accept: false });
  });

  it("lets a failed hand-over be tried again", async () => {
    const error = vi.spyOn(console, "error").mockImplementation(() => {});
    const invoke = vi.fn().mockRejectedValueOnce(new Error("ipc")).mockResolvedValue(undefined);
    const { answer } = createConfirmActions(invoke);
    expect(await answer(7, true)).toBe("failed");
    expect(await answer(7, true)).toBe("sent");
    expect(error).toHaveBeenCalled();
    error.mockRestore();
  });
});

describe("reporting where the buttons are", () => {
  it("sends the id and the rectangles", async () => {
    const invoke = vi.fn().mockResolvedValue(undefined);
    const { report } = createConfirmActions(invoke);
    await report(7, BOXES);
    expect(invoke).toHaveBeenCalledWith(REPORT_COMMAND, { id: 7, areas: BOXES });
  });

  it("does not repeat an identical report", async () => {
    const invoke = vi.fn().mockResolvedValue(undefined);
    const { report } = createConfirmActions(invoke);
    await report(7, BOXES);
    await report(7, BOXES.map((b) => ({ ...b })));
    expect(invoke).toHaveBeenCalledTimes(1);
    await report(7, []);
    await report(7, BOXES);
    expect(invoke).toHaveBeenCalledTimes(3);
  });

  it("retries a report that failed, and never throws", async () => {
    const warn = vi.spyOn(console, "warn").mockImplementation(() => {});
    const invoke = vi.fn().mockRejectedValueOnce(new Error("ipc")).mockResolvedValue(undefined);
    const { report } = createConfirmActions(invoke);
    await expect(report(7, BOXES)).resolves.toBeUndefined();
    await report(7, BOXES);
    expect(invoke).toHaveBeenCalledTimes(2);
    warn.mockRestore();
  });
});

describe("forgetting what was reported", () => {
  it("sends an identical report again after forget (the host may have lost it)", async () => {
    const invoke = vi.fn().mockResolvedValue(undefined);
    const { report, forget } = createConfirmActions(invoke);
    await report(7, BOXES);
    await report(7, BOXES);
    expect(invoke).toHaveBeenCalledTimes(1);
    forget();
    await report(7, BOXES);
    expect(invoke).toHaveBeenCalledTimes(2);
  });
});
