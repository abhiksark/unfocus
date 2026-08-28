// src/lib/pre-break-cue.ts

export type PreBreakCueStage = "compact" | "countdown" | "due";

export type PreBreakCuePresentation = {
  stage: PreBreakCueStage;
  secondsLeft: number;
};

export function preBreakCuePresentationFromRemaining(
  remainingMs: number
): PreBreakCuePresentation {
  const boundedRemainingMs = Math.max(0, remainingMs);
  const secondsLeft = Math.ceil(boundedRemainingMs / 1_000);
  return {
    stage:
      boundedRemainingMs === 0
        ? "due"
        : boundedRemainingMs <= 10_000
          ? "countdown"
          : "compact",
    secondsLeft
  };
}

export function preBreakCuePresentation(
  deadlineMs: number,
  nowMs: number
): PreBreakCuePresentation {
  return preBreakCuePresentationFromRemaining(deadlineMs - nowMs);
}
