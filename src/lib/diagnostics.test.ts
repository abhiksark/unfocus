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

  test("transport failure is unavailable even with a stale report", () => {
    expect(diagnosticsHealth(report, "IPC failed")).toBe("unavailable");
  });
});
