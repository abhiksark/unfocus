<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { listen, type UnlistenFn } from "@tauri-apps/api/event";
  import { onMount, tick } from "svelte";
  import { createTrayPanelController, panelControls, panelCountdown, panelError, type TrayPanelSnapshot, type TrayPanelView } from "./tray-panel";

  let view = $state<TrayPanelView>({ snapshot: null, sampledAt: 0, busy: false, error: null });
  let now = $state(0);
  const controller = createTrayPanelController({
    read: () => invoke<TrayPanelSnapshot>("tray_panel_state"),
    act: (action, openingGeneration, requestId) => invoke<TrayPanelSnapshot>("tray_panel_action", { action, openingGeneration, requestId }),
    ready: (openingGeneration) => invoke<TrayPanelSnapshot>("tray_panel_ready", { openingGeneration }),
    paint: async () => { await tick(); },
    now: () => performance.now()
  }, (next) => { view = next; now = performance.now(); });
  const controls = $derived(panelControls(view));
  const error = $derived(panelError(view));
  const countdown = $derived(panelCountdown(view, now));
  const reminder = $derived(view.snapshot?.reminder);
  const remaining = $derived(reminder?.phase === "paused" ? reminder.pauseExpiresInMilliseconds : reminder?.remainingMilliseconds);
  const timing = $derived(remaining == null ? null : Math.max(0, Math.ceil((remaining - Math.max(0, now - view.sampledAt)) / 60000)));

  onMount(() => {
    let disposed = false;
    let openingReceived = false;
    let poll: ReturnType<typeof setInterval> | undefined;
    let paint: ReturnType<typeof setInterval> | undefined;
    const listeners: UnlistenFn[] = [];
    const stop = () => { clearInterval(poll); clearInterval(paint); poll = undefined; paint = undefined; };
    const start = () => {
      stop();
      poll = setInterval(() => void controller.refresh(), 1000);
      paint = setInterval(() => { if (view.snapshot?.visible) now = performance.now(); }, 100);
    };
    async function mount() {
      try {
        for (const [name, handler] of [
          ["tray-panel-opening", (generation: number) => { const opening = controller.open(generation); if (opening) { openingReceived = true; start(); void opening; } }],
          ["tray-panel-hidden", (generation: number) => { if (controller.hidden(generation)) stop(); }]
        ] as const) {
          const unlisten = await listen<number>(name, (event) => handler(event.payload), { target: { kind: "WebviewWindow", label: "tray-panel" } });
          if (disposed) { unlisten(); return; }
          listeners.push(unlisten);
        }
        if (!disposed) { start(); await controller.open(); if (!openingReceived && !view.snapshot?.visible) stop(); }
      } catch {
        if (!disposed) view = { ...view, error: "Could not connect. Open Unfocus to check." };
      }
    }
    void mount();
    return () => { disposed = true; stop(); controller.destroy(); listeners.forEach((unlisten) => unlisten()); };
  });
</script>

<svelte:window onkeydown={(event) => { if (event.key === "Escape") { event.preventDefault(); void controller.action("hide"); } }} />
<main class="tray-panel" aria-label="Unfocus break controls" data-type-role="interface">
  <header><img src="/unfocus-wordmark-mist.svg" alt="Unfocus" width="88" height="24" /></header>
  <section class="status" aria-label="Reminder status">
    {#if countdown !== null}
      <p class="status-label">A moment to settle</p>
      <p class="timing" aria-hidden="true">{countdown > 0 ? `Starting in ${countdown}` : "Starting…"}</p>
      <span class="sr-only" role="status">Your break will start shortly.</span>
    {:else if !reminder}
      <p class="status-label">Connecting…</p>
      <p class="timing">—</p>
    {:else if reminder.phase === "working"}
      <p class="status-label"><span class="state-dot"></span>Next break</p>
      <p class="timing">{timing === null ? "Time unavailable" : timing === 0 ? "Soon" : `In ${timing} min`}</p>
    {:else if reminder.phase === "paused"}
      <p class="status-label">Paused</p>
      <p class="timing">{timing === null ? "Reminders paused" : `Resumes in ${timing} min`}</p>
    {:else}
      <p class="status-label">{reminder.phase === "break" ? "Taking a break" : "Reminders unavailable"}</p>
      <p class="timing">{reminder.phase === "break" ? "Rest your eyes" : "Open Unfocus to check"}</p>
    {/if}
  </section>
  <div class="actions">
    {#if countdown !== null}
      <button type="button" disabled>Pause 30m</button>
      <button type="button" disabled={!controls.cancel} onclick={() => void controller.action("cancel")}>Cancel</button>
    {:else}
      <button type="button" disabled={!controls.pause} onclick={() => void controller.action(reminder?.pauseAction === "resume" ? "resume" : "pause")}>{reminder?.pauseAction === "resume" ? "Resume" : "Pause 30m"}</button>
      <button type="button" disabled={!controls.begin} onclick={() => void controller.action("begin")}>Take a break</button>
    {/if}
  </div>
  {#if error}<p class="error" role="status">{error}</p>{/if}
  <footer>
    <button type="button" disabled={view.busy} onclick={() => void controller.action("open")}>Open Unfocus <span aria-hidden="true">↗</span></button>
    <button type="button" disabled={view.busy} onclick={() => void controller.action("quit")}>Quit</button>
  </footer>
</main>

<style>
  .tray-panel { width: 100%; min-height: 228px; padding: var(--s4); font-family: var(--sans); color: var(--ink); background: var(--bg); font-size: 0.8125rem; }
  header { height: 28px; display: flex; align-items: center; }
  img { width: 88px; height: auto; }
  .status { padding: var(--s4) 0; }
  p { margin: 0; }
  .status-label { color: var(--ink-2); display: flex; align-items: center; gap: 6px; }
  .state-dot { width: 5px; height: 5px; border-radius: 50%; background: var(--accent); }
  .timing { margin-top: var(--s2); font-size: 1.125rem; font-weight: 500; font-variant-numeric: tabular-nums; }
  .actions { display: flex; gap: var(--s2); }
  button { font: inherit; color: var(--ink); cursor: pointer; border: 1px solid var(--line-2); background: transparent; border-radius: var(--r-control); min-height: 36px; }
  .actions button { flex: 1; padding: var(--s2) var(--s1); white-space: nowrap; }
  button:hover:enabled { background: var(--line); }
  button:disabled { color: var(--ink-3); cursor: default; }
  button:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  footer { display: flex; justify-content: space-between; gap: var(--s2); margin-top: var(--s3); border-top: 1px solid var(--line); padding-top: var(--s2); }
  footer button { border: 0; color: var(--ink-2); padding: var(--s1); }
  .error { color: var(--warn); font-size: 0.75rem; margin-top: var(--s2); }
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip-path: inset(50%); }
  @media (prefers-color-scheme: light) { img { filter: brightness(0.15); } }
</style>
