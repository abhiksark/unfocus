#[cfg(target_os = "macos")]
use super::labels::MAX_OVERLAY_MONITORS;
#[cfg(target_os = "macos")]
use super::macos;
use super::{
    labels::{
        next_overlay_run_id, overlay_deadline_ms, overlay_run_id_from_label, plan_overlay_run,
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
    utils::config::WindowConfig, webview::PageLoadEvent, AppHandle, Emitter, EventTarget, Manager,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};

static OVERLAY_START_LOCK: Mutex<()> = Mutex::new(());
const MONITOR_ENUMERATION_TIMEOUT: Duration = Duration::from_secs(5);
const OVERLAY_WINDOW_READY_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(target_os = "macos")]
const OVERLAY_CLEANUP_ATTEMPTS: usize = 2;
#[cfg(target_os = "macos")]
const OVERLAY_ABSENCE_POLL_ATTEMPTS: usize = 25;
#[cfg(target_os = "macos")]
const APPKIT_OPERATION_TIMEOUT_MILLIS: u64 = 5_000;
#[cfg(target_os = "macos")]
const OVERLAY_ABSENCE_POLL_INTERVAL_MILLIS: u64 = 10;
#[cfg(target_os = "macos")]
pub(super) const MACOS_CONFIRMED_CLOSE_BUDGET_MILLIS: u64 = OVERLAY_CLEANUP_ATTEMPTS as u64
    * (MAX_OVERLAY_MONITORS as u64 * APPKIT_OPERATION_TIMEOUT_MILLIS
        + OVERLAY_ABSENCE_POLL_ATTEMPTS as u64 * OVERLAY_ABSENCE_POLL_INTERVAL_MILLIS);
#[cfg(target_os = "macos")]
pub(super) const MACOS_INTENTIONAL_CLOSE_SUPPRESSION: Duration = Duration::from_millis(
    MACOS_CONFIRMED_CLOSE_BUDGET_MILLIS + OVERLAY_ABSENCE_POLL_INTERVAL_MILLIS,
);
#[cfg(target_os = "macos")]
pub(super) const APPKIT_OPERATION_TIMEOUT: Duration =
    Duration::from_millis(APPKIT_OPERATION_TIMEOUT_MILLIS);
#[cfg(target_os = "macos")]
const OVERLAY_ABSENCE_POLL_INTERVAL: Duration =
    Duration::from_millis(OVERLAY_ABSENCE_POLL_INTERVAL_MILLIS);

fn overlay_visible_on_all_workspaces(target_is_macos: bool) -> bool {
    target_is_macos
}

fn overlay_window_config(label: &str) -> WindowConfig {
    WindowConfig {
        label: label.to_owned(),
        url: WebviewUrl::App("index.html".into()),
        visible: !cfg!(target_os = "macos"),
        visible_on_all_workspaces: overlay_visible_on_all_workspaces(cfg!(target_os = "macos")),
        ..Default::default()
    }
}

fn complete_overlay_startup<WaitReady, OrderAll, Rollback>(
    total: usize,
    mut wait_ready: WaitReady,
    order_all: OrderAll,
    rollback: Rollback,
) -> Result<(), String>
where
    WaitReady: FnMut(usize) -> Result<(), String>,
    OrderAll: FnOnce() -> Result<(), String>,
    Rollback: FnOnce() -> Result<(), String>,
{
    let startup_result = (|| {
        for index in 0..total {
            wait_ready(index)?;
        }
        order_all()
    })();

    finish_overlay_startup(startup_result, rollback)
}

fn finish_overlay_startup<Rollback>(
    startup_result: Result<(), String>,
    rollback: Rollback,
) -> Result<(), String>
where
    Rollback: FnOnce() -> Result<(), String>,
{
    match startup_result {
        Ok(()) => Ok(()),
        Err(startup_error) => match rollback() {
            Ok(()) => Err(startup_error),
            Err(rollback_error) => Err(format!(
                "{startup_error}; overlay rollback failed: {rollback_error}"
            )),
        },
    }
}

fn rollback_overlay_startup(
    app: &AppHandle,
    controller: &OverlayController,
    run_id: u64,
    prefix: &str,
    reason: &str,
) -> Result<(), String> {
    rollback_or_retain_cleanup(
        || controller.close_windows(app, Some(prefix), reason),
        || controller.retain_startup_cleanup(run_id, prefix.to_owned()),
    )
}

fn rollback_or_retain_cleanup<Rollback, Retain>(
    rollback: Rollback,
    retain: Retain,
) -> Result<(), String>
where
    Rollback: FnOnce() -> Result<(), String>,
    Retain: FnOnce() -> Result<(), String>,
{
    match rollback() {
        Ok(()) => Ok(()),
        Err(rollback_error) => match retain() {
            Ok(()) => Err(rollback_error),
            Err(scheduling_error) => Err(format!(
                "{rollback_error}; persistent cleanup scheduling failed: {scheduling_error}"
            )),
        },
    }
}

#[cfg(any(target_os = "macos", test))]
fn retry_overlay_cleanup<Close, Exists, Wait>(
    close_attempts: usize,
    absence_poll_attempts: usize,
    mut close: Close,
    mut overlay_run_exists: Exists,
    mut wait: Wait,
) -> Result<(), String>
where
    Close: FnMut() -> Result<(), String>,
    Exists: FnMut() -> bool,
    Wait: FnMut(),
{
    let mut errors = Vec::new();
    for _ in 0..close_attempts {
        if let Err(error) = close() {
            errors.push(error);
        }
        if !overlay_run_exists() {
            return Ok(());
        }
        for _ in 0..absence_poll_attempts {
            wait();
            if !overlay_run_exists() {
                return Ok(());
            }
        }
    }

    errors.push(format!(
        "overlay windows remain after {close_attempts} cleanup attempts"
    ));
    Err(errors.join("; "))
}

fn close_overlay_window(window: &WebviewWindow) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::close_overlay_panel(window)
    }
    #[cfg(not(target_os = "macos"))]
    {
        window
            .close()
            .map_err(|error| format!("could not close overlay {}: {error}", window.label()))
    }
}

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
) -> Result<(), String> {
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

    let mut errors = Vec::new();
    for (label, window) in windows {
        eprintln!("closing overlay {label}: {reason}");
        if let Err(error) = close_overlay_window(&window) {
            eprintln!("{error}");
            errors.push(error);
        }
    }

    if errors.is_empty() {
        Ok(())
    } else {
        Err(errors.join("; "))
    }
}

pub(super) fn close_overlay_windows_confirmed(
    app: &AppHandle,
    close_origins: &OverlayCloseOrigins,
    prefix: Option<&str>,
    reason: &str,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        retry_overlay_cleanup(
            OVERLAY_CLEANUP_ATTEMPTS,
            OVERLAY_ABSENCE_POLL_ATTEMPTS,
            || close_overlay_windows(app, close_origins, prefix, reason),
            || overlay_run_exists(app, prefix.unwrap_or("overlay-")),
            || std::thread::sleep(OVERLAY_ABSENCE_POLL_INTERVAL),
        )
    }
    #[cfg(not(target_os = "macos"))]
    {
        close_overlay_windows(app, close_origins, prefix, reason)
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
        controller.close_windows(app, None, "superseded by a new overlay run")?;
    } else if controller.has_active_run() || overlay_run_exists(app, "overlay-") {
        return Err("another overlay run is already active".into());
    }

    // TAO's GTK monitor conversion reads X11 workarea properties directly.
    // Always perform that operation on the application thread, even when an
    // overlay was requested by a reminder or another background worker.
    let monitors = available_monitors(app)?;
    let run_id = next_overlay_run_id();
    let deadline_ms = overlay_deadline_ms(duration_seconds)?;
    let plan = plan_overlay_run(run_id, monitors.len(), duration_seconds, deadline_ms)?;
    let prefix = format!("overlay-{}-", plan.run_id);
    let total = plan.labels.len();
    let starts_at = Instant::now();
    let completes_at = starts_at + Duration::from_secs(plan.duration_seconds);
    let closes_at = completes_at + OVERLAY_COMPLETION_GRACE;
    let mut windows = Vec::with_capacity(total);
    let mut ready_receivers = Vec::with_capacity(total);

    for (index, (monitor, label)) in monitors.iter().zip(plan.labels.iter()).enumerate() {
        let scale_factor = monitor.scale_factor();
        let position = monitor.position();
        let size = monitor.size();
        let (ready_sender, ready_receiver) = mpsc::sync_channel(1);

        let window_config = overlay_window_config(label);
        let window_builder = WebviewWindowBuilder::new(app, window_config.label, window_config.url)
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
            .visible(window_config.visible)
            .skip_taskbar(true);
        #[cfg(target_os = "macos")]
        let window_builder =
            window_builder.visible_on_all_workspaces(window_config.visible_on_all_workspaces);
        let build_result = window_builder
            .background_color(tauri::webview::Color(7, 19, 16, 255))
            .on_page_load(move |_, payload| {
                if matches!(payload.event(), PageLoadEvent::Finished) {
                    let _ = ready_sender.try_send(());
                }
            })
            .build();

        // Multi-monitor invariant: any single failure rolls back every window
        // already opened in this run so the desk is never half-covered.
        let window = match build_result {
            Ok(window) => window,
            Err(error) => {
                return finish_overlay_startup(
                    Err(format!(
                        "could not create overlay {index} of {total}: {error}"
                    )),
                    || {
                        rollback_overlay_startup(
                            app,
                            controller,
                            plan.run_id,
                            &prefix,
                            "overlay startup failed",
                        )
                    },
                )
                .map(|()| total);
            }
        };

        #[cfg(target_os = "macos")]
        if let Err(error) = macos::configure_overlay_panel(&window) {
            return finish_overlay_startup(
                Err(format!(
                    "could not configure overlay {index} of {total}: {error}"
                )),
                || {
                    rollback_overlay_startup(
                        app,
                        controller,
                        plan.run_id,
                        &prefix,
                        "overlay startup failed",
                    )
                },
            )
            .map(|()| total);
        }

        windows.push(window);
        ready_receivers.push(ready_receiver);
    }

    // Every native panel stays hidden until the complete monitor set has
    // loaded. A later failure therefore cannot leave an earlier panel visible.
    let mut ready_receivers = ready_receivers.into_iter();
    let startup_result = complete_overlay_startup(
        total,
        |index| {
            let ready_receiver = ready_receivers
                .next()
                .ok_or_else(|| format!("missing readiness channel for overlay {index}"))?;
            ready_receiver
                .recv_timeout(OVERLAY_WINDOW_READY_TIMEOUT)
                .map_err(|error| match error {
                    mpsc::RecvTimeoutError::Timeout => format!(
                        "overlay {index} of {total} did not finish loading within {} seconds",
                        OVERLAY_WINDOW_READY_TIMEOUT.as_secs()
                    ),
                    mpsc::RecvTimeoutError::Disconnected => {
                        format!("overlay {index} of {total} closed before it finished loading")
                    }
                })
        },
        || {
            #[cfg(target_os = "macos")]
            {
                macos::order_overlay_panels(&windows)
            }
            #[cfg(not(target_os = "macos"))]
            {
                Ok(())
            }
        },
        || {
            rollback_overlay_startup(
                app,
                controller,
                plan.run_id,
                &prefix,
                "overlay startup failed",
            )
        },
    );
    startup_result?;

    eprintln!(
        "opened overlay run {} on {total} display(s) for {} second(s)",
        plan.run_id, plan.duration_seconds
    );

    if let Err(error) = controller.register(OverlayRunLifecycle {
        run_id: plan.run_id,
        prefix: prefix.clone(),
        completes_at,
        closes_at,
        dismiss_at: None,
        next_close_attempt_at: None,
        close_failures: 0,
        completed: false,
        closing_emitted: false,
    }) {
        return finish_overlay_startup(Err(error), || {
            rollback_overlay_startup(
                app,
                controller,
                plan.run_id,
                &prefix,
                "overlay lifecycle registration failed",
            )
        })
        .map(|()| total);
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

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::{Cell, RefCell};

    #[test]
    fn all_windows_finish_loading_before_the_run_is_ordered() {
        let events = RefCell::new(Vec::new());

        complete_overlay_startup(
            2,
            |index| {
                events.borrow_mut().push(format!("ready-{index}"));
                Ok(())
            },
            || {
                events.borrow_mut().push("order-all".to_owned());
                Ok(())
            },
            || {
                events.borrow_mut().push("rollback".to_owned());
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(events.into_inner(), ["ready-0", "ready-1", "order-all"]);
    }

    #[test]
    fn a_later_load_failure_never_orders_any_window() {
        let ordered = Cell::new(false);
        let rollbacks = Cell::new(0);

        let result = complete_overlay_startup(
            3,
            |index| {
                if index == 1 {
                    Err("later overlay failed".to_owned())
                } else {
                    Ok(())
                }
            },
            || {
                ordered.set(true);
                Ok(())
            },
            || {
                rollbacks.set(rollbacks.get() + 1);
                Ok(())
            },
        );

        assert_eq!(result.unwrap_err(), "later overlay failed");
        assert!(!ordered.get());
        assert_eq!(rollbacks.get(), 1);
    }

    #[test]
    fn ordering_failure_rolls_back_the_startup_transaction_once() {
        let rollbacks = Cell::new(0);

        let result = complete_overlay_startup(
            2,
            |_| Ok(()),
            || Err("panel ordering failed".to_owned()),
            || {
                rollbacks.set(rollbacks.get() + 1);
                Ok(())
            },
        );

        assert_eq!(result.unwrap_err(), "panel ordering failed");
        assert_eq!(rollbacks.get(), 1);
    }

    #[test]
    fn startup_error_includes_a_failed_rollback() {
        let result = complete_overlay_startup(
            1,
            |_| Err("overlay did not load".to_owned()),
            || Ok(()),
            || Err("native panel remained".to_owned()),
        );

        assert_eq!(
            result.unwrap_err(),
            "overlay did not load; overlay rollback failed: native panel remained"
        );
    }

    #[test]
    fn exhausted_startup_rollback_is_transferred_to_persistent_cleanup() {
        let retained = Cell::new(0);

        let result = rollback_or_retain_cleanup(
            || Err("native panels remain".to_owned()),
            || {
                retained.set(retained.get() + 1);
                Ok(())
            },
        );

        assert_eq!(result.unwrap_err(), "native panels remain");
        assert_eq!(retained.get(), 1);
    }

    #[test]
    fn startup_rollback_reports_persistent_cleanup_scheduling_failure() {
        let result = rollback_or_retain_cleanup(
            || Err("native panels remain".to_owned()),
            || Err("overlay lifecycle worker has stopped".to_owned()),
        );

        assert_eq!(
            result.unwrap_err(),
            "native panels remain; persistent cleanup scheduling failed: overlay lifecycle worker has stopped"
        );
    }

    #[test]
    fn startup_rollback_retries_until_cleanup_is_confirmed() {
        let attempts = Cell::new(0);

        retry_overlay_cleanup(
            2,
            0,
            || {
                attempts.set(attempts.get() + 1);
                if attempts.get() == 1 {
                    Err("first native close timed out".to_owned())
                } else {
                    Ok(())
                }
            },
            || attempts.get() < 2,
            || {},
        )
        .unwrap();

        assert_eq!(attempts.get(), 2);
    }

    #[test]
    fn startup_rollback_reports_every_failed_attempt_and_remaining_windows() {
        let attempts = Cell::new(0);

        let result = retry_overlay_cleanup(
            2,
            2,
            || {
                attempts.set(attempts.get() + 1);
                Err(format!("close attempt {} failed", attempts.get()))
            },
            || true,
            || {},
        );

        assert_eq!(attempts.get(), 2);
        assert_eq!(
            result.unwrap_err(),
            "close attempt 1 failed; close attempt 2 failed; overlay windows remain after 2 cleanup attempts"
        );
    }

    #[test]
    fn async_window_removal_is_confirmed_before_another_close_attempt() {
        let close_attempts = Cell::new(0);
        let waits = Cell::new(0);
        let visible = Cell::new(true);

        retry_overlay_cleanup(
            2,
            3,
            || {
                close_attempts.set(close_attempts.get() + 1);
                Ok(())
            },
            || visible.get(),
            || {
                waits.set(waits.get() + 1);
                visible.set(false);
            },
        )
        .unwrap();

        assert_eq!(close_attempts.get(), 1);
        assert_eq!(waits.get(), 1);
    }

    #[test]
    fn only_macos_overlay_windows_join_all_workspaces() {
        assert!(overlay_visible_on_all_workspaces(true));
        assert!(!overlay_visible_on_all_workspaces(false));

        assert_eq!(
            overlay_window_config("overlay-test").visible_on_all_workspaces,
            cfg!(target_os = "macos")
        );
    }
}
