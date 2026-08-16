import { describe, expect, test } from "bun:test";
import {
  HISTORY_PAGE_DAYS,
  buildHistoryPageRequest,
  type ActivityRangeBucket,
  type BreakHistoryEvent
} from "./history";
import { createHistoryPageLoader } from "./history-loader";

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

function buckets(count: number): ActivityRangeBucket[] {
  return Array.from({ length: count }, () => ({
    activeMs: 0,
    afkMs: 0,
    longestActiveMs: 0
  }));
}

describe("history page loading", () => {
  test("ignores a slower older page after a newer page is requested", async () => {
    const nowMs = Date.parse("2026-08-15T10:30:00Z");
    const older = buildHistoryPageRequest(1, nowMs, 0);
    const newer = buildHistoryPageRequest(0, nowMs, 0);
    const daily = new Map<number, Deferred<ActivityRangeBucket[]>>();
    const hourly = new Map<number, Deferred<ActivityRangeBucket[]>>();
    const breaks = new Map<number, Deferred<BreakHistoryEvent[]>>();
    const applied: number[] = [];
    const loadPage = createHistoryPageLoader(
      {
        getActivityRange: ({ boundaries }) => {
          const store = boundaries.length === HISTORY_PAGE_DAYS + 1 ? daily : hourly;
          const pending = store.get(boundaries[0]);
          if (!pending) throw new Error("unexpected activity request");
          return pending.promise;
        },
        getBreakRange: ({ startMs }) => {
          const pending = breaks.get(startMs);
          if (!pending) throw new Error("unexpected break request");
          return pending.promise;
        }
      },
      (page) => {
        applied.push(page.pageIndex);
      },
      () => {
        throw new Error("no load should fail");
      }
    );

    for (const request of [older, newer]) {
      daily.set(request.startMs, deferred());
      hourly.set(request.startMs, deferred());
      breaks.set(request.startMs, deferred());
    }

    const olderLoad = loadPage(older);
    const newerLoad = loadPage(newer);

    daily.get(newer.startMs)?.resolve(buckets(newer.days.length));
    hourly
      .get(newer.startMs)
      ?.resolve(buckets(newer.hourBoundariesMs.length - 1));
    breaks.get(newer.startMs)?.resolve([]);
    await expect(newerLoad).resolves.toBe("applied");

    daily.get(older.startMs)?.resolve(buckets(older.days.length));
    hourly
      .get(older.startMs)
      ?.resolve(buckets(older.hourBoundariesMs.length - 1));
    breaks.get(older.startMs)?.resolve([]);
    await expect(olderLoad).resolves.toBe("stale");

    expect(applied).toEqual([0]);
  });

  test("ignores a slower older failure after a newer page is applied", async () => {
    const nowMs = Date.parse("2026-08-15T10:30:00Z");
    const older = buildHistoryPageRequest(1, nowMs, 0);
    const newer = buildHistoryPageRequest(0, nowMs, 0);
    const daily = new Map<number, Deferred<ActivityRangeBucket[]>>();
    const hourly = new Map<number, Deferred<ActivityRangeBucket[]>>();
    const breaks = new Map<number, Deferred<BreakHistoryEvent[]>>();
    const applied: number[] = [];
    const failures: string[] = [];
    const loadPage = createHistoryPageLoader(
      {
        getActivityRange: ({ boundaries }) => {
          const store = boundaries.length === HISTORY_PAGE_DAYS + 1 ? daily : hourly;
          const pending = store.get(boundaries[0]);
          if (!pending) throw new Error("unexpected activity request");
          return pending.promise;
        },
        getBreakRange: ({ startMs }) => {
          const pending = breaks.get(startMs);
          if (!pending) throw new Error("unexpected break request");
          return pending.promise;
        }
      },
      (page) => {
        applied.push(page.pageIndex);
      },
      (error) => {
        failures.push(error instanceof Error ? error.message : String(error));
      }
    );

    for (const request of [older, newer]) {
      daily.set(request.startMs, deferred());
      hourly.set(request.startMs, deferred());
      breaks.set(request.startMs, deferred());
    }

    const olderLoad = loadPage(older);
    const newerLoad = loadPage(newer);

    daily.get(newer.startMs)?.resolve(buckets(newer.days.length));
    hourly
      .get(newer.startMs)
      ?.resolve(buckets(newer.hourBoundariesMs.length - 1));
    breaks.get(newer.startMs)?.resolve([]);
    await expect(newerLoad).resolves.toBe("applied");

    daily.get(older.startMs)?.reject(new Error("native path /too/long"));
    await expect(olderLoad).resolves.toBe("stale");

    expect(applied).toEqual([0]);
    expect(failures).toEqual([]);
  });
});
