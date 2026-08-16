import {
  breakOutcomeStats,
  type BreakOutcomeKind,
  type BreakSummary
} from "./break-summary";
import { DEFAULT_DAY_START_HOUR } from "./day-start";
import { formatActivityDuration } from "./today-activity";

export const HISTORY_PAGE_DAYS = 30;
export const HISTORY_MAX_DAYS = 90;
export const HISTORY_PAGE_COUNT = HISTORY_MAX_DAYS / HISTORY_PAGE_DAYS;

const HOURS_PER_DAY = 24;
const MILLIS_PER_HOUR = 60 * 60 * 1_000;

export type ActivityRangeBucket = {
  activeMs: number;
  afkMs: number;
  longestActiveMs: number;
};

export type BreakHistoryEvent = {
  atMs: number;
  kind: BreakOutcomeKind;
};

export type HistoryActivityKind = "blank" | "active" | "afk" | "mixed";

export type HistoryBreakCount = {
  kind: BreakOutcomeKind;
  label: string;
  count: number;
};

export type HistoryTotals = {
  activeMs: number;
  afkMs: number;
  longestActiveMs: number;
  activeLabel: string;
  afkLabel: string;
  longestLabel: string;
  isBlank: boolean;
};

export type HistoryHourSlotRequest = {
  slotIndex: number;
  wallHour: number;
  label: string;
  bucketIndexes: number[];
};

export type HistoryDayRequest = {
  index: number;
  dateKey: string;
  label: string;
  startMs: number;
  endMs: number;
  hourSlots: HistoryHourSlotRequest[];
};

export type HistoryPageRequest = {
  pageIndex: number;
  dayStartHour: number;
  isCurrentPage: boolean;
  startMs: number;
  endMs: number;
  dayBoundariesMs: number[];
  hourBoundariesMs: number[];
  days: HistoryDayRequest[];
};

export type HistoryHourSlot = HistoryHourSlotRequest & {
  activeMs: number;
  afkMs: number;
  longestActiveMs: number;
  kind: HistoryActivityKind;
  breakMarkers: BreakHistoryEvent[];
};

export type HistoryDay = Omit<HistoryDayRequest, "hourSlots"> & {
  totals: HistoryTotals;
  breakCounts: HistoryBreakCount[];
  hourSlots: HistoryHourSlot[];
};

export type HistoryPage = Omit<HistoryPageRequest, "days"> & {
  totals: HistoryTotals;
  breakCounts: HistoryBreakCount[];
  days: HistoryDay[];
};

const EMPTY_BREAK_SUMMARY: BreakSummary = {
  windowLabel: "",
  windowSeconds: 0,
  scheduledShown: 0,
  naturalIdle: 0,
  fullscreenSuppress: 0,
  manualTakeBreak: 0,
  weekScheduledShown: 0,
  weekNaturalIdle: 0,
  weekFullscreenSuppress: 0,
  weekManualTakeBreak: 0
};

const BREAK_COUNT_TEMPLATE = breakOutcomeStats(EMPTY_BREAK_SUMMARY).map(({ kind, label }) => ({
  kind,
  label
}));

function normalizeDayStartHour(dayStartHour: number | null | undefined): number {
  if (
    typeof dayStartHour === "number" &&
    Number.isInteger(dayStartHour) &&
    dayStartHour >= 0 &&
    dayStartHour < HOURS_PER_DAY
  ) {
    return dayStartHour;
  }
  return DEFAULT_DAY_START_HOUR;
}

function requireFiniteMs(value: number, name: string): number {
  if (!Number.isFinite(value)) {
    throw new RangeError(`${name} must be a finite epoch-millisecond timestamp`);
  }
  return value;
}

function shiftLocalDay(startMs: number, deltaDays: number, dayStartHour: number): number {
  const next = new Date(startMs);
  next.setDate(next.getDate() + deltaDays);
  next.setHours(dayStartHour, 0, 0, 0);
  return next.getTime();
}

function dayKey(timestampMs: number): string {
  const at = new Date(timestampMs);
  const year = at.getFullYear();
  const month = String(at.getMonth() + 1).padStart(2, "0");
  const day = String(at.getDate()).padStart(2, "0");
  return `${year}-${month}-${day}`;
}

function formatDayLabel(timestampMs: number): string {
  return new Intl.DateTimeFormat([], {
    weekday: "short",
    month: "short",
    day: "numeric"
  }).format(new Date(timestampMs));
}

function formatHourLabel(wallHour: number): string {
  return new Intl.DateTimeFormat([], { hour: "numeric" }).format(
    new Date(2026, 0, 15, wallHour, 0, 0, 0)
  );
}

function slotIndexForWallHour(wallHour: number, dayStartHour: number): number {
  return (wallHour - dayStartHour + HOURS_PER_DAY) % HOURS_PER_DAY;
}

function sanitizeMs(value: number): number {
  if (!Number.isFinite(value) || value <= 0) return 0;
  return Math.floor(value);
}

function sanitizeBucket(bucket: ActivityRangeBucket | undefined): ActivityRangeBucket {
  return {
    activeMs: sanitizeMs(bucket?.activeMs ?? 0),
    afkMs: sanitizeMs(bucket?.afkMs ?? 0),
    longestActiveMs: sanitizeMs(bucket?.longestActiveMs ?? 0)
  };
}

function formatHistoryDuration(milliseconds: number): string {
  return milliseconds === 0 ? "—" : formatActivityDuration(milliseconds / 1_000);
}

function totalsFromBuckets(buckets: ActivityRangeBucket[]): HistoryTotals {
  const activeMs = buckets.reduce((total, bucket) => total + sanitizeMs(bucket.activeMs), 0);
  const afkMs = buckets.reduce((total, bucket) => total + sanitizeMs(bucket.afkMs), 0);
  const longestActiveMs = buckets.reduce(
    (longest, bucket) => Math.max(longest, sanitizeMs(bucket.longestActiveMs)),
    0
  );
  return {
    activeMs,
    afkMs,
    longestActiveMs,
    activeLabel: formatHistoryDuration(activeMs),
    afkLabel: formatHistoryDuration(afkMs),
    longestLabel: formatHistoryDuration(longestActiveMs),
    isBlank: activeMs === 0 && afkMs === 0
  };
}

function activityKind(activeMs: number, afkMs: number): HistoryActivityKind {
  if (activeMs === 0 && afkMs === 0) return "blank";
  if (activeMs > 0 && afkMs === 0) return "active";
  if (activeMs === 0 && afkMs > 0) return "afk";
  return "mixed";
}

function breakCounts(events: BreakHistoryEvent[]): HistoryBreakCount[] {
  return BREAK_COUNT_TEMPLATE.map(({ kind, label }) => ({
    kind,
    label,
    count: events.filter((event) => event.kind === kind).length
  }));
}

export function historyDayBoundsAt(
  timestampMs: number,
  dayStartHour: number = DEFAULT_DAY_START_HOUR
): { startMs: number; endMs: number } {
  const atMs = requireFiniteMs(timestampMs, "timestampMs");
  const normalizedDayStart = normalizeDayStartHour(dayStartHour);
  const start = new Date(atMs);
  start.setHours(normalizedDayStart, 0, 0, 0);
  if (start.getTime() > atMs) {
    start.setDate(start.getDate() - 1);
    start.setHours(normalizedDayStart, 0, 0, 0);
  }
  const startMs = start.getTime();
  return {
    startMs,
    endMs: shiftLocalDay(startMs, 1, normalizedDayStart)
  };
}

export function buildHistoryPageRequest(
  pageIndex: number,
  nowMs: number,
  dayStartHour: number = DEFAULT_DAY_START_HOUR
): HistoryPageRequest {
  const normalizedDayStart = normalizeDayStartHour(dayStartHour);
  const currentNowMs = requireFiniteMs(nowMs, "nowMs");
  if (!Number.isInteger(pageIndex) || pageIndex < 0 || pageIndex >= HISTORY_PAGE_COUNT) {
    throw new RangeError(`pageIndex must be an integer between 0 and ${HISTORY_PAGE_COUNT - 1}`);
  }

  const currentDay = historyDayBoundsAt(currentNowMs, normalizedDayStart);
  const currentPageDayCount =
    currentNowMs === currentDay.startMs ? HISTORY_PAGE_DAYS : HISTORY_PAGE_DAYS - 1;
  const currentPageStart = shiftLocalDay(
    currentDay.startMs,
    -currentPageDayCount,
    normalizedDayStart
  );
  const startMs = shiftLocalDay(
    currentPageStart,
    -HISTORY_PAGE_DAYS * pageIndex,
    normalizedDayStart
  );
  const endMs =
    pageIndex === 0
      ? currentNowMs
      : shiftLocalDay(
          currentPageStart,
          -HISTORY_PAGE_DAYS * (pageIndex - 1),
          normalizedDayStart
        );

  const dayBoundariesMs: number[] = [startMs];
  const hourBoundariesMs: number[] = [startMs];
  const days: HistoryDayRequest[] = [];
  let dayStartMs = startMs;

  for (let index = 0; index < HISTORY_PAGE_DAYS; index += 1) {
    const nextDayStartMs =
      index === HISTORY_PAGE_DAYS - 1
        ? endMs
        : shiftLocalDay(dayStartMs, 1, normalizedDayStart);
    dayBoundariesMs.push(nextDayStartMs);

    const hourSlots: HistoryHourSlotRequest[] = Array.from({ length: HOURS_PER_DAY }, (_, slotIndex) => {
      const wallHour = (normalizedDayStart + slotIndex) % HOURS_PER_DAY;
      return {
        slotIndex,
        wallHour,
        label: formatHourLabel(wallHour),
        bucketIndexes: []
      };
    });

    let hourStartMs = dayStartMs;
    while (hourStartMs < nextDayStartMs) {
      const bucketIndex = hourBoundariesMs.length - 1;
      const nextWallHour = new Date(hourStartMs);
      nextWallHour.setHours(nextWallHour.getHours() + 1, 0, 0, 0);
      const hourEndMs = Math.min(
        nextDayStartMs,
        hourStartMs + MILLIS_PER_HOUR,
        nextWallHour.getTime()
      );
      const wallHour = new Date(hourStartMs).getHours();
      hourSlots[slotIndexForWallHour(wallHour, normalizedDayStart)].bucketIndexes.push(
        bucketIndex
      );
      hourBoundariesMs.push(hourEndMs);
      hourStartMs = hourEndMs;
    }

    days.push({
      index,
      dateKey: dayKey(dayStartMs),
      label: formatDayLabel(dayStartMs),
      startMs: dayStartMs,
      endMs: nextDayStartMs,
      hourSlots
    });
    dayStartMs = nextDayStartMs;
  }

  return {
    pageIndex,
    dayStartHour: normalizedDayStart,
    isCurrentPage: pageIndex === 0,
    startMs,
    endMs,
    dayBoundariesMs,
    hourBoundariesMs,
    days
  };
}

export function buildHistoryPages(
  nowMs: number,
  dayStartHour: number = DEFAULT_DAY_START_HOUR
): HistoryPageRequest[] {
  return Array.from({ length: HISTORY_PAGE_COUNT }, (_, pageIndex) =>
    buildHistoryPageRequest(pageIndex, nowMs, dayStartHour)
  );
}

export function materializeHistoryPage(
  request: HistoryPageRequest,
  dailyBuckets: ActivityRangeBucket[],
  hourlyBuckets: ActivityRangeBucket[],
  breakEvents: BreakHistoryEvent[]
): HistoryPage {
  if (dailyBuckets.length !== request.days.length) {
    throw new RangeError(
      `dailyBuckets length ${dailyBuckets.length} does not match page days ${request.days.length}`
    );
  }
  if (hourlyBuckets.length !== request.hourBoundariesMs.length - 1) {
    throw new RangeError(
      `hourlyBuckets length ${hourlyBuckets.length} does not match hour buckets ${
        request.hourBoundariesMs.length - 1
      }`
    );
  }

  const filteredEvents = breakEvents
    .filter(
      (event) =>
        Number.isFinite(event.atMs) && event.atMs >= request.startMs && event.atMs < request.endMs
    )
    .slice()
    .sort((left, right) => left.atMs - right.atMs);

  const days = request.days.map((day, index) => {
    const dayEvents = filteredEvents.filter(
      (event) => event.atMs >= day.startMs && event.atMs < day.endMs
    );
    const totals = totalsFromBuckets([sanitizeBucket(dailyBuckets[index])]);
    const hourSlots = day.hourSlots.map((slot) => {
      const slotBuckets = slot.bucketIndexes.map((bucketIndex) => sanitizeBucket(hourlyBuckets[bucketIndex]));
      const activeMs = slotBuckets.reduce((total, bucket) => total + bucket.activeMs, 0);
      const afkMs = slotBuckets.reduce((total, bucket) => total + bucket.afkMs, 0);
      const longestActiveMs = slotBuckets.reduce(
        (longest, bucket) => Math.max(longest, bucket.longestActiveMs),
        0
      );
      return {
        ...slot,
        activeMs,
        afkMs,
        longestActiveMs,
        kind: activityKind(activeMs, afkMs),
        breakMarkers: dayEvents.filter(
          (event) =>
            slotIndexForWallHour(new Date(event.atMs).getHours(), request.dayStartHour) ===
            slot.slotIndex
        )
      };
    });

    return {
      ...day,
      totals,
      breakCounts: breakCounts(dayEvents),
      hourSlots
    };
  });

  return {
    ...request,
    totals: totalsFromBuckets(dailyBuckets.map((bucket) => sanitizeBucket(bucket))),
    breakCounts: breakCounts(filteredEvents),
    days
  };
}
