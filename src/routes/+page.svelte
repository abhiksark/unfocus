<!-- src/routes/+page.svelte -->

<script lang="ts">
  import BreakOverlay from "$lib/BreakOverlay.svelte";
  import ConsumerDashboard from "$lib/ConsumerDashboard.svelte";
  import DeveloperDashboard from "$lib/DeveloperDashboard.svelte";
  import HistoryView from "$lib/HistoryView.svelte";
  import PreBreakCue from "$lib/PreBreakCue.svelte";
  import { deviceGridOffsetMinutes } from "$lib/break-grid";
  import {
    consumerReminderPresentation,
    consumerWarning
  } from "$lib/consumer-dashboard";
  import {
    readDashboardMode,
    writeDashboardMode,
    type DashboardMode
  } from "$lib/dashboard-mode";
  import {
    DEFAULT_DAY_START_HOUR,
    readDayStartHour,
    writeDayStartHour
  } from "$lib/day-start";
  import type { DiagnosticsReport } from "$lib/diagnostics";
  import { historyActivationUsesKeyboard } from "$lib/history";
  import { parseWindowLabel } from "$lib/overlay-label";
  import {
    loadAuthoritativeReminderSettings,
    reminderSettingsRecovery,
    resolveReminderSettingsSave,
    type ReminderSettings,
    type ReminderSettingsErrorContext,
    type ReminderSettingsSnapshotFollowUp,
    type ReminderSettingsView,
    validateReminderSettings
  } from "$lib/reminder-settings";
  import {
    createReminderOperationGate,
    pauseActionCommand,
    reminderCapabilityAvailable,
    type ReminderActionCommand,
    type ReminderStatus
  } from "$lib/reminder-status";
  import type { BreakSummary } from "$lib/break-summary";
  import {
    createRefreshGenerationGuard,
    createRequestSequence,
    settleLatestRequest,
    type LatestRequestResult
  } from "$lib/refresh-generation";
  import {
    applyReflectionFailure,
    applyReflectionRecoveryRejection,
    applyReflectionRecoverySnapshot,
    applyReflectionSnapshot,
    initialReflectionResource,
    recoveryErrorAfterReflectionPoll,
    type ReflectionRecoveryError,
    type ReflectionResource
  } from "$lib/reflection-resource";
  import type { LocalSnapshot, StorageLoadHealth } from "$lib/storage-health";
  import type { TodayActivity } from "$lib/today-activity";
  import { invoke } from "@tauri-apps/api/core";
  import { getCurrentWindow } from "@tauri-apps/api/window";
  import { onMount, tick } from "svelte";

  type SettingsResult = "saved" | "reset" | null;

  const windowRoute = parseWindowLabel(getCurrentWindow().label);
  const overlayParameters = windowRoute.kind === "overlay" ? windowRoute.parameters : null;
  const cueParameters = windowRoute.kind === "cue" ? windowRoute.parameters : null;
  document.documentElement.classList.toggle(
    "cue-window",
    windowRoute.kind === "cue" || windowRoute.kind === "invalid-cue"
  );

  let dashboardMode = $state<DashboardMode>("consumer");
  let dashboardView = $state<"dashboard" | "history">("dashboard");
  let historyMounted = $state(false);
  let dashboardScrollY = 0;
  let historyScrollY = 0;
  let dayStartHour = $state(DEFAULT_DAY_START_HOUR);
  let report = $state<DiagnosticsReport | null>(null);
  let diagnosticsError = $state<string | null>(null);
  let diagnosticsCurrentSuccessful = $state(false);
  const diagnosticsRequests = createRequestSequence();
  let overlayError = $state<string | null>(null);
  let invalidOverlayCloseError = $state<string | null>(null);
  let authorWebsiteError = $state(false);
  let refreshing = $state(false);
  let overlayRunning = $state(false);
  let timingEditorExpanded = $state(false);
  let savedSettings = $state<ReminderSettings | null>(null);
  let settingsStorageHealth = $state<StorageLoadHealth | null>(null);
  let settingsOperationPending = $state<"retry" | "save" | "reset" | null>(null);
  let settingsRecoveryError = $state(false);
  const reminderOperationGate = createReminderOperationGate();
  const settingsRefreshGeneration = createRefreshGenerationGuard();
  const settingsRequests = createRequestSequence();
  let workMinutesInput = $state("");
  let breakSecondsInput = $state("");
  let syncAcrossDevices = $state(false);
  let gridOffsetMinutes = $state(0);
  let preBreakCueEnabled = $state(true);
  let settingsLoading = $state(true);
  const settingsSaving = $derived(
    settingsOperationPending === "save" || settingsOperationPending === "reset"
  );
  const settingsRecoveryPending = $derived<"retry" | "reset" | null>(
    settingsOperationPending === "retry" || settingsOperationPending === "reset"
      ? settingsOperationPending
      : null
  );
  let settingsError = $state<string | null>(null);
  let settingsErrorContext = $state<ReminderSettingsErrorContext>(null);
  let settingsResult = $state<SettingsResult>(null);
  let reminderStatus = $state<ReminderStatus | null>(null);
  let reminderStatusError = $state<string | null>(null);
  let reminderActionError = $state<string | null>(null);
  let reminderActionPending = $state<ReminderActionCommand | null>(null);
  let reminderActionResult = $state<string | null>(null);
  let reminderRefreshGeneration = 0;
  const reminderRequests = createRequestSequence();
  let activityResource = $state<ReflectionResource<TodayActivity>>(
    initialReflectionResource()
  );
  let activityRecoveryPending = $state<"retry" | "startNew" | null>(null);
  let activityRecoveryError = $state<ReflectionRecoveryError>(null);
  const activityRefreshGeneration = createRefreshGenerationGuard();
  const activityRequests = createRequestSequence();
  let breakResource = $state<ReflectionResource<BreakSummary>>(
    initialReflectionResource()
  );
  let breakRecoveryPending = $state<"retry" | "startNew" | null>(null);
  let breakRecoveryError = $state<ReflectionRecoveryError>(null);
  const breakRefreshGeneration = createRefreshGenerationGuard();
  const breakRequests = createRequestSequence();

  function errorMessage(value: unknown): string {
    return value instanceof Error ? value.message : String(value);
  }

  function requestDiagnostics() {
    diagnosticsCurrentSuccessful = false;
    return settleLatestRequest(diagnosticsRequests, () =>
      invoke<DiagnosticsReport>("get_diagnostics")
    );
  }

  function publishDiagnostics(
    request: LatestRequestResult<DiagnosticsReport>,
    current: boolean
  ): void {
    if (!current) return;
    if (request.settled.status === "fulfilled") {
      report = request.settled.value;
      diagnosticsError = null;
      diagnosticsCurrentSuccessful = true;
    } else {
      diagnosticsError = errorMessage(request.settled.reason);
      diagnosticsCurrentSuccessful = false;
    }
  }

  const settingsValidation = $derived(
    validateReminderSettings(workMinutesInput, breakSecondsInput, {
      syncAcrossDevices,
      gridOffsetMinutes,
      preBreakCueEnabled
    })
  );
  const authoritativeSettingsHealth = $derived<StorageLoadHealth | null>(
    settingsStorageHealth ??
      (report && diagnosticsError === null
        ? report.storage.reminderSettings
        : null)
  );
  const settingsRecovery = $derived(
    reminderSettingsRecovery(
      authoritativeSettingsHealth,
      savedSettings,
      settingsLoading
    )
  );
  const settingsUnavailable = $derived(settingsRecovery.unavailable);
  const workMinutesError = $derived(
    settingsUnavailable ? null : settingsValidation.workMinutesError
  );
  const breakSecondsError = $derived(
    settingsUnavailable ? null : settingsValidation.breakSecondsError
  );
  const reminderPresentation = $derived(
    consumerReminderPresentation(
      reminderStatus,
      reminderStatusError,
      authoritativeSettingsHealth
    )
  );
  const preBreakCueAvailable = $derived(
    report?.probeBackend?.kind === "x11" ||
      (report?.operatingSystem === "linux" && report.sessionType?.toLowerCase() === "x11")
  );
  const warning = $derived(
    consumerWarning({
      report,
      diagnosticsError,
      reminderStatus,
      reminderStatusError,
      reminderActionError,
      settingsStorageHealth: authoritativeSettingsHealth,
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
    if (mode !== "consumer") dashboardView = "dashboard";
    writeDashboardMode(browserStorage(), mode);
  }

  async function openHistory(event: MouseEvent): Promise<void> {
    dashboardScrollY = window.scrollY;
    historyMounted = true;
    dashboardView = "history";
    await tick();
    window.scrollTo({ top: historyScrollY, behavior: "auto" });
    if (historyActivationUsesKeyboard(event.detail)) {
      document.getElementById("history-back-button")?.focus({ preventScroll: true });
    }
  }

  async function returnFromHistory(restoreFocus: boolean): Promise<void> {
    historyScrollY = window.scrollY;
    dashboardView = "dashboard";
    await tick();
    window.scrollTo({ top: dashboardScrollY, behavior: "auto" });
    if (restoreFocus) {
      document.getElementById("view-history-trigger")?.focus({ preventScroll: true });
    }
  }

  function setDayStartHour(hour: number): void {
    dayStartHour = hour;
    historyMounted = false;
    historyScrollY = 0;
    writeDayStartHour(browserStorage(), hour);
  }

  async function refresh() {
    if (
      refreshing ||
      reminderActionPending ||
      activityRecoveryPending ||
      breakRecoveryPending ||
      settingsOperationPending
    ) return;
    refreshing = true;
    const reminderGeneration = reminderRefreshGeneration;
    const activityGeneration = activityRefreshGeneration.capture();
    const breakGeneration = breakRefreshGeneration.capture();
    const settingsGeneration = settingsRefreshGeneration.capture();
    const [diagnostics, reminder, activity, breaks] = await Promise.all([
      requestDiagnostics(),
      settleLatestRequest(reminderRequests, () =>
        invoke<ReminderStatus>("get_reminder_status")
      ),
      settleLatestRequest(activityRequests, () =>
        invoke<LocalSnapshot<TodayActivity>>("get_today_activity")
      ),
      settleLatestRequest(breakRequests, () =>
        invoke<LocalSnapshot<BreakSummary>>("get_break_summary")
      )
    ]);

    const diagnosticsCurrent =
      diagnostics.latest &&
      activityRefreshGeneration.isCurrent(activityGeneration) &&
      breakRefreshGeneration.isCurrent(breakGeneration) &&
      settingsRefreshGeneration.isCurrent(settingsGeneration);
    publishDiagnostics(diagnostics, diagnosticsCurrent);

    const reminderCurrent =
      reminder.latest && reminderGeneration === reminderRefreshGeneration;
    if (reminderCurrent && reminder.settled.status === "fulfilled") {
      if (
        reminderStatus &&
        reminder.settled.value.stateRevision !== reminderStatus.stateRevision
      ) {
        reminderActionResult = null;
      }
      reminderStatus = reminder.settled.value;
      reminderStatusError = null;
      reminderActionError = null;
    } else if (reminderCurrent && reminder.settled.status === "rejected") {
      reminderStatusError = errorMessage(reminder.settled.reason);
    }

    const activityCurrent =
      activity.latest &&
      activityRefreshGeneration.isCurrent(activityGeneration) &&
      activityRecoveryPending === null;
    if (activityCurrent && activity.settled.status === "fulfilled") {
      activityResource = applyReflectionSnapshot(
        activity.settled.value,
        "Local activity storage is unavailable."
      );
      activityRecoveryError = recoveryErrorAfterReflectionPoll(
        activityRecoveryError,
        activityResource
      );
    } else if (activityCurrent && activity.settled.status === "rejected") {
      activityResource = applyReflectionFailure(
        activityResource,
        errorMessage(activity.settled.reason)
      );
    }

    const breakCurrent =
      breaks.latest &&
      breakRefreshGeneration.isCurrent(breakGeneration) &&
      breakRecoveryPending === null;
    if (breakCurrent && breaks.settled.status === "fulfilled") {
      breakResource = applyReflectionSnapshot(
        breaks.settled.value,
        "Local break storage is unavailable."
      );
      breakRecoveryError = recoveryErrorAfterReflectionPoll(
        breakRecoveryError,
        breakResource
      );
    } else if (breakCurrent && breaks.settled.status === "rejected") {
      breakResource = applyReflectionFailure(
        breakResource,
        errorMessage(breaks.settled.reason)
      );
    }
    refreshing = false;
  }

  async function recoverActivity(action: "retry" | "startNew") {
    if (activityRecoveryPending) return;
    activityRecoveryPending = action;
    activityRecoveryError = null;
    activityRefreshGeneration.invalidate();
    try {
      const command = await settleLatestRequest(activityRequests, () =>
        invoke<StorageLoadHealth>(
          action === "retry" ? "retry_activity_history" : "start_new_activity_history"
        )
      );
      if (!command.latest || command.settled.status === "rejected") {
        activityRecoveryError = "operation";
        return;
      }

      // Command health is provisional. Keep the unavailable surface and pending
      // text until the canonical snapshot below resolves.
      activityRefreshGeneration.invalidate();
      const snapshotGeneration = activityRefreshGeneration.capture();
      const diagnosticsBreakGeneration = breakRefreshGeneration.capture();
      const diagnosticsSettingsGeneration = settingsRefreshGeneration.capture();
      const [snapshot, diagnostics] = await Promise.all([
        settleLatestRequest(activityRequests, () =>
          invoke<LocalSnapshot<TodayActivity>>("get_today_activity")
        ),
        requestDiagnostics()
      ]);

      const snapshotCurrent =
        snapshot.latest && activityRefreshGeneration.isCurrent(snapshotGeneration);
      if (snapshotCurrent && snapshot.settled.status === "fulfilled") {
        const transition = applyReflectionRecoverySnapshot(
          activityResource,
          snapshot.settled.value,
          "Local activity storage is unavailable."
        );
        activityResource = transition.resource;
        activityRecoveryError = transition.error;
      } else if (snapshotCurrent && snapshot.settled.status === "rejected") {
        const transition = applyReflectionRecoveryRejection(
          activityResource,
          errorMessage(snapshot.settled.reason)
        );
        activityResource = transition.resource;
        activityRecoveryError = transition.error;
      }

      const diagnosticsCurrent =
        diagnostics.latest &&
        breakRefreshGeneration.isCurrent(diagnosticsBreakGeneration) &&
        settingsRefreshGeneration.isCurrent(diagnosticsSettingsGeneration);
      publishDiagnostics(diagnostics, diagnosticsCurrent);
    } finally {
      activityRefreshGeneration.invalidate();
      activityRecoveryPending = null;
    }
  }

  async function recoverBreakHistory(action: "retry" | "startNew") {
    if (breakRecoveryPending) return;
    breakRecoveryPending = action;
    breakRecoveryError = null;
    breakRefreshGeneration.invalidate();
    try {
      const command = await settleLatestRequest(breakRequests, () =>
        invoke<StorageLoadHealth>(
          action === "retry" ? "retry_break_ledger" : "start_new_break_ledger"
        )
      );
      if (!command.latest || command.settled.status === "rejected") {
        breakRecoveryError = "operation";
        return;
      }

      breakRefreshGeneration.invalidate();
      const diagnosticsActivityGeneration = activityRefreshGeneration.capture();
      const snapshotGeneration = breakRefreshGeneration.capture();
      const diagnosticsSettingsGeneration = settingsRefreshGeneration.capture();
      const [snapshot, diagnostics] = await Promise.all([
        settleLatestRequest(breakRequests, () =>
          invoke<LocalSnapshot<BreakSummary>>("get_break_summary")
        ),
        requestDiagnostics()
      ]);

      const snapshotCurrent =
        snapshot.latest && breakRefreshGeneration.isCurrent(snapshotGeneration);
      if (snapshotCurrent && snapshot.settled.status === "fulfilled") {
        const transition = applyReflectionRecoverySnapshot(
          breakResource,
          snapshot.settled.value,
          "Local break storage is unavailable."
        );
        breakResource = transition.resource;
        breakRecoveryError = transition.error;
      } else if (snapshotCurrent && snapshot.settled.status === "rejected") {
        const transition = applyReflectionRecoveryRejection(
          breakResource,
          errorMessage(snapshot.settled.reason)
        );
        breakResource = transition.resource;
        breakRecoveryError = transition.error;
      }

      const diagnosticsCurrent =
        diagnostics.latest &&
        activityRefreshGeneration.isCurrent(diagnosticsActivityGeneration) &&
        settingsRefreshGeneration.isCurrent(diagnosticsSettingsGeneration);
      publishDiagnostics(diagnostics, diagnosticsCurrent);
    } finally {
      breakRefreshGeneration.invalidate();
      breakRecoveryPending = null;
    }
  }

  async function runOverlayTest() {
    if (
      !reminderCapabilityAvailable(
        reminderStatus,
        reminderStatusError,
        authoritativeSettingsHealth,
        "previewEnabled"
      )
    ) return;
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
    const capability =
      command === "take_break_now" ? "takeBreakEnabled" : "pauseActionEnabled";
    if (
      reminderActionPending ||
      settingsOperationPending ||
      !reminderCapabilityAvailable(
        reminderStatus,
        reminderStatusError,
        authoritativeSettingsHealth,
        capability
      )
    ) return;
    const operation = reminderOperationGate.begin("action");
    if (!operation) return;
    reminderActionPending = command;
    reminderRefreshGeneration += 1;
    reminderStatusError = null;
    reminderActionError = null;
    reminderActionResult = null;
    settingsResult = null;
    try {
      const result = await settleLatestRequest(reminderRequests, () =>
        invoke<ReminderStatus>(command)
      );
      reminderRefreshGeneration += 1;
      if (!result.latest) return;
      if (result.settled.status === "fulfilled") {
        reminderStatus = result.settled.value;
        reminderStatusError = null;
        reminderActionResult = reminderSuccessMessage(command);
      } else {
        reminderActionError = errorMessage(result.settled.reason);
      }
    } finally {
      reminderActionPending = null;
      reminderOperationGate.finish(operation);
    }
  }

  function runPauseAction() {
    if (!reminderStatus) return;
    void runReminderAction(pauseActionCommand(reminderStatus));
  }

  function clearSettings(): void {
    savedSettings = null;
    workMinutesInput = "";
    breakSecondsInput = "";
    syncAcrossDevices = false;
    gridOffsetMinutes = 0;
    preBreakCueEnabled = false;
    timingEditorExpanded = false;
  }

  function useSettings(settings: ReminderSettings) {
    savedSettings = settings;
    workMinutesInput = String(settings.workMinutes);
    breakSecondsInput = String(settings.breakSeconds);
    syncAcrossDevices = settings.syncAcrossDevices;
    gridOffsetMinutes = settings.gridOffsetMinutes;
    preBreakCueEnabled = settings.preBreakCueEnabled;
  }

  function finishSettingsUpdate(result: Exclude<SettingsResult, null>): void {
    settingsResult = result;
    timingEditorExpanded = false;
    void tick().then(() => {
      const trigger = document.getElementById("timing-editor-trigger");
      trigger?.focus({ preventScroll: true });
      window.requestAnimationFrame(() => {
        if (trigger?.isConnected) {
          trigger.scrollIntoView({ block: "center", behavior: "auto" });
        }
      });
    });
  }

  function toggleTimingEditor(): void {
    if (settingsUnavailable) return;
    const expanding = !timingEditorExpanded;
    timingEditorExpanded = expanding;
    if (expanding) settingsResult = null;
  }

  function toggleSyncAcrossDevices(enabled: boolean): void {
    settingsResult = null;
    syncAcrossDevices = enabled;
    if (enabled) {
      gridOffsetMinutes = deviceGridOffsetMinutes(new Date());
    }
  }

  function togglePreBreakCue(enabled: boolean): void {
    settingsResult = null;
    preBreakCueEnabled = enabled;
  }

  function applySettingsView(view: ReminderSettingsView): void {
    settingsStorageHealth = view.loadHealth;
    if (view.loadHealth.status === "available" && view.data !== null) {
      useSettings(view.data);
    } else {
      clearSettings();
    }
  }

  async function loadReminderSettings() {
    settingsLoading = true;
    settingsError = null;
    settingsErrorContext = null;
    const generation = settingsRefreshGeneration.capture();
    const request = await settleLatestRequest(settingsRequests, () =>
      loadAuthoritativeReminderSettings(() =>
        invoke<ReminderSettingsView>("get_reminder_settings")
      )
    );
    if (!request.latest || !settingsRefreshGeneration.isCurrent(generation)) return;

    const result = request.settled;
    if (result.status === "rejected") {
      clearSettings();
      settingsStorageHealth = null;
      settingsError = `Could not load reminder settings: ${errorMessage(result.reason)}`;
      settingsErrorContext = "load";
    } else if (result.value.outcome === "rejected") {
      clearSettings();
      settingsStorageHealth = null;
      settingsError = `Could not load reminder settings: ${errorMessage(result.value.error)}`;
      settingsErrorContext = "load";
    } else {
      applySettingsView(result.value.view);
    }
    settingsLoading = false;
  }

  async function refreshAfterSettingsRecovery(
    preserveSettingsError = false
  ): Promise<ReminderSettingsSnapshotFollowUp> {
    settingsRefreshGeneration.invalidate();
    const settingsGeneration = settingsRefreshGeneration.capture();
    const reminderGeneration = ++reminderRefreshGeneration;
    const activityGeneration = activityRefreshGeneration.capture();
    const breakGeneration = breakRefreshGeneration.capture();
    const [settingsRequest, reminder, diagnostics] = await Promise.all([
      settleLatestRequest(settingsRequests, () =>
        loadAuthoritativeReminderSettings(() =>
          invoke<ReminderSettingsView>("get_reminder_settings")
        )
      ),
      settleLatestRequest(reminderRequests, () =>
        invoke<ReminderStatus>("get_reminder_status")
      ),
      requestDiagnostics()
    ]);

    const reminderCurrent =
      reminder.latest && reminderGeneration === reminderRefreshGeneration;
    if (reminderCurrent && reminder.settled.status === "fulfilled") {
      reminderStatus = reminder.settled.value;
      reminderStatusError = null;
      reminderActionError = null;
    } else if (reminderCurrent && reminder.settled.status === "rejected") {
      reminderStatusError = errorMessage(reminder.settled.reason);
    }

    const diagnosticsCurrent =
      diagnostics.latest &&
      settingsRefreshGeneration.isCurrent(settingsGeneration) &&
      activityRefreshGeneration.isCurrent(activityGeneration) &&
      breakRefreshGeneration.isCurrent(breakGeneration);
    publishDiagnostics(diagnostics, diagnosticsCurrent);

    if (
      !settingsRequest.latest ||
      !settingsRefreshGeneration.isCurrent(settingsGeneration)
    ) {
      return {
        outcome: "rejected",
        error: new Error("A newer settings refresh took precedence")
      };
    }
    if (settingsRequest.settled.status === "rejected") {
      return { outcome: "rejected", error: settingsRequest.settled.reason };
    }

    const settings = settingsRequest.settled.value;
    if (settings.outcome !== "rejected") {
      applySettingsView(settings.view);
      if (!preserveSettingsError) {
        settingsError = null;
        settingsErrorContext = null;
      }
    } else if (!preserveSettingsError) {
      settingsError = `Could not refresh reminder settings: ${errorMessage(settings.error)}`;
      settingsErrorContext = "load";
    }
    return settings;
  }

  async function retryReminderSettings() {
    if (
      settingsOperationPending ||
      reminderActionPending ||
      !settingsRecovery.canRetry
    ) return;
    const operation = reminderOperationGate.begin("settings");
    if (!operation) return;
    settingsOperationPending = "retry";
    settingsRecoveryError = false;
    settingsError = null;
    settingsErrorContext = null;
    settingsRefreshGeneration.invalidate();
    try {
      const command = await settleLatestRequest(settingsRequests, () =>
        invoke<StorageLoadHealth>("retry_reminder_settings")
      );
      if (!command.latest || command.settled.status === "rejected") {
        const error =
          command.settled.status === "rejected"
            ? errorMessage(command.settled.reason)
            : "A newer settings request took precedence";
        settingsError = `Could not retry reminder settings: ${error}`;
        settingsErrorContext = "load";
        const followUp = await refreshAfterSettingsRecovery(true);
        const recovered = followUp.outcome === "confirmed";
        settingsRecoveryError = !recovered;
        if (recovered) {
          settingsError = null;
          settingsErrorContext = null;
        }
      } else {
        // Command health is provisional. The typed recovery surface remains
        // authoritative until the grouped canonical follow-ups settle.
        settingsRecoveryError =
          (await refreshAfterSettingsRecovery()).outcome !== "confirmed";
      }
    } finally {
      settingsOperationPending = null;
      reminderOperationGate.finish(operation);
    }
  }

  async function saveReminderSettings() {
    const settings = settingsValidation.settings;
    if (
      !settings ||
      settingsLoading ||
      settingsOperationPending ||
      reminderActionPending ||
      settingsUnavailable
    ) return;
    const operation = reminderOperationGate.begin("settings");
    if (!operation) return;

    settingsOperationPending = "save";
    settingsError = null;
    settingsErrorContext = null;
    settingsResult = null;
    try {
      const command = await settleLatestRequest(settingsRequests, () =>
        invoke<ReminderSettings>("save_reminder_settings", { settings })
      );
      const resolution = await resolveReminderSettingsSave(
        settings,
        command,
        () => refreshAfterSettingsRecovery(true)
      );
      if (resolution.outcome === "saved") {
        useSettings(resolution.settings);
        settingsStorageHealth = { status: "available", recovery: "none" };
        settingsRecoveryError = false;
        settingsError = null;
        settingsErrorContext = null;
        finishSettingsUpdate("saved");
      } else if (resolution.outcome === "reloaded") {
        useSettings(resolution.settings);
        settingsRecoveryError = false;
        settingsError =
          command.latest && command.settled.status === "rejected"
            ? `Save response rejected: ${errorMessage(command.settled.reason)}. Canonical timing differs from the request.`
            : "Save response was superseded. Canonical timing differs from the request.";
        settingsErrorContext = "saveReloaded";
        timingEditorExpanded = true;
      } else {
        if (resolution.outcome === "unconfirmed") {
          clearSettings();
          settingsStorageHealth = null;
        }
        const commandDetail =
          command.latest && command.settled.status === "rejected"
            ? errorMessage(command.settled.reason)
            : "save response was superseded";
        const followUpDetail =
          resolution.outcome === "unconfirmed"
            ? errorMessage(resolution.error)
            : `canonical storage remained ${resolution.view.loadHealth.status}`;
        settingsRecoveryError = true;
        settingsError = `Could not confirm reminder settings save (${commandDetail}); follow-up: ${followUpDetail}`;
        settingsErrorContext = "saveUnconfirmed";
        timingEditorExpanded = false;
      }
    } finally {
      settingsOperationPending = null;
      reminderOperationGate.finish(operation);
    }
  }

  async function resetReminderSettings() {
    if (settingsLoading || settingsOperationPending || reminderActionPending) return;

    const recovering = settingsRecovery.canRestoreDefaults;
    if (settingsUnavailable && !recovering) return;
    const operation = reminderOperationGate.begin("settings");
    if (!operation) return;
    settingsOperationPending = "reset";
    settingsError = null;
    settingsErrorContext = null;
    settingsResult = null;
    try {
      const result = await settleLatestRequest(settingsRequests, () =>
        invoke<ReminderSettings>("reset_reminder_settings")
      );
      if (!result.latest || result.settled.status === "rejected") {
        const error =
          result.settled.status === "rejected"
            ? errorMessage(result.settled.reason)
            : "A newer settings request took precedence";
        settingsError = `Could not reset reminder settings: ${error}`;
        settingsErrorContext = "reset";
        const followUp = await refreshAfterSettingsRecovery(true);
        const refreshed = followUp.outcome === "confirmed";
        settingsRecoveryError = recovering && !refreshed;
        if (recovering && refreshed) {
          settingsError = null;
          settingsErrorContext = null;
        }
        timingEditorExpanded = !recovering && !settingsUnavailable;
      } else if (recovering) {
        // The command result cannot expose editable fields. Keep confirmed
        // unavailable health until the canonical settings envelope arrives.
        settingsRecoveryError =
          (await refreshAfterSettingsRecovery()).outcome !== "confirmed";
      } else {
        useSettings(result.settled.value);
        settingsErrorContext = null;
        finishSettingsUpdate("reset");
      }
    } finally {
      settingsOperationPending = null;
      reminderOperationGate.finish(operation);
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

  async function openAuthorWebsite() {
    authorWebsiteError = false;
    try {
      await invoke("open_author_website");
    } catch {
      authorWebsiteError = true;
    }
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
    dayStartHour = readDayStartHour(browserStorage());
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

{#if cueParameters}
  <PreBreakCue deadlineMs={cueParameters.deadlineMs} />
{:else if windowRoute.kind === "invalid-cue"}
  <main class="invalid-cue" aria-hidden="true"></main>
{:else if overlayParameters}
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
    {activityResource}
    {breakResource}
    {diagnosticsError}
    {diagnosticsCurrentSuccessful}
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
    {savedSettings}
    settingsStorageHealth={authoritativeSettingsHealth}
    {settingsRecoveryPending}
    {settingsOperationPending}
    {settingsRecoveryError}
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
    onRetrySettings={() => void retryReminderSettings()}
  />
{:else}
  <div class="view-shell" hidden={dashboardView !== "dashboard"}>
    <ConsumerDashboard
      presentation={reminderPresentation}
      {warning}
      {reminderStatus}
      {reminderStatusError}
      {reminderActionPending}
      {reminderActionResult}
      {overlayRunning}
      diagnosticsReady={report !== null}
      activityRefresh={activityResource.refresh}
      activityStorageHealth={activityResource.storageHealth}
      {activityRecoveryPending}
      {activityRecoveryError}
      breakRefresh={breakResource.refresh}
      breakStorageHealth={breakResource.storageHealth}
      {breakRecoveryPending}
      {breakRecoveryError}
      {dayStartHour}
      onDayStartChange={setDayStartHour}
      {savedSettings}
      settingsStorageHealth={authoritativeSettingsHealth}
      {settingsRecoveryPending}
      {settingsOperationPending}
      {settingsRecoveryError}
      {timingEditorExpanded}
      {workMinutesInput}
      {breakSecondsInput}
      {syncAcrossDevices}
      {gridOffsetMinutes}
      {preBreakCueEnabled}
      {preBreakCueAvailable}
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
      onToggleTimingEditor={toggleTimingEditor}
      onWorkMinutesInput={updateWorkMinutes}
      onBreakSecondsInput={updateBreakSeconds}
      onToggleSync={toggleSyncAcrossDevices}
      onTogglePreBreakCue={togglePreBreakCue}
      onSaveSettings={() => void saveReminderSettings()}
      onResetSettings={() => void resetReminderSettings()}
      onRetrySettings={() => void retryReminderSettings()}
      onOpenDeveloperMode={() => setDashboardMode("developer")}
      {authorWebsiteError}
      onOpenAuthorWebsite={() => void openAuthorWebsite()}
      onViewHistory={openHistory}
      onRetryActivity={() => void recoverActivity("retry")}
      onStartNewActivity={() => void recoverActivity("startNew")}
      onRetryBreakHistory={() => void recoverBreakHistory("retry")}
      onStartNewBreakHistory={() => void recoverBreakHistory("startNew")}
    />
  </div>
  {#if historyMounted}
    <div class="view-shell" hidden={dashboardView !== "history"}>
      <HistoryView
        {dayStartHour}
        active={dashboardView === "history"}
        onBack={(restoreFocus) => void returnFromHistory(restoreFocus)}
      />
    </div>
  {/if}
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
    --accent-hover: #98e5aa;
    --away: #7a93a8;
    --warn: #d9b573;

    --s1: 4px;
    --s2: 8px;
    --s3: 12px;
    --s4: 16px;
    --s5: 24px;
    --s6: 32px;

    --r-control: 8px;
    --r-button: 999px;

    --sans: ui-sans-serif, system-ui, -apple-system, "Segoe UI", Roboto,
      "Helvetica Neue", sans-serif;
    --display: "Newsreader", Georgia, serif;
    --mono: ui-monospace, "SF Mono", Menlo, Consolas, monospace;
  }

  @media (prefers-contrast: more) {
    :global(:root) {
      --ink-2: #d3ded6;
      --ink-3: #bccbc2;
      --line: #58665e;
      --line-2: #7d8d84;
    }
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

  :global(html.cue-window),
  :global(html.cue-window body) {
    min-width: 0;
    min-height: 0;
    overflow: hidden;
    background: transparent;
  }

  .invalid-cue {
    width: 100vw;
    height: 100vh;
    background: transparent;
    pointer-events: none;
  }

  .view-shell {
    display: contents;
  }

  .view-shell[hidden] {
    display: none;
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
    font-weight: 600;
    letter-spacing: 0.17em;
    text-transform: uppercase;
  }

  .invalid-overlay .shortcut-hint {
    color: #9ca69e;
    font-size: 0.75rem;
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
    font-family: var(--mono);
    overflow-wrap: anywhere;
  }

  .invalid-overlay button {
    border: 1px solid #3a473d;
    border-radius: 9px;
    padding: 11px 16px;
    color: #dce7de;
    background: #18201a;
    font: inherit;
    font-weight: 600;
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
