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

  test("uses one fixed premium-hybrid card without a progress rail, expanding shell, or blur", async () => {
    const source = await Bun.file(new URL("./PreBreakCue.svelte", import.meta.url)).text();

    expect(source).toContain("width: min(336px, calc(100vw - 40px))");
    expect(source).toContain("height: min(68px, calc(100vh - 40px))");
    expect(source).toContain("border-radius: 24px");
    expect(source).toContain("{#if presentation.visible}");
    expect(source).toContain('invoke("set_pre_break_cue_visibility", { visible })');
    expect(source).toContain("animation: cue-card-arrive 160ms ease both");
    expect(source).not.toContain("cue-progress");
    expect(source).not.toContain("class:countdown");
    expect(source).not.toMatch(/^\s+filter:/m);
  });

  test("keeps reduced motion to short opacity and color fades", async () => {
    const source = await Bun.file(new URL("./PreBreakCue.svelte", import.meta.url)).text();
    const reducedMotion = source.split("@media (prefers-reduced-motion: reduce)")[1];

    expect(reducedMotion).toContain("animation: cue-card-arrive 120ms linear both");
    expect(reducedMotion).toContain("border-color 120ms linear");
    expect(reducedMotion).not.toContain("blur");
  });
});
