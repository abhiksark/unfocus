use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicU64, Ordering},
        Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindowBuilder,
};

static OVERLAY_RUN_ID: AtomicU64 = AtomicU64::new(1);
static OVERLAY_START_LOCK: Mutex<()> = Mutex::new(());
const OVERLAY_DISMISS_DELAY: Duration = Duration::from_millis(500);
const OVERLAY_COMPLETION_GRACE: Duration = Duration::from_millis(1_250);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MonitorReport {
    name: Option<String>,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_factor: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct DiagnosticsReport {
    operating_system: &'static str,
    session_type: Option<String>,
    desktop: Option<String>,
    display: Option<String>,
    monitors: Vec<MonitorReport>,
    idle_seconds: Option<u64>,
    idle_error: Option<String>,
    active_window_fullscreen: Option<bool>,
    fullscreen_error: Option<String>,
}

#[cfg(target_os = "linux")]
mod linux_probe {
    use x11rb::{
        connection::Connection,
        protocol::{
            screensaver::ConnectionExt as ScreensaverConnectionExt,
            xproto::{AtomEnum, ConnectionExt as XprotoConnectionExt, Window},
        },
        rust_connection::RustConnection,
    };

    fn connect() -> Result<(RustConnection, usize), String> {
        x11rb::connect(None).map_err(|error| format!("X11 connection failed: {error}"))
    }

    fn atom(connection: &RustConnection, name: &[u8]) -> Result<u32, String> {
        connection
            .intern_atom(false, name)
            .map_err(|error| format!("could not request X11 atom: {error}"))?
            .reply()
            .map(|reply| reply.atom)
            .map_err(|error| format!("could not read X11 atom: {error}"))
    }

    fn root(connection: &RustConnection, screen_number: usize) -> Window {
        connection.setup().roots[screen_number].root
    }

    pub fn idle_seconds() -> Result<u64, String> {
        let (connection, screen_number) = connect()?;
        let reply = connection
            .screensaver_query_info(root(&connection, screen_number))
            .map_err(|error| format!("XScreenSaver query failed: {error}"))?
            .reply()
            .map_err(|error| format!("XScreenSaver reply failed: {error}"))?;

        Ok(u64::from(reply.ms_since_user_input) / 1_000)
    }

    pub fn active_window_fullscreen() -> Result<bool, String> {
        let (connection, screen_number) = connect()?;
        let root = root(&connection, screen_number);
        let active_window_atom = atom(&connection, b"_NET_ACTIVE_WINDOW")?;
        let window_state_atom = atom(&connection, b"_NET_WM_STATE")?;
        let fullscreen_atom = atom(&connection, b"_NET_WM_STATE_FULLSCREEN")?;

        let active_window_reply = connection
            .get_property(false, root, active_window_atom, AtomEnum::WINDOW, 0, 1)
            .map_err(|error| format!("active-window query failed: {error}"))?
            .reply()
            .map_err(|error| format!("active-window reply failed: {error}"))?;

        let Some(active_window) = active_window_reply
            .value32()
            .and_then(|mut values| values.next())
        else {
            return Ok(false);
        };

        let state_reply = connection
            .get_property(
                false,
                active_window,
                window_state_atom,
                AtomEnum::ATOM,
                0,
                u32::MAX,
            )
            .map_err(|error| format!("window-state query failed: {error}"))?
            .reply()
            .map_err(|error| format!("window-state reply failed: {error}"))?;

        Ok(state_reply
            .value32()
            .is_some_and(|mut states| states.any(|state| state == fullscreen_atom)))
    }
}

#[cfg(not(target_os = "linux"))]
mod linux_probe {
    pub fn idle_seconds() -> Result<u64, String> {
        Err("the Linux X11 probe is unavailable on this operating system".into())
    }

    pub fn active_window_fullscreen() -> Result<bool, String> {
        Err("the Linux X11 probe is unavailable on this operating system".into())
    }
}

fn environment_value(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

fn monitor_report(
    name: Option<&str>,
    position: &PhysicalPosition<i32>,
    size: &PhysicalSize<u32>,
    scale_factor: f64,
) -> MonitorReport {
    MonitorReport {
        name: name.map(str::to_owned),
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
        scale_factor,
    }
}

#[tauri::command]
fn get_diagnostics(app: AppHandle) -> DiagnosticsReport {
    let monitors = app
        .available_monitors()
        .unwrap_or_default()
        .iter()
        .map(|monitor| {
            monitor_report(
                monitor.name().map(String::as_str),
                monitor.position(),
                monitor.size(),
                monitor.scale_factor(),
            )
        })
        .collect();

    let (idle_seconds, idle_error) = match linux_probe::idle_seconds() {
        Ok(seconds) => (Some(seconds), None),
        Err(error) => (None, Some(error)),
    };
    let (active_window_fullscreen, fullscreen_error) = match linux_probe::active_window_fullscreen()
    {
        Ok(fullscreen) => (Some(fullscreen), None),
        Err(error) => (None, Some(error)),
    };

    DiagnosticsReport {
        operating_system: std::env::consts::OS,
        session_type: environment_value("XDG_SESSION_TYPE"),
        desktop: environment_value("XDG_CURRENT_DESKTOP"),
        display: environment_value("DISPLAY"),
        monitors,
        idle_seconds,
        idle_error,
        active_window_fullscreen,
        fullscreen_error,
    }
}

fn close_overlay_windows(app: &AppHandle, prefix: Option<&str>, reason: &str) {
    for (label, window) in app.webview_windows() {
        if label.starts_with("overlay-") && prefix.is_none_or(|prefix| label.starts_with(prefix)) {
            eprintln!("closing overlay {label}: {reason}");
            if let Err(error) = window.close() {
                eprintln!("could not close overlay {label}: {error}");
            }
        }
    }
}

fn begin_overlay_close(app: &AppHandle, prefix: &str) -> Result<(), String> {
    let mut matched = false;
    for (label, window) in app.webview_windows() {
        if label.starts_with(prefix) {
            matched = true;
            if let Err(error) = window.emit("unfocus-overlay-closing", ()) {
                eprintln!("could not animate overlay {label} closed: {error}");
            }
        }
    }

    if !matched {
        return Err("the overlay preview has already closed".into());
    }

    let app = app.clone();
    let prefix = prefix.to_owned();
    std::thread::spawn(move || {
        std::thread::sleep(OVERLAY_DISMISS_DELAY);
        close_overlay_windows(&app, Some(&prefix), "preview dismissed");
    });

    Ok(())
}

fn bounded_overlay_duration(seconds: u64) -> u64 {
    seconds.clamp(3, 30)
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn overlay_label(
    run_id: u64,
    index: usize,
    total: usize,
    duration_seconds: u64,
    deadline_ms: u64,
) -> String {
    format!("overlay-{run_id}-{index}-{total}-{duration_seconds}-{deadline_ms}")
}

fn show_overlay_test_impl(app: &AppHandle, duration_seconds: u64) -> Result<usize, String> {
    let _start_guard = OVERLAY_START_LOCK
        .lock()
        .map_err(|_| "overlay preview start lock is poisoned".to_owned())?;

    close_overlay_windows(app, None, "superseded by a new preview");

    let monitors = app
        .available_monitors()
        .map_err(|error| format!("could not enumerate monitors: {error}"))?;
    if monitors.is_empty() {
        return Err("Tauri did not report any monitors".into());
    }

    let duration_seconds = bounded_overlay_duration(duration_seconds);
    let run_id = OVERLAY_RUN_ID.fetch_add(1, Ordering::Relaxed);
    let prefix = format!("overlay-{run_id}-");
    let total = monitors.len();
    let deadline_ms = unix_timestamp_ms().saturating_add(duration_seconds.saturating_mul(1_000));
    let close_at =
        Instant::now() + Duration::from_secs(duration_seconds) + OVERLAY_COMPLETION_GRACE;

    for (index, monitor) in monitors.iter().enumerate() {
        let scale_factor = monitor.scale_factor();
        let position = monitor.position();
        let size = monitor.size();
        let label = overlay_label(run_id, index, total, duration_seconds, deadline_ms);

        let build_result =
            WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
                .title("Unfocus overlay feasibility test")
                .position(
                    f64::from(position.x) / scale_factor,
                    f64::from(position.y) / scale_factor,
                )
                .inner_size(
                    f64::from(size.width) / scale_factor,
                    f64::from(size.height) / scale_factor,
                )
                .decorations(false)
                .resizable(false)
                .closable(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .background_color(tauri::webview::Color(7, 19, 16, 255))
                .build();

        if let Err(error) = build_result {
            close_overlay_windows(app, Some(&prefix), "preview startup failed");
            return Err(format!("could not create overlay {index}: {error}"));
        }
    }

    eprintln!("opened overlay run {run_id} on {total} display(s) for {duration_seconds} second(s)");

    let app_for_timeout = app.clone();
    std::thread::spawn(move || {
        std::thread::sleep(close_at.saturating_duration_since(Instant::now()));
        close_overlay_windows(&app_for_timeout, Some(&prefix), "preview duration elapsed");
    });

    Ok(total)
}

#[tauri::command]
fn show_overlay_test(app: AppHandle, duration_seconds: u64) -> Result<usize, String> {
    show_overlay_test_impl(&app, duration_seconds)
}

#[tauri::command]
fn close_overlay_test(app: AppHandle, run_id: u64) -> Result<(), String> {
    if run_id == 0 {
        return Err("invalid overlay preview identifier".into());
    }

    begin_overlay_close(&app, &format!("overlay-{run_id}-"))
}

fn install_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Unfocus", true, None::<&str>)?;
    let overlay = MenuItem::with_id(
        app,
        "overlay",
        "Test overlays (8 seconds)",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &overlay, &quit])?;

    TrayIconBuilder::new()
        .icon(
            app.default_window_icon()
                .expect("the application bundle must contain an icon")
                .clone(),
        )
        .tooltip("Unfocus Linux feasibility spike")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "overlay" => {
                if let Err(error) = show_overlay_test_impl(app, 8) {
                    eprintln!("overlay test failed: {error}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

fn schedule_automatic_overlay_test(app: &tauri::App) {
    if environment_value("UNFOCUS_SPIKE_AUTO_OVERLAY").as_deref() != Some("1") {
        return;
    }

    let duration_seconds = environment_value("UNFOCUS_SPIKE_OVERLAY_SECONDS")
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);
    let app = app.handle().clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        match show_overlay_test_impl(&app, duration_seconds) {
            Ok(count) => eprintln!("automatic overlay test opened {count} window(s)"),
            Err(error) => eprintln!("automatic overlay test failed: {error}"),
        }
    });
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            install_tray(app)?;
            schedule_automatic_overlay_test(app);
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_diagnostics,
            show_overlay_test,
            close_overlay_test
        ])
        .run(tauri::generate_context!())
        .expect("error while running Unfocus");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_reports_preserve_physical_topology() {
        let report = monitor_report(
            Some("DP-4"),
            &PhysicalPosition::new(-1920, 240),
            &PhysicalSize::new(1920, 1080),
            1.25,
        );

        assert_eq!(report.name.as_deref(), Some("DP-4"));
        assert_eq!((report.x, report.y), (-1920, 240));
        assert_eq!((report.width, report.height), (1920, 1080));
        assert_eq!(report.scale_factor, 1.25);
    }

    #[test]
    fn overlay_test_duration_has_safe_bounds() {
        assert_eq!(bounded_overlay_duration(0), 3);
        assert_eq!(bounded_overlay_duration(8), 8);
        assert_eq!(bounded_overlay_duration(300), 30);
    }

    #[test]
    fn overlay_labels_share_an_absolute_deadline() {
        assert_eq!(
            overlay_label(7, 1, 2, 20, 1_800_000_000_000),
            "overlay-7-1-2-20-1800000000000"
        );
    }
}
