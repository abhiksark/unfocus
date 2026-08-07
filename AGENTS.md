# Unfocus

Cross-platform eye-break reminder. Tauri 2 (Rust core) + SvelteKit 2 with
Svelte 5 runes, built with Bun.

```
src/         SvelteKit frontend: dashboard + break overlay (see src/AGENTS.md)
src-tauri/   Rust core: tray, windows, probes (see src-tauri/AGENTS.md)
scripts/     scene generator, container runner, and the version, toolchain,
             dependency, notices, SBOM, and release-artifact gates
static/      icon SVG and vendored fonts
```

Toolchains are pinned: Bun in `.bun-version`, Rust in `rust-toolchain.toml`.
`bun run toolchains:check` is the only place that proves those pins agree with
`package.json`, both workflows, and the container image — change a version in
one place and that check tells you the rest.

## Commands

Use Bun for all JS tooling. npm, npx, and pnpm are not used in this repo.

- `bun install` — install JS dependencies
- `bun run check` — svelte-check
- `bun run build` — production frontend build
- `bun run test` — frontend unit tests (`bun test`)
- `cargo test --manifest-path src-tauri/Cargo.toml` — Rust tests
- `bun run tauri dev` — run the app; on Linux needs native prerequisites
  (webkit2gtk-4.1, libayatana-appindicator3, libxdo), on macOS needs only
  the Xcode command line tools
- `./scripts/run-linux-spike-container.sh` — build and run in a container
  against the host X11 display when native headers are not installed. It
  reaches the host display and session bus; it is not a sandbox
- `bun run version:check` / `bun run version:set X.Y.Z` — the four version
  declarations must agree; never edit them by hand
- `bun run toolchains:check` — Bun and Rust pins agree everywhere
- `bun run dependencies:check` / `bun run dependencies:audit-rust` — advisory
  audits; anything unresolved needs an exact, expiring entry in
  `.github/dependency-exceptions.json`
- `bun run notices:check` / `bun run notices:generate` — THIRD_PARTY_NOTICES.txt
  is generated, never hand-edited
- `bun run sbom:generate <path>` — CycloneDX SBOM from the locked trees

Before claiming a change done, run the checks for the area you touched.
Frontend changes need `bun run test`, `bun run check`, and `bun run build` at
0 errors and 0 warnings. Rust changes need `cargo fmt --check`, then clippy and
test with `--all-targets --all-features --locked` and warnings denied.
Dependency, version, or toolchain changes need the matching gate above.
CI runs all of them plus a macOS and Windows compile check, and `Required CI`
fails unless every job passes.

Releases are cut by tagging a commit already contained in `main`. The tag is
rechecked against the declared version before anything is packaged, and the
publisher creates a draft pre-release; published releases are never
overwritten.

A tag may carry a prerelease label (`v0.1.0-rc1`) that the declared version
cannot: a Windows MSI ProductVersion has no way to express one, so `version:set`
still refuses anything but `X.Y.Z`. The tag's numeric core is compared exactly,
so `v0.2.0-rc1` is rejected against a declared `0.1.0` just as `v0.2.0` is.
Packages built under a prerelease tag are named for the declared version, so
`v0.1.0-rc1` and a later `v0.1.0` produce identically named files — tell them
apart by the release they hang off, not the filename.

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
