// Timecode formatting shared by the transport bar and the trim slider. Tenths of
// a second are enough to line a playhead up with a cut, and the short form keeps
// the readouts stable in width (tabular numerals).
export function formatTimecode(secs: number): string {
  const total = Number.isFinite(secs) && secs > 0 ? secs : 0;
  const m = Math.floor(total / 60);
  const s = Math.floor(total % 60);
  const d = Math.floor((total % 1) * 10);
  return `${m}:${s.toString().padStart(2, "0")}.${d}`;
}
