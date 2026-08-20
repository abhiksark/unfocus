import { describe, expect, test } from "bun:test";
import {
  DEFAULT_BREAK_SECONDS,
  DEFAULT_WORK_MINUTES,
  validateReminderSettings
} from "./reminder-settings";

describe("reminder settings", () => {
  test("accepts the defaults and both range boundaries", () => {
    expect(
      validateReminderSettings(String(DEFAULT_WORK_MINUTES), String(DEFAULT_BREAK_SECONDS), {
        syncAcrossDevices: false,
        gridOffsetMinutes: 0
      }).settings
    ).toEqual({
      workMinutes: 20,
      breakSeconds: 20,
      syncAcrossDevices: false,
      gridOffsetMinutes: 0
    });
    expect(
      validateReminderSettings("1", "3", {
        syncAcrossDevices: false,
        gridOffsetMinutes: 0
      }).settings
    ).toEqual({
      workMinutes: 1,
      breakSeconds: 3,
      syncAcrossDevices: false,
      gridOffsetMinutes: 0
    });
    expect(
      validateReminderSettings("120", "30", {
        syncAcrossDevices: false,
        gridOffsetMinutes: 0
      }).settings
    ).toEqual({
      workMinutes: 120,
      breakSeconds: 30,
      syncAcrossDevices: false,
      gridOffsetMinutes: 0
    });
  });

  test.each([
    ["", "20"],
    ["1.5", "20"],
    ["twenty", "20"],
    ["-1", "20"],
    ["0", "20"],
    ["121", "20"],
    ["9".repeat(400), "20"],
    ["20", ""],
    ["20", "3.5"],
    ["20", "seconds"],
    ["20", "-1"],
    ["20", "0"],
    ["20", "2"],
    ["20", "31"],
    ["20", "9".repeat(400)]
  ])("rejects invalid work=%s break=%s", (work, rest) => {
    const validation = validateReminderSettings(work, rest, {
      syncAcrossDevices: false,
      gridOffsetMinutes: 0
    });

    expect(validation.settings).toBeNull();
    expect(validation.workMinutesError ?? validation.breakSecondsError).not.toBeNull();
  });

  test("canonicalizes leading zeroes after a valid save", () => {
    expect(
      validateReminderSettings("020", "08", {
        syncAcrossDevices: false,
        gridOffsetMinutes: 0
      }).settings
    ).toEqual({
      workMinutes: 20,
      breakSeconds: 8,
      syncAcrossDevices: false,
      gridOffsetMinutes: 0
    });
  });

  test("carries the sync pair through into validated settings", () => {
    const result = validateReminderSettings("20", "20", {
      syncAcrossDevices: true,
      gridOffsetMinutes: 330
    });
    expect(result.settings).toEqual({
      workMinutes: 20,
      breakSeconds: 20,
      syncAcrossDevices: true,
      gridOffsetMinutes: 330
    });
  });

  test("still rejects an invalid duration regardless of the sync pair", () => {
    const result = validateReminderSettings("0", "20", {
      syncAcrossDevices: true,
      gridOffsetMinutes: 330
    });
    expect(result.settings).toBeNull();
    expect(result.workMinutesError).not.toBeNull();
  });
});
