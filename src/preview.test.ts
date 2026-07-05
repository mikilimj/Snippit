import { describe, it, expect } from "vitest";
import { previewPath, shouldTryProxy } from "./preview";

describe("previewPath", () => {
  it("loads the original when no proxy exists yet", () => {
    expect(previewPath("/v/a.mkv", null)).toBe("/v/a.mkv");
  });

  it("loads the proxy once one has been generated", () => {
    expect(previewPath("/v/a.mkv", "/cache/proxies/abc.mp4")).toBe("/cache/proxies/abc.mp4");
  });

  it("prefers the video-only remux over everything once it exists", () => {
    // The remux is derived from whichever source was active, so it wins outright.
    expect(previewPath("/v/a.mkv", null, "/cache/proxies/abc-vo.mkv")).toBe(
      "/cache/proxies/abc-vo.mkv"
    );
    expect(previewPath("/v/a.mkv", "/cache/proxies/abc.mp4", "/cache/proxies/def-vo.mp4")).toBe(
      "/cache/proxies/def-vo.mp4"
    );
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
