// src/lib/overlay-label.test.ts

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

  test("parses the strict cue label protocol", () => {
    expect(parseWindowLabel("cue-42-1770000000000")).toEqual({
      kind: "cue",
      parameters: { runId: 42, deadlineMs: 1_770_000_000_000 }
    });
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

  test.each([
    "cue",
    "cue-",
    "cue-garbage",
    "cue-1-1770000000000-extra",
    "cue-01-1770000000000",
    "cue-0-1770000000000",
    "cue-1-0",
    "cue-9007199254740992-1770000000000",
    "cue-1-9007199254740992"
  ])("routes malformed cue label %s to an inert cue surface", (label) => {
    expect(parseWindowLabel(label).kind).toBe("invalid-cue");
  });
});
