import { describe, expect, test } from "bun:test";
import {
  applyReflectionFailure,
  applyReflectionRecoveryRejection,
  applyReflectionRecoverySnapshot,
  applyReflectionSnapshot,
  keepReflectionRecoveryPending,
  recoveryErrorAfterReflectionPoll,
  reflectionRecoveryFeedback
} from "./reflection-resource";

const unavailable = {
  loadHealth: { status: "unavailable", recovery: "retryOrStartNew" },
  data: null
} as const;

const availableHealth = { status: "available", recovery: "none" } as const;

describe("reflection recovery transitions", () => {
  test("does not publish command-returned available health before the snapshot", () => {
    const previous = applyReflectionSnapshot<{ count: number }>(
      unavailable,
      "history unavailable",
      1_000
    );
    const pending = keepReflectionRecoveryPending(previous);

    expect(pending).toBe(previous);
    expect(pending.storageHealth).toEqual(unavailable.loadHealth);
    expect(pending.refresh.status).toBe("unavailable");
  });

  test("an available canonical snapshot atomically publishes fresh data and health", () => {
    const previous = applyReflectionSnapshot<{ count: number }>(
      unavailable,
      "history unavailable"
    );
    const data = { count: 4 };
    const transition = applyReflectionRecoverySnapshot(
      previous,
      { loadHealth: availableHealth, data },
      "history unavailable",
      2_000
    );

    expect(transition.error).toBeNull();
    expect(transition.resource).toEqual({
      refresh: { status: "fresh", data, error: null, asOfMs: 2_000 },
      storageHealth: availableHealth,
      transportFailure: false
    });
  });

  test("fulfilled unavailable follow-up remains unavailable and fails follow-up", () => {
    const previous = applyReflectionSnapshot<{ count: number }>(
      unavailable,
      "history unavailable"
    );
    const followUp = {
      loadHealth: { status: "unavailable", recovery: "retry" },
      data: null
    } as const;
    const transition = applyReflectionRecoverySnapshot(
      previous,
      followUp,
      "history unavailable"
    );

    expect(transition.error).toBe("followUp");
    expect(transition.resource.refresh.status).toBe("unavailable");
    expect(transition.resource.storageHealth).toEqual(followUp.loadHealth);
    expect(transition.resource.transportFailure).toBe(false);
    const feedback = reflectionRecoveryFeedback("activity", transition.error);
    expect(feedback).toContain("could not be confirmed");
    expect(feedback).toContain("remains unavailable");
    expect(feedback).not.toContain("Recovery completed");
  });

  test("unavailable or rejected follow-up retains only real prior data as stale", () => {
    const data = { count: 7 };
    const previous = applyReflectionSnapshot(
      { loadHealth: availableHealth, data },
      "history unavailable",
      3_000
    );
    const unavailableTransition = applyReflectionRecoverySnapshot(
      previous,
      unavailable,
      "history unavailable"
    );
    const rejectedTransition = applyReflectionRecoveryRejection(
      previous,
      "snapshot IPC failed"
    );

    expect(unavailableTransition.error).toBe("followUp");
    expect(unavailableTransition.resource.refresh.status).toBe("stale");
    expect(unavailableTransition.resource.refresh.data).toBe(data);
    expect(unavailableTransition.resource.refresh.asOfMs).toBe(3_000);
    expect(rejectedTransition.error).toBe("followUp");
    expect(rejectedTransition.resource.refresh.status).toBe("stale");
    expect(rejectedTransition.resource.refresh.data).toBe(data);
    expect(unavailableTransition.resource.transportFailure).toBe(false);
    expect(rejectedTransition.resource.transportFailure).toBe(true);
  });

  test.each(["activity", "break"] as const)(
    "%s snapshots distinguish typed unavailability from transport rejection",
    () => {
      const typedUnavailable = applyReflectionSnapshot<{ count: number }>(
        unavailable,
        "storage unavailable"
      );
      const rejected = applyReflectionRecoveryRejection(
        typedUnavailable,
        "IPC rejected"
      ).resource;

      expect(typedUnavailable.refresh.status).toBe("unavailable");
      expect(typedUnavailable.transportFailure).toBe(false);
      expect(rejected.refresh.status).toBe("unavailable");
      expect(rejected.transportFailure).toBe(true);
    }
  );

  test.each(["activity", "break"] as const)(
    "%s ordinary polls clear recovery feedback only after fresh availability",
    () => {
      const typedUnavailable = applyReflectionSnapshot<{ count: number }>(
        unavailable,
        "storage unavailable"
      );
      const stale = applyReflectionFailure(
        applyReflectionSnapshot(
          { loadHealth: availableHealth, data: { count: 1 } },
          "storage unavailable"
        ),
        "IPC rejected"
      );
      const fresh = applyReflectionSnapshot(
        { loadHealth: availableHealth, data: { count: 2 } },
        "storage unavailable"
      );

      expect(recoveryErrorAfterReflectionPoll("followUp", typedUnavailable)).toBe(
        "followUp"
      );
      expect(recoveryErrorAfterReflectionPoll("operation", stale)).toBe("operation");
      expect(recoveryErrorAfterReflectionPoll("followUp", fresh)).toBeNull();
    }
  );
});
