import { describe, expect, test } from "bun:test";
import { deviceGridOffsetMinutes, formatGridOffset, gridPreview, nextGrid } from "./break-grid";

const BASE = 1_787_220_420; // 2026-08-20T10:07:00Z

describe("nextGrid", () => {
  // This table is mirrored byte-for-byte in
  // src-tauri/src/reminder/schedule.rs. Change both together.
  //
  // The grid is anchored to the local-time epoch (unix_secs + offset*60),
  // not to local midnight; the two coincide only when interval_secs divides
  // 86400. Rows (840, 1500) and (-720, 1500) are the ones in this table
  // where a local-midnight anchor would produce a different answer, so they
  // are what actually pins the local-epoch behavior here. Do not remove or
  // "simplify" them.
  const cases: [number, number, number][] = [
    [0, 1200, 1_787_221_200],
    [0, 1500, 1_787_221_500],
    [330, 1200, 1_787_220_600],
    [330, 1500, 1_787_221_200],
    [345, 1200, 1_787_220_900],
    [345, 1500, 1_787_221_800],
    [765, 1200, 1_787_220_900],
    [765, 1500, 1_787_220_600],
    [-300, 1200, 1_787_221_200],
    [-300, 1500, 1_787_221_500],
    [-720, 1200, 1_787_221_200],
    [-720, 1500, 1_787_221_200], // discriminates local-epoch from local-midnight
    [840, 1200, 1_787_221_200],
    [840, 1500, 1_787_220_600] // discriminates local-epoch from local-midnight
  ];

  for (const [offset, interval, expected] of cases) {
    test(`offset ${offset} interval ${interval}`, () => {
      expect(nextGrid(BASE, interval, offset)).toBe(expected);
    });
  }

  test("is always strictly after the observation", () => {
    const onPoint = nextGrid(BASE, 1200, 0);
    expect(nextGrid(onPoint, 1200, 0) - onPoint).toBe(1200);
  });
});

describe("deviceGridOffsetMinutes", () => {
  test("negates getTimezoneOffset so east of UTC is positive", () => {
    // Stubbed rather than read from the ambient zone, so the test is
    // deterministic on any machine and in CI.
    const date = new Date(BASE * 1000);
    date.getTimezoneOffset = () => -330; // IST
    expect(deviceGridOffsetMinutes(date)).toBe(330);
  });

  test("keeps west of UTC negative", () => {
    const date = new Date(BASE * 1000);
    date.getTimezoneOffset = () => 300; // EST
    expect(deviceGridOffsetMinutes(date)).toBe(-300);
  });
});

describe("formatGridOffset", () => {
  // The hourly preview is offset-independent, so the offset must be shown
  // separately or two devices in different zones would display an identical
  // pattern while breaking ten minutes apart.
  test("renders offsets in a comparable form", () => {
    expect(formatGridOffset(330)).toBe("UTC+05:30");
    expect(formatGridOffset(345)).toBe("UTC+05:45");
    expect(formatGridOffset(-300)).toBe("UTC-05:00");
    expect(formatGridOffset(0)).toBe("UTC");
  });
});

describe("gridPreview", () => {
  test("reports a repeating hourly pattern when the interval divides an hour", () => {
    expect(gridPreview(BASE * 1000, 20, 330)).toEqual({
      kind: "hourly",
      minutes: [0, 20, 40]
    });
  });

  test("gives the same hourly pattern whatever the offset", () => {
    // Deliberate: an interval dividing an hour always lands on multiples of
    // itself in local time. This is why formatGridOffset must be shown too.
    for (const offset of [0, 330, 345, 765, -300, -720, 840]) {
      expect(gridPreview(BASE * 1000, 20, offset)).toEqual({
        kind: "hourly",
        minutes: [0, 20, 40]
      });
    }
  });

  test("describes a 1-minute interval as every minute instead of listing all sixty", () => {
    // 1 divides an hour, so this would otherwise fall into the "hourly"
    // branch and list all sixty minutes past the hour, wrecking the layout.
    expect(gridPreview(BASE * 1000, 1, 330)).toEqual({ kind: "everyMinute" });
  });

  test("every-minute description is the same whatever the offset", () => {
    for (const offset of [0, 330, 345, 765, -300, -720, 840]) {
      expect(gridPreview(BASE * 1000, 1, offset)).toEqual({ kind: "everyMinute" });
    }
  });

  test("falls back to upcoming times when there is no hourly repeat", () => {
    const preview = gridPreview(BASE * 1000, 25, 330);
    expect(preview.kind).toBe("upcoming");
    if (preview.kind !== "upcoming") throw new Error("unreachable");
    expect(preview.atMs).toEqual([1_787_221_200_000, 1_787_222_700_000, 1_787_224_200_000]);
  });
});
