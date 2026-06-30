import { describe, expect, it } from "vitest";
import { setIn, setOut } from "./trim";

describe("trim clamps", () => {
  it("clamps IN to >= 0", () => {
    expect(setIn(-5, null)).toBe(0);
  });
  it("keeps IN strictly left of OUT", () => {
    expect(setIn(9, 8)).toBeCloseTo(7.99, 5);
  });
  it("clamps OUT to <= duration", () => {
    expect(setOut(100, null, 30)).toBe(30);
  });
  it("keeps OUT strictly right of IN", () => {
    expect(setOut(2, 5, 30)).toBeCloseTo(5.01, 5);
  });
});
