/**
 * Local break-outcome counts from the native ledger (observe-only, no scores).
 */

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
      label: "Shown",
      count: summary.scheduledShown,
      hint: "Scheduled break covered the desk"
    },
    {
      kind: "naturalIdle",
      label: "Natural",
      count: summary.naturalIdle,
      hint: "Already away when a break was due"
    },
    {
      kind: "manualTakeBreak",
      label: "Manual",
      count: summary.manualTakeBreak,
      hint: "You started a break yourself"
    },
    {
      kind: "fullscreenSuppress",
      label: "Held",
      count: summary.fullscreenSuppress,
      hint: "Held back while a window was fullscreen"
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
    return "No break outcomes in the last day yet.";
  }
  return "Local counts for this device · observe only";
}

/**
 * Quieter week line. Avoids competing with day totals.
 */
export function weekBreakCaption(summary: BreakSummary): string {
  const total =
    summary.weekScheduledShown +
    summary.weekNaturalIdle +
    summary.weekManualTakeBreak +
    summary.weekFullscreenSuppress;
  if (total === 0) return "No outcomes in the last seven days.";
  if (total === 1) return "1 outcome in the last seven days.";
  return `${total} outcomes in the last seven days.`;
}

/** Loading copy before the first ledger summary arrives. */
export function breakLoadingCaption(): string {
  return "Reading local break history…";
}

/** Calm error copy; timer continues regardless. */
export function breakErrorCaption(error: string | null): string {
  if (!error || !error.trim()) {
    return "Break outcomes are unavailable right now. The break timer is unaffected.";
  }
  const trimmed = error.trim();
  if (trimmed.length <= 160) {
    return `${trimmed} The break timer is unaffected.`;
  }
  return "Break outcomes are unavailable right now. The break timer is unaffected.";
}
