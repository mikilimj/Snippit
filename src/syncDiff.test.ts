import { describe, expect, it } from "vitest";
import { displayNameFor, formatSize, percentIsValid } from "./syncDiff";

describe("formatSize", () => {
  it("formats bytes, MB and GB", () => {
    expect(formatSize(500)).toBe("500 B");
    expect(formatSize(1536 * 1024)).toBe("1.5 MB");
    expect(formatSize(2.25 * 1024 * 1024 * 1024)).toBe("2.25 GB");
  });
});

describe("displayNameFor", () => {
  const index = [{ relPath: "sub/a.mp4", displayName: "Ace round", favorite: true, folders: [] }];
  it("prefers the remote index display name", () => {
    expect(displayNameFor("sub/a.mp4", index)).toBe("Ace round");
  });
  it("falls back to the filename without extension", () => {
    expect(displayNameFor("sub/other.mp4", index)).toBe("other");
    expect(displayNameFor("plain.mp4", null)).toBe("plain");
  });
});

describe("percentIsValid", () => {
  it("accepts integers 1-99 only", () => {
    expect(percentIsValid(50)).toBe(true);
    expect(percentIsValid(1)).toBe(true);
    expect(percentIsValid(99)).toBe(true);
    expect(percentIsValid(0)).toBe(false);
    expect(percentIsValid(100)).toBe(false);
    expect(percentIsValid(49.5)).toBe(false);
  });
});
