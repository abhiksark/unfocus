# Unfocus

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
  you are already away from the desk or presenting (X11 today)
- Runs local-first: no account, no telemetry, no network calls

## Status

Early development. The Linux X11 backend came first and already drives the
tray, the synchronized multi-monitor overlays, and the idle and fullscreen
probes. Windows and macOS share the same Tauri core and are next. Wayland
needs its own acceptance work and is not supported yet.

![The diagnostics dashboard showing live probe data](.github/media/dashboard.png)

## Build from source

Install the [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/)
and [Bun](https://bun.sh), then:

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
