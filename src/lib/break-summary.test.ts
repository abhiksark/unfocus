import { describe, expect, test } from "bun:test";
import {
  breakSummaryCaption,
  weekBreakCaption,
  type BreakSummary
} from "./break-summary";

function sample(partial: Partial<BreakSummary> = {}): BreakSummary {
  return {
    windowLabel: "Last 24 hours",
    windowSeconds: 86_400,
    scheduledShown: 0,
    naturalIdle: 0,
    fullscreenSuppress: 0,
    manualTakeBreak: 0,
    weekScheduledShown: 0,
    weekNaturalIdle: 0,
    weekFullscreenSuppress: 0,
    weekManualTakeBreak: 0,
    ...partial
  };
}

describe("break summary presentation", () => {
  test("describes empty windows without gamification", () => {
    expect(breakSummaryCaption(sample())).toBe(
      "No break outcomes recorded yet in this window."
    );
    expect(weekBreakCaption(sample())).toBe("Nothing recorded in the last seven days.");
  });

  test("lists distinguishable outcomes calmly", () => {
    expect(
      breakSummaryCaption(
        sample({
          scheduledShown: 2,
          naturalIdle: 1,
          manualTakeBreak: 1,
          fullscreenSuppress: 3
        })
      )
    ).toBe(
      "2 rests shown · 1 natural rest · 1 manual rest · 3 held for fullscreen"
    );
  });

  test("summarizes the week without scores", () => {
    expect(
      weekBreakCaption(
        sample({
          weekScheduledShown: 4,
          weekNaturalIdle: 2,
          weekManualTakeBreak: 1
        })
      )
    ).toBe("7 outcomes in the last seven days.");
  });
});
