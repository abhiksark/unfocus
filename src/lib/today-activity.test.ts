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
  stripAxisTicks,
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

describe("stripAxisTicks", () => {
  const DAY_SECONDS = 86_400;
  // Mid-January carries no daylight-saving transition in common zones, so the
  // tick count is the same in CI (UTC) and on a developer machine.
  const JANUARY_NOON_MS = Date.parse("2026-01-15T12:00:00Z");

  test("marks six four-hour ticks across a day window", () => {
    expect(stripAxisTicks(DAY_SECONDS, JANUARY_NOON_MS)).toHaveLength(6);
  });

  test("lands every tick on a four-hour local boundary", () => {
    for (const tick of stripAxisTicks(DAY_SECONDS, JANUARY_NOON_MS)) {
      const at = new Date(tick.timestampMs);
      expect(at.getHours() % 4).toBe(0);
      expect(at.getMinutes()).toBe(0);
      expect(at.getSeconds()).toBe(0);
    }
  });

  test("orders ticks left to right inside the window", () => {
    const ticks = stripAxisTicks(DAY_SECONDS, JANUARY_NOON_MS);
    const startMs = JANUARY_NOON_MS - DAY_SECONDS * 1_000;
    let previous = -1;
    for (const tick of ticks) {
      expect(tick.positionPercent).toBeGreaterThan(previous);
      previous = tick.positionPercent;
      expect(tick.positionPercent).toBeGreaterThanOrEqual(0);
      expect(tick.positionPercent).toBeLessThanOrEqual(100);
      expect(tick.timestampMs).toBeGreaterThanOrEqual(startMs);
      expect(tick.timestampMs).toBeLessThan(JANUARY_NOON_MS);
    }
  });

  test("suppresses a label that would collide with the now anchor", () => {
    const base = stripAxisTicks(DAY_SECONDS, JANUARY_NOON_MS);
    const lastBoundaryMs = base[base.length - 1].timestampMs;
    // Ending the window 30 minutes past a boundary puts that tick at ~97.9%.
    const ticks = stripAxisTicks(DAY_SECONDS, lastBoundaryMs + 30 * 60 * 1_000);
    const final = ticks[ticks.length - 1];

    expect(final.timestampMs).toBe(lastBoundaryMs);
    expect(final.positionPercent).toBeGreaterThan(94);
    expect(final.showLabel).toBe(false);
    expect(ticks.filter((tick) => tick.showLabel)).toHaveLength(ticks.length - 1);
  });

  test("keeps a daylight-saving length window in range", () => {
    const ticks = stripAxisTicks(25 * 60 * 60, JANUARY_NOON_MS);

    expect(ticks.length).toBeGreaterThanOrEqual(6);
    expect(ticks.length).toBeLessThanOrEqual(7);
    expect(new Set(ticks.map((tick) => tick.timestampMs)).size).toBe(ticks.length);
    for (const tick of ticks) {
      expect(tick.positionPercent).toBeGreaterThanOrEqual(0);
      expect(tick.positionPercent).toBeLessThanOrEqual(100);
    }
  });

  test("returns nothing for an unusable window or clock", () => {
    expect(stripAxisTicks(0, JANUARY_NOON_MS)).toEqual([]);
    expect(stripAxisTicks(-1, JANUARY_NOON_MS)).toEqual([]);
    expect(stripAxisTicks(Number.NaN, JANUARY_NOON_MS)).toEqual([]);
    expect(stripAxisTicks(DAY_SECONDS, Number.NaN)).toEqual([]);
    expect(stripAxisTicks(DAY_SECONDS, Number.POSITIVE_INFINITY)).toEqual([]);
  });

  test("labels every tick with localized text", () => {
    for (const tick of stripAxisTicks(DAY_SECONDS, JANUARY_NOON_MS)) {
      expect(tick.label.length).toBeGreaterThan(0);
    }
  });
});
