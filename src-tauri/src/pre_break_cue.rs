// src-tauri/src/pre_break_cue.rs

use crate::reminder::{ReminderAction, ReminderControl};
use crate::tray::{TrayPhase, TraySnapshot};
use crate::{authorize_main_caller, overlay::OverlayController};
#[cfg(target_os = "macos")]
mod interaction;
use serde::Serialize;
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc, Mutex,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{
    webview::PageLoadEvent, AppHandle, LogicalPosition, LogicalSize, Manager, Monitor, State,
    WebviewUrl, WebviewWindow, WebviewWindowBuilder,
};
#[cfg(target_os = "macos")]
use tauri_nspanel::{
    objc2_app_kit::{
        NSScreen, NSWindowAnimationBehavior, NSWindowCollectionBehavior, NSWindowStyleMask,
    },
    ManagerExt, WebviewWindowExt,
};

pub(crate) const CUE_LEAD_MILLISECONDS: u64 = 60_000;
const MACOS_CUE_WIDTH: f64 = 200.0;
const MACOS_CUE_HEIGHT: f64 = 36.0;
const MACOS_CUE_TOP_GAP: f64 = 12.0;
#[cfg(any(target_os = "macos", test))]
const NOTCH_WING_WIDTH: f64 = 64.0;
#[cfg(any(target_os = "macos", test))]
const NOTCH_SHOULDER_WIDTH: f64 = 9.0;
const X11_CUE_WIDTH: f64 = 456.0;
const X11_CUE_HEIGHT: f64 = 160.0;
const X11_CUE_TOP_GAP: f64 = 48.0;
const CUE_PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(5);
const PREVIEW_CLOSE_MILLISECONDS: u64 = 17_000;
const JAVASCRIPT_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
static CUE_RUN_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CueFrontendPlatform {
    Macos,
    Linux,
}

fn cue_frontend_platform() -> CueFrontendPlatform {
    if cfg!(target_os = "macos") {
        CueFrontendPlatform::Macos
    } else {
        CueFrontendPlatform::Linux
    }
}

fn cue_platform_initialization_script(platform: CueFrontendPlatform) -> &'static str {
    match platform {
        CueFrontendPlatform::Macos => r#"window.__UNFOCUS_PRE_BREAK_CUE_PLATFORM__ = "macos";"#,
        CueFrontendPlatform::Linux => r#"window.__UNFOCUS_PRE_BREAK_CUE_PLATFORM__ = "linux";"#,
    }
}

pub(crate) fn pre_break_cue_platform_enabled(
    qualified_x11_session: bool,
    target_is_macos: bool,
) -> bool {
    qualified_x11_session || target_is_macos
}

fn pre_break_cue_visible_on_all_workspaces(target_is_macos: bool) -> bool {
    target_is_macos
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CuePanelPolicy {
    level: i64,
    joins_all_spaces: bool,
    full_screen_auxiliary: bool,
    nonactivating: bool,
    can_become_key_window: bool,
    can_become_main_window: bool,
    ignores_pointer_events: bool,
}

fn cue_panel_policy() -> CuePanelPolicy {
    CuePanelPolicy {
        level: 25,
        joins_all_spaces: true,
        full_screen_auxiliary: true,
        nonactivating: true,
        can_become_key_window: false,
        can_become_main_window: false,
        ignores_pointer_events: true,
    }
}

#[cfg(target_os = "macos")]
tauri_nspanel::tauri_panel! {
    panel!(UnfocusPreBreakCuePanel {
        config: {
            can_become_key_window: cue_panel_policy().can_become_key_window,
            can_become_main_window: cue_panel_policy().can_become_main_window
        }
    })
}

#[cfg(target_os = "macos")]
fn configure_macos_cue_panel(window: &WebviewWindow) -> Result<(), String> {
    let policy = cue_panel_policy();
    let panel = window
        .to_panel::<UnfocusPreBreakCuePanel>()
        .map_err(|error| format!("could not convert pre-break cue to an NSPanel: {error}"))?;
    panel.set_released_when_closed(false);
    panel.set_has_shadow(false);
    // Only the wings animate; AppKit must not zoom the whole camera-aligned window.
    panel
        .as_panel()
        .setAnimationBehavior(NSWindowAnimationBehavior::None);
    let mut style = panel.as_panel().styleMask();
    if policy.nonactivating {
        style |= NSWindowStyleMask::NonactivatingPanel;
    }
    panel.set_style_mask(style);
    panel.set_level(policy.level);
    panel.set_hides_on_deactivate(false);
    let mut collection_behavior = NSWindowCollectionBehavior::Stationary;
    if policy.joins_all_spaces {
        collection_behavior |= NSWindowCollectionBehavior::CanJoinAllSpaces;
    }
    if policy.full_screen_auxiliary {
        collection_behavior |= NSWindowCollectionBehavior::FullScreenAuxiliary;
    }
    panel.set_collection_behavior(collection_behavior);
    panel.set_accepts_mouse_moved_events(true);
    panel.set_ignores_mouse_events(policy.ignores_pointer_events);
    Ok(())
}

#[cfg(target_os = "macos")]
fn set_macos_cue_visibility(window: &WebviewWindow, visible: bool) -> Result<(), String> {
    let panel = window
        .app_handle()
        .get_webview_panel(window.label())
        .map_err(|_| "could not find the converted pre-break cue panel".to_owned())?;
    if visible {
        panel.set_ignores_mouse_events(true);
        panel.order_front_regardless();
    } else {
        panel.hide();
    }
    Ok(())
}

fn close_cue_window_on_main(app: &AppHandle, label: &str) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    let window = match app.get_webview_panel(label) {
        Ok(panel) => panel.to_window().or_else(|| app.get_webview_window(label)),
        Err(_) => app.get_webview_window(label),
    };
    #[cfg(not(target_os = "macos"))]
    let window = app.get_webview_window(label);

    let Some(window) = window else {
        return Ok(());
    };

    let result = window
        .close()
        .map_err(|error| format!("could not close pre-break cue {label}: {error}"));
    if result.is_ok() && label.starts_with("cue-preview-") {
        if let (Some((run_id, _)), Some(main)) = (
            cue_parameters_from_label(label),
            app.get_webview_window("main"),
        ) {
            let _ = main.eval(format!("window.dispatchEvent(new CustomEvent('unfocus-pre-break-cue-preview-ended', {{ detail: {{ runId: {run_id} }} }}))"));
        }
    }
    result
}

fn request_cue_window_close(app: &AppHandle, label: String, reason: &str) {
    let close_app = app.clone();
    let reason = reason.to_owned();
    let task_reason = reason.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        if let Err(error) = close_cue_window_on_main(&close_app, &label) {
            eprintln!("could not close pre-break cue ({task_reason}): {error}");
        }
    }) {
        eprintln!("could not schedule pre-break cue close ({reason}): {error}");
    }
}

#[cfg(target_os = "macos")]
fn request_cue_handoff(app: &AppHandle, label: String) {
    if let Some(window) = app.get_webview_window(&label) {
        // Native-to-local-DOM signal: cue windows need no event capability.
        let _ = window.eval("window.dispatchEvent(new Event('unfocus-cue-handoff'))");
    }
    let close_app = app.clone();
    let close_label = label.clone();
    if std::thread::Builder::new()
        .name("unfocus-cue-handoff".into())
        .spawn(move || {
            std::thread::sleep(Duration::from_millis(350));
            request_cue_window_close(&close_app, close_label, "overlay handoff complete");
        })
        .is_err()
    {
        request_cue_window_close(app, label, "handoff worker unavailable");
    }
}

fn abort_cue_window(window: &WebviewWindow, reason: &'static str) {
    request_cue_window_close(window.app_handle(), window.label().to_owned(), reason);
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct CueGeometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Logical points, resolved only by native code before a hidden cue is revealed.
#[derive(Debug, Clone, Copy, PartialEq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct CueLayout {
    is_notched: bool,
    notch_width: f64,
    wing_width: f64,
    shoulder_width: f64,
    height: f64,
}

#[cfg(any(target_os = "macos", test))]
fn notch_placement(display: DisplayRect, notch: CueGeometry) -> Option<(CueGeometry, CueLayout)> {
    let values = [
        display.x,
        display.y,
        display.width,
        display.height,
        notch.x,
        notch.y,
        notch.width,
        notch.height,
    ];
    if values.iter().any(|value| !value.is_finite())
        || notch.width <= 0.0
        || notch.height <= 0.0
        || notch.height > display.height
        || (notch.y - display.y).abs() > 1.0
        || notch.x - NOTCH_WING_WIDTH - NOTCH_SHOULDER_WIDTH < display.x
        || notch.x + notch.width + NOTCH_WING_WIDTH + NOTCH_SHOULDER_WIDTH
            > display.x + display.width
    {
        return None;
    }
    Some((
        CueGeometry {
            x: notch.x - NOTCH_WING_WIDTH - NOTCH_SHOULDER_WIDTH,
            y: display.y,
            width: notch.width + 2.0 * (NOTCH_WING_WIDTH + NOTCH_SHOULDER_WIDTH),
            height: notch.height,
        },
        CueLayout {
            is_notched: true,
            notch_width: notch.width,
            wing_width: NOTCH_WING_WIDTH,
            shoulder_width: NOTCH_SHOULDER_WIDTH,
            height: notch.height,
        },
    ))
}

#[cfg(target_os = "macos")]
fn macos_notch_placement(monitor: &Monitor) -> Option<(CueGeometry, CueLayout)> {
    if !tauri_nspanel::objc2::available!(macos = 12.0) {
        return None;
    }
    let mtm = tauri_nspanel::objc2::MainThreadMarker::new()?;
    let screens = NSScreen::screens(mtm);
    // AppKit has a bottom-left origin; Tauri/Quartz use top-left screen points.
    // screens[0] owns the global menu bar and defines that shared origin.
    let primary = screens.firstObject()?.frame();
    let desktop_top = primary.origin.y + primary.size.height;
    let display = display_rect(monitor, false)?;
    let screen = screens.iter().find(|screen| {
        let frame = screen.frame();
        (frame.origin.x - display.x).abs() < 1.0
            && (desktop_top - frame.origin.y - frame.size.height - display.y).abs() < 1.0
            && (frame.size.width - display.width).abs() < 1.0
            && (frame.size.height - display.height).abs() < 1.0
    })?;
    let top = screen.safeAreaInsets().top;
    let left = screen.auxiliaryTopLeftArea();
    let right = screen.auxiliaryTopRightArea();
    if top <= 0.0
        || left.size.width <= 0.0
        || right.size.width <= 0.0
        || (left.size.height - top).abs() > 1.0
        || (right.size.height - top).abs() > 1.0
    {
        return None;
    }
    let notch_left = left.origin.x + left.size.width;
    notch_placement(
        display,
        CueGeometry {
            x: notch_left,
            y: desktop_top - left.origin.y - left.size.height,
            width: right.origin.x - notch_left,
            height: top,
        },
    )
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct DisplayRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
    primary: bool,
}

fn intersection_area(left: DisplayRect, right: DisplayRect) -> f64 {
    let width = (left.x + left.width).min(right.x + right.width) - left.x.max(right.x);
    let height = (left.y + left.height).min(right.y + right.height) - left.y.max(right.y);
    width.max(0.0) * height.max(0.0)
}

fn select_active_display_index(displays: &[DisplayRect], window: Option<DisplayRect>) -> usize {
    let primary = displays
        .iter()
        .position(|display| display.primary)
        .unwrap_or(0);
    let Some(window) = window else {
        return primary;
    };
    let Some((index, area)) = displays
        .iter()
        .enumerate()
        .map(|(index, display)| (index, intersection_area(*display, window)))
        .max_by(|left, right| left.1.total_cmp(&right.1))
    else {
        return primary;
    };
    if area > 0.0 {
        index
    } else {
        primary
    }
}

fn display_rect(monitor: &Monitor, primary: bool) -> Option<DisplayRect> {
    let scale = monitor.scale_factor();
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }
    Some(DisplayRect {
        x: f64::from(monitor.position().x) / scale,
        y: f64::from(monitor.position().y) / scale,
        width: f64::from(monitor.size().width) / scale,
        height: f64::from(monitor.size().height) / scale,
        primary,
    })
}

fn same_display(left: &Monitor, right: &Monitor) -> bool {
    left.position() == right.position() && left.size() == right.size()
}

fn selected_cue_monitor(app: &AppHandle) -> Result<Monitor, String> {
    let primary = app
        .primary_monitor()
        .map_err(|error| format!("could not read the primary monitor: {error}"))?
        .ok_or_else(|| "Tauri did not report a primary monitor".to_owned())?;

    #[cfg(target_os = "macos")]
    {
        let Ok(monitors) = app.available_monitors() else {
            return Ok(primary);
        };
        if monitors.is_empty() {
            return Ok(primary);
        }
        let display_rects: Vec<_> = monitors
            .iter()
            .map(|monitor| display_rect(monitor, same_display(monitor, &primary)))
            .collect::<Option<_>>()
            .unwrap_or_default();
        if display_rects.len() != monitors.len() {
            return Ok(primary);
        }
        let window = match crate::probes::frontmost_window_bounds() {
            Ok(Some(window)) => Some(DisplayRect {
                x: window.x,
                y: window.y,
                width: window.width,
                height: window.height,
                primary: false,
            }),
            Ok(None) | Err(_) => None,
        };
        let selected = monitors[select_active_display_index(&display_rects, window)].clone();
        if selected.name().is_none() {
            Ok(primary)
        } else {
            Ok(selected)
        }
    }
    #[cfg(not(target_os = "macos"))]
    Ok(primary)
}

#[cfg(test)]
fn cue_geometry(
    work_x: i32,
    work_y: i32,
    work_width: u32,
    work_height: u32,
    scale_factor: f64,
) -> Result<CueGeometry, String> {
    cue_geometry_for_platform(work_x, work_y, work_width, work_height, scale_factor, true)
}

fn cue_geometry_for_platform(
    work_x: i32,
    work_y: i32,
    work_width: u32,
    work_height: u32,
    scale_factor: f64,
    target_is_macos: bool,
) -> Result<CueGeometry, String> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return Err("primary monitor reported an invalid scale factor".into());
    }

    let work_width = f64::from(work_width) / scale_factor;
    let work_height = f64::from(work_height) / scale_factor;
    let (canvas_width, canvas_height, top_gap) = if target_is_macos {
        (MACOS_CUE_WIDTH, MACOS_CUE_HEIGHT, MACOS_CUE_TOP_GAP)
    } else {
        (X11_CUE_WIDTH, X11_CUE_HEIGHT, X11_CUE_TOP_GAP)
    };
    if work_width <= 0.0 || work_height <= top_gap {
        return Err("primary monitor reported an unusable work area".into());
    }

    let width = work_width.min(canvas_width);
    let height = (work_height - top_gap).min(canvas_height);
    Ok(CueGeometry {
        x: f64::from(work_x) / scale_factor + (work_width - width) / 2.0,
        y: f64::from(work_y) / scale_factor + top_gap,
        width,
        height,
    })
}

fn parse_canonical_u64(value: &str) -> Option<u64> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse().ok()
}

fn cue_parameters_from_label(label: &str) -> Option<(u64, u64)> {
    let mut parts = label.split('-');
    if parts.next()? != "cue" {
        return None;
    }
    let run = parts.next()?;
    let run_id = if run == "preview" {
        parse_canonical_u64(parts.next()?)?
    } else {
        parse_canonical_u64(run)?
    };
    let deadline_ms = parse_canonical_u64(parts.next()?)?;
    if parts.next().is_some()
        || run_id == 0
        || run_id > JAVASCRIPT_MAX_SAFE_INTEGER
        || deadline_ms == 0
        || deadline_ms > JAVASCRIPT_MAX_SAFE_INTEGER
    {
        return None;
    }
    Some((run_id, deadline_ms))
}

#[tauri::command]
pub(crate) async fn prepare_pre_break_cue(window: WebviewWindow) -> Result<CueLayout, String> {
    if cue_parameters_from_label(window.label()).is_none() {
        return Err("this command is only available to a valid pre-break cue window".into());
    }
    let (sender, receiver) = mpsc::sync_channel(1);
    let cue = window.clone();
    window
        .run_on_main_thread(move || {
            let result = set_pre_break_cue_visibility(cue.clone(), false)
                .and_then(|()| position_cue_window(&cue));
            let _ = sender.send(result);
        })
        .map_err(|error| format!("could not prepare pre-break cue: {error}"))?;
    tauri::async_runtime::spawn_blocking(move || {
        receiver
            .recv_timeout(CUE_PAGE_LOAD_TIMEOUT)
            .map_err(|error| format!("cue preparation did not finish: {error}"))?
    })
    .await
    .map_err(|error| format!("cue preparation worker failed: {error}"))?
}

#[tauri::command]
pub(crate) async fn skip_pre_break_cue(window: WebviewWindow) -> Result<(), String> {
    if !cfg!(target_os = "macos") {
        return Err("notch skip is only available on macOS".into());
    }
    let (run_id, deadline) = cue_parameters_from_label(window.label())
        .ok_or("this command is only available to a valid pre-break cue window")?;
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis();
    if now >= u128::from(deadline) {
        return Err("this cue has already ended".into());
    }
    if window.label().starts_with("cue-preview-") {
        let controller = window.state::<PreBreakCueController>();
        let preview = {
            let mut state = controller.state();
            if state
                .preview
                .as_ref()
                .and_then(|preview| preview.label.as_deref())
                != Some(window.label())
            {
                return Err("this preview is no longer current".into());
            }
            state
                .lifecycle
                .close_preview(run_id)
                .map_err(|error| error.message())?;
            state.preview.take().ok_or("preview already closed")?
        };
        preview.cancelled.store(true, Ordering::Release);
        controller.retire_after_skip(window.app_handle(), window.label().to_owned());
        return Ok(());
    }
    let control = window.state::<ReminderControl>().inner().clone();
    tauri::async_runtime::spawn_blocking(move || control.request(ReminderAction::SkipCue(run_id)))
        .await
        .map_err(|error| format!("cue skip task failed: {error}"))??;
    Ok(())
}

#[tauri::command]
pub(crate) fn set_pre_break_cue_interactive(
    window: WebviewWindow,
    enabled: bool,
) -> Result<(), String> {
    if cue_parameters_from_label(window.label()).is_none() {
        return Err("this command is only available to a valid pre-break cue window".into());
    }
    #[cfg(target_os = "macos")]
    return interaction::set_enabled(&window, enabled);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = enabled;
        Err("notch interaction is only available on macOS".into())
    }
}

#[tauri::command]
pub(crate) fn set_pre_break_cue_visibility(
    window: WebviewWindow,
    visible: bool,
) -> Result<(), String> {
    if cue_parameters_from_label(window.label()).is_none() {
        return Err("this command is only available to a valid pre-break cue window".into());
    }
    if !visible {
        #[cfg(target_os = "macos")]
        return set_macos_cue_visibility(&window, false);
        #[cfg(not(target_os = "macos"))]
        return window
            .hide()
            .map_err(|error| format!("could not hide pre-break cue: {error}"));
    }

    #[cfg(target_os = "macos")]
    {
        if let Err(error) = window.set_ignore_cursor_events(true) {
            abort_cue_window(&window, "click-through setup failed");
            return Err(format!(
                "could not make the pre-break cue click-through: {error}"
            ));
        }
        set_macos_cue_visibility(&window, true).inspect_err(|_| {
            abort_cue_window(&window, "native reveal failed");
        })
    }
    #[cfg(not(target_os = "macos"))]
    {
        window
            .show()
            .map_err(|error| format!("could not reveal pre-break cue: {error}"))?;
        if let Err(error) = window.set_ignore_cursor_events(true) {
            let _ = window.close();
            return Err(format!(
                "could not make the pre-break cue click-through: {error}"
            ));
        }
        Ok(())
    }
}

fn position_cue_window(window: &WebviewWindow) -> Result<CueLayout, String> {
    let monitor = selected_cue_monitor(window.app_handle())?;
    let work_area = monitor.work_area();
    let geometry = cue_geometry_for_platform(
        work_area.position.x,
        work_area.position.y,
        work_area.size.width,
        work_area.size.height,
        monitor.scale_factor(),
        cfg!(target_os = "macos"),
    )?;
    let layout = CueLayout {
        is_notched: false,
        notch_width: 0.0,
        wing_width: geometry.width / 2.0,
        shoulder_width: 0.0,
        height: geometry.height,
    };
    #[cfg(target_os = "macos")]
    let (geometry, layout) = macos_notch_placement(&monitor).unwrap_or((geometry, layout));
    // Resize first: AppKit keeps the bottom edge fixed during a size change.
    // Positioning first would shift a 36-point fallback two points above a 38-point notch.
    window
        .set_size(LogicalSize::new(geometry.width, geometry.height))
        .map_err(|error| format!("could not size pre-break cue: {error}"))?;
    window
        .set_position(LogicalPosition::new(geometry.x, geometry.y))
        .map_err(|error| format!("could not position pre-break cue: {error}"))?;
    #[cfg(target_os = "macos")]
    interaction::set_layout(window, layout);
    Ok(layout)
}

fn next_cue_run_id() -> u64 {
    let current = CUE_RUN_ID.load(Ordering::Relaxed);
    let run_id = if (1..=JAVASCRIPT_MAX_SAFE_INTEGER).contains(&current) {
        current
    } else {
        1
    };
    CUE_RUN_ID.store(
        if run_id == JAVASCRIPT_MAX_SAFE_INTEGER {
            1
        } else {
            run_id + 1
        },
        Ordering::Relaxed,
    );
    run_id
}

fn cue_deadline_ms(wall_now: SystemTime, remaining_ms: u64) -> Result<u64, String> {
    wall_now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock predates the Unix epoch".to_owned())?
        .as_millis()
        .try_into()
        .ok()
        .and_then(|now_ms: u64| now_ms.checked_add(remaining_ms))
        .filter(|deadline| (1..=JAVASCRIPT_MAX_SAFE_INTEGER).contains(deadline))
        .ok_or_else(|| "cue deadline exceeds JavaScript's safe-integer range".to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CueReminderState {
    working: bool,
    remaining_ms: Option<u64>,
    revision: u64,
    enabled: bool,
    presentation_allowed: bool,
}

impl CueReminderState {
    fn from_snapshot(snapshot: &TraySnapshot, enabled: bool, presentation_allowed: bool) -> Self {
        Self {
            working: snapshot.phase == TrayPhase::Working,
            remaining_ms: snapshot.remaining_milliseconds,
            revision: snapshot.state_revision,
            enabled,
            presentation_allowed,
        }
    }

    fn in_lead_window(self) -> bool {
        self.working
            && self
                .remaining_ms
                .is_some_and(|remaining| (1..=CUE_LEAD_MILLISECONDS).contains(&remaining))
    }

    fn should_show(self) -> bool {
        self.in_lead_window() && self.enabled && self.presentation_allowed
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CueDecision {
    None,
    Create,
    Close,
}

fn cue_decision(
    state: CueReminderState,
    attempted_revision: Option<u64>,
    occupied_revision: Option<u64>,
    cleanup_pending: bool,
) -> CueDecision {
    if let Some(occupied_revision) = occupied_revision {
        return if cleanup_pending || !state.should_show() || occupied_revision != state.revision {
            CueDecision::Close
        } else {
            CueDecision::None
        };
    }
    if state.should_show() && attempted_revision != Some(state.revision) {
        CueDecision::Create
    } else {
        CueDecision::None
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum CueOccupancy {
    #[default]
    Empty,
    Preview(u64),
    Production,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CueStartError {
    Overlay,
    Preview,
    Production,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CueCloseError {
    NoPreview,
    WrongRun,
}

impl CueStartError {
    fn message(self) -> &'static str {
        match self {
            Self::Overlay => "an overlay is already active",
            Self::Preview => "a pre-break cue preview is already active",
            Self::Production => "a scheduled pre-break cue is already active",
        }
    }
}

impl CueCloseError {
    fn message(self) -> &'static str {
        match self {
            Self::NoPreview => "the pre-break cue preview has already closed",
            Self::WrongRun => "the pre-break cue preview run ID does not match",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ProductionStart {
    Start,
    AlreadyActive,
    PreemptPreview(u64),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct CueLifecycle {
    occupancy: CueOccupancy,
}

impl CueLifecycle {
    fn begin_preview(&mut self, run_id: u64, overlay_active: bool) -> Result<(), CueStartError> {
        if overlay_active {
            return Err(CueStartError::Overlay);
        }
        match self.occupancy {
            CueOccupancy::Empty => {
                self.occupancy = CueOccupancy::Preview(run_id);
                Ok(())
            }
            CueOccupancy::Preview(_) => Err(CueStartError::Preview),
            CueOccupancy::Production => Err(CueStartError::Production),
        }
    }

    fn close_preview(&mut self, run_id: u64) -> Result<(), CueCloseError> {
        match self.occupancy {
            CueOccupancy::Preview(active) if active == run_id => {
                self.occupancy = CueOccupancy::Empty;
                Ok(())
            }
            CueOccupancy::Preview(_) => Err(CueCloseError::WrongRun),
            CueOccupancy::Empty | CueOccupancy::Production => Err(CueCloseError::NoPreview),
        }
    }

    fn begin_production(&mut self) -> ProductionStart {
        match std::mem::replace(&mut self.occupancy, CueOccupancy::Production) {
            CueOccupancy::Empty => ProductionStart::Start,
            CueOccupancy::Preview(run_id) => ProductionStart::PreemptPreview(run_id),
            CueOccupancy::Production => ProductionStart::AlreadyActive,
        }
    }

    fn end_production(&mut self) {
        if self.occupancy == CueOccupancy::Production {
            self.occupancy = CueOccupancy::Empty;
        }
    }
}

#[derive(Debug)]
struct PreviewCueRuntime {
    run_id: u64,
    label: Option<String>,
    cancelled: Arc<AtomicBool>,
}

#[derive(Debug, Default)]
struct CueRuntimeState {
    lifecycle: CueLifecycle,
    preview: Option<PreviewCueRuntime>,
    retiring: Option<String>,
    #[cfg(target_os = "macos")]
    pointers: std::collections::HashMap<String, Arc<Mutex<interaction::PointerState>>>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct PreBreakCueController {
    inner: Arc<Mutex<CueRuntimeState>>,
}

impl PreBreakCueController {
    fn state(&self) -> std::sync::MutexGuard<'_, CueRuntimeState> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn reserve_preview_at_boundary<OverlayOccupied>(
        &self,
        run_id: u64,
        overlay_occupied: OverlayOccupied,
    ) -> Result<Arc<AtomicBool>, CueStartError>
    where
        OverlayOccupied: FnOnce() -> bool,
    {
        let mut state = self.state();
        state.lifecycle.begin_preview(run_id, overlay_occupied())?;
        let cancelled = Arc::new(AtomicBool::new(false));
        state.preview = Some(PreviewCueRuntime {
            run_id,
            label: None,
            cancelled: Arc::clone(&cancelled),
        });
        Ok(cancelled)
    }

    fn activate_preview(&self, run_id: u64, label: String) -> bool {
        let mut state = self.state();
        if state.lifecycle.occupancy != CueOccupancy::Preview(run_id) {
            return false;
        }
        let Some(preview) = state
            .preview
            .as_mut()
            .filter(|preview| preview.run_id == run_id)
        else {
            return false;
        };
        preview.label = Some(label);
        true
    }

    fn fail_preview(&self, run_id: u64) {
        let mut state = self.state();
        if state.lifecycle.close_preview(run_id).is_ok() {
            state.preview = None;
        }
    }

    fn close_preview(&self, app: &AppHandle, run_id: u64) -> Result<(), CueCloseError> {
        let preview = {
            let mut state = self.state();
            state.lifecycle.close_preview(run_id)?;
            state.preview.take()
        };
        if let Some(preview) = preview {
            preview.cancelled.store(true, Ordering::Release);
            if let Some(label) = preview.label {
                request_cue_window_close(app, label, "preview closed");
            }
        }
        Ok(())
    }

    fn begin_production(&self, app: &AppHandle) {
        self.close_retiring(app);
        let preview = {
            let mut state = self.state();
            match state.lifecycle.begin_production() {
                ProductionStart::PreemptPreview(_) => state.preview.take(),
                ProductionStart::Start | ProductionStart::AlreadyActive => None,
            }
        };
        if let Some(preview) = preview {
            preview.cancelled.store(true, Ordering::Release);
            if let Some(label) = preview.label {
                request_cue_window_close(app, label, "production cue started");
            }
        }
    }

    fn end_production(&self) {
        self.state().lifecycle.end_production();
    }

    fn cancel_for_reminder_action(&self, app: &AppHandle) {
        self.close_retiring(app);
        let preview_run = self.state().preview.as_ref().map(|preview| preview.run_id);
        if let Some(run_id) = preview_run {
            let _ = self.close_preview(app, run_id);
        }
    }

    fn close_retiring(&self, app: &AppHandle) {
        let label = self.state().retiring.take();
        if let Some(label) = label {
            request_cue_window_close(app, label, "skip confirmation cancelled");
        }
    }

    fn retire_after_skip(&self, app: &AppHandle, label: String) {
        self.close_retiring(app);
        self.state().retiring = Some(label.clone());
        let close_app = app.clone();
        let close_label = label.clone();
        let controller = self.clone();
        if std::thread::Builder::new()
            .name("unfocus-cue-skip".into())
            .spawn(move || {
                // Frontend confirms for 450 ms, then retracts for 350 ms. Native cleanup
                // also runs if WebKit stops executing, without holding the scheduler.
                std::thread::sleep(Duration::from_millis(900));
                request_cue_window_close(&close_app, close_label.clone(), "break skipped");
                let mut state = controller.state();
                if state.retiring.as_deref() == Some(&close_label) {
                    state.retiring = None;
                }
            })
            .is_err()
        {
            request_cue_window_close(app, label, "skip cleanup unavailable");
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct PreBreakCuePreview {
    run_id: u64,
    closes_after_ms: u64,
}

fn schedule_preview_auto_close(
    app: AppHandle,
    controller: PreBreakCueController,
    run_id: u64,
    deadline_ms: u64,
) -> Result<(), String> {
    std::thread::Builder::new()
        .name("unfocus-pre-break-cue-preview".into())
        .spawn(move || {
            let now_ms = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map_or(0, |duration| {
                    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
                });
            std::thread::sleep(Duration::from_millis(deadline_ms.saturating_sub(now_ms)));
            let _ = controller.close_preview(&app, run_id);
        })
        .map(|_| ())
        .map_err(|error| format!("could not start the cue preview close worker: {error}"))
}

fn show_pre_break_cue_preview(
    app: &AppHandle,
    controller: &PreBreakCueController,
    overlay_controller: &OverlayController,
    wall_now: SystemTime,
) -> Result<PreBreakCuePreview, String> {
    if !cfg!(target_os = "macos") {
        return Err("pre-break cue preview is only available on macOS".into());
    }
    controller.close_retiring(app);
    let deadline_ms = cue_deadline_ms(wall_now, PREVIEW_CLOSE_MILLISECONDS)?;
    let run_id = next_cue_run_id();
    let cancelled = controller
        .reserve_preview_at_boundary(run_id, || overlay_controller.has_active_run())
        .map_err(|error| error.message().to_owned())?;
    let label = format!("cue-preview-{run_id}-{deadline_ms}");
    if cue_parameters_from_label(&label) != Some((run_id, deadline_ms)) {
        controller.fail_preview(run_id);
        return Err("generated cue preview label was invalid".into());
    }

    let (sender, receiver) = mpsc::sync_channel(1);
    let main_app = app.clone();
    let main_cancelled = Arc::clone(&cancelled);
    let main_label = label.clone();
    if let Err(error) = app.run_on_main_thread(move || {
        let _ = sender.send(create_cue_window(&main_app, main_label, main_cancelled));
    }) {
        cancelled.store(true, Ordering::Release);
        controller.fail_preview(run_id);
        return Err(format!("could not schedule cue preview creation: {error}"));
    }

    let creation = receiver
        .recv_timeout(CUE_PAGE_LOAD_TIMEOUT)
        .map_err(|error| {
            cancelled.store(true, Ordering::Release);
            controller.fail_preview(run_id);
            format!("cue preview creation did not finish: {error}")
        })?;
    let Some(cue) = creation.inspect_err(|_| {
        controller.fail_preview(run_id);
    })?
    else {
        controller.fail_preview(run_id);
        return Err("cue preview creation was cancelled".into());
    };
    if !controller.activate_preview(run_id, cue.label.clone()) {
        cue.cancelled.store(true, Ordering::Release);
        request_cue_window_close(app, cue.label, "production cue took precedence");
        return Err("a scheduled pre-break cue took precedence".into());
    }
    if let Err(error) =
        schedule_preview_auto_close(app.clone(), controller.clone(), run_id, deadline_ms)
    {
        let _ = controller.close_preview(app, run_id);
        return Err(error);
    }

    Ok(PreBreakCuePreview {
        run_id,
        closes_after_ms: PREVIEW_CLOSE_MILLISECONDS,
    })
}

#[tauri::command]
pub(crate) async fn show_pre_break_cue_test(
    window: WebviewWindow,
    controller: State<'_, PreBreakCueController>,
    overlay_controller: State<'_, OverlayController>,
) -> Result<PreBreakCuePreview, String> {
    authorize_main_caller(window.label())?;
    let app = window.app_handle().clone();
    let controller = controller.inner().clone();
    let overlay_controller = overlay_controller.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        show_pre_break_cue_preview(&app, &controller, &overlay_controller, SystemTime::now())
    })
    .await
    .map_err(|error| format!("cue preview task failed: {error}"))?
}

#[tauri::command]
pub(crate) fn close_pre_break_cue_test(
    window: WebviewWindow,
    controller: State<'_, PreBreakCueController>,
    run_id: u64,
) -> Result<(), String> {
    authorize_main_caller(window.label())?;
    controller
        .close_preview(window.app_handle(), run_id)
        .map_err(|error| error.message().to_owned())
}

#[derive(Debug)]
struct CreatedCue {
    label: String,
    cancelled: Arc<AtomicBool>,
}

type CueCreationResult = Result<Option<CreatedCue>, String>;

#[derive(Debug)]
enum CueSlot {
    Pending {
        revision: u64,
        cancelled: Arc<AtomicBool>,
        receiver: Receiver<CueCreationResult>,
    },
    Active {
        revision: u64,
        cue: CreatedCue,
    },
}

impl CueSlot {
    fn revision(&self) -> u64 {
        match self {
            Self::Pending { revision, .. } | Self::Active { revision, .. } => *revision,
        }
    }

    fn cancel(&self) {
        match self {
            Self::Pending { cancelled, .. } => cancelled.store(true, Ordering::Release),
            Self::Active { cue, .. } => cue.cancelled.store(true, Ordering::Release),
        }
    }

    fn cleanup_pending(&self) -> bool {
        match self {
            Self::Pending { cancelled, .. } => cancelled.load(Ordering::Acquire),
            Self::Active { cue, .. } => cue.cancelled.load(Ordering::Acquire),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreBreakCue {
    platform_enabled: bool,
    attempted_revision: Option<u64>,
    slot: Option<CueSlot>,
    controller: PreBreakCueController,
}

impl PreBreakCue {
    pub(crate) fn skippable_revision(&self, run_id: u64) -> Option<u64> {
        match self.slot.as_ref()? {
            CueSlot::Active { revision, cue }
                if !cue.cancelled.load(Ordering::Acquire)
                    && cue_parameters_from_label(&cue.label)
                        .is_some_and(|(active, _)| active == run_id) =>
            {
                Some(*revision)
            }
            _ => None,
        }
    }

    pub(crate) fn finish_skip(&mut self, app: &AppHandle) {
        if let Some(CueSlot::Active { cue, .. }) = self.slot.take() {
            cue.cancelled.store(true, Ordering::Release);
            self.controller.end_production();
            self.controller.retire_after_skip(app, cue.label);
        }
    }
    pub(crate) fn new(platform_enabled: bool, controller: PreBreakCueController) -> Self {
        Self {
            platform_enabled,
            attempted_revision: None,
            slot: None,
            controller,
        }
    }

    pub(crate) fn reconcile(
        &mut self,
        app: &AppHandle,
        snapshot: &TraySnapshot,
        preference_enabled: bool,
        presentation_allowed: bool,
        wall_now: SystemTime,
    ) {
        self.poll_creation(app);
        self.clear_absent_window(app);

        let state = CueReminderState::from_snapshot(
            snapshot,
            self.platform_enabled && preference_enabled,
            presentation_allowed,
        );
        let decision = cue_decision(
            state,
            self.attempted_revision,
            self.slot.as_ref().map(CueSlot::revision),
            self.slot.as_ref().is_some_and(CueSlot::cleanup_pending),
        );
        match decision {
            CueDecision::None => {}
            CueDecision::Close => {
                // A preference or confirmed presentation-state suppression may
                // clear again before this deadline. Rearm this revision only
                // for that case; startup failures still remain one attempt per
                // revision and cannot spin every scheduler poll.
                let rearm = state.in_lead_window() && !state.should_show();
                self.close(app, "reminder state changed");
                if rearm {
                    self.attempted_revision = None;
                }
            }
            CueDecision::Create => {
                self.attempted_revision = Some(state.revision);
                self.controller.begin_production(app);
                let Some(remaining_ms) = state.remaining_ms else {
                    self.controller.end_production();
                    return;
                };
                match cue_deadline_ms(wall_now, remaining_ms)
                    .and_then(|deadline_ms| schedule_cue_creation(app, state.revision, deadline_ms))
                {
                    Ok(slot) => self.slot = Some(slot),
                    Err(error) => {
                        self.controller.end_production();
                        eprintln!("could not create pre-break cue: {error}");
                    }
                }
            }
        }
    }

    pub(crate) fn cancel(&mut self, app: &AppHandle, reason: &str) {
        self.cancel_preview(app);
        self.close(app, reason);
    }

    pub(crate) fn cancel_preview(&self, app: &AppHandle) {
        self.controller.cancel_for_reminder_action(app);
    }

    pub(crate) fn close_scheduled(&mut self, app: &AppHandle, reason: &str) {
        #[cfg(target_os = "macos")]
        if matches!(self.slot, Some(CueSlot::Active { .. })) {
            if let Some(CueSlot::Active { cue, .. }) = self.slot.take() {
                cue.cancelled.store(true, Ordering::Release);
                self.controller.end_production();
                request_cue_handoff(app, cue.label);
                return;
            }
        }
        self.close(app, reason);
    }

    fn poll_creation(&mut self, app: &AppHandle) {
        let result = match self.slot.as_ref() {
            Some(CueSlot::Pending { receiver, .. }) => receiver.try_recv(),
            _ => return,
        };
        match result {
            Ok(result) => {
                let Some(CueSlot::Pending {
                    revision,
                    cancelled,
                    ..
                }) = self.slot.take()
                else {
                    return;
                };
                match result {
                    Ok(Some(cue)) => {
                        self.slot = Some(CueSlot::Active { revision, cue });
                        if cancelled.load(Ordering::Acquire) {
                            self.close(app, "reminder state changed during cue startup");
                        }
                    }
                    Ok(None) => {}
                    Err(error) if !cancelled.load(Ordering::Acquire) => {
                        eprintln!("could not create pre-break cue: {error}");
                    }
                    Err(_) => {}
                }
                if self.slot.is_none() {
                    self.controller.end_production();
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                let cancelled = self.slot.as_ref().is_some_and(|slot| match slot {
                    CueSlot::Pending { cancelled, .. } => cancelled.load(Ordering::Acquire),
                    CueSlot::Active { .. } => false,
                });
                self.slot = None;
                self.controller.end_production();
                if !cancelled {
                    eprintln!("could not create pre-break cue: startup worker stopped");
                }
            }
        }
    }

    fn clear_absent_window(&mut self, app: &AppHandle) {
        let absent = matches!(
            self.slot.as_ref(),
            Some(CueSlot::Active { cue, .. }) if app.get_webview_window(&cue.label).is_none()
        );
        if absent {
            self.clear_absent_active_slot();
        }
    }

    fn clear_absent_active_slot(&mut self) {
        self.slot = None;
        self.controller.end_production();
    }

    fn close(&mut self, app: &AppHandle, reason: &str) {
        let Some(slot) = self.slot.as_ref() else {
            self.controller.end_production();
            return;
        };
        slot.cancel();
        let CueSlot::Active { cue, .. } = slot else {
            return;
        };
        if app.get_webview_window(&cue.label).is_none() {
            self.clear_absent_active_slot();
            return;
        }
        request_cue_window_close(app, cue.label.clone(), reason);
    }
}

fn schedule_cue_creation(
    app: &AppHandle,
    revision: u64,
    deadline_ms: u64,
) -> Result<CueSlot, String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let main_cancelled = Arc::clone(&cancelled);
    let main_app = app.clone();
    let run_id = next_cue_run_id();
    let label = format!("cue-{run_id}-{deadline_ms}");
    app.run_on_main_thread(move || {
        let result = create_cue_window(&main_app, label, main_cancelled);
        let _ = sender.send(result);
    })
    .map_err(|error| format!("could not schedule cue window creation: {error}"))?;
    Ok(CueSlot::Pending {
        revision,
        cancelled,
        receiver,
    })
}

fn create_cue_window(
    app: &AppHandle,
    label: String,
    cancelled: Arc<AtomicBool>,
) -> CueCreationResult {
    if cancelled.load(Ordering::Acquire) {
        return Ok(None);
    }
    let monitor = selected_cue_monitor(app)?;
    let work_area = monitor.work_area();
    let geometry = cue_geometry_for_platform(
        work_area.position.x,
        work_area.position.y,
        work_area.size.width,
        work_area.size.height,
        monitor.scale_factor(),
        cfg!(target_os = "macos"),
    )?;
    if cue_parameters_from_label(&label).is_none() {
        return Err("generated cue label was invalid".into());
    }

    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let window_builder =
        WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
            .initialization_script(cue_platform_initialization_script(cue_frontend_platform()))
            .title("Unfocus eye break reminder")
            .position(geometry.x, geometry.y)
            .inner_size(geometry.width, geometry.height)
            .decorations(false)
            .closable(false)
            .focusable(false)
            .always_on_top(true)
            .visible(false)
            .skip_taskbar(true)
            .shadow(false)
            .transparent(true)
            .background_color(tauri::webview::Color(0, 0, 0, 0));
    // GTK otherwise promotes a non-resizable WebKit window to its 200 px natural height.
    // Equal constraints keep the X11 card fixed, while macOS must remain resizable so a
    // reveal after a display change can clamp to its new work area.
    #[cfg(not(target_os = "macos"))]
    let window_builder = window_builder
        .min_inner_size(geometry.width, geometry.height)
        .max_inner_size(geometry.width, geometry.height);
    #[cfg(target_os = "macos")]
    let window_builder = window_builder
        .visible_on_all_workspaces(pre_break_cue_visible_on_all_workspaces(true))
        // On macOS 14+, keep the short-lived cue's quiet-stage clock and paint
        // callbacks running. Reveal never awaits a hidden-window animation frame.
        .background_throttling(tauri::utils::config::BackgroundThrottlingPolicy::Disabled);
    let window = window_builder
        .on_page_load(move |_, payload| {
            if matches!(payload.event(), PageLoadEvent::Finished) {
                let _ = ready_sender.try_send(());
            }
        })
        .build()
        .map_err(|error| format!("could not build the cue window: {error}"))?;
    #[cfg(target_os = "macos")]
    if let Err(error) = configure_macos_cue_panel(&window) {
        let _ = window.close();
        return Err(error);
    }
    #[cfg(target_os = "macos")]
    if let Err(error) = interaction::start(&window, Arc::clone(&cancelled)) {
        let _ = close_cue_window_on_main(app, &label);
        return Err(error);
    }
    if cancelled.load(Ordering::Acquire) {
        let _ = close_cue_window_on_main(app, &label);
        return Ok(None);
    }

    let ready_app = app.clone();
    let ready_label = label.clone();
    let ready_cancelled = Arc::clone(&cancelled);
    std::thread::Builder::new()
        .name("unfocus-pre-break-cue-ready".into())
        .spawn(
            move || match ready_receiver.recv_timeout(CUE_PAGE_LOAD_TIMEOUT) {
                Ok(()) if !ready_cancelled.load(Ordering::Acquire) => {
                    // The cue page owns native visibility so GTK does not retain
                    // a stale transparent frame during the quiet interval.
                }
                Ok(()) => {
                    request_cue_window_close(&ready_app, ready_label, "cue startup was cancelled");
                }
                Err(error) => {
                    if !ready_cancelled.load(Ordering::Acquire) {
                        eprintln!("pre-break cue did not finish loading: {error}");
                    }
                    request_cue_window_close(&ready_app, ready_label, "cue page did not load");
                }
            },
        )
        .map_err(|error| {
            let _ = close_cue_window_on_main(app, &label);
            format!("could not start the cue readiness worker: {error}")
        })?;

    Ok(Some(CreatedCue { label, cancelled }))
}

#[cfg(test)]
mod tests {
    #[test]
    fn skip_authorization_requires_the_current_uncancelled_production_run() {
        let mut cue = PreBreakCue::new(true, PreBreakCueController::default());
        assert_eq!(cue.skippable_revision(42), None);
        let cancelled = Arc::new(AtomicBool::new(false));
        cue.slot = Some(CueSlot::Active {
            revision: 7,
            cue: CreatedCue {
                label: "cue-42-90000".into(),
                cancelled: Arc::clone(&cancelled),
            },
        });
        assert_eq!(cue.skippable_revision(41), None);
        assert_eq!(cue.skippable_revision(42), Some(7));
        cancelled.store(true, Ordering::Release);
        assert_eq!(cue.skippable_revision(42), None);
    }

    use super::*;

    #[test]
    fn macos_and_qualified_x11_enable_the_cue() {
        assert!(pre_break_cue_platform_enabled(false, true));
        assert!(pre_break_cue_platform_enabled(true, false));
        assert!(!pre_break_cue_platform_enabled(false, false));
    }

    #[test]
    fn only_macos_cues_join_all_workspaces() {
        assert!(pre_break_cue_visible_on_all_workspaces(true));
        assert!(!pre_break_cue_visible_on_all_workspaces(false));
    }

    fn working(remaining_ms: u64, revision: u64) -> CueReminderState {
        CueReminderState {
            working: true,
            remaining_ms: Some(remaining_ms),
            revision,
            enabled: true,
            presentation_allowed: true,
        }
    }

    #[test]
    fn cue_starts_at_exactly_sixty_seconds_once_per_revision() {
        assert_eq!(
            cue_decision(working(60_000, 4), None, None, false),
            CueDecision::Create
        );
        assert_eq!(
            cue_decision(working(60_001, 4), None, None, false),
            CueDecision::None
        );
        assert_eq!(
            cue_decision(working(0, 4), None, None, false),
            CueDecision::None
        );
        assert_eq!(
            cue_decision(working(30_000, 4), Some(4), None, false),
            CueDecision::None
        );
    }

    #[test]
    fn startup_inside_warning_window_creates_a_cue() {
        assert_eq!(
            cue_decision(working(9_500, 0), None, None, false),
            CueDecision::Create
        );
    }

    #[test]
    fn non_working_and_revision_changes_close_the_current_cue() {
        for state in [
            CueReminderState {
                working: false,
                remaining_ms: None,
                revision: 8,
                enabled: true,
                presentation_allowed: true,
            },
            working(59_000, 9),
            working(61_000, 8),
            working(0, 8),
        ] {
            assert_eq!(
                cue_decision(state, Some(8), Some(8), false),
                CueDecision::Close
            );
        }
    }

    #[test]
    fn settings_changes_and_clock_rebases_close_the_old_revision() {
        assert_eq!(
            cue_decision(working(30_000, 21), Some(20), Some(20), false),
            CueDecision::Close
        );
        assert_eq!(
            cue_decision(working(30_000, 22), Some(21), Some(21), false),
            CueDecision::Close
        );
    }

    #[test]
    fn pause_manual_break_and_suspend_past_deadline_close_the_cue() {
        let left_working = CueReminderState {
            working: false,
            remaining_ms: None,
            revision: 31,
            enabled: true,
            presentation_allowed: true,
        };
        for _cause in ["pause", "manual break", "suspend past deadline"] {
            assert_eq!(
                cue_decision(left_working, Some(30), Some(30), false),
                CueDecision::Close
            );
        }
    }

    #[test]
    fn resume_never_reuses_the_previous_working_revision() {
        assert_eq!(
            cue_decision(working(60_000, 41), Some(40), Some(40), false),
            CueDecision::Close
        );
    }

    #[test]
    fn preference_and_presentation_suppression_close_an_active_cue() {
        let mut preference_off = working(30_000, 12);
        preference_off.enabled = false;
        assert_eq!(
            cue_decision(preference_off, Some(12), Some(12), false),
            CueDecision::Close
        );

        let mut presentation_blocked = working(30_000, 12);
        presentation_blocked.presentation_allowed = false;
        assert_eq!(
            cue_decision(presentation_blocked, Some(12), Some(12), false),
            CueDecision::Close
        );
        assert_eq!(
            cue_decision(working(30_000, 12), None, None, false),
            CueDecision::Create,
            "the same timer revision can be rearmed after suppression clears"
        );
    }

    #[test]
    fn failed_creation_is_not_retried_and_cleanup_repeats_until_absent() {
        let state = working(20_000, 12);
        assert_eq!(
            cue_decision(state, Some(12), None, false),
            CueDecision::None
        );
        assert_eq!(
            cue_decision(state, Some(12), Some(11), false),
            CueDecision::Close
        );
        assert_eq!(
            cue_decision(state, Some(12), Some(11), false),
            CueDecision::Close
        );
        assert_eq!(
            cue_decision(state, Some(12), Some(12), true),
            CueDecision::Close
        );
    }

    #[test]
    fn cue_labels_are_strict_and_javascript_safe() {
        assert_eq!(
            cue_parameters_from_label("cue-7-1800000000000"),
            Some((7, 1_800_000_000_000))
        );
        for label in [
            "cue-garbage",
            "cue-01-1800000000000",
            "cue-0-1800000000000",
            "cue-1-0",
            "cue-1-1800000000000-extra",
            "cue-9007199254740992-1800000000000",
            "cue-1-9007199254740992",
        ] {
            assert_eq!(cue_parameters_from_label(label), None, "{label}");
        }
    }

    #[test]
    fn preview_cue_labels_are_strict_and_javascript_safe() {
        assert_eq!(
            cue_parameters_from_label("cue-preview-9-1800000000000"),
            Some((9, 1_800_000_000_000))
        );
        for label in [
            "cue-preview",
            "cue-preview-09-1800000000000",
            "cue-preview-9-0",
            "cue-preview-9-1800000000000-extra",
            "cue-other-9-1800000000000",
        ] {
            assert_eq!(cue_parameters_from_label(label), None, "{label}");
        }
    }

    #[test]
    fn work_area_geometry_handles_scale_reserved_bars_negative_origins_and_narrow_displays() {
        assert_eq!(
            cue_geometry(0, 48, 2_560, 1_392, 2.0).unwrap(),
            CueGeometry {
                x: 540.0,
                y: 36.0,
                width: 200.0,
                height: 36.0,
            }
        );
        assert_eq!(
            cue_geometry(-1_920, 24, 1_920, 1_056, 1.0).unwrap().x,
            -1060.0
        );
        assert_eq!(
            cue_geometry(0, 0, 300, 800, 1.0).unwrap(),
            CueGeometry {
                x: 50.0,
                y: 12.0,
                width: 200.0,
                height: 36.0,
            }
        );
    }

    #[test]
    fn fallback_pill_uses_the_selected_scaled_work_area() {
        assert_eq!(
            cue_geometry(-2_560, 48, 2_560, 1_392, 2.0).unwrap(),
            CueGeometry {
                x: -740.0,
                y: 36.0,
                width: 200.0,
                height: 36.0,
            }
        );
    }

    #[test]
    fn notch_wings_align_to_the_measured_camera_gap_not_the_screen_center() {
        let display = DisplayRect {
            x: 0.0,
            y: 0.0,
            width: 1710.0,
            height: 1112.0,
            primary: true,
        };
        let notch = CueGeometry {
            x: 751.0,
            y: 0.0,
            width: 209.0,
            height: 38.0,
        };
        let (window, layout) = notch_placement(display, notch).unwrap();
        assert_eq!(
            window,
            CueGeometry {
                x: 678.0,
                y: 0.0,
                width: 355.0,
                height: 38.0
            }
        );
        assert_eq!(
            window.x + layout.shoulder_width + layout.wing_width,
            notch.x
        );
        assert_eq!(
            window.width - 2.0 * (layout.wing_width + layout.shoulder_width),
            notch.width
        );
        assert_eq!(layout.wing_width, 64.0);
        assert_eq!(layout.shoulder_width, 9.0);
        assert!(layout.is_notched);
        // A display above and to the left uses the same logical-point layout.
        let shifted = notch_placement(
            DisplayRect {
                x: -1710.0,
                y: -1112.0,
                ..display
            },
            CueGeometry {
                x: notch.x - 1710.0,
                y: -1112.0,
                ..notch
            },
        )
        .unwrap()
        .0;
        assert_eq!(
            shifted,
            CueGeometry {
                x: -1032.0,
                y: -1112.0,
                ..window
            }
        );
    }

    #[test]
    fn invalid_or_unusable_notch_geometry_uses_the_fallback() {
        let display = DisplayRect {
            x: 0.0,
            y: 0.0,
            width: 1710.0,
            height: 1112.0,
            primary: true,
        };
        let valid = CueGeometry {
            x: 751.0,
            y: 0.0,
            width: 209.0,
            height: 38.0,
        };
        for notch in [
            CueGeometry {
                width: 0.0,
                ..valid
            },
            CueGeometry {
                height: 0.0,
                ..valid
            },
            CueGeometry {
                width: f64::NAN,
                ..valid
            },
            CueGeometry { x: 20.0, ..valid },
            // The wing fits, but its curved shoulder would be clipped by the display edge.
            CueGeometry { x: 70.0, ..valid },
            CueGeometry { x: 1430.0, ..valid },
            CueGeometry { x: 1500.0, ..valid },
            CueGeometry { y: 38.0, ..valid },
        ] {
            assert!(notch_placement(display, notch).is_none());
        }
    }

    #[test]
    fn active_display_uses_the_largest_front_window_intersection() {
        let displays = [
            DisplayRect {
                x: -1_440.0,
                y: 0.0,
                width: 1_440.0,
                height: 900.0,
                primary: false,
            },
            DisplayRect {
                x: 0.0,
                y: 0.0,
                width: 1_920.0,
                height: 1_080.0,
                primary: true,
            },
        ];
        let window = DisplayRect {
            x: -300.0,
            y: 80.0,
            width: 800.0,
            height: 600.0,
            primary: false,
        };

        assert_eq!(select_active_display_index(&displays, Some(window)), 1);
    }

    #[test]
    fn active_display_falls_back_to_primary_when_window_data_is_missing_or_stale() {
        let displays = [
            DisplayRect {
                x: -1_440.0,
                y: 0.0,
                width: 1_440.0,
                height: 900.0,
                primary: false,
            },
            DisplayRect {
                x: 0.0,
                y: 0.0,
                width: 1_920.0,
                height: 1_080.0,
                primary: true,
            },
        ];
        let stale_window = DisplayRect {
            x: 4_000.0,
            y: 0.0,
            width: 800.0,
            height: 600.0,
            primary: false,
        };

        assert_eq!(select_active_display_index(&displays, None), 1);
        assert_eq!(
            select_active_display_index(&displays, Some(stale_window)),
            1
        );
    }

    #[test]
    fn fallback_pill_clamps_both_dimensions_for_a_small_work_area() {
        assert_eq!(
            cue_geometry(0, 0, 160, 40, 1.0).unwrap(),
            CueGeometry {
                x: 0.0,
                y: 12.0,
                width: 160.0,
                height: 28.0,
            }
        );
    }

    #[test]
    fn qualified_x11_keeps_the_existing_card_canvas_geometry() {
        assert_eq!(
            cue_geometry_for_platform(0, 48, 2_560, 1_392, 2.0, false).unwrap(),
            CueGeometry {
                x: 412.0,
                y: 72.0,
                width: 456.0,
                height: 160.0,
            }
        );
    }

    #[test]
    fn cue_platform_initialization_matches_the_trusted_frontend_global() {
        assert_eq!(
            cue_platform_initialization_script(CueFrontendPlatform::Macos),
            r#"window.__UNFOCUS_PRE_BREAK_CUE_PLATFORM__ = "macos";"#
        );
        assert_eq!(
            cue_platform_initialization_script(CueFrontendPlatform::Linux),
            r#"window.__UNFOCUS_PRE_BREAK_CUE_PLATFORM__ = "linux";"#
        );
    }

    #[test]
    fn macos_cue_policy_is_nonactivating_status_level_and_click_through() {
        let policy = cue_panel_policy();

        assert_eq!(policy.level, 25);
        assert!(policy.joins_all_spaces);
        assert!(policy.full_screen_auxiliary);
        assert!(policy.nonactivating);
        assert!(!policy.can_become_key_window);
        assert!(!policy.can_become_main_window);
        assert!(policy.ignores_pointer_events);
    }

    #[test]
    fn preview_lifecycle_is_scoped_and_refuses_occupied_surfaces() {
        let mut lifecycle = CueLifecycle::default();

        assert_eq!(
            lifecycle.begin_preview(41, true),
            Err(CueStartError::Overlay)
        );
        assert_eq!(lifecycle.begin_preview(41, false), Ok(()));
        assert_eq!(
            lifecycle.begin_preview(42, false),
            Err(CueStartError::Preview)
        );
        assert_eq!(lifecycle.close_preview(42), Err(CueCloseError::WrongRun));
        assert_eq!(lifecycle.close_preview(41), Ok(()));
    }

    #[test]
    fn production_cue_preempts_preview_and_has_precedence() {
        let mut lifecycle = CueLifecycle::default();
        lifecycle.begin_preview(77, false).unwrap();

        assert_eq!(
            lifecycle.begin_production(),
            ProductionStart::PreemptPreview(77)
        );
        assert_eq!(
            lifecycle.begin_preview(78, false),
            Err(CueStartError::Production)
        );
        lifecycle.end_production();
        assert_eq!(lifecycle.begin_preview(78, false), Ok(()));
    }

    #[test]
    fn preview_start_failure_releases_the_surface_without_timer_state() {
        let controller = PreBreakCueController::default();
        controller
            .reserve_preview_at_boundary(91, || false)
            .unwrap();

        controller.fail_preview(91);

        assert!(controller.reserve_preview_at_boundary(92, || false).is_ok());
    }

    #[test]
    fn preview_commands_are_main_window_only() {
        assert!(authorize_main_caller("main").is_ok());
        assert!(authorize_main_caller("cue-preview-1-1800000000000").is_err());
        assert!(authorize_main_caller("cue-1-1800000000000").is_err());
    }

    #[test]
    fn preview_result_serializes_the_run_and_seventeen_second_bound() {
        assert_eq!(
            serde_json::to_value(PreBreakCuePreview {
                run_id: 12,
                closes_after_ms: PREVIEW_CLOSE_MILLISECONDS,
            })
            .unwrap(),
            serde_json::json!({ "runId": 12, "closesAfterMs": 17_000 })
        );
    }

    #[test]
    fn absent_active_window_cancellation_releases_production_occupancy() {
        let controller = PreBreakCueController::default();
        controller.state().lifecycle.begin_production();
        let mut cue = PreBreakCue::new(true, controller.clone());
        cue.slot = Some(CueSlot::Active {
            revision: 5,
            cue: CreatedCue {
                label: "cue-5-1800000000000".into(),
                cancelled: Arc::new(AtomicBool::new(false)),
            },
        });

        cue.clear_absent_active_slot();

        assert!(controller.reserve_preview_at_boundary(6, || false).is_ok());
    }

    #[test]
    fn preview_reservation_rechecks_overlay_occupancy_after_dispatch() {
        let controller = PreBreakCueController::default();
        let overlay_starting = AtomicBool::new(false);
        let command_time_snapshot = overlay_starting.load(Ordering::Acquire);
        assert!(!command_time_snapshot);
        overlay_starting.store(true, Ordering::Release);

        let result =
            controller.reserve_preview_at_boundary(8, || overlay_starting.load(Ordering::Acquire));

        assert!(matches!(result, Err(CueStartError::Overlay)));
    }
}
