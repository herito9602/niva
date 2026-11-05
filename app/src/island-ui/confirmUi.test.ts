import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { CONFIRM_ARM_MS } from "./confirmCommands";
import { ARM_SLACK_MS, DIM_MS, buttonSize, createArmTimer, reportFor } from "./confirmUi";
import { BUTTON_HEIGHT, BUTTON_WIDTH, LINE_HEIGHT, computeLayout, confirmButtonRects } from "./layout";

const layout = computeLayout({ lineWidth: 90, blockHeight: LINE_HEIGHT, actions: true });

describe("what the island reports to the host", () => {
  it("reports nothing without a confirmation", () => {
    expect(reportFor({ confirmId: undefined, buttonsOn: false, layout, stageWidth: 640 })).toBeNull();
    expect(reportFor({ confirmId: undefined, buttonsOn: true, layout, stageWidth: 640 })).toBeNull();
  });

  it("reports no rectangles while the buttons are not shown, so the capture is released", () => {
    expect(reportFor({ confirmId: 7, buttonsOn: false, layout, stageWidth: 640 })).toEqual({ id: 7, areas: [] });
  });

  it("reports the two settled rectangles of the confirmation on screen", () => {
    const { cancel, confirm } = confirmButtonRects(layout, 640);
    expect(reportFor({ confirmId: 7, buttonsOn: true, layout, stageWidth: 640 })).toEqual({
      id: 7,
      areas: [cancel, confirm],
    });
  });

  it("reports exactly the size the buttons are drawn with", () => {
    const report = reportFor({ confirmId: 7, buttonsOn: true, layout, stageWidth: 640 });
    expect(report?.areas).toHaveLength(2);
    for (const box of report!.areas) {
      expect({ width: box.width, height: box.height }).toEqual(buttonSize);
    }
    expect(buttonSize).toEqual({ width: BUTTON_WIDTH, height: BUTTON_HEIGHT });
  });
});

describe("the arming timer of the buttons", () => {
  beforeEach(() => {
    vi.useFakeTimers();
  });
  afterEach(() => {
    vi.useRealTimers();
  });

  it("dims a little longer than the host's guard, so the host (which starts later) agrees", () => {
    expect(ARM_SLACK_MS).toBeGreaterThan(0);
    expect(DIM_MS).toBe(CONFIRM_ARM_MS + ARM_SLACK_MS);
  });

  it("arms once, when the dim period is over", () => {
    const armed = vi.fn();
    createArmTimer(armed);
    vi.advanceTimersByTime(DIM_MS - 1);
    expect(armed).not.toHaveBeenCalled();
    vi.advanceTimersByTime(1);
    expect(armed).toHaveBeenCalledTimes(1);
    vi.advanceTimersByTime(10_000);
    expect(armed).toHaveBeenCalledTimes(1);
  });

  it("never arms after it was cancelled (the buttons were replaced or removed)", () => {
    const armed = vi.fn();
    const cancel = createArmTimer(armed);
    vi.advanceTimersByTime(DIM_MS - 100);
    cancel();
    vi.advanceTimersByTime(10_000);
    expect(armed).not.toHaveBeenCalled();
  });
});
