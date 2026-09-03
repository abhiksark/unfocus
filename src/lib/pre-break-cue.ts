// src/lib/pre-break-cue.ts

export const PRE_BREAK_CUE_LEAD_MS = 60_000;
export const PRE_BREAK_CUE_HEADS_UP_MS = 4_000;
export const PRE_BREAK_CUE_COUNTDOWN_MS = 10_000;

export type PreBreakCueStage = "heads-up" | "quiet" | "countdown" | "handoff";

export type PreBreakCuePresentation = {
  stage: PreBreakCueStage;
  secondsLeft: number;
  visible: boolean;
};

export function preBreakCuePresentationFromRemaining(
  remainingMs: number
): PreBreakCuePresentation {
  const boundedRemainingMs = Math.max(0, remainingMs);
  const secondsLeft = Math.ceil(boundedRemainingMs / 1_000);
  const headsUpEndsAtMs = PRE_BREAK_CUE_LEAD_MS - PRE_BREAK_CUE_HEADS_UP_MS;
  const stage: PreBreakCueStage =
    boundedRemainingMs > PRE_BREAK_CUE_LEAD_MS
      ? "quiet"
      : boundedRemainingMs > headsUpEndsAtMs
        ? "heads-up"
        : boundedRemainingMs > PRE_BREAK_CUE_COUNTDOWN_MS
          ? "quiet"
          : boundedRemainingMs > 0
            ? "countdown"
            : "handoff";

  return {
    stage,
    secondsLeft,
    visible: stage !== "quiet"
  };
}

export function preBreakCuePresentation(
  deadlineMs: number,
  nowMs: number
): PreBreakCuePresentation {
  return preBreakCuePresentationFromRemaining(deadlineMs - nowMs);
}
