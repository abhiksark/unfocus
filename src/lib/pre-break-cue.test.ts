// src/lib/pre-break-cue.test.ts

import { describe, expect, test } from "bun:test";
import { preBreakCuePresentation } from "./pre-break-cue";

describe("pre-break cue", () => {
  test("shows a brief heads-up, rests quietly, counts down, and holds the handoff", () => {
    const deadline = 100_000;
    expect(preBreakCuePresentation(deadline, 39_999)).toEqual({
      stage: "quiet",
      secondsLeft: 61,
      visible: false
    });
    expect(preBreakCuePresentation(deadline, 40_000)).toEqual({
      stage: "heads-up",
      secondsLeft: 60,
      visible: true
    });
    expect(preBreakCuePresentation(deadline, 43_999)).toEqual({
      stage: "heads-up",
      secondsLeft: 57,
      visible: true
    });
    expect(preBreakCuePresentation(deadline, 44_000)).toEqual({
      stage: "quiet",
      secondsLeft: 56,
      visible: false
    });
    expect(preBreakCuePresentation(deadline, 89_999)).toEqual({
      stage: "quiet",
      secondsLeft: 11,
      visible: false
    });
    expect(preBreakCuePresentation(deadline, 90_000)).toEqual({
      stage: "countdown",
      secondsLeft: 10,
      visible: true
    });
    expect(preBreakCuePresentation(deadline, 99_999)).toEqual({
      stage: "countdown",
      secondsLeft: 1,
      visible: true
    });
    expect(preBreakCuePresentation(deadline, 100_000)).toEqual({
      stage: "handoff",
      secondsLeft: 0,
      visible: true
    });
  });

  test("uses one fixed card without a progress rail, expanding shell, or blur", async () => {
    const source = await Bun.file(new URL("./PreBreakCue.svelte", import.meta.url)).text();

    expect(source).toContain("width: min(352px, calc(100vw - 40px))");
    expect(source).not.toContain("cue-progress");
    expect(source).not.toContain("class:countdown");
    expect(source).not.toMatch(/^\s+filter:/m);
  });

  test("keeps reduced motion to short opacity and color fades", async () => {
    const source = await Bun.file(new URL("./PreBreakCue.svelte", import.meta.url)).text();
    const reducedMotion = source.split("@media (prefers-reduced-motion: reduce)")[1];

    expect(reducedMotion).toContain("opacity 120ms linear");
    expect(reducedMotion).toContain("transform: none");
    expect(reducedMotion).not.toContain("blur");
  });
});
