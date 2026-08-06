# Unfocus

[![CI](https://github.com/abhiksark/unfocus/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/abhiksark/unfocus/actions/workflows/ci.yml)
[![Release](https://github.com/abhiksark/unfocus/actions/workflows/release.yml/badge.svg)](https://github.com/abhiksark/unfocus/actions/workflows/release.yml)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Platforms](https://img.shields.io/badge/platforms-Linux%20X11%20%7C%20macOS-lightgrey.svg)](#status)
[![Built with Tauri 2](https://img.shields.io/badge/built%20with-Tauri%202-24c8db.svg)](https://v2.tauri.app)

A break reminder that asks one thing of you: look at something far away.

![The Unfocus break screen, a generated night landscape with a countdown](.github/media/break-resting.png)

Every twenty minutes, Unfocus dims every monitor into a quiet night scene and
counts down twenty seconds while your eyes rest on the farthest point you can
find. When the scene warms to amber, dawn is rising behind the ridge and the
break is almost over. No streaks, no badges, no mascot asking to be watched.
The whole point is that you stop looking at the screen.

![The same scene near the end of a break, with amber dawn light rising behind the summit](.github/media/break-returning.png)

## What it does

- Covers every monitor with a synchronized full-screen break, rendered as a
  generated landscape that moves too slowly to be worth watching
- Signals state through light: cool green while you rest, amber dawn when it
  is time to come back
- Ends early without a fight: press Escape or click once
- Reads idle time and fullscreen state so breaks can stay out of the way when
  you are already away from the desk or presenting (Linux X11 and macOS today)
- Runs local-first: no account, no telemetry, no network calls

## Status

Early development. The Linux X11 backend came first and already drives the
tray, the synchronized multi-monitor overlays, and the idle and fullscreen
probes. macOS now reads the same two signals through Quartz — idle from the
HID event source, fullscreen by measuring the frontmost window against each
display — though it has not yet had a multi-monitor acceptance run. Windows
shares the same Tauri core but has no probes yet, so it reports them as
unavailable rather than guessing. Wayland needs its own acceptance work and
is not supported yet.

![The diagnostics dashboard showing live probe data](.github/media/dashboard.png)

## Builds

CI runs on every push to `main` and every pull request. It builds on Ubuntu
22.04 and gates on five checks: `svelte-check`, the production frontend
build, `cargo fmt --check`, `cargo clippy` with warnings denied, and
`cargo test`. The badge above reflects that run.

Releases are cut by pushing a `v*` tag, which fans out to four targets —
Linux (`.deb`, `.AppImage`), Windows, macOS arm64, and macOS x86_64 — and
opens a draft GitHub release. **No tagged release exists yet**, so the
Release badge will read "no status" until the first one. Artifacts are not
code-signed or notarized; see the release notes for the per-platform
first-launch steps.

## Build from source

Install the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
for your platform — the listed system libraries on Linux, the Xcode command
line tools on macOS — plus [Bun](https://bun.sh), then:

```sh
bun install
bun run tauri dev
```

On an X11 machine without the native headers installed, the container runner
builds and runs against your real display and session bus:

```sh
./scripts/run-linux-spike-container.sh
```

Before sending changes, run the checks:

```sh
bun run check
bun run build
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml
```

## Design

The break screen is designed to lose a staring contest. The landscape is
generated once by `scripts/gen-scene.js` and baked into a single SVG; motion
is confined to a few slow, stepped layers, so the scene feels alive without
inviting attention, and every animation stops under `prefers-reduced-motion`.
If you change the scene, edit the generator and regenerate rather than
touching the SVG by hand.

## License

[MIT](LICENSE)
