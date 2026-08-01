# Rust core

Tauri 2 backend: tray, window lifecycle, overlay orchestration, OS probes,
diagnostics.

- Probes (idle via XScreenSaver, fullscreen via EWMH) return a `Result` per
  poll. On failure, surface the error string in diagnostics and keep the timer
  running. A probe must never panic or change timer behavior.
- Overlay windows: one per monitor, borderless, always-on-top, skip-taskbar.
  The deadline is computed in Rust and encoded into the window label so every
  display derives presentation from the same clock. Closing one overlay closes
  its siblings; window-manager close of a single overlay must not strand the
  rest.
- Keep timing logic pure and clock-injected so tests run without real waits.
- Gate before committing: `cargo fmt --check`, `cargo clippy` with warnings
  denied, `cargo test`.
