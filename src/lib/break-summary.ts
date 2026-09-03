/**
 * Local break-outcome counts from the native ledger (observe-only, no scores).
 */

import type { RefreshState } from "./refresh-state";

export type BreakSummary = {
  windowLabel: string;
  windowSeconds: number;
  scheduledShown: number;
  naturalIdle: number;
  fullscreenSuppress: number;
  manualTakeBreak: number;
  weekScheduledShown: number;
  weekNaturalIdle: number;
  weekFullscreenSuppress: number;
  weekManualTakeBreak: number;
};

export type BreakOutcomeKind =
  | "scheduledShown"
  | "naturalIdle"
  | "manualTakeBreak"
  | "fullscreenSuppress";

export type BreakOutcomeStat = {
  kind: BreakOutcomeKind;
  label: string;
  count: number;
  /** Short plain-language meaning for title/aria, not a score. */
  hint: string;
};

function dayTotal(summary: BreakSummary): number {
  return (
    summary.scheduledShown +
    summary.naturalIdle +
    summary.manualTakeBreak +
    summary.fullscreenSuppress
  );
}

function weekTotal(summary: BreakSummary): number {
  return (
    summary.weekScheduledShown +
    summary.weekNaturalIdle +
    summary.weekManualTakeBreak +
    summary.weekFullscreenSuppress
  );
}

/** True when the last-day window has no ledger events. */
export function isBreakDayEmpty(summary: BreakSummary): boolean {
  return dayTotal(summary) === 0;
}

/**
 * Ordered day stats for the counts grid.
 * Zeros stay in the list so layout is stable; the UI mutes them.
 */
export function breakOutcomeStats(summary: BreakSummary): BreakOutcomeStat[] {
  return [
    {
      kind: "scheduledShown",
      label: "Scheduled",
      count: summary.scheduledShown,
      hint: "A scheduled break covered every display"
    },
    {
      kind: "naturalIdle",
      label: "Already away",
      count: summary.naturalIdle,
      hint: "You were already away when a break was due"
    },
    {
      kind: "manualTakeBreak",
      label: "Started by you",
      count: summary.manualTakeBreak,
      hint: "You started a break yourself"
    },
    {
      kind: "fullscreenSuppress",
      label: "Held for fullscreen",
      count: summary.fullscreenSuppress,
      hint: "A break waited while an app was fullscreen"
    }
  ];
}

/**
 * Primary day caption under the counts.
 * Empty windows get a calm empty state; non-empty windows get a short
 * observational line without re-listing every count (the grid already shows them).
 */
export function breakSummaryCaption(summary: BreakSummary): string {
  if (isBreakDayEmpty(summary)) {
    return weekTotal(summary) === 0
      ? "No break outcomes in the last seven days."
      : "No break outcomes in the last day.";
  }
  return "Stored only on this device.";
}

/**
 * Quieter week line. Avoids competing with day totals and stays absent when
 * the empty day caption already describes the complete seven-day window.
 */
export function weekBreakCaption(summary: BreakSummary): string {
  const total = weekTotal(summary);
  if (total === 0) return "";
  if (total === 1) return "1 outcome in the last seven days.";
  return `${total} outcomes in the last seven days.`;
}

/** Loading copy before the first ledger summary arrives. */
export function breakLoadingCaption(): string {
  return "Reading local break history…";
}

/** Calm error copy; timer continues regardless. */
export function breakErrorCaption(_error: string | null): string {
  return "Break outcomes are unavailable right now. The break timer is unaffected.";
}

/** Error-precedence caption for retained last-known break figures. */
export function breakStaleCaption(_error: string | null): string {
  return "Break outcomes are unavailable; last known summary shown. The break timer is unaffected.";
}

/** Consumer caption selected from freshness, never from data presence alone. */
export function breakRefreshCaption(state: RefreshState<BreakSummary>): string {
  switch (state.status) {
    case "loading":
      return breakLoadingCaption();
    case "fresh":
      return breakSummaryCaption(state.data);
    case "stale":
      return breakStaleCaption(state.error);
    case "unavailable":
      return breakErrorCaption(state.error);
  }
}
