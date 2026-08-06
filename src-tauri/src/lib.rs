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

/// A rectangle in Quartz screen coordinates (points, top-left origin).
///
/// Only the macOS probe produces these, but the comparison logic below stays
/// platform-independent so it can be unit tested on any host.
#[cfg(any(target_os = "macos", test))]
#[derive(Debug, Clone, Copy, PartialEq)]
struct ScreenRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Quartz reports geometry as floating point points, and a fullscreen window's
/// bounds are not always bit-identical to its display's. Allow a point of slack
/// rather than demanding exact equality.
#[cfg(any(target_os = "macos", test))]
const FULLSCREEN_TOLERANCE: f64 = 1.0;

/// True when `window` covers `display` edge to edge.
///
/// A macOS window that is merely zoomed still leaves the menu bar visible, so
/// its height falls short of the display and it is correctly not fullscreen.
#[cfg(any(target_os = "macos", test))]
fn window_covers_display(window: ScreenRect, display: ScreenRect) -> bool {
    (window.x - display.x).abs() <= FULLSCREEN_TOLERANCE
        && (window.y - display.y).abs() <= FULLSCREEN_TOLERANCE
        && (window.width - display.width).abs() <= FULLSCREEN_TOLERANCE
        && (window.height - display.height).abs() <= FULLSCREEN_TOLERANCE
}

/// True when `window` covers any one of `displays` entirely.
#[cfg(any(target_os = "macos", test))]
fn window_is_fullscreen(window: ScreenRect, displays: &[ScreenRect]) -> bool {
    displays
        .iter()
        .any(|display| window_covers_display(window, *display))
}

#[cfg(target_os = "linux")]
mod platform_probe {
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

#[cfg(target_os = "macos")]
mod platform_probe {
    use super::{window_is_fullscreen, ScreenRect};
    use objc2_core_foundation::{CFDictionary, CFNumber, CFNumberType, CGPoint, CGRect, CGSize};
    use objc2_core_graphics::{
        kCGWindowBounds, kCGWindowLayer, CGDirectDisplayID, CGDisplayBounds, CGError,
        CGEventSource, CGEventSourceStateID, CGEventType, CGGetActiveDisplayList,
        CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption,
    };
    use std::ffi::c_void;

    /// `kCGAnyInputEventType`, which Apple defines as `(uint32_t)~0`. The
    /// generated bindings do not export the constant, so it is named here.
    const ANY_INPUT_EVENT: CGEventType = CGEventType(u32::MAX);

    /// Normal application windows sit on layer 0; menu bar, dock and overlay
    /// windows sit above it. Only layer 0 can be the active window.
    const NORMAL_WINDOW_LAYER: i64 = 0;

    /// `kCGNullWindowID` — list windows relative to no particular window.
    const NULL_WINDOW: u32 = 0;

    pub fn idle_seconds() -> Result<u64, String> {
        let seconds = CGEventSource::seconds_since_last_event_type(
            CGEventSourceStateID::HIDSystemState,
            ANY_INPUT_EVENT,
        );

        if !seconds.is_finite() || seconds < 0.0 {
            return Err(format!(
                "Quartz reported an implausible idle time: {seconds}"
            ));
        }

        // Rust saturates float-to-integer casts, and the value is already
        // known finite and non-negative.
        Ok(seconds as u64)
    }

    pub fn active_window_fullscreen() -> Result<bool, String> {
        let displays = active_display_bounds()?;
        if displays.is_empty() {
            return Err("Quartz reported no active displays".into());
        }

        // No frontmost normal window at all (empty desktop) is not fullscreen.
        let Some(window) = frontmost_window_bounds()? else {
            return Ok(false);
        };

        Ok(window_is_fullscreen(window, &displays))
    }

    fn screen_rect(rect: CGRect) -> ScreenRect {
        ScreenRect {
            x: rect.origin.x,
            y: rect.origin.y,
            width: rect.size.width,
            height: rect.size.height,
        }
    }

    fn active_display_bounds() -> Result<Vec<ScreenRect>, String> {
        let mut count: u32 = 0;
        // SAFETY: a null list pointer asks Quartz for the count only.
        let status = unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) };
        if status != CGError::Success {
            return Err(format!("could not count Quartz displays: {status:?}"));
        }

        let mut ids: Vec<CGDirectDisplayID> = vec![0; count as usize];
        let mut filled: u32 = 0;
        // SAFETY: `ids` has room for exactly `count` identifiers.
        let status = unsafe { CGGetActiveDisplayList(count, ids.as_mut_ptr(), &mut filled) };
        if status != CGError::Success {
            return Err(format!("could not list Quartz displays: {status:?}"));
        }
        ids.truncate(filled as usize);

        Ok(ids
            .into_iter()
            .map(|id| screen_rect(CGDisplayBounds(id)))
            .collect())
    }

    /// Bounds of the frontmost normal window, or `None` when none is on screen.
    ///
    /// `CGWindowListCopyWindowInfo` returns on-screen windows in front-to-back
    /// order, so the first layer-0 entry is the active window.
    fn frontmost_window_bounds() -> Result<Option<ScreenRect>, String> {
        let options =
            CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
        let windows = CGWindowListCopyWindowInfo(options, NULL_WINDOW)
            .ok_or("Quartz did not return a window list")?;

        for index in 0..windows.count() {
            // SAFETY: `index` is within the array we just received.
            let entry = unsafe { windows.value_at_index(index) };
            if entry.is_null() {
                continue;
            }
            // SAFETY: the window list holds CFDictionary entries.
            let entry = unsafe { &*entry.cast::<CFDictionary>() };

            if dictionary_i64(entry, window_key_layer()) != Some(NORMAL_WINDOW_LAYER) {
                continue;
            }

            return Ok(dictionary_rect(entry, window_key_bounds()));
        }

        Ok(None)
    }

    fn window_key_layer() -> *const c_void {
        // SAFETY: reading an immutable framework constant.
        std::ptr::from_ref(unsafe { kCGWindowLayer }).cast()
    }

    fn window_key_bounds() -> *const c_void {
        // SAFETY: reading an immutable framework constant.
        std::ptr::from_ref(unsafe { kCGWindowBounds }).cast()
    }

    fn dictionary_i64(entry: &CFDictionary, key: *const c_void) -> Option<i64> {
        // SAFETY: `key` is a valid CFString constant.
        let value = unsafe { entry.value(key) };
        if value.is_null() {
            return None;
        }

        // SAFETY: the layer entry is a CFNumber.
        let number = unsafe { &*value.cast::<CFNumber>() };
        let mut result: i64 = 0;
        // SAFETY: the out pointer matches the requested SInt64 type.
        let read = unsafe {
            number.value(
                CFNumberType::SInt64Type,
                std::ptr::from_mut(&mut result).cast(),
            )
        };

        read.then_some(result)
    }

    fn dictionary_rect(entry: &CFDictionary, key: *const c_void) -> Option<ScreenRect> {
        // SAFETY: `key` is a valid CFString constant.
        let value = unsafe { entry.value(key) };
        if value.is_null() {
            return None;
        }

        // SAFETY: the bounds entry is a CFDictionary rect representation.
        let bounds = unsafe { &*value.cast::<CFDictionary>() };
        let mut rect = CGRect {
            origin: CGPoint { x: 0.0, y: 0.0 },
            size: CGSize {
                width: 0.0,
                height: 0.0,
            },
        };
        // SAFETY: `rect` is a valid out pointer for the duration of the call.
        let read = unsafe {
            CGRectMakeWithDictionaryRepresentation(Some(bounds), std::ptr::from_mut(&mut rect))
        };

        read.then(|| screen_rect(rect))
    }
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
mod platform_probe {
    pub fn idle_seconds() -> Result<u64, String> {
        Err(format!(
            "no idle probe is implemented for {}",
            std::env::consts::OS
        ))
    }

    pub fn active_window_fullscreen() -> Result<bool, String> {
        Err(format!(
            "no fullscreen probe is implemented for {}",
            std::env::consts::OS
        ))
    }
}

fn environment_value(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

/// The windowing session, reported per platform rather than left blank.
///
/// The XDG variables these read on Linux do not exist on macOS, where the
/// session is always Quartz, so returning `None` there would read as a failed
/// probe rather than a different platform.
fn session_type() -> Option<String> {
    if cfg!(target_os = "macos") {
        return Some("quartz".to_owned());
    }

    environment_value("XDG_SESSION_TYPE")
}

fn desktop() -> Option<String> {
    if cfg!(target_os = "macos") {
        return Some("Aqua".to_owned());
    }

    environment_value("XDG_CURRENT_DESKTOP")
}

fn display() -> Option<String> {
    if cfg!(target_os = "macos") {
        return Some("Quartz Compositor".to_owned());
    }

    environment_value("DISPLAY")
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

    let (idle_seconds, idle_error) = match platform_probe::idle_seconds() {
        Ok(seconds) => (Some(seconds), None),
        Err(error) => (None, Some(error)),
    };
    let (active_window_fullscreen, fullscreen_error) =
        match platform_probe::active_window_fullscreen() {
            Ok(fullscreen) => (Some(fullscreen), None),
            Err(error) => (None, Some(error)),
        };

    DiagnosticsReport {
        operating_system: std::env::consts::OS,
        session_type: session_type(),
        desktop: desktop(),
        display: display(),
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

    const LAPTOP: ScreenRect = ScreenRect {
        x: 0.0,
        y: 0.0,
        width: 1710.0,
        height: 1112.0,
    };
    const EXTERNAL: ScreenRect = ScreenRect {
        x: 1710.0,
        y: 0.0,
        width: 2560.0,
        height: 1440.0,
    };

    #[test]
    fn a_window_filling_its_display_is_fullscreen() {
        assert!(window_is_fullscreen(LAPTOP, &[LAPTOP, EXTERNAL]));
    }

    #[test]
    fn a_zoomed_window_below_the_menu_bar_is_not_fullscreen() {
        // macOS keeps the menu bar visible for a merely zoomed window, so the
        // window starts lower and is shorter than the display.
        let zoomed = ScreenRect {
            x: 0.0,
            y: 25.0,
            width: 1710.0,
            height: 1087.0,
        };

        assert!(!window_is_fullscreen(zoomed, &[LAPTOP]));
    }

    #[test]
    fn fullscreen_is_detected_on_a_secondary_display() {
        assert!(window_is_fullscreen(EXTERNAL, &[LAPTOP, EXTERNAL]));
    }

    #[test]
    fn sub_pixel_differences_still_count_as_fullscreen() {
        let rounded = ScreenRect {
            x: 0.0,
            y: 0.0,
            width: 1709.6,
            height: 1112.4,
        };

        assert!(window_is_fullscreen(rounded, &[LAPTOP]));
    }

    #[test]
    fn a_window_matching_no_display_is_not_fullscreen() {
        let floating = ScreenRect {
            x: 400.0,
            y: 300.0,
            width: 900.0,
            height: 600.0,
        };

        assert!(!window_is_fullscreen(floating, &[LAPTOP, EXTERNAL]));
    }

    #[test]
    fn a_window_with_no_displays_to_match_is_not_fullscreen() {
        assert!(!window_is_fullscreen(LAPTOP, &[]));
    }

    #[test]
    fn overlay_labels_share_an_absolute_deadline() {
        assert_eq!(
            overlay_label(7, 1, 2, 20, 1_800_000_000_000),
            "overlay-7-1-2-20-1800000000000"
        );
    }
}
