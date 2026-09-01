import type { StorageLoadHealth } from "./storage-health";

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

export type ReminderCapability =
  | "pauseActionEnabled"
  | "takeBreakEnabled"
  | "previewEnabled";

export type ReminderOperationKind = "action" | "settings";

export type ReminderOperationToken = {
  readonly kind: ReminderOperationKind;
  readonly id: number;
};

export type ReminderOperationGate = {
  begin: (kind: ReminderOperationKind) => ReminderOperationToken | null;
  finish: (token: ReminderOperationToken) => void;
};

/** One in-flight reminder action or settings mutation owns publication. */
export function createReminderOperationGate(): ReminderOperationGate {
  let active: ReminderOperationToken | null = null;
  let nextId = 0;
  return {
    begin(kind) {
      if (active !== null) return null;
      active = { kind, id: (nextId += 1) };
      return active;
    },
    finish(token) {
      if (active === token) active = null;
    }
  };
}

/**
 * Native status is authoritative for timer actions and previews. Settings
 * health only vetoes a capability when native code explicitly reports that
 * settings are unavailable; an unknown or rejected editor snapshot must not
 * strand an otherwise healthy timer.
 */
export function reminderCapabilityAvailable(
  status: ReminderStatus | null,
  statusError: string | null,
  settingsHealth: StorageLoadHealth | null,
  capability: ReminderCapability
): boolean {
  return (
    statusError === null &&
    settingsHealth?.status !== "unavailable" &&
    status?.phase !== "unavailable" &&
    status?.[capability] === true
  );
}

export function reminderPreviewLabel(
  status: Pick<ReminderStatus, "overlayActive"> | null,
  opening: boolean
): string {
  if (opening) return "Opening…";
  return status?.overlayActive ? "Break screen open" : "Preview break screen";
}

export function developerOverlayTestLabel(
  status: Pick<ReminderStatus, "overlayActive"> | null,
  opening: boolean
): string {
  if (opening) return "Opening overlay…";
  return status?.overlayActive ? "Overlay active" : "Run overlay test";
}

export function pauseActionCommand(
  status: Pick<ReminderStatus, "pauseAction">
): ReminderActionCommand {
  return status.pauseAction === "resume" ? "resume_reminders" : "pause_reminders";
}
