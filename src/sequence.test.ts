import { describe, it, expect } from "vitest";
import {
  moveSegment,
  removeSegment,
  sequenceDuration,
  toBackendSegments,
  type SequenceSegment,
} from "./sequence";

function s(id: number, inSecs: number, outSecs: number): SequenceSegment {
  return { id, path: `/v/${id}.mp4`, displayName: `${id}`, inSecs, outSecs, tracks: [] };
}

describe("moveSegment", () => {
  it("moves a segment later in the order", () => {
    const list = [s(1, 0, 1), s(2, 0, 1), s(3, 0, 1)];
    expect(moveSegment(list, 0, 1).map(x => x.id)).toEqual([2, 1, 3]);
  });

  it("moves a segment earlier in the order", () => {
    const list = [s(1, 0, 1), s(2, 0, 1), s(3, 0, 1)];
    expect(moveSegment(list, 2, -1).map(x => x.id)).toEqual([1, 3, 2]);
  });

  it("is a no-op at the edges", () => {
    const list = [s(1, 0, 1), s(2, 0, 1)];
    expect(moveSegment(list, 0, -1).map(x => x.id)).toEqual([1, 2]);
    expect(moveSegment(list, 1, 1).map(x => x.id)).toEqual([1, 2]);
  });
});

describe("removeSegment", () => {
  it("drops the segment with the given id", () => {
    const list = [s(1, 0, 1), s(2, 0, 1), s(3, 0, 1)];
    expect(removeSegment(list, 2).map(x => x.id)).toEqual([1, 3]);
  });
});

describe("sequenceDuration", () => {
  it("sums each segment's trimmed span", () => {
    const list = [s(1, 0, 2.5), s(2, 1, 4)]; // 2.5 + 3 = 5.5
    expect(sequenceDuration(list)).toBeCloseTo(5.5, 6);
  });

  it("is zero for an empty sequence", () => {
    expect(sequenceDuration([])).toBe(0);
  });
});

describe("toBackendSegments", () => {
  it("maps UI segments to the Rust Segment shape (path -> input)", () => {
    const list = [s(1, 0.5, 3.5)];
    expect(toBackendSegments(list)).toEqual([
      { input: "/v/1.mp4", inSecs: 0.5, outSecs: 3.5, tracks: [] },
    ]);
  });

  it("carries the snapshotted track configs through as copies", () => {
    const seg: SequenceSegment = {
      ...s(1, 0, 2),
      tracks: [
        { streamIndex: 1, enabled: true, volume: 1 },
        { streamIndex: 2, enabled: false, volume: 0.5 },
      ],
    };
    const [backend] = toBackendSegments([seg]);
    expect(backend.tracks).toEqual(seg.tracks);
    expect(backend.tracks[0]).not.toBe(seg.tracks[0]); // defensive copy
  });
});
