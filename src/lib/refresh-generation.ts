export type RefreshGenerationGuard = {
  capture: () => number;
  invalidate: () => number;
  isCurrent: (generation: number) => boolean;
};

/**
 * Guards independently fetched snapshots against publishing across a native
 * mutation. Invalidate both when the mutation starts and immediately before
 * its result is published, mirroring the reminder refresh generation rule.
 */
export function createRefreshGenerationGuard(): RefreshGenerationGuard {
  let generation = 0;
  return {
    capture: () => generation,
    invalidate: () => (generation += 1),
    isCurrent: (candidate) => candidate === generation
  };
}

export type RequestSequence = {
  begin: () => number;
  isLatest: (requestId: number) => boolean;
};

/** Every resource owns one monotonic sequence across polls and follow-ups. */
export function createRequestSequence(): RequestSequence {
  let latestRequestId = 0;
  return {
    begin: () => (latestRequestId += 1),
    isLatest: (requestId) => requestId === latestRequestId
  };
}

export type LatestRequestResult<T> = {
  latest: boolean;
  settled: PromiseSettledResult<T>;
};

/**
 * Allocate ordering when a request starts, then report whether it still owns
 * publication after settling. Callers may layer mutation generations on top.
 */
export async function settleLatestRequest<T>(
  sequence: RequestSequence,
  request: () => Promise<T>
): Promise<LatestRequestResult<T>> {
  const requestId = sequence.begin();
  try {
    const value = await request();
    return {
      latest: sequence.isLatest(requestId),
      settled: { status: "fulfilled", value }
    };
  } catch (reason) {
    return {
      latest: sequence.isLatest(requestId),
      settled: { status: "rejected", reason }
    };
  }
}
