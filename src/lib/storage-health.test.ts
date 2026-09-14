import { describe, expect, test } from "bun:test";
import {
  developerStoragePresentation,
  storageUnavailable,
  storageUnavailableCopy,
  type LocalSnapshot
} from "./storage-health";

describe("reflection storage health", () => {
  test("distinguishes unavailable history from a valid empty snapshot", () => {
    const unavailable: LocalSnapshot<{ count: number }> = {
      loadHealth: { status: "unavailable", recovery: "retry" },
      data: null
    };
    const empty: LocalSnapshot<{ count: number }> = {
      loadHealth: { status: "available", recovery: "none" },
      data: { count: 0 }
    };

    expect(storageUnavailable(unavailable.loadHealth)).toBe(true);
    expect(unavailable.data).toBeNull();
    expect(storageUnavailable(empty.loadHealth)).toBe(false);
    expect(empty.data).toEqual({ count: 0 });
  });

  test("resource transport failure outranks retained available diagnostics", () => {
    const diagnostic = {
      status: "available",
      recovery: "none",
      category: null,
      error: null
    } as const;

    expect(
      developerStoragePresentation(diagnostic, true, null, {
        refresh: { status: "stale", error: "break summary IPC failed" },
        storageHealth: { status: "available", recovery: "none" },
        transportFailure: true
      })
    ).toEqual({
      status: "refresh stale",
      category: "transport",
      recovery: "unknown",
      error: "break summary IPC failed"
    });
  });

  test.each([
    ["activity", "retry", "read"],
    ["break", "retryOrStartNew", "invalid"]
  ] as const)(
    "%s fulfilled unavailable snapshots retain native storage provenance",
    (_resource, recovery, category) => {
      const diagnostic = {
        status: "unavailable",
        recovery,
        category,
        error: `${_resource} native storage diagnostic`
      } as const;
      const presentation = developerStoragePresentation(
        diagnostic,
        true,
        null,
        {
          refresh: { status: "unavailable", error: "generic unavailable" },
          storageHealth: { status: "unavailable", recovery },
          transportFailure: false
        },
        true
      );

      expect(presentation).toEqual({
        status: "unavailable",
        category,
        recovery,
        error: `${_resource} native storage diagnostic`
      });
      expect(presentation.category).not.toBe("transport");
    }
  );

  test("stale invalid diagnostics are not attributed to a current read envelope", () => {
    const presentation = developerStoragePresentation(
      {
        status: "unavailable",
        recovery: "retryOrStartNew",
        category: "invalid",
        error: "stale invalid native detail"
      },
      true,
      "latest diagnostics IPC failed",
      {
        refresh: { status: "unavailable", error: "generic unavailable" },
        storageHealth: { status: "unavailable", recovery: "retry" },
        transportFailure: false
      },
      false
    );

    expect(presentation).toEqual({
      status: "unavailable",
      category: "read",
      recovery: "retry",
      error: "Native resource snapshot reported storage unavailable; latest diagnostics refresh failed: latest diagnostics IPC failed"
    });
    expect(JSON.stringify(presentation)).not.toContain("stale invalid native detail");
  });

  test("a stale diagnostic with matching recovery still uses generic current-envelope detail", () => {
    const presentation = developerStoragePresentation(
      {
        status: "unavailable",
        recovery: "retry",
        category: "read",
        error: "stale native read detail"
      },
      true,
      null,
      {
        refresh: { status: "unavailable", error: "generic unavailable" },
        storageHealth: { status: "unavailable", recovery: "retry" },
        transportFailure: false
      },
      false
    );

    expect(presentation).toEqual({
      status: "unavailable",
      category: "read",
      recovery: "retry",
      error: "Native resource snapshot reported storage unavailable"
    });
  });

  test("a matching current diagnostic retains native detail", () => {
    const presentation = developerStoragePresentation(
      {
        status: "unavailable",
        recovery: "retry",
        category: "read",
        error: "current native read detail"
      },
      true,
      null,
      {
        refresh: { status: "unavailable", error: "generic unavailable" },
        storageHealth: { status: "unavailable", recovery: "retry" },
        transportFailure: false
      },
      true
    );

    expect(presentation.category).toBe("read");
    expect(presentation.recovery).toBe("retry");
    expect(presentation.error).toBe("current native read detail");
  });

  test("a current available snapshot outranks stale unavailable diagnostics", () => {
    const presentation = developerStoragePresentation(
      {
        status: "unavailable",
        recovery: "retryOrStartNew",
        category: "invalid",
        error: "stale invalid native detail"
      },
      true,
      null,
      {
        refresh: { status: "fresh", error: null },
        storageHealth: { status: "available", recovery: "none" },
        transportFailure: false
      },
      false
    );

    expect(presentation).toEqual({
      status: "available",
      category: "none",
      recovery: "none",
      error: "Native resource snapshot reported storage available"
    });
    expect(JSON.stringify(presentation)).not.toContain("stale invalid native detail");
  });

  test("a matching current available diagnostic retains its no-error detail", () => {
    const presentation = developerStoragePresentation(
      {
        status: "available",
        recovery: "none",
        category: null,
        error: null
      },
      true,
      null,
      {
        refresh: { status: "fresh", error: null },
        storageHealth: { status: "available", recovery: "none" },
        transportFailure: false
      },
      true
    );

    expect(presentation).toEqual({
      status: "available",
      category: "none",
      recovery: "none",
      error: "No load error"
    });
  });

  test.each(["activity", "break"] as const)(
    "%s is called transport unavailable only after rejection",
    (_resource) => {
      const presentation = developerStoragePresentation(null, false, null, {
        refresh: { status: "unavailable", error: `${_resource} IPC rejected` },
        storageHealth: null,
        transportFailure: true
      });
      expect(presentation.status).toBe("refresh unavailable");
      expect(presentation.category).toBe("transport");
      expect(presentation.error).toContain("IPC rejected");
    }
  );

  test("diagnostics copy distinguishes pending, unavailable, last-known, and fresh", () => {
    const diagnostic = {
      status: "available",
      recovery: "none",
      category: null,
      error: null
    } as const;

    expect(developerStoragePresentation(null, false, null).error).toBe(
      "Diagnostics pending"
    );
    expect(developerStoragePresentation(null, false, "IPC failed").status).toBe(
      "unavailable"
    );
    const lastKnown = developerStoragePresentation(diagnostic, true, "IPC failed");
    expect(lastKnown.status).toBe("last known");
    expect(lastKnown.error).toContain("latest refresh failed");
    expect(lastKnown.error).not.toBe("No load error");
    expect(developerStoragePresentation(diagnostic, true, null).error).toBe(
      "No load error"
    );
  });

  test("safe unavailable copy never includes technical detail or empty-history wording", () => {
    const technical = "/home/person/.config/unfocus/activity-history.json: permission denied";
    for (const resource of ["activity", "breaks"] as const) {
      const copy = storageUnavailableCopy(resource);
      const rendered = `${copy.heading} ${copy.message}`;
      expect(rendered).toContain("unavailable");
      expect(rendered).toContain("timing is unaffected");
      expect(rendered).not.toContain(technical);
      expect(rendered).not.toMatch(/no (activity|break outcomes)/i);
    }
  });
});
