import { describe, expect, test } from "bun:test";
import { pauseActionCommand } from "./reminder-status";

describe("reminder controls", () => {
  test("dispatches only the native action represented by the shared status", () => {
    expect(pauseActionCommand({ pauseAction: "pause" })).toBe("pause_reminders");
    expect(pauseActionCommand({ pauseAction: "resume" })).toBe("resume_reminders");
  });
});
