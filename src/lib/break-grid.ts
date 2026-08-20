/**
 * Break-grid helpers mirroring `src-tauri/src/reminder/schedule.rs`.
 *
 * The Rust timer and this module compute the same grid independently, the same
 * way the overlay window label is parsed on both sides. The shared test table
 * in `break-grid.test.ts` is what keeps them from drifting; change both
 * together.
 *
 * The grid is anchored to the local-time epoch — `unixSecs + offsetMinutes*60`
 * — not to local midnight. The two coincide only when `intervalSecs` divides
 * 86400 (true for the default 20-minute work interval, false for e.g. 25
 * minutes, where the grid slides 900s per day). Mirror the Rust arithmetic
 * exactly; do not "simplify" this into a seconds-since-midnight calculation.
 */

const MINUTES_PER_HOUR = 60;
const UPCOMING_COUNT = 3;

/** Euclidean remainder, so offsets west of UTC stay correct. */
function remEuclid(value: number, modulus: number): number {
  return ((value % modulus) + modulus) % modulus;
}

/** The device's UTC offset in minutes, positive east of UTC. */
export function deviceGridOffsetMinutes(date: Date): number {
  // getTimezoneOffset is inverted: IST reports -330, and we store +330.
  return -date.getTimezoneOffset();
}

/** The next grid point strictly after `unixSecs`. */
export function nextGrid(unixSecs: number, intervalSecs: number, offsetMinutes: number): number {
  const localSeconds = unixSecs + offsetMinutes * 60;
  const phase = remEuclid(localSeconds, intervalSecs);
  return unixSecs + (intervalSecs - phase);
}

/** The stored offset in a form two people can compare across devices. */
export function formatGridOffset(minutes: number): string {
  if (minutes === 0) return "UTC";
  const sign = minutes < 0 ? "-" : "+";
  const absolute = Math.abs(minutes);
  const hours = String(Math.floor(absolute / 60)).padStart(2, "0");
  const rest = String(absolute % 60).padStart(2, "0");
  return `UTC${sign}${hours}:${rest}`;
}

export type GridPreview =
  | { kind: "hourly"; minutes: number[] }
  | { kind: "upcoming"; atMs: number[] };

/**
 * How to describe the grid to the reader.
 *
 * An interval dividing an hour repeats every hour, so it is shown as minutes
 * past the hour — stable whenever each device is looked at, which is what makes
 * two devices comparable by eye. Anything else has no hourly repeat, so the
 * next few absolute times are shown instead of implying a pattern.
 */
export function gridPreview(
  nowMs: number,
  workMinutes: number,
  gridOffsetMinutes: number
): GridPreview {
  const intervalSecs = workMinutes * 60;

  if (MINUTES_PER_HOUR % workMinutes === 0) {
    // An interval dividing an hour always lands on local minutes that are
    // multiples of itself, whatever the offset — that is the tidy-times
    // guarantee. It also means this pattern alone cannot reveal an offset
    // mismatch, so callers must display formatGridOffset alongside it.
    const minutes: number[] = [];
    for (let minute = 0; minute < MINUTES_PER_HOUR; minute += workMinutes) {
      minutes.push(minute);
    }
    return { kind: "hourly", minutes };
  }

  const atMs: number[] = [];
  let cursor = Math.floor(nowMs / 1000);
  for (let index = 0; index < UPCOMING_COUNT; index += 1) {
    cursor = nextGrid(cursor, intervalSecs, gridOffsetMinutes);
    atMs.push(cursor * 1000);
  }
  return { kind: "upcoming", atMs };
}
