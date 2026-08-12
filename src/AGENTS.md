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
  and `--r-button` only. `--sans` and `--serif` are the only font stacks.
- Use a token where one matches the value exactly. Where none matches, keep the
  literal rather than snapping to the nearest step; the overlay's spacing is
  tuned against monitor height and a 2px snap moves it.
- `--accent` is reserved for the live state dot, whichever single button is
  currently primary, and the day strip's active bars. Never two accent buttons
  at once.
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
- The strip is presence-only (OS idle). Never request or display keylogging.
- `break-summary.ts` formats calm counts from `get_break_summary`. Day captions
  must not re-list the grid counts; mute zeros in the UI rather than inventing
  scores, streaks, or competitive framing.

## Window routing and native events

- Window labels route rendering:
  `overlay-<run>-<index>-<count>-<duration>-<deadline>` renders `BreakOverlay`;
  any other label renders the dashboard.
- The label is the only channel for overlay parameters. Keep its format and
  parsing synchronized with the Rust side.
- Overlay events carry a `runId`; filter on it in every handler.
- An untargeted `listen()` receives events from every run. Pass the current
  window label as `target` so the Rust side can scope delivery.

## Break scene

- `src/lib/scene.svg` is generated with
  `bun scripts/gen-scene.js > src/lib/scene.svg`. Change the generator and
  regenerate; never hand-edit the SVG.
- Use smooth noise-based silhouettes with one asymmetric focal summit, valley
  mist with radial falloff, and hairline strokes. No zigzag polygons, glow
  effects, full-width gradient bands, or drop-shadow halos.
- Full-viewport layers stay static; a repainted full-screen gradient previously
  reached 217% CPU. Continuous motion is limited to transform/opacity on small
  layers and stepped with `steps()` so repaints land every 2-3 seconds; state
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
