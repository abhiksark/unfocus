export type MonitorReport = {
  name: string | null;
  x: number;
  y: number;
  width: number;
  height: number;
  scaleFactor: number;
};

/** Discriminated backend from Rust; never invent support from environment alone. */
export type ProbeBackend =
  | { kind: "x11" }
  | { kind: "quartz" }
  | { kind: "win32" }
  | { kind: "sway"; version: string; candidate: boolean }
  | { kind: "unsupported" };

export type DiagnosticsReport = {
  operatingSystem: string;
  sessionType: string | null;
  desktop: string | null;
  display: string | null;
  monitors: MonitorReport[];
  monitorError: string | null;
  idleSeconds: number | null;
  idleError: string | null;
  activeWindowFullscreen: boolean | null;
  fullscreenError: string | null;
  /** Present on current builds; older reports may omit it. */
  probeBackend?: ProbeBackend | null;
  tray: {
    available: boolean;
    error: string | null;
  };
};

export type DiagnosticsHealth = "connecting" | "healthy" | "degraded" | "unavailable";

export function diagnosticsHealth(
  report: DiagnosticsReport | null,
  transportError: string | null
): DiagnosticsHealth {
  if (transportError) return "unavailable";
  if (!report) return "connecting";
  if (
    !report.tray.available ||
    report.monitorError ||
    report.idleError ||
    report.fullscreenError ||
    report.tray.error
  ) {
    return "degraded";
  }
  return "healthy";
}

export function diagnosticsHealthLabel(health: DiagnosticsHealth): string {
  switch (health) {
    case "healthy":
      return "Live";
    case "degraded":
      return "Degraded";
    case "unavailable":
      return "Unavailable";
    default:
      return "Connecting";
  }
}

export function probeBackend(report: DiagnosticsReport | null): string {
  if (!report) return "native probes";

  const backend = report.probeBackend;
  if (backend) {
    switch (backend.kind) {
      case "x11":
        return "X11";
      case "quartz":
        return "Quartz";
      case "win32":
        return "Win32";
      case "sway":
        return backend.candidate
          ? `Sway ${backend.version} (candidate)`
          : `Sway ${backend.version}`;
      case "unsupported": {
        const platform = report.sessionType?.trim() || report.operatingSystem;
        return `${platform} (unsupported)`;
      }
    }
  }

  // Fallback for older reports without probeBackend.
  if (report.operatingSystem === "macos") return "Quartz";
  if (report.operatingSystem === "windows") return "Win32";
  if (
    report.operatingSystem === "linux" &&
    report.sessionType?.trim().toLowerCase() === "x11"
  ) {
    return "X11";
  }

  const platform = report.sessionType?.trim() || report.operatingSystem;
  return `${platform} (unsupported)`;
}
