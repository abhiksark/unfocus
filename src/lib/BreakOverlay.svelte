<script lang="ts">
  import {
    anchorFromRemaining,
    createOverlayClock,
    formatCountdown,
    presentationOffset,
    remainingAt,
    type OverlayClockAnchor
  } from "$lib/overlay-clock";
  import {
    OVERLAY_REGION_LABEL,
    overlayAnnouncement,
    overlayCountdownLabel
  } from "$lib/overlay-a11y";
  import {
    BREAK_SCENE_IMAGE_URL,
    breakScenePeriodForRun,
    breakScenePhase
  } from "$lib/break-scene";
  import { listen } from "@tauri-apps/api/event";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount, untrack } from "svelte";

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
  // The deadline is shared by every monitor in the run. Reconstructing the
  // start keeps all displays on one local-time palette across a clock boundary.
  const scenePeriod = untrack(() =>
    breakScenePeriodForRun(deadlineMs, durationSeconds)
  );
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
  let scenePhase = $derived(breakScenePhase({ complete, finalSeconds }));
  let messageIntervalMs = $derived(Math.max(4_000, Math.min(12_000, durationMs / 3)));
  let messageIndex = $derived(Math.floor(elapsedMs / messageIntervalMs) % messages.length);
  let currentMessage = $derived(
    complete ? "Come back slowly." : messages[messageIndex]
  );
  let countdown = $derived(formatCountdown(secondsLeft));
  let clock = $derived(
    new Intl.DateTimeFormat([], { hour: "2-digit", minute: "2-digit" }).format(wallNowMs)
  );
  let announcement = $derived(overlayAnnouncement({ complete, secondsLeft }));
  let countdownAccessibleLabel = $derived(overlayCountdownLabel(complete, secondsLeft));

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
  class:returning={scenePhase === "returning"}
  aria-label={OVERLAY_REGION_LABEL}
  data-scene-period={scenePeriod}
  data-scene-phase={scenePhase}
  style={presentationStyle}
>
  <div class="atmosphere" aria-hidden="true">
    <img class="break-artwork" src={BREAK_SCENE_IMAGE_URL} alt="" draggable="false" />
    <div class="scene-veil"></div>
    <div class="return-light"></div>
    <div class="copy-veil"></div>
  </div>

  <header class="overlay-header" aria-hidden="true">
    <div class="wordmark"><span></span>Unfocus</div>
    <time data-type-role="mono">{clock}</time>
  </header>

  <section class="break-content" aria-labelledby="break-message">
    <p class="eyebrow" data-type-role="ui">A moment for your eyes</p>
    {#key currentMessage}
      <h1 id="break-message" class="message" data-type-role="reflective-display">
        {currentMessage}
      </h1>
    {/key}
    <p class="guidance" data-type-role="ui">
      {complete
        ? "Notice how your eyes feel before returning."
        : "Rest your focus on something beyond the screen."}
    </p>

    <div
      class="timer"
      role="timer"
      aria-live="off"
      aria-label={countdownAccessibleLabel}
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
        <span class="timer-digits" data-type-role="mono">{countdown}</span>
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
    <p class="display-label" data-type-role="mono">
      Display {monitorIndex + 1} of {monitorCount}
    </p>
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
    --scene-tint: linear-gradient(
      180deg,
      rgba(41, 77, 67, 0.12) 38.2%,
      rgba(2, 9, 7, 0.28)
    );
    --scene-copy-x: 50%;
    --scene-copy-core: rgba(3, 14, 13, 0.62);
    --scene-copy-mid: rgba(3, 14, 13, 0.44);
    color: #f6f3e8;
    background: #07100c;
  }

  .break-overlay[data-scene-period="dawn"] {
    --scene-tint:
      radial-gradient(
        ellipse 61.8% 38.2% at 38.2% 61.8%,
        rgba(166, 190, 183, 0.28),
        transparent 61.8%
      ),
      linear-gradient(180deg, rgba(91, 123, 134, 0.24) 38.2%, rgba(33, 67, 57, 0.34));
    --scene-copy-core: rgba(3, 14, 13, 0.66);
    --scene-copy-mid: rgba(3, 14, 13, 0.47);
  }

  .break-overlay[data-scene-period="dusk"] {
    --scene-tint:
      radial-gradient(
        ellipse 61.8% 38.2% at 38.2% 61.8%,
        rgba(84, 104, 126, 0.16),
        transparent 61.8%
      ),
      linear-gradient(180deg, rgba(29, 37, 73, 0.26) 38.2%, rgba(8, 29, 38, 0.42));
    --scene-copy-core: rgba(3, 14, 13, 0.54);
    --scene-copy-mid: rgba(3, 14, 13, 0.38);
  }

  .break-overlay[data-scene-period="night"] {
    --scene-tint: linear-gradient(
      180deg,
      rgba(14, 25, 54, 0.38) 38.2%,
      rgba(3, 15, 17, 0.52)
    );
    --scene-copy-core: rgba(3, 14, 13, 0.48);
    --scene-copy-mid: rgba(3, 14, 13, 0.34);
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

  /* The 4K delivery derivative and full-viewport veil stay static. The return light is the
     only scene state change, and it fades once around the lower golden point. */
  .break-artwork,
  .scene-veil,
  .return-light,
  .copy-veil {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
    pointer-events: none;
  }

  .break-artwork {
    object-fit: cover;
    object-position: 38.2% 50%;
    transform: translateX(7%) scale(1.14);
  }

  .scene-veil {
    background: var(--scene-tint);
  }

  .return-light {
    --return-light-x: 50%;

    background: radial-gradient(
      ellipse 61.8% 38.2% at var(--return-light-x) 61.8%,
      rgba(232, 184, 101, 0.48),
      rgba(196, 128, 52, 0.16) 38.2%,
      transparent 61.8%
    );
    opacity: 0;
    transition: opacity 2.4s cubic-bezier(0.382, 0, 0.618, 1);
  }

  .returning .return-light {
    opacity: 1;
  }

  .copy-veil {
    background:
      linear-gradient(180deg, rgba(2, 9, 7, 0.62) 0 8%, transparent 38.2%),
      linear-gradient(0deg, rgba(2, 9, 7, 0.66) 0 13%, transparent 38.2%),
      radial-gradient(
        ellipse 61.8% 38.2% at var(--scene-copy-x) 50%,
        var(--scene-copy-core) 0 38.2%,
        var(--scene-copy-mid) 61.8%,
        transparent
      );
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
    font-weight: 600;
    letter-spacing: 0.15em;
    text-shadow: 0 2px 8px rgba(0, 0, 0, 0.55);
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
    text-shadow: 0 2px 8px rgba(0, 0, 0, 0.55);
  }

  .break-content {
    position: relative;
    left: 0;
    display: flex;
    width: min(61.8vw, 960px);
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
    margin: 0 0 21px;
    color: rgba(190, 232, 205, 0.82);
    font-size: 0.7rem;
    font-weight: 500;
    letter-spacing: 0.17em;
    text-transform: uppercase;
  }

  .message {
    max-width: 960px;
    margin: 0;
    color: #f6f3e8;
    font-size: clamp(2.6rem, 4.8vw, 4.8rem);
    font-weight: 450;
    letter-spacing: -0.006em;
    line-height: 1.1;
    text-wrap: balance;
    text-shadow: 0 13px 34px rgba(0, 0, 0, 0.42);
    animation: message-arrive 650ms cubic-bezier(0.22, 1, 0.36, 1);
    animation-delay: calc(0ms - var(--presentation-offset));
    animation-fill-mode: both;
  }

  .guidance {
    max-width: 55ch;
    margin: 21px 0 0;
    color: rgba(239, 242, 232, 0.82);
    font-size: clamp(0.82rem, 1.05vw, 1rem);
    line-height: 1.55;
  }

  .timer {
    display: flex;
    flex-direction: column;
    align-items: center;
    gap: 10px;
    margin-top: clamp(21px, 3.8vh, 34px);
  }

  .timer svg {
    width: 55px;
    height: 55px;
    overflow: visible;
    transform: rotate(-90deg);
  }

  .timer circle {
    fill: none;
    stroke-width: 1.25;
  }

  .ring-track {
    stroke: rgba(228, 238, 228, 0.22);
  }

  .ring-progress {
    stroke: #a7d8b5;
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
    font-size: 0.8rem;
    font-variant-numeric: tabular-nums;
    letter-spacing: 0.08em;
    text-shadow: 0 2px 8px rgba(0, 0, 0, 0.72);
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
    min-width: 144px;
    min-height: 55px;
    align-items: center;
    justify-content: center;
    gap: var(--s2);
    border: 1px solid rgba(235, 243, 233, 0.52);
    border-radius: 999px;
    padding: 13px 21px;
    color: rgba(246, 247, 238, 0.92);
    background: rgba(3, 10, 7, 0.34);
    box-shadow:
      inset 0 1px 0 rgba(255, 255, 255, 0.06),
      0 12px 34px rgba(0, 0, 0, 0.16);
    font: inherit;
    font-size: 0.78rem;
    font-weight: 600;
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
    border-color: rgba(246, 248, 239, 0.78);
    color: #ffffff;
    background: rgba(6, 18, 12, 0.52);
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
    margin: 13px 0 0;
    color: rgba(228, 240, 232, 0.74);
    font-size: 0.75rem;
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
    margin-top: 8px;
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
    }
    to {
      opacity: 1;
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
      min-height: 44px;
      padding-block: 8px;
    }
  }

  @media (max-aspect-ratio: 4 / 3) {
    .break-artwork {
      object-position: 38.2% 50%;
      transform: none;
    }

    .break-content {
      width: min(88vw, 760px);
    }
  }

  @media (prefers-contrast: more) {
    .scene-veil {
      background: rgba(1, 5, 3, 0.76);
    }

    .copy-veil {
      background: none;
    }

    .return-light {
      background: radial-gradient(
        ellipse 61.8% 38.2% at var(--return-light-x) 61.8%,
        rgba(236, 187, 99, 0.42),
        transparent 61.8%
      );
    }

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

    .message,
    .timer-digits {
      animation: none;
      filter: none;
    }

    .return-light {
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
