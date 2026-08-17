use std::{
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    time::Duration,
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_nspanel::{
    objc2_app_kit::{NSWindowCollectionBehavior, NSWindowStyleMask},
    ManagerExt, Panel, PanelLevel, WebviewWindowExt,
};

const APPKIT_OPERATION_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Copy)]
struct OverlayPanelPolicy {
    collection_behavior: NSWindowCollectionBehavior,
    level: i64,
    hides_on_deactivate: bool,
    can_become_key_window: bool,
    can_become_main_window: bool,
}

fn overlay_panel_policy() -> OverlayPanelPolicy {
    OverlayPanelPolicy {
        collection_behavior: NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::CanJoinAllApplications
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary,
        level: PanelLevel::ScreenSaver.value(),
        hides_on_deactivate: false,
        can_become_key_window: true,
        can_become_main_window: false,
    }
}

fn overlay_style_mask(existing: NSWindowStyleMask) -> NSWindowStyleMask {
    existing | NSWindowStyleMask::NonactivatingPanel
}

tauri_nspanel::tauri_panel! {
    panel!(UnfocusOverlayPanel {
        config: {
            can_become_key_window: overlay_panel_policy().can_become_key_window,
            can_become_main_window: overlay_panel_policy().can_become_main_window
        }
    })
}

fn run_on_main_thread<T, F>(
    app: &AppHandle,
    operation_name: &str,
    operation: F,
) -> Result<T, String>
where
    T: Send + 'static,
    F: FnOnce() -> Result<T, String> + Send + 'static,
{
    let (sender, receiver) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let task_cancelled = Arc::clone(&cancelled);

    app.run_on_main_thread(move || {
        if !task_cancelled.load(Ordering::Acquire) {
            let _ = sender.send(operation());
        }
    })
    .map_err(|error| format!("could not schedule {operation_name} on the main thread: {error}"))?;

    receiver
        .recv_timeout(APPKIT_OPERATION_TIMEOUT)
        .map_err(|error| {
            cancelled.store(true, Ordering::Release);
            match error {
                mpsc::RecvTimeoutError::Timeout => format!(
                    "{operation_name} did not finish within {} seconds",
                    APPKIT_OPERATION_TIMEOUT.as_secs()
                ),
                mpsc::RecvTimeoutError::Disconnected => {
                    format!("{operation_name} stopped before returning a result")
                }
            }
        })?
}

fn resolve_then_order<Identifiers, Identifier, Resolved, Resolve, OrderPanels>(
    identifiers: Identifiers,
    mut resolve: Resolve,
    mut order_panel: OrderPanels,
) -> Result<(), String>
where
    Identifiers: IntoIterator<Item = Identifier>,
    Resolve: FnMut(Identifier) -> Result<Resolved, String>,
    OrderPanels: FnMut(Resolved),
{
    let resolved: Result<Vec<_>, _> = identifiers.into_iter().map(&mut resolve).collect();
    for panel in resolved? {
        order_panel(panel);
    }
    Ok(())
}

pub(super) fn configure_overlay_panel(window: &WebviewWindow) -> Result<(), String> {
    let app = window.app_handle().clone();
    let panel_window = window.clone();
    let label = window.label().to_owned();

    run_on_main_thread(&app, "macOS overlay panel configuration", move || {
        let policy = overlay_panel_policy();
        let panel = panel_window
            .to_panel::<UnfocusOverlayPanel>()
            .map_err(|error| format!("could not convert overlay {label} to an NSPanel: {error}"))?;
        panel.set_released_when_closed(false);
        panel.set_style_mask(overlay_style_mask(panel.as_panel().styleMask()));
        panel.set_level(policy.level);
        panel.set_hides_on_deactivate(policy.hides_on_deactivate);
        panel.set_collection_behavior(policy.collection_behavior);
        Ok(())
    })
}

pub(super) fn order_overlay_panels(windows: &[WebviewWindow]) -> Result<(), String> {
    let app = windows
        .first()
        .ok_or_else(|| "cannot order an empty overlay panel set".to_owned())?
        .app_handle()
        .clone();
    let panel_app = app.clone();
    let labels: Vec<_> = windows
        .iter()
        .map(|window| window.label().to_owned())
        .collect();

    run_on_main_thread(&app, "macOS overlay panel ordering", move || {
        resolve_then_order(
            labels,
            |label| {
                panel_app
                    .get_webview_panel(&label)
                    .map_err(|_| format!("could not find converted overlay panel {label}"))
            },
            |panel| panel.order_front_regardless(),
        )
    })
}

pub(super) fn close_overlay_panel(window: &WebviewWindow) -> Result<(), String> {
    let app = window.app_handle().clone();
    let panel_app = app.clone();
    let panel_window = window.clone();
    let label = window.label().to_owned();

    run_on_main_thread(&app, "macOS overlay panel teardown", move || {
        let window_to_close = match panel_app.get_webview_panel(&label) {
            Ok(panel) => panel.to_window().ok_or_else(|| {
                format!("could not restore overlay {label} to its original window class")
            })?,
            Err(_) => panel_window,
        };

        window_to_close
            .close()
            .map_err(|error| format!("could not close overlay {label}: {error}"))
    })
}

pub(super) fn prepare_unexpected_overlay_teardown(
    app: &AppHandle,
    window_label: &str,
) -> Result<(), String> {
    let app = app.clone();
    let panel_app = app.clone();
    let label = window_label.to_owned();

    run_on_main_thread(&app, "unexpected macOS overlay panel cleanup", move || {
        if let Ok(panel) = panel_app.get_webview_panel(&label) {
            // `to_window` first removes the retained handle from the plugin
            // store and then restores the class and original delegate.
            let _ = panel.to_window();
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::{overlay_panel_policy, overlay_style_mask, resolve_then_order};
    use std::cell::RefCell;
    use tauri_nspanel::{
        objc2_app_kit::{NSWindowCollectionBehavior, NSWindowStyleMask},
        PanelLevel,
    };

    #[test]
    fn overlay_collection_behavior_covers_spaces_apps_and_fullscreen() {
        let expected = NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::CanJoinAllApplications
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::Stationary;

        assert_eq!(overlay_panel_policy().collection_behavior, expected);
    }

    #[test]
    fn overlay_style_preserves_builder_bits_and_adds_nonactivating_panel() {
        let existing =
            NSWindowStyleMask::Titled | NSWindowStyleMask::Closable | NSWindowStyleMask::Resizable;
        let expected = existing | NSWindowStyleMask::NonactivatingPanel;

        assert_eq!(overlay_style_mask(existing), expected);
    }

    #[test]
    fn overlay_uses_screen_saver_level_and_stays_visible_when_deactivated() {
        let policy = overlay_panel_policy();

        assert_eq!(policy.level, PanelLevel::ScreenSaver.value());
        assert!(!policy.hides_on_deactivate);
    }

    #[test]
    fn overlay_can_become_key_but_not_main_window() {
        let policy = overlay_panel_policy();

        assert!(policy.can_become_key_window);
        assert!(!policy.can_become_main_window);
    }

    #[test]
    fn a_later_panel_lookup_failure_orders_nothing() {
        let ordered = RefCell::new(Vec::new());

        let result = resolve_then_order(
            ["first", "missing"],
            |label| {
                if label == "missing" {
                    Err("missing panel".to_owned())
                } else {
                    Ok(label)
                }
            },
            |panel| ordered.borrow_mut().push(panel),
        );

        assert_eq!(result.unwrap_err(), "missing panel");
        assert!(ordered.into_inner().is_empty());
    }

    #[test]
    fn every_panel_is_resolved_before_the_full_set_is_ordered() {
        let events = RefCell::new(Vec::new());

        resolve_then_order(
            ["first", "second"],
            |label| {
                events.borrow_mut().push(format!("resolve-{label}"));
                Ok(label)
            },
            |panel| events.borrow_mut().push(format!("order-{panel}")),
        )
        .unwrap();

        assert_eq!(
            events.into_inner(),
            [
                "resolve-first",
                "resolve-second",
                "order-first",
                "order-second"
            ]
        );
    }
}
