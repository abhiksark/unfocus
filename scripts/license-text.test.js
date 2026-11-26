import { expect, test } from "bun:test";
import { normalizeLicenseText } from "./license-text.js";

test("normalizes CRLF and trailing whitespace without changing license wording or indentation", () => {
  expect(normalizeLicenseText("MIT License \r\n\r\n  Copyright Example\t\r\nPermission is granted.  \r\n\r\n"))
    .toBe("MIT License\n\n  Copyright Example\nPermission is granted.\n");
});

test("license normalization is idempotent", () => {
  const text = "License\n  Indented text\n";
  expect(normalizeLicenseText(normalizeLicenseText(text))).toBe(text);
});
