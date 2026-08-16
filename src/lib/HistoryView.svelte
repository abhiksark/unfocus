<script lang="ts">
  import {
    buildHistoryCalendarRequest,
    buildHistoryDayDetailRequest,
    historyMonthMarkers,
    initialHistoryDateKey,
    moveHistoryGridFocus,
    type ActivityRangeBucket,
    type BreakHistoryEvent,
    type HistoryCalendar,
    type HistoryCalendarDay,
    type HistoryDay,
    type HistoryHourSlot
  } from "$lib/history";
  import {
    createHistoryCalendarLoader,
    createHistoryDayLoader
  } from "$lib/history-loader";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount, tick, untrack } from "svelte";

  type Props = {
    dayStartHour: number;
    onBack: () => void;
  };

  let { dayStartHour, onBack }: Props = $props();

  let calendar = $state<HistoryCalendar | null>(null);
  let calendarLoading = $state(true);
  let calendarError = $state<string | null>(null);
  let selectedDateKey = $state<string | null>(null);
  let focusedDateKey = $state<string | null>(null);
  let previewDateKey = $state<string | null>(null);
  let detail = $state<HistoryDay | null>(null);
  let detailLoading = $state(false);
  let detailError = $state<string | null>(null);
  let backButton: HTMLButtonElement;

  const calendarRequest = buildHistoryCalendarRequest(
    Date.now(),
    untrack(() => dayStartHour)
  );
  const fetcher = {
    getActivityRange: ({ boundaries }: { boundaries: number[] }) =>
      invoke<ActivityRangeBucket[]>("get_activity_range", { boundaries }),
    getBreakRange: ({ startMs, endMs }: { startMs: number; endMs: number }) =>
      invoke<BreakHistoryEvent[]>("get_break_range", { startMs, endMs })
  };

  const selectedCalendarDay = $derived(
    calendar?.days.find((day) => day.dateKey === selectedDateKey) ?? null
  );
  const previewDay = $derived(
    calendar?.days.find((day) => day.dateKey === previewDateKey) ??
      selectedCalendarDay
  );
  const monthMarkers = $derived(calendar ? historyMonthMarkers(calendar) : []);
  const rangeLabel = $derived(
    calendar
      ? `${formatShortDate(calendar.days[0].startMs)} to ${formatShortDate(
          calendar.days[calendar.days.length - 1].startMs
        )}`
      : ""
  );
  const calendarCaption = $derived(
    calendarLoading
      ? "Reading local activity…"
      : calendarError
        ? "History is unavailable right now."
        : rangeLabel
  );

  const loadCalendar = createHistoryCalendarLoader(
    fetcher,
    (loaded) => {
      calendar = loaded;
      calendarError = null;
      calendarLoading = false;
      const initialDateKey = initialHistoryDateKey(loaded.days);
      selectedDateKey = initialDateKey;
      focusedDateKey = initialDateKey;
      if (initialDateKey) requestDay(initialDateKey);
    },
    () => {
      calendarError = "Could not read local activity history. Please try again.";
      calendarLoading = false;
    }
  );

  const loadDay = createHistoryDayLoader(
    fetcher,
    (loaded) => {
      detail = loaded;
      detailError = null;
      detailLoading = false;
    },
    () => {
      detailError = "Could not read hourly activity and break outcomes for this day.";
      detailLoading = false;
    }
  );

  function requestCalendar(): void {
    calendar = null;
    calendarLoading = true;
    calendarError = null;
    selectedDateKey = null;
    focusedDateKey = null;
    previewDateKey = null;
    detail = null;
    detailError = null;
    void loadCalendar(calendarRequest);
  }

  function requestDay(dateKey: string): void {
    const day = calendar?.days.find((candidate) => candidate.dateKey === dateKey);
    if (!day) return;
    selectedDateKey = dateKey;
    focusedDateKey = dateKey;
    previewDateKey = null;
    detail = null;
    detailLoading = true;
    detailError = null;
    void loadDay(buildHistoryDayDetailRequest(calendarRequest, dateKey), {
      activeMs: day.totals.activeMs,
      afkMs: day.totals.afkMs,
      longestActiveMs: day.totals.longestActiveMs
    });
  }

  function retryDay(): void {
    if (selectedDateKey) requestDay(selectedDateKey);
  }

  async function moveGridFocus(event: KeyboardEvent, day: HistoryCalendarDay): Promise<void> {
    if (
      event.key !== "ArrowLeft" &&
      event.key !== "ArrowRight" &&
      event.key !== "ArrowUp" &&
      event.key !== "ArrowDown"
    ) {
      return;
    }
    event.preventDefault();
    const nextDateKey = calendar
      ? moveHistoryGridFocus(calendar.days, day.dateKey, event.key)
      : null;
    if (!nextDateKey) return;
    focusedDateKey = nextDateKey;
    await tick();
    document.getElementById(`history-day-${nextDateKey}`)?.focus();
  }

  function clearPreview(dateKey: string): void {
    if (previewDateKey === dateKey) previewDateKey = null;
  }

  function formatShortDate(timestampMs: number): string {
    return new Intl.DateTimeFormat([], { month: "short", day: "numeric" }).format(
      new Date(timestampMs)
    );
  }

  function formatLongDate(timestampMs: number): string {
    return new Intl.DateTimeFormat([], {
      weekday: "long",
      month: "long",
      day: "numeric"
    }).format(new Date(timestampMs));
  }

  function formatMonth(timestampMs: number): string {
    return new Intl.DateTimeFormat([], { month: "short" }).format(new Date(timestampMs));
  }

  function formatTime(timestampMs: number): string {
    return new Intl.DateTimeFormat([], { timeStyle: "short" }).format(
      new Date(timestampMs)
    );
  }

  function activityLabel(day: HistoryCalendarDay): string {
    if (day.activityLevel === "no-data") return "No classified activity";
    if (day.totals.activeMs === 0) return "0 active minutes";
    return `${day.totals.activeLabel} active`;
  }

  function calendarCellClass(day: HistoryCalendarDay): string {
    return day.activityLevel === "no-data"
      ? "calendar-cell is-no-data"
      : `calendar-cell is-level-${day.activityLevel}`;
  }

  function slotClass(slot: HistoryHourSlot): string {
    return `slot is-${slot.kind}`;
  }

  function slotKindLabel(slot: HistoryHourSlot): string {
    switch (slot.kind) {
      case "active":
        return "active";
      case "afk":
        return "away";
      case "mixed":
        return "active and away";
      case "blank":
        return "no data";
    }
  }

  function daySlotSummary(day: HistoryDay): string {
    return `Activity slots: ${day.hourSlots
      .map((slot) => `${slot.label} ${slotKindLabel(slot)}`)
      .join("; ")}.`;
  }

  function breakKindLabel(event: BreakHistoryEvent, day: HistoryDay): string {
    return (
      day.breakCounts.find((count) => count.kind === event.kind)?.label ?? event.kind
    );
  }

  function breakMarkerLabel(slot: HistoryHourSlot, day: HistoryDay): string {
    const outcomes = slot.breakMarkers
      .map((event) => `${formatTime(event.atMs)} ${breakKindLabel(event, day)}`)
      .join("; ");
    return slot.breakMarkers.length === 1
      ? `Break outcome: ${outcomes}`
      : `${slot.breakMarkers.length} break outcomes: ${outcomes}`;
  }

  onMount(() => {
    backButton.focus();
    requestCalendar();
  });
</script>

<main class="history-wrap">
  <header class="history-top">
    <button bind:this={backButton} class="btn-text" type="button" onclick={onBack}>Back</button>
    <div>
      <p class="t-label">History</p>
      <h1 class="t-title">The last 90 days</h1>
    </div>
  </header>

  <section class="history-panel" aria-busy={calendarLoading}>
    <div class="section-head">
      <div>
        <h2 class="t-title">Active minutes</h2>
        <p class="t-micro" aria-live="polite">{calendarCaption}</p>
      </div>
      {#if previewDay}
        <p class="calendar-preview">
          <strong>{formatLongDate(previewDay.startMs)}</strong>
          <span>{activityLabel(previewDay)}</span>
        </p>
      {/if}
    </div>

    {#if calendarLoading}
      <p class="t-micro" role="status">Reading the newest 90 local days…</p>
    {:else if calendarError}
      <div class="history-message" role="alert">
        <p>History is unavailable right now. The break timer is unaffected.</p>
        <p class="t-micro">{calendarError}</p>
        <button class="btn-ghost" type="button" onclick={requestCalendar}>Retry</button>
      </div>
    {:else if calendar}
      <div
        class="calendar-plot"
        style={`--history-week-count: ${calendar.weekColumnCount}`}
      >
        <div class="month-row" aria-hidden="true">
          {#each monthMarkers as marker (marker.startMs)}
            <span style={`grid-column: ${marker.column}`}>
              {formatMonth(marker.startMs)}
            </span>
          {/each}
        </div>
        <div class="calendar-body">
          <div class="weekday-labels" aria-hidden="true">
            <span>Mon</span><span></span><span>Wed</span><span></span><span>Fri</span><span></span><span>Sun</span>
          </div>
          <p id="history-calendar-instructions" class="sr-only">
            Use arrow keys to move between days. Press Enter or Space to select a day.
          </p>
          <div
            class="calendar-cells"
            role="group"
            aria-label="90-day active minutes calendar"
            aria-describedby="history-calendar-instructions"
          >
            {#each Array.from({ length: calendar.leadingEmptyCells }) as _, index (`leading-${index}`)}
              <span class="calendar-empty" aria-hidden="true"></span>
            {/each}
            {#each calendar.days as day (day.dateKey)}
              <button
                id={`history-day-${day.dateKey}`}
                class={calendarCellClass(day)}
                type="button"
                tabindex={focusedDateKey === day.dateKey ? 0 : -1}
                aria-label={`${formatLongDate(day.startMs)}, ${activityLabel(day)}`}
                aria-pressed={selectedDateKey === day.dateKey}
                title={`${formatLongDate(day.startMs)} · ${activityLabel(day)}`}
                onclick={() => requestDay(day.dateKey)}
                onkeydown={(event) => void moveGridFocus(event, day)}
                onfocus={() => {
                  focusedDateKey = day.dateKey;
                  previewDateKey = day.dateKey;
                }}
                onblur={() => clearPreview(day.dateKey)}
                onmouseenter={() => (previewDateKey = day.dateKey)}
                onmouseleave={() => clearPreview(day.dateKey)}
              ></button>
            {/each}
            {#each Array.from({ length: calendar.trailingEmptyCells }) as _, index (`trailing-${index}`)}
              <span class="calendar-empty" aria-hidden="true"></span>
            {/each}
          </div>
        </div>
      </div>

      <ul class="activity-legend" aria-label="Active-minute intensity scale">
        <li><span class="legend-swatch is-no-data" aria-hidden="true"></span>No data</li>
        <li><span class="legend-swatch is-level-0" aria-hidden="true"></span>0</li>
        <li><span class="legend-swatch is-level-1" aria-hidden="true"></span>&lt;1h</li>
        <li><span class="legend-swatch is-level-2" aria-hidden="true"></span>1–3h</li>
        <li><span class="legend-swatch is-level-3" aria-hidden="true"></span>3–5h</li>
        <li><span class="legend-swatch is-level-4" aria-hidden="true"></span>5h+</li>
      </ul>

      <section class="day-panel" aria-busy={detailLoading} aria-labelledby="selected-day-title">
        <div class="section-head">
          <div>
            <p class="t-label">Selected day</p>
            <h2 id="selected-day-title" class="t-title">
              {selectedCalendarDay ? formatLongDate(selectedCalendarDay.startMs) : "No day selected"}
            </h2>
          </div>
        </div>

        {#if detailLoading}
          <p class="t-micro" role="status">Reading hourly activity and break outcomes…</p>
        {:else if detailError}
          <div class="history-message" role="alert">
            <p>Details are unavailable for this day. The break timer is unaffected.</p>
            <p class="t-micro">{detailError}</p>
            <button class="btn-ghost" type="button" onclick={retryDay}>Retry</button>
          </div>
        {:else if detail}
          <div class="stats" role="group" aria-label={`Activity and breaks for ${detail.label}`}>
            <div class="stat" class:is-zero={detail.totals.activeMs <= 0}>
              <span class="num">{detail.totals.activeLabel}</span>
              <span class="t-micro">Active</span>
            </div>
            <div class="stat" class:is-zero={detail.totals.afkMs <= 0}>
              <span class="num">{detail.totals.afkLabel}</span>
              <span class="t-micro">Away</span>
            </div>
            <div class="stat" class:is-zero={detail.totals.longestActiveMs <= 0}>
              <span class="num">{detail.totals.longestLabel}</span>
              <span class="t-micro">Longest stretch</span>
            </div>
            {#each detail.breakCounts as count (count.kind)}
              <div class="stat" class:is-zero={count.count <= 0}>
                <span class="num">{count.count}</span>
                <span class="t-micro">{count.label}</span>
              </div>
            {/each}
          </div>

          <ul class="timeline-legend" aria-label="Hourly activity legend">
            <li><span class="legend-swatch is-active" aria-hidden="true"></span>Active</li>
            <li><span class="legend-swatch is-afk" aria-hidden="true"></span>Away</li>
            <li><span class="legend-swatch is-mixed" aria-hidden="true"></span>Mixed</li>
            <li><span class="legend-swatch is-blank" aria-hidden="true"></span>No data</li>
            <li><span class="break-symbol" aria-hidden="true">◆</span>Break outcome</li>
          </ul>

          <p class="sr-only">{daySlotSummary(detail)}</p>
          <div class="timeline" aria-label={`Hourly activity for ${detail.label}`}>
            {#each detail.hourSlots as slot (slot.slotIndex)}
              <span class={slotClass(slot)} title={`${slot.label}: ${slotKindLabel(slot)}`}>
                {#if slot.breakMarkers.length > 0}
                  <button
                    class="slot-marker"
                    type="button"
                    aria-label={breakMarkerLabel(slot, detail)}
                    data-tooltip={breakMarkerLabel(slot, detail)}
                    title={breakMarkerLabel(slot, detail)}
                  >
                    <span>{slot.breakMarkers.length > 1 ? slot.breakMarkers.length : ""}</span>
                  </button>
                {/if}
              </span>
            {/each}
          </div>
          <div class="timeline-axis" aria-hidden="true">
            <span>{detail.hourSlots[0]?.label}</span>
            <span>{detail.hourSlots[6]?.label}</span>
            <span>{detail.hourSlots[12]?.label}</span>
            <span>{detail.hourSlots[18]?.label}</span>
          </div>
        {:else}
          <p class="t-micro">Select a day to read its hourly activity.</p>
        {/if}
      </section>
    {/if}
  </section>
</main>

<style>
  .history-wrap {
    display: flex;
    width: min(100%, 860px);
    min-height: 100vh;
    flex-direction: column;
    gap: var(--s5);
    margin: 0 auto;
    padding: var(--s5) var(--s6) var(--s6);
  }

  .history-top,
  .section-head {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--s3) var(--s4);
  }

  .section-head {
    justify-content: space-between;
  }

  .history-panel,
  .day-panel,
  .history-message {
    display: flex;
    flex-direction: column;
    gap: var(--s3);
  }

  .t-title {
    margin: 0;
    font-size: 1.3rem;
    font-weight: 500;
    letter-spacing: -0.01em;
    line-height: 1.25;
  }

  .t-label {
    margin: 0 0 var(--s1);
    color: var(--ink-2);
    font-size: 0.8rem;
    font-weight: 500;
  }

  .t-micro {
    margin: 0;
    color: var(--ink-3);
    font-size: 0.74rem;
    line-height: 1.45;
  }

  button {
    border-radius: var(--r-button);
    font: inherit;
    cursor: pointer;
  }

  button:focus-visible {
    outline: 3px solid #d9efdf;
    outline-offset: 3px;
  }

  .btn-text {
    border: 0;
    padding: 6px 2px;
    color: var(--ink-2);
    background: transparent;
    font-size: 0.85rem;
    text-decoration: underline;
    text-decoration-color: var(--line-2);
    text-underline-offset: 4px;
  }

  .btn-text:hover {
    color: var(--ink);
  }

  .btn-ghost {
    align-self: flex-start;
    border: 1px solid var(--line-2);
    padding: 10px 16px;
    color: var(--ink-2);
    background: transparent;
    font-size: 0.9rem;
    font-weight: 500;
  }

  .btn-ghost:hover {
    border-color: var(--ink-3);
    color: var(--ink);
  }

  .calendar-preview {
    display: flex;
    min-width: 150px;
    flex-direction: column;
    gap: 2px;
    margin: 0;
    color: var(--ink-3);
    font-size: 0.74rem;
    text-align: right;
  }

  .calendar-preview strong {
    color: var(--ink-2);
    font-weight: 500;
  }

  .calendar-plot {
    border: 1px solid var(--line);
    border-radius: var(--r-control);
    padding: var(--s3);
    background: #0c130f;
  }

  .month-row,
  .calendar-cells {
    display: grid;
    grid-auto-flow: column;
    grid-template-columns: repeat(var(--history-week-count), minmax(12px, 22px));
    gap: var(--s1);
    justify-content: space-between;
  }

  .month-row {
    min-height: 18px;
    margin-left: 34px;
    color: var(--ink-3);
    font-size: 0.68rem;
  }

  .month-row span {
    white-space: nowrap;
  }

  .calendar-body {
    display: grid;
    grid-template-columns: 26px minmax(0, 1fr);
    gap: var(--s2);
  }

  .weekday-labels,
  .calendar-cells {
    grid-template-rows: repeat(7, minmax(12px, 22px));
  }

  .weekday-labels {
    display: grid;
    gap: var(--s1);
    color: var(--ink-3);
    font-size: 0.64rem;
    line-height: 1;
  }

  .weekday-labels span {
    display: flex;
    align-items: center;
  }

  .calendar-cell,
  .calendar-empty {
    width: 100%;
    min-width: 12px;
    max-width: 22px;
    aspect-ratio: 1;
  }

  .calendar-cell {
    position: relative;
    border: 1px solid transparent;
    border-radius: 3px;
    padding: 0;
  }

  .calendar-cell:hover {
    border-color: var(--ink-2);
  }

  .calendar-cell[aria-pressed="true"] {
    box-shadow: 0 0 0 2px var(--warn);
  }

  .is-no-data {
    border-color: var(--line-2);
    border-style: dashed;
    background: transparent;
  }

  .is-level-0 {
    background: var(--line-2);
  }

  .is-level-1 {
    background: color-mix(in srgb, var(--accent) 30%, var(--line));
  }

  .is-level-2 {
    background: color-mix(in srgb, var(--accent) 50%, var(--line));
  }

  .is-level-3 {
    background: color-mix(in srgb, var(--accent) 72%, var(--line));
  }

  .is-level-4 {
    background: var(--accent);
  }

  .activity-legend,
  .timeline-legend {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s2) var(--s4);
    margin: 0;
    padding: 0;
    color: var(--ink-2);
    font-size: 0.72rem;
    list-style: none;
  }

  .activity-legend li,
  .timeline-legend li {
    display: inline-flex;
    align-items: center;
    gap: 6px;
  }

  .legend-swatch {
    display: inline-block;
    width: 10px;
    height: 10px;
    border: 1px solid var(--line-2);
    border-radius: 2px;
  }

  .legend-swatch.is-active {
    background: var(--accent);
  }

  .legend-swatch.is-afk {
    background: var(--away);
  }

  .legend-swatch.is-mixed {
    background: linear-gradient(180deg, var(--accent) 0 52%, var(--away) 52% 100%);
  }

  .legend-swatch.is-blank {
    background: var(--line);
  }

  .break-symbol {
    color: var(--warn);
  }

  .day-panel {
    margin-top: var(--s2);
    border-top: 1px solid var(--line);
    padding-top: var(--s4);
  }

  .stats {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s3) var(--s5);
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

  .num {
    color: var(--ink);
    font-size: 1.2rem;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.02em;
  }

  .timeline {
    display: grid;
    height: 44px;
    gap: 2px;
    grid-template-columns: repeat(24, minmax(0, 1fr));
  }

  .slot {
    position: relative;
    border-radius: 2px;
    background: var(--line);
  }

  .slot.is-active {
    background: var(--accent);
    opacity: 0.82;
  }

  .slot.is-afk {
    background: var(--away);
    opacity: 0.72;
  }

  .slot.is-mixed {
    background: linear-gradient(180deg, var(--accent) 0 52%, var(--away) 52% 100%);
    opacity: 0.8;
  }

  .slot-marker {
    position: absolute;
    z-index: 1;
    right: 50%;
    bottom: 3px;
    display: grid;
    width: 16px;
    height: 16px;
    transform: translateX(50%) rotate(45deg);
    place-items: center;
    border: 1px solid var(--bg);
    border-radius: 2px;
    padding: 0;
    color: var(--bg);
    background: var(--warn);
    font-size: 0.54rem;
    font-weight: 700;
    line-height: 1;
  }

  .slot-marker > span {
    transform: rotate(-45deg);
  }

  .slot-marker::after {
    position: absolute;
    z-index: 2;
    bottom: 18px;
    left: 50%;
    display: none;
    width: max-content;
    max-width: 220px;
    transform: translateX(-50%) rotate(-45deg);
    border-radius: var(--r-control);
    padding: var(--s2);
    color: var(--bg);
    background: var(--ink);
    content: attr(data-tooltip);
    font-size: 0.72rem;
    font-weight: 500;
    line-height: 1.35;
    text-align: left;
    white-space: normal;
  }

  .slot-marker:hover::after,
  .slot-marker:focus-visible::after {
    display: block;
  }

  .timeline-axis {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    color: var(--ink-3);
    font-size: 0.7rem;
    font-variant-numeric: tabular-nums;
  }

  .sr-only {
    position: absolute;
    width: 1px;
    height: 1px;
    overflow: hidden;
    clip: rect(0 0 0 0);
    clip-path: inset(50%);
    white-space: nowrap;
  }

  @media (max-width: 760px) {
    .history-wrap {
      padding: var(--s4) var(--s4) var(--s6);
    }

    .calendar-preview {
      width: 100%;
      text-align: left;
    }

    .calendar-plot {
      padding: var(--s2);
    }

    .month-row {
      margin-left: 32px;
    }

    .stats {
      gap: var(--s3) var(--s4);
    }
  }

  @media (prefers-contrast: more) {
    .calendar-plot,
    .day-panel {
      border-width: 2px;
    }

    .calendar-cell:not(.is-no-data) {
      border-color: var(--ink-2);
    }

    .calendar-cell:not(.is-no-data)::after {
      position: absolute;
      top: 50%;
      left: 50%;
      width: var(--history-dot-size, 2px);
      height: var(--history-dot-size, 2px);
      transform: translate(-50%, -50%);
      border-radius: 50%;
      background: var(--ink);
      content: "";
    }

    .is-level-1 { --history-dot-size: 4px; }
    .is-level-2 { --history-dot-size: 6px; }
    .is-level-3 { --history-dot-size: 8px; }
    .is-level-4 { --history-dot-size: 10px; }
  }
</style>
