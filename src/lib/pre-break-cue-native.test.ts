import { expect, test } from "bun:test";
import { createCueVisibilityController } from "./pre-break-cue-native";

const layout = { isNotched: true, notchWidth: 209, wingWidth: 64, shoulderWidth: 9, height: 38 };
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

test("prepares and paints before revealing, and prepares again for the next appearance", async () => {
  const calls: string[] = [];
  const controller = createCueVisibilityController({
    prepare: async () => { calls.push("prepare"); return layout; },
    applyLayout: async (value) => { expect(value).toEqual(layout); calls.push("paint"); },
    setVisible: async (visible) => { calls.push(String(visible)); },
    onError: () => { throw new Error("unexpected error"); }
  });
  await controller.setVisible(true);
  await controller.setVisible(false);
  await controller.setVisible(true);
  expect(calls).toEqual(["prepare", "paint", "true", "false", "prepare", "paint", "true"]);
});

for (const cancelledDuring of ["preparation", "paint"] as const) {
  test(`cancellation during ${cancelledDuring} cannot reveal the cue`, async () => {
    const gate = deferred<void>();
    const started = deferred<void>();
    const reveals: boolean[] = [];
    const controller = createCueVisibilityController({
      prepare: async () => {
        if (cancelledDuring === "preparation") { started.resolve(); await gate.promise; }
        return layout;
      },
      applyLayout: async () => {
        if (cancelledDuring === "paint") { started.resolve(); await gate.promise; }
      },
      setVisible: async (visible) => { reveals.push(visible); },
      onError: () => {}
    });
    const pending = controller.setVisible(true);
    await started.promise;
    const hidden = controller.setVisible(false);
    gate.resolve();
    await Promise.all([pending, hidden]);
    expect(reveals).toEqual([false]);
  });
}

test("a failed preparation stays hidden and reports the failure", async () => {
  const calls: unknown[] = [];
  const controller = createCueVisibilityController({
    prepare: async () => { throw new Error("display unavailable"); },
    applyLayout: async () => { calls.push("paint"); },
    setVisible: async (visible) => { calls.push(visible); },
    onError: (error) => { calls.push((error as Error).message); }
  });
  await controller.setVisible(true);
  expect(calls).toEqual(["display unavailable", false]);
});

test("disposal hides and prevents subsequent reveals", async () => {
  const reveals: boolean[] = [];
  const controller = createCueVisibilityController({
    applyLayout: async () => {},
    setVisible: async (visible) => { reveals.push(visible); },
    onError: () => {}
  });
  await controller.setVisible(true);
  await controller.dispose();
  await controller.setVisible(true);
  expect(reveals).toEqual([true, false]);
});
