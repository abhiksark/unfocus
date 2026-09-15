// src/lib/start-at-login.ts

export type StartAtLoginStatus = { supported: boolean; enabled: boolean };
export type StartAtLoginState = {
  status: StartAtLoginStatus | null;
  pending: boolean;
  error: string | null;
  dismissed: boolean;
};

export const STARTUP_PROMPT_KEY = "unfocus.start-at-login-prompt.v1";

export function initialStartAtLoginState(): StartAtLoginState {
  return { status: null, pending: false, error: null, dismissed: false };
}

/** Startup registration and its prompt never read or save reminder settings. */
export function createStartAtLoginController(
  native: {
    get: () => Promise<StartAtLoginStatus>;
    set: (enabled: boolean) => Promise<StartAtLoginStatus>;
  },
  storage: () => Pick<Storage, "getItem" | "setItem"> | null,
  publish: (state: StartAtLoginState) => void
) {
  let state = initialStartAtLoginState();
  function update(changes: Partial<StartAtLoginState>) {
    state = { ...state, ...changes };
    publish(state);
  }
  function dismiss() {
    update({ dismissed: true });
    try {
      storage()?.setItem(STARTUP_PROMPT_KEY, "dismissed");
    } catch {
      // Session dismissal still works when browser storage is unavailable.
    }
  }
  async function request(enabled?: boolean) {
    if (state.pending) return;
    update({ pending: true, error: null });
    try {
      const status = enabled === undefined ? await native.get() : await native.set(enabled);
      if (enabled !== undefined && (!status.supported || status.enabled !== enabled)) {
        throw new Error("The requested startup setting could not be confirmed.");
      }
      update({ status });
      if (status.enabled || enabled !== undefined) dismiss();
    } catch (error) {
      update({
        status: enabled === undefined ? null : state.status,
        error: error instanceof Error ? error.message : String(error)
      });
    } finally {
      update({ pending: false });
    }
  }
  return {
    async load() {
      try {
        if (storage()?.getItem(STARTUP_PROMPT_KEY) === "dismissed") {
          update({ dismissed: true });
        }
      } catch {
        // A missing or inaccessible marker gets the same first-run prompt.
      }
      await request();
    },
    set: (enabled: boolean) => request(enabled),
    dismiss
  };
}
