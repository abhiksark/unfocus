# Unfocus

[![CI](https://github.com/abhiksark/unfocus/actions/workflows/ci.yml/badge.svg?branch=main)](https://github.com/abhiksark/unfocus/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/abhiksark/unfocus?include_prereleases&sort=semver)](https://github.com/abhiksark/unfocus/releases)
[![License: MIT](https://img.shields.io/badge/license-MIT-blue.svg)](LICENSE)
[![Qualified platform: Linux X11](https://img.shields.io/badge/qualified-Linux%20X11-4f8a66.svg)](#platform-support)
[![Built with Tauri 2](https://img.shields.io/badge/built%20with-Tauri%202-24c8db.svg)](https://v2.tauri.app)

**Take a real break. See your day without turning it into a score.**

Unfocus is a local-first desktop app that reminds you to look away from the
screen and gives you a calm, observe-only picture of how your day went. Breaks
cover every monitor, reflection stays on your device, and there are no streaks,
badges, accounts, or mascots competing for your attention.

[Download](https://github.com/abhiksark/unfocus/releases) ·
[Install guide](docs/install.md) ·
[Changelog](CHANGELOG.md) ·
[Report a bug](https://github.com/abhiksark/unfocus/issues/new?template=bug_report.yml) ·
[Request a feature](https://github.com/abhiksark/unfocus/issues/new?template=feature_request.yml)

![The Unfocus dashboard with the focus countdown, Your day activity, and break outcomes](.github/media/dashboard.png)

## Why Unfocus

A screen-break reminder should help you look away, not give you another
interface to watch. When a break is due, Unfocus places the same calm, static
landscape across every monitor and asks you to focus on something far away. As
the break ends, a warm amber light signals that it is time to return.

The dashboard then reflects the rhythm of your day using presence and break
outcomes. It does not grade your productivity, gamify consistency, or use that
history to control the reminder timer.

## Features

### Breaks that feel like breaks

- Covers every monitor with a synchronized full-screen break
- Uses a bundled, first-party landscape that stays still throughout the break
- Selects one cool-biased palette from local device time and holds it across
  every display; amber light signals that the break is nearly complete
- Defaults to a 20-minute work interval and a 20-second break, configurable
  from 1–120 minutes and 3–30 seconds
- Supports a fixed 30-minute pause, immediate breaks, break previews, and early
  exit with Escape or **End break**
- Uses idle and fullscreen signals where supported to avoid interrupting you
  while you are already away or presenting
- Keeps the timer running if a platform probe is unavailable or fails

![The bundled Unfocus break scene: a calm illustrated mountain valley](static/break-scene.jpg)

### Reflection without judgment

- **Your day** shows a rolling 24-hour view of active and away time
- History shows the newest 90 local days in a compact activity calendar, with
  hourly activity and break outcomes for a selected day
- Break outcomes distinguish shown breaks, natural rest, manual rest, and
  breaks held for fullscreen
- Presence comes only from keyboard and mouse idle signals; Unfocus never
  records which keys you press
- Reflection is observe-only: it never pauses, skips, or advances the timer

### Local by design

- No account, telemetry, cloud dependency, or packaged-app runtime network
  calls
- Settings, activity history, and break outcomes stay on your device
- The scene and typeface are bundled with the app; there are no asset or font
  downloads
- Developer mode exposes platform signals, connected displays, and probe
  errors when you need to troubleshoot

Selecting the linked author name is the only action that asks the operating
system to open a web address (`https://abhik.ai`) in your default browser.
Unfocus itself does not make that request over the network.

## Install

Unfocus is alpha software. Linux X11 is the only qualified platform today;
macOS and Windows builds are available for early testing.

### Ubuntu and Debian on X11

The preferred Linux installation is the signed public APT repository:

```sh
curl -fsSL https://apt.abhik.ai/public-key.asc \
  | sudo gpg --dearmor -o /usr/share/keyrings/unfocus-archive-keyring.gpg
echo 'deb [arch=amd64 signed-by=/usr/share/keyrings/unfocus-archive-keyring.gpg] https://apt.abhik.ai alpha main' \
  | sudo tee /etc/apt/sources.list.d/unfocus-alpha.list
sudo apt update
sudo apt install unfocus
```

Confirm that `echo $XDG_SESSION_TYPE` prints `x11`. On Ubuntu GNOME, choose
**Ubuntu on Xorg** at login if necessary and enable the Ubuntu AppIndicators
extension if the tray icon is missing.

### macOS

Install the alpha from the public Homebrew tap:

```sh
brew install --cask abhiksark/unfocus/unfocus@alpha
```

The macOS build is not notarized. On first launch, right-click Unfocus and
select **Open**. While running, Unfocus is intentionally available from the
menu-bar icon without a Dock icon or application menu.

### Other packages

[GitHub Releases](https://github.com/abhiksark/unfocus/releases) provides
`.deb`, `.rpm`, AppImage, Windows, and macOS packages. Verify downloaded assets
against `SHA256SUMS` before installing them. The
[full install guide](docs/install.md) covers package selection, verification,
first launch, updates, troubleshooting, and uninstalling.

## Platform support

Platform labels describe tested behavior, not just whether a package can be
built.

| Platform | Status | Notes |
| --- | --- | --- |
| Linux X11 | **Qualified** | Synchronized multi-monitor overlays, idle detection, fullscreen detection, and AppIndicator tray support. Ubuntu is the primary test environment. |
| macOS | **Preview** | Idle and fullscreen probes work interactively. Physical multi-monitor acceptance is not complete. |
| Windows x64 | **Early build** | Idle and fullscreen probes are implemented. Interactive multi-monitor qualification is pending. |
| Linux Wayland | **Unsupported** | Default packages do not provide Wayland probes. |

APT repository metadata is signed with the archive OpenPGP key; the alpha
packages themselves remain application-unsigned. The macOS app is not
code-signed or notarized, and Windows installers are not code-signed, so
platform warnings may appear. Unfocus does not auto-update; install updates
through APT, Homebrew, or a newer release package.

## Using Unfocus

1. Open the dashboard and choose your work interval and break duration.
2. Leave Unfocus running in the tray. You can pause reminders, resume them,
   take a break immediately, preview the overlay, or reopen the dashboard from
   the tray menu.
3. Open **Your day** for the current rhythm or **History** for earlier local
   activity and break outcomes.

Closing the dashboard normally leaves the reminder running in the tray.
Starting Unfocus again reveals the existing dashboard instead of starting a
second tray or timer. If tray setup fails, the dashboard reports the problem
and closing it exits the app so no unreachable process is left running.

## Build from source

Install the [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/)
for your platform, Bun `1.3.5`, and Rust `1.98.0`. The repository pins both
toolchains in `.bun-version` and `rust-toolchain.toml`. The app is built with
Tauri 2, Rust, SvelteKit 2, and Svelte 5.

```sh
git clone https://github.com/abhiksark/unfocus.git
cd unfocus
bun install --frozen-lockfile
bun run tauri dev
```

Before submitting application changes, run the local frontend and Rust gates:

```sh
bun run test
bun run check
bun run build
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked
```

Version, dependency, release, and generated-asset changes have additional
named checks in `package.json`.

## Contributing

Bug reports and focused pull requests are welcome. Normal development targets
`dev`; create a short-lived branch from current `dev` and open the pull request
back into `dev`. The protected `main` branch is reserved for releasable
promotions.

- [Report a bug](https://github.com/abhiksark/unfocus/issues/new?template=bug_report.yml)
- [Request a feature](https://github.com/abhiksark/unfocus/issues/new?template=feature_request.yml)
- [Review the changelog](CHANGELOG.md)

Please report suspected vulnerabilities privately through the
[security policy](.github/SECURITY.md), not in a public issue.

## License

- Unfocus source code and first-party break-scene artwork: [MIT](LICENSE)
- Vendored Newsreader font: [SIL Open Font License 1.1](static/fonts/OFL.txt)
- Dependency licenses and notices: [THIRD_PARTY_NOTICES.txt](THIRD_PARTY_NOTICES.txt)
