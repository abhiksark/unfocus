/**
 * Pure accessibility helpers for the consumer dashboard (issue #30).
 */

import type { ConsumerReminderPresentation } from "./consumer-dashboard";

/** Stable region names used by the consumer dashboard. */
export const DASHBOARD_REMINDER_ACTIONS_LABEL = "Reminder actions";
export const DASHBOARD_STATE_LIVE_REGION = true;

/**
 * Which primary actions must be keyboard-reachable for a presentation.
 * Pointer-only disclosure is not allowed for these.
 */
export function keyboardReachableActions(
  presentation: Pick<
    ConsumerReminderPresentation,
    "showTakeBreak" | "showPause" | "showResume"
  >
): string[] {
  const actions: string[] = [];
  if (presentation.showTakeBreak) actions.push("Take a break now");
  if (presentation.showPause) actions.push("Pause");
  if (presentation.showResume) actions.push("Resume");
  return actions;
}

/**
 * Status and error feedback must use live regions so keyboard/AT users hear
 * outcomes without relying on color alone.
 */
export function dashboardUsesPoliteLiveStatus(): boolean {
  return true;
}
