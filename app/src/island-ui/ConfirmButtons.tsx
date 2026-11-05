import { useEffect, useState } from "react";
import type { ConfirmView } from "../events";
import { buttonSize, createArmTimer } from "./confirmUi";
import { ACTIONS_GAP, BUTTON_GAP, BUTTON_HEIGHT } from "./layout";

interface Props {
  confirm: ConfirmView;
  /** `accept` is Confirm, otherwise Cancel. */
  onAnswer: (id: number, accept: boolean) => void;
}

/**
 * The two answers to the open confirmation. The labels and the id come from the
 * view (the core), never from here. Confirm stays inactive, and both buttons
 * dimmed, for the first half second (and a little more, see `DIM_MS`) after they appear, so a click meant for
 * another app cannot land on it as the island drops in; Cancel always works.
 * The host ignores an early Confirm too, whatever this page does. Sizes come
 * from the layout constants, which is also where the click-through rectangles
 * are computed (`confirmButtonRects`). The parent mounts one per confirmation
 * (keyed by its id) so the half second restarts with every new question.
 */
export function ConfirmButtons({ confirm, onAnswer }: Props) {
  const [armed, setArmed] = useState(false);
  useEffect(() => createArmTimer(() => setArmed(true)), []);

  const size = buttonSize;
  return (
    <div className={`actions${armed ? "" : " arming"}`} style={{ marginTop: ACTIONS_GAP, gap: BUTTON_GAP, height: BUTTON_HEIGHT }}>
      <button type="button" className="btn cancel" style={size} onClick={() => onAnswer(confirm.id, false)}>
        {confirm.cancelLabel}
      </button>
      <button type="button" className="btn accept" style={size} disabled={!armed} onClick={() => onAnswer(confirm.id, true)}>
        {confirm.confirmLabel}
      </button>
    </div>
  );
}
