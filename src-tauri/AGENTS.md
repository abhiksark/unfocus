# Rust core

This file applies to `src-tauri/**`. Follow the root `AGENTS.md` as well. Read
`src/AGENTS.md` for frontend-interface changes and `scripts/AGENTS.md` before
changing generated tray assets, dependencies, versions, or toolchain pins.

The Tauri 2 backend owns tray behavior, window lifecycle, overlay
orchestration, OS probes, timing, and diagnostics.

## Probes and platform gates

- IMPORTANT: probes return a `Result` per poll. On failure, surface the error
  in diagnostics and keep the timer running. A probe must never panic, guess,
  or change timer behavior.
- `platform_probe` has one `cfg`-selected arm per platform. Linux support is
  limited to X11: it reads idle from XScreenSaver and fullscreen from EWMH
  `_NET_WM_STATE`; Wayland is unsupported. macOS reads idle from
  `CGEventSourceSecondsSinceLastEventType` and compares the frontmost layer-0
  window's `kCGWindowBounds` with every active display. Windows has no probes,
  and unsupported platforms return an error naming themselves.
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

## Overlay lifecycle and events

- Create one borderless, always-on-top, skip-taskbar overlay per monitor.
- Compute the deadline in Rust and encode it in the window label so every
  display derives presentation from the same clock.
- Closing one overlay closes its siblings; closing one through the window
  manager must never strand the rest.
- `Emitter::emit` broadcasts regardless of the calling window, and an
  untargeted JavaScript `listen()` subscribes globally. Emit per window with
  `emit_to(EventTarget::webview_window(..))`, while the overlay subscribes with
  its own label. Both sides are required to prevent cross-run delivery.

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
