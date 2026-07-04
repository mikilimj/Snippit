import { describe, it, expect } from "vitest";
import { audioSyncCommand, previewPath, shouldTryProxy } from "./preview";

describe("previewPath", () => {
  it("loads the original when no proxy exists yet", () => {
    expect(previewPath("/v/a.mkv", null)).toBe("/v/a.mkv");
  });

  it("loads the proxy once one has been generated", () => {
    expect(previewPath("/v/a.mkv", "/cache/proxies/abc.mp4")).toBe("/cache/proxies/abc.mp4");
  });
});

describe("shouldTryProxy", () => {
  it("builds a proxy after the original fails to play", () => {
    // We were still on the original (no proxy yet) — a fallback is worth trying.
    expect(shouldTryProxy(null)).toBe(true);
  });

  it("does not re-proxy when the proxy itself fails (avoids an infinite loop)", () => {
    expect(shouldTryProxy("/cache/proxies/abc.mp4")).toBe(false);
  });
});

describe("audioSyncCommand", () => {
  it("does nothing while the video is paused", () => {
    expect(audioSyncCommand(10, true, 3, true)).toEqual({ seek: false, play: false });
    expect(audioSyncCommand(10, true, 3, false)).toEqual({ seek: false, play: false });
  });

  it("does nothing when audio tracks are in sync and playing", () => {
    expect(audioSyncCommand(10, false, 10.1, false)).toEqual({ seek: false, play: false });
  });

  it("restarts a stalled audio track (paused while video plays), re-synced", () => {
    // The stall recovery path: a track that buffer-underran and never resumed.
    expect(audioSyncCommand(10, false, 7, true)).toEqual({ seek: true, play: true });
  });

  it("snaps a drifted track back to the video clock", () => {
    expect(audioSyncCommand(10, false, 9.5, false)).toEqual({ seek: true, play: false });
    expect(audioSyncCommand(10, false, 10.5, false)).toEqual({ seek: true, play: false });
  });

  it("leaves drift under the tolerance alone to avoid seek stutter", () => {
    expect(audioSyncCommand(10, false, 9.8, false)).toEqual({ seek: false, play: false });
  });
});
