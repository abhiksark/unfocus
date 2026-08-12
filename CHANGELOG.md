# Changelog

Notable changes to Unfocus are recorded here. Dates use `YYYY-MM-DD`, and
released versions link to their GitHub release.

## [Unreleased]

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
- Implemented Windows idle (`GetLastInputInfo`) and foreground fullscreen
  probes (window outer rect vs monitor `rcMonitor`). Windows remains an early
  build until interactive qualification is recorded.
- Scaffolded an opt-in `wayland-sway` Cargo feature for a Sway 1.11+ Wayland
  probe candidate (Sway IPC fullscreen, `ext_idle_notifier_v1` input-idle,
  positive runtime gates). Default packages still treat Wayland as unsupported;
  no multi-monitor or release qualification is claimed.

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

[Unreleased]: https://github.com/abhiksark/unfocus/compare/v0.2.0-alpha.1...dev
[0.2.0-alpha.1]: https://github.com/abhiksark/unfocus/compare/v0.1.0-alpha.1...v0.2.0-alpha.1
[0.1.0-alpha.1]: https://github.com/abhiksark/unfocus/releases/tag/v0.1.0-alpha.1
