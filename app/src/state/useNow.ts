import { useEffect, useState } from "react";

/**
 * The current time in ms, refreshed every `everyMs` while `active` and not at
 * all otherwise (nothing ticks at rest). Used by the sleep countdown, whose
 * number is computed from a deadline in the view.
 */
export function useNow(active: boolean, everyMs = 200): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    if (!active) return;
    setNow(Date.now());
    const id = window.setInterval(() => setNow(Date.now()), everyMs);
    return () => window.clearInterval(id);
  }, [active, everyMs]);
  return now;
}
