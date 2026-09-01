import { describe, expect, test } from "bun:test";
import {
  breakErrorCaption,
  breakLoadingCaption,
  breakOutcomeStats,
  breakSummaryCaption,
  isBreakDayEmpty,
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
  test("describes empty windows once without gamification", () => {
    expect(isBreakDayEmpty(sample())).toBe(true);
    expect(breakSummaryCaption(sample())).toBe(
      "No break outcomes in the last seven days."
    );
    expect(weekBreakCaption(sample())).toBe("");
  });

  test("distinguishes an empty day when the week has outcomes", () => {
    const summary = sample({ weekScheduledShown: 2 });
    expect(breakSummaryCaption(summary)).toBe("No break outcomes in the last day.");
    expect(weekBreakCaption(summary)).toBe("2 outcomes in the last seven days.");
  });

  test("does not re-list counts when the day has outcomes", () => {
    const summary = sample({
      scheduledShown: 2,
      naturalIdle: 1,
      manualTakeBreak: 1,
      fullscreenSuppress: 3
    });
    expect(isBreakDayEmpty(summary)).toBe(false);
    expect(breakSummaryCaption(summary)).toBe("Stored only on this device.");
    expect(breakSummaryCaption(summary)).not.toMatch(/\d+ rests? shown/);
  });

  test("exposes stable stats with calm hints", () => {
    const stats = breakOutcomeStats(
      sample({ scheduledShown: 2, naturalIdle: 0, manualTakeBreak: 1, fullscreenSuppress: 0 })
    );
    expect(stats.map((stat) => stat.label)).toEqual([
      "Scheduled",
      "Already away",
      "Started by you",
      "Held for fullscreen"
    ]);
    expect(stats.find((stat) => stat.kind === "scheduledShown")?.count).toBe(2);
    expect(stats.find((stat) => stat.kind === "naturalIdle")?.count).toBe(0);
    expect(stats.every((stat) => stat.hint.length > 0)).toBe(true);
    expect(stats.map((stat) => stat.hint).join(" ")).not.toMatch(/streak|score|badge/i);
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

  test("uses calm loading and error captions", () => {
    expect(breakLoadingCaption()).toContain("Reading");
    expect(breakErrorCaption(null)).toContain("unaffected");
    expect(breakErrorCaption("ledger lock poisoned")).toContain("ledger lock poisoned");
  });
});
