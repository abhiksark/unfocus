import type { ReminderStatus } from "./reminder-status";

export interface TrayPanelSnapshot {
  reminder: ReminderStatus;
  pending: { requestId: number; remainingMs: number } | null;
  openingGeneration: number;
  countdownRequestId: number | null;
  visible: boolean;
}
export type TrayPanelAction = "begin" | "cancel" | "pause" | "resume" | "open" | "hide" | "quit";
export interface TrayPanelView {
  snapshot: TrayPanelSnapshot | null;
  sampledAt: number;
  busy: boolean;
  error: string | null;
}
export function panelCountdown(view: TrayPanelView, now: number): number | null {
  const state = view.snapshot;
  if (!state?.visible || !state.pending || state.countdownRequestId !== state.pending.requestId) return null;
  return Math.max(0, Math.ceil((state.pending.remainingMs - Math.max(0, now - view.sampledAt)) / 1000));
}
export function panelError(view: TrayPanelView): string | null {
  return view.error ?? (view.snapshot?.reminder.actionError
    ? "The last action failed. Open Unfocus to check."
    : null);
}
export function panelControls(view: TrayPanelView) {
  const state = view.snapshot;
  const available = !!state?.visible && !view.busy && state.reminder.phase !== "unavailable";
  return {
    pause: available && !state?.pending && state?.reminder.pauseActionEnabled === true,
    begin: available && !state?.pending && state?.reminder.takeBreakEnabled === true,
    cancel: available && !!state?.pending && state.countdownRequestId === state.pending.requestId
  };
}

/** Owns publication only. Native code owns deadlines and dispatches the break. */
export function createTrayPanelController(deps: {
  read: () => Promise<TrayPanelSnapshot>;
  act: (action: TrayPanelAction, generation: number, requestId?: number) => Promise<TrayPanelSnapshot>;
  ready: (generation: number) => Promise<TrayPanelSnapshot>;
  paint: () => Promise<void>;
  now: () => number;
}, publish: (view: TrayPanelView) => void) {
  let view: TrayPanelView = { snapshot: null, sampledAt: 0, busy: false, error: null };
  let epoch = 0;
  let sequence = 0;
  let generation = 0;
  let destroyed = false;
  let pendingOpening: number | null = null;
  const emit = () => { if (!destroyed) publish({ ...view }); };
  function apply(snapshot: TrayPanelSnapshot) {
    if (snapshot.openingGeneration < generation) return false;
    generation = snapshot.openingGeneration;
    view = { ...view, snapshot, sampledAt: deps.now() };
    emit();
    return true;
  }
  async function refresh(opening = false) {
    if (destroyed || view.busy || (!opening && (pendingOpening !== null || !view.snapshot?.visible))) return;
    const currentEpoch = epoch;
    const request = ++sequence;
    try {
      const snapshot = await deps.read();
      if (destroyed || currentEpoch !== epoch || request !== sequence || !apply(snapshot)) return;
      if (opening && snapshot.visible) {
        await deps.paint();
        if (destroyed || currentEpoch !== epoch || request !== sequence) return;
        const ready = await deps.ready(snapshot.openingGeneration);
        if (!destroyed && currentEpoch === epoch && request === sequence) apply(ready);
      }
    } catch {
      if (!destroyed && currentEpoch === epoch && request === sequence) {
        view = { ...view, error: "Could not refresh. Open Unfocus to check." };
        emit();
      }
    } finally {
      if (opening && pendingOpening === currentEpoch) pendingOpening = null;
    }
  }
  return {
    refresh: () => refresh(),
    open(nextGeneration?: number) {
      if (destroyed || (nextGeneration !== undefined && nextGeneration < generation)) return;
      generation = nextGeneration ?? generation;
      epoch++;
      pendingOpening = epoch;
      view = { snapshot: null, sampledAt: 0, busy: false, error: null };
      emit();
      return refresh(true);
    },
    hidden(nextGeneration: number) {
      if (nextGeneration < generation || destroyed) return false;
      generation = nextGeneration;
      epoch++;
      pendingOpening = null;
      view = { ...view, busy: false, error: null,
        snapshot: view.snapshot ? { ...view.snapshot, visible: false, countdownRequestId: null } : null };
      emit();
      return true;
    },
    async action(action: TrayPanelAction) {
      if (destroyed || view.busy) return;
      const controls = panelControls(view);
      if ((action === "begin" && !controls.begin) ||
          ((action === "pause" || action === "resume") && !controls.pause) ||
          (action === "cancel" && !controls.cancel)) return;
      const requestId = view.snapshot?.pending?.requestId;
      const currentEpoch = ++epoch;
      sequence++;
      view = { ...view, busy: true, error: null };
      emit();
      try {
        const snapshot = await deps.act(action, generation, requestId);
        if (!destroyed && currentEpoch === epoch) apply(snapshot);
      } catch {
        if (!destroyed && currentEpoch === epoch) {
          view = { ...view, error: action === "cancel" ? "Could not cancel. The break may still start." : "That action did not finish. Try again." };
        }
      } finally {
        if (!destroyed && currentEpoch === epoch) {
          view = { ...view, busy: false };
          emit();
        }
      }
    },
    destroy() { destroyed = true; epoch++; sequence++; }
  };
}
