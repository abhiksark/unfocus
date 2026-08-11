/**
 * Rolling-window continuous activity and AFK summary from the native idle probe.
 * Observe-only: no keylogging and no effect on the break timer.
 */

export type ActivityKind = "active" | "afk" | "unknown";

export type StripBucket = {
  activeRatio: number;
  afkRatio: number;
};

export type TodayActivity = {
  windowLabel: string;
  windowSeconds: number;
  activeSeconds: number;
  afkSeconds: number;
  unknownSeconds: number;
  longestActiveSeconds: number;
  deepBlockCount: number;
  deepBlockMinSeconds: number;
  afkThresholdSeconds: number;
  currentKind: ActivityKind | null;
  probeAvailable: boolean;
  strip: StripBucket[];
};

const SECONDS_PER_MINUTE = 60;
const SECONDS_PER_HOUR = 60 * SECONDS_PER_MINUTE;

/** Compact duration for dashboard stats (e.g. "4h 12m", "47m", "<1m"). */
export function formatActivityDuration(totalSeconds: number): string {
  if (!Number.isFinite(totalSeconds) || totalSeconds < 0) return "—";
  const seconds = Math.floor(totalSeconds);
  if (seconds < SECONDS_PER_MINUTE) return "<1m";
  const hours = Math.floor(seconds / SECONDS_PER_HOUR);
  const minutes = Math.floor((seconds % SECONDS_PER_HOUR) / SECONDS_PER_MINUTE);
  if (hours === 0) return `${minutes}m`;
  if (minutes === 0) return `${hours}h`;
  return `${hours}h ${minutes}m`;
}

export function currentKindLabel(kind: ActivityKind | null, probeAvailable: boolean): string {
  if (!probeAvailable || kind === null || kind === "unknown") {
    return "Idle probe unavailable";
  }
  if (kind === "active") return "At the keyboard";
  return "Away";
}

export function deepBlockCaption(count: number, minSeconds: number): string {
  const threshold = formatActivityDuration(minSeconds);
  if (count === 0) return `No deep blocks yet (≥${threshold})`;
  if (count === 1) return `1 deep block (≥${threshold})`;
  return `${count} deep blocks (≥${threshold})`;
}

/**
 * Accessible description of the strip: active vs AFK presence across buckets.
 */
export function stripAriaLabel(activity: Pick<TodayActivity, "strip" | "windowLabel">): string {
  const buckets = activity.strip;
  if (buckets.length === 0) {
    return `${activity.windowLabel}: no activity samples yet`;
  }
  const activeBuckets = buckets.filter((bucket) => bucket.activeRatio >= 0.5).length;
  const afkBuckets = buckets.filter(
    (bucket) => bucket.afkRatio >= 0.5 && bucket.activeRatio < 0.5
  ).length;
  return `${activity.windowLabel}: ${activeBuckets} mostly-active and ${afkBuckets} mostly-away half-hour blocks`;
}

/** Height factor 0–1 for the active bar in a strip bucket. */
export function stripActiveHeight(bucket: StripBucket): number {
  if (!Number.isFinite(bucket.activeRatio)) return 0;
  return Math.min(1, Math.max(0, bucket.activeRatio));
}

/** Height factor 0–1 for the AFK bar in a strip bucket. */
export function stripAfkHeight(bucket: StripBucket): number {
  if (!Number.isFinite(bucket.afkRatio)) return 0;
  return Math.min(1, Math.max(0, bucket.afkRatio));
}
