import { describe, expect, test } from "bun:test";
import { parseWindowLabel } from "./overlay-label";

describe("parseWindowLabel", () => {
  test("parses the exact Rust label protocol", () => {
    expect(parseWindowLabel("overlay-42-1-3-20-1770000000000")).toEqual({
      kind: "overlay",
      parameters: {
        runId: 42,
        monitorIndex: 1,
        monitorCount: 3,
        durationSeconds: 20,
        deadlineMs: 1_770_000_000_000
      }
    });
  });

  test("routes ordinary labels to the dashboard", () => {
    expect(parseWindowLabel("main")).toEqual({ kind: "dashboard" });
  });

  test.each([
    "overlay-garbage",
    "overlay-1-0-1-8-1770000000000-extra",
    "overlay-01-0-1-8-1770000000000",
    "overlay-0-0-1-8-1770000000000",
    "overlay-1-1-1-8-1770000000000",
    "overlay-1-0-0-8-1770000000000",
    "overlay-1-0-65-8-1770000000000",
    "overlay-1-0-1-2-1770000000000",
    "overlay-1-0-1-31-1770000000000",
    "overlay-1-0-1-8-0"
  ])("fails safe for malformed overlay label %s", (label) => {
    expect(parseWindowLabel(label).kind).toBe("invalid-overlay");
  });
});
