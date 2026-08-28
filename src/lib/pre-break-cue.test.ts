// src/lib/pre-break-cue.test.ts

import { describe, expect, test } from "bun:test";
import { preBreakCuePresentation } from "./pre-break-cue";

describe("pre-break cue", () => {
  test("uses compact, final-ten-second, and zero-time stages", () => {
    const deadline = 100_000;
    expect(preBreakCuePresentation(deadline, 89_999)).toEqual({
      stage: "compact",
      secondsLeft: 11,
      countdownProgress: 0
    });
    expect(preBreakCuePresentation(deadline, 90_000)).toEqual({
      stage: "countdown",
      secondsLeft: 10,
      countdownProgress: 0
    });
    expect(preBreakCuePresentation(deadline, 95_000)).toEqual({
      stage: "countdown",
      secondsLeft: 5,
      countdownProgress: 0.5
    });
    expect(preBreakCuePresentation(deadline, 99_999)).toEqual({
      stage: "countdown",
      secondsLeft: 1,
      countdownProgress: 0.9999
    });
    expect(preBreakCuePresentation(deadline, 100_000)).toEqual({
      stage: "due",
      secondsLeft: 0,
      countdownProgress: 1
    });
  });

  test("keeps reduced motion free of expansion and blur", async () => {
    const source = await Bun.file(new URL("./PreBreakCue.svelte", import.meta.url)).text();
    const reducedMotion = source.split("@media (prefers-reduced-motion: reduce)")[1];

    expect(reducedMotion).toContain("animation: cue-arrive 120ms linear forwards");
    expect(reducedMotion).toContain("transition: none");
    expect(reducedMotion).toContain("filter: none");
  });
});
