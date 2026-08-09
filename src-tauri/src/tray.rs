mod model;

pub(crate) use model::{TrayPhase, TraySnapshot, TrayStatus};

use crate::{
    instance::reveal_dashboard,
    overlay::{show_overlay, OverlayController},
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
const OPEN_MENU_ID: &str = "unfocus.tray.open";
const PREVIEW_MENU_ID: &str = "unfocus.tray.preview";
const QUIT_MENU_ID: &str = "unfocus.tray.quit";
const PREVIEW_DURATION_SECONDS: u64 = 8;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TrayAction {
    Open,
    Preview,
    Quit,
}

fn tray_action(menu_id: &str) -> Option<TrayAction> {
    match menu_id {
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

trait TrayStatusItem {
    fn update_text(&self, text: &str) -> Result<(), String>;
}

impl TrayStatusItem for MenuItem<tauri::Wry> {
    fn update_text(&self, text: &str) -> Result<(), String> {
        self.set_text(text).map_err(|error| error.to_string())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum TrayUpdateOutcome {
    Updated,
    Recovered,
    FailedFirst(String),
    FailedRepeated,
}

fn apply_status_update(
    status: &impl TrayStatusItem,
    text: &str,
    health: &TrayHealth,
) -> TrayUpdateOutcome {
    match status.update_text(text) {
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
    status: MenuItem<tauri::Wry>,
    subscription: TrayStatusSubscription,
    health: TrayHealth,
) {
    while subscription.recv().is_ok() {
        let Some(snapshot) = subscription.current() else {
            break;
        };
        let text = snapshot.presentation().status;
        match apply_status_update(&status, &text, &health) {
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
    let initial_status = tray_status.current().presentation().status;
    let status = MenuItem::with_id(app, STATUS_MENU_ID, initial_status, false, None::<&str>)?;
    let separator = PredefinedMenuItem::separator(app)?;
    let open = MenuItem::with_id(app, OPEN_MENU_ID, "Open Unfocus", true, None::<&str>)?;
    let preview = MenuItem::with_id(
        app,
        PREVIEW_MENU_ID,
        format!("Preview break ({PREVIEW_DURATION_SECONDS} seconds)"),
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, QUIT_MENU_ID, "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&status, &separator, &open, &preview, &quit])?;

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
            Some(TrayAction::Open) => reveal_dashboard(app),
            Some(TrayAction::Preview) => {
                let controller = app.state::<OverlayController>();
                if let Err(error) = show_overlay(app, &controller, PREVIEW_DURATION_SECONDS) {
                    eprintln!("overlay preview failed: {error}");
                }
            }
            Some(TrayAction::Quit) => app.exit(0),
            None => {}
        })
        .build(app)?;
    health.mark_installed();

    let subscription = tray_status.subscribe();
    let worker_status = status.clone();
    let worker_health = health.clone();
    std::thread::Builder::new()
        .name("unfocus-tray-status".into())
        .spawn(move || run_status_worker(worker_status, subscription, worker_health))?;

    Ok(TrayController {
        _tray: tray,
        _status: status,
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
    struct FakeStatusItem {
        results: RefCell<VecDeque<Result<(), String>>>,
        visible_text: RefCell<String>,
    }

    impl FakeStatusItem {
        fn with_results(results: impl IntoIterator<Item = Result<(), String>>) -> Self {
            Self {
                results: RefCell::new(results.into_iter().collect()),
                visible_text: RefCell::new("Working · break in 20 min".into()),
            }
        }
    }

    impl TrayStatusItem for FakeStatusItem {
        fn update_text(&self, text: &str) -> Result<(), String> {
            let result = self.results.borrow_mut().pop_front().unwrap_or(Ok(()));
            if result.is_ok() {
                *self.visible_text.borrow_mut() = text.into();
            }
            result
        }
    }

    #[test]
    fn update_failures_preserve_text_and_report_once_until_recovery() {
        let health = TrayHealth::default();
        health.mark_installed();
        let item = FakeStatusItem::with_results([
            Err("panel disconnected".into()),
            Err("panel still disconnected".into()),
            Ok(()),
            Err("panel disconnected again".into()),
        ]);

        assert!(matches!(
            apply_status_update(&item, "Working · break in 19 min", &health),
            TrayUpdateOutcome::FailedFirst(_)
        ));
        assert_eq!(&*item.visible_text.borrow(), "Working · break in 20 min");
        let diagnostics = health.diagnostics();
        assert!(!diagnostics.available);
        assert!(diagnostics.error.is_some());

        assert_eq!(
            apply_status_update(&item, "Working · break in 18 min", &health),
            TrayUpdateOutcome::FailedRepeated
        );
        assert_eq!(&*item.visible_text.borrow(), "Working · break in 20 min");

        assert_eq!(
            apply_status_update(&item, "Working · break in 17 min", &health),
            TrayUpdateOutcome::Recovered
        );
        assert_eq!(&*item.visible_text.borrow(), "Working · break in 17 min");
        let diagnostics = health.diagnostics();
        assert!(diagnostics.available);
        assert_eq!(diagnostics.error, None);

        assert!(matches!(
            apply_status_update(&item, "Working · break in 16 min", &health),
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
