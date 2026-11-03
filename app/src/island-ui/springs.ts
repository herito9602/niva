// Closed-form damped spring from rest to 1, as a pure function of time. Used
// to drive the island's size and position from a rAF loop and from tests, with
// no integration state (so any moment can be sampled, or frozen for a
// screenshot).
//
//   k(t) = 1 - e^(-zeta w t) (cos(wd t) + (zeta w / wd) sin(wd t)),  wd = w sqrt(1 - zeta^2)
//
// Overshoot of the first peak is exp(-pi zeta / sqrt(1 - zeta^2)): zeta 0.7 is
// 4.6 %, 0.8 is 1.5 %. Critical damping (zeta >= 1) never overshoots.

/** Progress of a spring `tMs` after it starts, from 0 (at rest) to 1 (arrived). */
export function springK(tMs: number, omega: number, zeta: number): number {
  if (!(tMs > 0) || !Number.isFinite(tMs)) return 0;
  const t = tMs / 1000;
  if (zeta >= 1) return 1 - (1 + omega * t) * Math.exp(-omega * t);
  const wd = omega * Math.sqrt(1 - zeta * zeta);
  const decay = Math.exp(-zeta * omega * t);
  return 1 - decay * (Math.cos(wd * t) + ((zeta * omega) / wd) * Math.sin(wd * t));
}

/** A time after which `|springK - 1| <= eps` for ever (an upper bound, in ms). */
export function settleMs(omega: number, zeta: number, eps: number): number {
  if (zeta >= 1) {
    for (let ms = 0; ms < 20000; ms += 1) {
      const t = ms / 1000;
      if ((1 + omega * t) * Math.exp(-omega * t) <= eps) return ms;
    }
    return 20000;
  }
  const envelope = eps * Math.sqrt(1 - zeta * zeta);
  return Math.ceil((1000 * Math.log(1 / envelope)) / (zeta * omega));
}
