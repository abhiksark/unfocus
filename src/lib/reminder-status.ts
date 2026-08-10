export type ReminderPhase = "working" | "break" | "paused" | "stopped" | "unavailable";
export type ReminderPauseAction = "pause" | "resume";
export type ReminderActionCommand =
  | "pause_reminders"
  | "resume_reminders"
  | "take_break_now";

export interface ReminderStatus {
  phase: ReminderPhase;
  status: string;
  remainingMilliseconds: number | null;
  pauseExpiresInMilliseconds: number | null;
  overlayActive: boolean;
  settingsRevision: number;
  stateRevision: number;
  actionError: string | null;
  pauseAction: ReminderPauseAction;
  pauseActionLabel: string;
  pauseActionEnabled: boolean;
  takeBreakEnabled: boolean;
  previewEnabled: boolean;
}

export function pauseActionCommand(
  status: Pick<ReminderStatus, "pauseAction">
): ReminderActionCommand {
  return status.pauseAction === "resume" ? "resume_reminders" : "pause_reminders";
}
