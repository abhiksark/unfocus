# Rust core

Tauri 2 backend: tray, window lifecycle, overlay orchestration, OS probes,
diagnostics.

- IMPORTANT: probes return a `Result` per poll. On failure, surface the error
  string in diagnostics and keep the timer running. A probe must never panic
  or change timer behavior.
- `platform_probe` has one `cfg`-selected arm per platform. Linux reads idle
  from XScreenSaver and fullscreen from EWMH `_NET_WM_STATE`. macOS reads
  idle from `CGEventSourceSecondsSinceLastEventType` and fullscreen by
  comparing the frontmost layer-0 window's `kCGWindowBounds` against every
  active display. Every other platform returns an error naming itself.
- Keep platform-independent logic out of those arms — the fullscreen rect
  comparison lives at module level gated `any(target_os = "macos", test)`.
  Both halves of that gate are load-bearing on Linux CI: drop the `cfg`
  entirely and the unused items trip `dead_code` under `clippy -D warnings`;
  narrow it to `macos` alone and the unit tests stop compiling because the
  items they call no longer exist.
- macOS window bounds and layer read without Screen Recording consent. Do not
  introduce a probe that needs that permission without saying so loudly; it
  turns a silent degrade into a prompt.
- Unfocus's own overlay sits above layer 0, so the macOS fullscreen probe
  does not see it. Preserve that — a break that reads as a fullscreen app
  would suppress the break after it.
- Overlay windows: one per monitor, borderless, always-on-top, skip-taskbar.
  The deadline is computed in Rust and encoded into the window label so every
  display derives presentation from the same clock. Closing one overlay closes
  its siblings; window-manager close of a single overlay must not strand the
  rest.
- Keep timing logic pure and clock-injected so tests run without real waits.
- `Emitter::emit` delivers to every target regardless of which window it is
  called on, and a JS `listen()` with no `target` option subscribes to all of
  them. Overlay events therefore go out per window through
  `emit_to(EventTarget::webview_window(..))`, and the overlay subscribes with
  its own label. Both halves are required: either one alone leaves every
  overlay receiving every run's events.
- The tray icon is per-platform: macOS embeds `icons/tray/tray-template.png`
  as a template image (black + alpha only), other platforms embed
  `icons/tray/tray-light.png` (white + alpha). Unit tests enforce both
  invariants. Regenerate with `bun run tray:generate`; never hand-edit the
  PNGs or reintroduce an SVG mask into the glyph source.
- Gate before committing: `cargo fmt --check`, then `cargo clippy` and
  `cargo test` with `--all-targets --all-features --locked` and warnings
  denied.
