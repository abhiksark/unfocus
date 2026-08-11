import { describe, expect, test } from "bun:test";
import {
  DASHBOARD_REMINDER_ACTIONS_LABEL,
  dashboardUsesPoliteLiveStatus,
  keyboardReachableActions
} from "./dashboard-a11y";
import { consumerReminderPresentation } from "./consumer-dashboard";
import type { ReminderStatus } from "./reminder-status";

function status(partial: Partial<ReminderStatus>): ReminderStatus {
  return {
    phase: "working",
    status: "Next break in 20 min",
    remainingMilliseconds: 20 * 60_000,
    pauseExpiresInMilliseconds: null,
    overlayActive: false,
    settingsRevision: 0,
    stateRevision: 0,
    actionError: null,
    pauseAction: "pause",
    pauseActionLabel: "Pause for 30 minutes",
    pauseActionEnabled: true,
    takeBreakEnabled: true,
    previewEnabled: true,
    ...partial
  };
}

describe("dashboard accessibility contract", () => {
  test("exposes a stable actions region name and polite live status", () => {
    expect(DASHBOARD_REMINDER_ACTIONS_LABEL).toBe("Reminder actions");
    expect(dashboardUsesPoliteLiveStatus()).toBe(true);
  });

  test("working presentation offers keyboard-reachable break and pause", () => {
    const presentation = consumerReminderPresentation(status({ phase: "working" }), null);
    expect(keyboardReachableActions(presentation)).toEqual([
      "Take a break now",
      "Pause"
    ]);
  });

  test("paused presentation offers keyboard-reachable resume only", () => {
    const presentation = consumerReminderPresentation(
      status({
        phase: "paused",
        pauseAction: "resume",
        pauseActionEnabled: true,
        takeBreakEnabled: false
      }),
      null
    );
    expect(keyboardReachableActions(presentation)).toEqual(["Resume"]);
  });

  test("break presentation does not invent pointer-only primary actions", () => {
    const presentation = consumerReminderPresentation(status({ phase: "break" }), null);
    expect(keyboardReachableActions(presentation)).toEqual([]);
  });
});
