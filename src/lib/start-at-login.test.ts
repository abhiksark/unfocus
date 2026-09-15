// src/lib/start-at-login.test.ts

import { expect, test } from "bun:test";
import {
  createStartAtLoginController, initialStartAtLoginState, STARTUP_PROMPT_KEY,
  type StartAtLoginStatus
} from "./start-at-login";

function harness(options: {
  storage?: Pick<Storage, "getItem" | "setItem"> | null;
  get?: () => Promise<StartAtLoginStatus>;
  set?: (enabled: boolean) => Promise<StartAtLoginStatus>;
} = {}) {
  let state = initialStartAtLoginState();
  const writes: boolean[] = [];
  const controller = createStartAtLoginController({
    get: options.get ?? (async () => ({ supported: true, enabled: false })),
    set: async (enabled) => {
      writes.push(enabled);
      return options.set ? options.set(enabled) : { supported: true, enabled };
    }
  }, () => options.storage ?? null, (next) => { state = next; });
  return { controller, writes, get state() { return state; } };
}

test("new and existing installations are asked once, across reopening and upgrades", async () => {
  const values = new Map<string, string>([["unfocus.dashboard-mode.v1", "developer"]]);
  const storage = {
    getItem: (key: string) => values.get(key) ?? null,
    setItem: (key: string, value: string) => { values.set(key, value); }
  };
  const first = harness({ storage });
  await first.controller.load();
  expect(first.state.dismissed).toBe(false);
  expect(first.writes).toEqual([]);
  first.controller.dismiss();
  expect(values.get(STARTUP_PROMPT_KEY)).toBe("dismissed");
  const reopened = harness({ storage });
  await reopened.controller.load();
  expect(reopened.state.dismissed).toBe(true);
  expect(values.get("unfocus.dashboard-mode.v1")).toBe("developer");
});

test("storage denial still allows session dismissal without registering startup", async () => {
  const app = harness({ storage: {
    getItem() { throw new Error("denied"); },
    setItem() { throw new Error("denied"); }
  } });
  await app.controller.load();
  app.controller.dismiss();
  await app.controller.load();
  expect(app.state.dismissed).toBe(true);
  expect(app.writes).toEqual([]);
});

test("already enabled startup skips the prompt; unsupported systems do not opt in", async () => {
  const enabled = harness({ get: async () => ({ supported: true, enabled: true }) });
  await enabled.controller.load();
  expect(enabled.state.dismissed).toBe(true);
  expect(enabled.writes).toEqual([]);
  const unsupported = harness({ get: async () => ({ supported: false, enabled: false }) });
  await unsupported.controller.load();
  expect(unsupported.state.status?.supported).toBe(false);
  expect(unsupported.writes).toEqual([]);
});

test("registration failure stays unconfirmed and retry closes only after success", async () => {
  let fail = true;
  const app = harness({ set: async (enabled) => {
    if (fail) throw new Error("permission denied");
    return { supported: true, enabled };
  } });
  await app.controller.load();
  await app.controller.set(true);
  expect(app.state.error).toBe("permission denied");
  expect(app.state.status?.enabled).toBe(false);
  expect(app.state.dismissed).toBe(false);
  fail = false;
  await app.controller.set(true);
  expect(app.state.error).toBeNull();
  expect(app.state.dismissed).toBe(true);
  expect(app.state.status?.enabled).toBe(true);
  await app.controller.set(false);
  expect(app.state.status?.enabled).toBe(false);
  expect(app.writes).toEqual([true, true, false]);
});

test("read failures stay unknown and a mismatched write response is not success", async () => {
  const app = harness({ get: async () => { throw new Error("unreadable"); },
    set: async () => ({ supported: true, enabled: false }) });
  await app.controller.load();
  expect(app.state.status).toBeNull();
  expect(app.state.error).toBe("unreadable");
  await app.controller.set(true);
  expect(app.state.error).toContain("could not be confirmed");
  expect(app.state.dismissed).toBe(false);
});

test("pending writes are serialized and preserve dismissal while a request finishes", async () => {
  let resolve!: (status: StartAtLoginStatus) => void;
  const app = harness({ set: () => new Promise((done) => { resolve = done; }) });
  await app.controller.load();
  const pending = app.controller.set(true);
  await app.controller.load();
  await app.controller.set(false);
  expect(app.state.pending).toBe(true);
  expect(app.state.status?.enabled).toBe(false);
  expect(app.writes).toEqual([true]);
  app.controller.dismiss();
  resolve({ supported: true, enabled: true });
  await pending;
  expect(app.state.pending).toBe(false);
  expect(app.state.dismissed).toBe(true);
});
