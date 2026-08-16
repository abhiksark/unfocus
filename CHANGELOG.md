# Changelog

Notable changes to Unfocus are recorded here. Dates use `YYYY-MM-DD`, and
released versions link to their GitHub release.

## [0.5.0-alpha.1] - 2026-08-16

### Added

- Added `© 2026 Abhik Sarkar` to the foot of the consumer dashboard, with only
  **Abhik Sarkar** linked. Explicitly activating the name asks the system browser
  to open the fixed `https://abhik.ai` address rather than navigating the
  dashboard away, and says so plainly if no browser could be opened. Unfocus
  itself still makes no application-originated runtime network call.
- Kept raw **Your day** presence history on this device for at least 90 days,
  beyond the 24-hour live window the strip already shows. The existing
  `activity-history.json` file stays hot for the latest day, and segments that
  age out of it are archived into local `activity-archive-<key>.json` 30-day
  epoch chunks in the same config directory. This stays local-only like
  everything else in Unfocus, with no account and no network call. History
  fills forward only: compatible existing local data is preserved, but activity
  from before History was installed cannot be backfilled. Whole archive chunks
  expire together, so an older partial chunk can remain until its complete
  30-day block expires.
- Added main-window-only range history commands for the consumer dashboard.
  `get_activity_range` returns bucketed `activeMs`, `afkMs`, and
  `longestActiveMs` values for caller-provided boundaries, and
  `get_break_range` returns chronological privacy-safe `{atMs, kind}` break
  outcomes for `[start, end)` queries up to 31 elapsed days.
- Added a transient consumer History view exposing the newest 90 browser-local
  days as three newest-first 30-day pages. It follows the saved **Day starts**
  preference, loads on demand, shows activity and break outcomes together, and
  does not change the reminder timer, probes, or overlays.

### Changed

- Raised the break-outcome ledger's retention from 7 to at least 90 days and
  removed its fixed event cap, matching the activity history above.
  `break-events.json` is now bounded by the 90-day time window and keeps enough
  local break outcomes to back the history view as well as the existing day
  and week summaries. The old 512-event cap was a live defect: a 20-minute
  work interval can produce up to 72 scheduled breaks a day, and with
  natural-idle and fullscreen-suppress outcomes added in, the cap could
  already be reached inside the old 7-day window, silently undercounting the
  dashboard's week totals.

## [0.4.0-alpha.1] - 2026-08-15

Supersedes the unpublished `v0.3.1-alpha.1` draft. This public alpha includes all of that draft’s dashboard, accessibility, and release-integrity improvements.

### Added

- Added a full multi-platform [install guide](docs/install.md) covering package
  selection, checksum and provenance verification, first-run defaults, local
  data and uninstall paths, build-from-source pins, and platform troubleshooting.
- Added an elapsed-progress indicator to the consumer dashboard. It appears only
  during an active work interval and stays hidden while paused, during a break
  or preview, and whenever reminder status is unavailable.
- Added a time axis to the **Your day** activity strip. Faint gridlines mark
  every fourth hour behind the bars, with the hour labeled beneath in the
  system clock format and the right edge marked `now`, so a bar can be traced
  to the time of day it covers.
- Added a **Day starts** setting to the **Your day** summary. The chosen hour is
  marked with a stronger rule and a brighter label on the activity strip's time
  axis, so the day can be read from when it actually begins. The setting is kept
  on this device and never affects the break timer.
- Added a public **APT** install path for Debian and Ubuntu alphas via
  [abhiksark/unfocus-apt](https://github.com/abhiksark/unfocus-apt) (`apt install
  unfocus`). The archive metadata at [apt.abhik.ai](https://apt.abhik.ai) is
  signed; application binaries remain unsigned.
- Added slow, stepped drift and twinkle to the break scene's stars, with a
  10-18 second cadence and a dawn fade. Reduced-motion preferences stop the
  motion entirely.

### Changed

- Rebuilt the consumer dashboard as a quieter single column with plain sections,
  shared design tokens, system body fonts, sentence-case labels, and WCAG AA
  contrast at rendered text sizes. The break screen keeps its layout and uses
  the shared display font token.
- Described Unfocus as both a break app and a local reflection surface in the
  README and package metadata. **Your day** and break outcomes remain
  observe-only.
- Clarified Ubuntu as the primary qualified Linux install path: README and
  [docs/install.md](docs/install.md) put the signed APT repo
  ([apt.abhik.ai](https://apt.abhik.ai)) first, separate package trust from
  macOS/Windows application code signing, and document APT upgrade and privacy
  notes for that path.
- Updated the README consumer dashboard screenshot to the single-column layout
  with the **Your day** time axis and **Day starts** selector. Break-screen media
  remains unchanged.
- Updated the frontend toolchain to Vite 8.2.1, svelte-check 4.7.5, the Svelte
  Vite plugin 7.3.0, and Node type definitions 26.2.0, and regenerated the
  dependency license notices.

### Fixed

- Fixed SBOM generation when a dependency update leaves two copies of a package
  in the lockfile. Each dependency now resolves the way the runtime would, so a
  nested copy serves its own owner while everything else sees the root copy.
- Relayed Homebrew release dispatch through the default branch so the
  environment-restricted tap automation can receive alpha publication events.

### Security

- Overrode the transitive `nanoid` dependency to 3.3.18 to remediate its
  advisory, and regenerated the lockfile and third-party notices.

## [0.3.1-alpha.1] - 2026-08-12

Tagged but unpublished. Its dashboard, accessibility, dependency, and SBOM
changes are included in and superseded by [0.4.0-alpha.1].

## [0.3.0-alpha.1] - 2026-08-12

### Added

- Added an observe-only **Your day** summary on the consumer dashboard: rolling
  last-24-hour active and away totals, longest continuous stretch, deep-block
  count, and a half-hour presence strip derived from the idle probe without
  keylogging. Probe failures freeze classification and never change the break
  timer.
- Persisted activity segments locally across restarts (atomic JSON next to app
  settings, pruned to the rolling window). Write failures keep the previous
  complete history and leave the reminder timer unchanged.
- Added a local break-event ledger for scheduled shown, natural idle, fullscreen
  suppress, and manual take-break outcomes, with calm dashboard counts for the
  last day and week.
- Adapted scheduled break presentation from continuous activity and AFK history
  without changing the pure timer: after a long active stretch a short micro-idle
  no longer natural-credits; long AFK still prefers quiet natural credit when
  idle; fullscreen is never overridden.
- Credited a due break as a natural rest when the user has already been idle for
  the break duration, instead of running a silent break phase.
- Added pure lifecycle and accessibility contract tests that pin release-tier
  evidence rules (timer stalls, multi-monitor topology policy, and a11y
  presentation) without inventing platform qualification.
- Implemented Windows idle (`GetLastInputInfo`) and foreground fullscreen
  probes (window outer rect vs monitor `rcMonitor`). Windows remains an early
  build until interactive qualification is recorded.
- Scaffolded an opt-in `wayland-sway` Cargo feature for a Sway 1.11+ Wayland
  probe candidate (Sway IPC fullscreen, `ext_idle_notifier_v1` input-idle,
  positive runtime gates). Default packages still treat Wayland as unsupported;
  no multi-monitor or release qualification is claimed.

### Changed

- Polished the consumer **Your day** and **Break outcomes** card: clearer
  loading, empty, and error copy; muted zero counts; strip legend; less
  redundant captions. Still observe-only with no timer control.
- Documented platform status honestly for this alpha: Linux X11 remains the
  only qualified backend; macOS is preview without multi-monitor acceptance;
  Windows packages include probes but interactive multi-monitor qualification
  is pending; Wayland is unsupported in default packages; packages are not
  code-signed or notarized.

## [0.2.0-alpha.1] - 2026-08-11

### Added

- Added a generated, monochrome system-tray icon with platform-specific light
  and template variants.
- Added single-instance activation so reopening Unfocus reveals the existing
  dashboard without starting another tray or reminder timer.
- Added a live tray status row and native controls for a bounded thirty-minute
  pause, resume, and an immediate configured break, updated at meaningful
  minute and state boundaries.
- Added the same reminder status and controls to the dashboard. Pause expiry is
  stored locally, remains bounded across restarts, and resumes into a fresh
  work interval.
- Added a private vulnerability-reporting policy.

### Changed

- Replaced the diagnostics-first startup screen with a scene-led consumer
  dashboard. The previous technical dashboard remains available through the
  timing editor's **Advanced** section, and the selected view is remembered
  locally.
- A known tray setup failure now appears in local diagnostics and keeps the
  dashboard reachable; closing the dashboard exits instead of hiding an
  unreachable background reminder.
- Updated the frontend toolchain to Vite 8, TypeScript 7, and the Svelte Vite
  plugin 7, and updated `x11rb` to 0.14.
- Updated the GitHub Actions used for checkout, artifact transfer, and build
  provenance.
- Improved SBOM platform metadata and regenerated dependency license notices.
- Clarified issue forms for current package and platform-support reporting.
- Documented the `dev`-first branching strategy, release workflow, download
  choices, platform status, and contributor checks.
- Expanded CI and release Rust tests to cover all targets and features.
- Normalized the embedded Debian package version so prereleases upgrade in
  Debian's intended alpha, beta, release-candidate, and stable order.
- Recorded Linux tray acceptance on Ubuntu 22.04.5 GNOME 42.9 X11 with
  Ubuntu AppIndicators 42-2~fakesync1: the AppImage ran on the host, while the
  Debian package was installed in Debian 12 and the RPM in Fedora 43 against
  that same physical two-display X11 session. Wayland remains unsupported.

## [0.1.0-alpha.1] - 2026-08-07

### Added

- Published the first Unfocus alpha packages for Linux X11, macOS, and
  Windows.
- Added synchronized full-screen break overlays, the diagnostics dashboard,
  the system tray, and the generated night landscape.
- Added Linux X11 idle and fullscreen probes and initial macOS Quartz probes.
- Added the containerized X11 development runner.
- Added CI and draft prerelease packaging for Linux, macOS, and Windows.
- Added checksums, build-provenance attestations, a CycloneDX SBOM, and bundled
  third-party notices to release artifacts.

### Changed

- Made release artifacts retain the complete prerelease version in their
  filenames while keeping the Windows MSI ProductVersion numeric-only.
- Pinned toolchains and added gates for version agreement, dependencies,
  licenses, generated metadata, and release integrity.
- Made unavailable or failing platform probes report their state without
  stopping or changing the break timer.

[0.5.0-alpha.1]: https://github.com/abhiksark/unfocus/compare/v0.4.0-alpha.1...v0.5.0-alpha.1
[0.4.0-alpha.1]: https://github.com/abhiksark/unfocus/compare/v0.3.0-alpha.1...v0.4.0-alpha.1
[0.3.1-alpha.1]: https://github.com/abhiksark/unfocus/compare/v0.3.0-alpha.1...v0.3.1-alpha.1
[0.3.0-alpha.1]: https://github.com/abhiksark/unfocus/compare/v0.2.0-alpha.1...v0.3.0-alpha.1
[0.2.0-alpha.1]: https://github.com/abhiksark/unfocus/compare/v0.1.0-alpha.1...v0.2.0-alpha.1
[0.1.0-alpha.1]: https://github.com/abhiksark/unfocus/releases/tag/v0.1.0-alpha.1
