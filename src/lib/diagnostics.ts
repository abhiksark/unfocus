export type MonitorReport = {
  name: string | null;
  x: number;
  y: number;
  width: number;
  height: number;
  scaleFactor: number;
};

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
};

export type DiagnosticsHealth = "connecting" | "healthy" | "degraded" | "unavailable";

export function diagnosticsHealth(
  report: DiagnosticsReport | null,
  transportError: string | null
): DiagnosticsHealth {
  if (transportError) return "unavailable";
  if (!report) return "connecting";
  if (report.monitorError || report.idleError || report.fullscreenError) return "degraded";
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
  if (report.operatingSystem === "macos") return "Quartz";
  if (
    report.operatingSystem === "linux" &&
    report.sessionType?.trim().toLowerCase() === "x11"
  ) {
    return "X11";
  }

  const platform = report.sessionType?.trim() || report.operatingSystem;
  return `${platform} (unsupported)`;
}
