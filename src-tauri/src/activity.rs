//! Continuous activity and AFK segmentation from idle-probe samples.
//!
//! Privacy boundary: only presence of input (OS idle seconds) is used. No
//! keylogging, mouse paths, app titles, or window content.

use serde::Serialize;
use std::{
    sync::{Arc, Mutex},
    time::{SystemTime, UNIX_EPOCH},
};

/// No keyboard/mouse input for this long means AFK (industry default ~5 min).
pub(crate) const AFK_THRESHOLD_SECONDS: u64 = 5 * 60;
/// Continuous active stretch at least this long counts as a deep block.
pub(crate) const DEEP_BLOCK_MIN_SECONDS: u64 = 25 * 60;
/// Rolling observation window for the dashboard strip and totals.
pub(crate) const ACTIVITY_WINDOW_SECONDS: u64 = 24 * 60 * 60;
/// Half-hour buckets across the rolling window (48 × 30 min = 24 h).
pub(crate) const STRIP_BUCKET_COUNT: usize = 48;

const MILLIS_PER_SECOND: u64 = 1_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ActivityKind {
    Active,
    Afk,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Segment {
    kind: ActivityKind,
    start_ms: u64,
    end_ms: u64,
}

#[derive(Debug, Clone)]
pub(crate) struct ActivityTracker {
    afk_threshold_seconds: u64,
    deep_block_min_seconds: u64,
    window_seconds: u64,
    strip_bucket_count: usize,
    segments: Vec<Segment>,
    last_sample_ms: Option<u64>,
    last_kind: Option<ActivityKind>,
    last_probe_ok: bool,
}

impl Default for ActivityTracker {
    fn default() -> Self {
        Self::new(
            AFK_THRESHOLD_SECONDS,
            DEEP_BLOCK_MIN_SECONDS,
            ACTIVITY_WINDOW_SECONDS,
            STRIP_BUCKET_COUNT,
        )
    }
}

impl ActivityTracker {
    pub(crate) fn new(
        afk_threshold_seconds: u64,
        deep_block_min_seconds: u64,
        window_seconds: u64,
        strip_bucket_count: usize,
    ) -> Self {
        assert!(afk_threshold_seconds > 0);
        assert!(deep_block_min_seconds > 0);
        assert!(window_seconds > 0);
        assert!(strip_bucket_count > 0);
        Self {
            afk_threshold_seconds,
            deep_block_min_seconds,
            window_seconds,
            strip_bucket_count,
            segments: Vec::new(),
            last_sample_ms: None,
            last_kind: None,
            last_probe_ok: false,
        }
    }

    /// Record one idle reading at `now_ms` (Unix epoch milliseconds).
    ///
    /// `idle_seconds = None` means the probe failed; classification freezes and
    /// does not invent active or AFK time (fail open for the timer elsewhere).
    pub(crate) fn observe(&mut self, now_ms: u64, idle_seconds: Option<u64>) {
        if let Some(previous) = self.last_sample_ms {
            if now_ms < previous {
                // Clock went backwards; drop history rather than corrupt spans.
                self.segments.clear();
                self.last_kind = None;
            }
        }

        self.last_sample_ms = Some(now_ms);
        let kind = match idle_seconds {
            Some(seconds) if seconds >= self.afk_threshold_seconds => {
                self.last_probe_ok = true;
                ActivityKind::Afk
            }
            Some(_) => {
                self.last_probe_ok = true;
                ActivityKind::Active
            }
            None => {
                self.last_probe_ok = false;
                ActivityKind::Unknown
            }
        };

        match kind {
            ActivityKind::Unknown => {
                // Close any open classified segment at the previous good end;
                // do not invent duration while the probe is dark.
                self.last_kind = Some(ActivityKind::Unknown);
            }
            ActivityKind::Active | ActivityKind::Afk => {
                self.extend_or_open(kind, now_ms);
                self.last_kind = Some(kind);
            }
        }

        self.prune(now_ms);
    }

    fn extend_or_open(&mut self, kind: ActivityKind, now_ms: u64) {
        if let Some(last) = self.segments.last_mut() {
            if last.kind == kind {
                if now_ms >= last.end_ms {
                    last.end_ms = now_ms;
                }
                return;
            }
            // Close previous at the transition instant.
            if now_ms >= last.start_ms {
                last.end_ms = now_ms;
            }
        }
        self.segments.push(Segment {
            kind,
            start_ms: now_ms,
            end_ms: now_ms,
        });
    }

    fn prune(&mut self, now_ms: u64) {
        let window_ms = self.window_seconds.saturating_mul(MILLIS_PER_SECOND);
        let cutoff = now_ms.saturating_sub(window_ms);
        self.segments.retain_mut(|segment| {
            if segment.end_ms <= cutoff {
                return false;
            }
            if segment.start_ms < cutoff {
                segment.start_ms = cutoff;
            }
            true
        });
    }

    pub(crate) fn summary(&self, now_ms: u64) -> ActivitySummary {
        let window_ms = self.window_seconds.saturating_mul(MILLIS_PER_SECOND);
        let window_start = now_ms.saturating_sub(window_ms);

        let mut active_ms = 0_u64;
        let mut afk_ms = 0_u64;
        let mut longest_active_ms = 0_u64;
        let mut deep_block_count = 0_u64;
        let deep_min_ms = self
            .deep_block_min_seconds
            .saturating_mul(MILLIS_PER_SECOND);

        for segment in &self.segments {
            let start = segment.start_ms.max(window_start);
            let end = segment.end_ms.min(now_ms).max(start);
            let duration = end.saturating_sub(start);
            match segment.kind {
                ActivityKind::Active => {
                    active_ms = active_ms.saturating_add(duration);
                    longest_active_ms = longest_active_ms.max(duration);
                    if duration >= deep_min_ms {
                        deep_block_count = deep_block_count.saturating_add(1);
                    }
                }
                ActivityKind::Afk => {
                    afk_ms = afk_ms.saturating_add(duration);
                }
                ActivityKind::Unknown => {}
            }
        }

        let known_ms = active_ms.saturating_add(afk_ms);
        let unknown_ms = window_ms.saturating_sub(known_ms.min(window_ms));

        ActivitySummary {
            window_label: "Last 24 hours".into(),
            window_seconds: self.window_seconds,
            active_seconds: active_ms / MILLIS_PER_SECOND,
            afk_seconds: afk_ms / MILLIS_PER_SECOND,
            unknown_seconds: unknown_ms / MILLIS_PER_SECOND,
            longest_active_seconds: longest_active_ms / MILLIS_PER_SECOND,
            deep_block_count,
            deep_block_min_seconds: self.deep_block_min_seconds,
            afk_threshold_seconds: self.afk_threshold_seconds,
            current_kind: self.last_kind,
            probe_available: self.last_probe_ok,
            strip: self.strip_buckets(now_ms, window_start, window_ms),
        }
    }

    fn strip_buckets(&self, now_ms: u64, window_start: u64, window_ms: u64) -> Vec<StripBucket> {
        let count = self.strip_bucket_count;
        let bucket_ms = (window_ms / count as u64).max(1);
        let mut buckets = Vec::with_capacity(count);

        for index in 0..count {
            let start = window_start.saturating_add(bucket_ms.saturating_mul(index as u64));
            let end = if index + 1 == count {
                now_ms.max(start)
            } else {
                start.saturating_add(bucket_ms).min(now_ms.max(start))
            };
            let span = end.saturating_sub(start).max(1);
            let mut active_ms = 0_u64;
            let mut afk_ms = 0_u64;
            for segment in &self.segments {
                let overlap_start = segment.start_ms.max(start);
                let overlap_end = segment.end_ms.min(end);
                if overlap_end <= overlap_start {
                    continue;
                }
                let overlap = overlap_end - overlap_start;
                match segment.kind {
                    ActivityKind::Active => active_ms = active_ms.saturating_add(overlap),
                    ActivityKind::Afk => afk_ms = afk_ms.saturating_add(overlap),
                    ActivityKind::Unknown => {}
                }
            }
            let active_ratio = (active_ms as f64 / span as f64).clamp(0.0, 1.0);
            let afk_ratio = (afk_ms as f64 / span as f64).clamp(0.0, 1.0);
            buckets.push(StripBucket {
                active_ratio,
                afk_ratio,
            });
        }
        buckets
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct StripBucket {
    pub(crate) active_ratio: f64,
    pub(crate) afk_ratio: f64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ActivitySummary {
    pub(crate) window_label: String,
    pub(crate) window_seconds: u64,
    pub(crate) active_seconds: u64,
    pub(crate) afk_seconds: u64,
    pub(crate) unknown_seconds: u64,
    pub(crate) longest_active_seconds: u64,
    pub(crate) deep_block_count: u64,
    pub(crate) deep_block_min_seconds: u64,
    pub(crate) afk_threshold_seconds: u64,
    pub(crate) current_kind: Option<ActivityKind>,
    pub(crate) probe_available: bool,
    pub(crate) strip: Vec<StripBucket>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ActivityTrackerHandle {
    inner: Arc<Mutex<ActivityTracker>>,
}

impl ActivityTrackerHandle {
    #[cfg(test)]
    fn new(tracker: ActivityTracker) -> Self {
        Self {
            inner: Arc::new(Mutex::new(tracker)),
        }
    }

    pub(crate) fn observe(&self, now_ms: u64, idle_seconds: Option<u64>) {
        if let Ok(mut tracker) = self.inner.lock() {
            tracker.observe(now_ms, idle_seconds);
        }
    }

    pub(crate) fn summary(&self, now_ms: u64) -> ActivitySummary {
        self.inner
            .lock()
            .map(|tracker| tracker.summary(now_ms))
            .unwrap_or_else(|_| ActivityTracker::default().summary(now_ms))
    }
}

pub(crate) fn epoch_ms(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

#[tauri::command]
pub(crate) fn get_today_activity(
    window: tauri::WebviewWindow,
    tracker: tauri::State<'_, ActivityTrackerHandle>,
) -> Result<ActivitySummary, String> {
    crate::authorize_main_caller(window.label())?;
    Ok(tracker.summary(epoch_ms(SystemTime::now())))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tracker() -> ActivityTracker {
        ActivityTracker::new(300, 25 * 60, 24 * 60 * 60, 48)
    }

    #[test]
    fn classifies_active_and_afk_from_idle_threshold() {
        let mut tracker = tracker();
        let t0 = 1_700_000_000_000_u64;

        tracker.observe(t0, Some(0));
        tracker.observe(t0 + 60_000, Some(5));
        tracker.observe(t0 + 120_000, Some(400));
        tracker.observe(t0 + 180_000, Some(450));
        tracker.observe(t0 + 240_000, Some(1));

        let summary = tracker.summary(t0 + 240_000);
        assert!(summary.probe_available);
        assert_eq!(summary.current_kind, Some(ActivityKind::Active));
        assert!(summary.active_seconds >= 120);
        assert!(summary.afk_seconds >= 60);
        assert_eq!(summary.strip.len(), 48);
        assert_eq!(summary.afk_threshold_seconds, 300);
        assert_eq!(summary.deep_block_min_seconds, 25 * 60);
    }

    #[test]
    fn probe_failure_does_not_invent_active_or_afk_time() {
        let mut tracker = tracker();
        let t0 = 1_700_000_000_000_u64;
        tracker.observe(t0, Some(0));
        tracker.observe(t0 + 60_000, Some(10));
        let before = tracker.summary(t0 + 60_000);

        tracker.observe(t0 + 120_000, None);
        tracker.observe(t0 + 180_000, None);
        let after = tracker.summary(t0 + 180_000);

        assert!(!after.probe_available);
        assert_eq!(after.current_kind, Some(ActivityKind::Unknown));
        // Known classified time must not grow while the probe is dark.
        assert_eq!(after.active_seconds, before.active_seconds);
        assert_eq!(after.afk_seconds, before.afk_seconds);
    }

    #[test]
    fn counts_deep_blocks_from_long_active_stretches() {
        let mut tracker = ActivityTracker::new(60, 120, 24 * 60 * 60, 4);
        let t0 = 1_700_000_000_000_u64;
        // 3 minutes active continuously (>= 120 s deep min).
        tracker.observe(t0, Some(0));
        tracker.observe(t0 + 180_000, Some(0));
        let summary = tracker.summary(t0 + 180_000);
        assert_eq!(summary.deep_block_count, 1);
        assert_eq!(summary.longest_active_seconds, 180);
    }

    #[test]
    fn prunes_segments_outside_the_rolling_window() {
        let mut tracker = ActivityTracker::new(60, 120, 300, 3);
        let t0 = 1_700_000_000_000_u64;
        tracker.observe(t0, Some(0));
        tracker.observe(t0 + 100_000, Some(0));
        // Jump far past the 300 s window.
        tracker.observe(t0 + 1_000_000, Some(0));
        let summary = tracker.summary(t0 + 1_000_000);
        assert!(summary.active_seconds <= 300);
        assert_eq!(summary.strip.len(), 3);
    }

    #[test]
    fn handle_exposes_summary_for_commands() {
        let handle = ActivityTrackerHandle::new(tracker());
        let t0 = 1_700_000_000_000_u64;
        handle.observe(t0, Some(0));
        handle.observe(t0 + 90_000, Some(2));
        let summary = handle.summary(t0 + 90_000);
        assert_eq!(summary.window_label, "Last 24 hours");
        assert!(summary.active_seconds >= 90);
        assert_eq!(summary.current_kind, Some(ActivityKind::Active));
    }
}
