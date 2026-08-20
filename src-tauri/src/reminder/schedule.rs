//! Pure wall-clock break-grid scheduling for sync mode.
//!
//! Rust has no calendar and must not gain one (`src-tauri/AGENTS.md`). Aligning
//! breaks to local midnight needs only a UTC offset in minutes, so everything
//! here is integer arithmetic over Unix seconds. `rem_euclid` keeps negative
//! offsets correct, and because only `offset % interval` can affect the result,
//! callers may store the raw offset without normalising it.

/// The next grid point strictly after `unix_secs`.
///
/// Never returns `unix_secs` itself: a break must not re-fire on the grid point
/// that just triggered it.
#[allow(dead_code)]
pub(crate) fn next_grid(unix_secs: i64, interval_secs: i64, offset_minutes: i16) -> i64 {
    let local_seconds = unix_secs + i64::from(offset_minutes) * 60;
    let phase = local_seconds.rem_euclid(interval_secs);
    unix_secs + (interval_secs - phase)
}

/// The next grid point, skipping one that has not been earned.
///
/// Applied once on Working entry and then stored, so it never re-runs in steady
/// state. `interval_secs` is always a whole number of minutes and therefore
/// even, so the halving cannot truncate.
#[allow(dead_code)]
pub(crate) fn deadline_with_grace(unix_secs: i64, interval_secs: i64, offset_minutes: i16) -> i64 {
    let next = next_grid(unix_secs, interval_secs, offset_minutes);
    if next - unix_secs < interval_secs / 2 {
        next + interval_secs
    } else {
        next
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const BASE: i64 = 1_787_220_420; // 2026-08-20T10:07:00Z

    #[test]
    fn grid_points_align_to_local_midnight_across_offset_zones() {
        // (offset_minutes, interval_secs, expected)
        let cases: &[(i16, i64, i64)] = &[
            (0, 1200, 1_787_221_200),
            (0, 1500, 1_787_221_500),
            (330, 1200, 1_787_220_600),
            (330, 1500, 1_787_221_200),
            (345, 1200, 1_787_220_900),
            (345, 1500, 1_787_221_800),
            (765, 1200, 1_787_220_900),
            (765, 1500, 1_787_220_600),
            (-300, 1200, 1_787_221_200),
            (-300, 1500, 1_787_221_500),
            (-720, 1200, 1_787_221_200),
            (-720, 1500, 1_787_221_200),
            (840, 1200, 1_787_221_200),
            (840, 1500, 1_787_220_600),
        ];
        for &(offset, interval, expected) in cases {
            assert_eq!(
                next_grid(BASE, interval, offset),
                expected,
                "offset {offset} interval {interval}"
            );
        }
    }

    #[test]
    fn a_grid_point_is_always_strictly_after_the_observation() {
        let on_point = next_grid(BASE, 1200, 0);
        assert_eq!(next_grid(on_point, 1200, 0) - on_point, 1200);
    }

    #[test]
    fn a_grid_point_is_never_more_than_one_interval_away() {
        for interval in [60, 1200, 1500, 3600, 7200] {
            for offset in [0i16, 330, 345, 765, -300, -720, 840] {
                let delta = next_grid(BASE, interval, offset) - BASE;
                assert!(
                    delta > 0 && delta <= interval,
                    "interval {interval} offset {offset}"
                );
            }
        }
    }

    #[test]
    fn only_the_offset_remainder_changes_the_grid() {
        // Storing the raw offset is safe because whole intervals cancel out.
        for (offset, interval) in [(330i16, 1200i64), (-300, 1200), (765, 1500)] {
            let whole_intervals = (interval / 60) as i16;
            assert_eq!(
                next_grid(BASE, interval, offset),
                next_grid(BASE, interval, offset + whole_intervals)
            );
        }
    }

    #[test]
    fn grace_skips_a_grid_point_less_than_half_an_interval_away() {
        // IST, twenty-minute grid at local :00/:20/:40.
        // (local start time, expected local break time)
        let ist = 330i16;
        let at = |hh: i64, mm: i64| {
            // 2026-08-20T00:00:00Z is 1787184000.
            1_787_184_000 + hh * 3600 + mm * 60 - i64::from(ist) * 60
        };
        assert_eq!(
            deadline_with_grace(at(10, 1), 1200, ist),
            at(10, 20),
            "10:01 takes 10:20"
        );
        assert_eq!(
            deadline_with_grace(at(10, 11), 1200, ist),
            at(10, 40),
            "10:11 skips to 10:40"
        );
        assert_eq!(
            deadline_with_grace(at(10, 19), 1200, ist),
            at(10, 40),
            "10:19 skips to 10:40"
        );
        assert_eq!(
            deadline_with_grace(at(10, 21), 1200, ist),
            at(10, 40),
            "10:21 takes 10:40"
        );
    }

    #[test]
    fn grace_takes_a_grid_point_exactly_half_an_interval_away() {
        // The comparison is `<`, so the exact boundary is taken, not skipped.
        let half_away = next_grid(BASE, 1200, 0) - 600;
        assert_eq!(deadline_with_grace(half_away, 1200, 0), next_grid(BASE, 1200, 0));
    }
}
