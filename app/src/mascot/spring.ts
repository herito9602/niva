// Damped spring on one number. Pure: returns a new state, never mutates.

export interface SpringState {
  value: number;
  velocity: number;
}

export interface SpringConfig {
  stiffness: number;
  damping: number;
}

/** Longest frame we simulate in one call; a hidden tab can report seconds. */
const MAX_DT = 0.1;
/** Sub-step size. Semi-implicit Euler is stable while h * sqrt(stiffness) < 2. */
const SUBSTEP = 1 / 240;

/** Damping that gives no overshoot for a unit mass. */
export function criticalDamping(stiffness: number): number {
  return 2 * Math.sqrt(stiffness);
}

export function stepSpring(
  state: SpringState,
  target: number,
  config: SpringConfig,
  dt: number,
): SpringState {
  // A NaN/Infinity target would poison the state for good, so it is ignored.
  if (!(dt > 0) || !Number.isFinite(dt) || !Number.isFinite(target)) return state;
  const total = Math.min(dt, MAX_DT);
  const steps = Math.ceil(total / SUBSTEP);
  const h = total / steps;
  let { value, velocity } = state;
  for (let i = 0; i < steps; i++) {
    velocity += (config.stiffness * (target - value) - config.damping * velocity) * h;
    value += velocity * h;
  }
  return { value, velocity };
}
