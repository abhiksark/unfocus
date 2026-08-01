# Unfocus

Cross-platform eye-break reminder. Tauri 2: Rust core (`src-tauri/`), SvelteKit
frontend (`src/`), Bun for JS tooling.

## Commands

- `bun install` — install JS dependencies
- `bun run check` — svelte-check; must stay at 0 errors, 0 warnings
- `bun run build` — production frontend build
- `cargo test --manifest-path src-tauri/Cargo.toml` — Rust tests
- `cargo fmt --check` and `cargo clippy` (warnings denied) before committing Rust
- `bun run tauri dev` — requires native prerequisites (webkit2gtk-4.1, appindicator, xdo)
- `./scripts/run-linux-spike-container.sh` — containerized build/run against the
  host X11 display when native headers are not installed

## Ground rules

- Linux X11 is the first qualified backend. Wayland is unsupported until it has
  its own acceptance run; on Wayland, probes report errors instead of guessing.
- A failing OS probe must never crash or alter the break timer. Degrade, log
  once, keep ticking.
- Do not state a memory footprint anywhere. The early 10-30 MB assumption was
  wrong (a release build measured ~203 MiB PSS); re-measure packaged builds
  before making any claim.
- No mascots or characters. Scene lighting carries state: cool green while
  resting, amber dawn while returning/complete.
- Local-first. No telemetry, accounts, or network calls at runtime.
- `plans/` and other notes are untracked working files. The only markdown that
  belongs in git is CLAUDE.md and AGENTS.md.
- Commit messages: plain imperative summaries, no tool attributions or emoji.
