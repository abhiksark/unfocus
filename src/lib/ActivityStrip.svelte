<script lang="ts">
  import { invoke } from "@tauri-apps/api/core";
  import { tick, untrack } from "svelte";
  import { activityIntervals, breakEventLabel, breakMarkerRefreshKey, createBreakMarkerLoader } from "./activity-strip";
  import type { BreakSummary } from "./break-summary";
  import type { BreakHistoryEvent } from "./history";
  import type { RefreshState } from "./refresh-state";
  import { stripAxisTicks, type TodayActivity } from "./today-activity";

  type Props = {
    activity: TodayActivity;
    endMs: number;
    dayStartHour: number;
    stale: boolean;
    visible: boolean;
    breakRefresh: RefreshState<BreakSummary>;
  };
  let { activity, endMs, dayStartHour, stale, visible, breakRefresh }: Props = $props();
  let events = $state<BreakHistoryEvent[]>([]);
  let markersFailed = $state(false);
  let markersLoading = $state(true);
  let focusedIndex = $state(47);
  let selectedIndex = $state<number | null>(null);
  let stripElement: HTMLDivElement;
  const intervals = $derived(activityIntervals(activity, endMs, events));
  const selected = $derived(selectedIndex === null ? null : intervals[selectedIndex]);
  const ticks = $derived(stripAxisTicks(activity.windowSeconds, endMs, dayStartHour));
  const markersUnavailable = $derived(stale || breakRefresh.status !== "fresh" || markersFailed);
  const markerKey = $derived(
    visible && !stale && breakRefresh.status === "fresh"
      ? breakMarkerRefreshKey(breakRefresh.data, endMs)
      : ""
  );
  const markerLoader = createBreakMarkerLoader(
    (range) => invoke<BreakHistoryEvent[]>("get_break_range", range),
    (result) => { events = result; markersLoading = false; markersFailed = false; },
    () => { events = []; markersLoading = false; markersFailed = true; }
  );
  $effect(() => {
    const key = markerKey;
    if (!key) {
      events = [];
      return;
    }
    untrack(() => {
      markersLoading = true;
      void markerLoader.load(Math.max(0, endMs - activity.windowSeconds * 1_000), endMs);
    });
    return () => markerLoader.cancel();
  });

  async function navigate(event: KeyboardEvent, index: number) {
    let next = index;
    if (event.key === "ArrowLeft") next = Math.max(0, index - 1);
    else if (event.key === "ArrowRight") next = Math.min(intervals.length - 1, index + 1);
    else if (event.key === "Home") next = 0;
    else if (event.key === "End") next = intervals.length - 1;
    else return;
    event.preventDefault();
    focusedIndex = next;
    selectedIndex = next;
    await tick();
    stripElement.querySelectorAll<HTMLButtonElement>("button")[next]?.focus();
  }
</script>

<div class="chart-note">Each bar represents 30 minutes. Hover or focus a bar for details.</div>
<div class="strip" bind:this={stripElement} role="group" aria-label={stale ? "Last-known activity, half-hour intervals" : "Activity, half-hour intervals"}>
  {#each intervals as interval, index (index)}
    <button
      type="button"
      class="bucket"
      class:selected={selectedIndex === index}
      tabindex={focusedIndex === index ? 0 : -1}
      aria-label={`${interval.label}: ${interval.details}. ${markersUnavailable ? "Break outcomes unavailable." : markersLoading ? "Reading break outcomes." : `${interval.events.length} break ${interval.events.length === 1 ? "outcome" : "outcomes"}. ${interval.events.map(breakEventLabel).join(". ")}`} Use arrow keys to explore.`}
      onkeydown={(event) => void navigate(event, index)}
      onfocus={() => { focusedIndex = index; selectedIndex = index; }}
      onmouseenter={() => (selectedIndex = index)}
      onclick={() => { focusedIndex = index; selectedIndex = index; }}
    >
      <span class="bar" aria-hidden="true">
        <span class="active" style={`height:${interval.active * 100}%`}></span>
        <span class="away" style={`height:${interval.away * 100}%`}></span>
        <span class="unknown" style={`height:${interval.unknown * 100}%`}></span>
      </span>
      <span class="marker" aria-hidden="true">{interval.events.length > 0 ? "◆" : ""}</span>
    </button>
  {/each}
</div>
<div class="axis" aria-hidden="true">
  {#each ticks as tick (tick.timestampMs)}
    {#if tick.showLabel}
      <span class:day-start={tick.isDayStart} style={`left:${tick.positionPercent}%`}>{tick.label}</span>
    {/if}
  {/each}
  <span class="endpoint">{stale ? "last known" : "now"}</span>
</div>
<ul class="legend" aria-label="Activity legend">
  <li><i class="active"></i>Active</li>
  <li><i class="away"></i>Away</li>
  <li><i class="unknown"></i>Unclassified</li>
  <li><span>◆</span>Break outcome</li>
</ul>
<div class="readout">
  {#if selected}
    <p><strong>{selected.label}</strong> · {selected.details}</p>
    {#if selected.events.length > 0}
      <ul>{#each selected.events as event}<li>{breakEventLabel(event)}</li>{/each}</ul>
    {/if}
  {:else}
    <p>Unclassified time has no confirmed active or away reading.</p>
  {/if}
  {#if stale || breakRefresh.status === "stale" || breakRefresh.status === "unavailable" || markersFailed}
    <p role="status">Break details are unavailable.</p>
  {:else if markersLoading}
    <p>Loading break details…</p>
  {/if}
</div>

<style>
  .chart-note, .legend, .readout, .axis { color: var(--ink-2); font-size: 0.75rem; }
  .chart-note { margin-bottom: var(--s3); }
  .strip { display: grid; grid-template-columns: repeat(48, minmax(0, 1fr)); gap: 2px; }
  .bucket { min-width: 0; padding: 0; border: 0; background: transparent; color: var(--ink-2); cursor: pointer; border-radius: 2px; }
  .bucket:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
  .bucket.selected .bar { outline: 1px solid var(--ink-2); outline-offset: 1px; }
  .bar { display: flex; flex-direction: column-reverse; height: clamp(64px, 6vw, 96px); border-radius: 2px; overflow: hidden; }
  .bar > span { flex-shrink: 0; width: 100%; }
  .active { background: var(--accent); }
  .away { background: var(--away); }
  .unknown { background: repeating-linear-gradient(135deg, transparent 0 3px, var(--line) 3px 4px); }
  .marker { display: block; height: 22px; line-height: 22px; font-size: 0.75rem; }
  .axis { position: relative; height: 20px; font-family: var(--mono); }
  .axis > span { position: absolute; transform: translateX(-50%); white-space: nowrap; }
  .axis .endpoint { right: 0; transform: none; }
  .axis .day-start { font-weight: 700; color: var(--ink); }
  .legend { display: flex; gap: var(--s4); flex-wrap: wrap; padding: 0; margin: var(--s3) 0; list-style: none; }
  .legend li { display: flex; align-items: center; gap: 6px; }
  .legend i { width: 10px; height: 10px; border-radius: 2px; }
  .readout { min-height: 38px; line-height: 1.6; }
  .readout p { margin: 0; }
  .readout strong { color: var(--ink); font-weight: 500; }
  .readout ul { padding-left: var(--s4); margin: var(--s1) 0; max-height: 120px; overflow: auto; }
</style>
