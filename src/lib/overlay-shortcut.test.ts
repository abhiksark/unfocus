// src/lib/overlay-shortcut.test.ts

import { describe, expect, test } from "bun:test";
import { isOverlayDismissShortcut } from "./overlay-shortcut";

const plainKey = {
  altKey: false,
  ctrlKey: false,
  isComposing: false,
  key: " ",
  metaKey: false,
  repeat: false,
  shiftKey: false
};

describe("overlay dismiss shortcut", () => {
  test("accepts plain Space and Escape", () => {
    expect(isOverlayDismissShortcut(plainKey)).toBe(true);
    expect(isOverlayDismissShortcut({ ...plainKey, key: "Escape" })).toBe(true);
  });

  test("rejects held, modified, composing, and unrelated keys", () => {
    expect(isOverlayDismissShortcut({ ...plainKey, repeat: true })).toBe(false);
    expect(isOverlayDismissShortcut({ ...plainKey, key: "Escape", repeat: true })).toBe(false);
    expect(isOverlayDismissShortcut({ ...plainKey, shiftKey: true })).toBe(false);
    expect(isOverlayDismissShortcut({ ...plainKey, ctrlKey: true })).toBe(false);
    expect(isOverlayDismissShortcut({ ...plainKey, altKey: true })).toBe(false);
    expect(isOverlayDismissShortcut({ ...plainKey, metaKey: true })).toBe(false);
    expect(isOverlayDismissShortcut({ ...plainKey, isComposing: true })).toBe(false);
    expect(isOverlayDismissShortcut({ ...plainKey, key: "Enter" })).toBe(false);
    expect(isOverlayDismissShortcut({ ...plainKey, key: "a" })).toBe(false);
  });
});
