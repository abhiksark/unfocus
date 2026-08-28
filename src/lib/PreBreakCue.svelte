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
  let progressStyle = $derived(
    `--cue-progress: ${presentation.countdownProgress * 100}%`
  );

  onMount(() => {
    const timer = window.setInterval(() => (monotonicNowMs = performance.now()), 100);
    return () => window.clearInterval(timer);
  });
</script>

<main class="cue-surface" aria-hidden="true">
  <div
    class="cue-pill"
    class:countdown={presentation.stage === "countdown"}
    class:due={presentation.stage === "due"}
    style={progressStyle}
  >
    {#if presentation.stage === "compact"}
      <span class="cue-mark"><span></span></span>
      <span class="cue-copy">
        <span class="cue-kicker">Eye break</span>
        <strong class="cue-message">in 1 minute</strong>
      </span>
    {:else if presentation.stage === "countdown"}
      <span class="cue-mark warning"><span></span></span>
      <span class="cue-copy">
        <span class="cue-kicker">Eye break</span>
        <strong class="cue-message">Look away soon</strong>
      </span>
      <span class="cue-count">
        <strong data-type-role="mono">{presentation.secondsLeft}</strong>
        <span>sec</span>
      </span>
      <span class="cue-progress"><span></span></span>
    {:else}
      <span class="cue-mark due-mark"><span></span></span>
      <span class="cue-copy">
        <span class="cue-kicker">Time for a break</span>
        <strong class="look-away" data-type-role="display">Look away</strong>
      </span>
      <span class="cue-progress"><span></span></span>
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
    position: relative;
    display: flex;
    width: min(254px, 100vw);
    height: 48px;
    align-items: center;
    gap: var(--s3);
    border: 1px solid rgba(170, 212, 184, 0.32);
    border-radius: var(--r-button);
    padding: 0 var(--s4);
    color: var(--ink);
    background: linear-gradient(135deg, rgba(14, 34, 24, 0.98), rgba(6, 18, 13, 0.97));
    box-shadow:
      0 14px 34px rgba(0, 0, 0, 0.34),
      inset 0 1px rgba(232, 248, 237, 0.09),
      inset 0 -1px rgba(0, 0, 0, 0.28);
    font-family: var(--sans);
    opacity: 0;
    animation: cue-arrive 180ms ease-out forwards;
    transition:
      width 360ms cubic-bezier(0.22, 1, 0.36, 1),
      height 360ms cubic-bezier(0.22, 1, 0.36, 1),
      border-color 360ms ease,
      background 360ms ease;
  }

  .cue-pill::before {
    position: absolute;
    inset: 1px 14% auto;
    height: 1px;
    background: linear-gradient(90deg, transparent, rgba(228, 246, 234, 0.2), transparent);
    content: "";
  }

  .cue-pill.countdown {
    width: min(328px, 100vw);
    height: 72px;
    border-color: rgba(217, 181, 115, 0.48);
    background: linear-gradient(135deg, rgba(21, 37, 27, 0.99), rgba(8, 20, 14, 0.98));
    box-shadow:
      0 16px 38px rgba(0, 0, 0, 0.38),
      inset 0 1px rgba(250, 235, 203, 0.1),
      inset 0 -1px rgba(0, 0, 0, 0.3);
  }

  .cue-pill.due {
    width: min(328px, 100vw);
    height: 72px;
    border-color: rgba(217, 181, 115, 0.62);
    background: linear-gradient(135deg, rgba(28, 41, 27, 0.99), rgba(10, 22, 14, 0.99));
    box-shadow:
      0 16px 40px rgba(0, 0, 0, 0.4),
      0 0 26px rgba(217, 181, 115, 0.09),
      inset 0 1px rgba(250, 235, 203, 0.12);
  }

  .cue-mark {
    position: relative;
    display: grid;
    width: 26px;
    height: 26px;
    flex: 0 0 auto;
    place-items: center;
    border: 1px solid rgba(127, 215, 154, 0.48);
    border-radius: 50%;
    background: rgba(127, 215, 154, 0.08);
    box-shadow: inset 0 0 12px rgba(127, 215, 154, 0.08);
    transition:
      border-color 300ms ease,
      background 300ms ease;
  }

  .cue-mark span {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
    box-shadow: 0 0 10px rgba(127, 215, 154, 0.42);
  }

  .cue-mark.warning,
  .cue-mark.due-mark {
    border-color: rgba(217, 181, 115, 0.58);
    background: rgba(217, 181, 115, 0.09);
  }

  .cue-mark.warning span,
  .cue-mark.due-mark span {
    background: var(--warn);
    box-shadow: 0 0 11px rgba(217, 181, 115, 0.46);
  }

  .cue-copy {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 1px;
    line-height: 1.05;
  }

  .cue-kicker {
    color: var(--ink-2);
    font-size: 0.75rem;
    font-weight: 600;
    letter-spacing: 0.08em;
    text-transform: uppercase;
  }

  .cue-message {
    color: var(--ink);
    font-size: 0.88rem;
    font-weight: 600;
    letter-spacing: 0.01em;
  }

  .cue-count {
    display: flex;
    margin-left: auto;
    align-items: baseline;
    gap: var(--s1);
    color: var(--ink-2);
    font-size: 0.75rem;
    font-weight: 600;
  }

  .cue-count strong[data-type-role="mono"] {
    min-width: 2ch;
    color: var(--warn);
    font-size: 1.75rem;
    font-weight: 600;
    line-height: 1;
    text-align: center;
    animation: cue-number-arrive 180ms ease-out;
  }

  .look-away {
    color: #f1d39a;
    font-family: var(--display);
    font-size: 1.25rem;
    font-weight: 450;
    line-height: 1;
  }

  .cue-progress {
    position: absolute;
    right: var(--s4);
    bottom: var(--s2);
    left: var(--s4);
    height: 2px;
    overflow: hidden;
    border-radius: var(--r-button);
    background: rgba(217, 181, 115, 0.14);
  }

  .cue-progress span {
    display: block;
    width: var(--cue-progress);
    height: 100%;
    border-radius: inherit;
    background: linear-gradient(90deg, rgba(217, 181, 115, 0.52), var(--warn));
    box-shadow: 0 0 8px rgba(217, 181, 115, 0.28);
    transition: width 90ms linear;
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

    .cue-pill::before {
      display: none;
    }

    .cue-kicker,
    .cue-message,
    .cue-count,
    .cue-count strong[data-type-role="mono"],
    .look-away {
      color: #ffffff;
    }

    .cue-mark,
    .cue-mark.warning,
    .cue-mark.due-mark {
      border-color: #ffffff;
      background: transparent;
    }

    .cue-mark span,
    .cue-mark.warning span,
    .cue-mark.due-mark span,
    .cue-progress span {
      background: #ffffff;
      box-shadow: none;
    }

    .cue-progress {
      background: rgba(255, 255, 255, 0.32);
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

    .cue-mark,
    .cue-progress span {
      transition: none;
    }

    .cue-progress {
      display: none;
    }

    .cue-count strong[data-type-role="mono"] {
      animation: none;
      filter: none;
    }
  }
</style>
