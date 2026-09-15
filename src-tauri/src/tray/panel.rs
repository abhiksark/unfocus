//! The compact tray surface owns presentation only; preparation belongs to the scheduler.
use serde::{Deserialize, Serialize};
use tauri::{Manager, WebviewWindow};

pub(crate) const LABEL: &str = "tray-panel";

fn authorize(label: &str, macos: bool) -> Result<(), String> {
    if macos && label == LABEL {
        Ok(())
    } else {
        Err("this command is only available to the macOS tray panel".into())
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Pending {
    request_id: u64,
    remaining_ms: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Snapshot {
    reminder: crate::reminder::ReminderStatus,
    pending: Option<Pending>,
    opening_generation: u64,
    countdown_request_id: Option<u64>,
    visible: bool,
}
#[derive(Deserialize)]
#[serde(rename_all = "lowercase")]
pub(crate) enum Action {
    Begin,
    Cancel,
    Pause,
    Resume,
    Open,
    Hide,
    Quit,
}

#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use crate::reminder::{ReminderAction, ReminderControl};
    use std::sync::Mutex;
    use tauri::{Emitter, EventTarget, WebviewUrl, WebviewWindowBuilder};
    use tauri_nspanel::{
        objc2_app_kit::{NSWindowCollectionBehavior, NSWindowStyleMask},
        ManagerExt, WebviewWindowExt,
    };
    pub(crate) const WIDTH: f64 = 250.0;
    pub(crate) const HEIGHT: f64 = 270.0;
    #[derive(Default)]
    pub(crate) struct Presentation {
        pub generation: u64,
        pub visible: bool,
        pub countdown: Option<u64>,
        pub ready: bool,
        pub tray_bounds: Option<[f64; 4]>,
    }
    #[derive(Default)]
    pub(crate) struct Runtime(pub Mutex<Presentation>);
    tauri_nspanel::tauri_panel! {
        panel!(UnfocusTrayPanel { config: { can_become_key_window: true, can_become_main_window: false } })
    }
    pub(crate) fn install(app: &tauri::AppHandle) -> Result<(), String> {
        app.manage(Runtime::default());
        let window = WebviewWindowBuilder::new(app, LABEL, WebviewUrl::App("index.html".into()))
            .title("Unfocus")
            .inner_size(WIDTH, HEIGHT)
            .visible(false)
            .focused(false)
            .decorations(false)
            .resizable(false)
            .skip_taskbar(true)
            .build()
            .map_err(|e| e.to_string())?;
        let panel = window
            .to_panel::<UnfocusTrayPanel>()
            .map_err(|e| e.to_string())?;
        panel.set_released_when_closed(false);
        panel.set_style_mask(panel.as_panel().styleMask() | NSWindowStyleMask::NonactivatingPanel);
        panel.set_level(25);
        panel.set_has_shadow(true);
        panel.as_panel().setOpaque(false);
        panel
            .as_panel()
            .setBackgroundColor(Some(&tauri_nspanel::objc2_app_kit::NSColor::clearColor()));
        if let Some(content) = panel.as_panel().contentView() {
            content.setWantsLayer(true);
            if let Some(layer) = content.layer() {
                // SAFETY: the live CALayer supports setCornerRadius: with CGFloat (f64 on macOS).
                unsafe {
                    let _: () = tauri_nspanel::objc2::msg_send![&*layer, setCornerRadius: 14.0_f64];
                }
                layer.setMasksToBounds(true);
            }
        }
        panel.set_hides_on_deactivate(false);
        panel.set_collection_behavior(
            NSWindowCollectionBehavior::CanJoinAllSpaces
                | NSWindowCollectionBehavior::FullScreenAuxiliary,
        );
        Ok(())
    }
    pub(crate) fn snapshot(app: &tauri::AppHandle) -> Snapshot {
        let runtime = app.state::<Runtime>();
        let state = runtime
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let prep = app.state::<ReminderControl>().preparation_status();
        let pending =
            prep.request_id
                .zip(prep.remaining_milliseconds)
                .map(|(request_id, remaining_ms)| Pending {
                    request_id,
                    remaining_ms,
                });
        Snapshot {
            reminder: crate::reminder::ReminderStatus::from_snapshot(
                app.state::<crate::tray::TrayStatus>().current(),
            ),
            countdown_request_id: state
                .countdown
                .filter(|id| pending.as_ref().is_some_and(|p| p.request_id == *id)),
            pending,
            opening_generation: state.generation,
            visible: state.visible,
        }
    }
    pub(crate) fn hide(app: &tauri::AppHandle) {
        let Some(runtime) = app.try_state::<Runtime>() else {
            return;
        };
        let generation = {
            let mut state = runtime
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.visible = false;
            state.countdown = None;
            state.ready = false;
            state.generation
        };
        if let Ok(panel) = app.get_webview_panel(LABEL) {
            panel.hide();
        }
        let _ = app.emit_to(
            EventTarget::webview_window(LABEL),
            "tray-panel-hidden",
            generation,
        );
    }
    pub(crate) fn focus_lost(app: &tauri::AppHandle) {
        use tauri_nspanel::objc2_app_kit::{NSEvent, NSScreen};
        if let Some(marker) = tauri_nspanel::objc2::MainThreadMarker::new() {
            let cursor = NSEvent::mouseLocation();
            let screens = NSScreen::screens(marker);
            if let Some(primary) = screens.firstObject() {
                let top = primary.frame().origin.y + primary.frame().size.height;
                let runtime = app.state::<Runtime>();
                let bounds = runtime
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .tray_bounds;
                if bounds.is_some_and(|[x, y, w, h]| {
                    cursor.x >= x
                        && cursor.x <= x + w
                        && top - cursor.y >= y
                        && top - cursor.y <= y + h
                }) {
                    return;
                }
            }
        }
        hide(app);
    }
    fn schedule_readiness_fallback(app: &tauri::AppHandle, generation: u64) {
        // A failed webview must never strand the only entry point to the app.
        let handle = app.clone();
        tauri::async_runtime::spawn_blocking(move || {
            std::thread::sleep(std::time::Duration::from_secs(5));
            let next = handle.clone();
            let _ = handle.run_on_main_thread(move || {
                let fallback = {
                    let runtime = next.state::<Runtime>();
                    let s = runtime
                        .0
                        .lock()
                        .unwrap_or_else(std::sync::PoisonError::into_inner);
                    s.visible && s.generation == generation && !s.ready
                };
                if fallback {
                    hide(&next);
                    eprintln!("Tray panel did not become ready; opening dashboard");
                    crate::instance::reveal_dashboard(&next);
                }
            });
        });
    }

    pub(crate) fn toggle(tray: &tauri::tray::TrayIcon) -> Result<(), String> {
        let app = tray.app_handle();
        if app
            .state::<Runtime>()
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .visible
        {
            hide(app);
            return Ok(());
        }
        // Read the status item's actual NSWindow/NSScreen. Global physical ranges
        // overlap on mixed-scale displays; never infer its host from those ranges.
        let (bounds, area, desktop_top) = tray
            .with_inner_tray_icon(|inner| {
                use tauri_nspanel::objc2_app_kit::NSScreen;
                let marker = tauri_nspanel::objc2::MainThreadMarker::new()
                    .ok_or("tray geometry requires the main thread")?;
                let item = inner
                    .ns_status_item()
                    .ok_or("native status item is unavailable")?;
                let button = item
                    .button(marker)
                    .ok_or("status item button is unavailable")?;
                let window = button.window().ok_or("status item window is unavailable")?;
                let screen = window
                    .screen()
                    .ok_or("status item display is unavailable")?;
                let screens = NSScreen::screens(marker);
                let primary = screens
                    .firstObject()
                    .ok_or("primary display is unavailable")?
                    .frame();
                let top = primary.origin.y + primary.size.height;
                let frame = window.frame();
                let work = screen.visibleFrame();
                Ok::<_, String>((
                    super::top_left_rect(
                        top,
                        [
                            frame.origin.x,
                            frame.origin.y,
                            frame.size.width,
                            frame.size.height,
                        ],
                    ),
                    super::top_left_rect(
                        top,
                        [
                            work.origin.x,
                            work.origin.y,
                            work.size.width,
                            work.size.height,
                        ],
                    ),
                    top,
                ))
            })
            .map_err(|e| e.to_string())??;
        let (x, y) = super::position(
            bounds[0],
            bounds[1],
            bounds[2],
            bounds[3],
            area,
            [WIDTH, HEIGHT],
        );
        let panel = app
            .get_webview_panel(LABEL)
            .map_err(|_| "tray panel is unavailable")?;
        // AppKit placement stays in global logical points all the way to the window.
        panel
            .as_panel()
            .setFrameTopLeftPoint(tauri_nspanel::objc2_foundation::NSPoint::new(
                x,
                desktop_top - y,
            ));
        let generation = {
            let runtime = app.state::<Runtime>();
            let mut state = runtime
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            state.tray_bounds = Some(bounds);
            state.generation += 1;
            state.visible = true;
            state.countdown = None;
            state.ready = false;
            state.generation
        };
        app.emit_to(
            EventTarget::webview_window(LABEL),
            "tray-panel-opening",
            generation,
        )
        .map_err(|e| e.to_string())?;
        schedule_readiness_fallback(app, generation);
        Ok(())
    }
    pub(crate) fn ready(
        app: &tauri::AppHandle,
        generation: u64,
        webview: &tauri_nspanel::objc2_app_kit::NSView,
    ) -> Result<(), String> {
        let runtime = app.state::<Runtime>();
        {
            let state = runtime
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !super::current_opening(state.visible, state.generation, generation) {
                return Err("tray panel opening is stale".into());
            }
            if state.ready {
                return Ok(());
            }
        }
        let panel = app
            .get_webview_panel(LABEL)
            .map_err(|_| "tray panel is unavailable".to_owned())?;
        panel.show();
        if !webview.acceptsFirstResponder() || !panel.make_first_responder(Some(webview)) {
            hide(app);
            crate::instance::reveal_dashboard(app);
            return Err("tray panel could not acquire keyboard focus".into());
        }
        panel.make_key_window();
        if !panel.as_panel().isKeyWindow() {
            hide(app);
            crate::instance::reveal_dashboard(app);
            return Err("tray panel did not acquire keyboard focus".into());
        }
        runtime
            .0
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .ready = true;
        Ok(())
    }
    pub(crate) fn action(
        app: &tauri::AppHandle,
        action: Action,
        generation: u64,
        request_id: Option<u64>,
    ) -> Result<(), String> {
        {
            let runtime = app.state::<Runtime>();
            let s = runtime
                .0
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner);
            if !super::current_opening(s.visible, s.generation, generation) {
                return Err("tray panel opening is stale".into());
            }
        }
        let control = app.state::<ReminderControl>();
        match action {
            Action::Begin => {
                let existing = control.preparation_status().request_id;
                let result = control.begin_manual_preparation()?;
                let runtime = app.state::<Runtime>();
                let mut s = runtime
                    .0
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner);
                if existing.is_none() && s.visible && s.generation == generation {
                    s.countdown = result.request_id;
                }
            }
            Action::Cancel => {
                control
                    .cancel_manual_preparation(request_id.ok_or("missing preparation request")?)?;
            }
            Action::Pause => {
                control.request(ReminderAction::Pause)?;
            }
            Action::Resume => {
                control.request(ReminderAction::Resume)?;
            }
            Action::Open | Action::Hide | Action::Quit => {
                let next = app.clone();
                let (sender, receiver) = std::sync::mpsc::sync_channel(1);
                app.run_on_main_thread(move || {
                    hide(&next);
                    match action {
                        Action::Open => crate::instance::reveal_dashboard(&next),
                        Action::Quit => next.exit(0),
                        _ => {}
                    }
                    let _ = sender.send(());
                })
                .map_err(|e| e.to_string())?;
                receiver
                    .recv_timeout(std::time::Duration::from_secs(3))
                    .map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}
#[cfg(target_os = "macos")]
pub(crate) use native::{hide as hide_on_main, install, toggle};

#[cfg(any(target_os = "macos", test))]
fn top_left_rect(desktop_top: f64, rect: [f64; 4]) -> [f64; 4] {
    [rect[0], desktop_top - rect[1] - rect[3], rect[2], rect[3]]
}

#[cfg(any(target_os = "macos", test))]
fn current_opening(visible: bool, current: u64, requested: u64) -> bool {
    visible && current == requested
}

#[cfg(any(target_os = "macos", test))]
fn position(x: f64, y: f64, w: f64, h: f64, area: [f64; 4], panel: [f64; 2]) -> (f64, f64) {
    (
        (x + w / 2.0 - panel[0] / 2.0).clamp(area[0], (area[0] + area[2] - panel[0]).max(area[0])),
        (y + h + 6.0).clamp(area[1], (area[1] + area[3] - panel[1]).max(area[1])),
    )
}

pub(crate) fn hide_for_break(app: &tauri::AppHandle) {
    #[cfg(target_os = "macos")]
    {
        let next = app.clone();
        let _ = app.run_on_main_thread(move || native::hide(&next));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = app;
}

pub(crate) fn window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    if let tauri::WindowEvent::CloseRequested { api, .. } = event {
        api.prevent_close();
    }
    #[cfg(target_os = "macos")]
    if matches!(event, tauri::WindowEvent::Focused(false)) {
        native::focus_lost(window.app_handle());
    }
    if matches!(event, tauri::WindowEvent::CloseRequested { .. }) {
        hide_for_break(window.app_handle());
    }
}

#[tauri::command]
pub(crate) fn tray_panel_state(window: WebviewWindow) -> Result<Snapshot, String> {
    authorize(window.label(), cfg!(target_os = "macos"))?;
    #[cfg(target_os = "macos")]
    return Ok(native::snapshot(window.app_handle()));
    #[cfg(not(target_os = "macos"))]
    Err("tray panel is unavailable".into())
}
#[tauri::command]
pub(crate) async fn tray_panel_action(
    window: WebviewWindow,
    action: Action,
    opening_generation: u64,
    request_id: Option<u64>,
) -> Result<Snapshot, String> {
    authorize(window.label(), cfg!(target_os = "macos"))?;
    #[cfg(target_os = "macos")]
    {
        let app = window.app_handle().clone();
        tauri::async_runtime::spawn_blocking(move || {
            native::action(&app, action, opening_generation, request_id)?;
            Ok(native::snapshot(&app))
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (action, opening_generation, request_id);
        Err("tray panel is unavailable".into())
    }
}
#[tauri::command]
pub(crate) async fn tray_panel_ready(
    window: WebviewWindow,
    opening_generation: u64,
) -> Result<Snapshot, String> {
    authorize(window.label(), cfg!(target_os = "macos"))?;
    #[cfg(target_os = "macos")]
    {
        let app = window.app_handle().clone();
        let next = app.clone();
        let (tx, rx) = std::sync::mpsc::channel();
        window
            .with_webview(move |platform| {
                // SAFETY: Tauri owns this live WKWebView (an NSView subclass); the borrow stays on the main thread.
                let webview = unsafe {
                    &*platform
                        .inner()
                        .cast::<tauri_nspanel::objc2_app_kit::NSView>()
                };
                let _ = tx.send(native::ready(&next, opening_generation, webview));
            })
            .map_err(|e| e.to_string())?;
        tauri::async_runtime::spawn_blocking(move || {
            rx.recv().map_err(|e| e.to_string())??;
            Ok(native::snapshot(&app))
        })
        .await
        .map_err(|e| e.to_string())?
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = opening_generation;
        Err("tray panel is unavailable".into())
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn caller_identity_is_exact_and_platform_restricted() {
        assert!(authorize(LABEL, true).is_ok());
        for label in ["main", "tray-panel-other", "overlay-1", ""] {
            assert!(authorize(label, true).is_err());
        }
        assert!(authorize(LABEL, false).is_err());
    }
    #[test]
    fn native_host_geometry_avoids_ambiguous_mixed_scale_ranges() {
        // A 1x external at x1512 alongside a 2x primary: physical x1600 could
        // otherwise be misidentified as primary logical x800.
        let tray = top_left_rect(982.0, [1600.0, 958.0, 24.0, 24.0]);
        let host = top_left_rect(982.0, [1512.0, -98.0, 1920.0, 1056.0]);
        assert_eq!(
            position(tray[0], tray[1], tray[2], tray[3], host, [250.0, 270.0]),
            (1512.0, 30.0)
        );
    }
    #[test]
    fn stale_readiness_and_actions_cannot_reopen_a_hidden_or_new_opening() {
        assert!(current_opening(true, 1, 1));
        assert!(!current_opening(false, 1, 1));
        assert!(!current_opening(true, 2, 1));
        assert!(!current_opening(false, 2, 2));
    }
    #[test]
    fn placement_clamps_negative_display_edges() {
        assert_eq!(
            position(
                -1910.0,
                0.0,
                20.0,
                24.0,
                [-1920.0, 24.0, 1920.0, 1056.0],
                [282.0, 298.0]
            ),
            (-1920.0, 30.0)
        );
        assert_eq!(
            position(
                1910.0,
                0.0,
                20.0,
                24.0,
                [0.0, 24.0, 1920.0, 1056.0],
                [282.0, 298.0]
            ),
            (1638.0, 30.0)
        );
    }
}
