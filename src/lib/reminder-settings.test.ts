import { describe, expect, test } from "bun:test";
import {
  DEFAULT_BREAK_SECONDS,
  DEFAULT_WORK_MINUTES,
  loadAuthoritativeReminderSettings,
  reminderSettingsExactlyMatch,
  reminderSettingsRecovery,
  resolveReminderSettingsSave,
  validateReminderSettings,
  type ReminderSettingsView
} from "./reminder-settings";

const defaultOptions = {
  syncAcrossDevices: false,
  gridOffsetMinutes: 0,
  preBreakCueEnabled: true
};

describe("reminder settings recovery", () => {
  const savedSettings = {
    workMinutes: 45,
    breakSeconds: 12,
    syncAcrossDevices: false,
    gridOffsetMinutes: 0,
    preBreakCueEnabled: true
  };

  test("keeps unknown health non-editable with Retry-only evidence-safe copy", () => {
    const recovery = reminderSettingsRecovery(null, null);

    expect(recovery.kind).toBe("unknown");
    expect(recovery.unavailable).toBe(true);
    expect(recovery.confirmedStorageUnavailable).toBe(false);
    expect(recovery.canRetry).toBe(true);
    expect(recovery.canRestoreDefaults).toBe(false);
    expect(recovery.heading).toContain("could not be confirmed");
    expect(recovery.message).not.toMatch(/corrupt|invalid|have not started|stopped/i);
  });

  test("offers retry without reset for confirmed read failures", () => {
    const recovery = reminderSettingsRecovery(
      { status: "unavailable", recovery: "retry" },
      null
    );

    expect(recovery.kind).toBe("storageUnavailable");
    expect(recovery.unavailable).toBe(true);
    expect(recovery.confirmedStorageUnavailable).toBe(true);
    expect(recovery.canRetry).toBe(true);
    expect(recovery.canRestoreDefaults).toBe(false);
  });

  test("offers preservation and defaults only for confirmed invalid health", () => {
    const recovery = reminderSettingsRecovery(
      { status: "unavailable", recovery: "retryOrStartNew" },
      null
    );

    expect(recovery.unavailable).toBe(true);
    expect(recovery.canRestoreDefaults).toBe(true);
    expect(JSON.stringify(recovery)).not.toContain("/private/path");
  });

  test("distinguishes a confirming envelope from confirmed available settings", () => {
    const confirming = reminderSettingsRecovery(
      { status: "available", recovery: "none" },
      null
    );
    expect(confirming.kind).toBe("confirming");
    expect(confirming.unavailable).toBe(true);
    expect(confirming.message).not.toMatch(/corrupt|have not started|stopped/i);

    const available = reminderSettingsRecovery(
      { status: "available", recovery: "none" },
      savedSettings
    );
    expect(available.kind).toBe("available");
    expect(available.unavailable).toBe(false);
  });

  test("a slow recovery follow-up cannot expose blank editable settings", async () => {
    let resolveSnapshot: ((view: ReminderSettingsView) => void) | undefined;
    const delayed = new Promise<ReminderSettingsView>((resolve) => {
      resolveSnapshot = resolve;
    });
    const followUp = loadAuthoritativeReminderSettings(() => delayed);

    const awaitingSnapshot = reminderSettingsRecovery(
      { status: "available", recovery: "none" },
      null
    );
    expect(awaitingSnapshot.unavailable).toBe(true);
    expect(awaitingSnapshot.canRestoreDefaults).toBe(false);

    resolveSnapshot?.({
      loadHealth: { status: "available", recovery: "none" },
      data: savedSettings
    });
    const result = await followUp;
    expect(result.outcome).toBe("confirmed");
    if (result.outcome === "confirmed") {
      expect(reminderSettingsRecovery(result.view.loadHealth, result.view.data).unavailable).toBe(
        false
      );
    }
  });

  test("a rejected recovery follow-up keeps available-without-data non-editable", async () => {
    const priorHealth = { status: "available", recovery: "none" } as const;
    const result = await loadAuthoritativeReminderSettings(() =>
      Promise.reject(new Error("snapshot IPC failed"))
    );

    expect(result.outcome).toBe("rejected");
    expect(reminderSettingsRecovery(priorHealth, null).unavailable).toBe(true);
    expect(reminderSettingsRecovery(priorHealth, null).canRetry).toBe(true);
    expect(reminderSettingsRecovery(priorHealth, null).canRestoreDefaults).toBe(false);
  });

  test("a committed save with a lost response waits for and accepts matching canonical timing", async () => {
    let resolveFollowUp!: (view: ReminderSettingsView) => void;
    const canonical = new Promise<ReminderSettingsView>((resolve) => {
      resolveFollowUp = resolve;
    });
    let settled = false;
    const resolution = resolveReminderSettingsSave(
      savedSettings,
      {
        latest: true,
        settled: { status: "rejected", reason: new Error("response lost") }
      },
      () => loadAuthoritativeReminderSettings(() => canonical)
    ).then((result) => {
      settled = true;
      return result;
    });

    await Promise.resolve();
    expect(settled).toBe(false);
    resolveFollowUp({
      loadHealth: { status: "available", recovery: "none" },
      data: savedSettings
    });

    expect(await resolution).toEqual({ outcome: "saved", settings: savedSettings });
  });

  test("a precommit rejection reloads a different canonical timing", async () => {
    const canonical = { ...savedSettings, workMinutes: 20 };
    const resolution = await resolveReminderSettingsSave(
      savedSettings,
      {
        latest: true,
        settled: { status: "rejected", reason: new Error("write rejected") }
      },
      async () => ({
        outcome: "confirmed",
        view: {
          loadHealth: { status: "available", recovery: "none" },
          data: canonical
        }
      })
    );

    expect(resolution).toEqual({ outcome: "reloaded", settings: canonical });
  });

  test("unavailable and rejected save follow-ups remain unconfirmed", async () => {
    const unavailable = await resolveReminderSettingsSave(
      savedSettings,
      {
        latest: true,
        settled: { status: "rejected", reason: new Error("save rejected") }
      },
      async () => ({
        outcome: "unavailable",
        view: {
          loadHealth: { status: "unavailable", recovery: "retry" },
          data: null
        }
      })
    );
    const rejected = await resolveReminderSettingsSave(
      savedSettings,
      {
        latest: true,
        settled: { status: "rejected", reason: new Error("save rejected") }
      },
      async () => ({ outcome: "rejected", error: new Error("follow-up rejected") })
    );

    expect(unavailable.outcome).toBe("unavailable");
    expect(rejected.outcome).toBe("unconfirmed");
  });

  test("a superseded save response still uses the authoritative follow-up", async () => {
    let followUps = 0;
    const resolution = await resolveReminderSettingsSave(
      savedSettings,
      { latest: false, settled: { status: "fulfilled", value: savedSettings } },
      async () => {
        followUps += 1;
        return {
          outcome: "confirmed",
          view: {
            loadHealth: { status: "available", recovery: "none" },
            data: savedSettings
          }
        };
      }
    );

    expect(followUps).toBe(1);
    expect(resolution).toEqual({ outcome: "saved", settings: savedSettings });
  });

  test("exact settings comparison checks every canonical field", () => {
    expect(reminderSettingsExactlyMatch(savedSettings, { ...savedSettings })).toBe(true);
    for (const changed of [
      { ...savedSettings, workMinutes: 44 },
      { ...savedSettings, breakSeconds: 13 },
      { ...savedSettings, syncAcrossDevices: true },
      { ...savedSettings, gridOffsetMinutes: 1 },
      { ...savedSettings, preBreakCueEnabled: false }
    ]) {
      expect(reminderSettingsExactlyMatch(savedSettings, changed)).toBe(false);
    }
  });
});

describe("reminder settings", () => {
  test("accepts the defaults and both range boundaries", () => {
    expect(
      validateReminderSettings(
        String(DEFAULT_WORK_MINUTES),
        String(DEFAULT_BREAK_SECONDS),
        defaultOptions
      ).settings
    ).toEqual({
      workMinutes: 20,
      breakSeconds: 20,
      ...defaultOptions
    });
    expect(validateReminderSettings("1", "3", defaultOptions).settings).toEqual({
      workMinutes: 1,
      breakSeconds: 3,
      ...defaultOptions
    });
    expect(validateReminderSettings("120", "30", defaultOptions).settings).toEqual({
      workMinutes: 120,
      breakSeconds: 30,
      ...defaultOptions
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
    const validation = validateReminderSettings(work, rest, defaultOptions);

    expect(validation.settings).toBeNull();
    expect(validation.workMinutesError ?? validation.breakSecondsError).not.toBeNull();
  });

  test("uses the displayed field names in validation feedback", () => {
    expect(validateReminderSettings("", "20", defaultOptions).workMinutesError).toBe(
      "Enter a focus duration."
    );
    expect(validateReminderSettings("20", "2", defaultOptions).breakSecondsError).toBe(
      "Rest duration must be between 3 and 30 seconds."
    );
  });

  test("canonicalizes leading zeroes after a valid save", () => {
    expect(validateReminderSettings("020", "08", defaultOptions).settings).toEqual({
      workMinutes: 20,
      breakSeconds: 8,
      ...defaultOptions
    });
  });

  test("carries sync and cue preferences into validated settings", () => {
    const result = validateReminderSettings("20", "20", {
      syncAcrossDevices: true,
      gridOffsetMinutes: 330,
      preBreakCueEnabled: false
    });
    expect(result.settings).toEqual({
      workMinutes: 20,
      breakSeconds: 20,
      syncAcrossDevices: true,
      gridOffsetMinutes: 330,
      preBreakCueEnabled: false
    });
  });

  test("still rejects an invalid duration regardless of other preferences", () => {
    const result = validateReminderSettings("0", "20", {
      syncAcrossDevices: true,
      gridOffsetMinutes: 330,
      preBreakCueEnabled: false
    });
    expect(result.settings).toBeNull();
    expect(result.workMinutesError).not.toBeNull();
  });
});
