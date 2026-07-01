import { describe, expect, it } from "vitest";
import { canContinueFromRootStep, canFinishIngestionStep } from "./wizardValidation";

describe("canContinueFromRootStep", () => {
  it("is false with no root path chosen", () => {
    expect(canContinueFromRootStep(null)).toBe(false);
  });
  it("is true once a root path is chosen", () => {
    expect(canContinueFromRootStep("/home/user/ClipSnip")).toBe(true);
  });
});

describe("canFinishIngestionStep", () => {
  it("is false with no mode selected", () => {
    expect(canFinishIngestionStep(null, null)).toBe(false);
  });
  it("is true for empty mode with no source folder", () => {
    expect(canFinishIngestionStep("empty", null)).toBe(true);
  });
  it("is false for import mode without a source folder", () => {
    expect(canFinishIngestionStep("import", null)).toBe(false);
  });
  it("is true for import mode with a source folder", () => {
    expect(canFinishIngestionStep("import", "/home/user/OBS")).toBe(true);
  });
  it("is false for watch mode without a source folder", () => {
    expect(canFinishIngestionStep("watch", null)).toBe(false);
  });
});
