//! Pure helpers for the light timeline: an ordered list of trimmed segments
//! (spec §4.3). All operations are immutable so React state updates stay simple.

import type { Segment, TrackConfig } from "./api";

/** One clip in the sequence, as tracked by the UI. */
export interface SequenceSegment {
  id: number;
  path: string;
  displayName: string;
  inSecs: number;
  outSecs: number;
  /**
   * Audio track configs snapshotted when the clip was added — the export must mix
   * exactly what the preview played at that moment, not whatever the sliders say
   * later (or only the file's first track, as the pre-fix export did).
   */
  tracks: TrackConfig[];
}

/** Move the segment at `index` by `dir` (-1 earlier, +1 later). Edges are no-ops. */
export function moveSegment(
  list: SequenceSegment[],
  index: number,
  dir: -1 | 1,
): SequenceSegment[] {
  const target = index + dir;
  if (index < 0 || index >= list.length || target < 0 || target >= list.length) {
    return list;
  }
  const next = list.slice();
  [next[index], next[target]] = [next[target], next[index]];
  return next;
}

/** Remove the segment with the given id. */
export function removeSegment(list: SequenceSegment[], id: number): SequenceSegment[] {
  return list.filter(s => s.id !== id);
}

/** Total playing time of the sequence (sum of each trimmed span). */
export function sequenceDuration(list: SequenceSegment[]): number {
  return list.reduce((sum, s) => sum + (s.outSecs - s.inSecs), 0);
}

/** Map UI segments to the Rust `Segment` shape the export command expects. */
export function toBackendSegments(list: SequenceSegment[]): Segment[] {
  return list.map(s => ({
    input: s.path,
    inSecs: s.inSecs,
    outSecs: s.outSecs,
    tracks: s.tracks.map(t => ({ ...t })),
  }));
}
