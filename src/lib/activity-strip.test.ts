import { describe, expect, test } from "bun:test";
import { activityIntervals, activityShares, activityWindowLabel, breakEventLabel, createBreakMarkerLoader, visibleAxisLabels } from "./activity-strip";
import type { BreakHistoryEvent } from "./history";

describe("activity interval presentation", () => {
  test("reserves measured endpoint space and prioritizes the selected day boundary", () => {
    const labels = [
      { left: -3, width: 30, isDayStart: false },
      { left: 120, width: 40, isDayStart: false },
      { left: 165, width: 40, isDayStart: true },
      { left: 727, width: 15, isDayStart: false }
    ];
    expect(visibleAxisLabels(labels, 802, 72)).toEqual([false, false, true, false]);
    expect(visibleAxisLabels(labels, 802, 22)).toEqual([false, false, true, true]);
    expect(visibleAxisLabels(labels, 100, 72)).toEqual([false, false, false, false]);
  });

  test("uses label widths after a font or locale change", () => {
    expect(visibleAxisLabels([
      { left: 30, width: 20, isDayStart: false },
      { left: 70, width: 20, isDayStart: false }
    ], 200, 20)).toEqual([true, true]);
    expect(visibleAxisLabels([
      { left: 30, width: 48, isDayStart: false },
      { left: 70, width: 48, isDayStart: false }
    ], 200, 20)).toEqual([true, false]);
  });
  test("preserves equal active and away shares rather than overlapping them", () => {
    expect(activityShares({ activeRatio: 0.5, afkRatio: 0.5 })).toEqual({ active: 0.5, away: 0.5, unknown: 0 });
    expect(activityShares({ activeRatio: 0.25, afkRatio: 0.25 })).toEqual({ active: 0.25, away: 0.25, unknown: 0.5 });
    expect(activityShares({ activeRatio: NaN, afkRatio: -1 })).toEqual({ active: 0, away: 0, unknown: 1 });
    expect(activityShares({ activeRatio: 0.8, afkRatio: 0.8 }).away).toBeCloseTo(0.2);
  });

  test("places event boundaries in one half-open interval and excludes outside events", () => {
    const end = new Date(2026, 8, 14, 14, 0).getTime();
    const start = end - 3_600_000;
    const events: BreakHistoryEvent[] = [
      { atMs: start - 1, kind: "naturalIdle" },
      { atMs: start, kind: "scheduledShown" },
      { atMs: start + 1_800_000, kind: "manualTakeBreak" },
      { atMs: end, kind: "fullscreenSuppress" }
    ];
    const intervals = activityIntervals({ windowSeconds: 3_600, strip: [
      { activeRatio: 0.5, afkRatio: 0.5 }, { activeRatio: 0, afkRatio: 0 }
    ] }, end, events);
    expect(intervals[0].events).toEqual([events[1]]);
    expect(intervals[1].events).toEqual([events[2]]);
    expect(intervals[0].details).toBe("15m active · 15m away · 0m unclassified");
    expect(intervals[1].details).toBe("0m active · 0m away · 30m unclassified");
    expect(intervals[1].endMs).toBe(end);
    const formatter = new Intl.DateTimeFormat([], {
      month: "short", day: "numeric", hour: "numeric", minute: "2-digit"
    });
    expect(activityWindowLabel(86_400, end)).toBe(
      `${formatter.format(end - 86_400_000)} – ${formatter.format(end)}`
    );
    expect(breakEventLabel(events[1])).toContain("Scheduled break shown");
  });
});

test("late marker responses cannot replace a newer window or survive cancellation", async () => {
  const requests: Array<{ resolve: (events: BreakHistoryEvent[]) => void; reject: () => void }> = [];
  const applied: BreakHistoryEvent[][] = [];
  let failures = 0;
  const loader = createBreakMarkerLoader(
    () => new Promise((resolve, reject) => requests.push({ resolve, reject })),
    (events) => applied.push(events),
    () => failures++
  );
  const first = loader.load(0, 100);
  const second = loader.load(100, 200);
  const fresh: BreakHistoryEvent[] = [{ atMs: 150, kind: "manualTakeBreak" }];
  requests[1].resolve(fresh);
  await second;
  requests[0].reject();
  await first;
  expect(applied).toEqual([fresh]);
  expect(failures).toBe(0);
  const hidden = loader.load(200, 300);
  loader.cancel();
  requests[2].resolve([]);
  await hidden;
  expect(applied).toEqual([fresh]);
  const failed = loader.load(200, 300);
  requests[3].reject();
  await failed;
  expect(failures).toBe(1);
});
