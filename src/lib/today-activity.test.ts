import { describe, expect, test } from "bun:test";
import {
  currentKindLabel,
  deepBlockCaption,
  formatActivityDuration,
  stripActiveHeight,
  stripAfkHeight,
  stripAriaLabel,
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

  test("labels current kind without inventing probe health", () => {
    expect(currentKindLabel("active", true)).toBe("At the keyboard");
    expect(currentKindLabel("afk", true)).toBe("Away");
    expect(currentKindLabel("unknown", true)).toBe("Idle probe unavailable");
    expect(currentKindLabel("active", false)).toBe("Idle probe unavailable");
    expect(currentKindLabel(null, true)).toBe("Idle probe unavailable");
  });

  test("describes deep blocks without gamified streaks", () => {
    expect(deepBlockCaption(0, 25 * 60)).toBe("No deep blocks yet (≥25m)");
    expect(deepBlockCaption(1, 25 * 60)).toBe("1 deep block (≥25m)");
    expect(deepBlockCaption(3, 25 * 60)).toBe("3 deep blocks (≥25m)");
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
