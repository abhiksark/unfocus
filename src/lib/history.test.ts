import { describe, expect, test } from "bun:test";
import {
  historyActivationUsesKeyboard,
  historyActivityLevel,
  historyCalendarNeedsRefresh,
  historyEscapeAction,
  historyHourDetailLabel,
  initialHistoryDateKey,
  moveHistoryGridFocus,
  moveHistoryHourFocus,
  type HistoryCalendarDay
} from "./history";

function runInTimezone<Result>(timezone: string, script: string): Result {
  const result = Bun.spawnSync({
    cmd: [
      "bun",
      "-e",
      `const mod = await import(${JSON.stringify(new URL("./history.ts", import.meta.url).href)});\n${script}`
    ],
    cwd: import.meta.dir,
    env: { ...process.env, TZ: timezone },
    stdout: "pipe",
    stderr: "pipe"
  });
  if (result.exitCode !== 0) {
    throw new Error(new TextDecoder().decode(result.stderr));
  }
  return JSON.parse(new TextDecoder().decode(result.stdout)) as Result;
}

describe("history paging and calendar boundaries", () => {
  test("builds the current 30-day page plus two prior full pages", () => {
    const pages = runInTimezone<
      Array<{
        startIso: string;
        endMs: number;
        dayCount: number;
        dayBoundaryCount: number;
        dayBoundaryStart: number;
        dayBoundaryEnd: number;
        hourBoundaryStart: number;
        hourBoundaryEnd: number;
      }>
    >(
      "UTC",
      `
const nowMs = Date.parse("2026-08-15T10:30:00Z");
const pages = mod.buildHistoryPages(nowMs, 4);
console.log(JSON.stringify(pages.map((page) => ({
  startIso: new Date(page.startMs).toISOString(),
  endMs: page.endMs,
  dayCount: page.days.length,
  dayBoundaryCount: page.dayBoundariesMs.length,
  dayBoundaryStart: page.dayBoundariesMs[0],
  dayBoundaryEnd: page.dayBoundariesMs[page.dayBoundariesMs.length - 1],
  hourBoundaryStart: page.hourBoundariesMs[0],
  hourBoundaryEnd: page.hourBoundariesMs[page.hourBoundariesMs.length - 1]
}))));
`
    );

    const nowMs = Date.parse("2026-08-15T10:30:00Z");
    expect(pages).toHaveLength(3);
    expect(pages[0].startIso).toBe("2026-07-17T04:00:00.000Z");
    expect(pages[0].endMs).toBe(nowMs);
    expect(pages[1].startIso).toBe("2026-06-17T04:00:00.000Z");
    expect(pages[1].endMs).toBe(Date.parse("2026-07-17T04:00:00Z"));
    expect(pages[2].startIso).toBe("2026-05-18T04:00:00.000Z");
    expect(pages[2].endMs).toBe(Date.parse("2026-06-17T04:00:00Z"));

    for (const page of pages) {
      expect(page.dayCount).toBe(30);
      expect(page.dayBoundaryCount).toBe(31);
      expect(page.dayBoundaryStart).toBe(Date.parse(page.startIso));
      expect(page.dayBoundaryEnd).toBe(page.endMs);
      expect(page.hourBoundaryStart).toBe(Date.parse(page.startIso));
      expect(page.hourBoundaryEnd).toBe(page.endMs);
    }
  });

  test("respects non-midnight day starts across a year rollover", () => {
    const bounds = runInTimezone<{ startIso: string; endIso: string }>(
      "UTC",
      `
const bounds = mod.historyDayBoundsAt(Date.parse("2026-01-02T03:30:00Z"), 4);
console.log(JSON.stringify({
  startIso: new Date(bounds.startMs).toISOString(),
  endIso: new Date(bounds.endMs).toISOString()
}));
`
    );

    expect(bounds.startIso).toBe("2026-01-01T04:00:00.000Z");
    expect(bounds.endIso).toBe("2026-01-02T04:00:00.000Z");
  });

  test("keeps 30 non-empty days when now is exactly the local day-start boundary", () => {
    const page = runInTimezone<{
      maxDays: number;
      startIso: string;
      endIso: string;
      dayCount: number;
      boundaryCount: number;
      boundariesStrictlyIncreasing: boolean;
      lastDayStartIso: string;
      lastDayEndIso: string;
      lastDayHourBucketCount: number;
    }>(
      "UTC",
      `
const nowMs = Date.parse("2026-08-15T04:00:00Z");
const page = mod.buildHistoryPageRequest(0, nowMs, 4);
const lastDay = page.days[page.days.length - 1];
console.log(JSON.stringify({
  maxDays: mod.HISTORY_MAX_DAYS,
  startIso: new Date(page.startMs).toISOString(),
  endIso: new Date(page.endMs).toISOString(),
  dayCount: page.days.length,
  boundaryCount: page.dayBoundariesMs.length,
  boundariesStrictlyIncreasing: page.dayBoundariesMs.every(
    (boundary, index, all) => index === 0 || boundary > all[index - 1]
  ),
  lastDayStartIso: new Date(lastDay.startMs).toISOString(),
  lastDayEndIso: new Date(lastDay.endMs).toISOString(),
  lastDayHourBucketCount: lastDay.hourSlots.reduce(
    (total, slot) => total + slot.bucketIndexes.length,
    0
  )
}));
`
    );

    expect(page.maxDays).toBe(90);
    expect(page.startIso).toBe("2026-07-16T04:00:00.000Z");
    expect(page.endIso).toBe("2026-08-15T04:00:00.000Z");
    expect(page.dayCount).toBe(30);
    expect(page.boundaryCount).toBe(31);
    expect(page.boundariesStrictlyIncreasing).toBe(true);
    expect(page.lastDayStartIso).toBe("2026-08-14T04:00:00.000Z");
    expect(page.lastDayEndIso).toBe("2026-08-15T04:00:00.000Z");
    expect(page.lastDayHourBucketCount).toBe(24);
  });

  test("uses local day lengths across daylight-saving transitions", () => {
    const hours = runInTimezone<{ springHours: number; fallHours: number }>(
      "America/New_York",
      `
const spring = mod.historyDayBoundsAt(new Date(2026, 2, 8, 12, 0, 0, 0).getTime(), 0);
const fall = mod.historyDayBoundsAt(new Date(2026, 10, 1, 12, 0, 0, 0).getTime(), 0);
console.log(JSON.stringify({
  springHours: (spring.endMs - spring.startMs) / 3600000,
  fallHours: (fall.endMs - fall.startMs) / 3600000
}));
`
    );

    expect(hours.springHours).toBe(23);
    expect(hours.fallHours).toBe(25);
  });

  test("keeps 24 wall-clock slots while DST changes the real hour count", () => {
    const counts = runInTimezone<{
      springSlots: number;
      springGap: number;
      springBuckets: number;
      fallSlots: number;
      fallRepeat: number;
      fallBuckets: number;
    }>(
      "America/New_York",
      `
const springPage = mod.buildHistoryPageRequest(0, new Date(2026, 2, 9, 12, 0, 0, 0).getTime(), 0);
const springDay = springPage.days.find((entry) => entry.dateKey === "2026-03-08");
const fallPage = mod.buildHistoryPageRequest(0, new Date(2026, 10, 2, 12, 0, 0, 0).getTime(), 0);
const fallDay = fallPage.days.find((entry) => entry.dateKey === "2026-11-01");
console.log(JSON.stringify({
  springSlots: springDay.hourSlots.length,
  springGap: springDay.hourSlots.find((slot) => slot.wallHour === 2).bucketIndexes.length,
  springBuckets: springDay.hourSlots.reduce((total, slot) => total + slot.bucketIndexes.length, 0),
  fallSlots: fallDay.hourSlots.length,
  fallRepeat: fallDay.hourSlots.find((slot) => slot.wallHour === 1).bucketIndexes.length,
  fallBuckets: fallDay.hourSlots.reduce((total, slot) => total + slot.bucketIndexes.length, 0)
}));
`
    );

    expect(counts.springSlots).toBe(24);
    expect(counts.springGap).toBe(0);
    expect(counts.springBuckets).toBe(23);
    expect(counts.fallSlots).toBe(24);
    expect(counts.fallRepeat).toBe(2);
    expect(counts.fallBuckets).toBe(25);
  });
});

describe("history page materialization", () => {
  test("labels exactly blank activity totals without implying classified time", () => {
    const page = runInTimezone<{
      totals: { activeLabel: string; afkLabel: string; longestLabel: string };
      day0: { activeLabel: string; afkLabel: string; longestLabel: string };
      blankKind: string;
    }>(
      "UTC",
      `
const request = mod.buildHistoryPageRequest(0, Date.parse("2026-08-15T10:30:00Z"), 0);
const dailyBuckets = Array.from({ length: request.days.length }, () => ({
  activeMs: 0,
  afkMs: 0,
  longestActiveMs: 0
}));
const hourlyBuckets = Array.from({ length: request.hourBoundariesMs.length - 1 }, () => ({
  activeMs: 0,
  afkMs: 0,
  longestActiveMs: 0
}));
const page = mod.materializeHistoryPage(request, dailyBuckets, hourlyBuckets, []);
console.log(JSON.stringify({
  totals: {
    activeLabel: page.totals.activeLabel,
    afkLabel: page.totals.afkLabel,
    longestLabel: page.totals.longestLabel
  },
  day0: {
    activeLabel: page.days[0].totals.activeLabel,
    afkLabel: page.days[0].totals.afkLabel,
    longestLabel: page.days[0].totals.longestLabel
  },
  blankKind: page.days[0].hourSlots[0].kind
}));
`
    );

    expect(page.totals).toEqual({
      activeLabel: "—",
      afkLabel: "—",
      longestLabel: "—"
    });
    expect(page.day0).toEqual({
      activeLabel: "—",
      afkLabel: "—",
      longestLabel: "—"
    });
    expect(page.blankKind).toBe("blank");
  });

  test("uses daily buckets for totals and keeps blank activity distinct from away", () => {
    const page = runInTimezone<{
      totals: {
        activeMs: number;
        afkMs: number;
        longestActiveMs: number;
        activeLabel: string;
        afkLabel: string;
        longestLabel: string;
      };
      day28: { activeMs: number; afkMs: number; longestLabel: string };
      kinds: { active: string; away: string; blank: string; currentAway: string };
    }>(
      "UTC",
      `
const request = mod.buildHistoryPageRequest(0, Date.parse("2026-08-15T10:30:00Z"), 0);
const dailyBuckets = Array.from({ length: request.days.length }, () => ({
  activeMs: 0,
  afkMs: 0,
  longestActiveMs: 0
}));
const hourlyBuckets = Array.from({ length: request.hourBoundariesMs.length - 1 }, () => ({
  activeMs: 0,
  afkMs: 0,
  longestActiveMs: 0
}));
dailyBuckets[28] = { activeMs: 7200000, afkMs: 1800000, longestActiveMs: 5400000 };
dailyBuckets[29] = { activeMs: 1800000, afkMs: 900000, longestActiveMs: 1200000 };
const fullDay = request.days[28];
const currentDay = request.days[29];
const fullDayActiveHour = fullDay.hourSlots.find((slot) => slot.wallHour === 9);
const fullDayAwayHour = fullDay.hourSlots.find((slot) => slot.wallHour === 10);
const currentDayAwayHour = currentDay.hourSlots.find((slot) => slot.wallHour === 8);
hourlyBuckets[fullDayActiveHour.bucketIndexes[0]] = {
  activeMs: 1800000,
  afkMs: 0,
  longestActiveMs: 1800000
};
hourlyBuckets[fullDayAwayHour.bucketIndexes[0]] = {
  activeMs: 0,
  afkMs: 900000,
  longestActiveMs: 0
};
hourlyBuckets[currentDayAwayHour.bucketIndexes[0]] = {
  activeMs: 0,
  afkMs: 600000,
  longestActiveMs: 0
};
const page = mod.materializeHistoryPage(request, dailyBuckets, hourlyBuckets, []);
console.log(JSON.stringify({
  totals: {
    activeMs: page.totals.activeMs,
    afkMs: page.totals.afkMs,
    longestActiveMs: page.totals.longestActiveMs,
    activeLabel: page.totals.activeLabel,
    afkLabel: page.totals.afkLabel,
    longestLabel: page.totals.longestLabel
  },
  day28: {
    activeMs: page.days[28].totals.activeMs,
    afkMs: page.days[28].totals.afkMs,
    longestLabel: page.days[28].totals.longestLabel
  },
  kinds: {
    active: page.days[28].hourSlots.find((slot) => slot.wallHour === 9).kind,
    away: page.days[28].hourSlots.find((slot) => slot.wallHour === 10).kind,
    blank: page.days[28].hourSlots.find((slot) => slot.wallHour === 11).kind,
    currentAway: page.days[29].hourSlots.find((slot) => slot.wallHour === 8).kind
  }
}));
`
    );

    expect(page.totals.activeMs).toBe(9_000_000);
    expect(page.totals.afkMs).toBe(2_700_000);
    expect(page.totals.longestActiveMs).toBe(5_400_000);
    expect(page.totals.activeLabel).toBe("2h 30m");
    expect(page.totals.afkLabel).toBe("45m");
    expect(page.totals.longestLabel).toBe("1h 30m");
    expect(page.day28.activeMs).toBe(7_200_000);
    expect(page.day28.afkMs).toBe(1_800_000);
    expect(page.day28.longestLabel).toBe("1h 30m");
    expect(page.kinds.active).toBe("active");
    expect(page.kinds.away).toBe("afk");
    expect(page.kinds.blank).toBe("blank");
    expect(page.kinds.currentAway).toBe("afk");
  });

  test("counts break outcomes and assigns markers with half-open page and day ranges", () => {
    const page = runInTimezone<{
      breakCounts: Array<{ kind: string; label: string; count: number }>;
      day0: Record<string, number>;
      day29: Record<string, number>;
      day0Midnight: string[];
      day29Midnight: string[];
      day29Nine: string[];
      day29Ten: string[];
      day29TwentyThree: string[];
      firstDayEndMs: number;
      firstBoundaryAfterStart: number;
    }>(
      "UTC",
      `
const request = mod.buildHistoryPageRequest(0, Date.parse("2026-08-15T10:30:00Z"), 0);
const dailyBuckets = Array.from({ length: request.days.length }, () => ({
  activeMs: 0,
  afkMs: 0,
  longestActiveMs: 0
}));
const hourlyBuckets = Array.from({ length: request.hourBoundariesMs.length - 1 }, () => ({
  activeMs: 0,
  afkMs: 0,
  longestActiveMs: 0
}));
const firstDay = request.days[0];
const lastDay = request.days[29];
const lastDayNine = lastDay.hourSlots.find((slot) => slot.wallHour === 9);
const events = [
  { atMs: request.startMs, kind: "scheduledShown" },
  { atMs: request.startMs + 10 * 60 * 1000, kind: "fullscreenSuppress" },
  { atMs: lastDay.startMs, kind: "naturalIdle" },
  { atMs: request.hourBoundariesMs[lastDayNine.bucketIndexes[0]] + 5 * 60 * 1000, kind: "manualTakeBreak" },
  { atMs: request.hourBoundariesMs[lastDayNine.bucketIndexes[0]] + 55 * 60 * 1000, kind: "scheduledShown" },
  { atMs: request.endMs, kind: "fullscreenSuppress" }
];
const page = mod.materializeHistoryPage(request, dailyBuckets, hourlyBuckets, events);
console.log(JSON.stringify({
  breakCounts: page.breakCounts,
  day0: Object.fromEntries(page.days[0].breakCounts.map((count) => [count.kind, count.count])),
  day29: Object.fromEntries(page.days[29].breakCounts.map((count) => [count.kind, count.count])),
  day0Midnight: page.days[0].hourSlots.find((slot) => slot.wallHour === 0).breakMarkers.map((event) => event.kind),
  day29Midnight: page.days[29].hourSlots.find((slot) => slot.wallHour === 0).breakMarkers.map((event) => event.kind),
  day29Nine: page.days[29].hourSlots.find((slot) => slot.wallHour === 9).breakMarkers.map((event) => event.kind),
  day29Ten: page.days[29].hourSlots.find((slot) => slot.wallHour === 10).breakMarkers.map((event) => event.kind),
  day29TwentyThree: page.days[29].hourSlots.find((slot) => slot.wallHour === 23).breakMarkers.map((event) => event.kind),
  firstDayEndMs: firstDay.endMs,
  firstBoundaryAfterStart: request.dayBoundariesMs[1]
}));
`
    );

    expect(page.breakCounts).toEqual([
      { kind: "scheduledShown", label: "Shown", count: 2 },
      { kind: "naturalIdle", label: "Natural", count: 1 },
      { kind: "manualTakeBreak", label: "Manual", count: 1 },
      { kind: "fullscreenSuppress", label: "Held", count: 1 }
    ]);
    expect(page.day0.scheduledShown).toBe(1);
    expect(page.day0.fullscreenSuppress).toBe(1);
    expect(page.day29.scheduledShown).toBe(1);
    expect(page.day29.naturalIdle).toBe(1);
    expect(page.day29.manualTakeBreak).toBe(1);
    expect(page.day0Midnight).toEqual(["scheduledShown", "fullscreenSuppress"]);
    expect(page.day29Midnight).toEqual(["naturalIdle"]);
    expect(page.day29Nine).toEqual(["manualTakeBreak", "scheduledShown"]);
    expect(page.day29Ten).toEqual([]);
    expect(page.day29TwentyThree).toEqual([]);
    expect(page.firstDayEndMs).toBe(page.firstBoundaryAfterStart);
  });

  test("combines the repeated fall-back hour into one wall-clock slot", () => {
    const repeated = runInTimezone<{
      bucketCount: number;
      activeMs: number;
      afkMs: number;
      kind: string;
      breakKinds: string[];
    }>(
      "America/New_York",
      `
const request = mod.buildHistoryPageRequest(0, new Date(2026, 10, 2, 12, 0, 0, 0).getTime(), 0);
const dailyBuckets = Array.from({ length: request.days.length }, () => ({
  activeMs: 0,
  afkMs: 0,
  longestActiveMs: 0
}));
const hourlyBuckets = Array.from({ length: request.hourBoundariesMs.length - 1 }, () => ({
  activeMs: 0,
  afkMs: 0,
  longestActiveMs: 0
}));
const repeatedDay = request.days.find((day) => day.dateKey === "2026-11-01");
const repeatedHour = repeatedDay.hourSlots.find((slot) => slot.wallHour === 1);
hourlyBuckets[repeatedHour.bucketIndexes[0]] = {
  activeMs: 600000,
  afkMs: 0,
  longestActiveMs: 600000
};
hourlyBuckets[repeatedHour.bucketIndexes[1]] = {
  activeMs: 1200000,
  afkMs: 300000,
  longestActiveMs: 900000
};
const page = mod.materializeHistoryPage(request, dailyBuckets, hourlyBuckets, [
  {
    atMs: request.hourBoundariesMs[repeatedHour.bucketIndexes[0]] + 5 * 60 * 1000,
    kind: "scheduledShown"
  },
  {
    atMs: request.hourBoundariesMs[repeatedHour.bucketIndexes[1]] + 10 * 60 * 1000,
    kind: "manualTakeBreak"
  }
]);
const slot = page.days.find((day) => day.dateKey === "2026-11-01").hourSlots.find(
  (hour) => hour.wallHour === 1
);
console.log(JSON.stringify({
  bucketCount: repeatedHour.bucketIndexes.length,
  activeMs: slot.activeMs,
  afkMs: slot.afkMs,
  kind: slot.kind,
  breakKinds: slot.breakMarkers.map((event) => event.kind)
}));
`
    );

    expect(repeated.bucketCount).toBe(2);
    expect(repeated.activeMs).toBe(1_800_000);
    expect(repeated.afkMs).toBe(300_000);
    expect(repeated.kind).toBe("mixed");
    expect(repeated.breakKinds).toEqual(["scheduledShown", "manualTakeBreak"]);
  });

  test("aligns Lord Howe spring buckets and break markers after the 30-minute gap", () => {
    const result = runInTimezone<{
      slotCount: number;
      twoIntervals: string[];
      threeIntervals: string[];
      two: { activeMs: number; breakKinds: string[] };
      three: { activeMs: number; breakKinds: string[] };
    }>(
      "Australia/Lord_Howe",
      `
const request = mod.buildHistoryPageRequest(0, new Date(2026, 9, 5, 12, 0, 0, 0).getTime(), 0);
const dailyBuckets = Array.from({ length: request.days.length }, () => ({
  activeMs: 0,
  afkMs: 0,
  longestActiveMs: 0
}));
const hourlyBuckets = Array.from({ length: request.hourBoundariesMs.length - 1 }, () => ({
  activeMs: 0,
  afkMs: 0,
  longestActiveMs: 0
}));
const day = request.days.find((entry) => entry.dateKey === "2026-10-04");
const event = { atMs: new Date(2026, 9, 4, 3, 0, 0, 0).getTime(), kind: "scheduledShown" };
const eventBucketIndex = request.hourBoundariesMs.findIndex(
  (start, index, all) => index + 1 < all.length && start <= event.atMs && event.atMs < all[index + 1]
);
hourlyBuckets[eventBucketIndex] = {
  activeMs: 600000,
  afkMs: 0,
  longestActiveMs: 600000
};
const page = mod.materializeHistoryPage(request, dailyBuckets, hourlyBuckets, [event]);
const materializedDay = page.days.find((entry) => entry.dateKey === "2026-10-04");
const twoRequest = day.hourSlots.find((slot) => slot.wallHour === 2);
const threeRequest = day.hourSlots.find((slot) => slot.wallHour === 3);
const interval = (bucketIndex) => {
  const start = new Date(request.hourBoundariesMs[bucketIndex]);
  const end = new Date(request.hourBoundariesMs[bucketIndex + 1]);
  const wall = (at) =>
    String(at.getHours()).padStart(2, "0") + ":" + String(at.getMinutes()).padStart(2, "0");
  return wall(start) + "/" + wall(end);
};
const two = materializedDay.hourSlots.find((slot) => slot.wallHour === 2);
const three = materializedDay.hourSlots.find((slot) => slot.wallHour === 3);
console.log(JSON.stringify({
  slotCount: day.hourSlots.length,
  twoIntervals: twoRequest.bucketIndexes.map(interval),
  threeIntervals: threeRequest.bucketIndexes.map(interval),
  two: { activeMs: two.activeMs, breakKinds: two.breakMarkers.map((marker) => marker.kind) },
  three: { activeMs: three.activeMs, breakKinds: three.breakMarkers.map((marker) => marker.kind) }
}));
`
    );

    expect(result.slotCount).toBe(24);
    expect(result.twoIntervals).toEqual(["02:30/03:00"]);
    expect(result.threeIntervals).toEqual(["03:00/04:00"]);
    expect(result.two).toEqual({ activeMs: 0, breakKinds: [] });
    expect(result.three).toEqual({ activeMs: 600_000, breakKinds: ["scheduledShown"] });
  });

  test("combines Lord Howe's repeated half-hour without shifting the next wall hour", () => {
    const result = runInTimezone<{
      oneIntervals: string[];
      twoIntervals: string[];
      one: { activeMs: number; afkMs: number; breakKinds: string[] };
      two: { activeMs: number; afkMs: number; breakKinds: string[] };
    }>(
      "Australia/Lord_Howe",
      `
const request = mod.buildHistoryPageRequest(0, new Date(2026, 3, 6, 12, 0, 0, 0).getTime(), 0);
const dailyBuckets = Array.from({ length: request.days.length }, () => ({
  activeMs: 0,
  afkMs: 0,
  longestActiveMs: 0
}));
const hourlyBuckets = Array.from({ length: request.hourBoundariesMs.length - 1 }, () => ({
  activeMs: 0,
  afkMs: 0,
  longestActiveMs: 0
}));
const day = request.days.find((entry) => entry.dateKey === "2026-04-05");
const oneRequest = day.hourSlots.find((slot) => slot.wallHour === 1);
hourlyBuckets[oneRequest.bucketIndexes[0]].activeMs = 600000;
hourlyBuckets[oneRequest.bucketIndexes[0]].longestActiveMs = 600000;
hourlyBuckets[oneRequest.bucketIndexes[1]].activeMs = 1200000;
hourlyBuckets[oneRequest.bucketIndexes[1]].longestActiveMs = 900000;
const event = { atMs: new Date(2026, 3, 5, 2, 0, 0, 0).getTime(), kind: "manualTakeBreak" };
const eventBucketIndex = request.hourBoundariesMs.findIndex(
  (start, index, all) => index + 1 < all.length && start <= event.atMs && event.atMs < all[index + 1]
);
hourlyBuckets[eventBucketIndex].afkMs = 300000;
const page = mod.materializeHistoryPage(request, dailyBuckets, hourlyBuckets, [event]);
const materializedDay = page.days.find((entry) => entry.dateKey === "2026-04-05");
const twoRequest = day.hourSlots.find((slot) => slot.wallHour === 2);
const interval = (bucketIndex) => {
  const start = new Date(request.hourBoundariesMs[bucketIndex]);
  const end = new Date(request.hourBoundariesMs[bucketIndex + 1]);
  const wall = (at) =>
    String(at.getHours()).padStart(2, "0") + ":" + String(at.getMinutes()).padStart(2, "0") +
      "@" + -at.getTimezoneOffset();
  return wall(start) + "/" + wall(end);
};
const one = materializedDay.hourSlots.find((slot) => slot.wallHour === 1);
const two = materializedDay.hourSlots.find((slot) => slot.wallHour === 2);
console.log(JSON.stringify({
  oneIntervals: oneRequest.bucketIndexes.map(interval),
  twoIntervals: twoRequest.bucketIndexes.map(interval),
  one: {
    activeMs: one.activeMs,
    afkMs: one.afkMs,
    breakKinds: one.breakMarkers.map((marker) => marker.kind)
  },
  two: {
    activeMs: two.activeMs,
    afkMs: two.afkMs,
    breakKinds: two.breakMarkers.map((marker) => marker.kind)
  }
}));
`
    );

    expect(result.oneIntervals).toEqual([
      "01:00@660/01:30@630",
      "01:30@630/02:00@630"
    ]);
    expect(result.twoIntervals).toEqual(["02:00@630/03:00@630"]);
    expect(result.one).toEqual({ activeMs: 1_800_000, afkMs: 0, breakKinds: [] });
    expect(result.two).toEqual({
      activeMs: 0,
      afkMs: 300_000,
      breakKinds: ["manualTakeBreak"]
    });
  });
});

describe("history activity calendar", () => {
  test("uses fixed active-minute levels while keeping no data distinct from zero", () => {
    expect(historyActivityLevel({ activeMs: 0, afkMs: 0 })).toBe("no-data");
    expect(historyActivityLevel({ activeMs: 0, afkMs: 60_000 })).toBe(0);
    expect(historyActivityLevel({ activeMs: 59 * 60_000, afkMs: 0 })).toBe(1);
    expect(historyActivityLevel({ activeMs: 60 * 60_000, afkMs: 0 })).toBe(2);
    expect(historyActivityLevel({ activeMs: 3 * 60 * 60_000, afkMs: 0 })).toBe(3);
    expect(historyActivityLevel({ activeMs: 5 * 60 * 60_000, afkMs: 0 })).toBe(4);
  });

  test("materializes exactly 90 chronological days in Monday-aligned week columns", () => {
    const result = runInTimezone<{
      dayCount: number;
      firstDateKey: string;
      lastDateKey: string;
      leadingEmptyCells: number;
      trailingEmptyCells: number;
      weekColumnCount: number;
      firstActiveMs: number;
      lastActiveMs: number;
    }>(
      "UTC",
      `
const request = mod.buildHistoryCalendarRequest(Date.parse("2026-08-13T10:30:00Z"), 4);
const buckets = request.pages.map((page, pageIndex) =>
  page.days.map((_, dayIndex) => ({
    activeMs: pageIndex * 100000 + dayIndex * 1000,
    afkMs: 0,
    longestActiveMs: 0
  }))
);
const calendar = mod.materializeHistoryCalendar(request, buckets);
console.log(JSON.stringify({
  dayCount: calendar.days.length,
  firstDateKey: calendar.days[0].dateKey,
  lastDateKey: calendar.days[calendar.days.length - 1].dateKey,
  leadingEmptyCells: calendar.leadingEmptyCells,
  trailingEmptyCells: calendar.trailingEmptyCells,
  weekColumnCount: calendar.weekColumnCount,
  firstActiveMs: calendar.days[0].totals.activeMs,
  lastActiveMs: calendar.days[calendar.days.length - 1].totals.activeMs
}));
`
    );

    expect(result).toEqual({
      dayCount: 90,
      firstDateKey: "2026-05-16",
      lastDateKey: "2026-08-13",
      leadingEmptyCells: 5,
      trailingEmptyCells: 3,
      weekColumnCount: 14,
      firstActiveMs: 200_000,
      lastActiveMs: 29_000
    });
  });

  test("uses thirteen columns when the exact range fits thirteen aligned weeks", () => {
    const result = runInTimezone<{
      firstDateKey: string;
      lastDateKey: string;
      leadingEmptyCells: number;
      trailingEmptyCells: number;
      weekColumnCount: number;
    }>(
      "UTC",
      `
const request = mod.buildHistoryCalendarRequest(Date.parse("2026-08-16T10:30:00Z"), 4);
const buckets = request.pages.map((page) =>
  page.days.map(() => ({ activeMs: 0, afkMs: 0, longestActiveMs: 0 }))
);
const calendar = mod.materializeHistoryCalendar(request, buckets);
console.log(JSON.stringify({
  firstDateKey: calendar.days[0].dateKey,
  lastDateKey: calendar.days[calendar.days.length - 1].dateKey,
  leadingEmptyCells: calendar.leadingEmptyCells,
  trailingEmptyCells: calendar.trailingEmptyCells,
  weekColumnCount: calendar.weekColumnCount
}));
`
    );

    expect(result).toEqual({
      firstDateKey: "2026-05-19",
      lastDateKey: "2026-08-16",
      leadingEmptyCells: 1,
      trailingEmptyCells: 0,
      weekColumnCount: 13
    });
  });

  test("places month labels over the week containing each month's first retained day", () => {
    const result = runInTimezone<Array<{ month: number; column: number }>>(
      "UTC",
      `
const request = mod.buildHistoryCalendarRequest(Date.parse("2026-08-13T10:30:00Z"), 4);
const buckets = request.pages.map((page) =>
  page.days.map(() => ({ activeMs: 0, afkMs: 0, longestActiveMs: 0 }))
);
const calendar = mod.materializeHistoryCalendar(request, buckets);
console.log(JSON.stringify(mod.historyMonthMarkers(calendar).map((marker) => ({
  month: new Date(marker.startMs).getMonth() + 1,
  column: marker.column
}))));
`
    );

    expect(result).toEqual([
      { month: 5, column: 1 },
      { month: 6, column: 4 },
      { month: 7, column: 8 },
      { month: 8, column: 12 }
    ]);
  });

  test("selects today even when it has no recorded activity", () => {
    const days = [
      calendarDay("2026-08-14", 60_000, 0),
      calendarDay("2026-08-15", 0, 120_000),
      calendarDay("2026-08-16", 0, 0)
    ];

    expect(initialHistoryDateKey(days)).toBe("2026-08-16");
    expect(initialHistoryDateKey(days.map((day) => calendarDay(day.dateKey, 0, 0)))).toBe(
      "2026-08-16"
    );
  });

  test("maps pointer, keyboard, Escape, and hourly focus interactions", () => {
    expect(historyActivationUsesKeyboard(0)).toBe(true);
    expect(historyActivationUsesKeyboard(1)).toBe(false);
    expect(historyEscapeAction(9)).toBe("close-break-popover");
    expect(historyEscapeAction(null)).toBe("return-dashboard");
    expect(moveHistoryHourFocus(8, "ArrowLeft", 24)).toBe(7);
    expect(moveHistoryHourFocus(8, "ArrowRight", 24)).toBe(9);
    expect(moveHistoryHourFocus(8, "Home", 24)).toBe(0);
    expect(moveHistoryHourFocus(8, "End", 24)).toBe(23);
    expect(moveHistoryHourFocus(0, "ArrowLeft", 24)).toBe(0);
    expect(moveHistoryHourFocus(23, "ArrowRight", 24)).toBe(23);
  });

  test("refreshes a mounted calendar only after its logical day changes", () => {
    const result = runInTimezone<boolean[]>(
      "UTC",
      `
const request = mod.buildHistoryCalendarRequest(
  Date.parse("2026-08-17T10:00:00Z"),
  4
);
console.log(JSON.stringify([
  mod.historyCalendarNeedsRefresh(false, false, request, Date.parse("2026-08-17T10:00:00Z")),
  mod.historyCalendarNeedsRefresh(true, false, request, Date.parse("2026-08-17T10:00:00Z")),
  mod.historyCalendarNeedsRefresh(true, true, request, Date.parse("2026-08-18T03:59:59Z")),
  mod.historyCalendarNeedsRefresh(true, true, request, Date.parse("2026-08-18T04:00:00Z"))
]));
`
    );

    expect(result).toEqual([false, true, false, true]);
  });

  test("describes exact hourly activity durations", () => {
    expect(
      historyHourDetailLabel({
        label: "9 AM",
        activeMs: 25 * 60_000,
        afkMs: 10 * 60_000,
        longestActiveMs: 12 * 60_000
      })
    ).toBe("9 AM: 25m active, 10m away, 12m longest stretch");
    expect(
      historyHourDetailLabel({
        label: "10 AM",
        activeMs: 0,
        afkMs: 0,
        longestActiveMs: 0
      })
    ).toBe("10 AM: 0m active, 0m away, 0m longest stretch");
  });

  test("moves focus by day or week and clamps at the retained range", () => {
    const days = Array.from({ length: 10 }, (_, index) =>
      calendarDay(`day-${index}`, 1_000, 0)
    );

    expect(moveHistoryGridFocus(days, "day-4", "ArrowLeft")).toBe("day-0");
    expect(moveHistoryGridFocus(days, "day-4", "ArrowRight")).toBe("day-9");
    expect(moveHistoryGridFocus(days, "day-4", "ArrowUp")).toBe("day-3");
    expect(moveHistoryGridFocus(days, "day-4", "ArrowDown")).toBe("day-5");
    expect(moveHistoryGridFocus(days, "day-0", "ArrowLeft")).toBe("day-0");
    expect(moveHistoryGridFocus(days, "day-9", "ArrowDown")).toBe("day-9");
  });

  test("isolates one selected day for hourly and break requests", () => {
    const result = runInTimezone<{
      dayCount: number;
      startIso: string;
      endIso: string;
      dayBoundaryCount: number;
      hourBucketCount: number;
      slotCount: number;
      bucketIndexesAreLocal: boolean;
    }>(
      "America/New_York",
      `
const calendar = mod.buildHistoryCalendarRequest(
  new Date(2026, 10, 2, 12, 0, 0, 0).getTime(),
  0
);
const selected = mod.buildHistoryDayDetailRequest(calendar, "2026-11-01");
console.log(JSON.stringify({
  dayCount: selected.days.length,
  startIso: new Date(selected.startMs).toISOString(),
  endIso: new Date(selected.endMs).toISOString(),
  dayBoundaryCount: selected.dayBoundariesMs.length,
  hourBucketCount: selected.hourBoundariesMs.length - 1,
  slotCount: selected.days[0].hourSlots.length,
  bucketIndexesAreLocal: selected.days[0].hourSlots
    .flatMap((slot) => slot.bucketIndexes)
    .every((bucketIndex) => bucketIndex >= 0 && bucketIndex < selected.hourBoundariesMs.length - 1)
}));
`
    );

    expect(result).toEqual({
      dayCount: 1,
      startIso: "2026-11-01T04:00:00.000Z",
      endIso: "2026-11-02T05:00:00.000Z",
      dayBoundaryCount: 2,
      hourBucketCount: 25,
      slotCount: 24,
      bucketIndexesAreLocal: true
    });
  });
});

function calendarDay(dateKey: string, activeMs: number, afkMs: number): HistoryCalendarDay {
  return {
    pageIndex: 0,
    index: 0,
    dateKey,
    label: dateKey,
    startMs: 0,
    endMs: 1,
    totals: {
      activeMs,
      afkMs,
      longestActiveMs: activeMs,
      activeLabel: "",
      afkLabel: "",
      longestLabel: "",
      isBlank: activeMs === 0 && afkMs === 0
    },
    activityLevel: historyActivityLevel({ activeMs, afkMs })
  };
}
