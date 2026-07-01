const EPS = 0.01;

// Clamp a new IN point: never below 0, and strictly left of OUT (if OUT set).
export function setIn(current: number, out: number | null): number {
  const lowerBounded = Math.max(0, current);
  if (out !== null) return Math.min(lowerBounded, out - EPS);
  return lowerBounded;
}

// Clamp a new OUT point: never above duration, and strictly right of IN (if IN set).
export function setOut(current: number, inp: number | null, duration: number): number {
  const upperBounded = Math.min(duration, current);
  if (inp !== null) return Math.max(upperBounded, inp + EPS);
  return upperBounded;
}
