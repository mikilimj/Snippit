export function clamp(value: number, min: number, max: number): number {
  return Math.min(max, Math.max(min, value));
}

// Parses a raw localStorage value into a clamped size. Falls back to a
// (clamped) default when the value is missing, non-numeric, or NaN.
export function parseStoredSize(
  raw: string | null,
  fallback: number,
  min: number,
  max: number
): number {
  const parsed = raw === null ? NaN : Number(raw);
  if (Number.isNaN(parsed)) return clamp(fallback, min, max);
  return clamp(parsed, min, max);
}
