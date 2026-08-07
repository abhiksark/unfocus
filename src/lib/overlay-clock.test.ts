import { describe, expect, test } from "bun:test";
import {
  anchorFromRemaining,
  createOverlayClock,
  formatCountdown,
  presentationOffset,
  remainingAt
} from "./overlay-clock";

describe("overlay clock", () => {
  test("uses wall time only to establish the initial monotonic anchor", () => {
    const anchor = createOverlayClock(20, 120_000, 105_000, 500);
    expect(anchor.initialRemainingMs).toBe(15_000);
    expect(remainingAt(anchor, 3_000)).toBe(12_500);
    expect(presentationOffset(anchor)).toBe(5_000);
  });

  test("clamps late loading and backwards monotonic samples", () => {
    const late = createOverlayClock(8, 100, 200, 50);
    expect(remainingAt(late, 100)).toBe(0);

    const native = anchorFromRemaining(8_000, 6_000, 500);
    expect(remainingAt(native, 400)).toBe(6_000);
    expect(remainingAt(native, 7_000)).toBe(0);
  });

  test("formats a stable countdown", () => {
    expect(formatCountdown(0)).toBe("00:00");
    expect(formatCountdown(65)).toBe("01:05");
  });
});
