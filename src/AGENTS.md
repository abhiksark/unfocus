# Frontend

This file applies to `src/**` and frontend assets in `static/fonts/**`. Follow
the root `AGENTS.md` as well. Read `src-tauri/AGENTS.md` for native-interface
changes and `scripts/AGENTS.md` before changing generated assets.

SvelteKit 2 uses the static adapter and Svelte 5 runes. Write runes syntax
(`$props()`, `$state()`, `$derived()`); do not introduce Svelte 4 patterns such
as `export let` or `$:` reactive statements.

## Design tokens

- Color, spacing, radius, and font tokens are declared once, as `:global(:root)`
  in `src/routes/+page.svelte`. Never declare them again from a component. Two
  `:global(:root)` blocks leave the winner dependent on stylesheet order, which
  Svelte does not guarantee.
- Spacing is `--s1` to `--s6` (4, 8, 12, 16, 24, 32). Radius is `--r-control`
  and `--r-button` only. `--sans` and `--serif` are the only shared font stacks;
  the overlay's monospace eyebrow and countdown keep their literal
  `ui-monospace` stack, which is deliberate and described under Break scene.
- Use a token where one matches the value exactly. Where none matches, keep the
  literal rather than snapping to the nearest step; the overlay's spacing is
  tuned against monitor height and a 2px snap moves it.
- `--accent` marks live state and the primary action: the state dot, the day
  strip's active bars and legend swatch, the elapsed-progress fill, and the
  focus ring on timing inputs. Give each context at most one primary button, so
  the reminder actions and the timing editor may each carry one while the editor
  is open. Do not spend it on section labels, secondary controls, or decoration.
- `prefers-contrast: more` raises the token values in `+page.svelte`. A color a
  component hardcodes is excluded from that and can invert the intent, so check
  hover and focus states against the raised values, not just the resting ones.
- Declare a rule only in the component whose own markup uses it. Svelte warns on
  a selector that matches nothing and the gate runs at zero warnings, so a class
  cannot be declared ahead of its first use.

## Activity strip and break ledger

- `today-activity.ts` formats the rolling-window summary from
  `get_today_activity`. Keep copy observational (active / away / deep blocks);
  no streaks, badges, or gamification. Empty, loading, probe-unavailable, and
  error captions stay distinct and note that the timer is unaffected.
- The strip's time axis derives client-side: `stripAxisTicks` converts the
  payload's `windowSeconds` and the render-time clock into round-hour ticks, so
  no timestamp crosses the Rust boundary. Labels use `Intl.DateTimeFormat` with
  the system locale; never hardcode a 12- or 24-hour format.
- The day start is a display preference in browser storage
  (`day-start.ts`, key `unfocus.day-start-hour.v1`), never in
  `reminder-settings.json`. The timer must not read it. `stripAxisTicks` takes
  the hour and flags exactly one matching tick per local day, so a day start
  already on the four-hour grid is marked rather than duplicated.
- The strip is presence-only (OS idle). Never request or display keylogging.
- `break-summary.ts` formats calm counts from `get_break_summary`. Day captions
  must not re-list the grid counts; mute zeros in the UI rather than inventing
  scores, streaks, or competitive framing.
- `history.ts` owns the main-window History shape: three 30-day native query
  partitions materialized as one Monday-aligned 90-day activity calendar, plus
  local day-start boundaries and a selected day's 24 wall-clock hour slots.
  Calendar intensity uses fixed confirmed-active thresholds; no data and zero
  active minutes stay distinct. Keep the saved day-start preference as a
  display preference only. It still must not affect reminder timing or probe
  behavior.
- `history-loader.ts` and `HistoryView.svelte` demand-load the three daily
  activity partitions only after History opens. Selecting a day requests only
  that day's hourly `activeMs`, `afkMs`, and `longestActiveMs` buckets plus
  chronological privacy-safe `{atMs, kind}` break outcomes. A slower prior
  selection must never replace the current detail.
- History stays observational. It may summarize activity and break outcomes,
  but it must not change reminders, probes, overlays, or developer mode
  behavior.

## Window routing and native events

- Window labels route rendering:
  `overlay-<run>-<index>-<count>-<duration>-<deadline>` renders `BreakOverlay`;
  any other label renders the dashboard.
- The label is the only channel for overlay parameters. Keep its format and
  parsing synchronized with the Rust side.
- Overlay events carry a `runId`; filter on it in every handler.
- An untargeted `listen()` receives events from every run. Pass the current
  window label as `target` so the Rust side can scope delivery.
- `break-grid.ts` mirrors the grid arithmetic in
  `src-tauri/src/reminder/schedule.rs`. The two implementations cannot share
  code, so the shared test table in `break-grid.test.ts` is the only guard
  against drift. Change both sides together, exactly as the overlay label
  parsers are kept synchronized.

## Break scene

- `src/lib/scene.svg` is generated with
  `bun scripts/gen-scene.js > src/lib/scene.svg`. Change the generator and
  regenerate; never hand-edit the SVG.
- Use smooth noise-based silhouettes with one asymmetric focal summit, valley
  mist with radial falloff, and hairline strokes. No zigzag polygons, glow
  effects, full-width gradient bands, or drop-shadow halos.
- Full-viewport layers stay static; a repainted full-screen gradient previously
  reached 217% CPU. Continuous motion is limited to transform/opacity on small
  layers and stepped with `steps()` so no layer repaints more often than about
  every two seconds; current layers land between two and five seconds. State
  changes use one-shot opacity transitions.
- `prefers-reduced-motion` stops every loop and reduces state fades to 120 ms.
  `prefers-contrast: more` must keep copy legible over the scene.
- Derive every animation delay from `--presentation-offset` so all monitors
  share one visual phase.
- Fraunces is vendored under `static/fonts/` with the OFL and is used for
  display copy; eyebrows and digits use monospace. Do not add font or asset
  CDNs.

## Checks

Run all three with zero errors and zero warnings:

```sh
bun run test
bun run check
bun run build
```

Run the Rust gate from `src-tauri/AGENTS.md` as well when changing commands,
events, labels, Tauri configuration, overlay timing, or another native
interface.
