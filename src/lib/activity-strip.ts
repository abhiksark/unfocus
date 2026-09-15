import { breakOutcomeStats, type BreakSummary } from "./break-summary";
import type { BreakHistoryEvent } from "./history";
import { formatActivityDuration, type StripBucket, type TodayActivity } from "./today-activity";

export type ActivityInterval = {
  startMs: number;
  endMs: number;
  active: number;
  away: number;
  unknown: number;
  label: string;
  details: string;
  events: BreakHistoryEvent[];
};

export type AxisLabelBounds = {
  left: number;
  width: number;
  isDayStart: boolean;
};

/** Keep the endpoint clear and prefer the chosen day boundary over nearby ticks. */
export function visibleAxisLabels(
  labels: AxisLabelBounds[],
  availableWidth: number,
  endpointWidth: number,
  gap = 12
): boolean[] {
  const visible = labels.map(() => false);
  const accepted: AxisLabelBounds[] = [];
  const ordered = labels.map((label, index) => ({ ...label, index }))
    .sort((a, b) => Number(b.isDayStart) - Number(a.isDayStart) || a.left - b.left);
  for (const label of ordered) {
    const right = label.left + label.width;
    if (label.left < 0 || right + gap > availableWidth - endpointWidth) continue;
    if (accepted.some((other) => label.left < other.left + other.width + gap && right + gap > other.left)) continue;
    visible[label.index] = true;
    accepted.push(label);
  }
  return visible;
}

function ratio(value: number): number {
  return Number.isFinite(value) ? Math.max(0, Math.min(1, value)) : 0;
}

/** Occupancy shares form one bar; unavailable coverage never becomes away time. */
export function activityShares(bucket: StripBucket) {
  const active = ratio(bucket.activeRatio);
  const away = Math.min(ratio(bucket.afkRatio), 1 - active);
  return { active, away, unknown: Math.max(0, 1 - active - away) };
}

export function activityWindowLabel(windowSeconds: number, endMs: number): string {
  const formatter = new Intl.DateTimeFormat([], {
    month: "short", day: "numeric", hour: "numeric", minute: "2-digit"
  });
  return `${formatter.format(endMs - windowSeconds * 1_000)} – ${formatter.format(endMs)}`;
}

export function activityIntervals(
  activity: Pick<TodayActivity, "strip" | "windowSeconds">,
  endMs: number,
  events: BreakHistoryEvent[] = []
): ActivityInterval[] {
  const startMs = endMs - activity.windowSeconds * 1_000;
  const spanMs = activity.windowSeconds * 1_000 / activity.strip.length;
  const time = new Intl.DateTimeFormat([], { hour: "numeric", minute: "2-digit" });
  return activity.strip.map((bucket, index) => {
    const start = startMs + index * spanMs;
    const end = start + spanMs;
    const shares = activityShares(bucket);
    const label = `${time.format(start)}–${time.format(end)}`;
    const duration = (share: number) => formatActivityDuration(Math.round(share * spanMs / 1_000));
    const intervalEvents = events.filter((event) => event.atMs >= start && event.atMs < end);
    return {
      startMs: start, endMs: end, ...shares, label,
      details: `${duration(shares.active)} active · ${duration(shares.away)} away · ${duration(shares.unknown)} unclassified`,
      events: intervalEvents
    };
  });
}

export function breakEventLabel(event: BreakHistoryEvent): string {
  const labels = {
    scheduledShown: "Scheduled break shown",
    naturalIdle: "Idle when a break was due",
    manualTakeBreak: "Break started by you",
    fullscreenSuppress: "Break held for fullscreen"
  };
  const time = new Intl.DateTimeFormat([], { hour: "numeric", minute: "2-digit" });
  return `${time.format(event.atMs)} · ${labels[event.kind]}`;
}

/** Refresh on a new outcome or every thirty seconds, not every dashboard poll. */
export function breakMarkerRefreshKey(summary: BreakSummary, endMs: number): string {
  return `${Math.floor(endMs / 30_000)}:${breakOutcomeStats(summary).map((stat) => stat.count).join(":")}`;
}

/** Fence late native responses when the window changes, hides, or unmounts. */
export function createBreakMarkerLoader(
  fetcher: (range: { startMs: number; endMs: number }) => Promise<BreakHistoryEvent[]>,
  apply: (events: BreakHistoryEvent[]) => void,
  fail: () => void
) {
  let generation = 0;
  return {
    cancel() { generation += 1; },
    async load(startMs: number, endMs: number) {
      const requested = ++generation;
      try {
        const events = await fetcher({ startMs, endMs });
        if (generation === requested) apply(events);
      } catch {
        if (generation === requested) fail();
      }
    }
  };
}
