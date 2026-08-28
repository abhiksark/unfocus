// src/lib/pre-break-cue.ts

export type PreBreakCueStage = "compact" | "countdown" | "due";

export type PreBreakCuePresentation = {
  stage: PreBreakCueStage;
  secondsLeft: number;
  countdownProgress: number;
};

export function preBreakCuePresentationFromRemaining(
  remainingMs: number
): PreBreakCuePresentation {
  const boundedRemainingMs = Math.max(0, remainingMs);
  const secondsLeft = Math.ceil(boundedRemainingMs / 1_000);
  const countdownProgress = Math.min(
    1,
    Math.max(0, (10_000 - boundedRemainingMs) / 10_000)
  );
  return {
    stage:
      boundedRemainingMs === 0
        ? "due"
        : boundedRemainingMs <= 10_000
          ? "countdown"
          : "compact",
    secondsLeft,
    countdownProgress
  };
}

export function preBreakCuePresentation(
  deadlineMs: number,
  nowMs: number
): PreBreakCuePresentation {
  return preBreakCuePresentationFromRemaining(deadlineMs - nowMs);
}
