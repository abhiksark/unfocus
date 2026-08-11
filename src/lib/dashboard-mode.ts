export type DashboardMode = "consumer" | "developer";

export const DASHBOARD_MODE_STORAGE_KEY = "unfocus.dashboard-mode.v1";

type ReadableStorage = Pick<Storage, "getItem">;
type WritableStorage = Pick<Storage, "setItem">;

export function readDashboardMode(storage: ReadableStorage | null): DashboardMode {
  if (!storage) return "consumer";

  try {
    const value = storage.getItem(DASHBOARD_MODE_STORAGE_KEY);
    return value === "consumer" || value === "developer" ? value : "consumer";
  } catch {
    return "consumer";
  }
}

export function writeDashboardMode(
  storage: WritableStorage | null,
  mode: DashboardMode
): boolean {
  if (!storage) return false;

  try {
    storage.setItem(DASHBOARD_MODE_STORAGE_KEY, mode);
    return true;
  } catch {
    return false;
  }
}
