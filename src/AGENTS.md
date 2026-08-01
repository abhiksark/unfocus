# Frontend

SvelteKit 2 + Svelte 5 runes, static adapter. Window labels route rendering:
`overlay-<run>-<index>-<count>-<duration>-<deadline>` renders `BreakOverlay`,
anything else the dashboard. The label is the only channel for overlay
parameters, so both sides must agree on its format.

## Break scene

- `src/lib/scene.svg` is generated: `bun scripts/gen-scene.js > src/lib/scene.svg`.
  Change the generator and regenerate; never hand-edit the SVG.
- Visual bar: smooth noise-based silhouettes with one asymmetric focal summit;
  valley mist with radial falloff; hairline strokes. No zigzag polygons, no
  glow effects, no full-width gradient bands, no drop-shadow halos.
- Animation budget (a repainted full-screen gradient once hit 217% CPU):
  full-viewport layers stay static; continuous motion is transform/opacity on
  small layers only, stepped with `steps()` so repaints land every 2-3 s;
  state changes are one-shot opacity transitions.
- `prefers-reduced-motion` stops every loop and drops state fades to 120 ms.
  `prefers-contrast: more` must keep copy legible over the scene.
- Multi-monitor sync: every animation delay derives from
  `--presentation-offset` so displays share one visual phase.
- Type: Fraunces (vendored, `static/fonts/`, OFL) for display copy; monospace
  for eyebrows and digits. No font or asset CDNs.

## Checks

`bun run check` and `bun run build` must pass before committing.
