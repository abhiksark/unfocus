<script lang="ts">
  import BreakOverlay from "$lib/BreakOverlay.svelte";
  import ConsumerDashboard from "$lib/ConsumerDashboard.svelte";
  import DeveloperDashboard from "$lib/DeveloperDashboard.svelte";
  import {
    consumerReminderPresentation,
    consumerWarning
  } from "$lib/consumer-dashboard";
  import {
    readDashboardMode,
    writeDashboardMode,
    type DashboardMode
  } from "$lib/dashboard-mode";
  import type { DiagnosticsReport } from "$lib/diagnostics";
  import { parseWindowLabel } from "$lib/overlay-label";
  import {
    type ReminderSettings,
    validateReminderSettings
  } from "$lib/reminder-settings";
  import {
    pauseActionCommand,
    type ReminderActionCommand,
    type ReminderStatus
  } from "$lib/reminder-status";
  import type { BreakSummary } from "$lib/break-summary";
  import type { TodayActivity } from "$lib/today-activity";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount } from "svelte";

  type SettingsResult = "saved" | "reset" | null;

  const windowRoute = parseWindowLabel(getCurrentWindow().label);
  const overlayParameters = windowRoute.kind === "overlay" ? windowRoute.parameters : null;

  let dashboardMode = $state<DashboardMode>("consumer");
  let report = $state<DiagnosticsReport | null>(null);
  let diagnosticsError = $state<string | null>(null);
  let overlayError = $state<string | null>(null);
  let invalidOverlayCloseError = $state<string | null>(null);
  let refreshing = $state(false);
  let overlayRunning = $state(false);
  let timingEditorExpanded = $state(false);
  let savedSettings = $state<ReminderSettings | null>(null);
  let workMinutesInput = $state("");
  let breakSecondsInput = $state("");
  let settingsLoading = $state(true);
  let settingsSaving = $state(false);
  let settingsError = $state<string | null>(null);
  let settingsErrorContext = $state<"load" | "save" | "reset" | null>(null);
  let settingsResult = $state<SettingsResult>(null);
  let reminderStatus = $state<ReminderStatus | null>(null);
  let reminderStatusError = $state<string | null>(null);
  let reminderActionError = $state<string | null>(null);
  let reminderActionPending = $state<ReminderActionCommand | null>(null);
  let reminderActionResult = $state<string | null>(null);
  let reminderRefreshGeneration = 0;
  let todayActivity = $state<TodayActivity | null>(null);
  let todayActivityError = $state<string | null>(null);
  let breakSummary = $state<BreakSummary | null>(null);
  let breakSummaryError = $state<string | null>(null);

  function errorMessage(value: unknown): string {
    return value instanceof Error ? value.message : String(value);
  }

  const settingsValidation = $derived(
    validateReminderSettings(workMinutesInput, breakSecondsInput)
  );
  const workMinutesError = $derived(
    settingsLoading ? null : settingsValidation.workMinutesError
  );
  const breakSecondsError = $derived(
    settingsLoading ? null : settingsValidation.breakSecondsError
  );
  const reminderPresentation = $derived(
    consumerReminderPresentation(reminderStatus, reminderStatusError)
  );
  const warning = $derived(
    consumerWarning({
      report,
      diagnosticsError,
      reminderStatus,
      reminderStatusError,
      reminderActionError,
      settingsError,
      settingsErrorContext,
      overlayError
    })
  );

  function browserStorage(): Storage | null {
    try {
      return window.localStorage;
    } catch {
      return null;
    }
  }

  function setDashboardMode(mode: DashboardMode) {
    dashboardMode = mode;
    writeDashboardMode(browserStorage(), mode);
  }

  async function refresh() {
    if (refreshing) return;
    refreshing = true;
    const reminderGeneration = reminderRefreshGeneration;
    const [diagnostics, reminder, activity, breaks] = await Promise.allSettled([
      invoke<DiagnosticsReport>("get_diagnostics"),
      invoke<ReminderStatus>("get_reminder_status"),
      invoke<TodayActivity>("get_today_activity"),
      invoke<BreakSummary>("get_break_summary")
    ]);

    if (diagnostics.status === "fulfilled") {
      report = diagnostics.value;
      diagnosticsError = null;
    } else {
      diagnosticsError = errorMessage(diagnostics.reason);
    }
    if (
      reminder.status === "fulfilled" &&
      reminderGeneration === reminderRefreshGeneration
    ) {
      if (reminderStatus && reminder.value.stateRevision !== reminderStatus.stateRevision) {
        reminderActionResult = null;
      }
      reminderStatus = reminder.value;
      reminderStatusError = null;
      reminderActionError = null;
    } else if (
      reminder.status === "rejected" &&
      reminderGeneration === reminderRefreshGeneration
    ) {
      reminderStatusError = errorMessage(reminder.reason);
    }
    if (activity.status === "fulfilled") {
      todayActivity = activity.value;
      todayActivityError = null;
    } else {
      todayActivityError = errorMessage(activity.reason);
    }
    if (breaks.status === "fulfilled") {
      breakSummary = breaks.value;
      breakSummaryError = null;
    } else {
      breakSummaryError = errorMessage(breaks.reason);
    }
    refreshing = false;
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

  function reminderSuccessMessage(command: ReminderActionCommand): string {
    switch (command) {
      case "pause_reminders":
        return "Reminders paused for 30 minutes.";
      case "resume_reminders":
        return "Reminders resumed.";
      case "take_break_now":
        return "Break started.";
    }
  }

  async function runReminderAction(command: ReminderActionCommand) {
    if (reminderActionPending) return;
    reminderActionPending = command;
    reminderRefreshGeneration += 1;
    reminderStatusError = null;
    reminderActionError = null;
    reminderActionResult = null;
    try {
      const updated = await invoke<ReminderStatus>(command);
      reminderRefreshGeneration += 1;
      reminderStatus = updated;
      reminderActionResult = reminderSuccessMessage(command);
    } catch (value) {
      reminderRefreshGeneration += 1;
      reminderActionError = errorMessage(value);
    } finally {
      reminderActionPending = null;
    }
  }

  function runPauseAction() {
    if (!reminderStatus) return;
    void runReminderAction(pauseActionCommand(reminderStatus));
  }

  function useSettings(settings: ReminderSettings) {
    savedSettings = settings;
    workMinutesInput = String(settings.workMinutes);
    breakSecondsInput = String(settings.breakSeconds);
  }

  async function loadReminderSettings() {
    settingsLoading = true;
    settingsError = null;
    settingsErrorContext = null;
    try {
      useSettings(await invoke<ReminderSettings>("get_reminder_settings"));
    } catch (value) {
      settingsError = `Could not load reminder settings: ${errorMessage(value)}`;
      settingsErrorContext = "load";
      timingEditorExpanded = true;
    } finally {
      settingsLoading = false;
    }
  }

  async function saveReminderSettings() {
    const settings = settingsValidation.settings;
    if (!settings || settingsLoading || settingsSaving) return;

    settingsSaving = true;
    settingsError = null;
    settingsErrorContext = null;
    settingsResult = null;
    try {
      const saved = await invoke<ReminderSettings>("save_reminder_settings", { settings });
      useSettings(saved);
      settingsResult = "saved";
      settingsErrorContext = null;
      timingEditorExpanded = false;
    } catch (value) {
      settingsError = `Could not save reminder settings: ${errorMessage(value)}`;
      settingsErrorContext = "save";
      timingEditorExpanded = true;
    } finally {
      settingsSaving = false;
    }
  }

  async function resetReminderSettings() {
    if (settingsLoading || settingsSaving) return;

    settingsSaving = true;
    settingsError = null;
    settingsErrorContext = null;
    settingsResult = null;
    try {
      const defaults = await invoke<ReminderSettings>("reset_reminder_settings");
      useSettings(defaults);
      settingsResult = "reset";
      settingsErrorContext = null;
      timingEditorExpanded = false;
    } catch (value) {
      settingsError = `Could not reset reminder settings: ${errorMessage(value)}`;
      settingsErrorContext = "reset";
      timingEditorExpanded = true;
    } finally {
      settingsSaving = false;
    }
  }

  function updateWorkMinutes(value: string) {
    workMinutesInput = value;
    settingsResult = null;
  }

  function updateBreakSeconds(value: string) {
    breakSecondsInput = value;
    settingsResult = null;
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

  function handleSafeModeKeydown(event: KeyboardEvent) {
    if (windowRoute.kind !== "invalid-overlay" || event.key !== "Escape") return;
    event.preventDefault();
    void closeInvalidOverlay();
  }

  onMount(() => {
    if (windowRoute.kind !== "dashboard") return;

    dashboardMode = readDashboardMode(browserStorage());
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
    content="Local eye-break reminder dashboard for the Unfocus Tauri desktop app"
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
{:else if dashboardMode === "developer"}
  <DeveloperDashboard
    {report}
    {diagnosticsError}
    {overlayError}
    {reminderStatus}
    {reminderStatusError}
    {reminderActionError}
    {reminderActionPending}
    {reminderActionResult}
    {refreshing}
    {overlayRunning}
    {workMinutesInput}
    {breakSecondsInput}
    {settingsLoading}
    {settingsSaving}
    {settingsValidation}
    {workMinutesError}
    {breakSecondsError}
    {settingsError}
    {settingsResult}
    onReturnToConsumerMode={() => setDashboardMode("consumer")}
    onRefresh={() => void refresh()}
    onPauseAction={runPauseAction}
    onTakeBreak={() => void runReminderAction("take_break_now")}
    onPreview={() => void runOverlayTest()}
    onWorkMinutesInput={updateWorkMinutes}
    onBreakSecondsInput={updateBreakSeconds}
    onSaveSettings={() => void saveReminderSettings()}
    onResetSettings={() => void resetReminderSettings()}
  />
{:else}
  <ConsumerDashboard
    presentation={reminderPresentation}
    {warning}
    {reminderStatus}
    {reminderActionPending}
    {reminderActionResult}
    {overlayRunning}
    diagnosticsReady={report !== null}
    {todayActivity}
    {todayActivityError}
    {breakSummary}
    {breakSummaryError}
    {savedSettings}
    {timingEditorExpanded}
    {workMinutesInput}
    {breakSecondsInput}
    {settingsLoading}
    {settingsSaving}
    {settingsValidation}
    {workMinutesError}
    {breakSecondsError}
    {settingsError}
    {settingsErrorContext}
    {settingsResult}
    onTakeBreak={() => void runReminderAction("take_break_now")}
    onPauseAction={runPauseAction}
    onPreview={() => void runOverlayTest()}
    onToggleTimingEditor={() => (timingEditorExpanded = !timingEditorExpanded)}
    onWorkMinutesInput={updateWorkMinutes}
    onBreakSecondsInput={updateBreakSeconds}
    onSaveSettings={() => void saveReminderSettings()}
    onResetSettings={() => void resetReminderSettings()}
    onOpenDeveloperMode={() => setDashboardMode("developer")}
  />
{/if}

<style>
  :global(:root) {
    --bg: #0b100d;
    --line: #1d2620;
    --line-2: #2a3730;
    --ink: #e9f0ea;
    --ink-2: #9aa7a0;
    --ink-3: #7f8d85;
    --accent: #7fd79a;
    --accent-ink: #07130b;
    --away: #7a93a8;
    --warn: #d9b573;

    --s1: 4px;
    --s2: 8px;
    --s3: 12px;
    --s4: 16px;
    --s5: 24px;
    --s6: 32px;
    --s7: 48px;

    --r-control: 8px;
    --r-button: 999px;

    --sans: ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto,
      "Helvetica Neue", sans-serif;
    --serif: "Fraunces", Georgia, serif;
  }

  :global(*) {
    box-sizing: border-box;
  }

  :global(html) {
    font-family: var(--sans);
    color: var(--ink);
    background: var(--bg);
    font-synthesis: none;
  }

  :global(body) {
    margin: 0;
    min-width: 320px;
    min-height: 100vh;
    background: var(--bg);
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

  .invalid-overlay .eyebrow {
    color: #79cf91;
    font-size: 0.7rem;
    font-weight: 750;
    letter-spacing: 0.17em;
    text-transform: uppercase;
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

  .invalid-overlay button {
    border: 1px solid #3a473d;
    border-radius: 9px;
    padding: 11px 16px;
    color: #dce7de;
    background: #18201a;
    font: inherit;
    font-weight: 620;
    cursor: pointer;
  }

  .invalid-overlay button:focus-visible {
    outline: 3px solid #d9efdf;
    outline-offset: 3px;
  }

  .invalid-overlay .error {
    color: #ffc4c4;
    overflow-wrap: anywhere;
  }
</style>
