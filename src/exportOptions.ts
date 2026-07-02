/** Discord-friendly target-size presets, in MB (design spec §4.5). */
export const TARGET_SIZE_PRESETS = [8, 10, 25, 50, 100] as const;

export type SizeSelection = "original" | "custom" | `${(typeof TARGET_SIZE_PRESETS)[number]}`;

/**
 * Map the export dialog's size selection to a target in MB.
 * `null` means "no size target" (plain trim export).
 * A custom selection with an unusable value is also `null` — callers must
 * check `isCustomSelectionValid` before treating that as "original quality".
 */
export function resolveTargetMb(selection: string, customMb: string): number | null {
  if (selection === "original") return null;
  const raw = selection === "custom" ? customMb : selection;
  const n = parseFloat(raw);
  return Number.isFinite(n) && n > 0 ? n : null;
}

export function isCustomSelectionValid(selection: string, customMb: string): boolean {
  if (selection !== "custom") return true;
  const n = parseFloat(customMb);
  return Number.isFinite(n) && n > 0;
}
