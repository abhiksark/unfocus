import { describe, expect, test } from "bun:test";
import {
  countdownLiveRegionAllowed,
  OVERLAY_END_BREAK_LABEL,
  OVERLAY_REGION_LABEL,
  overlayAnnouncement,
  overlayCountdownLabel
} from "./overlay-a11y";

describe("overlay accessibility contract", () => {
  test("does not announce every second of the countdown", () => {
    expect(countdownLiveRegionAllowed()).toBe(false);
    for (let seconds = 20; seconds >= 0; seconds -= 1) {
      if (seconds === 5 || seconds === 0) continue;
      expect(overlayAnnouncement({ complete: false, secondsLeft: seconds })).toBe(
        ""
      );
    }
  });

  test("announces only the five-second and complete milestones", () => {
    expect(overlayAnnouncement({ complete: false, secondsLeft: 5 })).toBe(
      "Five seconds remain in this break."
    );
    expect(overlayAnnouncement({ complete: true, secondsLeft: 0 })).toBe(
      "Break complete."
    );
  });

  test("exposes stable region and control names", () => {
    expect(OVERLAY_REGION_LABEL).toBe("Eye break");
    expect(OVERLAY_END_BREAK_LABEL).toBe("End break");
  });

  test("countdown label carries remaining time without live spam", () => {
    expect(overlayCountdownLabel(false, 12)).toBe("12 seconds remaining");
    expect(overlayCountdownLabel(true, 0)).toBe("Break complete");
  });
});
