# Changelog

Notable changes to Unfocus are recorded here. Dates use `YYYY-MM-DD`, and
released versions link to their GitHub release.

## [Unreleased]

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

[Unreleased]: https://github.com/abhiksark/unfocus/compare/v0.1.0-alpha.1...dev
[0.1.0-alpha.1]: https://github.com/abhiksark/unfocus/releases/tag/v0.1.0-alpha.1
