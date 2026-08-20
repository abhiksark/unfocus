import { describe, expect, test } from "bun:test";
import {
  consumerReminderPresentation,
  consumerWarning,
  describeRhythm,
  focusProgress,
  formatMinuteDuration,
  formatSyncPreview,
  type ConsumerWarningInput
} from "./consumer-dashboard";
import type { GridPreview } from "./break-grid";
import type { DiagnosticsReport } from "./diagnostics";
import type { ReminderPhase, ReminderStatus } from "./reminder-status";
import type { ReminderSettings } from "./reminder-settings";

function status(
  phase: ReminderPhase,
  overrides: Partial<ReminderStatus> = {}
): ReminderStatus {
  return {
    phase,
    status: phase,
    remainingMilliseconds: phase === "working" ? 20 * 60_000 : null,
    pauseExpiresInMilliseconds: phase === "paused" ? 30 * 60_000 : null,
    overlayActive: false,
    settingsRevision: 1,
    stateRevision: 1,
    actionError: null,
    pauseAction: phase === "paused" ? "resume" : "pause",
    pauseActionLabel: phase === "paused" ? "Resume reminders" : "Pause for 30 minutes",
    pauseActionEnabled: phase === "working" || phase === "paused",
    takeBreakEnabled: phase === "working",
    previewEnabled: true,
    ...overrides
  };
}

const report: DiagnosticsReport = {
  operatingSystem: "linux",
  sessionType: "x11",
  desktop: "GNOME",
  display: ":0",
  monitors: [],
  monitorError: null,
  idleSeconds: 1,
  idleError: null,
  activeWindowFullscreen: false,
  fullscreenError: null,
  tray: { available: true, error: null }
};

function warningInput(overrides: Partial<ConsumerWarningInput> = {}): ConsumerWarningInput {
  return {
    report,
    diagnosticsError: null,
    reminderStatus: status("working"),
    reminderStatusError: null,
    reminderActionError: null,
    settingsError: null,
    settingsErrorContext: null,
    overlayError: null,
    ...overrides
  };
}

describe("consumer reminder presentation", () => {
  test.each([
    ["working", "You’re in focus time.", true, true, false],
    ["paused", "Reminders are paused.", false, false, true],
    ["break", "Your eye break is in progress.", false, false, false],
    ["stopped", "Your workday is complete.", false, false, false],
    ["unavailable", "Reminder status is unavailable.", false, false, false]
  ] as const)("presents %s without invalid actions", (phase, heading, take, pause, resume) => {
    const result = consumerReminderPresentation(status(phase), null);
    expect(result.heading).toBe(heading);
    expect(result.showTakeBreak).toBe(take);
    expect(result.showPause).toBe(pause);
    expect(result.showResume).toBe(resume);
  });

  test("gives a preview precedence outside a scheduled break", () => {
    expect(
      consumerReminderPresentation(status("working", { overlayActive: true }), null).kind
    ).toBe("preview");
    expect(
      consumerReminderPresentation(status("paused", { overlayActive: true }), null).kind
    ).toBe("preview");
    expect(
      consumerReminderPresentation(status("break", { overlayActive: true }), null).kind
    ).toBe("break");
  });

  test("treats a failed status read as unavailable even with stale state", () => {
    const result = consumerReminderPresentation(status("working"), "IPC failed");
    expect(result.kind).toBe("unavailable");
    expect(result.secondary).toContain("retry automatically");
  });

  test("does not invent missing work or pause timing", () => {
    expect(
      consumerReminderPresentation(status("working", { remainingMilliseconds: null }), null)
        .secondary
    ).toBe("The next break time is unavailable.");
    expect(
      consumerReminderPresentation(
        status("paused", { pauseExpiresInMilliseconds: null }),
        null
      ).secondary
    ).toBe("The automatic resume time is unavailable.");
  });

  test("rounds exact, partial, and sub-minute durations with the native ceiling rule", () => {
    expect(formatMinuteDuration(60_000)).toBe("1 min");
    expect(formatMinuteDuration(120_000)).toBe("2 min");
    expect(formatMinuteDuration(60_001)).toBe("2 min");
    expect(formatMinuteDuration(59_999)).toBe("less than 1 min");
  });

  test("uses a quiet loading state before the first native response", () => {
    const result = consumerReminderPresentation(null, null);
    expect(result.kind).toBe("loading");
    expect(result.showTakeBreak || result.showPause || result.showResume).toBe(false);
  });
});

describe("consumer warnings", () => {
  test("is absent when the dashboard is healthy", () => {
    expect(consumerWarning(warningInput())).toBeNull();
  });

  test("gives tray failure priority and never exposes raw native errors", () => {
    const raw = "indicator construction failed at /secret/path";
    const warning = consumerWarning(
      warningInput({
        report: { ...report, tray: { available: false, error: raw } },
        diagnosticsError: "transport secret",
        reminderActionError: "action secret"
      })
    );
    expect(warning?.kind).toBe("tray");
    expect(warning?.message).toContain("Closing this window exits Unfocus");
    expect(JSON.stringify(warning)).not.toContain(raw);
    expect(JSON.stringify(warning)).not.toContain("secret");
  });

  test.each(["monitorError", "idleError", "fullscreenError"] as const)(
    "explains a %s failure while preserving timer continuity",
    (field) => {
      const warning = consumerWarning(
        warningInput({ report: { ...report, [field]: "raw probe error" } })
      );
      expect(warning?.kind).toBe("probes");
      expect(warning?.message).toContain("timer keeps running");
      expect(JSON.stringify(warning)).not.toContain("raw probe error");
    }
  );

  test("uses one warning for multiple probe failures", () => {
    const warning = consumerWarning(
      warningInput({
        report: { ...report, monitorError: "one", idleError: "two", fullscreenError: "three" }
      })
    );
    expect(warning?.kind).toBe("probes");
  });

  test("only claims the timer is running after an independent reminder read", () => {
    expect(
      consumerWarning(warningInput({ diagnosticsError: "IPC failed" }))?.message
    ).toContain("timer is still running");
    expect(
      consumerWarning(
        warningInput({ diagnosticsError: "IPC failed", reminderStatus: null })
      )?.message
    ).not.toContain("timer is still running");
    expect(
      consumerWarning(
        warningInput({ diagnosticsError: "IPC failed", reminderStatus: status("stopped") })
      )?.message
    ).not.toContain("timer is still running");
  });

  test("prioritizes direct reminder and settings failures ahead of background health", () => {
    expect(
      consumerWarning(
        warningInput({ reminderActionError: "raw", diagnosticsError: "diagnostics raw" })
      )?.kind
    ).toBe("reminder-action");
    expect(
      consumerWarning(
        warningInput({
          settingsError: "raw",
          settingsErrorContext: "save",
          diagnosticsError: "diagnostics raw"
        })
      )?.kind
    ).toBe("settings");
  });

  test("distinguishes settings load failure without exposing its raw error", () => {
    const warning = consumerWarning(
      warningInput({ settingsError: "private path", settingsErrorContext: "load" })
    );
    expect(warning?.heading).toBe("Saved timing is unavailable");
    expect(JSON.stringify(warning)).not.toContain("private path");
  });

  test("recovers once the current errors clear", () => {
    const degraded = warningInput({ reminderActionError: "raw failure" });
    expect(consumerWarning(degraded)).not.toBeNull();
    expect(consumerWarning({ ...degraded, reminderActionError: null })).toBeNull();
  });
});

const settings: ReminderSettings = {
  workMinutes: 20,
  breakSeconds: 20,
  syncAcrossDevices: false,
  gridOffsetMinutes: 0
};

describe("focusProgress", () => {
  test("returns null without a status", () => {
    expect(focusProgress(null, settings)).toBeNull();
  });

  test("returns null without settings", () => {
    expect(focusProgress(status("working"), null)).toBeNull();
  });

  test("returns null when the remaining time is unavailable", () => {
    expect(
      focusProgress(status("working", { remainingMilliseconds: null }), settings)
    ).toBeNull();
  });

  test("returns null outside the working phase", () => {
    for (const phase of ["paused", "break", "stopped", "unavailable"] as const) {
      expect(focusProgress(status(phase), settings)).toBeNull();
    }
  });

  test("returns null while a preview overlay is open", () => {
    expect(
      focusProgress(status("working", { overlayActive: true }), settings)
    ).toBeNull();
  });

  test("reports the elapsed fraction of the work interval", () => {
    expect(
      focusProgress(status("working", { remainingMilliseconds: 10 * 60_000 }), settings)
    ).toBe(0.5);
  });

  test("clamps to zero when remaining exceeds the interval", () => {
    expect(
      focusProgress(status("working", { remainingMilliseconds: 40 * 60_000 }), settings)
    ).toBe(0);
  });

  test("clamps to one when remaining is negative", () => {
    expect(
      focusProgress(status("working", { remainingMilliseconds: -5_000 }), settings)
    ).toBe(1);
  });

  test("returns null for a non-positive work interval", () => {
    expect(
      focusProgress(status("working"), {
        workMinutes: 0,
        breakSeconds: 20,
        syncAcrossDevices: false,
        gridOffsetMinutes: 0
      })
    ).toBeNull();
  });
});

describe("describeRhythm", () => {
  test("describes the rhythm with its sync state", () => {
    expect(
      describeRhythm({
        workMinutes: 20,
        breakSeconds: 20,
        syncAcrossDevices: false,
        gridOffsetMinutes: 0
      })
    ).toBe("20 min focus → 20 sec rest");

    expect(
      describeRhythm({
        workMinutes: 20,
        breakSeconds: 20,
        syncAcrossDevices: true,
        gridOffsetMinutes: 330
      })
    ).toBe("20 min focus → 20 sec rest · synced across devices");
  });
});

describe("formatSyncPreview", () => {
  const timeFormat = new Intl.DateTimeFormat([], { hour: "numeric", minute: "2-digit" });

  test("pads minutes past the hour with a leading zero", () => {
    const preview: GridPreview = { kind: "hourly", minutes: [0, 20, 40] };
    expect(formatSyncPreview(preview, 330, timeFormat)).toBe(
      "Breaks at :00, :20, :40 past the hour, UTC+05:30."
    );
  });

  test("formats upcoming absolute times with the offset", () => {
    const preview: GridPreview = { kind: "upcoming", atMs: [0] };
    const listed = timeFormat.format(new Date(0));
    expect(formatSyncPreview(preview, 0, timeFormat)).toBe(
      `Next breaks at ${listed}, UTC.`
    );
  });
});
