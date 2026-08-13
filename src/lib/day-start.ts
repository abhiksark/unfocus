/**
 * When the reader's day begins, as a whole local hour.
 *
 * A display preference for the Your day axis only. It is deliberately kept in
 * browser storage rather than `reminder-settings.json`: the break timer must
 * never read it, and it needs no native validation or schema version.
 */

export const DAY_START_STORAGE_KEY = "unfocus.day-start-hour.v1";

/** Midnight, the conventional boundary and the one a 24-hour strip crosses. */
export const DEFAULT_DAY_START_HOUR = 0;

const HOURS_PER_DAY = 24;

type ReadableStorage = Pick<Storage, "getItem">;
type WritableStorage = Pick<Storage, "setItem">;

function isWholeHour(value: number): boolean {
  return Number.isInteger(value) && value >= 0 && value < HOURS_PER_DAY;
}

export function readDayStartHour(storage: ReadableStorage | null): number {
  if (!storage) return DEFAULT_DAY_START_HOUR;

  try {
    const raw = storage.getItem(DAY_START_STORAGE_KEY);
    // Only the canonical decimal form this module writes is accepted; anything
    // else in the key came from outside and falls back rather than being
    // reinterpreted (Number("1e1") would otherwise read as hour 10).
    if (raw === null || !/^\d{1,2}$/.test(raw.trim())) return DEFAULT_DAY_START_HOUR;
    const hour = Number(raw.trim());
    return isWholeHour(hour) ? hour : DEFAULT_DAY_START_HOUR;
  } catch {
    return DEFAULT_DAY_START_HOUR;
  }
}

export function writeDayStartHour(storage: WritableStorage | null, hour: number): boolean {
  if (!storage || !isWholeHour(hour)) return false;

  try {
    storage.setItem(DAY_START_STORAGE_KEY, String(hour));
    return true;
  } catch {
    return false;
  }
}

/** Every whole hour, labeled in the reader's locale for the day-start control. */
export function dayStartOptions(): { hour: number; label: string }[] {
  const hourLabel = new Intl.DateTimeFormat([], { hour: "numeric", minute: "2-digit" });
  return Array.from({ length: HOURS_PER_DAY }, (_, hour) => ({
    hour,
    label: hourLabel.format(new Date(2026, 0, 15, hour, 0, 0, 0))
  }));
}
