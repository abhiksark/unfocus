# Unfocus

Cross-platform eye-break reminder. Tauri 2 (Rust core) + SvelteKit 2 with
Svelte 5 runes, built with Bun.

```
src/         SvelteKit frontend: dashboard + break overlay (see src/AGENTS.md)
src-tauri/   Rust core: tray, windows, probes (see src-tauri/AGENTS.md)
scripts/     gen-scene.js (scene generator) and the container runner
static/      icon SVG and vendored fonts
```

## Commands

Use Bun for all JS tooling. npm, npx, and pnpm are not used in this repo.

- `bun install` — install JS dependencies
- `bun run check` — svelte-check
- `bun run build` — production frontend build
- `cargo test --manifest-path src-tauri/Cargo.toml` — Rust tests
- `bun run tauri dev` — run the app; on Linux needs native prerequisites
  (webkit2gtk-4.1, libayatana-appindicator3, libxdo), on macOS needs only
  the Xcode command line tools
- `./scripts/run-linux-spike-container.sh` — build and run in a container
  against the host X11 display when native headers are not installed

Before claiming a change done, run the checks for the area you touched:
frontend changes need `bun run check` and `bun run build` at 0 errors and
0 warnings; Rust changes need `cargo fmt --check`, `cargo clippy` with
warnings denied, and `cargo test`.

## Ground rules

- Linux X11 is the qualified backend. macOS reads idle and fullscreen
  through Quartz and has been verified interactively, but not through a
  multi-monitor acceptance run — do not describe it as qualified until it
  has one. Wayland and Windows have no probes at all. Any platform without a
  probe reports an error per poll instead of guessing.
- IMPORTANT: a failing OS probe must never crash or alter the break timer.
  Degrade, log once, keep ticking.
- IMPORTANT: never state a memory footprint in code, docs, or commits. The
  early 10-30 MB assumption was measured wrong (~203 MiB PSS); packaged
  builds must be re-measured before any claim.
- No mascots or characters anywhere in the product. Scene lighting carries
  state: cool green while resting, amber dawn while returning or complete.
- Local-first. No telemetry, accounts, or runtime network calls.
- `plans/` and other notes are untracked working files. Tracked markdown is
  README.md plus the CLAUDE.md/AGENTS.md set, nothing else.
- Commit messages: plain imperative summaries. No tool attributions, no
  emoji.
