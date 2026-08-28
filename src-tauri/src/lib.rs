// src-tauri/src/lib.rs

mod activity;
mod activity_archive;
mod break_ledger;
mod diagnostics;
#[cfg(desktop)]
mod instance;
/// Pure lifecycle/a11y product contract for issue #30 (compiled with unit tests).
#[cfg(test)]
mod lifecycle_contract;
mod overlay;
mod pre_break_cue;
mod probes;
mod reminder;
mod tray;

use activity::{get_activity_range, get_today_activity, ActivityTrackerHandle};
use break_ledger::{get_break_range, get_break_summary, BreakLedgerHandle};
use diagnostics::get_diagnostics;
#[cfg(desktop)]
use instance::handle_secondary_launch;
#[cfg(debug_assertions)]
use overlay::schedule_automatic_overlay_test;
use overlay::{
    close_overlay_test, overlay_run_id_from_label, show_overlay_test, OverlayCloseEvent,
    OverlayController,
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

/// The author's site, shown as attribution on the dashboard.
const AUTHOR_WEBSITE: &str = "https://abhik.ai";

/// Hands the author's site to the desktop's default browser.
///
/// The address is a constant rather than a parameter, so the dashboard cannot
/// ask the host to open anything else. Unfocus itself still makes no network
/// call; the browser does, and only when the reader asks for it. A failure is
/// reported to the caller, never panics, and never touches the reminder timer.
///
/// The launcher runs off the async runtime so a cold browser start cannot
/// block the command handler.
#[tauri::command]
async fn open_author_website(window: tauri::WebviewWindow) -> Result<(), String> {
    authorize_main_caller(window.label())?;

    #[cfg(target_os = "linux")]
    let mut launcher = {
        let mut launcher = std::process::Command::new("xdg-open");
        launcher.arg(AUTHOR_WEBSITE);
        launcher
    };
    #[cfg(target_os = "macos")]
    let mut launcher = {
        let mut launcher = std::process::Command::new("open");
        launcher.arg(AUTHOR_WEBSITE);
        launcher
    };
    // `start` reads its first quoted argument as the window title, so the empty
    // string is required before the address.
    #[cfg(target_os = "windows")]
    let mut launcher = {
        let mut launcher = std::process::Command::new("cmd");
        launcher.args(["/C", "start", "", AUTHOR_WEBSITE]);
        launcher
    };

    #[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
    {
        let status = tauri::async_runtime::spawn_blocking(move || launcher.status())
            .await
            .map_err(|error| format!("could not open {AUTHOR_WEBSITE}: {error}"))?;
        browser_launch_result(status)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    Err(format!(
        "opening the author website is unsupported on {}",
        std::env::consts::OS
    ))
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn browser_launch_result(status: io::Result<std::process::ExitStatus>) -> Result<(), String> {
    match status {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!(
            "could not open {AUTHOR_WEBSITE}: launcher returned {status}"
        )),
        Err(error) => Err(format!("could not open {AUTHOR_WEBSITE}: {error}")),
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
    #[cfg(target_os = "macos")]
    let builder = builder.plugin(tauri_nspanel::init());

    builder
        .setup(|app| {
            #[cfg(target_os = "macos")]
            app.set_activation_policy(tauri::ActivationPolicy::Accessory);

            let config_dir = app.path().app_config_dir()?;
            let settings_manager = ReminderSettingsManager::load(&config_dir)?;
            let probe_cache = ProbeCache::start()?;
            let activity_tracker =
                ActivityTrackerHandle::load(&config_dir).unwrap_or_else(|error| {
                    eprintln!("could not load activity history: {error}; starting empty");
                    ActivityTrackerHandle::default()
                });
            let break_ledger = BreakLedgerHandle::load(&config_dir).unwrap_or_else(|error| {
                eprintln!("could not load break event ledger: {error}; starting empty");
                BreakLedgerHandle::default()
            });
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
            if !app.manage(break_ledger.clone()) {
                return Err(io::Error::other("break event ledger was already managed").into());
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
                break_ledger,
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
            } else if let Some(run_id) = overlay_run_id_from_label(window.label()) {
                let close_event = match event {
                    tauri::WindowEvent::CloseRequested { .. } => Some(OverlayCloseEvent::Requested),
                    tauri::WindowEvent::Destroyed => Some(OverlayCloseEvent::Destroyed),
                    _ => None,
                };
                if let Some(close_event) = close_event {
                    let prevent_close = window
                        .app_handle()
                        .state::<OverlayController>()
                        .sibling_closed(run_id, window.label().to_owned(), close_event);
                    if prevent_close {
                        if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                            api.prevent_close();
                        }
                    }
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_diagnostics,
            get_today_activity,
            get_activity_range,
            get_break_range,
            get_break_summary,
            get_reminder_settings,
            get_reminder_status,
            save_reminder_settings,
            reset_reminder_settings,
            pause_reminders,
            resume_reminders,
            take_break_now,
            show_overlay_test,
            close_overlay_test,
            open_author_website
        ])
        .run(tauri::generate_context!())
        .expect("error while running Unfocus");
}

#[cfg(test)]
mod tests {
    use super::browser_launch_result;
    use std::process::Command;

    #[test]
    fn browser_launcher_nonzero_exit_is_an_error() {
        #[cfg(unix)]
        let status = Command::new("sh").args(["-c", "exit 1"]).status().unwrap();
        #[cfg(windows)]
        let status = Command::new("cmd").args(["/C", "exit 1"]).status().unwrap();

        assert!(browser_launch_result(Ok(status)).is_err());
    }
}
