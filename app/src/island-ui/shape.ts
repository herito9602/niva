// The island's outline as an SVG path: a pill that hangs from the top edge of
// the window (which is the top edge of the screen), its top corners flaring
// out into the edge with concave fillets and its bottom corners rounded.
//
// Pure geometry; edge.ts decides how the parameters move over time.

/** One frame of the island, measured from the top edge, centred on x = cx. */
export interface Shape {
  /** Half-width of the body. */
  bw: number;
  /** Height from the top edge to the bottom of the body. */
  h: number;
  /** Radius of the concave fillets that flare into the top edge. */
  F: number;
  /** Radius of the bottom corners. */
  r: number;
}

const KAPPA = 0.5523;
const fin = (v: number, fallback: number) => (Number.isFinite(v) ? v : fallback);
const clamp = (v: number, lo: number, hi: number) => Math.min(Math.max(v, lo), Math.max(lo, hi));
const num = (v: number) => String(+v.toFixed(2));

/**
 * The outline as an SVG path, left fillet first, ending on the right fillet,
 * both on y = 0. It is not closed: filling closes it along the top edge, and a
 * stroke leaves that edge alone. Out-of-range parameters are repaired.
 */
export function shapePath(s: Shape, cx: number): string {
  const bw = Math.max(1, fin(s.bw, 1));
  const h = Math.max(2, fin(s.h, 2));
  const F = clamp(fin(s.F, 1), 0.5, h / 2);
  const r = clamp(fin(s.r, 0), 0, Math.min(bw, h - F));

  const yR = h - r;
  const k = KAPPA;
  const left = cx - bw;
  const right = cx + bw;

  return [
    `M${num(left - F)} 0`,
    `C${num(left - F + k * F)} 0 ${num(left)} ${num(F - k * F)} ${num(left)} ${num(F)}`,
    `L${num(left)} ${num(yR)}`,
    `C${num(left)} ${num(yR + k * r)} ${num(left + r - k * r)} ${num(h)} ${num(left + r)} ${num(h)}`,
    `L${num(right - r)} ${num(h)}`,
    `C${num(right - r + k * r)} ${num(h)} ${num(right)} ${num(yR + k * r)} ${num(right)} ${num(yR)}`,
    `L${num(right)} ${num(F)}`,
    `C${num(right)} ${num(F - k * F)} ${num(right + F - k * F)} 0 ${num(right + F)} 0`,
  ].join("");
}
