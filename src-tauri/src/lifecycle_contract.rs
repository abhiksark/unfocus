//! Shared lifecycle and multi-monitor qualification contract (issue #30).
//!
//! These pure helpers document product semantics that acceptance runs must
//! verify on hardware. They do not talk to the OS; the reminder timer and
//! overlay modules implement the behavior these tests pin.

use std::time::Duration;

/// How the pure reminder clock treats a long gap between polls (suspend,
/// freeze, debugger pause, or a stalled scheduler thread).
///
/// Product rule: Break and Pause durations, and Working in relative mode, are
/// measured in the injected monotonic clock. Working in sync mode uses a
/// stored wall-clock deadline; see `discontinuity_observation`. When the
/// monotonic clock jumps past one or more phase deadlines, the scheduler may
/// perform **at most one** phase transition for that observation. It never
/// replays a backlog of work/break cycles or presents multiple overlays.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StallObservation {
    /// Still inside the current phase after the gap.
    StillInPhase,
    /// Crossed exactly one phase boundary (work→break or break→work).
    SingleTransition,
}

/// Classify a pure-timer observation after a long stall.
///
/// `elapsed_in_phase` is how far into the current phase the clock was before
/// the gap; `phase_duration` is that phase's configured length; `gap` is how
/// much monotonic time advanced during the stall.
pub(crate) fn stall_observation(
    elapsed_in_phase: Duration,
    phase_duration: Duration,
    gap: Duration,
) -> StallObservation {
    let after = elapsed_in_phase.saturating_add(gap);
    if after < phase_duration {
        StallObservation::StillInPhase
    } else {
        StallObservation::SingleTransition
    }
}

/// How many automatic phase transitions a pure timer is allowed to emit for
/// one `tick(now)` call. Always 0 or 1; never a backlog size.
pub(crate) fn max_transitions_per_tick() -> usize {
    1
}

/// Whether the wall and monotonic clocks stayed consistent between two polls.
///
/// A stall advances both clocks together and is handled by
/// `stall_observation`. A discontinuity — a clock step, or a suspend on
/// platforms where it is observable — moves them apart. `std` does not specify
/// whether suspends count as elapsed monotonic time, so the design must not
/// depend on suspend being detected here; an undetected wake degrades into the
/// stall case and is then absorbed by the idle-probe presentation veto.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum DiscontinuityObservation {
    Continuous,
    Rebased,
}

pub(crate) fn discontinuity_observation(
    wall_delta_ms: i64,
    mono_delta_ms: i64,
    tolerance_ms: i64,
) -> DiscontinuityObservation {
    if wall_delta_ms.abs_diff(mono_delta_ms) > tolerance_ms.unsigned_abs() {
        DiscontinuityObservation::Rebased
    } else {
        DiscontinuityObservation::Continuous
    }
}

/// Product policy for display topology changes relative to an overlay run.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TopologyPolicy {
    /// Enumerate monitors only when starting a new run.
    EnumerateOnStart,
    /// A display lost mid-run (or any unexpected sibling window loss) ends the
    /// entire run so the desk is never half-covered.
    UnexpectedWindowLossEndsRun,
    /// A newly attached display does not get an overlay mid-run; the next
    /// break covers the full set.
    NoMidRunSpawn,
    /// Partial create failure rolls back every window already opened for the run.
    PartialCreateRollsBack,
}

pub(crate) fn topology_policies() -> &'static [TopologyPolicy] {
    &[
        TopologyPolicy::EnumerateOnStart,
        TopologyPolicy::UnexpectedWindowLossEndsRun,
        TopologyPolicy::NoMidRunSpawn,
        TopologyPolicy::PartialCreateRollsBack,
    ]
}

/// Whether a topology event during an active run should spawn additional
/// overlay windows immediately.
pub(crate) fn should_spawn_on_hotplug_add(run_active: bool) -> bool {
    !run_active
}

/// Whether losing one overlay window unexpectedly should tear down siblings.
pub(crate) fn unexpected_sibling_loss_ends_run() -> bool {
    true
}

/// Evidence status for a qualification row. Automated CI must not report a
/// skipped physical case as passing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum EvidenceStatus {
    Automated,
    PhysicallyObserved,
    NotRun,
    Failed,
}

impl EvidenceStatus {
    pub(crate) fn may_claim_pass(self) -> bool {
        matches!(
            self,
            EvidenceStatus::Automated | EvidenceStatus::PhysicallyObserved
        )
    }
}

/// Release tier required for a qualification claim. Alpha may ship with a
/// narrower physical matrix than stable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReleaseTier {
    Alpha,
    Beta,
    Stable,
}

/// Whether Linux X11 multi-monitor lifecycle acceptance is required for a tier.
pub(crate) fn linux_x11_lifecycle_required(tier: ReleaseTier) -> bool {
    match tier {
        ReleaseTier::Alpha | ReleaseTier::Beta | ReleaseTier::Stable => true,
    }
}

/// Whether macOS multi-monitor acceptance is required for a tier.
/// macOS remains preview until #23 evidence lands; not required for alpha.
pub(crate) fn macos_multi_monitor_required(tier: ReleaseTier) -> bool {
    matches!(tier, ReleaseTier::Stable)
}

/// Accessibility contract bits that pure frontend helpers must preserve.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct AccessibilityContract {
    pub countdown_must_not_live_region_every_second: bool,
    pub reduced_motion_stops_loops: bool,
    pub state_also_in_text: bool,
}

pub(crate) fn accessibility_contract() -> AccessibilityContract {
    AccessibilityContract {
        countdown_must_not_live_region_every_second: true,
        reduced_motion_stops_loops: true,
        state_also_in_text: true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_stall_crossing_many_phase_lengths_is_still_one_transition() {
        let work = Duration::from_secs(20 * 60);
        // 5 hours of wall sleep relative to a 20-minute phase.
        let gap = Duration::from_secs(5 * 60 * 60);
        assert_eq!(
            stall_observation(Duration::from_secs(60), work, gap),
            StallObservation::SingleTransition
        );
        assert_eq!(max_transitions_per_tick(), 1);
    }

    #[test]
    fn short_stall_inside_a_phase_does_not_transition() {
        assert_eq!(
            stall_observation(
                Duration::from_secs(30),
                Duration::from_secs(60),
                Duration::from_secs(10)
            ),
            StallObservation::StillInPhase
        );
    }

    #[test]
    fn stall_exactly_on_boundary_is_a_single_transition() {
        assert_eq!(
            stall_observation(
                Duration::from_secs(0),
                Duration::from_secs(60),
                Duration::from_secs(60)
            ),
            StallObservation::SingleTransition
        );
    }

    #[test]
    fn hotplug_add_never_spawns_during_an_active_run() {
        assert!(!should_spawn_on_hotplug_add(true));
        assert!(should_spawn_on_hotplug_add(false));
        assert!(unexpected_sibling_loss_ends_run());
        assert!(topology_policies().contains(&TopologyPolicy::NoMidRunSpawn));
        assert!(topology_policies().contains(&TopologyPolicy::PartialCreateRollsBack));
        assert!(topology_policies().contains(&TopologyPolicy::UnexpectedWindowLossEndsRun));
    }

    #[test]
    fn evidence_status_cannot_claim_pass_when_not_run_or_failed() {
        assert!(EvidenceStatus::Automated.may_claim_pass());
        assert!(EvidenceStatus::PhysicallyObserved.may_claim_pass());
        assert!(!EvidenceStatus::NotRun.may_claim_pass());
        assert!(!EvidenceStatus::Failed.may_claim_pass());
    }

    #[test]
    fn release_tiers_require_linux_x11_and_defer_macos_until_stable() {
        assert!(linux_x11_lifecycle_required(ReleaseTier::Alpha));
        assert!(!macos_multi_monitor_required(ReleaseTier::Alpha));
        assert!(!macos_multi_monitor_required(ReleaseTier::Beta));
        assert!(macos_multi_monitor_required(ReleaseTier::Stable));
    }

    #[test]
    fn accessibility_contract_keeps_calm_break_announcements() {
        let contract = accessibility_contract();
        assert!(contract.countdown_must_not_live_region_every_second);
        assert!(contract.reduced_motion_stops_loops);
        assert!(contract.state_also_in_text);
    }

    #[test]
    fn clocks_advancing_together_are_continuous_however_long_the_stall() {
        // A starved thread advances both clocks equally; that is a stall, not a
        // discontinuity, and the existing stall rule handles it.
        assert_eq!(
            discontinuity_observation(600_000, 600_000, 5_000),
            DiscontinuityObservation::Continuous
        );
    }

    #[test]
    fn a_forward_clock_step_is_a_discontinuity() {
        assert_eq!(
            discontinuity_observation(3_600_000, 250, 5_000),
            DiscontinuityObservation::Rebased
        );
    }

    #[test]
    fn a_backward_clock_step_is_a_discontinuity() {
        assert_eq!(
            discontinuity_observation(-3_600_000, 250, 5_000),
            DiscontinuityObservation::Rebased
        );
    }

    #[test]
    fn divergence_exactly_at_tolerance_is_still_continuous() {
        assert_eq!(
            discontinuity_observation(5_250, 250, 5_000),
            DiscontinuityObservation::Continuous
        );
    }

    #[test]
    fn topology_reconcile_never_mixes_runs_or_spawns_mid_break() {
        // Mid-run: add display → no spawn. Remove/unexpected loss → end run.
        assert!(!should_spawn_on_hotplug_add(true));
        assert!(unexpected_sibling_loss_ends_run());
        assert_eq!(max_transitions_per_tick(), 1);
        // After the run ends, a later start may enumerate a larger topology.
        assert!(should_spawn_on_hotplug_add(false));
    }
}
