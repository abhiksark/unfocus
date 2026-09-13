// src/lib/pre-break-cue.test.ts

import { describe, expect, test } from "bun:test";
import {
  preBreakCueAnimationDelayMs,
  preBreakCuePlatformFromNativeContext,
  preBreakCuePresentation,
  preBreakCueRingProgress,
  preBreakCueViewFromPresentation
} from "./pre-break-cue";

describe("pre-break cue", () => {
  test("ring follows the deadline continuously, with preview handoff excluded", () => {
    for (const [mode, offset] of [["scheduled", 0], ["preview", 1_000]] as const) {
      expect(preBreakCueRingProgress(10_000 + offset, mode)).toBe(1);
      expect(preBreakCueRingProgress(7_250 + offset, mode)).toBe(0.725);
      expect(preBreakCueRingProgress(1_000 + offset, mode)).toBe(0.1);
      expect(preBreakCueRingProgress(50 + offset, mode)).toBe(0.005);
      expect(preBreakCueRingProgress(offset, mode)).toBe(0);
      expect(preBreakCueRingProgress(-1, mode)).toBe(0);
    }
    expect(preBreakCueRingProgress(60_000, "scheduled")).toBe(1);
    expect(preBreakCueRingProgress(17_000, "preview")).toBe(1);
  });
  test("selects macOS only from the exact trusted native literal", () => {
    expect(preBreakCuePlatformFromNativeContext("macos")).toBe("macos");
    expect(preBreakCuePlatformFromNativeContext("linux")).toBe("linux");
    expect(preBreakCuePlatformFromNativeContext("MacIntel")).toBe("linux");
    expect(preBreakCuePlatformFromNativeContext(undefined)).toBe("linux");
  });

  test("uses the scheduled T-60, T-56, T-10, and zero boundaries", () => {
    const deadline = 100_000;
    expect(preBreakCuePresentation(deadline, 39_999, "scheduled")).toEqual({
      stage: "quiet",
      secondsLeft: 61,
      visible: false
    });
    expect(preBreakCuePresentation(deadline, 40_000, "scheduled")).toEqual({
      stage: "heads-up",
      secondsLeft: 60,
      visible: true
    });
    expect(preBreakCuePresentation(deadline, 43_999, "scheduled")).toEqual({
      stage: "heads-up",
      secondsLeft: 57,
      visible: true
    });
    expect(preBreakCuePresentation(deadline, 44_000, "scheduled")).toEqual({
      stage: "quiet",
      secondsLeft: 56,
      visible: false
    });
    expect(preBreakCuePresentation(deadline, 89_999, "scheduled")).toEqual({
      stage: "quiet",
      secondsLeft: 11,
      visible: false
    });
    expect(preBreakCuePresentation(deadline, 90_000, "scheduled")).toEqual({
      stage: "horizon",
      secondsLeft: 10,
      visible: true
    });
    expect(preBreakCuePresentation(deadline, 99_999, "scheduled")).toEqual({
      stage: "horizon",
      secondsLeft: 1,
      visible: true
    });
    expect(preBreakCuePresentation(deadline, 100_000, "scheduled")).toEqual({
      stage: "handoff",
      secondsLeft: 0,
      visible: true
    });
  });

  test("aligns late scheduled heads-up and horizon animation phases to the deadline", () => {
    expect(preBreakCueAnimationDelayMs(58_750, "scheduled")).toBe(-1_250);
    expect(preBreakCueAnimationDelayMs(7_200, "scheduled")).toBe(-2_800);
  });

  test("fits the preview into four heads-up, two quiet, ten horizon, and one handoff seconds", () => {
    const deadline = 117_000;
    expect(preBreakCuePresentation(deadline, 99_999, "preview").stage).toBe("quiet");
    expect(preBreakCuePresentation(deadline, 100_000, "preview")).toEqual({
      stage: "heads-up",
      secondsLeft: 17,
      visible: true
    });
    expect(preBreakCuePresentation(deadline, 103_999, "preview").stage).toBe("heads-up");
    expect(preBreakCuePresentation(deadline, 104_000, "preview")).toEqual({
      stage: "quiet",
      secondsLeft: 13,
      visible: false
    });
    expect(preBreakCuePresentation(deadline, 105_999, "preview").stage).toBe("quiet");
    expect(preBreakCuePresentation(deadline, 106_000, "preview")).toEqual({
      stage: "horizon",
      secondsLeft: 11,
      visible: true
    });
    expect(preBreakCuePresentation(deadline, 115_999, "preview").stage).toBe("horizon");
    expect(preBreakCuePresentation(deadline, 116_000, "preview")).toEqual({
      stage: "handoff",
      secondsLeft: 1,
      visible: true
    });
    expect(preBreakCuePresentation(deadline, 117_000, "preview")).toEqual({
      stage: "handoff",
      secondsLeft: 0,
      visible: true
    });
  });

  test("aligns late preview heads-up and horizon animation phases to the close deadline", () => {
    expect(preBreakCueAnimationDelayMs(15_400, "preview")).toBe(-1_600);
    expect(preBreakCueAnimationDelayMs(6_250, "preview")).toBe(-4_750);
  });

  test("uses compact notch copy and counts down without changing the Linux model", () => {
    const deadline = 100_000;
    expect(preBreakCueViewFromPresentation("macos", preBreakCuePresentation(deadline, 40_000))).toEqual({
      surface: "notch", title: "Break in 1m", support: null, countdown: null
    });
    expect(preBreakCueViewFromPresentation("macos", preBreakCuePresentation(deadline, 90_000)).countdown).toBe(10);
    expect(preBreakCueViewFromPresentation("macos", preBreakCuePresentation(deadline, 99_999)).countdown).toBe(1);
    expect(preBreakCueViewFromPresentation("macos", preBreakCuePresentation(deadline, 100_000)).title).toBe("Look away");
  });

  test("the preview subtracts its handoff second and displays exactly 10 through 1", () => {
    const values = Array.from({ length: 10 }, (_, index) =>
      preBreakCueViewFromPresentation("macos", preBreakCuePresentation(17_000, 6_000 + index * 1_000, "preview"), "preview").countdown
    );
    expect(values).toEqual([10, 9, 8, 7, 6, 5, 4, 3, 2, 1]);
    expect(preBreakCueViewFromPresentation("macos", preBreakCuePresentation(17_000, 16_000, "preview"), "preview").countdown).toBeNull();
  });

  test("preserves the Ubuntu X11 card copy and numeric final countdown", () => {
    const deadline = 100_000;
    expect(
      preBreakCueViewFromPresentation(
        "linux",
        preBreakCuePresentation(deadline, 40_000)
      )
    ).toEqual({
      surface: "card",
      title: "Eye break in 1 minute",
      support: "Finish your thought.",
      countdown: null
    });
    expect(
      preBreakCueViewFromPresentation(
        "linux",
        preBreakCuePresentation(deadline, 92_250)
      )
    ).toEqual({
      surface: "card",
      title: "Eye break",
      support: "Finish your thought.",
      countdown: 8
    });
    expect(
      preBreakCueViewFromPresentation(
        "linux",
        preBreakCuePresentation(deadline, 100_000)
      )
    ).toEqual({
      surface: "card",
      title: "Look away",
      support: "Rest your focus beyond the screen.",
      countdown: null
    });
  });

  test("feeds the preserved Ubuntu X11 view model into the Linux markup", async () => {
    const source = await Bun.file(new URL("./PreBreakCue.svelte", import.meta.url)).text();
    const linuxBranch =
      source.split('{:else}\n      <div\n        class="cue-card"')[1]?.split("    {/if}")[0] ??
      "";

    expect(linuxBranch).toContain("{view.title}");
    expect(linuxBranch).toContain("{view.support}");
    expect(linuxBranch).toContain("{view.countdown}");
  });

  test("uses one fixed premium-hybrid card without a progress rail, expanding shell, or blur", async () => {
    const source = await Bun.file(new URL("./PreBreakCue.svelte", import.meta.url)).text();

    expect(source).toContain("width: min(336px, calc(100vw - 40px))");
    expect(source).toContain("height: min(68px, calc(100vh - 40px))");
    expect(source).toContain("border-radius: 24px");
    expect(source).toContain("{#if nativeVisible}");
    expect(source).toContain('invoke("set_pre_break_cue_visibility", { visible })');
    expect(source).toContain("animation: cue-card-arrive 160ms ease both");
    expect(source).not.toContain("cue-progress");
    expect(source).not.toContain("class:countdown");
    expect(source).not.toMatch(/^\s+filter:/m);
  });

  test("reserves the physical camera gap and prepares native geometry before reveal", async () => {
    const source = await Bun.file(new URL("./PreBreakCue.svelte", import.meta.url)).text();
    expect(source).toContain('class="camera-gap"');
    expect(source).toContain('invoke<PreBreakCueLayout>("prepare_pre_break_cue")');
    expect(source).toContain("await tick()");
    expect(source).toContain("document.documentElement.getBoundingClientRect()");
    expect(source).not.toContain("requestAnimationFrame");
    expect(source).not.toContain("setTimeout");
    expect(source).toContain("width: calc(var(--notch-width) + 2 * var(--wing-width))");
    expect(source).toContain("--shoulder-width: ${layout.shoulderWidth}px");
    expect(source).not.toContain("macos-notification");
    expect(source).toContain("width: var(--shoulder-width)");
    expect(source).toContain("8.75%, 91.25%");
    expect(source).toContain('presentation.stage === "heads-up" ? "heads-up" : "final"');
  });

  test("freezes the deadline-derived phase delay at macOS stage insertion", async () => {
    const source = await Bun.file(new URL("./PreBreakCue.svelte", import.meta.url)).text();

    expect(source).toContain("preBreakCueAnimationDelayMs(initialRemainingMs, mode)");
    expect(source).not.toContain("$derived(preBreakCueAnimationDelayMs(remainingMs, mode))");
    expect(source).toContain("let animationDelayMs = $state(initialAnimationDelayMs)");
    expect(source.match(/--macos-cue-animation-delay/g)).toHaveLength(3);
    expect(source.match(/animation-delay: var\(--macos-cue-animation-delay\)/g)).toHaveLength(2);
  });

  test("captures a fresh deadline offset when the horizon stage begins after page load", async () => {
    const source = await Bun.file(new URL("./PreBreakCue.svelte", import.meta.url)).text();

    expect(source).toContain("$effect.pre(() =>");
    expect(source).toContain("animationDelayMs = preBreakCueAnimationDelayMs(remainingMs, mode)");
  });

  test("keeps reduced motion to short opacity and color fades", async () => {
    const source = await Bun.file(new URL("./PreBreakCue.svelte", import.meta.url)).text();
    const reducedMotion = source.split("@media (prefers-reduced-motion: reduce)")[1];

    expect(reducedMotion).toContain("animation: cue-card-arrive 120ms linear both");
    expect(reducedMotion).toContain("border-color 120ms linear");
    expect(reducedMotion).not.toContain("blur");
  });

  test("keeps reduced-motion macOS transitions to a short opacity fade", async () => {
    const source = await Bun.file(new URL("./PreBreakCue.svelte", import.meta.url)).text();
    const reducedMotion = source.split("@media (prefers-reduced-motion: reduce)")[1] ?? "";
    const macosRules = reducedMotion.split(".cue-card")[0];

    expect(macosRules).toContain(".notch-shape");
    expect(macosRules).toContain("animation: cue-content-arrive 120ms linear both");
    expect(macosRules).toContain("animation: notch-heads-up-fade 4s linear both");
    expect(source).toContain("3%, 97% { opacity: 1; }");
    expect(macosRules).not.toContain("10s");
    expect(macosRules).not.toContain("scale");
  });

  test("uses token-colored ring and system timing without an eye in the macOS notch", async () => {
    const source = await Bun.file(new URL("./PreBreakCue.svelte", import.meta.url)).text();
    const notchRules = source.split(".notch-cue {")[1]?.split(".cue-card {")[0] ?? "";
    expect(notchRules).toContain("color: var(--ink)");
    expect(notchRules).toContain("stroke: var(--accent)");
    expect(notchRules).toContain("font-size: 15px");
    expect(source).not.toContain('class="notch-eye"');
    expect(source).toContain('class="ring-progress"');
    expect(notchRules).toContain("background: #000000");
    expect(notchRules).toContain("font-variant-numeric: tabular-nums");
  });
});
