import { describe, expect, it } from "vitest";
import { isCustomSelectionValid, resolveTargetMb, TARGET_SIZE_PRESETS } from "./exportOptions";

describe("resolveTargetMb", () => {
  it("returns null for original quality (no size target)", () => {
    expect(resolveTargetMb("original", "")).toBeNull();
  });

  it("maps every preset to its MB value", () => {
    for (const preset of TARGET_SIZE_PRESETS) {
      expect(resolveTargetMb(String(preset), "")).toBe(preset);
    }
  });

  it("uses the custom field when custom is selected", () => {
    expect(resolveTargetMb("custom", "42")).toBe(42);
    expect(resolveTargetMb("custom", "7.5")).toBe(7.5);
  });

  it("returns null for unusable custom values", () => {
    expect(resolveTargetMb("custom", "")).toBeNull();
    expect(resolveTargetMb("custom", "0")).toBeNull();
    expect(resolveTargetMb("custom", "-5")).toBeNull();
    expect(resolveTargetMb("custom", "abc")).toBeNull();
  });
});

describe("isCustomSelectionValid", () => {
  it("is always true for non-custom selections", () => {
    expect(isCustomSelectionValid("original", "")).toBe(true);
    expect(isCustomSelectionValid("10", "abc")).toBe(true);
  });

  it("requires a positive number when custom is selected", () => {
    expect(isCustomSelectionValid("custom", "12")).toBe(true);
    expect(isCustomSelectionValid("custom", "")).toBe(false);
    expect(isCustomSelectionValid("custom", "0")).toBe(false);
  });
});
