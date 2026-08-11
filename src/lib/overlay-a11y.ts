/**
 * Pure accessibility helpers for the break overlay (issue #30).
 * Keep presentation rules here so unit tests pin the contract without DOM.
 */

export type OverlayAnnouncementInput = {
  complete: boolean;
  secondsLeft: number;
};

/**
 * Spoken announcements for the break overlay.
 * Must stay sparse: never announce every second of the countdown.
 */
export function overlayAnnouncement(input: OverlayAnnouncementInput): string {
  if (input.complete) return "Break complete.";
  if (input.secondsLeft === 5) return "Five seconds remain in this break.";
  return "";
}

/** Static accessible name for the break region. */
export const OVERLAY_REGION_LABEL = "Eye break";

/** Accessible name for the End break control. */
export const OVERLAY_END_BREAK_LABEL = "End break";

/**
 * Countdown lives on a timer role with aria-live="off" so assistive tech does
 * not speak every second. The label still exposes remaining time for query.
 */
export function overlayCountdownLabel(
  complete: boolean,
  secondsLeft: number
): string {
  if (complete) return "Break complete";
  return `${secondsLeft} seconds remaining`;
}

/** Whether a continuous countdown live region is allowed (must be false). */
export function countdownLiveRegionAllowed(): boolean {
  return false;
}
