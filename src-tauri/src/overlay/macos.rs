use super::windows::APPKIT_OPERATION_TIMEOUT;
use std::sync::{
    atomic::{AtomicBool, Ordering},
    mpsc, Arc,
};
use tauri::{AppHandle, Manager, WebviewWindow};
use tauri_nspanel::{
    objc2_app_kit::{NSView as NativeView, NSWindowCollectionBehavior, NSWindowStyleMask},
    ManagerExt, Panel, PanelLevel, WebviewWindowExt,
};

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

    await_main_thread_result(receiver, &cancelled, operation_name)
}

fn await_main_thread_result<T>(
    receiver: mpsc::Receiver<Result<T, String>>,
    cancelled: &AtomicBool,
    operation_name: &str,
) -> Result<T, String> {
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

fn resolve_order_and_focus<Identifiers, Identifier, Resolved, Resolve, OrderPanels, Focus>(
    identifiers: Identifiers,
    cancelled: &AtomicBool,
    mut resolve: Resolve,
    mut order_panel: OrderPanels,
    focus: Focus,
) -> Result<(), String>
where
    Identifiers: IntoIterator<Item = Identifier>,
    Resolve: FnMut(Identifier) -> Result<Resolved, String>,
    OrderPanels: FnMut(&Resolved),
    Focus: FnOnce(&Resolved) -> Result<(), String>,
{
    let ensure_not_cancelled = || {
        if cancelled.load(Ordering::Acquire) {
            Err("macOS overlay reveal and focus was cancelled".to_owned())
        } else {
            Ok(())
        }
    };
    ensure_not_cancelled()?;
    let resolved = identifiers
        .into_iter()
        .map(&mut resolve)
        .collect::<Result<Vec<_>, _>>()?;
    let owner = resolved
        .first()
        .ok_or_else(|| "cannot focus an empty overlay panel set".to_owned())?;
    for panel in &resolved {
        ensure_not_cancelled()?;
        order_panel(panel);
    }
    ensure_not_cancelled()?;
    focus(owner)
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
    // The planned first overlay is index 0 and owns the accessibility announcement.
    // Show every display before giving only this webview initial keyboard ownership.
    let owner = windows
        .first()
        .ok_or_else(|| "cannot focus an empty overlay panel set".to_owned())?;
    let panel_app = owner.app_handle().clone();
    let owner_label = owner.label().to_owned();
    let labels: Vec<_> = windows
        .iter()
        .map(|window| window.label().to_owned())
        .collect();

    let operation_name = "macOS overlay panel reveal and keyboard focus";
    let (sender, receiver) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let task_cancelled = Arc::clone(&cancelled);

    // Tauri supplies the actual WKWebView on the main thread. Dispatch the whole
    // transaction here rather than waiting on another main-thread operation.
    owner
        .with_webview(move |platform_webview| {
            crate::tray::panel::hide_on_main(&panel_app);
            let result = resolve_order_and_focus(
                labels,
                &task_cancelled,
                |label| {
                    panel_app
                        .get_webview_panel(&label)
                        .map_err(|_| format!("could not find converted overlay panel {label}"))
                },
                |panel| panel.order_front_regardless(),
                |panel| {
                    // SAFETY: `inner` is Tauri's live WKWebView, an NSView subclass.
                    // This borrow and every AppKit operation stay inside its main-thread
                    // callback; no native pointer or reference escapes the callback.
                    let webview = unsafe { platform_webview.inner().cast::<NativeView>().as_ref() }
                        .ok_or_else(|| format!("overlay {owner_label} has no native WKWebView"))?;
                    focus_overlay_webview(panel.as_ref(), webview)
                },
            );
            let _ = sender.send(result);
        })
        .map_err(|error| {
            format!("could not schedule {operation_name} on the main thread: {error}")
        })?;

    await_main_thread_result(receiver, &cancelled, operation_name)
}

fn focus_overlay_webview(panel: &dyn Panel, webview: &NativeView) -> Result<(), String> {
    let accepts_responder = webview.acceptsFirstResponder();
    let assigned_responder = accepts_responder && panel.make_first_responder(Some(webview));
    if assigned_responder {
        panel.make_key_window();
    }

    let is_key = panel.as_panel().isKeyWindow();
    let responder_in_webview = panel.as_panel().firstResponder().is_some_and(|responder| {
        responder
            .downcast_ref::<NativeView>()
            .is_some_and(|view| std::ptr::eq(view, webview) || view.isDescendantOf(webview))
    });
    if accepts_responder && assigned_responder && is_key && responder_in_webview {
        #[cfg(debug_assertions)]
        eprintln!(
            "overlay keyboard focus acquired: owner={}, key={is_key}, \
             responder_in_webview={responder_in_webview}",
            panel.label()
        );
        return Ok(());
    }

    let label = panel.label();
    #[cfg(debug_assertions)]
    eprintln!(
        "overlay keyboard focus failed: owner={label}, accepts_responder={accepts_responder}, \
         assigned_responder={assigned_responder}, key={is_key}, \
         responder_in_webview={responder_in_webview}"
    );
    Err(format!(
        "could not give overlay {label} keyboard focus \
         (accepts responder: {accepts_responder}, assigned responder: {assigned_responder}, \
         key: {is_key}, responder in WKWebView: {responder_in_webview})"
    ))
}

pub(super) fn close_overlay_panel(window: &WebviewWindow) -> Result<(), String> {
    let app = window.app_handle().clone();
    let panel_app = app.clone();
    let panel_window = window.clone();
    let label = window.label().to_owned();

    run_on_main_thread(&app, "macOS overlay panel teardown", move || {
        let window_to_close = match panel_app.get_webview_panel(&label) {
            Ok(panel) => {
                // Let AppKit release temporary key ownership while this is still
                // the nonactivating NSPanel, before restoring its original class.
                panel.hide();
                panel.to_window().ok_or_else(|| {
                    format!("could not restore overlay {label} to its original window class")
                })?
            }
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
            panel.hide();
            // `to_window` first removes the retained handle from the plugin
            // store and then restores the class and original delegate.
            let _ = panel.to_window();
        }
        Ok(())
    })
}

#[cfg(test)]
mod tests {
    use super::{
        await_main_thread_result, overlay_panel_policy, overlay_style_mask, resolve_order_and_focus,
    };
    use std::{
        cell::RefCell,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
    };
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
    fn a_later_panel_lookup_failure_neither_orders_nor_focuses() {
        let events = RefCell::new(Vec::new());

        let result = resolve_order_and_focus(
            ["first", "missing"],
            &AtomicBool::new(false),
            |label| {
                if label == "missing" {
                    Err("missing panel".to_owned())
                } else {
                    Ok(label)
                }
            },
            |panel| events.borrow_mut().push(format!("order-{panel}")),
            |panel| {
                events.borrow_mut().push(format!("focus-{panel}"));
                Ok(())
            },
        );

        assert_eq!(result.unwrap_err(), "missing panel");
        assert!(events.into_inner().is_empty());
    }

    #[test]
    fn every_panel_is_resolved_before_the_full_set_is_ordered() {
        let events = RefCell::new(Vec::new());

        resolve_order_and_focus(
            ["first", "second"],
            &AtomicBool::new(false),
            |label| {
                events.borrow_mut().push(format!("resolve-{label}"));
                Ok(label)
            },
            |panel| events.borrow_mut().push(format!("order-{panel}")),
            |_| Ok(()),
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

    #[test]
    fn all_panels_are_visible_before_the_first_panel_takes_keyboard_focus() {
        let events = RefCell::new(Vec::new());

        resolve_order_and_focus(
            ["first", "second"],
            &AtomicBool::new(false),
            |label| {
                events.borrow_mut().push(format!("resolve-{label}"));
                Ok(label)
            },
            |panel| events.borrow_mut().push(format!("order-{panel}")),
            |panel| {
                events.borrow_mut().push(format!("focus-{panel}"));
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(
            events.into_inner(),
            [
                "resolve-first",
                "resolve-second",
                "order-first",
                "order-second",
                "focus-first"
            ]
        );
    }

    #[test]
    fn an_empty_panel_set_cannot_take_focus() {
        let result = resolve_order_and_focus(
            std::iter::empty::<&str>(),
            &AtomicBool::new(false),
            Ok,
            |_| panic!("an empty run must not reveal a panel"),
            |_| panic!("an empty run must not take focus"),
        );

        assert!(result.is_err());
    }

    #[test]
    fn a_focus_failure_is_returned_for_startup_rollback() {
        let ordered = RefCell::new(Vec::new());
        let result = resolve_order_and_focus(
            ["first", "second"],
            &AtomicBool::new(false),
            Ok,
            |panel| ordered.borrow_mut().push(*panel),
            |_| Err("WKWebView rejected first responder".to_owned()),
        );

        assert_eq!(result.unwrap_err(), "WKWebView rejected first responder");
        assert_eq!(ordered.into_inner(), ["first", "second"]);
    }

    #[test]
    fn a_cancelled_queued_callback_neither_resolves_orders_nor_focuses() {
        let cancelled = AtomicBool::new(false);
        let events = RefCell::new(Vec::new());
        let queued_callback = || {
            resolve_order_and_focus(
                ["first", "second"],
                &cancelled,
                |label| {
                    events.borrow_mut().push(format!("resolve-{label}"));
                    Ok(label)
                },
                |panel| events.borrow_mut().push(format!("order-{panel}")),
                |panel| {
                    events.borrow_mut().push(format!("focus-{panel}"));
                    Ok(())
                },
            )
        };

        cancelled.store(true, Ordering::Release);
        assert!(queued_callback().is_err());
        assert!(events.into_inner().is_empty());
    }

    #[test]
    fn cancellation_during_resolution_prevents_reveal_and_focus() {
        let cancelled = AtomicBool::new(false);
        let events = RefCell::new(Vec::new());
        let result = resolve_order_and_focus(
            ["first", "second"],
            &cancelled,
            |label| {
                events.borrow_mut().push(format!("resolve-{label}"));
                if label == "second" {
                    cancelled.store(true, Ordering::Release);
                }
                Ok(label)
            },
            |panel| events.borrow_mut().push(format!("order-{panel}")),
            |panel| {
                events.borrow_mut().push(format!("focus-{panel}"));
                Ok(())
            },
        );

        assert!(result.is_err());
        assert_eq!(events.into_inner(), ["resolve-first", "resolve-second"]);
    }

    #[test]
    fn cancellation_after_reveal_prevents_keyboard_focus() {
        let cancelled = AtomicBool::new(false);
        let events = RefCell::new(Vec::new());
        let result = resolve_order_and_focus(
            ["first", "second"],
            &cancelled,
            Ok,
            |panel| {
                events.borrow_mut().push(format!("order-{panel}"));
                if *panel == "second" {
                    cancelled.store(true, Ordering::Release);
                }
            },
            |panel| {
                events.borrow_mut().push(format!("focus-{panel}"));
                Ok(())
            },
        );

        assert!(result.is_err());
        assert_eq!(events.into_inner(), ["order-first", "order-second"]);
    }

    #[test]
    fn a_disconnected_main_thread_operation_is_cancelled() {
        let (sender, receiver) = mpsc::sync_channel::<Result<(), String>>(1);
        let cancelled = AtomicBool::new(false);
        drop(sender);

        assert!(await_main_thread_result(receiver, &cancelled, "test operation").is_err());
        assert!(cancelled.load(Ordering::Acquire));
    }
}
