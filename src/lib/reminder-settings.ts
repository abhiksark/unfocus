import {
  MAX_OVERLAY_DURATION_SECONDS,
  MIN_OVERLAY_DURATION_SECONDS
} from "./overlay-label";

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
