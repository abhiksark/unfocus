use crate::{
    overlay::{show_overlay, OverlayController},
    probes::{ProbeCache, ProbeSnapshot},
};
use std::{
    io,
    time::{Duration, Instant},
};
use tauri::AppHandle;

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

pub(crate) fn start_scheduler(
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
}
