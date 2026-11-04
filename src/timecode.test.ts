import { describe, expect, it } from "vitest";
import { formatTimecode } from "./timecode";

describe("formatTimecode", () => {
  it("formats sub-minute times", () => {
    expect(formatTimecode(0)).toBe("0:00.0");
    expect(formatTimecode(9.99)).toBe("0:09.9");
  });

  it("rolls over into minutes", () => {
    expect(formatTimecode(65.45)).toBe("1:05.4");
    expect(formatTimecode(600)).toBe("10:00.0");
  });

  it("treats missing or negative durations as zero", () => {
    expect(formatTimecode(NaN)).toBe("0:00.0");
    expect(formatTimecode(Infinity)).toBe("0:00.0");
    expect(formatTimecode(-3)).toBe("0:00.0");
  });
});
