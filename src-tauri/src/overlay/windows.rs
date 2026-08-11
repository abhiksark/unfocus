use super::{
    labels::{
        bounded_overlay_duration, next_overlay_run_id, overlay_deadline_ms, overlay_label,
        overlay_run_id_from_label, MAX_OVERLAY_MONITORS,
    },
    lifecycle::{OverlayRunLifecycle, OVERLAY_COMPLETION_GRACE},
    OverlayCloseOrigins, OverlayController,
};
use serde::Serialize;
use std::{
    sync::{mpsc, Mutex},
    time::{Duration, Instant},
};
use tauri::{
    webview::PageLoadEvent, AppHandle, Emitter, EventTarget, Manager, WebviewUrl,
    WebviewWindowBuilder,
};

static OVERLAY_START_LOCK: Mutex<()> = Mutex::new(());
const MONITOR_ENUMERATION_TIMEOUT: Duration = Duration::from_secs(5);
const OVERLAY_WINDOW_READY_TIMEOUT: Duration = Duration::from_secs(5);

fn available_monitors(app: &AppHandle) -> Result<Vec<tauri::Monitor>, String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let main_app = app.clone();
    app.run_on_main_thread(move || {
        let _ = sender.send(main_app.available_monitors());
    })
    .map_err(|error| format!("could not schedule monitor enumeration: {error}"))?;

    receiver
        .recv_timeout(MONITOR_ENUMERATION_TIMEOUT)
        .map_err(|error| match error {
            mpsc::RecvTimeoutError::Timeout => format!(
                "monitor enumeration did not finish within {} seconds",
                MONITOR_ENUMERATION_TIMEOUT.as_secs()
            ),
            mpsc::RecvTimeoutError::Disconnected => {
                "monitor enumeration stopped before returning a result".to_owned()
            }
        })?
        .map_err(|error| format!("could not enumerate monitors: {error}"))
}

pub(super) fn close_overlay_windows(
    app: &AppHandle,
    close_origins: &OverlayCloseOrigins,
    prefix: Option<&str>,
    reason: &str,
) {
    let windows: Vec<_> = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| {
            label.starts_with("overlay-") && prefix.is_none_or(|prefix| label.starts_with(prefix))
        })
        .collect();

    // Mark every run before closing the first window: close events may be
    // delivered synchronously and must not look like unexpected WM teardown.
    for (label, _) in &windows {
        if let Some(run_id) = overlay_run_id_from_label(label) {
            close_origins.mark_intentional(run_id);
        }
    }

    for (label, window) in windows {
        eprintln!("closing overlay {label}: {reason}");
        if let Err(error) = window.close() {
            eprintln!("could not close overlay {label}: {error}");
        }
    }
}

pub(super) fn emit_overlay_event<T: Clone + Serialize>(
    app: &AppHandle,
    prefix: &str,
    event: &str,
    payload: T,
) {
    // `Emitter::emit` delivers to every target regardless of the receiver, so
    // emitting from each window would broadcast the event once per window in
    // the run. Address the windows explicitly instead.
    for label in app.webview_windows().into_keys() {
        if label.starts_with(prefix) {
            if let Err(error) = app.emit_to(
                EventTarget::webview_window(label.clone()),
                event,
                payload.clone(),
            ) {
                eprintln!("could not emit {event} to overlay {label}: {error}");
            }
        }
    }
}

pub(super) fn overlay_run_exists(app: &AppHandle, prefix: &str) -> bool {
    app.webview_windows()
        .keys()
        .any(|label| label.starts_with(prefix))
}

pub(crate) fn show_overlay(
    app: &AppHandle,
    controller: &OverlayController,
    duration_seconds: u64,
) -> Result<usize, String> {
    start_overlay(app, controller, duration_seconds, true)
}

pub(crate) fn show_overlay_if_idle(
    app: &AppHandle,
    controller: &OverlayController,
    duration_seconds: u64,
) -> Result<usize, String> {
    start_overlay(app, controller, duration_seconds, false)
}

fn start_overlay(
    app: &AppHandle,
    controller: &OverlayController,
    duration_seconds: u64,
    replace_existing: bool,
) -> Result<usize, String> {
    let _start_guard = OVERLAY_START_LOCK
        .lock()
        .map_err(|_| "overlay start lock is poisoned".to_owned())?;

    if replace_existing {
        controller.cancel_all()?;
        controller.close_windows(app, None, "superseded by a new overlay run");
    } else if controller.has_active_run() || overlay_run_exists(app, "overlay-") {
        return Err("another overlay run is already active".into());
    }

    // TAO's GTK monitor conversion reads X11 workarea properties directly.
    // Always perform that operation on the application thread, even when an
    // overlay was requested by a reminder or another background worker.
    let monitors = available_monitors(app)?;
    if monitors.is_empty() {
        return Err("Tauri did not report any monitors".into());
    }
    if monitors.len() > MAX_OVERLAY_MONITORS {
        return Err(format!(
            "Tauri reported {} monitors; overlays support at most {MAX_OVERLAY_MONITORS}",
            monitors.len()
        ));
    }

    let duration_seconds = bounded_overlay_duration(duration_seconds);
    let run_id = next_overlay_run_id();
    let prefix = format!("overlay-{run_id}-");
    let total = monitors.len();
    let starts_at = Instant::now();
    let deadline_ms = overlay_deadline_ms(duration_seconds)?;
    let completes_at = starts_at + Duration::from_secs(duration_seconds);
    let closes_at = completes_at + OVERLAY_COMPLETION_GRACE;

    for (index, monitor) in monitors.iter().enumerate() {
        let scale_factor = monitor.scale_factor();
        let position = monitor.position();
        let size = monitor.size();
        let label = overlay_label(run_id, index, total, duration_seconds, deadline_ms);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);

        let build_result =
            WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
                .title("Unfocus eye break")
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
                .on_page_load(move |_, payload| {
                    if matches!(payload.event(), PageLoadEvent::Finished) {
                        let _ = ready_sender.try_send(());
                    }
                })
                .build();

        // Multi-monitor invariant: any single failure rolls back every window
        // already opened in this run so the desk is never half-covered.
        if let Err(error) = build_result {
            controller.close_windows(app, Some(&prefix), "overlay startup failed");
            return Err(format!(
                "could not create overlay {index} of {total}: {error}"
            ));
        }

        // Native construction can return before the local page has initialized.
        // Wait from the background caller so the application thread remains
        // free to realize this window before another one is queued.
        match ready_receiver.recv_timeout(OVERLAY_WINDOW_READY_TIMEOUT) {
            Ok(()) => {}
            Err(mpsc::RecvTimeoutError::Timeout) => {
                controller.close_windows(app, Some(&prefix), "overlay startup failed");
                return Err(format!(
                    "overlay {index} of {total} did not finish loading within {} seconds",
                    OVERLAY_WINDOW_READY_TIMEOUT.as_secs()
                ));
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => {
                controller.close_windows(app, Some(&prefix), "overlay startup failed");
                return Err(format!(
                    "overlay {index} of {total} closed before it finished loading"
                ));
            }
        }
    }

    eprintln!("opened overlay run {run_id} on {total} display(s) for {duration_seconds} second(s)");

    if let Err(error) = controller.register(OverlayRunLifecycle {
        run_id,
        prefix: prefix.clone(),
        completes_at,
        closes_at,
        dismiss_at: None,
        completed: false,
        closing_emitted: false,
    }) {
        controller.close_windows(app, Some(&prefix), "overlay lifecycle registration failed");
        return Err(error);
    }

    Ok(total)
}

#[cfg(debug_assertions)]
fn environment_value(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

#[cfg(debug_assertions)]
pub(crate) fn schedule_automatic_overlay_test(app: &tauri::App, controller: OverlayController) {
    if environment_value("UNFOCUS_SPIKE_AUTO_OVERLAY").as_deref() != Some("1") {
        return;
    }

    let duration_seconds = environment_value("UNFOCUS_SPIKE_OVERLAY_SECONDS")
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);
    let app = app.handle().clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        match show_overlay(&app, &controller, duration_seconds) {
            Ok(count) => eprintln!("automatic overlay test opened {count} window(s)"),
            Err(error) => eprintln!("automatic overlay test failed: {error}"),
        }
    });
}
