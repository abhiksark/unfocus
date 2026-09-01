<!-- src/lib/DeveloperDashboard.svelte -->

<script lang="ts">
  import {
    diagnosticsHealth,
    diagnosticsHealthLabel,
    probeBackend,
    type DiagnosticsReport
  } from "$lib/diagnostics";
  import {
    MAX_BREAK_SECONDS,
    MAX_WORK_MINUTES,
    MIN_BREAK_SECONDS,
    MIN_WORK_MINUTES,
    type ReminderSettingsValidation
  } from "$lib/reminder-settings";
  import type { ReminderActionCommand, ReminderStatus } from "$lib/reminder-status";

  type SettingsResult = "saved" | "reset" | null;

  type Props = {
    report: DiagnosticsReport | null;
    diagnosticsError: string | null;
    overlayError: string | null;
    reminderStatus: ReminderStatus | null;
    reminderStatusError: string | null;
    reminderActionError: string | null;
    reminderActionPending: ReminderActionCommand | null;
    reminderActionResult: string | null;
    refreshing: boolean;
    overlayRunning: boolean;
    workMinutesInput: string;
    breakSecondsInput: string;
    settingsLoading: boolean;
    settingsSaving: boolean;
    settingsValidation: ReminderSettingsValidation;
    workMinutesError: string | null;
    breakSecondsError: string | null;
    settingsError: string | null;
    settingsResult: SettingsResult;
    onReturnToConsumerMode: () => void;
    onRefresh: () => void;
    onPauseAction: () => void;
    onTakeBreak: () => void;
    onPreview: () => void;
    onWorkMinutesInput: (value: string) => void;
    onBreakSecondsInput: (value: string) => void;
    onSaveSettings: () => void;
    onResetSettings: () => void;
  };

  let {
    report,
    diagnosticsError,
    overlayError,
    reminderStatus,
    reminderStatusError,
    reminderActionError,
    reminderActionPending,
    reminderActionResult,
    refreshing,
    overlayRunning,
    workMinutesInput,
    breakSecondsInput,
    settingsLoading,
    settingsSaving,
    settingsValidation,
    workMinutesError,
    breakSecondsError,
    settingsError,
    settingsResult,
    onReturnToConsumerMode,
    onRefresh,
    onPauseAction,
    onTakeBreak,
    onPreview,
    onWorkMinutesInput,
    onBreakSecondsInput,
    onSaveSettings,
    onResetSettings
  }: Props = $props();

  const isMac = $derived(report?.operatingSystem === "macos");
  const health = $derived(diagnosticsHealth(report, diagnosticsError));
  const healthLabel = $derived(diagnosticsHealthLabel(health));
  const backend = $derived(probeBackend(report));
  const settingsStatus = $derived(
    settingsResult === "saved"
      ? "Saved locally. Timing changes restart a running work countdown; cue-only changes do not. An active break keeps its current deadline."
      : settingsResult === "reset"
        ? "Defaults restored locally. A running work countdown restarts from now; an active break keeps its current deadline."
        : null
  );

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
</script>

<main class="developer-dashboard">
  <header>
    <div class="brand-lockup">
      <div class="scene-swatch" class:degraded={health !== "healthy"} aria-hidden="true">
        <span></span>
      </div>
      <div>
        <p class="eyebrow">Developer mode</p>
        <h1 data-type-role="ui">Your timer is running.</h1>
        <p class="lede">Native evidence from Tauri and {backend}—not mocked browser data.</p>
      </div>
    </div>
    <div class="header-actions">
      <div
        class="status-pill"
        class:healthy={health === "healthy"}
        class:degraded={health === "degraded"}
        class:unavailable={health === "unavailable"}
      >
        <span></span>{healthLabel}
      </div>
      <button class="return-button" type="button" onclick={onReturnToConsumerMode}>
        Return to consumer mode
      </button>
    </div>
  </header>

  {#if diagnosticsError}
    <div class="error" role="alert">Diagnostics unavailable: {diagnosticsError}</div>
  {/if}
  {#if report?.tray.error}
    <div class="error" role="alert">Tray needs attention: {report.tray.error}</div>
  {/if}
  {#if reminderStatusError}
    <div class="error" role="alert">Reminder status unavailable: {reminderStatusError}</div>
  {/if}
  {#if reminderActionError}
    <div class="error" role="alert">
      Could not update reminder state: {reminderActionError}. The existing timer state was kept.
    </div>
  {/if}
  {#if reminderStatus?.actionError}
    <div class="error" role="alert">
      Last reminder action failed: {reminderStatus.actionError}. The existing timer state was kept.
    </div>
  {/if}
  {#if overlayError}
    <div class="error" role="alert">Could not open the overlay: {overlayError}</div>
  {/if}

  <section class="summary-grid" aria-label="Platform probe summary">
    <article>
      <span>Session</span>
      <strong data-type-role="mono">{report?.sessionType?.toUpperCase() ?? "—"}</strong>
      <small>{desktopCaption}</small>
    </article>
    <article>
      <span>Displays</span>
      <strong data-type-role="mono">
        {report && !report.monitorError ? report.monitors.length : "—"}
      </strong>
      <small>{report?.monitorError ?? displayCaption}</small>
    </article>
    <article>
      <span>Idle time</span>
      <strong data-type-role="mono">
        {report?.idleSeconds ?? "—"}<i>{typeof report?.idleSeconds === "number" ? "s" : ""}</i>
      </strong>
      <small>{idleCaption}</small>
    </article>
    <article>
      <span>Active fullscreen</span>
      <strong data-type-role="mono">
        {report?.activeWindowFullscreen === true
          ? "Yes"
          : report?.activeWindowFullscreen === false
            ? "No"
            : "—"}
      </strong>
      <small>{fullscreenCaption}</small>
    </article>
  </section>

  <section class="panel reminder-controls-panel" aria-labelledby="developer-reminder-controls-title">
    <div class="reminder-controls-copy">
      <p class="eyebrow">Reminder controls</p>
      <h2 id="developer-reminder-controls-title">{reminderStatus?.status ?? "Reading timer status…"}</h2>
      <p>
        Pause is stored locally and expires after 30 minutes, including across a restart.
        Resuming starts a fresh work countdown. Take a break now uses the configured break
        duration and then begins the next work phase.
      </p>
    </div>
    <div class="reminder-actions">
      <button
        class="secondary"
        type="button"
        onclick={onPauseAction}
        disabled={!reminderStatus?.pauseActionEnabled || reminderActionPending !== null}
      >
        {reminderActionPending === "pause_reminders" || reminderActionPending === "resume_reminders"
          ? "Updating…"
          : (reminderStatus?.pauseActionLabel ?? "Pause for 30 minutes")}
      </button>
      <button
        class="primary"
        type="button"
        onclick={onTakeBreak}
        disabled={!reminderStatus?.takeBreakEnabled || reminderActionPending !== null}
      >
        {reminderActionPending === "take_break_now" ? "Starting…" : "Take a break now"}
      </button>
      <div class="reminder-feedback" aria-live="polite">{reminderActionResult ?? ""}</div>
    </div>
  </section>

  <section class="panel settings-panel" aria-labelledby="developer-reminder-settings-title">
    <div class="settings-copy">
      <p class="eyebrow">Reminder timing</p>
      <h2 id="developer-reminder-settings-title">Choose your work and eye-break rhythm</h2>
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
        onSaveSettings();
      }}
    >
      <div class="duration-fields">
        <div class="duration-field">
          <label for="developer-work-duration">Focus duration</label>
          <div class="duration-input" class:invalid={workMinutesError}>
            <input
              id="developer-work-duration"
              data-type-role="mono"
              type="text"
              inputmode="numeric"
              pattern="[0-9]*"
              autocomplete="off"
              value={workMinutesInput}
              disabled={settingsLoading || settingsSaving}
              aria-invalid={workMinutesError ? "true" : "false"}
              aria-describedby={workMinutesError ? "developer-work-help developer-work-error" : "developer-work-help"}
              oninput={(event) => onWorkMinutesInput((event.currentTarget as HTMLInputElement).value)}
            />
            <span>minutes</span>
          </div>
          <small id="developer-work-help">{MIN_WORK_MINUTES}–{MAX_WORK_MINUTES} whole minutes</small>
          {#if workMinutesError}
            <small id="developer-work-error" class="field-error">{workMinutesError}</small>
          {/if}
        </div>
        <div class="duration-field">
          <label for="developer-break-duration">Rest duration</label>
          <div class="duration-input" class:invalid={breakSecondsError}>
            <input
              id="developer-break-duration"
              data-type-role="mono"
              type="text"
              inputmode="numeric"
              pattern="[0-9]*"
              autocomplete="off"
              value={breakSecondsInput}
              disabled={settingsLoading || settingsSaving}
              aria-invalid={breakSecondsError ? "true" : "false"}
              aria-describedby={breakSecondsError ? "developer-break-help developer-break-error" : "developer-break-help"}
              oninput={(event) => onBreakSecondsInput((event.currentTarget as HTMLInputElement).value)}
            />
            <span>seconds</span>
          </div>
          <small id="developer-break-help">{MIN_BREAK_SECONDS}–{MAX_BREAK_SECONDS} whole seconds</small>
          {#if breakSecondsError}
            <small id="developer-break-error" class="field-error">{breakSecondsError}</small>
          {/if}
        </div>
      </div>
      <div class="settings-actions">
        <button class="primary" type="submit" disabled={settingsLoading || settingsSaving || !settingsValidation.settings}>
          {settingsSaving ? "Saving…" : "Save timing"}
        </button>
        <button class="secondary" type="button" onclick={onResetSettings} disabled={settingsLoading || settingsSaving}>
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
      <button class="secondary compact" type="button" onclick={onRefresh} disabled={refreshing}>
        {refreshing ? "Reading…" : "Refresh"}
      </button>
    </div>
    <div class="monitor-list">
      {#each report?.monitors ?? [] as monitor, index}
        <article class="monitor">
          <div class="monitor-number" data-type-role="mono">{index + 1}</div>
          <div>
            <strong>{monitor.name ?? `Display ${index + 1}`}</strong>
            <span data-type-role="mono">{monitor.width} × {monitor.height} px</span>
          </div>
          <code data-type-role="mono">{monitor.x}, {monitor.y} · {monitor.scaleFactor}×</code>
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
        itself; Space ends it early, and Escape remains the safety fallback.
      </p>
    </div>
    <button
      class="primary"
      type="button"
      onclick={onPreview}
      disabled={overlayRunning || !report || !reminderStatus?.previewEnabled}
    >
      {overlayRunning
        ? "Opening…"
        : reminderStatus && !reminderStatus.previewEnabled
          ? "Overlay active"
          : "Run overlay test"}
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

<style>
  .developer-dashboard {
    width: min(100%, 980px);
    margin: 0 auto;
    padding: 36px 42px 32px;
  }

  header,
  .panel-heading,
  .test-panel {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: 28px;
  }

  .brand-lockup,
  .header-actions {
    display: flex;
    min-width: 0;
    align-items: center;
    gap: 16px;
  }

  .header-actions {
    flex-direction: column;
    align-items: flex-end;
    gap: 9px;
  }

  .scene-swatch {
    position: relative;
    flex: 0 0 auto;
    width: 58px;
    height: 58px;
    overflow: hidden;
    border: 1px solid #2c463a;
    border-radius: 15px;
    background:
      radial-gradient(
        ellipse 61.8% 38.2% at 61.8% 38.2%,
        rgba(125, 207, 155, 0.24),
        transparent 61.8%
      ),
      linear-gradient(145deg, #789a8d 0%, #274b3f 61.8%, #07100c 100%);
  }

  .scene-swatch::before {
    position: absolute;
    right: -9px;
    bottom: -13px;
    left: -11px;
    height: 34px;
    border-radius: 61.8% 38.2% 0 0 / 100% 100% 0 0;
    background: #153a2e;
    content: "";
    transform: rotate(-5deg);
  }

  .scene-swatch::after {
    position: absolute;
    top: calc(38.2% - 9px);
    left: calc(61.8% - 9px);
    width: 18px;
    height: 18px;
    border: 1px solid rgba(215, 236, 220, 0.42);
    border-radius: 50%;
    content: "";
  }

  .scene-swatch span {
    position: absolute;
    z-index: 1;
    top: calc(38.2% - 3px);
    left: calc(61.8% - 3px);
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #7dcf9b;
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
    margin-bottom: 8px;
    font-size: clamp(2rem, 5vw, 3.3rem);
    font-weight: 600;
    line-height: 1.08;
  }

  h2 {
    margin-bottom: 0;
    font-size: 1.15rem;
    font-weight: 600;
    letter-spacing: -0.02em;
  }

  .eyebrow {
    margin-bottom: 8px;
    color: #79cf91;
    font-size: 0.7rem;
    font-weight: 600;
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
    border: 1px solid #303b32;
    border-radius: 100px;
    padding: 7px 11px;
    color: #89918a;
    font-size: 0.76rem;
  }

  .status-pill span {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: #687069;
  }

  .status-pill.healthy { color: #b9ddc2; }
  .status-pill.healthy span { background: #66d184; }
  .status-pill.degraded { border-color: #78633a; color: #ecd5a5; }
  .status-pill.degraded span { background: #d9bb7d; }
  .status-pill.unavailable { border-color: #6e3434; color: #ffc4c4; }
  .status-pill.unavailable span { background: #d67878; }

  button {
    border: 0;
    border-radius: 9px;
    padding: 11px 16px;
    font: inherit;
    font-weight: 600;
    cursor: pointer;
  }

  button:focus-visible {
    outline: 3px solid #d9efdf;
    outline-offset: 3px;
  }

  button:disabled {
    cursor: not-allowed;
    opacity: 0.55;
  }

  .return-button {
    border: 1px solid #5f8068;
    padding: 8px 12px;
    color: #e4f3e8;
    background: #1c2b20;
    font-size: 0.75rem;
  }

  .error {
    margin-top: 20px;
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
    margin: 30px 0 12px;
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

  .summary-grid span { color: #a0aaa2; font-size: 0.75rem; }
  .summary-grid strong {
    margin: auto 0 5px;
    overflow: hidden;
    color: #edf5ef;
    font-size: 1.65rem;
    font-weight: 600;
    text-overflow: ellipsis;
  }
  .summary-grid i { margin-left: 2px; color: #a0aaa2; font-size: 0.8rem; font-style: normal; }
  .summary-grid small { color: #a4aea6; font-size: 0.75rem; line-height: 1.4; overflow-wrap: anywhere; }

  .panel { border-radius: 14px; padding: 22px; }

  .reminder-controls-panel {
    display: grid;
    grid-template-columns: minmax(260px, 1fr) auto;
    align-items: center;
    gap: 28px;
    margin-bottom: 12px;
    border-color: #31513d;
  }

  .reminder-controls-copy p:not(.eyebrow),
  .settings-copy p:not(.eyebrow) {
    max-width: 620px;
    margin: 10px 0 0;
    color: #abb5ad;
    font-size: 0.78rem;
    line-height: 1.55;
  }

  .reminder-actions { display: flex; flex-wrap: wrap; justify-content: flex-end; gap: 8px; }
  .reminder-feedback { width: 100%; color: #b9ddc2; font-size: 0.75rem; text-align: right; }

  .settings-panel {
    display: grid;
    grid-template-columns: minmax(220px, 0.8fr) minmax(380px, 1.2fr);
    gap: 32px;
    margin-bottom: 12px;
  }

  .settings-form { min-width: 0; }
  .duration-fields { display: grid; grid-template-columns: repeat(2, minmax(0, 1fr)); gap: 12px; }
  .duration-field { min-width: 0; }
  .duration-field label { display: block; margin-bottom: 7px; color: #dce7de; font-size: 0.78rem; font-weight: 600; }
  .duration-input { display: flex; align-items: center; border: 1px solid #3a473d; border-radius: 9px; background: #0f1511; }
  .duration-input:focus-within { border-color: #75d38e; box-shadow: 0 0 0 3px rgba(117, 211, 142, 0.12); }
  .duration-input.invalid { border-color: #a95454; }
  .duration-input input { width: 100%; min-width: 0; border: 0; outline: 0; padding: 10px 5px 10px 12px; color: #edf5ef; background: transparent; font: inherit; font-family: var(--mono); }
  .duration-input span { padding-right: 11px; color: #929c94; font-size: 0.75rem; }
  .duration-field small { display: block; margin-top: 6px; color: #929c94; font-size: 0.75rem; line-height: 1.4; }
  .duration-field .field-error,
  .settings-error { color: #ffc4c4; }
  .settings-actions { display: flex; flex-wrap: wrap; gap: 8px; margin-top: 16px; }
  .settings-feedback { min-height: 1.2rem; margin-top: 8px; }
  .settings-feedback p { margin: 0; font-size: 0.75rem; line-height: 1.4; }
  .settings-success { color: #b9ddc2; }

  .panel-heading { padding-bottom: 18px; }
  .secondary { border: 1px solid #3a473d; color: #dce7de; background: #18201a; }
  .secondary:hover:not(:disabled) { border-color: #617365; background: #1e2921; }
  .primary { flex: 0 0 auto; color: #0a140d; background: #75d38e; }
  .primary:hover:not(:disabled) { background: #8ee1a5; }
  .compact { padding: 8px 12px; font-size: 0.75rem; }

  .monitor-list { display: grid; gap: 8px; }
  .monitor { display: grid; grid-template-columns: auto 1fr auto; align-items: center; gap: 13px; border-radius: 10px; padding: 11px 13px; background: #0f1511; }
  .monitor-number { display: grid; width: 30px; height: 30px; place-items: center; border: 1px solid #344039; border-radius: 7px; color: #84b790; font-size: 0.75rem; }
  .monitor strong,
  .monitor span { display: block; }
  .monitor strong { font-size: 0.82rem; font-weight: 600; }
  .monitor span,
  .monitor code,
  .empty { color: #9ca69e; font-size: 0.75rem; }
  .empty { margin: 8px 0; }

  .test-panel { margin-top: 12px; border-radius: 14px; padding: 22px; }
  .test-panel > div { max-width: 600px; }
  .test-panel p:not(.eyebrow) { margin-top: 9px; font-size: 0.78rem; }
  footer { margin-top: 20px; color: #929c94; font-size: 0.75rem; text-align: center; }

  @media (max-width: 760px) {
    .developer-dashboard { padding: 28px 22px; }
    .summary-grid { grid-template-columns: repeat(2, 1fr); }
    .reminder-controls-panel,
    .settings-panel,
    .duration-fields { grid-template-columns: 1fr; }
    .reminder-actions { justify-content: flex-start; }
    .reminder-feedback { text-align: left; }
    header,
    .test-panel { align-items: flex-start; flex-direction: column; }
    .brand-lockup { align-items: flex-start; gap: 14px; }
    .header-actions { align-items: flex-start; }
    .monitor { grid-template-columns: auto 1fr; }
    .monitor code { display: none; }
  }
</style>
