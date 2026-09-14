import { describe, expect, test } from "bun:test";
import { createTrayPanelController, panelControls, panelCountdown, panelError, type TrayPanelSnapshot, type TrayPanelView } from "./tray-panel";
import type { ReminderStatus } from "./reminder-status";
const snapshot = (generation = 1): TrayPanelSnapshot => ({
  reminder: { phase: "working", pauseActionEnabled: true, takeBreakEnabled: true, stateRevision: 1 } as ReminderStatus,
  pending: null, openingGeneration: generation, countdownRequestId: null, visible: true
});
const deferred = <T>() => { let resolve!: (value: T) => void; let reject!: (error: Error) => void; const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; }); return { promise, resolve, reject }; };
function harness() {
  let value = snapshot();
  let view!: TrayPanelView;
  const actions: string[] = [];
  const action = deferred<TrayPanelSnapshot>();
  let read = () => Promise.resolve(value);
  let paints = 0;
  let paint = () => Promise.resolve();
  let readyCalls = 0;
  const controller = createTrayPanelController({
    read: () => read(), act: (name) => { actions.push(name); return action.promise; },
    ready: () => { readyCalls++; return Promise.resolve(value); }, now: () => 1000, paint: async () => { paints++; await paint(); }
  }, next => { view = next; });
  return { controller, action, actions, setValue: (next: TrayPanelSnapshot) => value = next,
    setRead: (next: typeof read) => read = next,
    setPaint: (next: typeof paint) => paint = next, readyCalls: () => readyCalls, view: () => view, paints: () => paints };
}
describe("tray panel authoritative presentation", () => {
  test("native preparation failure is presented after countdown ends", async () => {
    const h = harness();
    h.setValue({ ...snapshot(), pending: { requestId: 7, remainingMs: 3000 }, countdownRequestId: 7 });
    await h.controller.open();
    expect(panelError(h.view())).toBeNull();
    const failed = snapshot();
    failed.reminder.actionError = "overlay startup failed";
    h.setValue(failed);
    await h.controller.refresh();
    expect(panelCountdown(h.view(), 4000)).toBeNull();
    expect(panelError(h.view())).toBe("The last action failed. Open Unfocus to check.");
    expect(panelControls(h.view()).begin).toBe(true);
  });
  test("polling while first paint is deferred cannot suppress readiness", async () => {
    const h = harness();
    const paint = deferred<void>();
    h.setPaint(() => paint.promise);
    const opening = h.controller.open();
    await Promise.resolve();
    expect(h.paints()).toBe(1);
    await h.controller.refresh();
    paint.resolve();
    await opening;
    expect(h.readyCalls()).toBe(1);
    let reads = 0;
    h.setRead(async () => { reads++; return snapshot(); });
    await h.controller.refresh();
    expect(reads).toBe(1);
  });
  test("hiding during deferred paint still invalidates readiness", async () => {
    const h = harness();
    const paint = deferred<void>();
    h.setPaint(() => paint.promise);
    const opening = h.controller.open();
    await Promise.resolve();
    h.controller.hidden(1);
    paint.resolve();
    await opening;
    expect(h.readyCalls()).toBe(0);
  });
  test("only the initiating opening paints countdown; expiry has no action dispatch", async () => {
    const h = harness(); const state = snapshot(); state.pending = { requestId: 7, remainingMs: 3000 }; state.countdownRequestId = 7;
    h.setValue(state); await h.controller.open();
    expect([1000,2000,3000,4000].map(now => panelCountdown(h.view(), now))).toEqual([3,2,1,0]);
    expect(panelControls(h.view())).toEqual({ begin: false, pause: false, cancel: true });
    expect(h.actions).toEqual([]);
    h.controller.hidden(1); h.setValue({ ...state, openingGeneration: 2, countdownRequestId: null }); await h.controller.open(2);
    expect(panelCountdown(h.view(), 1000)).toBeNull();
    expect(panelControls(h.view())).toEqual({ begin: false, pause: false, cancel: false });
  });
  test("double click starts one request", async () => {
    const h = harness(); await h.controller.open(); const request = h.controller.action("begin");
    await h.controller.action("begin"); expect(h.actions).toEqual(["begin"]);
    h.action.resolve(snapshot()); await request; expect(h.view().busy).toBe(false);
  });
  test("delayed action after hide/reopen cannot restore an old countdown", async () => {
    const h = harness(); await h.controller.open(); const request = h.controller.action("begin");
    h.controller.hidden(1); h.setValue(snapshot(2)); await h.controller.open(2);
    h.action.resolve({ ...snapshot(), pending: { requestId: 7, remainingMs: 3000 }, countdownRequestId: 7 }); await request;
    expect(h.view().snapshot?.openingGeneration).toBe(2); expect(panelCountdown(h.view(), 1000)).toBeNull();
  });
  test("cancellation failure preserves pending state and usable cancel", async () => {
    const h = harness(); h.setValue({ ...snapshot(), pending: { requestId: 7, remainingMs: 3000 }, countdownRequestId: 7 }); await h.controller.open();
    const request = h.controller.action("cancel"); h.action.reject(new Error("failed")); await request;
    expect(h.view().error).toContain("may still start"); expect(panelControls(h.view()).cancel).toBe(true);
  });
  test("stale poll cannot re-enable controls during action", async () => {
    const h = harness(); await h.controller.open(); const read = deferred<TrayPanelSnapshot>(); h.setRead(() => read.promise);
    const poll = h.controller.refresh(); const request = h.controller.action("begin");
    h.action.resolve({ ...snapshot(), pending: { requestId: 7, remainingMs: 3000 }, countdownRequestId: 7 }); await request;
    read.resolve(snapshot()); await poll; expect(panelControls(h.view()).begin).toBe(false);
  });
  test("hidden and destroyed controllers neither poll nor publish late replies", async () => {
    const h = harness(); await h.controller.open(); h.controller.hidden(1);
    let reads = 0; h.setRead(async () => { reads++; return snapshot(); }); await h.controller.refresh(); expect(reads).toBe(0);
    const read = deferred<TrayPanelSnapshot>(); h.setRead(() => read.promise); const pending = h.controller.open(2);
    const before = h.view(); h.controller.destroy(); read.resolve(snapshot(2)); await pending;
    expect(h.view()).toBe(before); expect(h.paints()).toBe(1);
  });
});
