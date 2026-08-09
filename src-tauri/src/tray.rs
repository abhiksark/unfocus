mod model;

pub(crate) use model::{TrayPhase, TraySnapshot, TrayStatus};

use crate::{
    instance::reveal_dashboard,
    overlay::{show_overlay, OverlayController},
};
use model::TrayStatusSubscription;
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

pub(crate) struct TrayController {
    _tray: TrayIcon,
    _status: MenuItem<tauri::Wry>,
}

fn run_status_worker(status: MenuItem<tauri::Wry>, subscription: TrayStatusSubscription) {
    let mut failure_reported = false;
    while subscription.recv().is_ok() {
        let Some(snapshot) = subscription.current() else {
            break;
        };
        let text = snapshot.presentation().status;
        match status.set_text(text) {
            Ok(()) => failure_reported = false,
            Err(error) if !failure_reported => {
                eprintln!("could not update tray reminder status: {error}");
                failure_reported = true;
            }
            Err(_) => {}
        }
    }
}

pub(crate) fn install(app: &tauri::App, tray_status: &TrayStatus) -> tauri::Result<TrayController> {
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

    let tray = TrayIconBuilder::new()
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Unfocus eye-break reminder")
        .menu(&menu)
        .show_menu_on_left_click(false)
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

    let subscription = tray_status.subscribe();
    let worker_status = status.clone();
    std::thread::Builder::new()
        .name("unfocus-tray-status".into())
        .spawn(move || run_status_worker(worker_status, subscription))?;

    Ok(TrayController {
        _tray: tray,
        _status: status,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
