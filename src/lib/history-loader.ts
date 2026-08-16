import {
  materializeHistoryPage,
  type ActivityRangeBucket,
  type BreakHistoryEvent,
  type HistoryPage,
  type HistoryPageRequest
} from "./history";

export type HistoryPageFetcher = {
  getActivityRange: (request: { boundaries: number[] }) => Promise<ActivityRangeBucket[]>;
  getBreakRange: (request: { startMs: number; endMs: number }) => Promise<BreakHistoryEvent[]>;
};

export type HistoryLoadResult = "applied" | "failed" | "stale";

export function createHistoryPageLoader(
  fetcher: HistoryPageFetcher,
  apply: (page: HistoryPage) => void,
  fail: (error: unknown) => void
): (request: HistoryPageRequest) => Promise<HistoryLoadResult> {
  let generation = 0;

  return async (request) => {
    const loadGeneration = (generation += 1);
    try {
      const [daily, hourly, breaks] = await Promise.all([
        fetcher.getActivityRange({ boundaries: request.dayBoundariesMs }),
        fetcher.getActivityRange({ boundaries: request.hourBoundariesMs }),
        fetcher.getBreakRange({ startMs: request.startMs, endMs: request.endMs })
      ]);
      if (loadGeneration !== generation) return "stale";
      apply(materializeHistoryPage(request, daily, hourly, breaks));
      return "applied";
    } catch (error) {
      if (loadGeneration !== generation) return "stale";
      fail(error);
      return "failed";
    }
  };
}
