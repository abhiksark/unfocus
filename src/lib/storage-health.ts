import type { RefreshState } from "./refresh-state";

export type StorageStatus = "available" | "unavailable";
export type StorageRecovery = "none" | "retry" | "retryOrStartNew";
export type StorageFailureCategory = "read" | "invalid";

export type StorageLoadHealth = {
  status: StorageStatus;
  recovery: StorageRecovery;
};

export type LocalSnapshot<T> = {
  loadHealth: StorageLoadHealth;
  data: T | null;
};

export type StorageDiagnostic = StorageLoadHealth & {
  category: StorageFailureCategory | null;
  error: string | null;
};

export function storageUnavailable(health: StorageLoadHealth | null): boolean {
  return health?.status === "unavailable";
}

export function storageUnavailableCopy(resource: "activity" | "breaks"): {
  heading: string;
  message: string;
} {
  return {
    heading:
      resource === "activity"
        ? "Local activity history unavailable"
        : "Local break history unavailable",
    message: "Reminder timing is unaffected. You can retry without restarting your timer."
  };
}

export type DeveloperStoragePresentation = {
  status: string;
  category: string;
  recovery: string;
  error: string;
};

type ResourceTransport = {
  refresh: Pick<RefreshState<unknown>, "status" | "error">;
  storageHealth: StorageLoadHealth | null;
  transportFailure: boolean;
};

function categoryFromRecovery(recovery: StorageRecovery): string {
  if (recovery === "retryOrStartNew") return "invalid";
  if (recovery === "retry") return "read";
  return "none";
}

/**
 * A current resource IPC result outranks stale storage diagnostics. A fulfilled
 * unavailable envelope remains native storage evidence, even though its
 * refresh state also has no payload; only a rejection is a transport failure.
 */
export function developerStoragePresentation(
  diagnostic: StorageDiagnostic | null,
  hasDiagnosticsReport: boolean,
  diagnosticsError: string | null,
  resourceTransport: ResourceTransport | null = null,
  diagnosticsCurrentSuccessful = false
): DeveloperStoragePresentation {
  if (
    resourceTransport?.transportFailure &&
    (resourceTransport.refresh.status === "stale" ||
      resourceTransport.refresh.status === "unavailable")
  ) {
    return {
      status:
        resourceTransport.refresh.status === "stale"
          ? "refresh stale"
          : "refresh unavailable",
      category: "transport",
      recovery: "unknown",
      error: resourceTransport.refresh.error ?? "Resource refresh failed"
    };
  }
  if (
    resourceTransport &&
    !resourceTransport.transportFailure &&
    resourceTransport.storageHealth?.status === "unavailable"
  ) {
    const envelopeRecovery = resourceTransport.storageHealth.recovery;
    const matchingDiagnostic =
      diagnosticsCurrentSuccessful &&
      diagnosticsError === null &&
      diagnostic?.status === "unavailable" &&
      diagnostic.recovery === envelopeRecovery
        ? diagnostic
        : null;
    const genericDetail = "Native resource snapshot reported storage unavailable";
    return {
      status: "unavailable",
      category:
        matchingDiagnostic?.category ?? categoryFromRecovery(envelopeRecovery),
      recovery: envelopeRecovery,
      error:
        matchingDiagnostic?.error ??
        (diagnosticsError
          ? `${genericDetail}; latest diagnostics refresh failed: ${diagnosticsError}`
          : genericDetail)
    };
  }
  if (resourceTransport?.refresh.status === "loading") {
    return {
      status: "pending",
      category: "unknown",
      recovery: "unknown",
      error: "Resource snapshot pending"
    };
  }
  if (
    resourceTransport &&
    !resourceTransport.transportFailure &&
    resourceTransport.storageHealth?.status === "available"
  ) {
    const matchingDiagnostic =
      diagnosticsCurrentSuccessful &&
      diagnosticsError === null &&
      diagnostic?.status === "available" &&
      diagnostic.recovery === "none"
        ? diagnostic
        : null;
    const genericDetail = "Native resource snapshot reported storage available";
    return {
      status: "available",
      category: "none",
      recovery: "none",
      error: matchingDiagnostic
        ? (matchingDiagnostic.error ?? "No load error")
        : diagnosticsError
          ? `${genericDetail}; latest diagnostics refresh failed: ${diagnosticsError}`
          : genericDetail
    };
  }
  if (diagnosticsError) {
    return {
      status: hasDiagnosticsReport ? "last known" : "unavailable",
      category: hasDiagnosticsReport ? (diagnostic?.category ?? "none") : "unknown",
      recovery: hasDiagnosticsReport ? (diagnostic?.recovery ?? "none") : "unknown",
      error: hasDiagnosticsReport
        ? `Last-known diagnostics; latest refresh failed: ${diagnosticsError}`
        : `Diagnostics unavailable: ${diagnosticsError}`
    };
  }
  if (!hasDiagnosticsReport || !diagnostic) {
    return {
      status: "pending",
      category: "unknown",
      recovery: "unknown",
      error: "Diagnostics pending"
    };
  }
  return {
    status: diagnostic.status,
    category: diagnostic.category ?? "none",
    recovery: diagnostic.recovery,
    error: diagnostic.error ?? "No load error"
  };
}
