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
      parameters: { runId: 42, deadlineMs: 1_770_000_000_000, mode: "scheduled" }
    });
  });

  test("parses preview cues without confusing preview for a run ID", () => {
    expect(parseWindowLabel("cue-preview-43-1770000000000")).toEqual({
      kind: "cue",
      parameters: { runId: 43, deadlineMs: 1_770_000_000_000, mode: "preview" }
    });
  });

  test("passes label-derived mode and trusted native platform into the cue", async () => {
    const source = await Bun.file(new URL("../routes/+page.svelte", import.meta.url)).text();

    expect(source).toContain("mode={cueParameters.mode}");
    expect(source).toContain("platform={cuePlatform}");
    expect(source).toContain("window.__UNFOCUS_PRE_BREAK_CUE_PLATFORM__");
    expect(source).not.toContain("userAgent");
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
    "cue-1-9007199254740992",
    "cue-preview",
    "cue-preview-01-1770000000000",
    "cue-preview-0-1770000000000",
    "cue-preview-1-0",
    "cue-preview-1-1770000000000-extra",
    "cue-other-1-1770000000000"
  ])("routes malformed cue label %s to an inert cue surface", (label) => {
    expect(parseWindowLabel(label).kind).toBe("invalid-cue");
  });
});

test("only exact main and tray-panel labels route to application surfaces", () => {
  expect(parseWindowLabel("tray-panel")).toEqual({ kind: "tray-panel" });
  for (const label of ["tray-panel-1", "unknown", "", "main-2"]) {
    expect(parseWindowLabel(label)).toEqual({ kind: "invalid-window" });
  }
});
