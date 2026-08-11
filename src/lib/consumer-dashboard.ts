import type { DiagnosticsReport } from "./diagnostics";
import type { ReminderStatus } from "./reminder-status";

export type ConsumerReminderKind =
  | "loading"
  | "working"
  | "paused"
  | "break"
  | "preview"
  | "stopped"
  | "unavailable";

export type ConsumerReminderPresentation = {
  kind: ConsumerReminderKind;
  heading: string;
  secondary: string;
  showTakeBreak: boolean;
  showPause: boolean;
  showResume: boolean;
};

export type ConsumerWarningKind =
  | "tray"
  | "reminder-action"
  | "settings"
  | "preview"
  | "reminder-status"
  | "diagnostics"
  | "probes";

export type ConsumerWarning = {
  kind: ConsumerWarningKind;
  heading: string;
  message: string;
};

export type ConsumerWarningInput = {
  report: DiagnosticsReport | null;
  diagnosticsError: string | null;
  reminderStatus: ReminderStatus | null;
  reminderStatusError: string | null;
  reminderActionError: string | null;
  settingsError: string | null;
  settingsErrorContext: "load" | "save" | "reset" | null;
  overlayError: string | null;
};

const MILLISECONDS_PER_MINUTE = 60_000;

export function formatMinuteDuration(milliseconds: number): string {
  if (milliseconds < MILLISECONDS_PER_MINUTE) return "less than 1 min";
  return `${Math.ceil(milliseconds / MILLISECONDS_PER_MINUTE)} min`;
}

function timedSecondary(
  prefix: string,
  unavailable: string,
  milliseconds: number | null
): string {
  if (milliseconds === null) return unavailable;
  return `${prefix} ${formatMinuteDuration(milliseconds)}.`;
}

function presentation(
  kind: ConsumerReminderKind,
  heading: string,
  secondary: string
): ConsumerReminderPresentation {
  return {
    kind,
    heading,
    secondary,
    showTakeBreak: kind === "working",
    showPause: kind === "working",
    showResume: kind === "paused"
  };
}

export function consumerReminderPresentation(
  status: ReminderStatus | null,
  statusError: string | null
): ConsumerReminderPresentation {
  if (statusError) {
    return presentation(
      "unavailable",
      "Reminder status is unavailable.",
      "Unfocus will retry automatically."
    );
  }
  if (!status) {
    return presentation("loading", "Checking your reminder…", "This will only take a moment.");
  }
  if (status.overlayActive && status.phase !== "break") {
    return presentation(
      "preview",
      "Your break preview is open.",
      "The preview closes automatically."
    );
  }

  switch (status.phase) {
    case "working":
      return presentation(
        "working",
        "You’re in focus time.",
        timedSecondary(
          "Next break in",
          "The next break time is unavailable.",
          status.remainingMilliseconds
        )
      );
    case "paused":
      return presentation(
        "paused",
        "Reminders are paused.",
        timedSecondary(
          "Resumes in",
          "The automatic resume time is unavailable.",
          status.pauseExpiresInMilliseconds
        )
      );
    case "break":
      return presentation(
        "break",
        "Your eye break is in progress.",
        "Look at something far away."
      );
    case "stopped":
      return presentation(
        "stopped",
        "Your workday is complete.",
        "Reminders are finished for now."
      );
    case "unavailable":
      return presentation(
        "unavailable",
        "Reminder status is unavailable.",
        "Unfocus will retry automatically."
      );
  }
}

function hasProbeFailure(report: DiagnosticsReport | null): boolean {
  return Boolean(report?.monitorError || report?.idleError || report?.fullscreenError);
}

function timerIsConfirmedRunning(
  status: ReminderStatus | null,
  statusError: string | null
): boolean {
  return (
    statusError === null &&
    status !== null &&
    (status.phase === "working" || status.phase === "break")
  );
}

export function consumerWarning(input: ConsumerWarningInput): ConsumerWarning | null {
  if (input.report?.tray.available === false || input.report?.tray.error) {
    return {
      kind: "tray",
      heading: "Keep this window open",
      message:
        "The system tray isn’t available. Closing this window exits Unfocus, so keep it open while you use reminders."
    };
  }
  if (input.reminderActionError || input.reminderStatus?.actionError) {
    return {
      kind: "reminder-action",
      heading: "That reminder change didn’t work",
      message: "Your previous reminder state was retained. You can try again."
    };
  }
  if (input.settingsError) {
    if (input.settingsErrorContext === "load") {
      return {
        kind: "settings",
        heading: "Saved timing is unavailable",
        message: "Unfocus couldn’t read your saved rhythm. Your reminder state was not changed."
      };
    }
    return {
      kind: "settings",
      heading: "Your timing change wasn’t saved",
      message: "Your previous rhythm was retained. Check the timing fields and try again."
    };
  }
  if (input.overlayError) {
    return {
      kind: "preview",
      heading: "The break screen couldn’t open",
      message: "Your reminder timer was unchanged. You can try the preview again."
    };
  }
  if (input.reminderStatusError) {
    return {
      kind: "reminder-status",
      heading: "Reminder controls are temporarily unavailable",
      message: "Unfocus will retry automatically. Your previous reminder state was retained."
    };
  }
  if (input.diagnosticsError) {
    const retry = "Device status is temporarily unavailable and will retry automatically.";
    return {
      kind: "diagnostics",
      heading: "Device status is temporarily unavailable",
      message: timerIsConfirmedRunning(input.reminderStatus, input.reminderStatusError)
        ? `${retry} Your reminder timer is still running.`
        : retry
    };
  }
  if (hasProbeFailure(input.report)) {
    return {
      kind: "probes",
      heading: "Some device checks are unavailable",
      message: "Your reminder timer keeps running while Unfocus retries those checks."
    };
  }
  return null;
}
