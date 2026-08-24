export const BREAK_SCENE_IMAGE_URL = "/break-scene.jpg";

export type BreakScenePhase = "resting" | "returning";
export type BreakScenePeriod = "dawn" | "day" | "dusk" | "night";

export function breakScenePeriodAt(date: Date): BreakScenePeriod {
  const localHour = date.getHours();
  if (!Number.isInteger(localHour)) return "day";
  if (localHour >= 5 && localHour < 9) return "dawn";
  if (localHour >= 9 && localHour < 17) return "day";
  if (localHour >= 17 && localHour < 21) return "dusk";
  return "night";
}

export function breakScenePeriodForRun(
  deadlineMs: number,
  durationSeconds: number
): BreakScenePeriod {
  return breakScenePeriodAt(new Date(deadlineMs - durationSeconds * 1_000));
}

export function breakScenePhase(state: {
  complete: boolean;
  finalSeconds: boolean;
}): BreakScenePhase {
  return state.complete || state.finalSeconds ? "returning" : "resting";
}
