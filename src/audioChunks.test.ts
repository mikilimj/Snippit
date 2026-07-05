import { describe, expect, it } from "vitest";
import {
  CHUNK_SECONDS,
  chunkIndexAt,
  chunkSchedule,
  chunkWindow,
  engineMediaTime,
  needsResync,
} from "./audioChunks";

describe("chunkIndexAt", () => {
  it("maps media time onto the chunk grid", () => {
    expect(chunkIndexAt(0)).toBe(0);
    expect(chunkIndexAt(19.99)).toBe(0);
    expect(chunkIndexAt(20)).toBe(1);
    expect(chunkIndexAt(45, 20)).toBe(2);
  });

  it("clamps negative times to the first chunk", () => {
    expect(chunkIndexAt(-3)).toBe(0);
  });
});

describe("chunkWindow", () => {
  it("holds only the current chunk while far from the boundary", () => {
    expect(chunkWindow(2, 100, 20, 5)).toEqual([0]);
  });

  it("adds the next chunk within the lookahead of the boundary", () => {
    expect(chunkWindow(15, 100, 20, 5)).toEqual([0, 1]);
    expect(chunkWindow(16.5, 100, 20, 5)).toEqual([0, 1]);
  });

  it("never schedules past the end of the media", () => {
    // 50s media => chunks 0,1,2; playhead near the end stays on the last chunk
    expect(chunkWindow(48, 50, 20, 5)).toEqual([2]);
    // playhead beyond the media clamps to the last chunk
    expect(chunkWindow(60, 50, 20, 5)).toEqual([2]);
  });

  it("is empty for empty media", () => {
    expect(chunkWindow(0, 0, 20, 5)).toEqual([]);
  });
});

describe("chunkSchedule", () => {
  // Anchor: media 30s pinned to ctx time 100.
  const media = 30;
  const ctx = 100;

  it("schedules a future chunk at its exact context time", () => {
    // chunk 2 starts at media 40 => ctx 110
    expect(chunkSchedule(2, media, ctx, 100, 300)).toEqual({
      when: 110,
      offset: 0,
      duration: CHUNK_SECONDS,
    });
  });

  it("starts the current chunk immediately with an offset into the buffer", () => {
    // chunk 1 covers media 20-40; at anchor the playhead is 10s into it
    expect(chunkSchedule(1, media, ctx, 100, 300)).toEqual({
      when: 100,
      offset: 10,
      duration: 10,
    });
  });

  it("returns null for chunks entirely in the past", () => {
    expect(chunkSchedule(0, media, ctx, 100, 300)).toBeNull();
  });

  it("shortens the final chunk to the real media end", () => {
    // 50s media: chunk 2 covers 40-50 => 10s long
    const s = chunkSchedule(2, media, ctx, 100, 50);
    expect(s).toEqual({ when: 110, offset: 0, duration: 10 });
  });

  it("returns null for chunks past the end of the media", () => {
    expect(chunkSchedule(3, media, ctx, 100, 50)).toBeNull();
  });

  it("accounts for context time already advanced past the anchor", () => {
    // 5s after anchoring, chunk 2 (media 40 => ctx 110) is still 5s away
    const s = chunkSchedule(2, media, ctx, 105, 300);
    expect(s).toEqual({ when: 110, offset: 0, duration: CHUNK_SECONDS });
    // 12s after anchoring the chunk started 2s ago: play the remainder now
    const late = chunkSchedule(2, media, ctx, 112, 300);
    expect(late).toEqual({ when: 112, offset: 2, duration: CHUNK_SECONDS - 2 });
  });
});

describe("engineMediaTime / needsResync", () => {
  it("derives the media clock from the context clock", () => {
    expect(engineMediaTime(105, 30, 100)).toBe(35);
  });

  it("tolerates small drift and flags large drift", () => {
    expect(needsResync(35.2, 35.0)).toBe(false); // within tolerance
    expect(needsResync(35.5, 35.0)).toBe(true); // past tolerance
    expect(needsResync(34.5, 35.0)).toBe(true); // drift is symmetric
  });
});
