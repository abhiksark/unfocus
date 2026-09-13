// Serialize native visibility changes so a cancelled preparation cannot reveal a stale cue.
import type { PreBreakCueLayout } from "./pre-break-cue";

type CueNativeActions = {
  prepare?: () => Promise<PreBreakCueLayout>;
  applyLayout: (layout: PreBreakCueLayout) => Promise<void>;
  setVisible: (visible: boolean) => Promise<void>;
  onError: (error: unknown) => void;
};

export function createCueVisibilityController(actions: CueNativeActions) {
  let revision = 0;
  let queue = Promise.resolve();
  let disposed = false;

  function update(visible: boolean): Promise<void> {
    const request = ++revision;
    queue = queue.then(async () => {
      if (request !== revision) return;
      try {
        if (visible && actions.prepare) {
          const layout = await actions.prepare();
          if (request !== revision) return;
          await actions.applyLayout(layout);
          if (request !== revision) return;
        }
        await actions.setVisible(visible);
      } catch (error) {
        actions.onError(error);
        // A failed geometry/layout handshake must leave no stale native surface.
        try { await actions.setVisible(false); } catch (hideError) { actions.onError(hideError); }
      }
    });
    return queue;
  }

  return {
    setVisible(visible: boolean) {
      return disposed ? queue : update(visible);
    },
    dispose() {
      disposed = true;
      return update(false);
    }
  };
}
