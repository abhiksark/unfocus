use std::{
    sync::atomic::{AtomicU64, Ordering},
    time::{SystemTime, UNIX_EPOCH},
};

static OVERLAY_RUN_ID: AtomicU64 = AtomicU64::new(1);
pub(super) const MAX_OVERLAY_MONITORS: usize = 64;
const JAVASCRIPT_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

pub(super) fn bounded_overlay_duration(seconds: u64) -> u64 {
    seconds.clamp(3, 30)
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

pub(super) fn overlay_deadline_ms(duration_seconds: u64) -> Result<u64, String> {
    unix_timestamp_ms()
        .checked_add(duration_seconds.saturating_mul(1_000))
        .filter(|deadline| *deadline <= JAVASCRIPT_MAX_SAFE_INTEGER)
        .ok_or_else(|| "overlay deadline exceeds JavaScript's safe-integer range".to_owned())
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
        || !(3..=30).contains(&duration)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::authorize_main_caller;

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
    fn overlay_labels_share_an_absolute_deadline() {
        assert_eq!(
            overlay_label(7, 1, 2, 20, 1_800_000_000_000),
            "overlay-7-1-2-20-1800000000000"
        );
    }
}
