use std::time::{Duration, Instant};

pub(super) const OVERLAY_TICK_INTERVAL: Duration = Duration::from_millis(250);
pub(super) const OVERLAY_DISMISS_DELAY: Duration = Duration::from_millis(500);
pub(super) const OVERLAY_COMPLETION_GRACE: Duration = Duration::from_millis(1_250);
pub(super) const OVERLAY_CLOSE_FAILURE_LIMIT: usize = 3;

pub(super) fn overlay_close_retry_delay(failures: usize) -> Duration {
    OVERLAY_TICK_INTERVAL * failures as u32
}

#[derive(Debug)]
pub(super) struct OverlayRunLifecycle {
    pub(super) run_id: u64,
    pub(super) prefix: String,
    pub(super) completes_at: Instant,
    pub(super) closes_at: Instant,
    pub(super) dismiss_at: Option<Instant>,
    pub(super) next_close_attempt_at: Option<Instant>,
    pub(super) close_failures: usize,
    pub(super) completed: bool,
    pub(super) closing_emitted: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
pub(super) struct OverlayLifecycleUpdate {
    pub(super) emit_complete: bool,
    pub(super) emit_closing: bool,
    pub(super) close: bool,
}

impl OverlayRunLifecycle {
    fn automatic_closing_at(&self) -> Instant {
        self.closes_at
            .checked_sub(OVERLAY_DISMISS_DELAY)
            .unwrap_or(self.completes_at)
            .max(self.completes_at)
    }

    fn effective_close_at(&self) -> Instant {
        let close_at = self
            .dismiss_at
            .map_or(self.closes_at, |dismiss_at| dismiss_at.min(self.closes_at));
        self.next_close_attempt_at
            .map_or(close_at, |retry_at| retry_at.max(close_at))
    }

    pub(super) fn begin_dismiss(&mut self, now: Instant) -> bool {
        let emit_closing = !self.closing_emitted;
        self.closing_emitted = true;
        let dismiss_at = now + OVERLAY_DISMISS_DELAY;
        self.dismiss_at = Some(
            self.dismiss_at
                .map_or(dismiss_at, |current| current.min(dismiss_at)),
        );
        emit_closing
    }

    pub(super) fn defer_close_retry(&mut self, now: Instant) -> bool {
        self.close_failures += 1;
        if self.close_failures >= OVERLAY_CLOSE_FAILURE_LIMIT {
            return false;
        }
        self.next_close_attempt_at = Some(now + overlay_close_retry_delay(self.close_failures));
        true
    }

    pub(super) fn advance(&mut self, now: Instant) -> OverlayLifecycleUpdate {
        let emit_complete =
            self.dismiss_at.is_none() && !self.completed && now >= self.completes_at;
        if emit_complete {
            self.completed = true;
        }

        let emit_closing = self.dismiss_at.is_none()
            && !self.closing_emitted
            && now >= self.automatic_closing_at();
        if emit_closing {
            self.closing_emitted = true;
        }

        OverlayLifecycleUpdate {
            emit_complete,
            emit_closing,
            close: now >= self.effective_close_at(),
        }
    }
}

pub(super) fn overlay_worker_timeout(runs: &[OverlayRunLifecycle], now: Instant) -> Duration {
    runs.iter().fold(OVERLAY_TICK_INTERVAL, |timeout, run| {
        let mut next = run.effective_close_at();
        if run.dismiss_at.is_none() {
            if !run.completed {
                next = next.min(run.completes_at);
            }
            if !run.closing_emitted {
                next = next.min(run.automatic_closing_at());
            }
        }
        timeout.min(next.saturating_duration_since(now))
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn automatic_overlay_completion_precedes_fade_and_teardown() {
        let started_at = Instant::now();
        let completes_at = started_at + Duration::from_secs(20);
        let closes_at = completes_at + OVERLAY_COMPLETION_GRACE;
        let closing_at = closes_at - OVERLAY_DISMISS_DELAY;
        let mut run = OverlayRunLifecycle {
            run_id: 1,
            prefix: "overlay-1-".into(),
            completes_at,
            closes_at,
            dismiss_at: None,
            next_close_attempt_at: None,
            close_failures: 0,
            completed: false,
            closing_emitted: false,
        };

        assert_eq!(
            run.advance(completes_at - Duration::from_millis(1)),
            OverlayLifecycleUpdate::default()
        );
        assert_eq!(
            run.advance(completes_at),
            OverlayLifecycleUpdate {
                emit_complete: true,
                emit_closing: false,
                close: false,
            }
        );
        assert_eq!(
            run.advance(closing_at - Duration::from_millis(1)),
            OverlayLifecycleUpdate::default()
        );
        assert_eq!(
            overlay_worker_timeout(&[run], closing_at - Duration::from_millis(100)),
            Duration::from_millis(100)
        );

        let mut run = OverlayRunLifecycle {
            run_id: 1,
            prefix: "overlay-1-".into(),
            completes_at,
            closes_at,
            dismiss_at: None,
            next_close_attempt_at: None,
            close_failures: 0,
            completed: true,
            closing_emitted: false,
        };
        assert_eq!(
            run.advance(closing_at),
            OverlayLifecycleUpdate {
                emit_complete: false,
                emit_closing: true,
                close: false,
            }
        );
        assert_eq!(
            run.advance(closes_at - Duration::from_millis(1)),
            OverlayLifecycleUpdate::default()
        );
        assert_eq!(
            run.advance(closes_at),
            OverlayLifecycleUpdate {
                emit_complete: false,
                emit_closing: false,
                close: true,
            }
        );
    }

    #[test]
    fn failed_close_waits_one_tick_before_becoming_due_again() {
        let closes_at = Instant::now();
        let mut run = OverlayRunLifecycle {
            run_id: 2,
            prefix: "overlay-2-".into(),
            completes_at: closes_at,
            closes_at,
            dismiss_at: None,
            next_close_attempt_at: None,
            close_failures: 0,
            completed: true,
            closing_emitted: true,
        };

        assert!(run.advance(closes_at).close);
        run.defer_close_retry(closes_at);

        assert_eq!(
            overlay_worker_timeout(&[run], closes_at),
            OVERLAY_TICK_INTERVAL
        );

        let mut run = OverlayRunLifecycle {
            run_id: 2,
            prefix: "overlay-2-".into(),
            completes_at: closes_at,
            closes_at,
            dismiss_at: None,
            next_close_attempt_at: None,
            close_failures: 0,
            completed: true,
            closing_emitted: true,
        };
        run.defer_close_retry(closes_at);
        assert!(
            !run.advance(closes_at + OVERLAY_TICK_INTERVAL - Duration::from_millis(1))
                .close
        );
        assert!(run.advance(closes_at + OVERLAY_TICK_INTERVAL).close);
    }

    #[test]
    fn failed_close_stops_retrying_at_the_shared_limit() {
        let closes_at = Instant::now();
        let mut run = OverlayRunLifecycle {
            run_id: 3,
            prefix: "overlay-3-".into(),
            completes_at: closes_at,
            closes_at,
            dismiss_at: None,
            next_close_attempt_at: None,
            close_failures: 0,
            completed: true,
            closing_emitted: true,
        };

        for _ in 1..OVERLAY_CLOSE_FAILURE_LIMIT {
            assert!(run.defer_close_retry(closes_at));
        }
        assert!(!run.defer_close_retry(closes_at));
    }
}
