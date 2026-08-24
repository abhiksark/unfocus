use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static OVERLAY_RUN_ID: AtomicU64 = AtomicU64::new(1);
pub(super) const MAX_OVERLAY_MONITORS: usize = 64;
pub(crate) const MIN_OVERLAY_DURATION_SECONDS: u64 = 3;
pub(crate) const MAX_OVERLAY_DURATION_SECONDS: u64 = 30;
const JAVASCRIPT_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub(super) fn bounded_overlay_duration(seconds: u64) -> u64 {
    seconds.clamp(MIN_OVERLAY_DURATION_SECONDS, MAX_OVERLAY_DURATION_SECONDS)
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

pub(super) fn next_overlay_run_id() -> u64 {
    // Overlay starts are serialized by `OVERLAY_START_LOCK`, so this explicit
    // wrap keeps every label within JavaScript's canonical safe-integer range.
    let current = OVERLAY_RUN_ID.load(Ordering::Relaxed);
    let run_id = if (1..=JAVASCRIPT_MAX_SAFE_INTEGER).contains(&current) {
        current
    } else {
        1
    };
    let next = if run_id == JAVASCRIPT_MAX_SAFE_INTEGER {
        1
    } else {
        run_id + 1
    };
    OVERLAY_RUN_ID.store(next, Ordering::Relaxed);
    run_id
}

fn overlay_deadline_ms_from(now_ms: u64, duration_seconds: u64) -> Result<u64, String> {
    let duration_seconds = bounded_overlay_duration(duration_seconds);
    now_ms
        .checked_add(duration_seconds.saturating_mul(1_000))
        .filter(|deadline| *deadline <= JAVASCRIPT_MAX_SAFE_INTEGER)
        .ok_or_else(|| "overlay deadline exceeds JavaScript's safe-integer range".to_owned())
}

pub(super) fn overlay_deadline_ms(duration_seconds: u64) -> Result<u64, String> {
    overlay_deadline_ms_from(unix_timestamp_ms(), duration_seconds)
}

pub(super) fn overlay_label(
    run_id: u64,
    index: usize,
    total: usize,
    duration_seconds: u64,
    deadline_ms: u64,
) -> String {
    format!("overlay-{run_id}-{index}-{total}-{duration_seconds}-{deadline_ms}")
}

fn parse_canonical_u64(value: &str) -> Option<u64> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse().ok()
}

pub(crate) fn overlay_run_id_from_label(label: &str) -> Option<u64> {
    let mut parts = label.split('-');
    if parts.next()? != "overlay" {
        return None;
    }
    let run_id = parse_canonical_u64(parts.next()?)?;
    let index = parse_canonical_u64(parts.next()?)?;
    let total = parse_canonical_u64(parts.next()?)?;
    let duration = parse_canonical_u64(parts.next()?)?;
    let deadline = parse_canonical_u64(parts.next()?)?;
    if parts.next().is_some()
        || run_id == 0
        || run_id > JAVASCRIPT_MAX_SAFE_INTEGER
        || total == 0
        || total > MAX_OVERLAY_MONITORS as u64
        || index >= total
        || !(MIN_OVERLAY_DURATION_SECONDS..=MAX_OVERLAY_DURATION_SECONDS).contains(&duration)
        || deadline == 0
        || deadline > JAVASCRIPT_MAX_SAFE_INTEGER
    {
        return None;
    }

    Some(run_id)
}

pub(super) fn authorize_overlay_close_caller(
    label: &str,
    requested_run_id: u64,
) -> Result<(), String> {
    match overlay_run_id_from_label(label) {
        Some(caller_run_id) if caller_run_id == requested_run_id => Ok(()),
        Some(_) => Err("an overlay can only close its own run".into()),
        None => Err("this command is only available to a valid overlay window".into()),
    }
}

/// Pure multi-monitor plan for one overlay run (issue #30 topology contract).
/// Does not talk to the OS; used by tests to pin label uniqueness and shared
/// run identity.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct OverlayRunPlan {
    pub(super) run_id: u64,
    pub(super) duration_seconds: u64,
    pub(super) deadline_ms: u64,
    pub(super) labels: Vec<String>,
}

/// Build the full set of window labels for `monitor_count` displays.
///
/// Rejects empty topologies and counts above [`MAX_OVERLAY_MONITORS`]. Every
/// label shares the same run id, duration, and absolute deadline.
pub(super) fn plan_overlay_run(
    run_id: u64,
    monitor_count: usize,
    duration_seconds: u64,
    deadline_ms: u64,
) -> Result<OverlayRunPlan, String> {
    if monitor_count == 0 {
        return Err("Tauri did not report any monitors".into());
    }
    if monitor_count > MAX_OVERLAY_MONITORS {
        return Err(format!(
            "Tauri reported {monitor_count} monitors; overlays support at most {MAX_OVERLAY_MONITORS}"
        ));
    }
    if run_id == 0 || run_id > JAVASCRIPT_MAX_SAFE_INTEGER {
        return Err("overlay run id is outside the safe integer range".into());
    }
    if deadline_ms == 0 || deadline_ms > JAVASCRIPT_MAX_SAFE_INTEGER {
        return Err("overlay deadline is outside the safe integer range".into());
    }
    let duration_seconds = bounded_overlay_duration(duration_seconds);
    let labels = (0..monitor_count)
        .map(|index| overlay_label(run_id, index, monitor_count, duration_seconds, deadline_ms))
        .collect();
    Ok(OverlayRunPlan {
        run_id,
        duration_seconds,
        deadline_ms,
        labels,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorize_main_caller;
    use std::collections::HashSet;

    #[test]
    fn overlay_run_labels_are_parsed_strictly() {
        assert_eq!(
            overlay_run_id_from_label("overlay-7-1-2-20-1800000000000"),
            Some(7)
        );
        assert_eq!(overlay_run_id_from_label("overlay-garbage"), None);
        assert_eq!(
            overlay_run_id_from_label("overlay-07-1-2-20-1800000000000"),
            None
        );
        assert_eq!(
            overlay_run_id_from_label("overlay-7-2-2-20-1800000000000"),
            None
        );
        assert_eq!(
            overlay_run_id_from_label("overlay-7-1-2-20-1800000000000-extra"),
            None
        );
        assert_eq!(
            overlay_run_id_from_label("overlay-9007199254740992-1-2-20-1800000000000"),
            None
        );
        assert_eq!(
            overlay_run_id_from_label("overlay-7-1-65-20-1800000000000"),
            None
        );
    }

    #[test]
    fn command_callers_are_authorized_from_their_window_labels() {
        let overlay = "overlay-7-1-2-20-1800000000000";

        assert_eq!(authorize_main_caller("main"), Ok(()));
        assert!(authorize_main_caller(overlay).is_err());
        assert_eq!(authorize_overlay_close_caller(overlay, 7), Ok(()));
        assert!(authorize_overlay_close_caller(overlay, 8).is_err());
        assert!(authorize_overlay_close_caller("main", 7).is_err());
        assert!(authorize_overlay_close_caller("overlay-garbage", 7).is_err());
    }

    #[test]
    fn overlay_test_duration_has_safe_bounds() {
        assert_eq!(bounded_overlay_duration(0), 3);
        assert_eq!(bounded_overlay_duration(8), 8);
        assert_eq!(bounded_overlay_duration(300), 30);
    }

    #[test]
    fn overlay_deadline_uses_the_bounded_duration() {
        assert_eq!(
            overlay_deadline_ms_from(1_800_000_000_000, 300),
            Ok(1_800_000_030_000)
        );
    }

    #[test]
    fn overlay_labels_share_an_absolute_deadline() {
        assert_eq!(
            overlay_label(7, 1, 2, 20, 1_800_000_000_000),
            "overlay-7-1-2-20-1800000000000"
        );
    }

    #[test]
    fn multi_monitor_plans_share_run_id_and_deadline_without_duplicates() {
        let plan = plan_overlay_run(9, 3, 20, 1_800_000_000_000).expect("valid topology");
        assert_eq!(plan.labels.len(), 3);
        assert_eq!(
            plan.labels,
            vec![
                "overlay-9-0-3-20-1800000000000".to_owned(),
                "overlay-9-1-3-20-1800000000000".to_owned(),
                "overlay-9-2-3-20-1800000000000".to_owned(),
            ]
        );
        let unique: HashSet<_> = plan.labels.iter().collect();
        assert_eq!(unique.len(), 3);
        for label in &plan.labels {
            assert_eq!(overlay_run_id_from_label(label), Some(9));
        }
    }

    #[test]
    fn multi_monitor_plans_reject_empty_and_oversized_topologies() {
        assert!(plan_overlay_run(1, 0, 20, 1_800_000_000_000).is_err());
        assert!(plan_overlay_run(1, MAX_OVERLAY_MONITORS + 1, 20, 1_800_000_000_000).is_err());
        let max = plan_overlay_run(2, MAX_OVERLAY_MONITORS, 8, 1_800_000_000_000).expect("max");
        assert_eq!(max.labels.len(), MAX_OVERLAY_MONITORS);
        assert_eq!(max.duration_seconds, 8);
    }

    #[test]
    fn multi_monitor_plans_clamp_duration_and_reject_unsafe_ids() {
        let clamped = plan_overlay_run(3, 1, 300, 1_800_000_000_000).expect("clamped");
        assert_eq!(clamped.duration_seconds, MAX_OVERLAY_DURATION_SECONDS);
        assert!(plan_overlay_run(0, 1, 20, 1_800_000_000_000).is_err());
        assert!(plan_overlay_run(1, 1, 20, 0).is_err());
        assert!(
            plan_overlay_run(JAVASCRIPT_MAX_SAFE_INTEGER + 1, 1, 20, 1_800_000_000_000).is_err()
        );
    }

    #[test]
    fn mixed_scale_or_negative_origin_does_not_change_label_identity() {
        // Geometry is applied at window creation; the pure plan only depends on
        // count and shared deadline so mixed DPI / negative x,y cannot fork run ids.
        let left_of_origin = plan_overlay_run(4, 2, 20, 1_700_000_000_000).expect("two");
        let after_hotplug_add = plan_overlay_run(4, 3, 20, 1_700_000_000_000).expect("three");
        assert_eq!(left_of_origin.run_id, after_hotplug_add.run_id);
        assert_eq!(left_of_origin.deadline_ms, after_hotplug_add.deadline_ms);
        // Mid-run we never apply a new plan; a new plan is only for the next break.
        assert_ne!(left_of_origin.labels.len(), after_hotplug_add.labels.len());
    }
}
