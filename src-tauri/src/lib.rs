mod activity;
mod diagnostics;
#[cfg(desktop)]
mod instance;
/// Pure lifecycle/a11y product contract for issue #30 (compiled with unit tests).
#[cfg(test)]
mod lifecycle_contract;
mod overlay;
mod probes;
mod reminder;
mod tray;

use activity::{get_today_activity, ActivityTrackerHandle};
use diagnostics::get_diagnostics;
#[cfg(desktop)]
use instance::handle_secondary_launch;
#[cfg(debug_assertions)]
use overlay::schedule_automatic_overlay_test;
use overlay::{
    close_overlay_test, overlay_run_id_from_label, show_overlay_test, OverlayController,
};
use probes::ProbeCache;
use reminder::{
    get_reminder_settings, get_reminder_status, pause_reminders, reset_reminder_settings,
    resume_reminders, save_reminder_settings, start_scheduler as start_reminder_scheduler,
    take_break_now, ReminderSettingsManager,
};
use std::io;
use tauri::Manager;
use tray::{dashboard_close_action, DashboardCloseAction, TrayRuntime, TrayStatus};

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
            let activity_tracker = ActivityTrackerHandle::default();
            let overlay_controller = OverlayController::start(app.handle().clone())?;
            let tray_status = TrayStatus::default();
            if !app.manage(settings_manager.clone()) {
                return Err(io::Error::other("reminder settings were already managed").into());
            }
            if !app.manage(probe_cache.clone()) {
                return Err(io::Error::other("probe cache was already managed").into());
            }
            if !app.manage(activity_tracker.clone()) {
                return Err(io::Error::other("activity tracker was already managed").into());
            }
            if !app.manage(overlay_controller.clone()) {
                return Err(io::Error::other("overlay controller was already managed").into());
            }
            if !app.manage(tray_status.clone()) {
                return Err(io::Error::other("tray status was already managed").into());
            }
            let reminder_control = start_reminder_scheduler(
                app.handle().clone(),
                probe_cache,
                activity_tracker,
                overlay_controller.clone(),
                settings_manager,
                tray_status.clone(),
            )?;
            if !app.manage(reminder_control) {
                return Err(io::Error::other("reminder control was already managed").into());
            }
            let tray_runtime = TrayRuntime::install(app, &tray_status);
            if !app.manage(tray_runtime) {
                return Err(io::Error::other("tray runtime was already managed").into());
            }
            #[cfg(debug_assertions)]
            schedule_automatic_overlay_test(app, overlay_controller.clone());
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let tray_available = window
                        .app_handle()
                        .try_state::<TrayRuntime>()
                        .is_some_and(|runtime| runtime.can_hide_dashboard());
                    match dashboard_close_action(tray_available) {
                        DashboardCloseAction::Hide => {
                            if let Err(error) = window.hide() {
                                eprintln!("could not hide the dashboard into the tray: {error}");
                            }
                        }
                        DashboardCloseAction::Exit => window.app_handle().exit(0),
                    }
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
            get_today_activity,
            get_reminder_settings,
            get_reminder_status,
            save_reminder_settings,
            reset_reminder_settings,
            pause_reminders,
            resume_reminders,
            take_break_now,
            show_overlay_test,
            close_overlay_test
        ])
        .run(tauri::generate_context!())
        .expect("error while running Unfocus");
}
