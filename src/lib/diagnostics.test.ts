import { describe, expect, test } from "bun:test";
import {
  diagnosticsHealth,
  diagnosticsHealthLabel,
  probeBackend,
  type DiagnosticsReport
} from "./diagnostics";

const report: DiagnosticsReport = {
  operatingSystem: "linux",
  sessionType: "x11",
  desktop: null,
  display: ":0",
  monitors: [],
  monitorError: null,
  idleSeconds: 1,
  idleError: null,
  idleStatus: "available",
  activeWindowFullscreen: false,
  fullscreenError: null,
  fullscreenStatus: "available",
  storage: {
    activityHistory: { status: "available", recovery: "none", category: null, error: null },
    breakLedger: { status: "available", recovery: "none", category: null, error: null },
    reminderSettings: { status: "available", recovery: "none", category: null, error: null }
  },
  tray: {
    available: true,
    error: null
  }
};

describe("diagnostics presentation", () => {
  test("treats pending probe diagnostics as startup state rather than failure", () => {
    const pending: DiagnosticsReport = {
      ...report,
      idleSeconds: null,
      idleError: null,
      idleStatus: "pending",
      activeWindowFullscreen: null,
      fullscreenError: null,
      fullscreenStatus: "pending"
    };

    expect(diagnosticsHealth(pending, null)).toBe("healthy");
    expect(pending.idleError).toBeNull();
    expect(pending.fullscreenError).toBeNull();
  });

  test("does not present probe errors as healthy", () => {
    const degraded = { ...report, idleSeconds: null, idleError: "probe failed" };
    expect(diagnosticsHealth(degraded, null)).toBe("degraded");
    expect(diagnosticsHealthLabel("degraded")).toBe("Degraded");
  });

  test("degrades when monitor enumeration fails", () => {
    expect(diagnosticsHealth({ ...report, monitorError: "enumeration failed" }, null)).toBe(
      "degraded"
    );
  });

  test("degrades when the native tray is known to be unavailable", () => {
    expect(
      diagnosticsHealth(
        {
          ...report,
          tray: { available: false, error: "indicator construction failed" }
        },
        null
      )
    ).toBe("degraded");
  });

  test("degrades when the native tray is unavailable without a current error", () => {
    expect(
      diagnosticsHealth(
        {
          ...report,
          tray: { available: false, error: null }
        },
        null
      )
    ).toBe("degraded");
  });

  test("does not guess X11 for unsupported platforms", () => {
    expect(
      probeBackend({
        ...report,
        operatingSystem: "windows",
        sessionType: null,
        probeBackend: { kind: "unsupported" }
      })
    ).toBe("windows (unsupported)");
    expect(
      probeBackend({
        ...report,
        sessionType: "wayland",
        probeBackend: { kind: "unsupported" }
      })
    ).toBe("wayland (unsupported)");
  });

  test("labels the Sway candidate from the Rust backend field", () => {
    expect(
      probeBackend({
        ...report,
        sessionType: "wayland",
        desktop: "sway",
        probeBackend: { kind: "sway", version: "1.11", candidate: true }
      })
    ).toBe("Sway 1.11 (candidate)");
  });

  test("labels Win32 when the Rust backend reports it", () => {
    expect(
      probeBackend({
        ...report,
        operatingSystem: "windows",
        sessionType: "win32",
        probeBackend: { kind: "win32" }
      })
    ).toBe("Win32");
  });

  test("storage diagnostics retain technical category and degrade health", () => {
    const degraded: DiagnosticsReport = {
      ...report,
      storage: {
        ...report.storage,
        activityHistory: {
          status: "unavailable",
          recovery: "retryOrStartNew",
          category: "invalid",
          error: "activity-history.json contains unsupported version 9"
        }
      }
    };

    expect(diagnosticsHealth(degraded, null)).toBe("degraded");
    expect(degraded.storage.activityHistory.category).toBe("invalid");
    expect(degraded.storage.activityHistory.error).toContain("unsupported version");
  });

  test("reminder settings diagnostics retain technical load detail", () => {
    const degraded: DiagnosticsReport = {
      ...report,
      storage: {
        ...report.storage,
        reminderSettings: {
          status: "unavailable",
          recovery: "retryOrStartNew",
          category: "invalid",
          error: "/private/config/reminder-settings.json: unsupported version 9"
        }
      }
    };

    expect(diagnosticsHealth(degraded, null)).toBe("degraded");
    expect(degraded.storage.reminderSettings.error).toContain("reminder-settings.json");
  });

  test("transport failure is unavailable even with a stale report", () => {
    expect(diagnosticsHealth(report, "IPC failed")).toBe("unavailable");
  });
});
