mod diagnostics;
mod overlay;
mod probes;

use diagnostics::get_diagnostics;
use overlay::{
    close_overlay_test, overlay_run_id_from_label, schedule_automatic_overlay_test, show_overlay,
    show_overlay_test, OverlayController,
};
use probes::{ProbeCache, ProbeSnapshot};
use std::{
    io,
    time::{Duration, Instant},
};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Manager,
};

const WORK_INTERVAL: Duration = Duration::from_secs(20 * 60);
const BREAK_DURATION: Duration = Duration::from_secs(20);
const REMINDER_POLL_INTERVAL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReminderPhase {
    Working,
    Break,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReminderTransition {
    StartBreak,
    EndBreak,
}

/// The recurring reminder clock. Its only input is monotonic elapsed time;
/// probe results deliberately do not participate in phase advancement.
#[derive(Debug)]
struct ReminderTimer {
    phase: ReminderPhase,
    phase_started_at: Duration,
    work_interval: Duration,
    break_duration: Duration,
}

impl ReminderTimer {
    fn new(now: Duration, work_interval: Duration, break_duration: Duration) -> Self {
        Self {
            phase: ReminderPhase::Working,
            phase_started_at: now,
            work_interval,
            break_duration,
        }
    }

    fn with_defaults(now: Duration) -> Self {
        Self::new(now, WORK_INTERVAL, BREAK_DURATION)
    }

    fn tick(&mut self, now: Duration) -> Option<ReminderTransition> {
        let elapsed = now.saturating_sub(self.phase_started_at);
        let phase_duration = match self.phase {
            ReminderPhase::Working => self.work_interval,
            ReminderPhase::Break => self.break_duration,
        };

        if elapsed < phase_duration {
            return None;
        }

        // Anchor the next phase at this observation rather than replaying every
        // missed cycle after a long scheduler stall.
        self.phase_started_at = now;
        Some(match self.phase {
            ReminderPhase::Working => {
                self.phase = ReminderPhase::Break;
                ReminderTransition::StartBreak
            }
            ReminderPhase::Break => {
                self.phase = ReminderPhase::Working;
                ReminderTransition::EndBreak
            }
        })
    }
}

/// Probe data can suppress presentation of a due break, but it cannot mutate
/// the timer. Errors fail open so an unavailable probe never disables breaks.
fn should_present_break(probes: &ProbeSnapshot, break_duration: Duration) -> bool {
    let user_is_already_resting = probes
        .idle_seconds
        .as_ref()
        .is_ok_and(|seconds| *seconds >= break_duration.as_secs());
    let presentation_is_active = probes
        .active_window_fullscreen
        .as_ref()
        .is_ok_and(|fullscreen| *fullscreen);

    !user_is_already_resting && !presentation_is_active
}

fn install_tray(app: &tauri::App) -> tauri::Result<()> {
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

fn start_reminder_scheduler(
    app: AppHandle,
    probe_cache: ProbeCache,
    overlay_controller: OverlayController,
) -> io::Result<()> {
    std::thread::Builder::new()
        .name("unfocus-reminders".into())
        .spawn(move || {
            let started_at = Instant::now();
            let mut timer = ReminderTimer::with_defaults(Duration::ZERO);

            loop {
                std::thread::sleep(REMINDER_POLL_INTERVAL);
                if timer.tick(started_at.elapsed()) != Some(ReminderTransition::StartBreak) {
                    continue;
                }

                let probes = probe_cache.snapshot();
                if !should_present_break(&probes, BREAK_DURATION) {
                    if probes
                        .idle_seconds
                        .as_ref()
                        .is_ok_and(|seconds| *seconds >= BREAK_DURATION.as_secs())
                    {
                        eprintln!("scheduled break stayed hidden because the user is already idle");
                    } else {
                        eprintln!("scheduled break stayed hidden while fullscreen is active");
                    }
                    continue;
                }

                if let Err(error) =
                    show_overlay(&app, &overlay_controller, BREAK_DURATION.as_secs())
                {
                    eprintln!("could not present scheduled break: {error}");
                }
            }
        })?;
    Ok(())
}

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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reminder_defaults_are_twenty_minutes_and_twenty_seconds() {
        let mut timer = ReminderTimer::with_defaults(Duration::ZERO);

        assert_eq!(timer.tick(WORK_INTERVAL - Duration::from_millis(1)), None);
        assert_eq!(
            timer.tick(WORK_INTERVAL),
            Some(ReminderTransition::StartBreak)
        );
        assert_eq!(
            timer.tick(WORK_INTERVAL + BREAK_DURATION),
            Some(ReminderTransition::EndBreak)
        );
        assert_eq!(
            timer.tick(WORK_INTERVAL + BREAK_DURATION + WORK_INTERVAL),
            Some(ReminderTransition::StartBreak)
        );
    }

    #[test]
    fn reminder_clock_is_injected_and_does_not_replay_missed_cycles() {
        let mut timer = ReminderTimer::new(
            Duration::from_secs(10),
            Duration::from_secs(60),
            Duration::from_secs(5),
        );

        assert_eq!(timer.tick(Duration::from_secs(69)), None);
        assert_eq!(
            timer.tick(Duration::from_secs(600)),
            Some(ReminderTransition::StartBreak)
        );
        assert_eq!(timer.tick(Duration::from_secs(600)), None);
        assert_eq!(
            timer.tick(Duration::from_secs(605)),
            Some(ReminderTransition::EndBreak)
        );
    }

    #[test]
    fn a_clock_regression_does_not_advance_the_reminder() {
        let mut timer = ReminderTimer::new(
            Duration::from_secs(100),
            Duration::from_secs(60),
            Duration::from_secs(5),
        );

        assert_eq!(timer.tick(Duration::from_secs(90)), None);
        assert_eq!(timer.phase, ReminderPhase::Working);
    }

    #[test]
    fn probes_only_control_break_presentation() {
        let active = ProbeSnapshot {
            idle_seconds: Ok(0),
            active_window_fullscreen: Ok(false),
        };
        let idle = ProbeSnapshot {
            idle_seconds: Ok(BREAK_DURATION.as_secs()),
            active_window_fullscreen: Ok(false),
        };
        let fullscreen = ProbeSnapshot {
            idle_seconds: Ok(0),
            active_window_fullscreen: Ok(true),
        };
        let failed = ProbeSnapshot {
            idle_seconds: Err("idle failed".into()),
            active_window_fullscreen: Err("fullscreen failed".into()),
        };

        assert!(should_present_break(&active, BREAK_DURATION));
        assert!(!should_present_break(&idle, BREAK_DURATION));
        assert!(!should_present_break(&fullscreen, BREAK_DURATION));
        assert!(should_present_break(&failed, BREAK_DURATION));

        // Timer advancement has no probe input and is identical whether the
        // presentation decision above succeeds, suppresses, or errors.
        for probes in [&active, &idle, &fullscreen, &failed] {
            let mut timer = ReminderTimer::new(
                Duration::ZERO,
                Duration::from_secs(1),
                Duration::from_secs(1),
            );
            let _ = should_present_break(probes, Duration::from_secs(1));
            assert_eq!(
                timer.tick(Duration::from_secs(1)),
                Some(ReminderTransition::StartBreak)
            );
            assert_eq!(
                timer.tick(Duration::from_secs(2)),
                Some(ReminderTransition::EndBreak)
            );
        }
    }

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
