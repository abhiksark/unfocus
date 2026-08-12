<script lang="ts">
  import {
    focusProgress,
    type ConsumerReminderPresentation,
    type ConsumerWarning
  } from "$lib/consumer-dashboard";
  import {
    MAX_BREAK_SECONDS,
    MAX_WORK_MINUTES,
    MIN_BREAK_SECONDS,
    MIN_WORK_MINUTES,
    type ReminderSettings,
    type ReminderSettingsValidation
  } from "$lib/reminder-settings";
  import type { ReminderActionCommand, ReminderStatus } from "$lib/reminder-status";
  import {
    breakErrorCaption,
    breakLoadingCaption,
    breakOutcomeStats,
    breakSummaryCaption,
    isBreakDayEmpty,
    weekBreakCaption,
    type BreakSummary
  } from "$lib/break-summary";
  import {
    activityFootnote,
    currentKindLabel,
    deepBlockCaption,
    formatActivityDuration,
    isActivityWindowEmpty,
    stripActiveHeight,
    stripAfkHeight,
    stripAriaLabel,
    todayErrorCaption,
    todayLoadingCaption,
    type TodayActivity
  } from "$lib/today-activity";

  type SettingsResult = "saved" | "reset" | null;

  type Props = {
    presentation: ConsumerReminderPresentation;
    warning: ConsumerWarning | null;
    reminderStatus: ReminderStatus | null;
    reminderActionPending: ReminderActionCommand | null;
    reminderActionResult: string | null;
    overlayRunning: boolean;
    diagnosticsReady: boolean;
    todayActivity: TodayActivity | null;
    todayActivityError: string | null;
    breakSummary: BreakSummary | null;
    breakSummaryError: string | null;
    savedSettings: ReminderSettings | null;
    timingEditorExpanded: boolean;
    workMinutesInput: string;
    breakSecondsInput: string;
    settingsLoading: boolean;
    settingsSaving: boolean;
    settingsValidation: ReminderSettingsValidation;
    workMinutesError: string | null;
    breakSecondsError: string | null;
    settingsError: string | null;
    settingsErrorContext: "load" | "save" | "reset" | null;
    settingsResult: SettingsResult;
    onTakeBreak: () => void;
    onPauseAction: () => void;
    onPreview: () => void;
    onToggleTimingEditor: () => void;
    onWorkMinutesInput: (value: string) => void;
    onBreakSecondsInput: (value: string) => void;
    onSaveSettings: () => void;
    onResetSettings: () => void;
    onOpenDeveloperMode: () => void;
  };

  let {
    presentation,
    warning,
    reminderStatus,
    reminderActionPending,
    reminderActionResult,
    overlayRunning,
    diagnosticsReady,
    todayActivity,
    todayActivityError,
    breakSummary,
    breakSummaryError,
    savedSettings,
    timingEditorExpanded,
    workMinutesInput,
    breakSecondsInput,
    settingsLoading,
    settingsSaving,
    settingsValidation,
    workMinutesError,
    breakSecondsError,
    settingsError,
    settingsErrorContext,
    settingsResult,
    onTakeBreak,
    onPauseAction,
    onPreview,
    onToggleTimingEditor,
    onWorkMinutesInput,
    onBreakSecondsInput,
    onSaveSettings,
    onResetSettings,
    onOpenDeveloperMode
  }: Props = $props();

  const progress = $derived(focusProgress(reminderStatus, savedSettings));
  const pauseLabel = $derived(
    reminderActionPending === "pause_reminders"
      ? "Pausing…"
      : reminderActionPending === "resume_reminders"
        ? "Resuming…"
        : (reminderStatus?.pauseActionLabel ?? "Pause for 30 minutes")
  );
  const previewDisabled = $derived(
    overlayRunning || !diagnosticsReady || !reminderStatus || !reminderStatus.previewEnabled
  );
  const previewLabel = $derived(
    overlayRunning
      ? "Opening…"
      : reminderStatus && !reminderStatus.previewEnabled
        ? "Break screen open"
        : "Preview break screen"
  );
  const rhythm = $derived(
    savedSettings
      ? `${savedSettings.workMinutes} min focus → ${savedSettings.breakSeconds} sec rest`
      : "Reading saved rhythm…"
  );
  const settingsConfirmation = $derived(
    settingsResult === "saved"
      ? "Timing saved."
      : settingsResult === "reset"
        ? "Default timing restored."
        : null
  );
  const hasReminderActions = $derived(
    presentation.showTakeBreak ||
      presentation.showPause ||
      presentation.showResume ||
      reminderActionResult !== null
  );
  const activityKind = $derived(
    todayActivity
      ? currentKindLabel(todayActivity.currentKind, todayActivity.probeAvailable)
      : todayActivityError
        ? "Your day unavailable"
        : "Gathering samples…"
  );
  const activityStripLabel = $derived(
    todayActivity ? stripAriaLabel(todayActivity) : "Activity strip loading"
  );
  const activityEmpty = $derived(
    todayActivity ? isActivityWindowEmpty(todayActivity) : false
  );
  const breakStats = $derived(breakSummary ? breakOutcomeStats(breakSummary) : []);
  const breakDayEmpty = $derived(breakSummary ? isBreakDayEmpty(breakSummary) : false);
  const breakCaption = $derived(
    breakSummary
      ? breakSummaryCaption(breakSummary)
      : breakSummaryError
        ? breakErrorCaption(breakSummaryError)
        : breakLoadingCaption()
  );
  const weekCaption = $derived(breakSummary ? weekBreakCaption(breakSummary) : "");
</script>

<main class="wrap">
  <header class="top">
    <span class="mark"><span class="dot" aria-hidden="true"></span>Unfocus</span>
  </header>

  <section class="state" aria-labelledby="consumer-state-title">
    <div class="state-copy" aria-live="polite" aria-atomic="true">
      <h1 id="consumer-state-title" class="t-display">{presentation.heading}</h1>
      <div class="meter">
        <p class="t-lead">{presentation.secondary}</p>
        {#if progress !== null}
          <div class="progress" aria-hidden="true">
            <i style={`width: ${Math.round(progress * 100)}%`}></i>
          </div>
        {/if}
      </div>
    </div>

    {#if hasReminderActions}
      <div class="cta">
        {#if presentation.showTakeBreak}
          <button
            class="btn-primary"
            type="button"
            onclick={onTakeBreak}
            disabled={!reminderStatus?.takeBreakEnabled || reminderActionPending !== null}
          >
            {reminderActionPending === "take_break_now" ? "Starting…" : "Take a break"}
          </button>
        {/if}
        {#if presentation.showPause || presentation.showResume}
          <button
            class={presentation.showResume ? "btn-primary" : "btn-ghost"}
            type="button"
            onclick={onPauseAction}
            disabled={!reminderStatus?.pauseActionEnabled || reminderActionPending !== null}
          >
            {pauseLabel}
          </button>
        {/if}
        <div class="action-feedback t-micro" aria-live="polite">
          {reminderActionResult ?? ""}
        </div>
      </div>
    {/if}
  </section>

  <hr class="rule" />

  <section class="section" aria-labelledby="today-title">
    <div class="section-head">
      <h2 id="today-title" class="t-title">Your day</h2>
      <p class="t-micro">{todayActivity?.windowLabel ?? "Last 24 hours"} · {activityKind}</p>
    </div>

    {#if todayActivity}
      <div class="stats" role="group" aria-label="Activity totals for the rolling window">
        <div class="stat" class:is-zero={todayActivity.activeSeconds <= 0}>
          <span class="num">{formatActivityDuration(todayActivity.activeSeconds)}</span>
          <span class="t-micro">Active</span>
        </div>
        <div class="stat" class:is-zero={todayActivity.afkSeconds <= 0}>
          <span class="num">{formatActivityDuration(todayActivity.afkSeconds)}</span>
          <span class="t-micro">Away</span>
        </div>
        <div class="stat" class:is-zero={todayActivity.longestActiveSeconds <= 0}>
          <span class="num">{formatActivityDuration(todayActivity.longestActiveSeconds)}</span>
          <span class="t-micro">Longest stretch</span>
        </div>
        <div class="stat" class:is-zero={todayActivity.deepBlockCount <= 0}>
          <span class="num">{todayActivity.deepBlockCount}</span>
          <span class="t-micro">Deep work</span>
          <span class="t-micro"
            >{deepBlockCaption(
              todayActivity.deepBlockCount,
              todayActivity.deepBlockMinSeconds
            )}</span
          >
        </div>
      </div>

      <div class="strip" role="img" aria-label={activityStripLabel}>
        {#each todayActivity.strip as bucket, index (index)}
          <div class="strip-bucket" aria-hidden="true">
            <span
              class="strip-afk"
              style={`height: ${Math.round(stripAfkHeight(bucket) * 100)}%`}
            ></span>
            <span
              class="strip-active"
              style={`height: ${Math.round(stripActiveHeight(bucket) * 100)}%`}
            ></span>
          </div>
        {/each}
      </div>
      <ul class="legend" aria-hidden="true">
        <li><span class="legend-swatch legend-active"></span> Active</li>
        <li><span class="legend-swatch legend-afk"></span> Away</li>
      </ul>

      {#if activityEmpty}
        <p class="t-micro" role="status">
          No active or away time classified in this window yet.
        </p>
      {/if}
      <p class="t-micro">{activityFootnote(todayActivity.afkThresholdSeconds)}</p>
    {:else if todayActivityError}
      <p class="t-micro is-error" role="status">{todayErrorCaption(todayActivityError)}</p>
    {:else}
      <p class="t-micro" role="status">{todayLoadingCaption()}</p>
    {/if}
  </section>

  <hr class="rule" />

  <section class="section" aria-labelledby="break-history-title">
    <div class="section-head">
      <h2 id="break-history-title" class="t-title">Breaks</h2>
      {#if breakSummary}
        <p class="t-micro">{breakSummary.windowLabel}</p>
      {/if}
    </div>

    {#if breakSummary}
      <div
        class="stats"
        class:is-empty={breakDayEmpty}
        role="group"
        aria-label="Break outcome counts for the last day"
      >
        {#each breakStats as stat (stat.kind)}
          <div class="stat" class:is-zero={stat.count <= 0} title={stat.hint}>
            <span class="num" aria-label={`${stat.count} ${stat.label.toLowerCase()}. ${stat.hint}`}
              >{stat.count}</span
            >
            <span class="t-micro">{stat.label}</span>
          </div>
        {/each}
      </div>
      <p class="t-micro">{breakCaption}</p>
      <p class="t-micro">{weekCaption}</p>
    {:else if breakSummaryError}
      <p class="t-micro is-error" role="status">{breakErrorCaption(breakSummaryError)}</p>
    {:else}
      <p class="t-micro" role="status">{breakLoadingCaption()}</p>
    {/if}
  </section>

  <hr class="rule" />

  <section class="rhythm" aria-labelledby="rhythm-title">
    <div>
      <p class="section-label">Your rhythm</p>
      <h2 id="rhythm-title">{rhythm}</h2>
    </div>
    <button
      class="text-button"
      type="button"
      aria-expanded={timingEditorExpanded}
      aria-controls="timing-editor"
      onclick={onToggleTimingEditor}
      disabled={settingsLoading}
    >
      {timingEditorExpanded ? "Close" : "Edit"}
    </button>

    {#if timingEditorExpanded}
      <div id="timing-editor" class="timing-editor">
        <form
          novalidate
          onsubmit={(event) => {
            event.preventDefault();
            onSaveSettings();
          }}
        >
          <div class="duration-fields">
            <div class="duration-field">
              <label for="consumer-work-duration">Focus duration</label>
              <div class="duration-input" class:invalid={workMinutesError}>
                <input
                  id="consumer-work-duration"
                  type="text"
                  inputmode="numeric"
                  pattern="[0-9]*"
                  autocomplete="off"
                  value={workMinutesInput}
                  disabled={settingsLoading || settingsSaving}
                  aria-invalid={workMinutesError ? "true" : "false"}
                  aria-describedby={workMinutesError
                    ? "consumer-work-help consumer-work-error"
                    : "consumer-work-help"}
                  oninput={(event) =>
                    onWorkMinutesInput((event.currentTarget as HTMLInputElement).value)}
                />
                <span>minutes</span>
              </div>
              <small id="consumer-work-help">{MIN_WORK_MINUTES}–{MAX_WORK_MINUTES} whole minutes</small>
              {#if workMinutesError}
                <small id="consumer-work-error" class="field-error">{workMinutesError}</small>
              {/if}
            </div>

            <div class="duration-field">
              <label for="consumer-break-duration">Rest duration</label>
              <div class="duration-input" class:invalid={breakSecondsError}>
                <input
                  id="consumer-break-duration"
                  type="text"
                  inputmode="numeric"
                  pattern="[0-9]*"
                  autocomplete="off"
                  value={breakSecondsInput}
                  disabled={settingsLoading || settingsSaving}
                  aria-invalid={breakSecondsError ? "true" : "false"}
                  aria-describedby={breakSecondsError
                    ? "consumer-break-help consumer-break-error"
                    : "consumer-break-help"}
                  oninput={(event) =>
                    onBreakSecondsInput((event.currentTarget as HTMLInputElement).value)}
                />
                <span>seconds</span>
              </div>
              <small id="consumer-break-help">{MIN_BREAK_SECONDS}–{MAX_BREAK_SECONDS} whole seconds</small>
              {#if breakSecondsError}
                <small id="consumer-break-error" class="field-error">{breakSecondsError}</small>
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
              onclick={onResetSettings}
              disabled={settingsLoading || settingsSaving}
            >
              Reset to defaults
            </button>
          </div>

          {#if settingsError}
            <p class="form-error" role="alert">
              {settingsErrorContext === "load"
                ? "We couldn’t load your saved timing."
                : settingsErrorContext === "reset"
                  ? "We couldn’t restore the default timing. Your previous rhythm was retained."
                  : "We couldn’t save this timing. Your previous rhythm was retained."}
            </p>
          {/if}
        </form>

        <details class="advanced">
          <summary>Advanced</summary>
          <p>Open technical health details and native probe controls.</p>
          <button class="secondary" type="button" onclick={onOpenDeveloperMode}>
            Open developer mode
          </button>
        </details>
      </div>
    {/if}
    <div class="settings-confirmation" aria-live="polite">
      {settingsConfirmation ?? ""}
    </div>
  </section>

  <section class="preview" aria-labelledby="preview-title">
    <div>
      <p class="section-label">Break screen</p>
      <h2 id="preview-title">Eight-second preview</h2>
    </div>
    <button class="secondary" type="button" onclick={onPreview} disabled={previewDisabled}>
      {previewLabel}
    </button>
  </section>

  {#if warning}
    <section class="warning" role="status" aria-labelledby="warning-title">
      <div>
        <p class="section-label">Needs attention</p>
        <h2 id="warning-title">{warning.heading}</h2>
        <p>{warning.message}</p>
      </div>
      <button class="text-button" type="button" onclick={onOpenDeveloperMode}>
        View details
      </button>
    </section>
  {/if}
</main>

<style>
  .wrap {
    display: flex;
    width: min(100%, 780px);
    min-height: 100vh;
    flex-direction: column;
    gap: var(--s5);
    margin: 0 auto;
    padding: var(--s5) var(--s6) var(--s7);
  }

  .top {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--s4);
  }

  .mark {
    display: inline-flex;
    align-items: center;
    gap: var(--s2);
    color: var(--ink-2);
    font-size: 0.78rem;
    font-weight: 600;
  }

  .dot {
    width: 7px;
    height: 7px;
    border-radius: 50%;
    background: var(--accent);
  }

  .rule {
    height: 1px;
    margin: 0;
    border: 0;
    background: var(--line);
  }

  .state,
  .state-copy {
    display: flex;
    flex-direction: column;
    gap: var(--s5);
  }

  .meter {
    display: flex;
    max-width: 46ch;
    flex-direction: column;
    gap: var(--s2);
  }

  .progress {
    overflow: hidden;
    height: 2px;
    border-radius: 2px;
    background: var(--line-2);
  }

  .progress i {
    display: block;
    height: 100%;
    border-radius: 2px;
    background: var(--accent);
    transition: width 400ms linear;
  }

  .cta {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s2);
  }

  .action-feedback {
    width: 100%;
    color: var(--ink-2);
  }

  .action-feedback:empty {
    display: none;
  }

  .t-display {
    max-width: 20ch;
    margin: 0;
    font-family: var(--serif);
    font-size: clamp(2rem, 4.4vw, 2.9rem);
    font-weight: 400;
    letter-spacing: -0.012em;
    line-height: 1.05;
  }

  .t-lead {
    margin: 0;
    color: var(--ink-2);
    font-size: 1.02rem;
    line-height: 1.5;
  }

  .t-micro {
    margin: 0;
    color: var(--ink-3);
    font-size: 0.74rem;
    line-height: 1.45;
  }

  button {
    border: 0;
    border-radius: var(--r-button);
    font: inherit;
    cursor: pointer;
  }

  button:focus-visible,
  summary:focus-visible {
    outline: 3px solid #d9efdf;
    outline-offset: 3px;
  }

  button:disabled {
    cursor: not-allowed;
    opacity: 0.52;
  }

  .btn-primary {
    padding: 11px 22px;
    color: var(--accent-ink);
    background: var(--accent);
    font-size: 0.9rem;
    font-weight: 600;
  }

  .btn-ghost {
    border: 1px solid var(--line-2);
    padding: 11px 18px;
    color: var(--ink-2);
    background: transparent;
    font-size: 0.9rem;
    font-weight: 500;
  }

  .btn-ghost:hover:not(:disabled) {
    border-color: #3b4b42;
    color: var(--ink);
  }

  .section-label {
    margin: 0 0 9px;
    color: #85d79b;
    font-size: 0.7rem;
    font-weight: 750;
    letter-spacing: 0.16em;
    text-transform: uppercase;
  }

  h1,
  h2,
  p {
    margin-top: 0;
  }

  .rhythm,
  .preview,
  .warning {
    border: 1px solid #27332b;
    border-radius: 15px;
    background: rgba(18, 25, 20, 0.92);
  }

  .t-title {
    margin: 0;
    font-size: 1.3rem;
    font-weight: 500;
    letter-spacing: -0.01em;
    line-height: 1.25;
  }

  .num {
    color: var(--ink);
    font-size: 1.45rem;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.02em;
  }

  .section {
    display: flex;
    flex-direction: column;
    gap: var(--s3);
  }

  .section-head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    justify-content: space-between;
    gap: var(--s1) var(--s4);
  }

  .stats {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s5) var(--s6);
  }

  .stats.is-empty {
    opacity: 0.72;
  }

  .stat {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 2px;
  }

  .stat.is-zero .num {
    color: var(--ink-3);
    font-weight: 400;
  }

  .strip {
    display: grid;
    height: 64px;
    align-items: end;
    gap: 2px;
    grid-template-columns: repeat(48, minmax(0, 1fr));
  }

  .strip-bucket {
    position: relative;
    height: 100%;
  }

  .strip-active,
  .strip-afk {
    position: absolute;
    right: 0;
    bottom: 0;
    left: 0;
    min-height: 0;
    border-radius: 2px 2px 0 0;
  }

  .strip-active {
    background: var(--accent);
    opacity: 0.85;
  }

  .strip-afk {
    background: var(--away);
    opacity: 0.7;
  }

  .legend {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s4);
    margin: 0;
    padding: 0;
    color: var(--ink-3);
    font-size: 0.74rem;
    list-style: none;
  }

  .legend li {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .legend-swatch {
    display: inline-block;
    width: 8px;
    height: 8px;
    border-radius: 2px;
  }

  .legend-active {
    background: var(--accent);
  }

  .legend-afk {
    background: var(--away);
  }

  .is-error {
    color: #ffc4c4;
  }

  .rhythm,
  .preview,
  .warning {
    display: grid;
    grid-template-columns: minmax(0, 1fr) auto;
    align-items: center;
    gap: 18px;
    padding: 15px 17px;
  }

  .rhythm {
    grid-column: 1 / -1;
    grid-row: 3;
  }

  .preview {
    grid-column: 2;
    grid-row: 1;
    grid-template-columns: 1fr;
    align-content: center;
    gap: 10px;
  }

  .preview button {
    width: 100%;
  }

  .rhythm h2,
  .preview h2,
  .warning h2 {
    margin-bottom: 0;
    font-size: 1.02rem;
    font-weight: 620;
    letter-spacing: -0.015em;
  }

  .timing-editor {
    grid-column: 1 / -1;
    border-top: 1px solid #2b372e;
    padding-top: 17px;
  }

  .duration-fields {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: 14px;
  }

  .duration-field label {
    display: block;
    margin-bottom: 7px;
    color: #e0eae2;
    font-size: 0.78rem;
    font-weight: 620;
  }

  .duration-input {
    display: flex;
    align-items: center;
    border: 1px solid #405044;
    border-radius: 9px;
    background: #0c130f;
  }

  .duration-input:focus-within {
    border-color: #8be0a2;
    box-shadow: 0 0 0 3px rgba(117, 211, 142, 0.18);
  }

  .duration-input.invalid {
    border-color: #c16a6a;
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
  }

  .duration-input span {
    padding-right: 11px;
    color: #9da8a0;
    font-size: 0.7rem;
  }

  .duration-field small {
    display: block;
    margin-top: 6px;
    color: #9da8a0;
    font-size: 0.68rem;
  }

  .duration-field .field-error,
  .form-error {
    color: #ffc4c4;
  }

  .settings-actions {
    display: flex;
    flex-wrap: wrap;
    gap: 8px;
    margin-top: 15px;
  }

  .form-error {
    margin: 10px 0 0;
    font-size: 0.75rem;
  }

  .advanced {
    margin-top: 17px;
    border-top: 1px solid #2b372e;
    padding-top: 14px;
    color: #b2beb5;
  }

  .advanced summary {
    width: fit-content;
    cursor: pointer;
    color: #dfe9e1;
    font-size: 0.78rem;
    font-weight: 650;
  }

  .advanced p {
    margin: 9px 0;
    font-size: 0.75rem;
  }

  .settings-confirmation {
    grid-column: 1 / -1;
    min-height: 0;
    color: #b9ddc2;
    font-size: 0.72rem;
  }

  .settings-confirmation:empty {
    display: none;
  }

  .warning {
    grid-column: 1 / -1;
    border-color: #78633a;
    background: #251f14;
  }

  .warning .section-label {
    color: #e2c487;
  }

  .warning p:not(.section-label) {
    max-width: 660px;
    margin: 8px 0 0;
    color: #d5c7a9;
    font-size: 0.8rem;
    line-height: 1.45;
  }

  .primary {
    color: #07120a;
    background: #80d997;
  }

  .primary:hover:not(:disabled) {
    background: #98e5aa;
  }

  .secondary {
    border: 1px solid #46564a;
    color: #e1eae3;
    background: #18211b;
  }

  .secondary:hover:not(:disabled) {
    border-color: #6c8171;
    background: #202c24;
  }

  .text-button {
    padding: 8px 4px;
    color: #a8dcb5;
    background: transparent;
    font-size: 0.78rem;
  }

  .text-button:hover:not(:disabled) {
    color: #d4efdb;
  }

  @media (max-width: 760px) {
    .rhythm,
    .preview,
    .warning {
      grid-column: 1;
      grid-row: auto;
    }

    .duration-fields {
      grid-template-columns: 1fr;
    }
  }

  @media (max-width: 520px) {
    .rhythm,
    .preview,
    .warning {
      grid-template-columns: 1fr;
    }

    .rhythm > .text-button,
    .warning > .text-button {
      width: fit-content;
    }
  }

  @media (prefers-contrast: more) {
    .rhythm,
    .preview,
    .warning {
      border-color: #829187;
    }
  }
</style>
