<script lang="ts">
  import {
    HISTORY_PAGE_COUNT,
    buildHistoryPageRequest,
    type BreakHistoryEvent,
    type HistoryDay,
    type HistoryHourSlot,
    type HistoryPage
  } from "$lib/history";
  import { createHistoryPageLoader } from "$lib/history-loader";
  import { invoke } from "@tauri-apps/api/core";
  import { onMount } from "svelte";

  type Props = {
    dayStartHour: number;
    onBack: () => void;
  };

  let { dayStartHour, onBack }: Props = $props();

  let pageIndex = $state(0);
  let page = $state<HistoryPage | null>(null);
  let loading = $state(true);
  let error = $state<string | null>(null);
  let backButton: HTMLButtonElement;
  const anchorMs = Date.now();

  const rows = $derived(page ? page.days.slice().reverse() : []);
  const hasHistory = $derived(
    page
      ? !page.totals.isBlank || page.breakCounts.some((count) => count.count > 0)
      : false
  );
  const rangeLabel = $derived(page ? pageRangeLabel(page) : "");
  const pageCaption = $derived(
    loading
      ? "Reading local history…"
      : error
        ? "History is unavailable for this page."
        : page
          ? rangeLabel
          : "No local history loaded."
  );
  const canGoNewer = $derived(pageIndex > 0);
  const canGoOlder = $derived(pageIndex < HISTORY_PAGE_COUNT - 1);

  const loadPage = createHistoryPageLoader(
    {
      getActivityRange: ({ boundaries }) =>
        invoke("get_activity_range", { boundaries }),
      getBreakRange: ({ startMs, endMs }) =>
        invoke("get_break_range", { startMs, endMs })
    },
    (loaded) => {
      page = loaded;
      error = null;
      loading = false;
    },
    (value) => {
      error = errorMessage(value);
      loading = false;
    }
  );

  function errorMessage(value: unknown): string {
    void value;
    return "Could not read local history for this page. Please try again.";
  }

  function requestPage(index: number): void {
    pageIndex = index;
    page = null;
    loading = true;
    error = null;
    void loadPage(buildHistoryPageRequest(index, anchorMs, dayStartHour));
  }

  function retry(): void {
    requestPage(pageIndex);
  }

  function pageRangeLabel(historyPage: HistoryPage): string {
    const first = historyPage.days[0];
    const last = historyPage.days[historyPage.days.length - 1];
    return `${formatDate(first.startMs)} to ${formatDate(last.startMs)}`;
  }

  function formatDate(timestampMs: number): string {
    return new Intl.DateTimeFormat([], {
      weekday: "short",
      month: "short",
      day: "numeric"
    }).format(new Date(timestampMs));
  }

  function formatLongDate(timestampMs: number): string {
    return new Intl.DateTimeFormat([], {
      weekday: "long",
      month: "long",
      day: "numeric"
    }).format(new Date(timestampMs));
  }

  function formatTime(timestampMs: number): string {
    return new Intl.DateTimeFormat([], { timeStyle: "short" }).format(
      new Date(timestampMs)
    );
  }

  function slotTitle(slot: HistoryHourSlot): string {
    return `${slot.label}: ${slotKindLabel(slot)}`;
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

  function dayBreakMarkers(day: HistoryDay): BreakHistoryEvent[] {
    return day.hourSlots.flatMap((slot) => slot.breakMarkers);
  }

  function breakLabel(event: BreakHistoryEvent, historyPage: HistoryPage): string {
    return (
      historyPage.breakCounts.find((count) => count.kind === event.kind)?.label ??
      event.kind
    );
  }

  onMount(() => {
    backButton.focus();
    requestPage(0);
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

  <section class="history-panel" aria-busy={loading}>
    <div class="section-head">
      <div>
        <h2 class="t-title">Page {pageIndex + 1} of {HISTORY_PAGE_COUNT}</h2>
        <p class="t-micro" aria-live="polite">{pageCaption}</p>
      </div>
      <nav class="page-actions" aria-label="History pages">
        <button class="btn-ghost" type="button" onclick={() => requestPage(pageIndex - 1)} disabled={!canGoNewer || loading}>
          Newer
        </button>
        <button class="btn-ghost" type="button" onclick={() => requestPage(pageIndex + 1)} disabled={!canGoOlder || loading}>
          Older
        </button>
      </nav>
    </div>

    {#if page}
      <div class="stats" class:is-empty={!hasHistory} role="group" aria-label="History totals for this page">
        <div class="stat" class:is-zero={page.totals.activeMs <= 0}>
          <span class="num">{page.totals.activeLabel}</span>
          <span class="t-micro">Active</span>
        </div>
        <div class="stat" class:is-zero={page.totals.afkMs <= 0}>
          <span class="num">{page.totals.afkLabel}</span>
          <span class="t-micro">Away</span>
        </div>
        {#each page.breakCounts as count (count.kind)}
          <div class="stat" class:is-zero={count.count <= 0}>
            <span class="num">{count.count}</span>
            <span class="t-micro">{count.label}</span>
          </div>
        {/each}
      </div>
    {/if}

    {#if loading}
      <p class="t-micro" role="status">Reading local history for this page…</p>
    {:else if error}
      <div class="history-message" role="alert">
        <p>History is unavailable right now. The break timer is unaffected.</p>
        <p class="t-micro">{error}</p>
        <button class="btn-ghost" type="button" onclick={retry}>Retry</button>
      </div>
    {:else if page && !hasHistory}
      <p class="t-micro" role="status">No active, away, or break history in this page yet.</p>
    {/if}

    {#if page}
      <ul class="timeline-legend" aria-label="Activity slot legend">
        <li><span class="legend-swatch is-active" aria-hidden="true"></span> Active</li>
        <li><span class="legend-swatch is-afk" aria-hidden="true"></span> Away</li>
        <li><span class="legend-swatch is-mixed" aria-hidden="true"></span> Mixed</li>
        <li><span class="legend-swatch is-blank" aria-hidden="true"></span> No data</li>
      </ul>
      <div class="day-list" aria-label="Daily history rows">
        {#each rows as day (day.dateKey)}
          <details class="day-row">
            <summary>
              <span>{formatLongDate(day.startMs)}</span>
              <span class="summary-stats">
                {day.totals.activeLabel} active · {day.totals.afkLabel} away
              </span>
            </summary>

            <div class="day-detail">
              <div class="stats compact" role="group" aria-label={`Activity and breaks for ${day.label}`}>
                <div class="stat" class:is-zero={day.totals.activeMs <= 0}>
                  <span class="num">{day.totals.activeLabel}</span>
                  <span class="t-micro">Active</span>
                </div>
                <div class="stat" class:is-zero={day.totals.afkMs <= 0}>
                  <span class="num">{day.totals.afkLabel}</span>
                  <span class="t-micro">Away</span>
                </div>
                <div class="stat" class:is-zero={day.totals.longestActiveMs <= 0}>
                  <span class="num">{day.totals.longestLabel}</span>
                  <span class="t-micro">Longest stretch</span>
                </div>
                {#each day.breakCounts as count (count.kind)}
                  <div class="stat" class:is-zero={count.count <= 0}>
                    <span class="num">{count.count}</span>
                    <span class="t-micro">{count.label}</span>
                  </div>
                {/each}
              </div>

              <p class="sr-only">{daySlotSummary(day)}</p>
              <div class="timeline" aria-hidden="true">
                {#each day.hourSlots as slot (slot.slotIndex)}
                  <span class={slotClass(slot)} title={slotTitle(slot)}>
                    {#if slot.breakMarkers.length > 0}
                      <span class="slot-marker" aria-hidden="true"></span>
                    {/if}
                  </span>
                {/each}
              </div>
              <div class="timeline-axis" aria-hidden="true">
                <span>{day.hourSlots[0]?.label}</span>
                <span>{day.hourSlots[6]?.label}</span>
                <span>{day.hourSlots[12]?.label}</span>
                <span>{day.hourSlots[18]?.label}</span>
              </div>

              {#if dayBreakMarkers(day).length > 0}
                <ul class="break-markers" aria-label={`Break markers for ${day.label}`}>
                  {#each dayBreakMarkers(day) as event}
                    <li>{formatTime(event.atMs)} · {breakLabel(event, page)}</li>
                  {/each}
                </ul>
              {:else}
                <p class="t-micro">No break outcomes recorded for this day.</p>
              {/if}
            </div>
          </details>
        {/each}
      </div>
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
  .section-head,
  .page-actions {
    display: flex;
    flex-wrap: wrap;
    align-items: baseline;
    gap: var(--s3) var(--s4);
  }

  .section-head {
    justify-content: space-between;
  }

  .history-panel,
  .day-detail,
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

  button:focus-visible,
  summary:focus-visible {
    outline: 3px solid #d9efdf;
    outline-offset: 3px;
  }

  button:disabled {
    cursor: not-allowed;
    opacity: 0.52;
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

  .btn-text:hover:not(:disabled) {
    color: var(--ink);
  }

  .btn-ghost {
    border: 1px solid var(--line-2);
    padding: 10px 16px;
    color: var(--ink-2);
    background: transparent;
    font-size: 0.9rem;
    font-weight: 500;
  }

  .btn-ghost:hover:not(:disabled) {
    border-color: var(--ink-3);
    color: var(--ink);
  }

  .stats {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s5) var(--s6);
  }

  .stats.compact {
    gap: var(--s3) var(--s5);
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

  .num {
    color: var(--ink);
    font-size: 1.35rem;
    font-weight: 500;
    font-variant-numeric: tabular-nums;
    letter-spacing: -0.02em;
  }

  .day-list {
    display: flex;
    flex-direction: column;
    gap: var(--s2);
  }

  .timeline-legend {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s2) var(--s4);
    margin: 0;
    padding: 0;
    color: var(--ink-2);
    font-size: 0.74rem;
    list-style: none;
  }

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

  .day-row {
    border: 1px solid var(--line);
    border-radius: var(--r-control);
    background: #0c130f;
  }

  .day-row[open] {
    border-color: var(--line-2);
  }

  summary {
    display: flex;
    flex-wrap: wrap;
    justify-content: space-between;
    gap: var(--s1) var(--s3);
    padding: var(--s3);
    cursor: pointer;
    color: var(--ink);
    font-weight: 520;
  }

  .summary-stats {
    color: var(--ink-3);
    font-size: 0.76rem;
    font-weight: 400;
  }

  .day-detail {
    border-top: 1px solid var(--line);
    padding: var(--s3);
  }

  .timeline {
    display: grid;
    height: 34px;
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
    right: 3px;
    bottom: 3px;
    width: 5px;
    height: 5px;
    border-radius: 50%;
    background: var(--ink);
  }

  .timeline-axis {
    display: grid;
    grid-template-columns: repeat(4, 1fr);
    color: var(--ink-3);
    font-size: 0.7rem;
    font-variant-numeric: tabular-nums;
  }

  .break-markers {
    display: flex;
    flex-wrap: wrap;
    gap: var(--s2) var(--s3);
    margin: 0;
    padding: 0;
    color: var(--ink-2);
    font-size: 0.74rem;
    list-style: none;
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

    .stats {
      gap: var(--s4) var(--s5);
    }
  }

  @media (prefers-contrast: more) {
    .day-row {
      border-width: 2px;
    }
  }

</style>
