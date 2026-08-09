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
  activeWindowFullscreen: false,
  fullscreenError: null,
  tray: {
    available: true,
    error: null
  }
};

describe("diagnostics presentation", () => {
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

  test("does not guess X11 for unsupported platforms", () => {
    expect(probeBackend({ ...report, operatingSystem: "windows", sessionType: null })).toBe(
      "windows (unsupported)"
    );
    expect(probeBackend({ ...report, sessionType: "wayland" })).toBe("wayland (unsupported)");
  });

  test("transport failure is unavailable even with a stale report", () => {
    expect(diagnosticsHealth(report, "IPC failed")).toBe("unavailable");
  });
});
