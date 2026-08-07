export type OverlayClockAnchor = {
  durationMs: number;
  initialRemainingMs: number;
  monotonicStartedAtMs: number;
};

function clamp(value: number, minimum: number, maximum: number): number {
  return Math.min(maximum, Math.max(minimum, value));
}

export function createOverlayClock(
  durationSeconds: number,
  deadlineMs: number,
  wallNowMs: number,
  monotonicNowMs: number
): OverlayClockAnchor {
  const durationMs = Math.max(1_000, durationSeconds * 1_000);
  return {
    durationMs,
    initialRemainingMs: clamp(deadlineMs - wallNowMs, 0, durationMs),
    monotonicStartedAtMs: monotonicNowMs
  };
}

export function anchorFromRemaining(
  durationMs: number,
  remainingMs: number,
  monotonicNowMs: number
): OverlayClockAnchor {
  return {
    durationMs,
    initialRemainingMs: clamp(remainingMs, 0, durationMs),
    monotonicStartedAtMs: monotonicNowMs
  };
}

export function remainingAt(anchor: OverlayClockAnchor, monotonicNowMs: number): number {
  const elapsedMs = Math.max(0, monotonicNowMs - anchor.monotonicStartedAtMs);
  return clamp(anchor.initialRemainingMs - elapsedMs, 0, anchor.durationMs);
}

export function presentationOffset(anchor: OverlayClockAnchor): number {
  return anchor.durationMs - anchor.initialRemainingMs;
}

export function formatCountdown(totalSeconds: number): string {
  const safeSeconds = Math.max(0, Math.floor(totalSeconds));
  const minutes = Math.floor(safeSeconds / 60);
  const seconds = safeSeconds % 60;
  return `${minutes.toString().padStart(2, "0")}:${seconds.toString().padStart(2, "0")}`;
}
