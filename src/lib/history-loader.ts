import {
  materializeHistoryCalendar,
  materializeHistoryPage,
  type ActivityRangeBucket,
  type BreakHistoryEvent,
  type HistoryCalendar,
  type HistoryCalendarRequest,
  type HistoryDay,
  type HistoryPageRequest
} from "./history";

export type HistoryPageFetcher = {
  getActivityRange: (request: { boundaries: number[] }) => Promise<ActivityRangeBucket[]>;
  getBreakRange: (request: { startMs: number; endMs: number }) => Promise<BreakHistoryEvent[]>;
};

export type HistoryLoadResult = "applied" | "failed" | "stale";

export function createHistoryCalendarLoader(
  fetcher: Pick<HistoryPageFetcher, "getActivityRange">,
  apply: (calendar: HistoryCalendar, request: HistoryCalendarRequest) => void,
  fail: (error: unknown) => void
): (request: HistoryCalendarRequest) => Promise<HistoryLoadResult> {
  let generation = 0;

  return async (request) => {
    const loadGeneration = (generation += 1);
    try {
      const dailyBuckets = await Promise.all(
        request.pages.map((page) =>
          fetcher.getActivityRange({ boundaries: page.dayBoundariesMs })
        )
      );
      if (loadGeneration !== generation) return "stale";
      apply(materializeHistoryCalendar(request, dailyBuckets), request);
      return "applied";
    } catch (error) {
      if (loadGeneration !== generation) return "stale";
      fail(error);
      return "failed";
    }
  };
}

export function createHistoryDayLoader(
  fetcher: HistoryPageFetcher,
  apply: (day: HistoryDay) => void,
  fail: (error: unknown) => void
): (
  request: HistoryPageRequest,
  dailyBucket: ActivityRangeBucket
) => Promise<HistoryLoadResult> {
  let generation = 0;

  return async (request, dailyBucket) => {
    const loadGeneration = (generation += 1);
    try {
      const [hourly, breaks] = await Promise.all([
        fetcher.getActivityRange({ boundaries: request.hourBoundariesMs }),
        fetcher.getBreakRange({ startMs: request.startMs, endMs: request.endMs })
      ]);
      if (loadGeneration !== generation) return "stale";
      apply(materializeHistoryPage(request, [dailyBucket], hourly, breaks).days[0]);
      return "applied";
    } catch (error) {
      if (loadGeneration !== generation) return "stale";
      fail(error);
      return "failed";
    }
  };
}
