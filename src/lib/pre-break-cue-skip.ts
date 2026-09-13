export type CueSkipState = "idle" | "pending" | "skipped" | "retracting";

export function createCueSkipController(actions: {
  request: () => Promise<void>;
  changed: (state: CueSkipState) => void;
  failed: (error: unknown) => void;
  afterConfirmation?: (callback: () => void) => () => void;
}) {
  let state: CueSkipState = "idle";
  let disposed = false;
  let cancelConfirmation: (() => void) | undefined;
  function change(next: CueSkipState) { state = next; actions.changed(next); }
  return {
    async skip() {
      if (disposed || state !== "idle") return;
      change("pending");
      try {
        await actions.request();
        if (disposed) return;
        change("skipped");
        const retract = () => { if (!disposed) change("retracting"); };
        if (actions.afterConfirmation) cancelConfirmation = actions.afterConfirmation(retract);
        else {
          const timer = setTimeout(retract, 450);
          cancelConfirmation = () => clearTimeout(timer);
        }
      } catch (error) {
        if (disposed) return;
        change("idle");
        actions.failed(error);
      }
    },
    dispose() { disposed = true; cancelConfirmation?.(); }
  };
}
