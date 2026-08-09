<script lang="ts">
  import BreakOverlay from "$lib/BreakOverlay.svelte";
  import {
    diagnosticsHealth,
    diagnosticsHealthLabel,
    probeBackend,
    type DiagnosticsReport
  } from "$lib/diagnostics";
  import { parseWindowLabel } from "$lib/overlay-label";
  import {
    MAX_BREAK_SECONDS,
    MAX_WORK_MINUTES,
    MIN_BREAK_SECONDS,
    MIN_WORK_MINUTES,
    type ReminderSettings,
    validateReminderSettings
  } from "$lib/reminder-settings";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";

  const windowRoute = parseWindowLabel(getCurrentWindow().label);
  const overlayParameters = windowRoute.kind === "overlay" ? windowRoute.parameters : null;

  let report = $state<DiagnosticsReport | null>(null);
  let diagnosticsError = $state<string | null>(null);
  let overlayError = $state<string | null>(null);
  let invalidOverlayCloseError = $state<string | null>(null);
  let refreshing = $state(false);
  let overlayRunning = $state(false);
  let workMinutesInput = $state("");
  let breakSecondsInput = $state("");
  let settingsLoading = $state(true);
  let settingsSaving = $state(false);
  let settingsError = $state<string | null>(null);
  let settingsStatus = $state<string | null>(null);

  function errorMessage(value: unknown): string {
    return value instanceof Error ? value.message : String(value);
  }

  const isMac = $derived(report?.operatingSystem === "macos");
  const health = $derived(diagnosticsHealth(report, diagnosticsError));
  const healthLabel = $derived(diagnosticsHealthLabel(health));
  const backend = $derived(probeBackend(report));
  const settingsValidation = $derived(
    validateReminderSettings(workMinutesInput, breakSecondsInput)
  );
  const workMinutesError = $derived(
    settingsLoading ? null : settingsValidation.workMinutesError
  );
  const breakSecondsError = $derived(
    settingsLoading ? null : settingsValidation.breakSecondsError
  );

  // A field the running platform simply does not report is not a pending
  // read, so say so rather than implying data is still on its way.
  function caption(value: string | null | undefined): string {
    if (!report) return "Connecting…";
    return value ?? `Not reported on ${report.operatingSystem}`;
  }

  const desktopCaption = $derived(caption(report?.desktop));
  const displayCaption = $derived(caption(report?.display));
  const idleCaption = $derived(
    report?.idleError ??
      (report ? (isMac ? "Quartz event source" : "XScreenSaver extension") : "Connecting…")
  );
  const fullscreenCaption = $derived(
    report?.fullscreenError ??
      (report ? (isMac ? "Quartz window list" : "EWMH window state") : "Connecting…")
  );

  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    try {
      report = await invoke<DiagnosticsReport>("get_diagnostics");
      diagnosticsError = null;
    } catch (value) {
      diagnosticsError = errorMessage(value);
    } finally {
      refreshing = false;
    }
  }

  async function runOverlayTest() {
    overlayRunning = true;
    overlayError = null;
    try {
      await invoke("show_overlay_test", { durationSeconds: 8 });
    } catch (value) {
      overlayError = errorMessage(value);
    } finally {
      overlayRunning = false;
    }
  }

  function useSettings(settings: ReminderSettings) {
    workMinutesInput = String(settings.workMinutes);
    breakSecondsInput = String(settings.breakSeconds);
  }

  async function loadReminderSettings() {
    settingsLoading = true;
    settingsError = null;
    try {
      useSettings(await invoke<ReminderSettings>("get_reminder_settings"));
    } catch (value) {
      settingsError = `Could not load reminder settings: ${errorMessage(value)}`;
    } finally {
      settingsLoading = false;
    }
  }

  async function saveReminderSettings() {
    const settings = settingsValidation.settings;
    if (!settings || settingsLoading || settingsSaving) return;

    settingsSaving = true;
    settingsError = null;
    settingsStatus = null;
    try {
      const saved = await invoke<ReminderSettings>("save_reminder_settings", { settings });
      useSettings(saved);
      settingsStatus =
        "Saved locally. A running work countdown restarts from now; an active break keeps its current deadline.";
    } catch (value) {
      settingsError = `Could not save reminder settings: ${errorMessage(value)}`;
    } finally {
      settingsSaving = false;
    }
  }

  async function resetReminderSettings() {
    if (settingsLoading || settingsSaving) return;

    settingsSaving = true;
    settingsError = null;
    settingsStatus = null;
    try {
      const defaults = await invoke<ReminderSettings>("reset_reminder_settings");
      useSettings(defaults);
      settingsStatus =
        "Defaults restored locally. A running work countdown restarts from now; an active break keeps its current deadline.";
    } catch (value) {
      settingsError = `Could not reset reminder settings: ${errorMessage(value)}`;
    } finally {
      settingsSaving = false;
    }
  }

  function updateWorkMinutes(event: Event) {
    workMinutesInput = (event.currentTarget as HTMLInputElement).value;
    settingsStatus = null;
  }

  function updateBreakSeconds(event: Event) {
    breakSecondsInput = (event.currentTarget as HTMLInputElement).value;
    settingsStatus = null;
  }

  async function closeOverlays() {
    if (!overlayParameters) throw new Error("Overlay parameters are unavailable");
    await invoke("close_overlay_test", { runId: overlayParameters.runId });
  }

  async function closeInvalidOverlay() {
    invalidOverlayCloseError = null;
    try {
      await getCurrentWindow().close();
    } catch (value) {
      invalidOverlayCloseError = errorMessage(value);
    }
  }

  // Safe mode replaces a break window that would otherwise be dismissable with
  // Escape, so it has to honour the same key. `<svelte:window>` has to sit at
  // the top level, so the route check lives here rather than in the markup.
  function handleSafeModeKeydown(event: KeyboardEvent) {
    if (windowRoute.kind !== "invalid-overlay" || event.key !== "Escape") return;
    event.preventDefault();
    void closeInvalidOverlay();
  }

  onMount(() => {
    if (windowRoute.kind !== "dashboard") return;

    let timer: number | undefined;
    const stopPolling = () => {
      if (timer !== undefined) window.clearInterval(timer);
      timer = undefined;
    };
    const updatePolling = () => {
      stopPolling();
      if (document.visibilityState !== "visible") return;
      void refresh();
      timer = window.setInterval(refresh, 2_000);
    };

    updatePolling();
    void loadReminderSettings();
    document.addEventListener("visibilitychange", updatePolling);
    return () => {
      stopPolling();
      document.removeEventListener("visibilitychange", updatePolling);
    };
  });
</script>

<svelte:head>
  <meta
    name="description"
    content="Platform diagnostics dashboard for the Unfocus Tauri desktop app"
  />
</svelte:head>

<svelte:window onkeydown={handleSafeModeKeydown} />

{#if overlayParameters}
  <BreakOverlay
    runId={overlayParameters.runId}
    monitorIndex={overlayParameters.monitorIndex}
    monitorCount={overlayParameters.monitorCount}
    durationSeconds={overlayParameters.durationSeconds}
    deadlineMs={overlayParameters.deadlineMs}
    onClose={closeOverlays}
  />
{:else if windowRoute.kind === "invalid-overlay"}
  <main class="invalid-overlay" role="alert" aria-labelledby="invalid-overlay-title">
    <p class="eyebrow">Safe mode</p>
    <h1 id="invalid-overlay-title">This break window could not start.</h1>
    <p>The native window label was invalid, so Unfocus did not render a blocking break.</p>
    <code>{windowRoute.reason}</code>
    <button class="secondary" type="button" onclick={closeInvalidOverlay}>Close this window</button>
    <p class="shortcut-hint">Press <kbd>Esc</kbd> to close.</p>
    {#if invalidOverlayCloseError}
      <p class="error">Could not close the window: {invalidOverlayCloseError}</p>
    {/if}
  </main>
{:else}
  <main class="dashboard">
    <header>
      <div class="brand-lockup">
        <div class="scene-swatch" class:degraded={health !== "healthy"} aria-hidden="true">
          <svg viewBox="0 0 56 56">
            <defs>
              <linearGradient id="sw-sky" x1="0" y1="0" x2="0" y2="56" gradientUnits="userSpaceOnUse">
                <stop offset="0" stop-color="#060f0d" />
                <stop offset="0.62" stop-color="#0c231d" />
                <stop offset="1" stop-color="#164434" />
              </linearGradient>
              <mask id="sw-moon">
                <circle cx="41" cy="14" r="6" fill="#fff" />
                <circle cx="38.8" cy="12.4" r="5.4" fill="#000" />
              </mask>
            </defs>
            <circle cx="41" cy="14" r="6" fill="#d7ecdc" opacity="0.85" mask="url(#sw-moon)" />
            <path
              d="M0 40 C6 38 10 24 16 21 C21 18.5 26 30 33 34 C42 39 50 39 56 37 L56 56 L0 56 Z"
              fill="#23493b"
            />
            <path
              d="M0 47 C9 43.5 18 46 27 47.5 C38 49.5 48 47 56 45 L56 56 L0 56 Z"
              fill="#0a1c16"
            />
          </svg>
        </div>
        <div>
          <p class="eyebrow">Unfocus · local reminder</p>
          <h1>Your timer is running.</h1>
          <p class="lede">
            Native evidence from Tauri and {backend}—not mocked browser data.
          </p>
        </div>
      </div>
      <div
        class="status-pill"
        class:healthy={health === "healthy"}
        class:degraded={health === "degraded"}
        class:unavailable={health === "unavailable"}
      >
        <span></span>{healthLabel}
      </div>
    </header>

    {#if diagnosticsError}
      <div class="error" role="alert">Diagnostics unavailable: {diagnosticsError}</div>
    {/if}

    {#if report?.tray.error}
      <div class="error" role="alert">Tray needs attention: {report.tray.error}</div>
    {/if}

    {#if overlayError}
      <div class="error" role="alert">Could not open the overlay: {overlayError}</div>
    {/if}

    <section class="summary-grid" aria-label="Platform probe summary">
      <article>
        <span>Session</span>
        <strong>{report?.sessionType?.toUpperCase() ?? "—"}</strong>
        <small>{desktopCaption}</small>
      </article>
      <article>
        <span>Displays</span>
        <strong>{report && !report.monitorError ? report.monitors.length : "—"}</strong>
        <small>{report?.monitorError ?? displayCaption}</small>
      </article>
      <article>
        <span>Idle time</span>
        <strong
          >{report?.idleSeconds ?? "—"}<i
            >{typeof report?.idleSeconds === "number" ? "s" : ""}</i
          ></strong
        >
        <small>{idleCaption}</small>
      </article>
      <article>
        <span>Active fullscreen</span>
        <strong>{report?.activeWindowFullscreen === true ? "Yes" : report?.activeWindowFullscreen === false ? "No" : "—"}</strong>
        <small>{fullscreenCaption}</small>
      </article>
    </section>

    <section class="panel settings-panel" aria-labelledby="reminder-settings-title">
      <div class="settings-copy">
        <p class="eyebrow">Reminder timing</p>
        <h2 id="reminder-settings-title">Choose your work and eye-break rhythm</h2>
        <p>
          Settings stay on this device. Saving during work starts a fresh work countdown;
          a break already on screen keeps the deadline it opened with.
        </p>
      </div>

      <form
        class="settings-form"
        novalidate
        onsubmit={(event) => {
          event.preventDefault();
          void saveReminderSettings();
        }}
      >
        <div class="duration-fields">
          <div class="duration-field">
            <label for="work-duration">Work duration</label>
            <div class="duration-input" class:invalid={workMinutesError}>
              <input
                id="work-duration"
                type="text"
                inputmode="numeric"
                pattern="[0-9]*"
                autocomplete="off"
                value={workMinutesInput}
                disabled={settingsLoading || settingsSaving}
                aria-invalid={workMinutesError ? "true" : "false"}
                aria-describedby={workMinutesError
                  ? "work-duration-help work-duration-error"
                  : "work-duration-help"}
                oninput={updateWorkMinutes}
              />
              <span>minutes</span>
            </div>
            <small id="work-duration-help">{MIN_WORK_MINUTES}–{MAX_WORK_MINUTES} whole minutes</small>
            {#if workMinutesError}
              <small id="work-duration-error" class="field-error">
                {workMinutesError}
              </small>
            {/if}
          </div>

          <div class="duration-field">
            <label for="break-duration">Break duration</label>
            <div class="duration-input" class:invalid={breakSecondsError}>
              <input
                id="break-duration"
                type="text"
                inputmode="numeric"
                pattern="[0-9]*"
                autocomplete="off"
                value={breakSecondsInput}
                disabled={settingsLoading || settingsSaving}
                aria-invalid={breakSecondsError ? "true" : "false"}
                aria-describedby={breakSecondsError
                  ? "break-duration-help break-duration-error"
                  : "break-duration-help"}
                oninput={updateBreakSeconds}
              />
              <span>seconds</span>
            </div>
            <small id="break-duration-help">{MIN_BREAK_SECONDS}–{MAX_BREAK_SECONDS} whole seconds</small>
            {#if breakSecondsError}
              <small id="break-duration-error" class="field-error">
                {breakSecondsError}
              </small>
            {/if}
          </div>
        </div>

        <div class="settings-actions">
          <button
            class="primary"
            type="submit"
            disabled={settingsLoading || settingsSaving || !settingsValidation.settings}
          >
            {settingsSaving ? "Saving…" : "Save timing"}
          </button>
          <button
            class="secondary"
            type="button"
            onclick={() => void resetReminderSettings()}
            disabled={settingsLoading || settingsSaving}
          >
            Reset to defaults
          </button>
        </div>

        <div class="settings-feedback" aria-live="polite">
          {#if settingsLoading}
            <p>Loading saved timing…</p>
          {:else if settingsError}
            <p class="settings-error" role="alert">{settingsError}</p>
          {:else if settingsStatus}
            <p class="settings-success">{settingsStatus}</p>
          {/if}
        </div>
      </form>
    </section>

    <section class="panel">
      <div class="panel-heading">
        <div>
          <p class="eyebrow">Monitor topology</p>
          <h2>Physical displays reported by Tauri</h2>
        </div>
        <button class="secondary compact" onclick={refresh} disabled={refreshing}>
          {refreshing ? "Reading…" : "Refresh"}
        </button>
      </div>

      <div class="monitor-list">
        {#each report?.monitors ?? [] as monitor, index}
          <article class="monitor">
            <div class="monitor-number">{index + 1}</div>
            <div>
              <strong>{monitor.name ?? `Display ${index + 1}`}</strong>
              <span>{monitor.width} × {monitor.height} px</span>
            </div>
            <code>{monitor.x}, {monitor.y} · {monitor.scaleFactor}×</code>
          </article>
        {:else}
          <p class="empty">Waiting for the native monitor API…</p>
        {/each}
      </div>
    </section>

    <section class="test-panel">
      <div>
        <p class="eyebrow">High-risk interaction</p>
        <h2>Cover every monitor for eight seconds</h2>
        <p>
          Creates one borderless, always-on-top Tauri window per display. The test closes
          itself; Escape is the safety exit.
        </p>
      </div>
      <button class="primary" onclick={runOverlayTest} disabled={overlayRunning || !report}>
        {overlayRunning ? "Opening…" : "Run overlay test"}
      </button>
    </section>

    <footer>
      {#if report?.tray.available === false}
        The tray is unavailable. Keep this dashboard open; closing it exits Unfocus safely.
      {:else if report?.tray.available}
        Closing this window sends Unfocus to the tray. Use the tray menu to reopen or quit.
      {:else}
        Closing uses the tray when available; otherwise Unfocus exits safely.
      {/if}
    </footer>
  </main>
{/if}

<style>
  :global(*) {
    box-sizing: border-box;
  }

  :global(html) {
    font-family:
      Inter, ui-sans-serif, system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI",
      sans-serif;
    color: #e8f0e9;
    background: #0d110e;
    font-synthesis: none;
  }

  :global(body) {
    margin: 0;
    min-width: 320px;
    min-height: 100vh;
    background:
      radial-gradient(circle at 85% 5%, rgba(104, 183, 126, 0.11), transparent 28rem),
      #0d110e;
  }

  button {
    border: 0;
    font: inherit;
    cursor: pointer;
  }

  button:disabled {
    cursor: wait;
    opacity: 0.55;
  }

  .dashboard {
    width: min(100%, 980px);
    margin: 0 auto;
    padding: 46px 42px 32px;
  }

  header,
  .panel-heading,
  .test-panel {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 28px;
  }

  .brand-lockup {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 20px;
  }

  /* Static Ridgeline swatch: the scene is the brand mark. Amber edge = a probe
     needs attention; the adjacent error banner carries the words. */
  .scene-swatch {
    flex: 0 0 auto;
    width: 58px;
    height: 58px;
    overflow: hidden;
    border: 1px solid #2c463a;
    border-radius: 15px;
    background: linear-gradient(180deg, #060f0d 0%, #0c231d 62%, #164434 100%);
  }

  .scene-swatch svg {
    display: block;
    width: 100%;
    height: 100%;
  }

  .scene-swatch.degraded {
    border-color: #d9bb7d;
  }

  h1,
  h2,
  p {
    margin-top: 0;
  }

  h1 {
    margin-bottom: 10px;
    font-family: "Fraunces", Georgia, serif;
    font-size: clamp(2.1rem, 5.5vw, 3.6rem);
    font-weight: 420;
    letter-spacing: 0;
    line-height: 1.05;
  }

  h2 {
    margin-bottom: 0;
    font-size: 1.15rem;
    font-weight: 590;
    letter-spacing: -0.02em;
  }

  .eyebrow {
    margin-bottom: 10px;
    color: #79cf91;
    font-size: 0.7rem;
    font-weight: 750;
    letter-spacing: 0.17em;
    text-transform: uppercase;
  }

  .lede,
  .test-panel p {
    margin-bottom: 0;
    color: #abb5ad;
    line-height: 1.55;
  }

  .status-pill {
    display: flex;
    align-items: center;
    gap: 9px;
    flex: 0 0 auto;
    border: 1px solid #303b32;
    border-radius: 100px;
    padding: 9px 13px;
    color: #89918a;
    font-size: 0.78rem;
  }

  .status-pill span {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #687069;
  }

  .status-pill.healthy {
    color: #b9ddc2;
  }

  .status-pill.healthy span {
    background: #66d184;
  }

  .status-pill.degraded {
    border-color: #78633a;
    color: #ecd5a5;
  }

  .status-pill.degraded span {
    background: #d9bb7d;
  }

  .status-pill.unavailable {
    border-color: #6e3434;
    color: #ffc4c4;
  }

  .status-pill.unavailable span {
    background: #d67878;
  }

  .error {
    margin-top: 24px;
    border: 1px solid #6e3434;
    border-radius: 12px;
    padding: 13px 15px;
    color: #ffc4c4;
    background: #2a1717;
    font-size: 0.83rem;
    overflow-wrap: anywhere;
  }

  .summary-grid {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    gap: 10px;
    margin: 36px 0 12px;
  }

  .summary-grid article,
  .panel,
  .test-panel {
    border: 1px solid #242d26;
    background: rgba(19, 25, 21, 0.82);
  }

  .summary-grid article {
    display: flex;
    min-width: 0;
    min-height: 132px;
    flex-direction: column;
    border-radius: 14px;
    padding: 17px;
  }

  .summary-grid span {
    color: #a0aaa2;
    font-size: 0.72rem;
  }

  .summary-grid strong {
    margin: auto 0 5px;
    overflow: hidden;
    color: #edf5ef;
    font-size: 1.65rem;
    font-weight: 590;
    text-overflow: ellipsis;
  }

  .summary-grid i {
    margin-left: 2px;
    color: #a0aaa2;
    font-size: 0.8rem;
    font-style: normal;
  }

  .summary-grid small {
    color: #a4aea6;
    font-size: 0.65rem;
    line-height: 1.35;
    overflow-wrap: anywhere;
  }

  .panel {
    border-radius: 14px;
    padding: 22px;
  }

  .settings-panel {
    display: grid;
    grid-template-columns: minmax(220px, 0.8fr) minmax(380px, 1.2fr);
    gap: 32px;
    margin-bottom: 12px;
  }

  .settings-copy p:not(.eyebrow) {
    margin: 10px 0 0;
    color: #abb5ad;
    font-size: 0.78rem;
    line-height: 1.55;
  }

  .settings-form {
    min-width: 0;
  }

  .duration-fields {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 12px;
  }

  .duration-field {
    min-width: 0;
  }

  .duration-field label {
    display: block;
    margin-bottom: 7px;
    color: #dce7de;
    font-size: 0.78rem;
    font-weight: 620;
  }

  .duration-input {
    display: flex;
    align-items: center;
    border: 1px solid #3a473d;
    border-radius: 9px;
    background: #0f1511;
  }

  .duration-input:focus-within {
    border-color: #75d38e;
    box-shadow: 0 0 0 3px rgba(117, 211, 142, 0.12);
  }

  .duration-input.invalid {
    border-color: #a95454;
  }

  .duration-input input {
    width: 100%;
    min-width: 0;
    border: 0;
    outline: 0;
    padding: 10px 5px 10px 12px;
    color: #edf5ef;
    background: transparent;
    font: inherit;
    font-variant-numeric: tabular-nums;
  }

  .duration-input input:disabled {
    color: #8c958e;
  }

  .duration-input span {
    flex: 0 0 auto;
    padding-right: 11px;
    color: #929c94;
    font-size: 0.68rem;
  }

  .duration-field small {
    display: block;
    margin-top: 6px;
    color: #929c94;
    font-size: 0.66rem;
    line-height: 1.35;
  }

  .duration-field .field-error,
  .settings-error {
    color: #ffc4c4;
  }

  .settings-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 16px;
  }

  .settings-feedback {
    min-height: 1.2rem;
    margin-top: 8px;
  }

  .settings-feedback p {
    margin: 0;
    font-size: 0.68rem;
    line-height: 1.4;
  }

  .settings-success {
    color: #b9ddc2;
  }

  .panel-heading {
    padding-bottom: 18px;
  }

  .secondary,
  .primary {
    border-radius: 9px;
    padding: 11px 16px;
    font-weight: 620;
  }

  .secondary {
    border: 1px solid #3a473d;
    color: #dce7de;
    background: #18201a;
  }

  .secondary:hover {
    border-color: #617365;
    background: #1e2921;
  }

  .compact {
    padding: 8px 12px;
    font-size: 0.75rem;
  }

  .monitor-list {
    display: grid;
    gap: 8px;
  }

  .monitor {
    display: grid;
    grid-template-columns: auto 1fr auto;
    align-items: center;
    gap: 13px;
    border-radius: 10px;
    padding: 11px 13px;
    background: #0f1511;
  }

  .monitor-number {
    display: grid;
    width: 30px;
    height: 30px;
    place-items: center;
    border: 1px solid #344039;
    border-radius: 7px;
    color: #84b790;
    font-size: 0.72rem;
  }

  .monitor strong,
  .monitor span {
    display: block;
  }

  .monitor strong {
    font-size: 0.82rem;
    font-weight: 610;
  }

  .monitor span,
  .monitor code,
  .empty {
    color: #9ca69e;
    font-size: 0.68rem;
  }

  .monitor code {
    font-family: "SFMono-Regular", Consolas, monospace;
  }

  .empty {
    margin: 8px 0;
  }

  .test-panel {
    margin-top: 12px;
    border-radius: 14px;
    padding: 22px;
  }

  .test-panel > div {
    max-width: 600px;
  }

  .test-panel p:not(.eyebrow) {
    margin-top: 9px;
    font-size: 0.78rem;
  }

  .primary {
    flex: 0 0 auto;
    color: #0a140d;
    background: #75d38e;
  }

  .primary:hover {
    background: #8ee1a5;
  }

  footer {
    margin-top: 20px;
    color: #929c94;
    font-size: 0.68rem;
    text-align: center;
  }

  .invalid-overlay {
    display: flex;
    min-height: 100vh;
    flex-direction: column;
    align-items: flex-start;
    justify-content: center;
    gap: 16px;
    padding: clamp(28px, 8vw, 72px);
    color: #edf5ef;
    background: #0d110e;
  }

  .invalid-overlay h1,
  .invalid-overlay p {
    max-width: 680px;
    margin-bottom: 0;
  }

  .invalid-overlay .shortcut-hint {
    color: #9ca69e;
    font-size: 0.72rem;
  }

  .invalid-overlay kbd {
    display: inline-block;
    min-width: 25px;
    border: 1px solid rgba(226, 240, 231, 0.38);
    border-radius: 5px;
    padding: 2px 5px;
    color: #edf7f0;
    background: rgba(255, 255, 255, 0.08);
    font: inherit;
    text-align: center;
  }

  .invalid-overlay code {
    max-width: 100%;
    border-radius: 8px;
    padding: 9px 11px;
    color: #ffc4c4;
    background: #2a1717;
    overflow-wrap: anywhere;
  }

  @media (max-width: 760px) {
    .dashboard {
      padding: 30px 22px;
    }

    .summary-grid {
      grid-template-columns: repeat(2, 1fr);
    }

    .settings-panel,
    .duration-fields {
      grid-template-columns: 1fr;
    }

    header,
    .test-panel {
      align-items: flex-start;
      flex-direction: column;
    }

    .brand-lockup {
      align-items: flex-start;
      gap: 14px;
    }

    .monitor {
      grid-template-columns: auto 1fr;
    }

    .monitor code {
      display: none;
    }
  }
</style>
