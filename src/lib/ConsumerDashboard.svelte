<script lang="ts">
  import scene from "$lib/scene.svg?raw";
  import type {
    ConsumerReminderPresentation,
    ConsumerWarning
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
    breakSummaryCaption,
    weekBreakCaption,
    type BreakSummary
  } from "$lib/break-summary";
  import {
    currentKindLabel,
    deepBlockCaption,
    formatActivityDuration,
    stripActiveHeight,
    stripAfkHeight,
    stripAriaLabel,
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
        ? "Activity unavailable"
        : "Gathering samples…"
  );
  const activityStripLabel = $derived(
    todayActivity ? stripAriaLabel(todayActivity) : "Activity strip loading"
  );
  const breakCaption = $derived(
    breakSummary
      ? breakSummaryCaption(breakSummary)
      : breakSummaryError
        ? "Break history unavailable"
        : "Reading break history…"
  );
  const weekCaption = $derived(breakSummary ? weekBreakCaption(breakSummary) : "");
</script>

<main class="consumer-dashboard">
  <section class="hero" aria-labelledby="consumer-state-title">
    <div class="hero-scene" aria-hidden="true">{@html scene}</div>
    <div class="hero-shade" aria-hidden="true"></div>
    <div class="hero-copy">
      <p class="eyebrow">Unfocus · local reminder</p>
      <div class="state-copy" aria-live="polite" aria-atomic="true">
        <h1 id="consumer-state-title">{presentation.heading}</h1>
        <p>{presentation.secondary}</p>
      </div>
    </div>
  </section>

  <div class="consumer-content" class:without-actions={!hasReminderActions}>
    {#if hasReminderActions}
      <section class="actions" aria-label="Reminder actions">
      {#if presentation.showTakeBreak}
        <button
          class="primary"
          type="button"
          onclick={onTakeBreak}
          disabled={!reminderStatus?.takeBreakEnabled || reminderActionPending !== null}
        >
          {reminderActionPending === "take_break_now" ? "Starting…" : "Take a break"}
        </button>
      {/if}
      {#if presentation.showPause}
        <button
          class="secondary"
          type="button"
          onclick={onPauseAction}
          disabled={!reminderStatus?.pauseActionEnabled || reminderActionPending !== null}
        >
          {reminderActionPending === "pause_reminders"
            ? "Pausing…"
            : "Pause for 30 minutes"}
        </button>
      {/if}
      {#if presentation.showResume}
        <button
          class="primary"
          type="button"
          onclick={onPauseAction}
          disabled={!reminderStatus?.pauseActionEnabled || reminderActionPending !== null}
        >
          {reminderActionPending === "resume_reminders" ? "Resuming…" : "Resume reminders"}
        </button>
      {/if}
        <div class="action-feedback" aria-live="polite">
          {reminderActionResult ?? ""}
        </div>
      </section>
    {/if}

    <section class="today" aria-labelledby="today-title">
      <div class="today-header">
        <div>
          <p class="section-label">Your day</p>
          <h2 id="today-title">{todayActivity?.windowLabel ?? "Last 24 hours"}</h2>
          <p class="today-status">{activityKind}</p>
        </div>
      </div>

      {#if todayActivity}
        <div class="today-stats" role="group" aria-label="Activity totals">
          <div>
            <span>Active</span>
            <strong>{formatActivityDuration(todayActivity.activeSeconds)}</strong>
          </div>
          <div>
            <span>Away</span>
            <strong>{formatActivityDuration(todayActivity.afkSeconds)}</strong>
          </div>
          <div>
            <span>Longest stretch</span>
            <strong>{formatActivityDuration(todayActivity.longestActiveSeconds)}</strong>
          </div>
          <div>
            <span>Deep work</span>
            <strong>{todayActivity.deepBlockCount}</strong>
            <small>{deepBlockCaption(todayActivity.deepBlockCount, todayActivity.deepBlockMinSeconds)}</small>
          </div>
        </div>

        <div
          class="today-strip"
          role="img"
          aria-label={activityStripLabel}
        >
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
        <p class="today-footnote">
          From keyboard and mouse presence only. Gaps under {formatActivityDuration(
            todayActivity.afkThresholdSeconds
          )} stay inside continuous work. Nothing is keylogged.
        </p>
      {:else if todayActivityError}
        <p class="today-error" role="status">{todayActivityError}</p>
      {:else}
        <p class="today-footnote">Collecting idle samples from this session…</p>
      {/if}

      <div class="break-history" aria-labelledby="break-history-title">
        <p class="section-label" id="break-history-title">Break outcomes</p>
        {#if breakSummary}
          <div class="break-counts" role="group" aria-label="Break outcome counts">
            <div>
              <span>Shown</span>
              <strong>{breakSummary.scheduledShown}</strong>
            </div>
            <div>
              <span>Natural</span>
              <strong>{breakSummary.naturalIdle}</strong>
            </div>
            <div>
              <span>Manual</span>
              <strong>{breakSummary.manualTakeBreak}</strong>
            </div>
            <div>
              <span>Fullscreen</span>
              <strong>{breakSummary.fullscreenSuppress}</strong>
            </div>
          </div>
          <p class="today-footnote">{breakCaption}</p>
          <p class="today-footnote">{weekCaption}</p>
        {:else if breakSummaryError}
          <p class="today-error" role="status">{breakSummaryError}</p>
        {:else}
          <p class="today-footnote">{breakCaption}</p>
        {/if}
      </div>
    </section>

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
  </div>
</main>

<style>
  .consumer-dashboard {
    width: min(100%, 920px);
    min-height: 100vh;
    margin: 0 auto;
    padding: 22px 28px 26px;
  }

  .hero {
    position: relative;
    min-height: 270px;
    overflow: hidden;
    border: 1px solid #294637;
    border-radius: 22px;
    background: #07120f;
    isolation: isolate;
  }

  .hero-scene,
  .hero-shade {
    position: absolute;
    inset: 0;
    width: 100%;
    height: 100%;
  }

  .hero-scene {
    overflow: hidden;
  }

  .hero-scene :global(.scene) {
    display: block;
    width: 100%;
    height: 100%;
  }

  .hero-shade {
    background: linear-gradient(90deg, rgba(4, 12, 9, 0.92) 0%, rgba(5, 14, 11, 0.74) 48%, rgba(5, 14, 11, 0.18) 82%);
  }

  .hero-copy {
    position: relative;
    z-index: 1;
    display: flex;
    min-height: 270px;
    max-width: 650px;
    flex-direction: column;
    justify-content: center;
    padding: 30px 38px;
  }

  .eyebrow,
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

  h1 {
    max-width: 620px;
    margin-bottom: 13px;
    font-family: "Fraunces", Georgia, serif;
    font-size: clamp(2.7rem, 7vw, 4.4rem);
    font-weight: 420;
    line-height: 0.98;
  }

  .state-copy p {
    margin-bottom: 0;
    color: #c7d5ca;
    font-size: clamp(1.05rem, 2.5vw, 1.28rem);
    line-height: 1.5;
  }

  .consumer-content {
    display: grid;
    grid-template-columns: minmax(0, 1fr) minmax(250px, 0.62fr);
    gap: 10px;
    margin-top: 12px;
  }

  .actions,
  .today,
  .rhythm,
  .preview,
  .warning {
    border: 1px solid #27332b;
    border-radius: 15px;
    background: rgba(18, 25, 20, 0.92);
  }

  .today {
    grid-column: 1 / -1;
    grid-row: 2;
    display: grid;
    gap: 14px;
    padding: 15px 17px 16px;
  }

  .today-header h2 {
    margin-bottom: 4px;
    font-size: 1.02rem;
    font-weight: 620;
    letter-spacing: -0.015em;
  }

  .today-status {
    margin: 0;
    color: #a8b7ac;
    font-size: 0.78rem;
  }

  .today-stats {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 10px;
  }

  .today-stats div {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 4px;
    padding: 10px 11px;
    border: 1px solid #2b372e;
    border-radius: 11px;
    background: #0f1612;
  }

  .today-stats span {
    color: #8f9d94;
    font-size: 0.68rem;
    font-weight: 650;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .today-stats strong {
    color: #e7f0e9;
    font-size: 1.05rem;
    font-weight: 650;
    letter-spacing: -0.02em;
  }

  .today-stats small {
    color: #8f9d94;
    font-size: 0.66rem;
    line-height: 1.35;
  }

  .today-strip {
    display: grid;
    grid-template-columns: repeat(48, minmax(0, 1fr));
    align-items: end;
    gap: 2px;
    height: 52px;
    padding: 8px 6px 4px;
    border: 1px solid #2b372e;
    border-radius: 11px;
    background:
      linear-gradient(180deg, rgba(17, 28, 22, 0.2), rgba(8, 12, 10, 0.55)),
      #0c130f;
  }

  .strip-bucket {
    position: relative;
    display: flex;
    height: 100%;
    align-items: flex-end;
    justify-content: center;
  }

  .strip-active,
  .strip-afk {
    position: absolute;
    bottom: 0;
    left: 0;
    right: 0;
    min-height: 0;
    border-radius: 2px 2px 0 0;
  }

  .strip-afk {
    background: rgba(120, 99, 58, 0.55);
  }

  .strip-active {
    background: #6fbf86;
  }

  .today-footnote,
  .today-error {
    margin: 0;
    color: #8f9d94;
    font-size: 0.7rem;
    line-height: 1.45;
  }

  .today-error {
    color: #ffc4c4;
  }

  .break-history {
    display: grid;
    gap: 10px;
    border-top: 1px solid #2b372e;
    padding-top: 14px;
  }

  .break-history .section-label {
    margin-bottom: 0;
  }

  .break-counts {
    display: grid;
    grid-template-columns: repeat(4, minmax(0, 1fr));
    gap: 8px;
  }

  .break-counts div {
    display: flex;
    min-width: 0;
    flex-direction: column;
    gap: 3px;
    padding: 8px 10px;
    border: 1px solid #2b372e;
    border-radius: 10px;
    background: #0f1612;
  }

  .break-counts span {
    color: #8f9d94;
    font-size: 0.66rem;
    font-weight: 650;
    letter-spacing: 0.04em;
    text-transform: uppercase;
  }

  .break-counts strong {
    color: #e7f0e9;
    font-size: 1rem;
    font-weight: 650;
  }

  .actions {
    display: flex;
    min-height: 76px;
    flex-wrap: wrap;
    align-items: center;
    gap: 9px;
    padding: 14px 16px;
  }

  .action-feedback {
    width: 100%;
    min-height: 0;
    color: #b9ddc2;
    font-size: 0.72rem;
  }

  .action-feedback:empty {
    display: none;
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

  .consumer-content.without-actions .preview {
    grid-column: 1 / -1;
    grid-template-columns: minmax(0, 1fr) auto;
  }

  .consumer-content.without-actions .preview button {
    width: auto;
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

  button {
    border: 0;
    border-radius: 9px;
    padding: 11px 16px;
    font: inherit;
    font-weight: 650;
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
    .consumer-dashboard {
      padding: 14px 16px 22px;
    }

    .hero,
    .hero-copy {
      min-height: 280px;
    }

    .hero-copy {
      padding: 24px 28px;
    }

    .consumer-content {
      grid-template-columns: 1fr;
    }

    .actions {
      grid-row: 1;
    }

    .today,
    .rhythm,
    .preview,
    .warning {
      grid-column: 1;
      grid-row: auto;
    }

    .today-stats,
    .break-counts {
      grid-template-columns: repeat(2, minmax(0, 1fr));
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

    .today-stats,
    .break-counts {
      grid-template-columns: 1fr;
    }

    .rhythm > .text-button,
    .warning > .text-button {
      width: fit-content;
    }
  }

  @media (prefers-contrast: more) {
    .hero-shade {
      background: rgba(3, 9, 7, 0.86);
    }

    .actions,
    .rhythm,
    .preview,
    .warning {
      border-color: #829187;
    }
  }
</style>
