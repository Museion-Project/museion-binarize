import { describe, expect, it } from "vitest";

import { formatBytes, formatPercent, formatSizeChange } from "./formatting";

describe("formatSizeChange", () => {
  it("reports a real reduction as smaller", () => {
    expect(formatSizeChange(0.871)).toBe("87% smaller");
  });

  it("reports growth as larger, never as negative shrinkage", () => {
    // The reported regression: a file that grew to 4.11x its input rendered
    // as "-311% smaller". It grew by 311%, so it is 311% larger.
    expect(formatSizeChange(-3.11)).toBe("311% larger");
    expect(formatSizeChange(-3.11)).not.toContain("smaller");
    expect(formatSizeChange(-3.11)).not.toContain("-");
  });

  it("does not claim a change that rounds to nothing", () => {
    expect(formatSizeChange(0.001)).toBe("about the same size");
    expect(formatSizeChange(-0.001)).toBe("about the same size");
  });

  it("omits the phrase when the input size was unknown", () => {
    expect(formatSizeChange(null)).toBeNull();
    expect(formatSizeChange(Number.NaN)).toBeNull();
  });

  it("matches the plain percentage helper for ordinary reductions", () => {
    expect(formatSizeChange(0.5)).toBe(`${formatPercent(0.5)} smaller`);
  });
});

describe("formatBytes", () => {
  it("keeps small sizes in bytes and scales the rest", () => {
    expect(formatBytes(512)).toBe("512 B");
    expect(formatBytes(2048)).toBe("2.0 KB");
  });
});
