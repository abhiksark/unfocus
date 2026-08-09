import { describe, expect, test } from "bun:test";
import {
  DEFAULT_BREAK_SECONDS,
  DEFAULT_WORK_MINUTES,
  validateReminderSettings
} from "./reminder-settings";

describe("reminder settings", () => {
  test("accepts the defaults and both range boundaries", () => {
    expect(
      validateReminderSettings(String(DEFAULT_WORK_MINUTES), String(DEFAULT_BREAK_SECONDS))
        .settings
    ).toEqual({ workMinutes: 20, breakSeconds: 20 });
    expect(validateReminderSettings("1", "3").settings).toEqual({
      workMinutes: 1,
      breakSeconds: 3
    });
    expect(validateReminderSettings("120", "30").settings).toEqual({
      workMinutes: 120,
      breakSeconds: 30
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
    const validation = validateReminderSettings(work, rest);

    expect(validation.settings).toBeNull();
    expect(validation.workMinutesError ?? validation.breakSecondsError).not.toBeNull();
  });

  test("canonicalizes leading zeroes after a valid save", () => {
    expect(validateReminderSettings("020", "08").settings).toEqual({
      workMinutes: 20,
      breakSeconds: 8
    });
  });
});
