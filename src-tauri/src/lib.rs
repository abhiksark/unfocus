mod diagnostics;
mod overlay;
mod probes;
mod reminder;
mod tray;

use diagnostics::get_diagnostics;
use overlay::{
    close_overlay_test, overlay_run_id_from_label, schedule_automatic_overlay_test,
    show_overlay_test, OverlayController,
};
use probes::ProbeCache;
use reminder::start_scheduler as start_reminder_scheduler;
use std::io;
use tauri::Manager;
use tray::install as install_tray;

fn authorize_main_caller(label: &str) -> Result<(), String> {
    if label == "main" {
        Ok(())
    } else {
        Err("this command is only available to the main window".into())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let probe_cache = ProbeCache::start()?;
            let overlay_controller = OverlayController::start(app.handle().clone())?;
            if !app.manage(probe_cache.clone()) {
                return Err(io::Error::other("probe cache was already managed").into());
            }
            if !app.manage(overlay_controller.clone()) {
                return Err(io::Error::other("overlay controller was already managed").into());
            }
            install_tray(app)?;
            schedule_automatic_overlay_test(app, overlay_controller.clone());
            start_reminder_scheduler(app.handle().clone(), probe_cache, overlay_controller)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            } else if matches!(
                event,
                tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
            ) {
                if let Some(run_id) = overlay_run_id_from_label(window.label()) {
                    window
                        .app_handle()
                        .state::<OverlayController>()
                        .sibling_closed(run_id);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_diagnostics,
            show_overlay_test,
            close_overlay_test
        ])
        .run(tauri::generate_context!())
        .expect("error while running Unfocus");
}
