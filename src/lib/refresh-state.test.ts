import { describe, expect, test } from "bun:test";
import {
  refreshDisplayAsOfMs,
  refreshFailed,
  refreshLoading,
  refreshSucceeded,
  refreshUnavailable,
  type RefreshState
} from "./refresh-state";

describe("refresh state", () => {
  test("starts in loading without data, error, or capture time", () => {
    expect(refreshLoading<object>()).toEqual({
      status: "loading",
      data: null,
      error: null,
      asOfMs: null
    });
  });

  test("loading rejection becomes unavailable", () => {
    expect(refreshFailed(refreshLoading<object>(), "IPC unavailable")).toEqual({
      status: "unavailable",
      data: null,
      error: "IPC unavailable",
      asOfMs: null
    });
  });

  test("an unavailable rejection stays unavailable without data", () => {
    const state = refreshFailed(refreshUnavailable<object>("first"), "second");

    expect(state).toEqual({
      status: "unavailable",
      data: null,
      error: "second",
      asOfMs: null
    });
  });

  test("repeated rejection retains the exact payload and successful capture time", () => {
    const data = { activeSeconds: 42 };
    const fresh = refreshSucceeded(data, 1_000);
    const once = refreshFailed(fresh, "first failure");
    const twice = refreshFailed(once, "second failure");

    expect(twice).toEqual({
      status: "stale",
      data,
      error: "second failure",
      asOfMs: 1_000
    });
    expect(once.data).toBe(data);
    expect(twice.data).toBe(data);
    expect(refreshDisplayAsOfMs(once, 9_000)).toBe(1_000);
    expect(refreshDisplayAsOfMs(twice, 15_000)).toBe(1_000);
  });

  test("stale to fresh captures the new payload and timestamp", () => {
    const previous = refreshFailed(refreshSucceeded({ value: 1 }, 1_000), "failed");
    const recovered = { value: 2 };
    const fresh = refreshSucceeded(recovered, 8_000);

    expect(fresh).toEqual({
      status: "fresh",
      data: recovered,
      error: null,
      asOfMs: 8_000
    });
    expect(fresh.data).toBe(recovered);
    expect(refreshDisplayAsOfMs(fresh, 9_000)).toBe(9_000);
    expect(previous.status).toBe("stale");
  });

  test("any valid success replaces unavailable state", () => {
    const unavailable: RefreshState<{ value: number }> = refreshUnavailable("offline");
    const recovered = refreshSucceeded({ value: 3 }, 3_000);

    expect(unavailable.status).toBe("unavailable");
    expect(recovered.status).toBe("fresh");
    expect(recovered.error).toBeNull();
  });
});
