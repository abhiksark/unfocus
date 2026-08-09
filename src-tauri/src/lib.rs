mod diagnostics;
#[cfg(desktop)]
mod instance;
mod overlay;
mod probes;
mod reminder;
mod tray;

use diagnostics::get_diagnostics;
#[cfg(desktop)]
use instance::handle_secondary_launch;
use overlay::{
    close_overlay_test, overlay_run_id_from_label, schedule_automatic_overlay_test,
    show_overlay_test, OverlayController,
};
use probes::ProbeCache;
use reminder::{
    get_reminder_settings, reset_reminder_settings, save_reminder_settings,
    start_scheduler as start_reminder_scheduler, ReminderSettingsManager,
};
use std::io;
use tauri::Manager;
use tray::{install as install_tray, TrayController, TrayStatus};

fn authorize_main_caller(label: &str) -> Result<(), String> {
    if label == "main" {
        Ok(())
    } else {
        Err("this command is only available to the main window".into())
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    let builder = tauri::Builder::default();
    // Instance coordination must be the first plugin so a secondary process
    // exits before setup can load settings or start any Unfocus worker.
    #[cfg(desktop)]
    let builder = builder.plugin(tauri_plugin_single_instance::init(
        |app, _arguments, _working_directory| handle_secondary_launch(app),
    ));

    builder
        .setup(|app| {
            let settings_manager = ReminderSettingsManager::load(&app.path().app_config_dir()?)?;
            let probe_cache = ProbeCache::start()?;
            let overlay_controller = OverlayController::start(app.handle().clone())?;
            let tray_status = TrayStatus::default();
            if !app.manage(settings_manager.clone()) {
                return Err(io::Error::other("reminder settings were already managed").into());
            }
            if !app.manage(probe_cache.clone()) {
                return Err(io::Error::other("probe cache was already managed").into());
            }
            if !app.manage(overlay_controller.clone()) {
                return Err(io::Error::other("overlay controller was already managed").into());
            }
            if !app.manage(tray_status.clone()) {
                return Err(io::Error::other("tray status was already managed").into());
            }
            let tray_controller = install_tray(app, &tray_status)?;
            if !app.manage::<TrayController>(tray_controller) {
                return Err(io::Error::other("tray controller was already managed").into());
            }
            schedule_automatic_overlay_test(app, overlay_controller.clone());
            start_reminder_scheduler(
                app.handle().clone(),
                probe_cache,
                overlay_controller,
                settings_manager,
                tray_status,
            )?;
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
            get_reminder_settings,
            save_reminder_settings,
            reset_reminder_settings,
            show_overlay_test,
            close_overlay_test
        ])
        .run(tauri::generate_context!())
        .expect("error while running Unfocus");
}
