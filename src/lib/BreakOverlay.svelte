<script lang="ts">
  import {
    anchorFromRemaining,
    createOverlayClock,
    formatCountdown,
    presentationOffset,
    remainingAt,
    type OverlayClockAnchor
  } from "$lib/overlay-clock";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";
  import scene from "$lib/scene.svg?raw";

  type Props = {
    runId: number;
    monitorIndex: number;
    monitorCount: number;
    durationSeconds: number;
    deadlineMs: number;
    onClose: () => Promise<void>;
  };

  type OverlayTick = { runId: number; remainingMs: number };
  type OverlayRunEvent = { runId: number };

  let { runId, monitorIndex, monitorCount, durationSeconds, deadlineMs, onClose }: Props = $props();

  const messages = [
    "Find the farthest point you can see.",
    "Let your eyes soften at the edges.",
    "Notice the room beyond the screen."
  ];
  const initialMonotonicMs = performance.now();
  const initialWallMs = Date.now();
  let fallbackClock = $derived(
    createOverlayClock(durationSeconds, deadlineMs, initialWallMs, initialMonotonicMs)
  );

  let presentationOffsetMs = $derived(presentationOffset(fallbackClock));
  let presentationStyle = $derived(`--presentation-offset: ${presentationOffsetMs}ms`);

  let monotonicNowMs = $state(initialMonotonicMs);
  let wallNowMs = $state(Date.now());
  let nativeClock = $state<OverlayClockAnchor | null>(null);
  let dismissing = $state(false);
  let actionPending = $state(false);
  let actionError = $state<string | null>(null);
  let syncError = $state<string | null>(null);
  let recoveryTimer: number | undefined;

  let durationMs = $derived(Math.max(1_000, durationSeconds * 1_000));
  let remainingMs = $derived(remainingAt(nativeClock ?? fallbackClock, monotonicNowMs));
  let elapsedMs = $derived(Math.max(0, durationMs - remainingMs));
  let secondsLeft = $derived(Math.ceil(remainingMs / 1_000));
  let remainingFraction = $derived(Math.min(1, Math.max(0, remainingMs / durationMs)));
  let ringOffset = $derived(100 - remainingFraction * 100);
  let complete = $derived(remainingMs === 0);
  let returningThresholdSeconds = $derived(
    Math.min(5, Math.max(2, Math.round(durationSeconds * 0.15)))
  );
  let finalSeconds = $derived(!complete && secondsLeft <= returningThresholdSeconds);
  let messageIntervalMs = $derived(Math.max(4_000, Math.min(12_000, durationMs / 3)));
  let messageIndex = $derived(Math.floor(elapsedMs / messageIntervalMs) % messages.length);
  let currentMessage = $derived(
    complete ? "Come back slowly." : messages[messageIndex]
  );
  let countdown = $derived(formatCountdown(secondsLeft));
  let clock = $derived(
    new Intl.DateTimeFormat([], { hour: "2-digit", minute: "2-digit" }).format(wallNowMs)
  );
  let announcement = $derived(
    complete
      ? "Break complete."
      : secondsLeft === 5
        ? "Five seconds remain in this break."
        : ""
  );

  function errorMessage(value: unknown): string {
    return value instanceof Error ? value.message : String(value);
  }

  function beginDismissing() {
    dismissing = true;
    if (recoveryTimer !== undefined) window.clearTimeout(recoveryTimer);
    recoveryTimer = window.setTimeout(() => {
      dismissing = false;
      actionError ??= "The native window did not close. Press Escape to try again.";
    }, 2_500);
  }

  async function closePreview() {
    if (actionPending) return;

    actionPending = true;
    actionError = null;
    beginDismissing();
    try {
      await onClose();
    } catch (value) {
      dismissing = false;
      if (recoveryTimer !== undefined) window.clearTimeout(recoveryTimer);
      actionError = errorMessage(value);
    } finally {
      actionPending = false;
    }
  }

  function handleKeydown(event: KeyboardEvent) {
    if (event.key !== "Escape") return;
    event.preventDefault();
    void closePreview();
  }

  onMount(() => {
    let disposed = false;
    const unlisteners: Array<() => void> = [];
    let timer: number | undefined;

    const tick = () => {
      monotonicNowMs = performance.now();
      wallNowMs = Date.now();
      timer = window.setTimeout(tick, 250);
    };

    tick();

    // Tauri delivers an untargeted listener every emit regardless of the
    // emitting window, so scope the subscription to this window's label.
    const ownLabel = getCurrentWindow().label;

    function register<T>(eventName: string, handler: (payload: T) => void) {
      void listen<T>(eventName, (event) => handler(event.payload), { target: ownLabel })
        .then((stopListening) => {
          if (disposed) stopListening();
          else unlisteners.push(stopListening);
        })
        .catch((value: unknown) => {
          if (!disposed) syncError = `Native ${eventName} updates unavailable: ${errorMessage(value)}`;
        });
    }

    register<OverlayTick>("unfocus-overlay-tick", (payload) => {
      if (payload.runId !== runId || !Number.isFinite(payload.remainingMs)) return;
      const sampledAt = performance.now();
      nativeClock = anchorFromRemaining(durationMs, payload.remainingMs, sampledAt);
      monotonicNowMs = sampledAt;
    });
    register<OverlayRunEvent>("unfocus-overlay-complete", (payload) => {
      if (payload.runId !== runId) return;
      const sampledAt = performance.now();
      nativeClock = anchorFromRemaining(durationMs, 0, sampledAt);
      monotonicNowMs = sampledAt;
    });
    register<OverlayRunEvent>("unfocus-overlay-closing", (payload) => {
      if (payload.runId !== runId) return;
      beginDismissing();
    });

    return () => {
      disposed = true;
      if (timer !== undefined) window.clearTimeout(timer);
      if (recoveryTimer !== undefined) window.clearTimeout(recoveryTimer);
      for (const stopListening of unlisteners) stopListening();
    };
  });
</script>

<svelte:window onkeydown={handleKeydown} />

<main
  class="break-overlay"
  class:closing={dismissing}
  class:complete
  class:final-seconds={finalSeconds}
  aria-label="Eye break"
  style={presentationStyle}
>
  <div class="atmosphere" aria-hidden="true">
    {@html scene}
  </div>

  <header class="overlay-header" aria-hidden="true">
    <div class="wordmark"><span></span>Unfocus</div>
    <time>{clock}</time>
  </header>

  <section class="break-content" aria-labelledby="break-message">
    <p class="eyebrow">A moment for your eyes</p>
    {#key currentMessage}
      <h1 id="break-message" class="message">{currentMessage}</h1>
    {/key}
    <p class="guidance">
      {complete
        ? "Notice how your eyes feel before returning."
        : "Rest your focus on something beyond the screen."}
    </p>

    <div
      class="timer"
      role="timer"
      aria-live="off"
      aria-label={complete ? "Break complete" : `${secondsLeft} seconds remaining`}
    >
      <svg viewBox="0 0 40 40" aria-hidden="true">
        <circle class="ring-track" cx="20" cy="20" r="15.9" pathLength="100"></circle>
        <circle
          class="ring-progress"
          cx="20"
          cy="20"
          r="15.9"
          pathLength="100"
          style:stroke-dashoffset={ringOffset}
        ></circle>
      </svg>
      {#key countdown}
        <span class="timer-digits">{countdown}</span>
      {/key}
    </div>
  </section>

  <footer class="overlay-controls">
    <button class="skip-button" type="button" onclick={closePreview} disabled={actionPending}>
      <svg viewBox="0 0 20 20" aria-hidden="true">
        <path d="m4.5 5 5 5-5 5M10.5 5l5 5-5 5"></path>
      </svg>
      End break
    </button>
    <p class="shortcut">Press <kbd>Esc</kbd> to close</p>
    <p class="display-label">Display {monitorIndex + 1} of {monitorCount}</p>
    {#if actionError}
      <p class="action-error" role="alert">Could not close the break: {actionError}</p>
    {/if}
    {#if syncError}
      <p class="sync-error" role="status">{syncError}</p>
    {/if}
  </footer>

  {#if monitorIndex === 0}
    <p class="screen-reader-announcement" role="status">
      Eye break started. Rest your focus on something beyond the screen.
    </p>
    <p class="screen-reader-announcement" aria-live="polite">{announcement}</p>
  {/if}
</main>

<style>
  .break-overlay {
    position: fixed;
    inset: 0;
    display: grid;
    height: 100vh;
    grid-template-rows: auto minmax(0, 1fr) auto;
    gap: clamp(14px, 3vh, 30px);
    isolation: isolate;
    overflow: auto;
    min-width: 320px;
    min-height: 100vh;
    padding: max(24px, 4.8vh) max(28px, 5vw) max(22px, 4.5vh);
    color: #f0f7f3;
    background: #060f0d;
  }

  .break-overlay.closing {
    pointer-events: none;
    animation: overlay-dismiss 460ms cubic-bezier(0.4, 0, 1, 1) forwards;
  }

  .atmosphere {
    position: absolute;
    z-index: -1;
    inset: 0;
    overflow: hidden;
    opacity: 0;
    animation: atmosphere-reveal 1.8s cubic-bezier(0.22, 1, 0.36, 1) forwards;
    animation-delay: calc(0ms - var(--presentation-offset));
  }

  /* The scene is one baked SVG (scripts/gen-scene.js). Full-viewport gradients
     stay static; the only continuous motion is stepped ridge drift and mist
     breathing — transform/opacity on a few elements, updated every 2-3 s. */

  .atmosphere :global(.scene) {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  .atmosphere :global(.ridge-1) {
    animation: ridge-drift-far 48s steps(16, end) infinite alternate;
    animation-delay: calc(0ms - var(--presentation-offset));
  }

  .atmosphere :global(.ridge-2) {
    animation: ridge-drift-mid 36s steps(12, end) infinite alternate-reverse;
    animation-delay: calc(0ms - var(--presentation-offset));
  }

  .atmosphere :global(.ridge-3) {
    animation: ridge-drift-near 28s steps(10, end) infinite alternate;
    animation-delay: calc(0ms - var(--presentation-offset));
  }

  .atmosphere :global(.mist) {
    animation: mist-breathe 21s steps(10, end) infinite alternate;
    animation-delay: calc(0ms - var(--presentation-offset));
  }

  /* Dawn rises behind the summit as the break ends: cool haze and mist recede,
     amber gathers at the ridge line. State-driven opacity, not animation. */
  .atmosphere :global(.dawn),
  .atmosphere :global(.mist-amber) {
    opacity: 0;
    transition: opacity 2.4s ease;
  }

  .atmosphere :global(.haze) {
    transition: opacity 2.4s ease;
  }

  .final-seconds .atmosphere :global(.dawn),
  .complete .atmosphere :global(.dawn) {
    opacity: 1;
  }

  .final-seconds .atmosphere :global(.mist-amber),
  .complete .atmosphere :global(.mist-amber) {
    opacity: 0.16;
  }

  .final-seconds .atmosphere :global(.haze),
  .complete .atmosphere :global(.haze) {
    opacity: 0.3;
  }

  .final-seconds .atmosphere :global(.mist),
  .complete .atmosphere :global(.mist) {
    animation: none;
    opacity: 0;
  }

  .overlay-header {
    position: relative;
    display: flex;
    align-items: center;
    justify-content: space-between;
    opacity: 0;
    animation: chrome-reveal 1s cubic-bezier(0.22, 1, 0.36, 1) forwards;
    animation-delay: calc(700ms - var(--presentation-offset));
  }

  .wordmark {
    display: flex;
    align-items: center;
    gap: 10px;
    color: rgba(235, 246, 239, 0.78);
    font-size: 0.72rem;
    font-weight: 650;
    letter-spacing: 0.15em;
    text-transform: uppercase;
  }

  .wordmark span {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #7dcf9b;
  }

  .overlay-header time {
    color: rgba(235, 246, 239, 0.78);
    font-size: 0.82rem;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.04em;
  }

  .break-content {
    position: relative;
    display: flex;
    width: min(88vw, 960px);
    max-height: 100%;
    min-height: 0;
    flex-direction: column;
    align-items: center;
    align-self: center;
    justify-self: center;
    overflow: auto;
    padding: 8px 0;
    text-align: center;
    opacity: 0;
    transform: translate3d(0, 18px, 0) scale(0.985);
    animation: content-reveal 1.45s cubic-bezier(0.22, 1, 0.36, 1) forwards;
    animation-delay: calc(180ms - var(--presentation-offset));
  }

  .eyebrow {
    margin: 0 0 26px;
    color: rgba(190, 232, 205, 0.82);
    font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
    font-size: 0.7rem;
    font-weight: 500;
    letter-spacing: 0.32em;
    text-transform: uppercase;
  }

  .message {
    max-width: 960px;
    margin: 0;
    color: #e9f3ec;
    font-family: "Fraunces", Georgia, serif;
    font-size: clamp(2.4rem, 4.6vw, 4.2rem);
    font-weight: 380;
    letter-spacing: 0.002em;
    line-height: 1.14;
    text-wrap: balance;
    text-shadow: 0 10px 40px rgba(0, 0, 0, 0.25);
    animation: message-arrive 650ms cubic-bezier(0.22, 1, 0.36, 1);
    animation-delay: calc(0ms - var(--presentation-offset));
    animation-fill-mode: both;
  }

  .guidance {
    max-width: 520px;
    margin: 20px 0 0;
    color: rgba(226, 239, 230, 0.76);
    font-size: clamp(0.82rem, 1.05vw, 1rem);
    line-height: 1.55;
  }

  .timer {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    margin-top: clamp(26px, 4.5vh, 46px);
  }

  .timer svg {
    width: 54px;
    height: 54px;
    overflow: visible;
    transform: rotate(-90deg);
  }

  .timer circle {
    fill: none;
    stroke-width: 1.25;
  }

  .ring-track {
    stroke: rgba(215, 236, 220, 0.14);
  }

  .ring-progress {
    stroke: #9fd8b4;
    stroke-dasharray: 100;
    stroke-linecap: round;
    transition:
      stroke 600ms ease,
      stroke-dashoffset 1s linear;
  }

  .final-seconds .ring-progress {
    stroke: #d9bb7d;
  }

  .timer-digits {
    color: rgba(240, 248, 243, 0.82);
    font-family: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.08em;
    animation: digits-arrive 210ms ease-out;
    animation-delay: calc(0ms - var(--presentation-offset));
    animation-fill-mode: both;
  }

  .overlay-controls {
    position: relative;
    display: flex;
    flex-direction: column;
    align-items: center;
    opacity: 0;
    animation: chrome-reveal 1s cubic-bezier(0.22, 1, 0.36, 1) forwards;
    animation-delay: calc(850ms - var(--presentation-offset));
  }

  .skip-button {
    display: inline-flex;
    min-width: 142px;
    min-height: 46px;
    align-items: center;
    justify-content: center;
    gap: 8px;
    border: 1px solid rgba(209, 234, 219, 0.16);
    border-radius: 999px;
    padding: 11px 20px;
    color: rgba(237, 247, 241, 0.82);
    background: rgba(222, 241, 230, 0.075);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.06),
      0 12px 34px rgba(0, 0, 0, 0.16);
    font: inherit;
    font-size: 0.78rem;
    font-weight: 620;
    cursor: pointer;
    transition:
      color 180ms ease,
      border-color 180ms ease,
      background 180ms ease,
      transform 180ms ease;
  }

  .skip-button svg {
    width: 15px;
    height: 15px;
    fill: none;
    stroke: currentColor;
    stroke-linecap: round;
    stroke-linejoin: round;
    stroke-width: 1.55;
  }

  .skip-button:hover {
    border-color: rgba(209, 234, 219, 0.28);
    color: #ffffff;
    background: rgba(222, 241, 230, 0.12);
    transform: translateY(-1px);
  }

  .skip-button:focus-visible {
    outline: 2px solid #a8e9bd;
    outline-offset: 4px;
  }

  .skip-button:disabled {
    cursor: default;
    opacity: 0.45;
    transform: none;
  }

  .shortcut,
  .display-label,
  .action-error,
  .sync-error {
    margin: 10px 0 0;
    color: rgba(228, 240, 232, 0.74);
    font-size: 0.68rem;
    line-height: 1.35;
    text-align: center;
  }

  kbd {
    display: inline-block;
    min-width: 25px;
    margin: 0 3px;
    border: 1px solid rgba(226, 240, 231, 0.38);
    border-radius: 5px;
    padding: 2px 5px;
    color: #edf7f0;
    background: rgba(255, 255, 255, 0.08);
    font: inherit;
  }

  .display-label {
    margin-top: 7px;
  }

  .action-error,
  .sync-error {
    color: #f1b0a7;
  }

  .screen-reader-announcement {
    position: absolute;
    width: 1px;
    height: 1px;
    margin: -1px;
    overflow: hidden;
    clip: rect(0, 0, 0, 0);
    white-space: nowrap;
  }

  @keyframes atmosphere-reveal {
    from {
      opacity: 0;
      transform: scale(1.035);
    }
    to {
      opacity: 1;
      transform: scale(1);
    }
  }

  @keyframes content-reveal {
    from {
      opacity: 0;
      transform: translate3d(0, 18px, 0) scale(0.985);
    }
    to {
      opacity: 1;
      transform: translate3d(0, 0, 0) scale(1);
    }
  }

  @keyframes chrome-reveal {
    from {
      opacity: 0;
      transform: translateY(8px);
    }
    to {
      opacity: 1;
      transform: translateY(0);
    }
  }

  @keyframes message-arrive {
    from {
      opacity: 0;
      filter: blur(9px);
      transform: translateY(7px);
    }
    to {
      opacity: 1;
      filter: blur(0);
      transform: translateY(0);
    }
  }

  @keyframes digits-arrive {
    from {
      opacity: 0.38;
      filter: blur(3px);
    }
    to {
      opacity: 1;
      filter: blur(0);
    }
  }

  @keyframes ridge-drift-far {
    to {
      transform: translateX(10px);
    }
  }

  @keyframes ridge-drift-mid {
    to {
      transform: translateX(-13px);
    }
  }

  @keyframes ridge-drift-near {
    to {
      transform: translateX(8px);
    }
  }

  @keyframes mist-breathe {
    from {
      opacity: 0.05;
    }
    to {
      opacity: 0.13;
    }
  }

  @keyframes overlay-dismiss {
    to {
      opacity: 0;
    }
  }

  @media (max-height: 760px) {
    .break-overlay {
      gap: 10px;
      padding-top: 18px;
      padding-bottom: 16px;
    }

    .message {
      font-size: clamp(1.8rem, 4.2vw, 3.4rem);
    }

    .timer {
      margin-top: 14px;
    }

    .guidance {
      margin-top: 10px;
    }

    .eyebrow {
      margin-bottom: 12px;
    }
  }

  @media (max-height: 520px) {
    .break-overlay {
      grid-template-rows: minmax(0, 1fr) auto;
    }

    .overlay-header,
    .guidance,
    .shortcut,
    .display-label {
      display: none;
    }

    .timer svg {
      width: 42px;
      height: 42px;
    }

    .skip-button {
      min-height: 40px;
      padding-block: 8px;
    }
  }

  @media (prefers-contrast: more) {
    .message,
    .guidance,
    .timer-digits,
    .eyebrow,
    .wordmark,
    .overlay-header time,
    .shortcut,
    .display-label {
      color: #ffffff;
      text-shadow: 0 2px 16px #000000;
    }

    .skip-button {
      border-color: rgba(255, 255, 255, 0.58);
      background: rgba(0, 0, 0, 0.42);
    }
  }

  @media (prefers-reduced-motion: reduce) {
    .atmosphere,
    .overlay-header,
    .break-content,
    .overlay-controls {
      opacity: 1;
      animation: none;
    }

    .break-content {
      transform: none;
    }

    .atmosphere :global(.ridge-1),
    .atmosphere :global(.ridge-2),
    .atmosphere :global(.ridge-3),
    .atmosphere :global(.mist),
    .message,
    .timer-digits {
      animation: none;
      filter: none;
    }

    .atmosphere :global(.dawn),
    .atmosphere :global(.mist-amber),
    .atmosphere :global(.haze),
    .atmosphere :global(.mist) {
      transition-duration: 120ms;
    }

    .ring-progress,
    .skip-button {
      transition: none;
    }

    .break-overlay.closing {
      animation: overlay-dismiss 120ms linear forwards;
    }
  }
</style>
