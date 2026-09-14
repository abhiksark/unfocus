// src/lib/developer-cue-preview.test.ts

import { describe, expect, test } from "bun:test";
import type { DiagnosticsReport } from "./diagnostics";
import * as developerCuePreviewModule from "./developer-cue-preview";
import {
  activateDeveloperCuePreview,
  clearDeveloperCuePreviewAtCloseBound,
  closeDeveloperCuePreview,
  createDeveloperCuePreviewController,
  developerCuePreviewVisible,
  initialDeveloperCuePreviewState,
  remainingDeveloperCuePreviewCloseMs,
  restoreDeveloperCuePreviewAfterCloseFailure,
  startDeveloperCuePreview
} from "./developer-cue-preview";

type PreviewResponse = { runId: number; closesAfterMs: number };

function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((resolvePromise) => {
    resolve = resolvePromise;
  });
  return { promise, resolve };
}

function controllerHarness(
  showPreview: () => Promise<PreviewResponse>,
  closePreview: (runId: number) => Promise<void> = async () => {}
) {
  let nowMs = 0;
  let nextTimerId = 1;
  const scheduled = new Map<number, { callback: () => void; delayMs: number }>();
  const cancelledTimers: number[] = [];
  const snapshots: Array<{
    state: ReturnType<typeof initialDeveloperCuePreviewState>;
    error: string | null;
  }> = [];
  const controller = createDeveloperCuePreviewController(
    {
      showPreview,
      closePreview,
      now: () => nowMs,
      schedule: (callback, delayMs) => {
        const timerId = nextTimerId++;
        scheduled.set(timerId, { callback, delayMs });
        return timerId;
      },
      cancel: (timerId) => {
        cancelledTimers.push(timerId);
        scheduled.delete(timerId);
      }
    },
    (snapshot) => snapshots.push({ state: { ...snapshot.state }, error: snapshot.error })
  );

  return {
    controller,
    snapshots,
    scheduled,
    cancelledTimers,
    setNow(value: number) {
      nowMs = value;
    }
  };
}

const macReport: DiagnosticsReport = {
  operatingSystem: "macos",
  sessionType: null,
  desktop: null,
  display: null,
  monitors: [],
  monitorError: null,
  idleSeconds: null,
  idleError: null,
  idleStatus: "pending",
  activeWindowFullscreen: null,
  fullscreenError: null,
  fullscreenStatus: "pending",
  storage: {
    activityHistory: { status: "available", recovery: "none", category: null, error: null },
    breakLedger: { status: "available", recovery: "none", category: null, error: null },
    reminderSettings: { status: "available", recovery: "none", category: null, error: null }
  },
  tray: { available: true, error: null }
};

describe("developer pre-break cue preview", () => {
  test("is visible on macOS and qualified X11, but not Wayland or unknown Linux", () => {
    expect(developerCuePreviewVisible(macReport)).toBe(true);
    expect(developerCuePreviewVisible({ ...macReport, operatingSystem: "linux", sessionType: "x11" })).toBe(true);
    expect(developerCuePreviewVisible({ ...macReport, operatingSystem: "linux", sessionType: "wayland" })).toBe(false);
    expect(developerCuePreviewVisible({ ...macReport, operatingSystem: "linux" })).toBe(false);
    expect(developerCuePreviewVisible(null)).toBe(false);
  });

  test("does not start a second preview while the first one is opening", () => {
    const opening = startDeveloperCuePreview(initialDeveloperCuePreviewState());

    expect(opening).toEqual({ phase: "opening", runId: null });
    expect(startDeveloperCuePreview(opening)).toEqual(opening);
  });

  test("retains the active preview when an early close request fails", () => {
    const active = activateDeveloperCuePreview(
      startDeveloperCuePreview(initialDeveloperCuePreviewState()),
      42
    );

    expect(closeDeveloperCuePreview(active)).toEqual({ phase: "closing", runId: 42 });
    expect(restoreDeveloperCuePreviewAfterCloseFailure({ phase: "closing", runId: 42 })).toEqual({
      phase: "active",
      runId: 42
    });
  });

  test("clears only the matching preview at its native close bound", () => {
    const active = activateDeveloperCuePreview(
      startDeveloperCuePreview(initialDeveloperCuePreviewState()),
      42
    );

    expect(clearDeveloperCuePreviewAtCloseBound(active, 41)).toEqual(active);
    expect(clearDeveloperCuePreviewAtCloseBound(active, 42)).toEqual(
      initialDeveloperCuePreviewState()
    );
  });

  test("subtracts delayed native response time from the remaining close bound", () => {
    expect(remainingDeveloperCuePreviewCloseMs(17_000, 3_750)).toBe(13_250);
    expect(remainingDeveloperCuePreviewCloseMs(17_000, 17_000)).toBe(0);
    expect(remainingDeveloperCuePreviewCloseMs(17_000, 20_000)).toBe(0);
  });

  test("activates only after the delayed native response and keeps the native close bound", async () => {
    const shown = deferred<PreviewResponse>();
    const harness = controllerHarness(() => shown.promise);

    const starting = harness.controller.start();
    expect(harness.snapshots.at(-1)).toEqual({
      state: { phase: "opening", runId: null },
      error: null
    });
    expect(harness.scheduled.size).toBe(0);

    harness.setNow(3_750);
    shown.resolve({ runId: 42, closesAfterMs: 17_000 });
    await starting;

    expect(harness.snapshots.at(-1)).toEqual({
      state: { phase: "active", runId: 42 },
      error: null
    });
    expect([...harness.scheduled.values()].map((timer) => timer.delayMs)).toEqual([13_250]);
  });

  test("does not reactivate a preview ended before its native show response", async () => {
    const shown = deferred<PreviewResponse>();
    const harness = controllerHarness(() => shown.promise);
    const starting = harness.controller.start();
    for (const runId of [42, 41]) {
      const endedRunId = developerCuePreviewModule.developerCuePreviewEndedRunId(
        new CustomEvent("native-end", { detail: { runId } }),
        harness.snapshots.at(-1)?.state.runId ?? null
      );
      if (endedRunId !== null) harness.controller.nativeEnded(endedRunId);
    }

    shown.resolve({ runId: 42, closesAfterMs: 17_000 });
    await starting;

    expect(harness.snapshots.at(-1)).toEqual({
      state: { phase: "idle", runId: null },
      error: null
    });
    expect(harness.snapshots.some(({ state }) => state.phase === "active")).toBe(false);
    expect(harness.scheduled.size).toBe(0);
  });

  test("ignores another run ending while a preview is opening", async () => {
    const shown = deferred<PreviewResponse>();
    const harness = controllerHarness(() => shown.promise);
    const starting = harness.controller.start();
    harness.controller.nativeEnded(41);

    shown.resolve({ runId: 42, closesAfterMs: 17_000 });
    await starting;

    expect(harness.snapshots.at(-1)).toEqual({
      state: { phase: "active", runId: 42 },
      error: null
    });
    expect(harness.scheduled.size).toBe(1);
  });

  test("does not carry opening end events into later preview requests", async () => {
    const first = deferred<PreviewResponse>();
    let calls = 0;
    const harness = controllerHarness(() => {
      calls += 1;
      return calls === 1
        ? first.promise
        : Promise.resolve({ runId: 43, closesAfterMs: 17_000 });
    });
    const starting = harness.controller.start();
    harness.controller.nativeEnded(43);
    first.resolve({ runId: 42, closesAfterMs: 17_000 });
    await starting;
    harness.controller.nativeEnded(42);

    await harness.controller.start();

    expect(harness.snapshots.at(-1)).toEqual({
      state: { phase: "active", runId: 43 },
      error: null
    });
    expect(harness.scheduled.size).toBe(1);
  });

  test("clears the matching preview automatically at the native close bound", async () => {
    const harness = controllerHarness(async () => ({ runId: 42, closesAfterMs: 17_000 }));
    await harness.controller.start();

    harness.scheduled.values().next().value?.callback();

    expect(harness.snapshots.at(-1)).toEqual({
      state: { phase: "idle", runId: null },
      error: null
    });
    expect(harness.scheduled.size).toBe(0);
  });

  test("closes only the active native preview run", async () => {
    const closedRuns: number[] = [];
    const harness = controllerHarness(
      async () => ({ runId: 42, closesAfterMs: 17_000 }),
      async (runId) => {
        closedRuns.push(runId);
      }
    );
    await harness.controller.start();

    await harness.controller.close();

    expect(closedRuns).toEqual([42]);
    expect(harness.snapshots.at(-1)).toEqual({
      state: { phase: "idle", runId: null },
      error: null
    });
  });

  test("restores the active preview when its native close fails", async () => {
    const harness = controllerHarness(
      async () => ({ runId: 42, closesAfterMs: 17_000 }),
      async () => {
        throw new Error("native close failed");
      }
    );
    await harness.controller.start();

    await harness.controller.close();

    expect(harness.snapshots.at(-1)).toEqual({
      state: { phase: "active", runId: 42 },
      error: "native close failed"
    });
    expect(harness.scheduled.size).toBe(1);
  });

  test("clears an active preview when native ownership ends first", async () => {
    const harness = controllerHarness(
      async () => ({ runId: 42, closesAfterMs: 17_000 }),
      async () => {
        throw new Error("native close failed");
      }
    );
    await harness.controller.start();
    await harness.controller.close();

    harness.controller.nativeEnded(42);

    expect(harness.snapshots.at(-1)).toEqual({
      state: { phase: "idle", runId: null },
      error: null
    });
    expect(harness.cancelledTimers).toEqual([1]);
  });

  test("accepts only a safe matching run ID from native lifecycle events", () => {
    const parseRunId = developerCuePreviewModule.developerCuePreviewEndedRunId;
    expect(
      parseRunId?.(new CustomEvent("native-end", { detail: { runId: 42 } }), 42)
    ).toBe(42);

    if (!parseRunId) return;

    for (const event of [
      new Event("native-end"),
      new CustomEvent("native-end", { detail: { runId: 41 } }),
      new CustomEvent("native-end", { detail: { runId: 0 } }),
      new CustomEvent("native-end", { detail: { runId: 4.2 } }),
      new CustomEvent("native-end", { detail: { runId: Number.MAX_SAFE_INTEGER + 1 } }),
      new CustomEvent("native-end", { detail: { runId: "42" } }),
      new CustomEvent("native-end", { detail: null })
    ]) {
      expect(parseRunId(event, 42)).toBeNull();
    }
  });

  test("accepts only safe native run IDs while the opening run is unknown", () => {
    const parseRunId = developerCuePreviewModule.developerCuePreviewEndedRunId;
    expect(parseRunId(new CustomEvent("native-end", { detail: { runId: 42 } }), null)).toBe(42);

    for (const runId of [0, -1, 4.2, Number.MAX_SAFE_INTEGER + 1, "42", null]) {
      expect(parseRunId(new CustomEvent("native-end", { detail: { runId } }), null)).toBeNull();
    }
  });

  test("cancels route-owned timers and ignores their callbacks after cleanup", async () => {
    const harness = controllerHarness(async () => ({ runId: 42, closesAfterMs: 17_000 }));
    await harness.controller.start();
    const timer = harness.scheduled.values().next().value;
    const snapshotCount = harness.snapshots.length;

    harness.controller.destroy();
    timer?.callback();

    expect(harness.cancelledTimers).toEqual([1]);
    expect(harness.snapshots).toHaveLength(snapshotCount);
  });
});
