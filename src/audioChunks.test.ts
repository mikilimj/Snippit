import { describe, expect, it } from "vitest";
import {
  CACHED_CHUNKS_PER_TRACK,
  CHUNK_SECONDS,
  chunkIndexAt,
  chunkSchedule,
  chunkWindow,
  engineMediaTime,
  needsResync,
} from "./audioChunks";

describe("chunkIndexAt", () => {
  it("maps media time onto the default (CHUNK_SECONDS) grid", () => {
    expect(chunkIndexAt(0)).toBe(0);
    expect(chunkIndexAt(9.99)).toBe(0);
    expect(chunkIndexAt(10)).toBe(1);
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
  // Anchor: media 35s pinned to ctx time 100, so 10s chunk 3 covers media 30-40.
  const media = 35;
  const ctx = 100;

  it("schedules a future chunk at its exact context time", () => {
    // chunk 4 starts at media 40 => ctx 105
    expect(chunkSchedule(4, media, ctx, 100, 300)).toEqual({
      when: 105,
      offset: 0,
      duration: CHUNK_SECONDS,
    });
  });

  it("starts the current chunk immediately with an offset into the buffer", () => {
    // chunk 3 covers media 30-40; the anchor sits 5s into it
    expect(chunkSchedule(3, media, ctx, 100, 300)).toEqual({
      when: 100,
      offset: 5,
      duration: 5,
    });
  });

  it("returns null for chunks entirely in the past", () => {
    expect(chunkSchedule(2, media, ctx, 100, 300)).toBeNull();
  });

  it("shortens the final chunk to the real media end", () => {
    // 45s media: chunk 4 covers 40-45 => 5s long
    const s = chunkSchedule(4, media, ctx, 100, 45);
    expect(s).toEqual({ when: 105, offset: 0, duration: 5 });
  });

  it("returns null for chunks past the end of the media", () => {
    expect(chunkSchedule(5, media, ctx, 100, 45)).toBeNull();
  });

  it("accounts for context time already advanced past the anchor", () => {
    // 2s after anchoring, chunk 4 (media 40 => ctx 105) is still 3s away
    const s = chunkSchedule(4, media, ctx, 102, 300);
    expect(s).toEqual({ when: 105, offset: 0, duration: CHUNK_SECONDS });
    // 12s after anchoring the chunk started 7s ago: play the remainder now
    const late = chunkSchedule(4, media, ctx, 112, 300);
    expect(late).toEqual({ when: 112, offset: 7, duration: CHUNK_SECONDS - 7 });
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

// The engine's RAM cost is these two constants multiplied out: every chunk is
// decoded to a float32 AudioBuffer (48kHz stereo here — the rate the extracted
// preview tracks carry), and every audible track keeps CACHED_CHUNKS_PER_TRACK
// of them. These guards exist so a future "let's prefetch more" tweak has to
// argue with the memory budget, not just with smoothness.
describe("decoded-PCM budget", () => {
  const FLOAT32_STEREO_48K_BYTES_PER_SECOND = 48_000 * 2 * 4;

  it("keeps one chunk small enough to fetch, decode and drop cheaply", () => {
    const chunkBytes = CHUNK_SECONDS * FLOAT32_STEREO_48K_BYTES_PER_SECOND;
    expect(chunkBytes).toBeLessThanOrEqual(4 * 1024 * 1024);
  });

  it("bounds one track's resident PCM to under 8MB", () => {
    const trackBytes =
      CACHED_CHUNKS_PER_TRACK * CHUNK_SECONDS * FLOAT32_STEREO_48K_BYTES_PER_SECOND;
    expect(trackBytes).toBeLessThanOrEqual(8 * 1024 * 1024);
  });
});
