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
  the `get_today_activity` command. It never keylogs and never mutates the
  reminder timer.
- `src/break_ledger.rs` owns the local break-outcome ledger (shown, natural,
  fullscreen suppress, manual), atomic persistence, and `get_break_summary`.
  Ledger failures must not stop the reminder scheduler.
- `src/diagnostics.rs` owns diagnostics serialization, environment reporting,
  monitor enumeration, and the diagnostics command.
- `src/reminder.rs` owns the pure reminder timer, probe-based presentation
  decision (including optional activity-history adaptation that never mutates
  the clock), and scheduler thread (including observe-only activity sampling).
- `src/tray.rs` owns tray construction, callbacks, embedded assets, and asset
  tests.
- `src/probes/mod.rs` owns probe caches, workers, snapshots, panic containment,
  stale-result handling, and platform dispatch. `src/probes/linux.rs` owns X11
  probes and validation; `src/probes/macos.rs` owns Quartz probes and fullscreen
  geometry.
- `src/overlay/mod.rs` owns the overlay controller, command channel, worker,
  close-origin tracking, and commands. `src/overlay/lifecycle.rs` owns the pure
  lifecycle state machine and timeout calculation; `src/overlay/labels.rs`
  owns canonical labels, run IDs, bounds, deadlines, and caller authorization;
  `src/overlay/windows.rs` owns monitor windows, targeted events, sibling
  teardown, and startup-preview scheduling.

## Probes and platform gates

- IMPORTANT: probes return a `Result` per poll. On failure, surface the error
  in diagnostics and keep the timer running. A probe must never panic, guess,
  or change timer behavior.
- `platform_probe` has one `cfg`-selected arm per platform. Linux support is
  limited to X11: it reads idle from XScreenSaver and fullscreen from EWMH
  `_NET_WM_STATE`; Wayland is unsupported. macOS reads idle from
  `CGEventSourceSecondsSinceLastEventType` and compares the frontmost layer-0
  window's `kCGWindowBounds` with every active display. Windows reads idle
  from `GetLastInputInfo` and compares the foreground window outer rect to the
  monitor's `rcMonitor` (not the work area). Unsupported platforms return an
  error naming themselves.
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
