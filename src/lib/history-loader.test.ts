import { describe, expect, test } from "bun:test";
import {
  buildHistoryCalendarRequest,
  buildHistoryDayDetailRequest,
  type ActivityRangeBucket,
  type BreakHistoryEvent,
  type HistoryCalendar,
  type HistoryDay
} from "./history";
import {
  createHistoryCalendarLoader,
  createHistoryDayLoader
} from "./history-loader";

type Deferred<Value> = {
  promise: Promise<Value>;
  resolve: (value: Value) => void;
  reject: (error: unknown) => void;
};

function deferred<Value>(): Deferred<Value> {
  let resolve!: (value: Value) => void;
  let reject!: (error: unknown) => void;
  const promise = new Promise<Value>((done, fail) => {
    resolve = done;
    reject = fail;
  });
  return { promise, resolve, reject };
}

function buckets(count: number, activeMs = 0): ActivityRangeBucket[] {
  return Array.from({ length: count }, () => ({
    activeMs,
    afkMs: 0,
    longestActiveMs: activeMs
  }));
}

describe("history calendar loading", () => {
  test("loads the 90-day grid with three daily activity requests", async () => {
    const request = buildHistoryCalendarRequest(Date.parse("2026-08-16T10:30:00Z"), 0);
    const starts = request.pages.map((page) => page.startMs);
    const calls: number[][] = [];
    const applied: HistoryCalendar[] = [];
    const load = createHistoryCalendarLoader(
      {
        getActivityRange: async ({ boundaries }) => {
          calls.push(boundaries);
          const pageIndex = starts.indexOf(boundaries[0]);
          if (pageIndex < 0) throw new Error("unexpected page");
          return buckets(boundaries.length - 1, (pageIndex + 1) * 1_000);
        }
      },
      (calendar) => {
        applied.push(calendar);
      },
      () => {
        throw new Error("calendar load must not fail");
      }
    );

    await expect(load(request)).resolves.toBe("applied");
    expect(calls).toHaveLength(3);
    expect(calls.every((boundaries) => boundaries.length === 31)).toBe(true);
    expect(applied[0].days).toHaveLength(90);
    expect(applied[0].days[0].totals.activeMs).toBe(3_000);
    expect(applied[0].days[89].totals.activeMs).toBe(1_000);
  });

  test("reports a failure without applying a partial calendar", async () => {
    const request = buildHistoryCalendarRequest(Date.parse("2026-08-16T10:30:00Z"), 0);
    const failure = new Error("archive unavailable");
    const failures: unknown[] = [];
    const load = createHistoryCalendarLoader(
      {
        getActivityRange: async ({ boundaries }) => {
          if (boundaries[0] === request.pages[1].startMs) throw failure;
          return buckets(boundaries.length - 1);
        }
      },
      () => {
        throw new Error("a partial calendar must not apply");
      },
      (error) => failures.push(error)
    );

    await expect(load(request)).resolves.toBe("failed");
    expect(failures).toEqual([failure]);
  });
});

describe("selected history day loading", () => {
  test("requests only the selected day's hours and break outcomes", async () => {
    const calendar = buildHistoryCalendarRequest(Date.parse("2026-08-16T10:30:00Z"), 0);
    const request = buildHistoryDayDetailRequest(calendar, "2026-08-15");
    const daily = { activeMs: 3_600_000, afkMs: 600_000, longestActiveMs: 1_800_000 };
    const activityCalls: number[][] = [];
    const breakCalls: Array<{ startMs: number; endMs: number }> = [];
    const applied: HistoryDay[] = [];
    const load = createHistoryDayLoader(
      {
        getActivityRange: async ({ boundaries }) => {
          activityCalls.push(boundaries);
          return buckets(boundaries.length - 1, 60_000);
        },
        getBreakRange: async (range) => {
          breakCalls.push(range);
          return [{ atMs: range.startMs + 5 * 60_000, kind: "scheduledShown" }];
        }
      },
      (day) => {
        applied.push(day);
      },
      () => {
        throw new Error("day load must not fail");
      }
    );

    await expect(load(request, daily)).resolves.toBe("applied");
    expect(activityCalls).toEqual([request.hourBoundariesMs]);
    expect(breakCalls).toEqual([{ startMs: request.startMs, endMs: request.endMs }]);
    expect(applied[0].dateKey).toBe("2026-08-15");
    expect(applied[0].totals.activeMs).toBe(daily.activeMs);
    expect(applied[0].breakCounts.find((count) => count.kind === "scheduledShown")?.count).toBe(1);
  });

  test("ignores a slower day after a newer selection is requested", async () => {
    const calendar = buildHistoryCalendarRequest(Date.parse("2026-08-16T10:30:00Z"), 0);
    const older = buildHistoryDayDetailRequest(calendar, "2026-08-14");
    const newer = buildHistoryDayDetailRequest(calendar, "2026-08-15");
    const activity = new Map<number, Deferred<ActivityRangeBucket[]>>();
    const breaks = new Map<number, Deferred<BreakHistoryEvent[]>>();
    const applied: string[] = [];
    const failures: string[] = [];
    const load = createHistoryDayLoader(
      {
        getActivityRange: ({ boundaries }) => {
          const pending = activity.get(boundaries[0]);
          if (!pending) throw new Error("unexpected activity request");
          return pending.promise;
        },
        getBreakRange: ({ startMs }) => {
          const pending = breaks.get(startMs);
          if (!pending) throw new Error("unexpected break request");
          return pending.promise;
        }
      },
      (day) => applied.push(day.dateKey),
      (error) => failures.push(error instanceof Error ? error.message : String(error))
    );

    for (const request of [older, newer]) {
      activity.set(request.startMs, deferred());
      breaks.set(request.startMs, deferred());
    }

    const olderLoad = load(older, buckets(1)[0]);
    const newerLoad = load(newer, buckets(1)[0]);
    activity.get(newer.startMs)?.resolve(buckets(newer.hourBoundariesMs.length - 1));
    breaks.get(newer.startMs)?.resolve([]);
    await expect(newerLoad).resolves.toBe("applied");

    activity.get(older.startMs)?.reject(new Error("stale native failure"));
    await expect(olderLoad).resolves.toBe("stale");
    expect(applied).toEqual(["2026-08-15"]);
    expect(failures).toEqual([]);
  });

  test("reports a failure from the currently selected day", async () => {
    const calendar = buildHistoryCalendarRequest(Date.parse("2026-08-16T10:30:00Z"), 0);
    const request = buildHistoryDayDetailRequest(calendar, "2026-08-15");
    const failure = new Error("selected day unavailable");
    const failures: unknown[] = [];
    const load = createHistoryDayLoader(
      {
        getActivityRange: async () => {
          throw failure;
        },
        getBreakRange: async () => []
      },
      () => {
        throw new Error("a failed day must not apply");
      },
      (error) => failures.push(error)
    );

    await expect(load(request, buckets(1)[0])).resolves.toBe("failed");
    expect(failures).toEqual([failure]);
  });
});
