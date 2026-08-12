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

/**
 * Live status under the Your day title.
 * Distinct for probe-down vs unknown classification vs active/away.
 */
export function currentKindLabel(kind: ActivityKind | null, probeAvailable: boolean): string {
  if (!probeAvailable) return "Presence probe unavailable";
  if (kind === null || kind === "unknown") return "Waiting for presence samples";
  if (kind === "active") return "At the keyboard";
  return "Away from the keyboard";
}

/**
 * Secondary line under the deep-work count.
 * Does not repeat the count already shown as the primary figure.
 */
export function deepBlockCaption(count: number, minSeconds: number): string {
  const threshold = formatActivityDuration(minSeconds);
  if (count === 0) return `None yet · ≥${threshold} continuous`;
  return `≥${threshold} continuous`;
}

/** Privacy and threshold footnote under the strip. */
export function activityFootnote(afkThresholdSeconds: number): string {
  return `Keyboard and mouse presence only · gaps under ${formatActivityDuration(
    afkThresholdSeconds
  )} stay in continuous work · nothing is keylogged`;
}

/** Loading copy before the first native summary arrives. */
export function todayLoadingCaption(): string {
  return "Collecting presence samples from this session…";
}

/**
 * Calm error copy. Prefer a short status; keep a technical detail only when it
 * adds something the user can act on (not a raw dump of every internal path).
 */
export function todayErrorCaption(error: string | null): string {
  if (!error || !error.trim()) {
    return "Your day is unavailable right now. The break timer is unaffected.";
  }
  const trimmed = error.trim();
  // Surface short native messages; clamp long stack-like strings.
  if (trimmed.length <= 160) {
    return `${trimmed} The break timer is unaffected.`;
  }
  return "Your day is unavailable right now. The break timer is unaffected.";
}

/**
 * True when the window has no classified active or away time yet.
 * Unknown-only windows still show the card structure with empty-feeling totals.
 */
export function isActivityWindowEmpty(
  activity: Pick<TodayActivity, "activeSeconds" | "afkSeconds">
): boolean {
  return activity.activeSeconds <= 0 && activity.afkSeconds <= 0;
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

/** Hours between labeled ticks on the strip's time axis. */
const AXIS_HOUR_STEP = 4;
/** Above this percent a label would collide with the "now" anchor. */
const AXIS_LABEL_LIMIT = 94;
/** Defensive bound so an absurd window cannot spin the hour walk. */
const AXIS_MAX_STEPS = 400;

/** One labeled hour on the activity strip's time axis. */
export type StripAxisTick = {
  /** Epoch milliseconds of the labeled hour. */
  timestampMs: number;
  /** Position across the strip, 0-100, measured from the window start. */
  positionPercent: number;
  /** Localized hour, e.g. "4 PM" or "16". */
  label: string;
  /** False when the label is suppressed so it cannot collide with "now". */
  showLabel: boolean;
};

/**
 * Round-hour ticks across the rolling strip window, newest at 100%.
 *
 * Walks local wall-clock hours rather than fixed millisecond offsets so the
 * labels stay on the hour across daylight-saving transitions, when the window
 * holds 23 or 25 hours.
 */
export function stripAxisTicks(windowSeconds: number, nowMs: number): StripAxisTick[] {
  if (!Number.isFinite(windowSeconds) || windowSeconds <= 0) return [];
  if (!Number.isFinite(nowMs)) return [];

  const windowMs = windowSeconds * 1_000;
  const startMs = nowMs - windowMs;
  const hourLabel = new Intl.DateTimeFormat([], { hour: "numeric" });
  const ticks: StripAxisTick[] = [];

  const cursor = new Date(startMs);
  cursor.setMinutes(0, 0, 0);
  if (cursor.getTime() < startMs) cursor.setHours(cursor.getHours() + 1);

  for (let step = 0; step < AXIS_MAX_STEPS && cursor.getTime() < nowMs; step += 1) {
    if (cursor.getHours() % AXIS_HOUR_STEP === 0) {
      const timestampMs = cursor.getTime();
      const positionPercent = ((timestampMs - startMs) / windowMs) * 100;
      ticks.push({
        timestampMs,
        positionPercent,
        label: hourLabel.format(cursor),
        showLabel: positionPercent <= AXIS_LABEL_LIMIT
      });
    }
    cursor.setHours(cursor.getHours() + 1);
  }

  return ticks;
}
