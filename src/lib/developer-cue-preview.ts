import type { DiagnosticsReport } from "./diagnostics";

export type DeveloperCuePreviewPhase = "idle" | "opening" | "active" | "closing";

export const DEVELOPER_CUE_PREVIEW_ENDED_EVENT = "unfocus-pre-break-cue-preview-ended";

export type DeveloperCuePreviewState = {
  phase: DeveloperCuePreviewPhase;
  runId: number | null;
};

export function developerCuePreviewVisible(report: DiagnosticsReport | null): boolean {
  return report?.operatingSystem === "macos";
}

export function initialDeveloperCuePreviewState(): DeveloperCuePreviewState {
  return { phase: "idle", runId: null };
}

export function startDeveloperCuePreview(
  state: DeveloperCuePreviewState
): DeveloperCuePreviewState {
  return state.phase === "idle" ? { phase: "opening", runId: null } : state;
}

export function activateDeveloperCuePreview(
  state: DeveloperCuePreviewState,
  runId: number
): DeveloperCuePreviewState {
  return state.phase === "opening" ? { phase: "active", runId } : state;
}

export function closeDeveloperCuePreview(
  state: DeveloperCuePreviewState
): DeveloperCuePreviewState {
  return state.phase === "active" ? { phase: "closing", runId: state.runId } : state;
}

export function restoreDeveloperCuePreviewAfterCloseFailure(
  state: DeveloperCuePreviewState
): DeveloperCuePreviewState {
  return state.phase === "closing" ? { phase: "active", runId: state.runId } : state;
}

export function clearDeveloperCuePreviewAtCloseBound(
  state: DeveloperCuePreviewState,
  runId: number
): DeveloperCuePreviewState {
  return state.runId === runId ? initialDeveloperCuePreviewState() : state;
}

export function remainingDeveloperCuePreviewCloseMs(
  closesAfterMs: number,
  elapsedRequestMs: number
): number {
  if (!Number.isFinite(closesAfterMs) || !Number.isFinite(elapsedRequestMs)) return 0;
  return Math.max(0, closesAfterMs - Math.max(0, elapsedRequestMs));
}

export function developerCuePreviewEndedRunId(
  event: Event,
  activeRunId: number | null
): number | null {
  if (!(event instanceof CustomEvent)) return null;
  const runId = (event.detail as { runId?: unknown } | null)?.runId;
  // Opening has no run ID yet; the controller correlates these events with its response.
  return typeof runId === "number" &&
    Number.isSafeInteger(runId) &&
    runId > 0 &&
    (activeRunId === null || runId === activeRunId)
    ? runId
    : null;
}

export type DeveloperCuePreviewResponse = {
  runId: number;
  closesAfterMs: number;
};

export type DeveloperCuePreviewSnapshot = {
  state: DeveloperCuePreviewState;
  error: string | null;
};

type DeveloperCuePreviewControllerDependencies = {
  showPreview: () => Promise<DeveloperCuePreviewResponse>;
  closePreview: (runId: number) => Promise<void>;
  now: () => number;
  schedule: (callback: () => void, delayMs: number) => number;
  cancel: (timerId: number) => void;
};

export type DeveloperCuePreviewController = {
  start: () => Promise<void>;
  close: () => Promise<void>;
  nativeEnded: (runId: number) => void;
  destroy: () => void;
};

export function createDeveloperCuePreviewController(
  dependencies: DeveloperCuePreviewControllerDependencies,
  onChange: (snapshot: DeveloperCuePreviewSnapshot) => void
): DeveloperCuePreviewController {
  let state = initialDeveloperCuePreviewState();
  let error: string | null = null;
  let closeTimer: number | undefined;
  let destroyed = false;
  const endedWhileOpening = new Set<number>();

  const publish = () => onChange({ state, error });
  const clearCloseTimer = () => {
    if (closeTimer === undefined) return;
    dependencies.cancel(closeTimer);
    closeTimer = undefined;
  };
  const clearAtCloseBound = (runId: number) => {
    if (destroyed || state.runId !== runId) return;
    clearCloseTimer();
    state = clearDeveloperCuePreviewAtCloseBound(state, runId);
    error = null;
    publish();
  };

  return {
    async start() {
      if (destroyed) return;
      const nextState = startDeveloperCuePreview(state);
      if (nextState === state) return;

      state = nextState;
      error = null;
      endedWhileOpening.clear();
      publish();
      const requestStartedAt = dependencies.now();
      try {
        const preview = await dependencies.showPreview();
        if (destroyed) return;
        // Native preemption may arrive before the show response identifies this run.
        const alreadyEnded = endedWhileOpening.has(preview.runId);
        endedWhileOpening.clear();
        if (alreadyEnded) {
          state = initialDeveloperCuePreviewState();
          publish();
          return;
        }
        state = activateDeveloperCuePreview(state, preview.runId);
        if (state.runId !== preview.runId) return;
        publish();

        const remainingMs = remainingDeveloperCuePreviewCloseMs(
          preview.closesAfterMs,
          dependencies.now() - requestStartedAt
        );
        clearCloseTimer();
        if (remainingMs === 0) {
          clearAtCloseBound(preview.runId);
        } else {
          closeTimer = dependencies.schedule(
            () => clearAtCloseBound(preview.runId),
            remainingMs
          );
        }
      } catch (value) {
        if (destroyed) return;
        endedWhileOpening.clear();
        state = initialDeveloperCuePreviewState();
        error = value instanceof Error ? value.message : String(value);
        publish();
      }
    },

    async close() {
      if (destroyed) return;
      const nextState = closeDeveloperCuePreview(state);
      if (nextState === state || nextState.runId === null) return;

      state = nextState;
      error = null;
      publish();
      const runId = nextState.runId;
      try {
        await dependencies.closePreview(runId);
        if (destroyed) return;
        clearAtCloseBound(runId);
      } catch (value) {
        if (destroyed) return;
        if (state.phase !== "closing" || state.runId !== runId) return;
        state = restoreDeveloperCuePreviewAfterCloseFailure(state);
        error = value instanceof Error ? value.message : String(value);
        publish();
      }
    },

    nativeEnded(runId) {
      if (destroyed) return;
      if (state.phase === "opening") {
        endedWhileOpening.add(runId);
        return;
      }
      clearAtCloseBound(runId);
    },

    destroy() {
      destroyed = true;
      endedWhileOpening.clear();
      clearCloseTimer();
    }
  };
}
