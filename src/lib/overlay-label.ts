export const MIN_OVERLAY_DURATION_SECONDS = 3;
export const MAX_OVERLAY_DURATION_SECONDS = 30;
export const MAX_OVERLAY_MONITORS = 64;

export type OverlayParameters = {
  runId: number;
  monitorIndex: number;
  monitorCount: number;
  durationSeconds: number;
  deadlineMs: number;
};

export type WindowRoute =
  | { kind: "dashboard" }
  | { kind: "overlay"; parameters: OverlayParameters }
  | { kind: "invalid-overlay"; reason: string };

const CANONICAL_DECIMAL = /^(0|[1-9]\d*)$/;

function integerField(
  value: string,
  name: string,
  minimum: number,
  maximum: number
): number | string {
  if (!CANONICAL_DECIMAL.test(value)) return `${name} is not a canonical decimal integer`;

  const parsed = Number(value);
  if (!Number.isSafeInteger(parsed) || parsed < minimum || parsed > maximum) {
    return `${name} is outside its supported range`;
  }

  return parsed;
}

export function parseWindowLabel(label: string): WindowRoute {
  if (!label.startsWith("overlay-")) return { kind: "dashboard" };

  const parts = label.split("-");
  if (parts.length !== 6 || parts[0] !== "overlay") {
    return { kind: "invalid-overlay", reason: "overlay labels require exactly five fields" };
  }

  const runId = integerField(parts[1], "run ID", 1, Number.MAX_SAFE_INTEGER);
  const monitorIndex = integerField(parts[2], "monitor index", 0, Number.MAX_SAFE_INTEGER);
  const monitorCount = integerField(parts[3], "monitor count", 1, MAX_OVERLAY_MONITORS);
  const durationSeconds = integerField(
    parts[4],
    "duration",
    MIN_OVERLAY_DURATION_SECONDS,
    MAX_OVERLAY_DURATION_SECONDS
  );
  const deadlineMs = integerField(parts[5], "deadline", 1, Number.MAX_SAFE_INTEGER);

  if (
    typeof runId === "string" ||
    typeof monitorIndex === "string" ||
    typeof monitorCount === "string" ||
    typeof durationSeconds === "string" ||
    typeof deadlineMs === "string"
  ) {
    const invalid = [runId, monitorIndex, monitorCount, durationSeconds, deadlineMs].find(
      (value): value is string => typeof value === "string"
    );
    return { kind: "invalid-overlay", reason: invalid ?? "overlay label is invalid" };
  }

  if (monitorIndex >= monitorCount) {
    return {
      kind: "invalid-overlay",
      reason: "monitor index must be smaller than monitor count"
    };
  }

  return {
    kind: "overlay",
    parameters: { runId, monitorIndex, monitorCount, durationSeconds, deadlineMs }
  };
}
