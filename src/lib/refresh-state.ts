export type RefreshState<T> =
  | { status: "loading"; data: null; error: null; asOfMs: null }
  | { status: "fresh"; data: T; error: null; asOfMs: number }
  | { status: "stale"; data: T; error: string; asOfMs: number }
  | { status: "unavailable"; data: null; error: string; asOfMs: null };

export type RefreshStatus = RefreshState<unknown>["status"];

export function refreshLoading<T>(): RefreshState<T> {
  return { status: "loading", data: null, error: null, asOfMs: null };
}

export function refreshSucceeded<T>(data: T, asOfMs = Date.now()): RefreshState<T> {
  return { status: "fresh", data, error: null, asOfMs };
}

/**
 * Retain the exact last successful payload and capture time when a later
 * transport refresh fails. A resource without successful data remains
 * unavailable instead of acquiring an invented empty value.
 */
export function refreshFailed<T>(
  previous: RefreshState<T>,
  error: string
): RefreshState<T> {
  if (previous.status === "fresh" || previous.status === "stale") {
    return {
      status: "stale",
      data: previous.data,
      error,
      asOfMs: previous.asOfMs
    };
  }
  return { status: "unavailable", data: null, error, asOfMs: null };
}

/** Native storage-unavailable envelopes override retained transport data. */
export function refreshUnavailable<T>(error: string): RefreshState<T> {
  return { status: "unavailable", data: null, error, asOfMs: null };
}

/** Stale visualizations stay anchored to the payload's successful capture. */
export function refreshDisplayAsOfMs(
  state: RefreshState<unknown>,
  liveNowMs: number
): number {
  return state.status === "stale" ? state.asOfMs : liveNowMs;
}
