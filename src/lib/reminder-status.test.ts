import { describe, expect, test } from "bun:test";
import {
  createReminderOperationGate,
  developerOverlayTestLabel,
  pauseActionCommand,
  reminderCapabilityAvailable,
  reminderPreviewLabel,
  type ReminderCapability,
  type ReminderOperationKind,
  type ReminderStatus
} from "./reminder-status";

const healthyStatus: ReminderStatus = {
  phase: "working",
  status: "Working · break in 20 min",
  remainingMilliseconds: 20 * 60_000,
  pauseExpiresInMilliseconds: null,
  overlayActive: false,
  settingsRevision: 1,
  stateRevision: 1,
  actionError: null,
  pauseAction: "pause",
  pauseActionLabel: "Pause for 30 minutes",
  pauseActionEnabled: true,
  takeBreakEnabled: true,
  previewEnabled: true
};

const capabilities: ReminderCapability[] = [
  "pauseActionEnabled",
  "takeBreakEnabled",
  "previewEnabled"
];

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((settle) => {
    resolve = settle;
  });
  return { promise, resolve };
}

describe("reminder operation serialization", () => {
  test.each([
    ["settings owns the gate, blocked settles first", "settings", "action", "blocked"],
    ["settings owns the gate, owner settles first", "settings", "action", "owner"],
    ["action owns the gate, blocked settles first", "action", "settings", "blocked"],
    ["action owns the gate, owner settles first", "action", "settings", "owner"]
  ] as const)("%s", async (_label, owner, blocked, settlesFirst) => {
    const gate = createReminderOperationGate();
    const ownerResponse = deferred<string>();
    const blockedResponse = deferred<string>();
    const publications: string[] = [];

    async function dispatch(kind: ReminderOperationKind, response: Promise<string>) {
      const token = gate.begin(kind);
      if (!token) return false;
      try {
        publications.push(await response);
        return true;
      } finally {
        gate.finish(token);
      }
    }

    const authoritative = dispatch(owner, ownerResponse.promise);
    const rejectedDispatch = dispatch(blocked, blockedResponse.promise);
    expect(await rejectedDispatch).toBe(false);

    if (settlesFirst === "blocked") {
      blockedResponse.resolve("blocked response");
      await blockedResponse.promise;
      expect(publications).toEqual([]);
    }
    ownerResponse.resolve("only authoritative response");
    expect(await authoritative).toBe(true);
    if (settlesFirst === "owner") {
      blockedResponse.resolve("blocked response");
      await blockedResponse.promise;
    }

    expect(publications).toEqual(["only authoritative response"]);
  });
});

describe("reminder controls", () => {
  test("dispatches only the native action represented by the shared status", () => {
    expect(pauseActionCommand({ pauseAction: "pause" })).toBe("pause_reminders");
    expect(pauseActionCommand({ pauseAction: "resume" })).toBe("resume_reminders");
  });

  test("labels only an active overlay as an open break screen", () => {
    expect(reminderPreviewLabel(healthyStatus, false)).toBe("Preview break screen");
    expect(reminderPreviewLabel({ overlayActive: true }, false)).toBe(
      "Break screen open"
    );
    expect(reminderPreviewLabel({ overlayActive: false }, true)).toBe("Opening…");
    expect(reminderPreviewLabel(null, false)).toBe("Preview break screen");
  });

  test("labels only an active overlay as an active developer test", () => {
    expect(developerOverlayTestLabel(healthyStatus, false)).toBe("Run overlay test");
    expect(developerOverlayTestLabel({ overlayActive: true }, false)).toBe(
      "Overlay active"
    );
    expect(developerOverlayTestLabel({ overlayActive: false }, true)).toBe(
      "Opening overlay…"
    );
    expect(developerOverlayTestLabel(null, false)).toBe("Run overlay test");
  });

  test.each([
    ["fresh status and settings", healthyStatus, null, { status: "available", recovery: "none" }, true],
    ["fresh status with unknown settings snapshot", healthyStatus, null, null, true],
    ["rejected status with retained data", healthyStatus, "status IPC failed", null, false],
    ["missing status", null, null, null, false],
    [
      "confirmed settings storage unavailable",
      healthyStatus,
      null,
      { status: "unavailable", recovery: "retry" },
      false
    ],
    [
      "native unavailable phase",
      { ...healthyStatus, phase: "unavailable" },
      null,
      null,
      false
    ]
  ] as const)("capability matrix: %s", (_label, status, statusError, health, expected) => {
    for (const capability of capabilities) {
      expect(
        reminderCapabilityAvailable(status, statusError, health, capability)
      ).toBe(expected);
    }
  });

  test("a settings-snapshot IPC failure alone does not disable fresh native status", () => {
    for (const capability of capabilities) {
      expect(reminderCapabilityAvailable(healthyStatus, null, null, capability)).toBe(true);
    }
  });

  test("each false native capability remains disabled", () => {
    for (const capability of capabilities) {
      expect(
        reminderCapabilityAvailable(
          { ...healthyStatus, [capability]: false },
          null,
          null,
          capability
        )
      ).toBe(false);
    }
  });
});
