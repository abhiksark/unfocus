# Rust core

This file applies to `src-tauri/**`. Follow the root `AGENTS.md` as well. Read
`src/AGENTS.md` for frontend-interface changes and `scripts/AGENTS.md` before
changing generated tray assets, dependencies, versions, or toolchain pins.

The Tauri 2 backend owns tray behavior, window lifecycle, overlay
orchestration, OS probes, timing, and diagnostics.

## Module ownership

- `src/lib.rs` is the composition root: module declarations, shared main-window
  authorization, Tauri setup and managed state, window-event routing, command
  registration, and exported `run()`.
- `src/activity.rs` owns pure continuous-activity / AFK segmentation from idle
  samples, local atomic history persistence, the rolling-window summary, and
  the `get_today_activity` and `get_activity_range` commands. Range buckets are
  main-window-only and serialize `activeMs`, `afkMs`, and `longestActiveMs`.
  This module never keylogs and never mutates the reminder timer.
- `src/activity_archive.rs` owns cold storage for activity segments aged out
  of the 24-hour hot file: fixed 30-day epoch chunk files
  (`activity-archive-<key>.json`, no calendar library), the archive write, the
  range read merged into `get_activity_range`, and chunk-level retention
  pruning.
- `src/break_ledger.rs` owns the local break-outcome ledger (shown, natural,
  fullscreen suppress, manual), atomic persistence, `get_break_summary`, and
  the main-window-only `get_break_range` command. Range reads return only
  privacy-safe chronological `{atMs, kind}` records. Ledger failures must not
  stop the reminder scheduler.
- `src/diagnostics.rs` owns diagnostics serialization, environment reporting,
  monitor enumeration, and the diagnostics command.
- `src/reminder.rs` owns the pure reminder timer, probe-based presentation
  decision (including optional activity-history adaptation that never mutates
  the clock), and scheduler thread (including observe-only activity sampling).
- `src/reminder/schedule.rs` owns pure wall-clock break-grid arithmetic for
  sync mode: the next grid point on the local-time epoch (`unix_secs +
  offset_minutes * 60`) and the half-interval grace rule. This coincides with
  local midnight only when the interval divides 86400. It has no calendar and
  no clock of its own; every input is injected. Its test table is mirrored
  byte-for-byte in `src/lib/break-grid.test.ts` and the two must be changed
  together.
- `src/tray.rs` owns tray construction, callbacks, embedded assets, and asset
  tests.
- `src/probes/mod.rs` owns probe caches, workers, snapshots, panic containment,
  stale-result handling, and platform dispatch. `src/probes/linux.rs` owns X11
  probes, Linux session routing, and validation; `src/probes/sway.rs` owns the
  opt-in Sway Wayland candidate; `src/probes/macos.rs` owns Quartz probes and
  fullscreen geometry; `src/probes/windows.rs` owns Win32 idle and fullscreen.
- `src/overlay/mod.rs` owns the overlay controller, command channel, worker,
  close-origin tracking, and commands. `src/overlay/lifecycle.rs` owns the pure
  lifecycle state machine and timeout calculation; `src/overlay/labels.rs`
  owns canonical labels, run IDs, bounds, deadlines, and caller authorization;
  `src/overlay/windows.rs` owns monitor windows, targeted events, sibling
  teardown, and startup-preview scheduling.

## Activity history: hot and cold storage

- `activity-history.json` is the hot file: it keeps its existing format, its
  24-hour live window, and its 30-second write throttle, unchanged by
  retention. The dashboard's two-second poll only ever scans that live set, so
  its cost does not grow with retention.
- `activity-archive-<key>.json` files are the cold path. Each file covers one
  fixed 30-day epoch block, keyed by `start_ms`, and lives beside the hot file
  in the same config directory.
- A segment becomes archivable exactly when the hot prune would otherwise drop
  it (`end_ms` at or before the 24-hour cutoff). It is archived to its
  30-day epoch chunk (`activity_archive::archive_segments`) before it leaves
  the hot set, on **both** paths that can find aged-out data: the live prune
  that runs on every observation, and the startup load path, since the app can
  sit closed for days with un-aged data still sitting in the hot file.
- The archive write must succeed before a segment is dropped from memory. A
  failed write keeps the segment hot for a later retry; a crash between the
  write and the drop cannot lose data.
- A segment straddling the 24-hour cutoff stays hot whole and is never
  truncated; the same rule applies at chunk boundaries — a segment is keyed by
  its `start_ms` even when it ends in the next chunk, and a range read also
  loads every preceding chunk so a straddler spanning one or more blocks is
  never missed.
- Retention deletes whole chunk files once their entire 30-day block is older
  than the cutoff, not individual segments. That makes effective retention run
  between the configured minimum and about one archive block longer, so state
  it as "at least ninety days," not an exact cutoff.
- Retention is forward-only. Existing installs cannot backfill activity from
  before this feature because only future hot-file prunes can populate archive
  chunks.
- Reading a range returns every segment overlapping the requested window,
  including the ones straddling either end. A corrupt or malformed chunk is
  skipped so the rest of the range still reads; only a single genuinely
  malformed segment shape (`end_ms < start_ms`) is skipped within an otherwise
  good chunk, never the whole chunk. An archive read or write failure never
  blocks reminders, matching the ledger's write-failures-never-stop-reminders
  contract.
- `get_activity_range` takes frontend-computed epoch-millisecond boundaries
  and only sums between them; local day/hour bucketing stays in the frontend
  because this crate has no calendar library and must not gain one.
- `get_activity_range` is main-window-only through `authorize_main_caller`.
  The frontend may request at most 1,024 buckets, and the total span must stay
  inside retained history.

## Break ledger history

- `break-events.json` is the local break-outcome ledger. It keeps scheduled
  shown, natural idle, fullscreen suppress, and manual take-break outcomes for
  at least 90 days, matching the activity history target.
- `get_break_summary` stays focused on the last day and last week. It is not
  the history page API.
- `get_break_range` is main-window-only through `authorize_main_caller`. It
  returns chronological privacy-safe `{atMs, kind}` records inside `[startMs,
  endMs)`, and rejects spans longer than 31 elapsed days.

## Probes and platform gates

- IMPORTANT: probes return a `Result` per poll. On failure, surface the error
  in diagnostics and keep the timer running. A probe must never panic, guess,
  or change timer behavior.
- `platform_probe` has one `cfg`-selected arm per platform. Linux defaults to
  X11 only: idle from XScreenSaver and fullscreen from EWMH `_NET_WM_STATE`.
  Wayland sessions error unless the non-default `wayland-sway` feature is
  built; that candidate positively identifies Sway 1.11+, `seat0`, and
  `ext_idle_notifier_v1` v2+ and never falls back to X11/XWayland properties.
  macOS reads idle from `CGEventSourceSecondsSinceLastEventType` and compares
  the frontmost layer-0 window's `kCGWindowBounds` with every active display.
  Windows reads idle from `GetLastInputInfo` and compares the foreground
  window outer rect to the monitor's `rcMonitor` (not the work area).
  Unsupported platforms return an error naming themselves.
- `src/probes/sway.rs` owns pure Sway qualification, IPC framing/tree parsing,
  input-idle baseline state, and (behind `wayland-sway`) the Linux runtime.
  Physical multi-monitor Sway acceptance is required before any support claim.
- Keep platform-independent logic outside the platform arms. The fullscreen
  rectangle comparison is gated with `any(target_os = "macos", test)`. Both
  halves are required: removing the gate trips `dead_code` under Linux clippy;
  narrowing it to macOS prevents its unit tests from compiling elsewhere.
- macOS window bounds and layer reads do not need Screen Recording consent.
  Do not introduce a probe that triggers a permission prompt without making
  that product change explicit.
- The Unfocus overlay remains above layer 0 so the macOS fullscreen probe does
  not see it. Otherwise a break could classify itself as fullscreen and
  suppress the next break.
- Keep timing logic pure and clock-injected so tests never require real waits.
  Test probe failure paths and prove that the timer continues.

## Lifecycle qualification (issue #30)

- `src/lifecycle_contract.rs` is test-only pure contract coverage for suspend
  stalls, topology policy, evidence-status, and release tiers (issue #30).
  Extend those tests when the shared contract changes; do not invent platform
  claims without physical evidence.
- Physical lifecycle and accessibility rows live in local `plans/` checklists
  and the GitHub **Platform report** form. Automated green is not acceptance.

## Overlay lifecycle and events

- Create one borderless, always-on-top, skip-taskbar overlay per monitor.
- Compute the deadline in Rust and encode it in the window label so every
  display derives presentation from the same clock.
- If any monitor window fails to build or finish loading, tear down the whole
  run immediately. Never leave a partial multi-monitor cover.
- Closing one overlay closes its siblings; closing one through the window
  manager (or losing a display that hosted an overlay) must never strand the
  rest. Unexpected window loss ends the entire run so the desk is never
  half-covered.
- Hotplug during a break does not spawn a new overlay mid-run. The next break
  re-enumerates monitors and covers the full set.
- `Emitter::emit` broadcasts regardless of the calling window, and an
  untargeted JavaScript `listen()` subscribes globally. Emit per window with
  `emit_to(EventTarget::webview_window(..))`, while the overlay subscribes with
  its own label. Both sides are required to prevent cross-run delivery.
- Multi-monitor acceptance is a real-hardware checklist under `plans/` (local
  only). Do not claim a platform multi-monitor-qualified without that evidence.

## Tray assets

- macOS embeds `icons/tray/tray-template.png` as a black-and-alpha template
  image. Other platforms embed `icons/tray/tray-light.png` as white-and-alpha.
  Unit tests enforce both invariants.
- Regenerate from `icons/tray/unfocus-tray.svg` with
  `bun run tray:generate`. Never hand-edit the PNGs or reintroduce an SVG mask
  into the glyph source.

## Checks

Run all three:

```sh
cargo fmt --check --manifest-path src-tauri/Cargo.toml
cargo clippy --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked -- -D warnings
cargo test --manifest-path src-tauri/Cargo.toml --all-targets --all-features --locked
```

Run the complete frontend gate from `src/AGENTS.md` as well for shared
interfaces and overlay behavior.
