import { describe, expect, test } from "bun:test";
import {
  DAY_START_STORAGE_KEY,
  DEFAULT_DAY_START_HOUR,
  dayStartOptions,
  readDayStartHour,
  writeDayStartHour
} from "./day-start";

function reader(value: string | null) {
  return { getItem: () => value };
}

describe("readDayStartHour", () => {
  test("defaults to midnight without storage", () => {
    expect(readDayStartHour(null)).toBe(DEFAULT_DAY_START_HOUR);
    expect(DEFAULT_DAY_START_HOUR).toBe(0);
  });

  test("reads a stored hour", () => {
    expect(readDayStartHour(reader("6"))).toBe(6);
    expect(readDayStartHour(reader("0"))).toBe(0);
    expect(readDayStartHour(reader("23"))).toBe(23);
  });

  test("falls back on anything unusable", () => {
    for (const value of [null, "", "24", "-1", "6.5", "six", "1e1", " "]) {
      expect(readDayStartHour(reader(value))).toBe(DEFAULT_DAY_START_HOUR);
    }
  });

  test("falls back when storage throws", () => {
    const hostile = {
      getItem: () => {
        throw new Error("denied");
      }
    };
    expect(readDayStartHour(hostile)).toBe(DEFAULT_DAY_START_HOUR);
  });
});

describe("writeDayStartHour", () => {
  test("writes a valid hour under the versioned key", () => {
    const written: [string, string][] = [];
    const storage = {
      setItem: (key: string, value: string) => {
        written.push([key, value]);
      }
    };

    expect(writeDayStartHour(storage, 6)).toBe(true);
    expect(written).toEqual([[DAY_START_STORAGE_KEY, "6"]]);
  });

  test("refuses an hour outside 0-23 and writes nothing", () => {
    const written: string[] = [];
    const storage = { setItem: (_key: string, value: string) => void written.push(value) };

    for (const hour of [-1, 24, 6.5, Number.NaN]) {
      expect(writeDayStartHour(storage, hour)).toBe(false);
    }
    expect(written).toEqual([]);
  });

  test("returns false without storage or when storage throws", () => {
    expect(writeDayStartHour(null, 6)).toBe(false);
    expect(
      writeDayStartHour(
        {
          setItem: () => {
            throw new Error("quota");
          }
        },
        6
      )
    ).toBe(false);
  });
});

describe("dayStartOptions", () => {
  test("offers every whole hour once, in order", () => {
    const options = dayStartOptions();

    expect(options).toHaveLength(24);
    expect(options.map((option) => option.hour)).toEqual(
      Array.from({ length: 24 }, (_, hour) => hour)
    );
    expect(new Set(options.map((option) => option.label)).size).toBe(24);
    for (const option of options) {
      expect(option.label.length).toBeGreaterThan(0);
    }
  });
});
