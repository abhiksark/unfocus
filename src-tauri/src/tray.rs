mod model;

pub(crate) use model::{TrayPhase, TraySnapshot, TrayStatus};

use crate::{
    instance::reveal_dashboard,
    overlay::{show_overlay_if_idle, OverlayController},
    reminder::{ReminderAction, ReminderControl, ReminderStatus},
};
use model::TrayStatusSubscription;
use serde::Serialize;
use std::sync::{Arc, Mutex};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{TrayIcon, TrayIconBuilder},
    Manager,
};

const STATUS_MENU_ID: &str = "unfocus.tray.status";
const PAUSE_MENU_ID: &str = "unfocus.tray.pause";
const TAKE_BREAK_MENU_ID: &str = "unfocus.tray.take-break";
const OPEN_MENU_ID: &str = "unfocus.tray.open";
const PREVIEW_MENU_ID: &str = "unfocus.tray.preview";
const QUIT_MENU_ID: &str = "unfocus.tray.quit";
const PREVIEW_DURATION_SECONDS: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    Pause,
    TakeBreak,
    Open,
    Preview,
    Quit,
}

fn tray_action(menu_id: &str) -> Option<TrayAction> {
    match menu_id {
        PAUSE_MENU_ID => Some(TrayAction::Pause),
        TAKE_BREAK_MENU_ID => Some(TrayAction::TakeBreak),
        OPEN_MENU_ID => Some(TrayAction::Open),
        PREVIEW_MENU_ID => Some(TrayAction::Preview),
        QUIT_MENU_ID => Some(TrayAction::Quit),
        _ => None,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DashboardCloseAction {
    Hide,
    Exit,
}

pub(crate) fn dashboard_close_action(tray_available: bool) -> DashboardCloseAction {
    if tray_available {
        DashboardCloseAction::Hide
    } else {
        DashboardCloseAction::Exit
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TrayDiagnostics {
    available: bool,
    error: Option<String>,
}

#[derive(Debug, Clone)]
struct TrayHealth {
    inner: Arc<Mutex<TrayDiagnostics>>,
}

impl Default for TrayHealth {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(TrayDiagnostics {
                available: false,
                error: None,
            })),
        }
    }
}

impl TrayHealth {
    fn diagnostics(&self) -> TrayDiagnostics {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .clone()
    }

    fn mark_installed(&self) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.available = true;
        state.error = None;
    }

    fn mark_unavailable(&self, error: String) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        state.available = false;
        state.error = Some(error);
    }

    fn record_update_failure(&self, error: String) -> bool {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let first_failure = state.error.is_none();
        state.available = false;
        state.error = Some(error);
        first_failure
    }

    fn record_update_success(&self) -> bool {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let recovered = state.error.take().is_some();
        state.available = true;
        recovered
    }
}

pub(crate) struct TrayController {
    _tray: TrayIcon,
    _status: MenuItem<tauri::Wry>,
    _pause: MenuItem<tauri::Wry>,
    _take_break: MenuItem<tauri::Wry>,
    _preview: MenuItem<tauri::Wry>,
}

pub(crate) struct TrayRuntime {
    controller: Option<TrayController>,
    health: TrayHealth,
}

fn finish_installation<T>(health: &TrayHealth, result: Result<T, String>) -> Result<T, String> {
    result.map_err(|error| {
        let message = format!(
            "Native tray setup failed: {error}. Keep this dashboard open; closing it exits Unfocus. Restore the desktop tray or indicator host, then restart Unfocus."
        );
        health.mark_unavailable(message.clone());
        message
    })
}

impl TrayRuntime {
    pub(crate) fn install(app: &tauri::App, tray_status: &TrayStatus) -> Self {
        let health = TrayHealth::default();
        let installation =
            install_controller(app, tray_status, health.clone()).map_err(|error| error.to_string());
        match finish_installation(&health, installation) {
            Ok(controller) => Self {
                controller: Some(controller),
                health,
            },
            Err(message) => {
                eprintln!("{message}");
                Self {
                    controller: None,
                    health,
                }
            }
        }
    }

    pub(crate) fn can_hide_dashboard(&self) -> bool {
        self.controller.is_some() && self.health.diagnostics().available
    }

    pub(crate) fn diagnostics(&self) -> TrayDiagnostics {
        self.health.diagnostics()
    }
}

trait TrayMenuItems {
    fn update(&self, status: &ReminderStatus) -> Result<(), String>;
}

#[derive(Clone)]
struct MutableTrayMenu {
    status: MenuItem<tauri::Wry>,
    pause: MenuItem<tauri::Wry>,
    take_break: MenuItem<tauri::Wry>,
    preview: MenuItem<tauri::Wry>,
}

impl TrayMenuItems for MutableTrayMenu {
    fn update(&self, reminder: &ReminderStatus) -> Result<(), String> {
        self.status
            .set_text(reminder.tray_status())
            .map_err(|error| format!("could not update status text: {error}"))?;
        self.pause
            .set_text(&reminder.pause_action_label)
            .map_err(|error| format!("could not update pause action text: {error}"))?;
        self.pause
            .set_enabled(reminder.pause_action_enabled)
            .map_err(|error| format!("could not update pause action availability: {error}"))?;
        self.take_break
            .set_enabled(reminder.take_break_enabled)
            .map_err(|error| format!("could not update take-break availability: {error}"))?;
        self.preview
            .set_enabled(reminder.preview_enabled)
            .map_err(|error| format!("could not update preview availability: {error}"))
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TrayUpdateOutcome {
    Updated,
    Recovered,
    FailedFirst(String),
    FailedRepeated,
}

fn apply_menu_update(
    menu: &impl TrayMenuItems,
    reminder: &ReminderStatus,
    health: &TrayHealth,
) -> TrayUpdateOutcome {
    match menu.update(reminder) {
        Ok(()) if health.record_update_success() => TrayUpdateOutcome::Recovered,
        Ok(()) => TrayUpdateOutcome::Updated,
        Err(error) => {
            let message = format!(
                "Tray status updates failed: {error}. Reminder timing is still running; use the dashboard until the tray recovers."
            );
            if health.record_update_failure(message.clone()) {
                TrayUpdateOutcome::FailedFirst(message)
            } else {
                TrayUpdateOutcome::FailedRepeated
            }
        }
    }
}

fn run_status_worker(
    menu: MutableTrayMenu,
    subscription: TrayStatusSubscription,
    health: TrayHealth,
) {
    while subscription.recv().is_ok() {
        let Some(snapshot) = subscription.current() else {
            break;
        };
        let reminder = ReminderStatus::from_snapshot(snapshot);
        match apply_menu_update(&menu, &reminder, &health) {
            TrayUpdateOutcome::FailedFirst(message) => eprintln!("{message}"),
            TrayUpdateOutcome::Recovered => eprintln!("tray reminder status updates recovered"),
            TrayUpdateOutcome::Updated | TrayUpdateOutcome::FailedRepeated => {}
        }
    }
}

fn install_controller(
    app: &tauri::App,
    tray_status: &TrayStatus,
    health: TrayHealth,
) -> tauri::Result<TrayController> {
    let initial = ReminderStatus::from_snapshot(tray_status.current());
    let status = MenuItem::with_id(
        app,
        STATUS_MENU_ID,
        initial.tray_status(),
        false,
        None::<&str>,
    )?;
    let controls_separator = PredefinedMenuItem::separator(app)?;
    let pause = MenuItem::with_id(
        app,
        PAUSE_MENU_ID,
        initial.pause_action_label,
        initial.pause_action_enabled,
        None::<&str>,
    )?;
    let take_break = MenuItem::with_id(
        app,
        TAKE_BREAK_MENU_ID,
        "Take a break now",
        initial.take_break_enabled,
        None::<&str>,
    )?;
    let open = MenuItem::with_id(app, OPEN_MENU_ID, "Open Unfocus", true, None::<&str>)?;
    let preview = MenuItem::with_id(
        app,
        PREVIEW_MENU_ID,
        format!("Preview break ({PREVIEW_DURATION_SECONDS} seconds)"),
        initial.preview_enabled,
        None::<&str>,
    )?;
    let quit_separator = PredefinedMenuItem::separator(app)?;
    let quit = MenuItem::with_id(app, QUIT_MENU_ID, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(
        app,
        &[
            &status,
            &controls_separator,
            &pause,
            &take_break,
            &open,
            &preview,
            &quit_separator,
            &quit,
        ],
    )?;

    // macOS recolours a template image to match the menubar theme; every
    // other platform gets a fixed light glyph for their (dark) panels.
    #[cfg(target_os = "macos")]
    const TRAY_ICON: &[u8] = include_bytes!("../icons/tray/tray-template.png");
    #[cfg(not(target_os = "macos"))]
    const TRAY_ICON: &[u8] = include_bytes!("../icons/tray/tray-light.png");

    let icon = Image::from_bytes(TRAY_ICON)?;

    let builder = TrayIconBuilder::new()
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .menu(&menu);

    // The locked Linux tray backend explicitly does not support tooltips or
    // programmable left-click menu behavior. Required information therefore
    // lives in the menu, and only platforms that implement these options get
    // them configured.
    #[cfg(not(target_os = "linux"))]
    let builder = builder
        .tooltip("Unfocus eye-break reminder")
        .show_menu_on_left_click(false);

    let tray = builder
        .on_menu_event(|app, event| match tray_action(event.id.as_ref()) {
            Some(TrayAction::Pause) => {
                let control = app.state::<ReminderControl>();
                let status = app.state::<TrayStatus>();
                let action = if status.current().phase == TrayPhase::Paused {
                    ReminderAction::Resume
                } else {
                    ReminderAction::Pause
                };
                if let Err(error) = control.dispatch(action) {
                    eprintln!("tray pause action failed: {error}");
                }
            }
            Some(TrayAction::TakeBreak) => {
                let control = app.state::<ReminderControl>();
                if let Err(error) = control.dispatch(ReminderAction::TakeBreakNow) {
                    eprintln!("tray take-break action failed: {error}");
                }
            }
            Some(TrayAction::Open) => reveal_dashboard(app),
            Some(TrayAction::Preview) => {
                let app = app.clone();
                let controller = app.state::<OverlayController>().inner().clone();
                tauri::async_runtime::spawn_blocking(move || {
                    if let Err(error) =
                        show_overlay_if_idle(&app, &controller, PREVIEW_DURATION_SECONDS)
                    {
                        eprintln!("overlay preview failed: {error}");
                    }
                });
            }
            Some(TrayAction::Quit) => app.exit(0),
            None => {}
        })
        .build(app)?;
    health.mark_installed();

    let subscription = tray_status.subscribe();
    let worker_menu = MutableTrayMenu {
        status: status.clone(),
        pause: pause.clone(),
        take_break: take_break.clone(),
        preview: preview.clone(),
    };
    let worker_health = health.clone();
    std::thread::Builder::new()
        .name("unfocus-tray-status".into())
        .spawn(move || run_status_worker(worker_menu, subscription, worker_health))?;

    Ok(TrayController {
        _tray: tray,
        _status: status,
        _pause: pause,
        _take_break: take_break,
        _preview: preview,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{cell::RefCell, collections::VecDeque};

    const TRAY_TEMPLATE_PNG: &[u8] = include_bytes!("../icons/tray/tray-template.png");
    const TRAY_LIGHT_PNG: &[u8] = include_bytes!("../icons/tray/tray-light.png");

    fn assert_tray_asset(bytes: &[u8], name: &str, channel_ok: impl Fn(u8) -> bool) {
        let image = tauri::image::Image::from_bytes(bytes)
            .unwrap_or_else(|error| panic!("{name} did not decode: {error}"));
        assert_eq!(
            (image.width(), image.height()),
            (32, 32),
            "{name} must be 32x32"
        );
        let rgba = image.rgba();
        assert_eq!(
            rgba.len(),
            32 * 32 * 4,
            "{name} has a truncated pixel buffer"
        );
        let mut visible = 0_usize;
        for pixel in rgba.chunks_exact(4) {
            if pixel[3] == 0 {
                continue;
            }
            visible += 1;
            assert!(
                pixel[0] == pixel[1] && pixel[1] == pixel[2] && channel_ok(pixel[0]),
                "{name} contains a non-monochrome pixel {pixel:?}"
            );
        }
        assert!(visible > 0, "{name} is fully transparent");
    }

    #[test]
    fn tray_template_asset_is_black_on_alpha() {
        assert_tray_asset(TRAY_TEMPLATE_PNG, "tray-template.png", |channel| {
            channel == 0
        });
    }

    #[test]
    fn tray_light_asset_is_white_on_alpha() {
        // The rasterizer's un-premultiply rounding can land a hair under 255
        // on anti-aliased edges; the template contract only needs "white".
        assert_tray_asset(TRAY_LIGHT_PNG, "tray-light.png", |channel| channel >= 250);
    }

    #[test]
    fn tray_ids_only_dispatch_known_unfocus_actions() {
        assert_eq!(tray_action(PAUSE_MENU_ID), Some(TrayAction::Pause));
        assert_eq!(tray_action(TAKE_BREAK_MENU_ID), Some(TrayAction::TakeBreak));
        assert_eq!(tray_action(OPEN_MENU_ID), Some(TrayAction::Open));
        assert_eq!(tray_action(PREVIEW_MENU_ID), Some(TrayAction::Preview));
        assert_eq!(tray_action(QUIT_MENU_ID), Some(TrayAction::Quit));
        assert_eq!(tray_action(STATUS_MENU_ID), None);
        assert_eq!(tray_action("open"), None);
        assert_eq!(tray_action("another-app.open"), None);
    }

    #[test]
    fn dashboard_only_hides_when_a_tray_was_constructed() {
        assert_eq!(dashboard_close_action(true), DashboardCloseAction::Hide);
        assert_eq!(dashboard_close_action(false), DashboardCloseAction::Exit);
    }

    #[derive(Default)]
    struct FakeMenu {
        results: RefCell<VecDeque<Result<(), String>>>,
        visible_text: RefCell<String>,
    }

    impl FakeMenu {
        fn with_results(results: impl IntoIterator<Item = Result<(), String>>) -> Self {
            Self {
                results: RefCell::new(results.into_iter().collect()),
                visible_text: RefCell::new("Working · break in 20 min".into()),
            }
        }
    }

    impl TrayMenuItems for FakeMenu {
        fn update(&self, reminder: &ReminderStatus) -> Result<(), String> {
            let result = self.results.borrow_mut().pop_front().unwrap_or(Ok(()));
            if result.is_ok() {
                *self.visible_text.borrow_mut() = reminder.tray_status().into();
            }
            result
        }
    }

    fn working_status(minutes: u64) -> ReminderStatus {
        ReminderStatus::from_snapshot(TraySnapshot::timer(
            TrayPhase::Working,
            std::time::Duration::from_secs(minutes * 60),
            false,
            0,
            0,
        ))
    }

    #[test]
    fn update_failures_preserve_text_and_report_once_until_recovery() {
        let health = TrayHealth::default();
        health.mark_installed();
        let menu = FakeMenu::with_results([
            Err("panel disconnected".into()),
            Err("panel still disconnected".into()),
            Ok(()),
            Err("panel disconnected again".into()),
        ]);

        assert!(matches!(
            apply_menu_update(&menu, &working_status(19), &health),
            TrayUpdateOutcome::FailedFirst(_)
        ));
        assert_eq!(&*menu.visible_text.borrow(), "Working · break in 20 min");
        let diagnostics = health.diagnostics();
        assert!(!diagnostics.available);
        assert!(diagnostics.error.is_some());

        assert_eq!(
            apply_menu_update(&menu, &working_status(18), &health),
            TrayUpdateOutcome::FailedRepeated
        );
        assert_eq!(&*menu.visible_text.borrow(), "Working · break in 20 min");

        assert_eq!(
            apply_menu_update(&menu, &working_status(17), &health),
            TrayUpdateOutcome::Recovered
        );
        assert_eq!(&*menu.visible_text.borrow(), "Working · break in 17 min");
        let diagnostics = health.diagnostics();
        assert!(diagnostics.available);
        assert_eq!(diagnostics.error, None);

        assert!(matches!(
            apply_menu_update(&menu, &working_status(16), &health),
            TrayUpdateOutcome::FailedFirst(_)
        ));
        assert!(!health.diagnostics().available);
    }

    #[test]
    fn construction_failure_is_explicitly_unavailable() {
        let health = TrayHealth::default();
        let result = finish_installation::<()>(&health, Err("indicator host unavailable".into()));

        let error = result.unwrap_err();
        assert!(error.contains("indicator host unavailable"));
        assert!(error.contains("Keep this dashboard open"));
        let diagnostics = health.diagnostics();
        assert!(!diagnostics.available);
        assert_eq!(diagnostics.error, Some(error));
    }
}
