import { describe, expect, test } from "bun:test";
import {
  createRefreshGenerationGuard,
  createRequestSequence,
  settleLatestRequest
} from "./refresh-generation";

function deferred<T>(): {
  promise: Promise<T>;
  resolve: (value: T) => void;
} {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((complete) => {
    resolve = complete;
  });
  return { promise, resolve };
}

describe("refresh generations", () => {
  test("rejects polls started before or during recovery publication", () => {
    const guard = createRefreshGenerationGuard();
    const beforeRecovery = guard.capture();

    guard.invalidate();
    const duringRecovery = guard.capture();
    guard.invalidate();
    const afterRecovery = guard.capture();

    expect(guard.isCurrent(beforeRecovery)).toBe(false);
    expect(guard.isCurrent(duringRecovery)).toBe(false);
    expect(guard.isCurrent(afterRecovery)).toBe(true);
  });

  test("resource invalidation is independent", () => {
    const activity = createRefreshGenerationGuard();
    const ledger = createRefreshGenerationGuard();
    const activityPoll = activity.capture();
    const ledgerPoll = ledger.capture();

    activity.invalidate();

    expect(activity.isCurrent(activityPoll)).toBe(false);
    expect(ledger.isCurrent(ledgerPoll)).toBe(true);
  });
});

describe("per-resource request sequences", () => {
  test("an older deferred response cannot publish over a newer response", async () => {
    const sequence = createRequestSequence();
    const older = deferred<string>();
    const newer = deferred<string>();
    const publications: string[] = [];

    const first = settleLatestRequest(sequence, () => older.promise).then((result) => {
      if (result.latest && result.settled.status === "fulfilled") {
        publications.push(result.settled.value);
      }
    });
    const second = settleLatestRequest(sequence, () => newer.promise).then((result) => {
      if (result.latest && result.settled.status === "fulfilled") {
        publications.push(result.settled.value);
      }
    });

    newer.resolve("newer");
    await second;
    older.resolve("older");
    await first;

    expect(publications).toEqual(["newer"]);
  });

  test("request ordering is independent per resource", async () => {
    const reminder = createRequestSequence();
    const diagnostics = createRequestSequence();
    const oldReminder = deferred<string>();
    const newReminder = deferred<string>();

    const reminderOld = settleLatestRequest(reminder, () => oldReminder.promise);
    const diagnosticsOnly = settleLatestRequest(diagnostics, () => Promise.resolve("report"));
    const reminderNew = settleLatestRequest(reminder, () => newReminder.promise);

    newReminder.resolve("new status");
    oldReminder.resolve("old status");
    const [oldResult, reportResult, newResult] = await Promise.all([
      reminderOld,
      diagnosticsOnly,
      reminderNew
    ]);

    expect(oldResult.latest).toBe(false);
    expect(newResult.latest).toBe(true);
    expect(reportResult.latest).toBe(true);
  });
});
