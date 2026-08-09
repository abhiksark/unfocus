use crate::overlay::{show_overlay, OverlayController};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    Manager,
};

pub(crate) fn install(app: &tauri::App) -> tauri::Result<()> {
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

    // macOS recolours a template image to match the menubar theme; every
    // other platform gets a fixed light glyph for their (dark) panels.
    #[cfg(target_os = "macos")]
    const TRAY_ICON: &[u8] = include_bytes!("../icons/tray/tray-template.png");
    #[cfg(not(target_os = "macos"))]
    const TRAY_ICON: &[u8] = include_bytes!("../icons/tray/tray-light.png");

    let icon = Image::from_bytes(TRAY_ICON)?;

    TrayIconBuilder::new()
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Unfocus eye-break reminder")
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
                let controller = app.state::<OverlayController>();
                if let Err(error) = show_overlay(app, &controller, 8) {
                    eprintln!("overlay test failed: {error}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

#[cfg(test)]
mod tests {
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
}
