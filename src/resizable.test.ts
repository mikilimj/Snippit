import { describe, expect, it } from "vitest";
import { clamp, parseStoredSize } from "./resizable";

describe("clamp", () => {
  it("passes through a value within range", () => {
    expect(clamp(50, 10, 100)).toBe(50);
  });
  it("clamps a value below min", () => {
    expect(clamp(5, 10, 100)).toBe(10);
  });
  it("clamps a value above max", () => {
    expect(clamp(500, 10, 100)).toBe(100);
  });
});

describe("parseStoredSize", () => {
  it("uses the parsed value when it's a valid in-range number", () => {
    expect(parseStoredSize("300", 224, 184, 480)).toBe(300);
  });
  it("clamps a parsed value below min", () => {
    expect(parseStoredSize("50", 224, 184, 480)).toBe(184);
  });
  it("clamps a parsed value above max", () => {
    expect(parseStoredSize("9999", 224, 184, 480)).toBe(480);
  });
  it("falls back to a clamped default when raw is null", () => {
    expect(parseStoredSize(null, 224, 184, 480)).toBe(224);
  });
  it("falls back to a clamped default when raw is non-numeric", () => {
    expect(parseStoredSize("not-a-number", 224, 184, 480)).toBe(224);
  });
  it("clamps the fallback itself when the fallback is out of range", () => {
    expect(parseStoredSize(null, 999, 184, 480)).toBe(480);
  });
});
