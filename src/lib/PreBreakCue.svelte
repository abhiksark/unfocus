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
  <div class="cue-pill" class:countdown={presentation.stage === "countdown"} class:due={presentation.stage === "due"}>
    {#if presentation.stage === "compact"}
      <span class="cue-dot"></span>
      <span>Eye break in 1 minute</span>
    {:else if presentation.stage === "countdown"}
      <span class="cue-label">Eye break in</span>
      <strong data-type-role="mono">{presentation.secondsLeft}</strong>
      <span class="cue-unit">seconds</span>
    {:else}
      <span class="cue-dot"></span>
      <strong class="look-away">Look away</strong>
    {/if}
  </div>
</main>

<style>
  .cue-surface {
    display: flex;
    width: 100vw;
    height: 100vh;
    align-items: flex-start;
    justify-content: center;
    overflow: hidden;
    color: #f4f7f2;
    background: transparent;
    pointer-events: none;
  }

  .cue-pill {
    display: flex;
    width: min(244px, 100vw);
    height: 44px;
    align-items: center;
    justify-content: center;
    gap: 10px;
    border: 1px solid rgba(210, 232, 218, 0.28);
    border-radius: 999px;
    padding: 0 18px;
    color: #f4f7f2;
    background: rgba(8, 19, 14, 0.94);
    box-shadow: 0 12px 32px rgba(0, 0, 0, 0.3);
    font-family: var(--sans);
    font-size: 0.82rem;
    font-weight: 600;
    letter-spacing: 0.01em;
    opacity: 0;
    animation: cue-arrive 180ms ease-out forwards;
    transition:
      width 360ms cubic-bezier(0.22, 1, 0.36, 1),
      height 360ms cubic-bezier(0.22, 1, 0.36, 1),
      border-color 360ms ease,
      background 360ms ease;
  }

  .cue-pill.countdown {
    width: min(328px, 100vw);
    height: 72px;
    border-color: rgba(222, 193, 126, 0.46);
    background: rgba(12, 24, 17, 0.97);
  }

  .cue-pill.due {
    width: min(328px, 100vw);
    height: 72px;
    border-color: rgba(222, 193, 126, 0.58);
    background: rgba(15, 27, 18, 0.98);
  }

  .cue-dot {
    width: 7px;
    height: 7px;
    flex: 0 0 auto;
    border-radius: 50%;
    background: #83d39e;
  }

  .cue-label,
  .cue-unit {
    color: rgba(235, 243, 237, 0.82);
  }

  .cue-pill strong[data-type-role="mono"] {
    min-width: 2ch;
    color: #f2d391;
    font-size: 1.55rem;
    font-weight: 600;
    text-align: center;
    animation: cue-number-arrive 180ms ease-out;
  }

  .look-away {
    color: #f3d59a;
    font-family: var(--display);
    font-size: 1.3rem;
    font-weight: 500;
  }

  @keyframes cue-arrive {
    to {
      opacity: 1;
    }
  }

  @keyframes cue-number-arrive {
    from {
      opacity: 0.35;
      filter: blur(3px);
    }
    to {
      opacity: 1;
      filter: blur(0);
    }
  }

  @media (prefers-contrast: more) {
    .cue-pill {
      border-color: #ffffff;
      color: #ffffff;
      background: #06100a;
    }

    .cue-label,
    .cue-unit,
    .cue-pill strong[data-type-role="mono"],
    .look-away {
      color: #ffffff;
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .cue-pill,
    .cue-pill.countdown,
    .cue-pill.due {
      width: min(328px, 100vw);
      height: 72px;
      animation: cue-arrive 120ms linear forwards;
      transition: none;
    }

    .cue-pill strong[data-type-role="mono"] {
      animation: none;
      filter: none;
    }
  }
</style>
