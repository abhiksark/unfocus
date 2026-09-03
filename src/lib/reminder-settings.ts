import {
  MAX_OVERLAY_DURATION_SECONDS,
  MIN_OVERLAY_DURATION_SECONDS
} from "./overlay-label";
import type { LocalSnapshot, StorageLoadHealth } from "./storage-health";

export const DEFAULT_WORK_MINUTES = 20;
export const DEFAULT_BREAK_SECONDS = 20;
export const MIN_WORK_MINUTES = 1;
export const MAX_WORK_MINUTES = 120;
export const MIN_BREAK_SECONDS = MIN_OVERLAY_DURATION_SECONDS;
export const MAX_BREAK_SECONDS = MAX_OVERLAY_DURATION_SECONDS;

export type ReminderSettings = {
  workMinutes: number;
  breakSeconds: number;
  syncAcrossDevices: boolean;
  gridOffsetMinutes: number;
  preBreakCueEnabled: boolean;
};

export type ReminderSettingsView = LocalSnapshot<ReminderSettings>;

export type ReminderSettingsErrorContext =
  | "load"
  | "save"
  | "saveReloaded"
  | "saveUnconfirmed"
  | "reset"
  | null;

/** Exact comparison of every canonical field persisted by native settings. */
export function reminderSettingsExactlyMatch(
  left: ReminderSettings,
  right: ReminderSettings
): boolean {
  return (
    left.workMinutes === right.workMinutes &&
    left.breakSeconds === right.breakSeconds &&
    left.syncAcrossDevices === right.syncAcrossDevices &&
    left.gridOffsetMinutes === right.gridOffsetMinutes &&
    left.preBreakCueEnabled === right.preBreakCueEnabled
  );
}

export type ReminderSettingsRecoveryKind =
  | "available"
  | "loading"
  | "unknown"
  | "confirming"
  | "storageUnavailable";

export type ReminderSettingsRecovery = {
  kind: ReminderSettingsRecoveryKind;
  unavailable: boolean;
  confirmedStorageUnavailable: boolean;
  canRetry: boolean;
  canRestoreDefaults: boolean;
  heading: string;
  message: string;
};

/**
 * The settings UI is usable only when one snapshot confirms both native
 * availability and authoritative data. Unknown IPC health gets retry-only
 * copy and never claims corruption or a stopped scheduler.
 */
export function reminderSettingsRecovery(
  health: StorageLoadHealth | null,
  settings: ReminderSettings | null,
  loading = false
): ReminderSettingsRecovery {
  const confirmedAvailable = health?.status === "available" && settings !== null;
  const confirmedStorageUnavailable = health?.status === "unavailable";
  const confirmedInvalid =
    confirmedStorageUnavailable && health.recovery === "retryOrStartNew";
  const kind: ReminderSettingsRecoveryKind = confirmedAvailable
    ? "available"
    : loading
      ? "loading"
      : confirmedStorageUnavailable
        ? "storageUnavailable"
        : health?.status === "available"
          ? "confirming"
          : "unknown";
  const copy = settingsRecoveryCopy(kind);
  return {
    kind,
    unavailable: !confirmedAvailable,
    confirmedStorageUnavailable,
    canRetry: !loading && !confirmedAvailable,
    canRestoreDefaults: !loading && confirmedInvalid,
    ...copy
  };
}

function settingsRecoveryCopy(
  kind: ReminderSettingsRecoveryKind
): Pick<ReminderSettingsRecovery, "heading" | "message"> {
  switch (kind) {
    case "available":
      return { heading: "Saved timing available", message: "" };
    case "loading":
      return {
        heading: "Reading saved timing",
        message: "Confirming the saved rhythm on this device…"
      };
    case "storageUnavailable":
      return {
        heading: "Saved timing unavailable",
        message: "Automatic reminders have not started. Retry, or preserve an invalid file before restoring defaults when offered."
      };
    case "confirming":
      return {
        heading: "Confirming saved timing",
        message: "Waiting for the authoritative saved rhythm before enabling editing."
      };
    case "unknown":
      return {
        heading: "Saved timing could not be confirmed",
        message: "Retry loading the saved rhythm. The reminder status above is unchanged."
      };
  }
}

export type ReminderSettingsSnapshotFollowUp =
  | { outcome: "confirmed"; view: ReminderSettingsView }
  | { outcome: "unavailable"; view: ReminderSettingsView }
  | { outcome: "rejected"; error: unknown };

export type ReminderSettingsSaveCommandResult = {
  latest: boolean;
  settled: PromiseSettledResult<ReminderSettings>;
};

export type ReminderSettingsSaveResolution =
  | { outcome: "saved"; settings: ReminderSettings }
  | { outcome: "reloaded"; settings: ReminderSettings }
  | { outcome: "unavailable"; view: ReminderSettingsView }
  | { outcome: "unconfirmed"; error: unknown };

/**
 * A rejected or superseded mutation response is ambiguous because native
 * persistence may already have committed. Only a canonical follow-up can
 * classify it; the response itself is authoritative only when fulfilled and
 * still latest.
 */
export async function resolveReminderSettingsSave(
  submitted: ReminderSettings,
  command: ReminderSettingsSaveCommandResult,
  followUp: () => Promise<ReminderSettingsSnapshotFollowUp>
): Promise<ReminderSettingsSaveResolution> {
  if (command.latest && command.settled.status === "fulfilled") {
    return { outcome: "saved", settings: command.settled.value };
  }

  const canonical = await followUp();
  if (canonical.outcome === "rejected") {
    return { outcome: "unconfirmed", error: canonical.error };
  }
  if (canonical.outcome === "unavailable") {
    return { outcome: "unavailable", view: canonical.view };
  }
  const canonicalSettings = canonical.view.data;
  if (canonicalSettings === null) {
    return {
      outcome: "unconfirmed",
      error: new Error("Confirmed settings follow-up had no canonical data")
    };
  }
  return reminderSettingsExactlyMatch(submitted, canonicalSettings)
    ? { outcome: "saved", settings: canonicalSettings }
    : { outcome: "reloaded", settings: canonicalSettings };
}

/** Shared asynchronous boundary for initial loads and recovery follow-ups. */
export async function loadAuthoritativeReminderSettings(
  load: () => Promise<ReminderSettingsView>
): Promise<ReminderSettingsSnapshotFollowUp> {
  try {
    const view = await load();
    return {
      outcome:
        view.loadHealth.status === "available" && view.data !== null
          ? "confirmed"
          : "unavailable",
      view
    };
  } catch (error) {
    return { outcome: "rejected", error };
  }
}

export type ReminderOptions = {
  syncAcrossDevices: boolean;
  gridOffsetMinutes: number;
  preBreakCueEnabled: boolean;
};

export type ReminderSettingsValidation = {
  settings: ReminderSettings | null;
  workMinutesError: string | null;
  breakSecondsError: string | null;
};

const DECIMAL_INTEGER = /^[0-9]+$/;

function integerField(
  rawValue: string,
  label: string,
  minimum: number,
  maximum: number,
  unit: string
): { value: number | null; error: string | null } {
  if (rawValue.length === 0) {
    return { value: null, error: `Enter a ${label.toLowerCase()}.` };
  }
  if (!DECIMAL_INTEGER.test(rawValue)) {
    return { value: null, error: `${label} must be a whole number.` };
  }

  const value = Number(rawValue);
  if (!Number.isSafeInteger(value) || value < minimum || value > maximum) {
    return {
      value: null,
      error: `${label} must be between ${minimum} and ${maximum} ${unit}.`
    };
  }

  return { value, error: null };
}

export function validateReminderSettings(
  workMinutes: string,
  breakSeconds: string,
  options: ReminderOptions
): ReminderSettingsValidation {
  const work = integerField(
    workMinutes,
    "Focus duration",
    MIN_WORK_MINUTES,
    MAX_WORK_MINUTES,
    "minutes"
  );
  const rest = integerField(
    breakSeconds,
    "Rest duration",
    MIN_BREAK_SECONDS,
    MAX_BREAK_SECONDS,
    "seconds"
  );

  return {
    settings:
      work.value === null || rest.value === null
        ? null
        : {
            workMinutes: work.value,
            breakSeconds: rest.value,
            syncAcrossDevices: options.syncAcrossDevices,
            gridOffsetMinutes: options.gridOffsetMinutes,
            preBreakCueEnabled: options.preBreakCueEnabled
          },
    workMinutesError: work.error,
    breakSecondsError: rest.error
  };
}
