// src/lib/overlay-entrance.test.ts

import { describe, expect, test } from "bun:test";

describe("overlay staged entrance", () => {
  test("uses the synchronized opacity and text-blur timeline without entrance transforms", async () => {
    const source = await Bun.file(new URL("./BreakOverlay.svelte", import.meta.url)).text();

    expect(source).toContain("animation: atmosphere-reveal 1.2s");
    expect(source).toContain("animation-delay: calc(1100ms - var(--presentation-offset))");
    expect(source).toContain("animation-delay: calc(1550ms - var(--presentation-offset))");
    expect(source).toContain("animation-delay: calc(1800ms - var(--presentation-offset))");

    for (const name of ["heading-reveal", "supporting-reveal", "chrome-reveal"]) {
      const keyframes = source.split(`@keyframes ${name}`)[1]?.split("@keyframes")[0] ?? "";
      expect(keyframes).not.toContain("transform:");
    }
  });

  test("reveals content immediately with reduced motion", async () => {
    const source = await Bun.file(new URL("./BreakOverlay.svelte", import.meta.url)).text();
    const reducedMotion = source.split("@media (prefers-reduced-motion: reduce)")[1];

    expect(reducedMotion).toContain(".heading-stage");
    expect(reducedMotion).toContain(".guidance");
    expect(reducedMotion).toContain(".timer");
    expect(reducedMotion).toContain("animation: none");
  });
});
