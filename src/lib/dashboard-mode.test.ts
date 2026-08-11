import { describe, expect, test } from "bun:test";
import {
  DASHBOARD_MODE_STORAGE_KEY,
  readDashboardMode,
  writeDashboardMode,
  type DashboardMode
} from "./dashboard-mode";

function storageWith(value: string | null) {
  return {
    getItem(key: string) {
      expect(key).toBe(DASHBOARD_MODE_STORAGE_KEY);
      return value;
    }
  };
}

describe("dashboard mode persistence", () => {
  test("defaults missing and corrupt values to consumer mode", () => {
    expect(readDashboardMode(storageWith(null))).toBe("consumer");
    expect(readDashboardMode(storageWith("diagnostics"))).toBe("consumer");
  });

  test.each<DashboardMode>(["consumer", "developer"])(
    "restores the valid %s mode",
    (mode) => {
      expect(readDashboardMode(storageWith(mode))).toBe(mode);
    }
  );

  test("defaults safely when storage cannot be read", () => {
    expect(
      readDashboardMode({
        getItem() {
          throw new Error("storage denied");
        }
      })
    ).toBe("consumer");
    expect(readDashboardMode(null)).toBe("consumer");
  });

  test("reports write failure without preventing an in-session mode change", () => {
    let sessionMode: DashboardMode = "consumer";
    sessionMode = "developer";

    expect(
      writeDashboardMode(
        {
          setItem() {
            throw new Error("quota exceeded");
          }
        },
        sessionMode
      )
    ).toBe(false);
    expect(sessionMode).toBe("developer");
    expect(writeDashboardMode(null, sessionMode)).toBe(false);
  });

  test("writes the versioned key", () => {
    let saved: [string, string] | null = null;
    expect(
      writeDashboardMode(
        {
          setItem(key, value) {
            saved = [key, value];
          }
        },
        "developer"
      )
    ).toBe(true);
    expect(saved as [string, string] | null).toEqual([
      DASHBOARD_MODE_STORAGE_KEY,
      "developer"
    ]);
  });
});
