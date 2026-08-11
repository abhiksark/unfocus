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

/** Calm one-line caption for the last-24-hour rest outcomes. */
export function breakSummaryCaption(summary: BreakSummary): string {
  const parts: string[] = [];
  if (summary.scheduledShown > 0) {
    parts.push(
      summary.scheduledShown === 1
        ? "1 rest shown"
        : `${summary.scheduledShown} rests shown`
    );
  }
  if (summary.naturalIdle > 0) {
    parts.push(
      summary.naturalIdle === 1
        ? "1 natural rest"
        : `${summary.naturalIdle} natural rests`
    );
  }
  if (summary.manualTakeBreak > 0) {
    parts.push(
      summary.manualTakeBreak === 1
        ? "1 manual rest"
        : `${summary.manualTakeBreak} manual rests`
    );
  }
  if (summary.fullscreenSuppress > 0) {
    parts.push(
      summary.fullscreenSuppress === 1
        ? "1 held for fullscreen"
        : `${summary.fullscreenSuppress} held for fullscreen`
    );
  }
  if (parts.length === 0) {
    return "No break outcomes recorded yet in this window.";
  }
  return parts.join(" · ");
}

export function weekBreakCaption(summary: BreakSummary): string {
  const total =
    summary.weekScheduledShown +
    summary.weekNaturalIdle +
    summary.weekManualTakeBreak +
    summary.weekFullscreenSuppress;
  if (total === 0) return "Nothing recorded in the last seven days.";
  if (total === 1) return "1 outcome in the last seven days.";
  return `${total} outcomes in the last seven days.`;
}
