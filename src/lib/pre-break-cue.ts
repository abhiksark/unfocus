// src/lib/pre-break-cue.ts

export const PRE_BREAK_CUE_LEAD_MS = 60_000;
export const PRE_BREAK_CUE_HEADS_UP_MS = 4_000;
export const PRE_BREAK_CUE_COUNTDOWN_MS = 10_000;
export const PRE_BREAK_CUE_PREVIEW_MS = 17_000;
const PRE_BREAK_CUE_PREVIEW_QUIET_MS = 2_000;
const PRE_BREAK_CUE_PREVIEW_HANDOFF_MS = 1_000;

export type PreBreakCueMode = "scheduled" | "preview";
export type PreBreakCuePlatform = "macos" | "linux";
export type PreBreakCueStage = "heads-up" | "quiet" | "horizon" | "handoff";

declare global {
  interface Window {
    readonly __UNFOCUS_PRE_BREAK_CUE_PLATFORM__?: PreBreakCuePlatform;
  }
}

export type PreBreakCuePresentation = {
  stage: PreBreakCueStage;
  secondsLeft: number;
  visible: boolean;
};

export type PreBreakCueLayout = {
  isNotched: boolean;
  notchWidth: number;
  wingWidth: number;
  shoulderWidth: number;
  height: number;
};

export type PreBreakCueView = {
  surface: "notch" | "card";
  title: string | null;
  support: string | null;
  countdown: number | null;
};

export function preBreakCuePlatformFromNativeContext(value: unknown): PreBreakCuePlatform {
  return value === "macos" ? "macos" : "linux";
}

export function preBreakCueViewFromPresentation(
  platform: PreBreakCuePlatform,
  presentation: PreBreakCuePresentation,
  mode: PreBreakCueMode = "scheduled"
): PreBreakCueView {
  if (platform === "linux") {
    const isHandoff = presentation.stage === "handoff";
    return {
      surface: "card",
      title:
        presentation.stage === "horizon"
          ? "Eye break"
          : isHandoff
            ? "Look away"
            : "Eye break in 1 minute",
      support: isHandoff ? "Rest your focus beyond the screen." : "Finish your thought.",
      countdown: presentation.stage === "horizon" ? presentation.secondsLeft : null
    };
  }

  return {
    surface: "notch",
    title: presentation.stage === "heads-up" ? "Break in 1m" :
      presentation.stage === "handoff" ? "Look away" : null,
    support: null,
    countdown: presentation.stage === "horizon"
      ? Math.max(1, Math.min(10, presentation.secondsLeft - (mode === "preview" ? 1 : 0)))
      : null
  };
}

export function preBreakCuePresentationFromRemaining(
  remainingMs: number,
  mode: PreBreakCueMode = "scheduled"
): PreBreakCuePresentation {
  const boundedRemainingMs = Math.max(0, remainingMs);
  const secondsLeft = Math.ceil(boundedRemainingMs / 1_000);
  const leadMs = mode === "preview" ? PRE_BREAK_CUE_PREVIEW_MS : PRE_BREAK_CUE_LEAD_MS;
  const headsUpEndsAtMs = leadMs - PRE_BREAK_CUE_HEADS_UP_MS;
  const horizonStartsAtMs =
    mode === "preview"
      ? headsUpEndsAtMs - PRE_BREAK_CUE_PREVIEW_QUIET_MS
      : PRE_BREAK_CUE_COUNTDOWN_MS;
  const handoffStartsAtMs = mode === "preview" ? PRE_BREAK_CUE_PREVIEW_HANDOFF_MS : 0;
  const stage: PreBreakCueStage =
    boundedRemainingMs > leadMs
      ? "quiet"
      : boundedRemainingMs > headsUpEndsAtMs
        ? "heads-up"
        : boundedRemainingMs > horizonStartsAtMs
          ? "quiet"
          : boundedRemainingMs > handoffStartsAtMs
            ? "horizon"
            : "handoff";

  return {
    stage,
    secondsLeft,
    visible: stage !== "quiet"
  };
}

export function preBreakCuePresentation(
  deadlineMs: number,
  nowMs: number,
  mode: PreBreakCueMode = "scheduled"
): PreBreakCuePresentation {
  return preBreakCuePresentationFromRemaining(deadlineMs - nowMs, mode);
}

export function preBreakCueAnimationDelayMs(
  remainingMs: number,
  mode: PreBreakCueMode
): number {
  const boundedRemainingMs = Math.max(0, remainingMs);
  const stage = preBreakCuePresentationFromRemaining(boundedRemainingMs, mode).stage;
  if (stage === "heads-up") {
    const headsUpStartsAtMs = mode === "preview" ? PRE_BREAK_CUE_PREVIEW_MS : PRE_BREAK_CUE_LEAD_MS;
    return boundedRemainingMs - headsUpStartsAtMs;
  }
  if (stage === "horizon") {
    const horizonStartsAtMs =
      mode === "preview"
        ? PRE_BREAK_CUE_PREVIEW_MS - PRE_BREAK_CUE_HEADS_UP_MS - PRE_BREAK_CUE_PREVIEW_QUIET_MS
        : PRE_BREAK_CUE_COUNTDOWN_MS;
    return boundedRemainingMs - horizonStartsAtMs;
  }
  return 0;
}

/** Ring depletion follows the same deadline as the digits, including preview handoff. */
export function preBreakCueRingProgress(remainingMs: number, mode: PreBreakCueMode): number {
  const stage = preBreakCuePresentationFromRemaining(remainingMs, mode).stage;
  if (stage === "heads-up" || stage === "quiet") return 1;
  const handoffMs = mode === "preview" ? PRE_BREAK_CUE_PREVIEW_HANDOFF_MS : 0;
  return Math.max(0, Math.min(1, (remainingMs - handoffMs) / PRE_BREAK_CUE_COUNTDOWN_MS));
}

/** The target stays fixed; don't accept clicks while the wings cross it. */
export function preBreakCueSkipAvailable(remainingMs: number, mode: PreBreakCueMode): boolean {
  const stage = preBreakCuePresentationFromRemaining(remainingMs, mode).stage;
  const elapsed = -preBreakCueAnimationDelayMs(remainingMs, mode);
  if (stage === "heads-up") return elapsed >= 350 && elapsed < 3_650;
  return stage === "horizon" && elapsed >= 350;
}
