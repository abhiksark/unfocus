<!-- src/lib/PreBreakCue.svelte -->

<script lang="ts">
  import {
    preBreakCueAnimationDelayMs,
    preBreakCuePresentationFromRemaining,
    preBreakCueRingProgress,
    preBreakCueSkipAvailable,
    preBreakCueViewFromPresentation,
    type PreBreakCueLayout,
    type PreBreakCueMode,
    type PreBreakCuePlatform
  } from "$lib/pre-break-cue";
  import { createCueSkipController, type CueSkipState } from "$lib/pre-break-cue-skip";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, tick, untrack } from "svelte";
  import { createCueVisibilityController } from "$lib/pre-break-cue-native";

  type Props = {
    deadlineMs: number;
    mode: PreBreakCueMode;
    platform: PreBreakCuePlatform;
  };

  let { deadlineMs, mode, platform }: Props = $props();
  const initialRemainingMs = untrack(() => Math.max(0, deadlineMs - Date.now()));
  const { initialStage, initialAnimationDelayMs } = untrack(() => ({
    initialStage: preBreakCuePresentationFromRemaining(initialRemainingMs, mode).stage,
    initialAnimationDelayMs: preBreakCueAnimationDelayMs(initialRemainingMs, mode)
  }));
  const monotonicStartedAtMs = performance.now();
  let monotonicNowMs = $state(monotonicStartedAtMs);
  let remainingMs = $derived(
    Math.max(0, initialRemainingMs - (monotonicNowMs - monotonicStartedAtMs))
  );
  let presentation = $derived(preBreakCuePresentationFromRemaining(remainingMs, mode));
  let animationStage = $state(initialStage);
  let animationDelayMs = $state(initialAnimationDelayMs);
  let view = $derived(preBreakCueViewFromPresentation(platform, presentation, mode));
  let skipState = $state<CueSkipState>("idle");
  let skipError = $state<string | null>(null);
  let skipped = $derived(skipState === "skipped" || skipState === "retracting");
  let nativeVisible = $derived(skipped || presentation.visible);
  const skipController = createCueSkipController({
    request: () => invoke("skip_pre_break_cue"),
    changed: (state) => { skipState = state; skipError = null; },
    failed: (error) => { skipError = String(error); console.error("Could not skip this break", error); }
  });
  let ringProgress = $derived(preBreakCueRingProgress(remainingMs, mode));

  let layout = $state<PreBreakCueLayout | null>(null);
  let nativeHandoff = $state(false);
  let skipAvailable = $derived(platform === "macos" && !!layout && !nativeHandoff && skipState === "idle" && preBreakCueSkipAvailable(remainingMs, mode));
  let layoutRevision = $state(0);
  let retracting = $derived(skipState === "retracting" || (!skipped && (nativeHandoff || (mode === "preview" && presentation.stage === "handoff"))));
  const visibility = untrack(() => createCueVisibilityController({
    prepare: platform === "macos" ? () => invoke<PreBreakCueLayout>("prepare_pre_break_cue") : undefined,
    applyLayout: async (prepared) => {
      layout = prepared;
      layoutRevision += 1;
      animationDelayMs = preBreakCueAnimationDelayMs(
        Math.max(0, initialRemainingMs - (performance.now() - monotonicStartedAtMs)), mode
      );
      await tick();
      // Hidden WKWebView suspends RAF and can clamp even a timeout fallback.
      // Flush style/layout synchronously; showing the panel releases its first paint.
      void document.documentElement.getBoundingClientRect();
    },
    setVisible: (visible) => invoke("set_pre_break_cue_visibility", { visible }),
    onError: (error) => console.error("Could not update pre-break cue visibility", error)
  }));

  $effect.pre(() => {
    const stage = presentation.stage;
    if (stage === animationStage) return;
    animationStage = stage;
    if (stage === "handoff") return;
    animationDelayMs = preBreakCueAnimationDelayMs(remainingMs, mode);
  });

  $effect(() => {
    void visibility.setVisible(nativeVisible);
  });

  $effect(() => {
    if (platform === "macos" && layout) {
      void invoke("set_pre_break_cue_interactive", { enabled: skipAvailable })
        .catch((error) => console.error("Could not update cue hit target", error));
    }
  });

  onMount(() => {
    const handoff = () => { nativeHandoff = true; };
    window.addEventListener("unfocus-cue-handoff", handoff);
    const timer = window.setInterval(() => { if (!skipped) monotonicNowMs = performance.now(); }, 50);
    return () => {
      window.clearInterval(timer);
      skipController.dispose();
      window.removeEventListener("unfocus-cue-handoff", handoff);
      void visibility.dispose();
    };
  });
</script>

<main class="cue-surface" aria-hidden={platform !== "macos"}>
  {#if nativeVisible}
    {#if platform === "macos"}
      {#if layout}
        <div
          class="notch-cue"
          class:pill={!layout.isNotched}
          class:retracting
          class:skipped
          style={`--notch-width: ${layout.notchWidth}px; --wing-width: ${layout.wingWidth}px; --shoulder-width: ${layout.shoulderWidth}px; --macos-cue-animation-delay: ${animationDelayMs}ms`}
        >
          {#key `${layoutRevision}-${presentation.stage === "heads-up" ? "heads-up" : "final"}`}
            <div class="notch-shape" class:heads-up={presentation.stage === "heads-up"}>
              <span class="notch-ring" aria-hidden="true">
                {#if skipped}
                  <svg class="skip-check" viewBox="0 0 24 24"><path d="m5 12 4 4 10-10"></path></svg>
                {:else}
                <svg viewBox="0 0 24 24">
                  <circle class="ring-track" cx="12" cy="12" r="10"></circle>
                  <circle class="ring-progress" cx="12" cy="12" r="10" pathLength="100" style:stroke-dashoffset={100 * (1 - ringProgress)}></circle>
                </svg>
                {/if}
              </span>
              <span class="camera-gap" aria-hidden="true"></span>
              <span class="notch-timing">
                {#if skipped}
                  <span class="skip-confirmation" role="status">Skipped</span>
                {:else}
                  <span class="notch-seconds" data-type-role="native-countdown">{presentation.stage === "heads-up" ? 1 : view.countdown ?? 1}<span class="notch-unit">{presentation.stage === "heads-up" ? "m" : "s"}</span></span>
                  <button type="button" class="skip-button" aria-label={skipError ? "Retry skip this break" : "Skip this break"} title={skipError ? `Could not skip: ${skipError}` : "Skip this break"} disabled={!skipAvailable} onclick={() => skipController.skip()}>
                    <svg viewBox="0 0 24 24" aria-hidden="true"><path d="m5 4 10 8-10 8zM19 4v16"></path></svg>
                  </button>
                {/if}
              </span>
            </div>
          {/key}
        </div>
      {/if}
    {:else}
      <div
        class="cue-card"
        class:imminent={presentation.stage === "horizon" || presentation.stage === "handoff"}
      >
        {#key presentation.stage}
          <div class="cue-content">
            <span class="cue-symbol" aria-hidden="true">
              <svg viewBox="0 0 24 24">
                <path d="M3.25 12s3.1-5.25 8.75-5.25S20.75 12 20.75 12 17.65 17.25 12 17.25 3.25 12 3.25 12Z"></path>
                <circle cx="12" cy="12" r="2.4"></circle>
              </svg>
            </span>
            {#if presentation.stage === "heads-up" || presentation.stage === "quiet"}
              <span class="cue-copy">
                <strong class="cue-title">{view.title}</strong>
                <span class="cue-support">{view.support}</span>
              </span>
            {:else if presentation.stage === "horizon"}
              <span class="cue-copy">
                <strong class="cue-title">{view.title}</strong>
                <span class="cue-support">{view.support}</span>
              </span>
              <span class="cue-count">
                <span>in</span>
                <strong data-type-role="mono">{view.countdown}</strong>
              </span>
            {:else if presentation.stage === "handoff"}
              <span class="cue-copy">
                <strong class="cue-title look-away" data-type-role="display">{view.title}</strong>
                <span class="cue-support">{view.support}</span>
              </span>
            {/if}
          </div>
        {/key}
      </div>
    {/if}
  {/if}
</main>

<style>
  .cue-surface {
    display: grid;
    width: 100vw;
    height: 100vh;
    place-items: center;
    overflow: hidden;
    color: var(--ink);
    background: transparent;
    pointer-events: none;
  }

  .notch-cue {
    display: grid;
    place-items: center;
    width: 100%;
    height: 100%;
    color: var(--ink);
    font-family: var(--sans);
    pointer-events: none;
  }

  .notch-shape {
    position: relative;
    width: calc(var(--notch-width) + 2 * var(--wing-width));
    height: 100%;
    border-radius: 0 0 12px 12px;
    background: #000000;
    animation: notch-expand 350ms cubic-bezier(0.22, 0.85, 0.3, 1) both;
    animation-delay: var(--macos-cue-animation-delay);
  }

  .notch-shape::before,
  .notch-shape::after {
    content: "";
    position: absolute;
    top: 0;
    width: var(--shoulder-width);
    height: var(--shoulder-width);
    background: radial-gradient(circle at 0 100%, transparent var(--shoulder-width), #000000 calc(var(--shoulder-width) + 0.5px));
  }

  .notch-shape::before { left: calc(-1 * var(--shoulder-width)); }
  .notch-shape::after { right: calc(-1 * var(--shoulder-width)); transform: scaleX(-1); }

  .camera-gap {
    position: absolute;
    inset: 0 auto 0 50%;
    width: var(--notch-width);
    transform: translateX(-50%);
    z-index: 1;
    background: #000000;
    border-radius: 0 0 12px 12px;
  }

  .notch-shape.heads-up {
    animation-name: notch-heads-up;
    animation-duration: 4s;
    animation-timing-function: linear;
  }

  .pill .notch-shape {
    border-radius: 18px;
    overflow: hidden;
  }

  .retracting .notch-shape {
    animation: notch-retract 350ms cubic-bezier(0.4, 0, 1, 1) both;
  }

  .notch-ring,
  .notch-timing {
    position: absolute;
    top: 0;
    width: var(--wing-width);
    height: 100%;
    display: flex;
    justify-content: center;
    align-items: center;
    min-width: 0;
  }

  .notch-ring { left: 0; }

  .notch-ring svg {
    width: 24px;
    height: 24px;
    fill: none;
    stroke-width: 2;
    stroke-linecap: round;
    transform: rotate(-90deg);
  }

  .ring-track { stroke: var(--accent); opacity: 0.2; }
  .ring-progress { stroke: var(--accent); stroke-dasharray: 100; transition: stroke-dashoffset 50ms linear; }

  .notch-timing {
    right: 0;
    padding: 0 3px;
    font-size: 15px;
    font-weight: 500;
    line-height: 1;
    white-space: nowrap;
  }

  .notch-seconds {
    font-family: var(--sans);
    font-variant-numeric: tabular-nums;
    width: 31px;
    flex: 0 0 31px;
    text-align: center;
  }

  .notch-unit { color: var(--ink-2); font-size: 12px; font-weight: 400; }

  .skip-button {
    display: grid;
    place-items: center;
    width: 27px;
    height: 30px;
    flex: 0 0 27px;
    padding: 0;
    border: 0;
    border-radius: var(--r-button);
    color: var(--ink);
    background: transparent;
    pointer-events: auto;
    cursor: pointer;
    transition: color 120ms ease, background 120ms ease, transform 90ms ease;
  }
  .skip-button:disabled { pointer-events: none; cursor: default; }
  .skip-button:hover:not(:disabled) { color: var(--accent); background: color-mix(in srgb, var(--accent) 16%, #000); }
  .skip-button:active:not(:disabled) { transform: scale(0.92); background: color-mix(in srgb, var(--accent) 28%, #000); }
  .skip-button:focus-visible { outline: 1px solid var(--accent); outline-offset: -2px; }
  .skip-button svg { width: 14px; height: 14px; fill: none; stroke: currentColor; stroke-width: 1.7; stroke-linecap: round; stroke-linejoin: round; }
  .notch-ring .skip-check { width: 17px; height: 17px; transform: none; stroke: var(--accent); }
  .skip-confirmation { color: var(--accent); font-size: 12px; font-weight: 400; }
  .skipped:not(.retracting) .notch-shape { animation: none; }


  @keyframes notch-expand {
    from { width: var(--notch-width); }
    to { width: calc(var(--notch-width) + 2 * var(--wing-width)); }
  }

  @keyframes notch-heads-up {
    0%, 100% { width: var(--notch-width); }
    0% { animation-timing-function: cubic-bezier(0.22, 0.85, 0.3, 1); }
    8.75%, 91.25% { width: calc(var(--notch-width) + 2 * var(--wing-width)); }
    91.25% { animation-timing-function: cubic-bezier(0.4, 0, 1, 1); }
  }

  @keyframes notch-heads-up-fade {
    0%, 100% { opacity: 0; }
    3%, 97% { opacity: 1; }
  }

  @keyframes notch-retract {
    from { width: calc(var(--notch-width) + 2 * var(--wing-width)); }
    to { width: var(--notch-width); }
  }

  .cue-card {
    display: flex;
    width: min(336px, calc(100vw - 40px));
    height: min(68px, calc(100vh - 40px));
    min-height: 52px;
    align-items: center;
    border: 1px solid rgba(255, 255, 255, 0.13);
    border-radius: 24px;
    padding: 0 var(--s4);
    color: var(--ink);
    background: rgba(10, 15, 12, 0.97);
    box-shadow:
      0 12px 28px rgba(0, 0, 0, 0.24),
      inset 0 1px rgba(255, 255, 255, 0.06);
    font-family: var(--sans);
    animation: cue-card-arrive 160ms ease both;
    transition:
      border-color 180ms ease,
      background-color 180ms ease;
  }

  .cue-card.imminent {
    border-color: rgba(241, 211, 154, 0.28);
    background: rgba(18, 18, 15, 0.975);
  }

  .cue-content {
    display: flex;
    width: 100%;
    min-width: 0;
    align-items: center;
    gap: var(--s3);
    animation: cue-content-arrive 140ms ease-out both;
  }

  .cue-symbol {
    display: grid;
    width: 30px;
    height: 30px;
    flex: 0 0 auto;
    place-items: center;
    border-radius: var(--r-button);
    color: #b8c5bd;
    background: rgba(255, 255, 255, 0.065);
    transition:
      color 180ms ease,
      background-color 180ms ease;
  }

  .cue-symbol svg {
    width: 18px;
    height: 18px;
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-width: 1.55;
  }

  .cue-card.imminent .cue-symbol {
    color: #f1d39a;
    background: rgba(241, 211, 154, 0.1);
  }

  .cue-copy {
    display: flex;
    min-width: 0;
    flex: 1;
    flex-direction: column;
    gap: 2px;
    line-height: 1.05;
  }

  .cue-title {
    overflow: hidden;
    color: var(--ink);
    font-size: 0.95rem;
    font-weight: 600;
    letter-spacing: 0.005em;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cue-support {
    overflow: hidden;
    color: var(--ink-2);
    font-size: 0.78rem;
    font-weight: 500;
    text-overflow: ellipsis;
    white-space: nowrap;
  }

  .cue-count {
    display: flex;
    margin-left: auto;
    align-items: baseline;
    gap: 6px;
    color: var(--ink-2);
    font-size: 0.75rem;
    font-weight: 600;
  }

  .cue-count strong[data-type-role="mono"] {
    min-width: 2ch;
    color: var(--warn);
    font-size: 1.85rem;
    font-weight: 600;
    font-variant-numeric: tabular-nums;
    line-height: 1;
    text-align: right;
  }

  .look-away {
    color: #f1d39a;
    font-family: var(--display);
    font-size: 1.25rem;
    font-weight: 450;
    line-height: 1;
  }

  @keyframes cue-card-arrive {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  @keyframes cue-content-arrive {
    from {
      opacity: 0;
    }
    to {
      opacity: 1;
    }
  }

  @media (prefers-contrast: more) {
    .cue-card,
    .cue-card.imminent {
      border-color: #ffffff;
      color: #ffffff;
      background: #06100a;
      box-shadow: none;
    }

    .cue-title,
    .cue-support,
    .cue-count,
    .cue-count strong[data-type-role="mono"],
    .look-away {
      color: #ffffff;
    }

    .cue-symbol,
    .cue-card.imminent .cue-symbol {
      color: #ffffff;
      background: transparent;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .ring-progress { transition: none; }
    .skip-button:active:not(:disabled) { transform: none; }
    .notch-shape,
    .notch-shape.heads-up {
      animation: cue-content-arrive 120ms linear both;
    }

    .notch-shape.heads-up {
      animation: notch-heads-up-fade 4s linear both;
      animation-delay: var(--macos-cue-animation-delay);
    }

    .retracting .notch-shape {
      animation: cue-content-arrive 120ms linear reverse both;
    }

    .cue-card {
      animation: cue-card-arrive 120ms linear both;
      transition:
        border-color 120ms linear,
        background-color 120ms linear;
    }

    .cue-content {
      animation: cue-content-arrive 120ms linear both;
    }
  }
</style>
