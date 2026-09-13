//! Ephemeral, scheduler-owned manual preparation; clocks and eligibility are injected.
use serde::Serialize;
use std::time::Duration;

const PREPARATION_DURATION: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ManualPreparationStatus {
    pub(crate) request_id: Option<u64>,
    pub(crate) remaining_milliseconds: Option<u64>,
}

#[derive(Debug, Clone, Copy)]
struct Pending {
    id: u64,
    deadline: Duration,
    revision: (u64, u64),
}

#[derive(Default)]
pub(super) struct ManualPreparation {
    pending: Option<Pending>,
    next_id: u64,
}

impl ManualPreparation {
    pub(super) fn begin(&mut self, now: Duration, revision: (u64, u64)) -> Result<(), String> {
        if self.pending.is_none() {
            self.next_id = self
                .next_id
                .checked_add(1)
                .ok_or("manual preparation request IDs exhausted")?;
            self.pending = Some(Pending {
                id: self.next_id,
                deadline: now.saturating_add(PREPARATION_DURATION),
                revision,
            });
        }
        Ok(())
    }

    pub(super) fn cancel(&mut self, id: u64) -> Result<(), String> {
        if self.pending.is_some_and(|pending| pending.id == id) {
            self.clear();
            Ok(())
        } else {
            Err("this break preparation is no longer pending".into())
        }
    }

    pub(super) fn clear(&mut self) {
        self.pending = None;
    }

    pub(super) fn reconcile(&mut self, eligible_revision: Option<(u64, u64)>) {
        if self
            .pending
            .is_some_and(|pending| Some(pending.revision) != eligible_revision)
        {
            self.clear();
        }
    }

    /// Consume before attempting presentation, including failed presentation.
    pub(super) fn take_due(&mut self, now: Duration) -> bool {
        if self.pending.is_some_and(|pending| now >= pending.deadline) {
            self.clear();
            true
        } else {
            false
        }
    }

    pub(super) fn status(&self, now: Duration) -> ManualPreparationStatus {
        self.pending
            .map_or_else(ManualPreparationStatus::default, |pending| {
                ManualPreparationStatus {
                    request_id: Some(pending.id),
                    remaining_milliseconds: Some(
                        u64::try_from(pending.deadline.saturating_sub(now).as_millis())
                            .unwrap_or(u64::MAX),
                    ),
                }
            })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn ms(value: u64) -> Duration {
        Duration::from_millis(value)
    }

    #[test]
    fn exhausted_request_ids_do_not_alias_an_old_request() {
        let mut state = ManualPreparation {
            next_id: u64::MAX,
            ..ManualPreparation::default()
        };
        assert!(state.begin(ms(0), (1, 2)).is_err());
        assert_eq!(state.status(ms(0)).request_id, None);
    }

    #[test]
    fn deadline_duplicate_and_presentation_failure_are_one_shot() {
        let mut state = ManualPreparation::default();
        state.begin(ms(0), (1, 2)).unwrap();
        let id = state.status(ms(0)).request_id;
        state.begin(ms(2000), (1, 2)).unwrap();
        assert_eq!(state.status(ms(2000)).request_id, id);
        assert_eq!(state.status(ms(2999)).remaining_milliseconds, Some(1));
        assert!(!state.take_due(ms(2999)));
        assert!(state.take_due(ms(3000)));
        // Failure to present cannot requeue the consumed request.
        assert!(!state.take_due(ms(3001)));
        assert!(state.cancel(id.unwrap()).is_err());
    }

    #[test]
    fn cancel_is_scoped_and_preserves_newer_request() {
        let mut state = ManualPreparation::default();
        state.begin(ms(0), (1, 2)).unwrap();
        let old = state.status(ms(0)).request_id.unwrap();
        state.cancel(old).unwrap();
        state.begin(ms(0), (1, 2)).unwrap();
        let new = state.status(ms(0)).request_id.unwrap();
        assert!(state.cancel(old).is_err());
        state.cancel(new).unwrap();
        assert!(!state.take_due(ms(3000)));
        state.begin(ms(0), (1, 2)).unwrap();
        state
            .cancel(state.status(ms(2999)).request_id.unwrap())
            .unwrap();
        assert!(!state.take_due(ms(3000)));
    }

    #[test]
    fn revisions_runtime_and_overlay_eligibility_invalidate() {
        for revision in [Some((2, 2)), Some((1, 3)), None] {
            let mut state = ManualPreparation::default();
            state.begin(ms(0), (1, 2)).unwrap();
            state.reconcile(revision);
            assert!(!state.take_due(ms(3000)));
        }
    }

    #[test]
    fn read_only_visibility_and_restart() {
        let mut state = ManualPreparation::default();
        state.begin(ms(0), (1, 2)).unwrap();
        let before = state.status(ms(1000));
        assert_eq!(state.status(ms(1000)), before);
        state.reconcile(Some((1, 2)));
        assert!(state.take_due(ms(3000)));
        assert_eq!(ManualPreparation::default().status(ms(0)).request_id, None);
    }

    #[test]
    fn discontinuity_and_superseding_action_clear_without_execution() {
        let mut state = ManualPreparation::default();
        state.begin(ms(0), (1, 2)).unwrap();
        state.clear();
        assert!(!state.take_due(ms(60_000)));
    }
}
