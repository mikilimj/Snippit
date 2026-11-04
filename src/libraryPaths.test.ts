import { describe, expect, it } from "vitest";
import { canRemoveIndexOnly, clipRelName, isInClipsFolder } from "./libraryPaths";

const ROOT = "/home/me/Snippit";

describe("clipRelName", () => {
  it("strips the Clips/ prefix", () => {
    expect(clipRelName(ROOT, `${ROOT}/Clips/clip.mp4`)).toBe("clip.mp4");
    expect(clipRelName(ROOT, `${ROOT}/Clips/2026/07/clip.mp4`)).toBe("2026/07/clip.mp4");
  });

  it("tolerates a trailing slash on the root", () => {
    expect(clipRelName(`${ROOT}/`, `${ROOT}/Clips/clip.mp4`)).toBe("clip.mp4");
  });

  it("normalises Windows separators", () => {
    expect(clipRelName("C:\\Snippit", "C:\\Snippit\\Clips\\clip.mp4")).toBe("clip.mp4");
  });

  it("is null outside Clips/", () => {
    expect(clipRelName(ROOT, "/videos/elsewhere/clip.mp4")).toBeNull();
    expect(clipRelName(ROOT, `${ROOT}/Exports/clip.mp4`)).toBeNull();
  });

  it("is null without a configured root", () => {
    expect(clipRelName(null, "/videos/clip.mp4")).toBeNull();
    expect(clipRelName(undefined, `${ROOT}/Clips/clip.mp4`)).toBeNull();
  });
});

describe("isInClipsFolder", () => {
  it("flags imported clips", () => {
    expect(isInClipsFolder(ROOT, `${ROOT}/Clips/clip.mp4`)).toBe(true);
    expect(isInClipsFolder(ROOT, "/videos/clip.mp4")).toBe(false);
  });
});

describe("canRemoveIndexOnly", () => {
  it("refuses for the app's own Clips/ copies (the watcher would re-index them)", () => {
    expect(canRemoveIndexOnly(ROOT, `${ROOT}/Clips/clip.mp4`)).toBe(false);
  });

  it("allows for files opened in place from another folder", () => {
    expect(canRemoveIndexOnly(ROOT, "/videos/elsewhere/clip.mp4")).toBe(true);
  });

  it("allows when the root is unknown, since the file is untouched", () => {
    expect(canRemoveIndexOnly(null, `${ROOT}/Clips/clip.mp4`)).toBe(true);
  });
});
