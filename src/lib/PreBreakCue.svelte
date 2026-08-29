<!-- src/lib/PreBreakCue.svelte -->

<script lang="ts">
  import { preBreakCuePresentationFromRemaining } from "$lib/pre-break-cue";
  import { onMount, untrack } from "svelte";

  type Props = { deadlineMs: number };

  let { deadlineMs }: Props = $props();
  const initialRemainingMs = untrack(() => Math.max(0, deadlineMs - Date.now()));
  const monotonicStartedAtMs = performance.now();
  let monotonicNowMs = $state(monotonicStartedAtMs);
  let remainingMs = $derived(
    Math.max(0, initialRemainingMs - (monotonicNowMs - monotonicStartedAtMs))
  );
  let presentation = $derived(preBreakCuePresentationFromRemaining(remainingMs));

  onMount(() => {
    const timer = window.setInterval(() => (monotonicNowMs = performance.now()), 100);
    return () => window.clearInterval(timer);
  });
</script>

<main class="cue-surface" aria-hidden="true">
  <div
    class="cue-card"
    class:visible={presentation.visible}
    class:imminent={presentation.stage === "countdown" || presentation.stage === "handoff"}
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
            <strong class="cue-title">Eye break in 1 minute</strong>
            <span class="cue-support">Finish your thought.</span>
          </span>
        {:else if presentation.stage === "countdown"}
          <span class="cue-copy">
            <strong class="cue-title">Eye break</strong>
            <span class="cue-support">Finish your thought.</span>
          </span>
          <span class="cue-count">
            <span>in</span>
            <strong data-type-role="mono">{presentation.secondsLeft}</strong>
          </span>
        {:else if presentation.stage === "handoff"}
          <span class="cue-copy">
            <strong class="cue-title look-away" data-type-role="display">Look away</strong>
            <span class="cue-support">Rest your focus beyond the screen.</span>
          </span>
        {/if}
      </div>
    {/key}
  </div>
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

  .cue-card {
    display: flex;
    width: min(352px, calc(100vw - 40px));
    height: min(72px, calc(100vh - 40px));
    min-height: 52px;
    align-items: center;
    border: 1px solid rgba(255, 255, 255, 0.13);
    border-radius: var(--r-button);
    padding: 0 18px;
    color: var(--ink);
    background: rgba(10, 15, 12, 0.97);
    box-shadow:
      0 12px 28px rgba(0, 0, 0, 0.24),
      inset 0 1px rgba(255, 255, 255, 0.06);
    font-family: var(--sans);
    opacity: 0;
    transition:
      opacity 160ms ease,
      border-color 180ms ease,
      background-color 180ms ease;
  }

  .cue-card.visible {
    opacity: 1;
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
    width: 32px;
    height: 32px;
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
    width: 19px;
    height: 19px;
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
    gap: 3px;
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
    .cue-card,
    .cue-card.visible {
      transform: none;
      transition:
        opacity 120ms linear,
        border-color 120ms linear,
        background-color 120ms linear;
    }

    .cue-content {
      animation: cue-content-arrive 120ms linear both;
    }
  }
</style>
