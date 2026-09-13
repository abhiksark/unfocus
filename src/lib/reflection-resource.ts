import {
  refreshFailed,
  refreshLoading,
  refreshSucceeded,
  refreshUnavailable,
  type RefreshState
} from "./refresh-state";
import type { LocalSnapshot, StorageLoadHealth } from "./storage-health";

export type ReflectionResource<T> = {
  refresh: RefreshState<T>;
  storageHealth: StorageLoadHealth | null;
  /** True only when the latest resource IPC promise rejected. */
  transportFailure: boolean;
};

export type ReflectionRecoveryError = "operation" | "followUp" | null;

export function reflectionRecoveryFeedback(
  resource: "activity" | "break",
  error: ReflectionRecoveryError
): string | null {
  if (error === null) return null;
  const history = resource === "activity" ? "Activity history" : "Break history";
  if (error === "operation") {
    return `Recovery did not make this history available. You can try again.`;
  }
  return `Recovery could not be confirmed. ${history} remains unavailable and will retry automatically.`;
}

export type ReflectionRecoveryTransition<T> = {
  resource: ReflectionResource<T>;
  error: ReflectionRecoveryError;
};

export function initialReflectionResource<T>(): ReflectionResource<T> {
  return { refresh: refreshLoading<T>(), storageHealth: null, transportFailure: false };
}

/** Canonical polling snapshots: typed storage failure outranks retained data. */
export function applyReflectionSnapshot<T>(
  snapshot: LocalSnapshot<T>,
  unavailableError: string,
  asOfMs = Date.now()
): ReflectionResource<T> {
  if (snapshot.loadHealth.status === "available" && snapshot.data !== null) {
    return {
      refresh: refreshSucceeded(snapshot.data, asOfMs),
      storageHealth: snapshot.loadHealth,
      transportFailure: false
    };
  }
  return {
    refresh: refreshUnavailable(unavailableError),
    storageHealth: snapshot.loadHealth,
    transportFailure: false
  };
}

export function applyReflectionFailure<T>(
  previous: ReflectionResource<T>,
  error: string
): ReflectionResource<T> {
  return {
    refresh: refreshFailed(previous.refresh, error),
    storageHealth: previous.storageHealth,
    transportFailure: true
  };
}

/** A command result is provisional; pending recovery keeps the exact old surface. */
export function keepReflectionRecoveryPending<T>(
  previous: ReflectionResource<T>
): ReflectionResource<T> {
  return previous;
}

/**
 * Only the canonical follow-up snapshot can complete recovery. Available data
 * is published atomically; unavailable or inconsistent envelopes preserve real
 * prior data only as stale and keep the snapshot's truthful capabilities.
 */
export function applyReflectionRecoverySnapshot<T>(
  previous: ReflectionResource<T>,
  snapshot: LocalSnapshot<T>,
  unavailableError: string,
  asOfMs = Date.now()
): ReflectionRecoveryTransition<T> {
  if (snapshot.loadHealth.status === "available" && snapshot.data !== null) {
    return {
      resource: {
        refresh: refreshSucceeded(snapshot.data, asOfMs),
        storageHealth: snapshot.loadHealth,
        transportFailure: false
      },
      error: null
    };
  }
  return {
    resource: {
      refresh: refreshFailed(previous.refresh, unavailableError),
      storageHealth: snapshot.loadHealth,
      transportFailure: false
    },
    error: "followUp"
  };
}

export function applyReflectionRecoveryRejection<T>(
  previous: ReflectionResource<T>,
  error: string
): ReflectionRecoveryTransition<T> {
  return {
    resource: applyReflectionFailure(previous, error),
    error: "followUp"
  };
}

/** Ordinary fresh polls clear old recovery feedback; failures retain it. */
export function recoveryErrorAfterReflectionPoll<T>(
  previous: ReflectionRecoveryError,
  resource: ReflectionResource<T>
): ReflectionRecoveryError {
  return resource.refresh.status === "fresh" && !resource.transportFailure
    ? null
    : previous;
}
