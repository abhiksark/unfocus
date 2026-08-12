import { describe, expect, test } from "bun:test";
import {
  activityFootnote,
  currentKindLabel,
  deepBlockCaption,
  formatActivityDuration,
  isActivityWindowEmpty,
  stripActiveHeight,
  stripAfkHeight,
  stripAriaLabel,
  todayErrorCaption,
  todayLoadingCaption,
  type TodayActivity
} from "./today-activity";

function sample(partial: Partial<TodayActivity> = {}): TodayActivity {
  return {
    windowLabel: "Last 24 hours",
    windowSeconds: 86_400,
    activeSeconds: 3_600,
    afkSeconds: 1_800,
    unknownSeconds: 80_000,
    longestActiveSeconds: 2_700,
    deepBlockCount: 2,
    deepBlockMinSeconds: 25 * 60,
    afkThresholdSeconds: 5 * 60,
    currentKind: "active",
    probeAvailable: true,
    strip: [
      { activeRatio: 0.8, afkRatio: 0.1 },
      { activeRatio: 0.2, afkRatio: 0.7 },
      { activeRatio: 0, afkRatio: 0 }
    ],
    ...partial
  };
}

describe("today activity presentation", () => {
  test("formats durations for dashboard stats", () => {
    expect(formatActivityDuration(0)).toBe("<1m");
    expect(formatActivityDuration(59)).toBe("<1m");
    expect(formatActivityDuration(60)).toBe("1m");
    expect(formatActivityDuration(3_600)).toBe("1h");
    expect(formatActivityDuration(3_720)).toBe("1h 2m");
    expect(formatActivityDuration(-1)).toBe("—");
  });

  test("labels current kind with distinct probe and waiting states", () => {
    expect(currentKindLabel("active", true)).toBe("At the keyboard");
    expect(currentKindLabel("afk", true)).toBe("Away from the keyboard");
    expect(currentKindLabel("unknown", true)).toBe("Waiting for presence samples");
    expect(currentKindLabel("active", false)).toBe("Presence probe unavailable");
    expect(currentKindLabel(null, true)).toBe("Waiting for presence samples");
    expect(currentKindLabel(null, false)).toBe("Presence probe unavailable");
  });

  test("describes deep blocks without repeating the count", () => {
    expect(deepBlockCaption(0, 25 * 60)).toBe("None yet · ≥25m continuous");
    expect(deepBlockCaption(1, 25 * 60)).toBe("≥25m continuous");
    expect(deepBlockCaption(3, 25 * 60)).toBe("≥25m continuous");
  });

  test("keeps the privacy footnote observational", () => {
    expect(activityFootnote(5 * 60)).toContain("nothing is keylogged");
    expect(activityFootnote(5 * 60)).toContain("5m");
    expect(activityFootnote(5 * 60)).not.toMatch(/streak|score|badge/i);
  });

  test("uses calm loading and error captions", () => {
    expect(todayLoadingCaption()).toContain("Collecting");
    expect(todayErrorCaption(null)).toContain("unaffected");
    expect(todayErrorCaption("probe cache lock is poisoned")).toContain(
      "probe cache lock is poisoned"
    );
    const longError = todayErrorCaption("x".repeat(200));
    expect(longError.length).toBeLessThan(120);
    expect(longError).toContain("unaffected");
  });

  test("detects an empty classified window", () => {
    expect(isActivityWindowEmpty(sample({ activeSeconds: 0, afkSeconds: 0 }))).toBe(true);
    expect(isActivityWindowEmpty(sample({ activeSeconds: 1, afkSeconds: 0 }))).toBe(false);
  });

  test("builds a calm strip aria label", () => {
    expect(stripAriaLabel(sample())).toBe(
      "Last 24 hours: 1 mostly-active and 1 mostly-away half-hour blocks"
    );
    expect(stripAriaLabel(sample({ strip: [] }))).toBe(
      "Last 24 hours: no activity samples yet"
    );
  });

  test("clamps strip bar heights", () => {
    expect(stripActiveHeight({ activeRatio: 1.4, afkRatio: 0 })).toBe(1);
    expect(stripAfkHeight({ activeRatio: 0, afkRatio: -0.2 })).toBe(0);
    expect(stripActiveHeight({ activeRatio: 0.5, afkRatio: 0.5 })).toBe(0.5);
  });
});
