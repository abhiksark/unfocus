use serde::Serialize;
#[cfg(test)]
use std::sync::mpsc::TryRecvError;
use std::{
    sync::{
        mpsc::{self, Receiver, SyncSender, TrySendError},
        Arc, Mutex, Weak,
    },
    time::Duration,
};

const UPDATE_CHANNEL_CAPACITY: usize = 1;
const MILLISECONDS_PER_MINUTE: u64 = 60_000;

// Stage B and C own the runtime producers for paused, stopped, and activity
// states. Keeping them in the shared model now prevents platform menus from
// inventing incompatible representations later.
#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum TrayPhase {
    Working,
    Break,
    Paused,
    Stopped,
    Unavailable,
}

#[allow(dead_code)]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(tag = "kind", rename_all = "camelCase")]
pub(crate) enum TrayActivity {
    Absent,
    Unknown,
    Tracked { seconds: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct TraySnapshot {
    pub(crate) phase: TrayPhase,
    pub(crate) remaining_milliseconds: Option<u64>,
    pub(crate) pause_expires_in_milliseconds: Option<u64>,
    pub(crate) overlay_active: bool,
    pub(crate) activity: TrayActivity,
    pub(crate) settings_revision: u64,
    pub(crate) state_revision: u64,
}

impl TraySnapshot {
    pub(crate) fn timer(
        phase: TrayPhase,
        remaining: Duration,
        overlay_active: bool,
        settings_revision: u64,
        state_revision: u64,
    ) -> Self {
        debug_assert!(matches!(phase, TrayPhase::Working | TrayPhase::Break));
        Self {
            phase,
            remaining_milliseconds: Some(duration_milliseconds(remaining)),
            pause_expires_in_milliseconds: None,
            overlay_active,
            activity: TrayActivity::Absent,
            settings_revision,
            state_revision,
        }
    }

    fn unavailable() -> Self {
        Self {
            phase: TrayPhase::Unavailable,
            remaining_milliseconds: None,
            pause_expires_in_milliseconds: None,
            overlay_active: false,
            activity: TrayActivity::Absent,
            settings_revision: 0,
            state_revision: 0,
        }
    }

    pub(crate) fn presentation(&self) -> TrayPresentation {
        TrayPresentation {
            status: format_status(self),
            activity: format_activity(self.activity),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TrayPresentation {
    pub(crate) status: String,
    pub(crate) activity: Option<String>,
}

fn duration_milliseconds(duration: Duration) -> u64 {
    duration.as_millis().try_into().unwrap_or(u64::MAX)
}

fn rounded_minutes(milliseconds: u64) -> u64 {
    milliseconds / MILLISECONDS_PER_MINUTE
        + u64::from(!milliseconds.is_multiple_of(MILLISECONDS_PER_MINUTE))
}

fn countdown_label(milliseconds: u64) -> String {
    if milliseconds < MILLISECONDS_PER_MINUTE {
        "less than 1 min".into()
    } else {
        let minutes = rounded_minutes(milliseconds);
        if minutes == 1 {
            "1 min".into()
        } else {
            format!("{minutes} min")
        }
    }
}

fn format_status(snapshot: &TraySnapshot) -> String {
    match snapshot.phase {
        TrayPhase::Working => snapshot.remaining_milliseconds.map_or_else(
            || "Status unavailable".into(),
            |milliseconds| format!("Working · break in {}", countdown_label(milliseconds)),
        ),
        TrayPhase::Break => "Break in progress".into(),
        TrayPhase::Paused => snapshot.pause_expires_in_milliseconds.map_or_else(
            || "Paused".into(),
            |milliseconds| format!("Paused · resumes in {}", countdown_label(milliseconds)),
        ),
        TrayPhase::Stopped => "Workday ended".into(),
        TrayPhase::Unavailable => "Status unavailable".into(),
    }
}

fn format_activity(activity: TrayActivity) -> Option<String> {
    let TrayActivity::Tracked { seconds } = activity else {
        return None;
    };

    let minutes = seconds / 60;
    let hours = minutes / 60;
    let remaining_minutes = minutes % 60;
    let duration = match (hours, remaining_minutes) {
        (0, minutes) => format!("{minutes}m"),
        (hours, 0) => format!("{hours}h"),
        (hours, minutes) => format!("{hours}h {minutes}m"),
    };
    Some(format!("Today · {duration} tracked"))
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ActivityUpdateKey {
    Absent,
    Unknown,
    Tracked(Option<String>),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct UpdateKey {
    presentation: TrayPresentation,
    activity: ActivityUpdateKey,
    overlay_active: bool,
    settings_revision: u64,
    state_revision: u64,
}

impl From<&TraySnapshot> for UpdateKey {
    fn from(snapshot: &TraySnapshot) -> Self {
        let presentation = snapshot.presentation();
        let activity = match snapshot.activity {
            TrayActivity::Absent => ActivityUpdateKey::Absent,
            TrayActivity::Unknown => ActivityUpdateKey::Unknown,
            TrayActivity::Tracked { .. } => {
                ActivityUpdateKey::Tracked(presentation.activity.clone())
            }
        };
        Self {
            presentation,
            activity,
            overlay_active: snapshot.overlay_active,
            settings_revision: snapshot.settings_revision,
            state_revision: snapshot.state_revision,
        }
    }
}

#[derive(Debug)]
struct TrayStatusState {
    current: TraySnapshot,
    last_update: UpdateKey,
    subscribers: Vec<SyncSender<()>>,
}

#[derive(Debug, Clone)]
pub(crate) struct TrayStatus {
    inner: Arc<Mutex<TrayStatusState>>,
}

impl Default for TrayStatus {
    fn default() -> Self {
        let current = TraySnapshot::unavailable();
        let last_update = UpdateKey::from(&current);
        Self {
            inner: Arc::new(Mutex::new(TrayStatusState {
                current,
                last_update,
                subscribers: Vec::new(),
            })),
        }
    }
}

impl TrayStatus {
    fn state(&self) -> std::sync::MutexGuard<'_, TrayStatusState> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    pub(crate) fn current(&self) -> TraySnapshot {
        self.state().current.clone()
    }

    pub(crate) fn publish(&self, snapshot: TraySnapshot) {
        let update = UpdateKey::from(&snapshot);
        let mut state = self.state();
        state.current = snapshot;
        if state.last_update == update {
            return;
        }

        state.last_update = update;
        state
            .subscribers
            .retain(|subscriber| match subscriber.try_send(()) {
                Ok(()) | Err(TrySendError::Full(())) => true,
                Err(TrySendError::Disconnected(())) => false,
            });
    }

    pub(crate) fn subscribe(&self) -> TrayStatusSubscription {
        let (sender, receiver) = mpsc::sync_channel(UPDATE_CHANNEL_CAPACITY);
        let _ = sender.try_send(());
        self.state().subscribers.push(sender);
        TrayStatusSubscription {
            status: Arc::downgrade(&self.inner),
            receiver,
        }
    }
}

pub(crate) struct TrayStatusSubscription {
    status: Weak<Mutex<TrayStatusState>>,
    receiver: Receiver<()>,
}

impl TrayStatusSubscription {
    pub(crate) fn recv(&self) -> Result<(), mpsc::RecvError> {
        self.receiver.recv()
    }

    pub(crate) fn current(&self) -> Option<TraySnapshot> {
        self.status.upgrade().map(|status| {
            status
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .current
                .clone()
        })
    }

    #[cfg(test)]
    fn try_recv(&self) -> Result<(), TryRecvError> {
        self.receiver.try_recv()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn snapshot(phase: TrayPhase, remaining_milliseconds: Option<u64>) -> TraySnapshot {
        TraySnapshot {
            phase,
            remaining_milliseconds,
            pause_expires_in_milliseconds: None,
            overlay_active: false,
            activity: TrayActivity::Absent,
            settings_revision: 3,
            state_revision: 7,
        }
    }

    #[test]
    fn formats_every_reminder_phase_without_a_live_clock() {
        for (snapshot, expected) in [
            (
                snapshot(TrayPhase::Working, Some(12 * MILLISECONDS_PER_MINUTE)),
                "Working · break in 12 min",
            ),
            (
                snapshot(TrayPhase::Working, Some(11 * MILLISECONDS_PER_MINUTE + 1)),
                "Working · break in 12 min",
            ),
            (
                snapshot(TrayPhase::Working, Some(MILLISECONDS_PER_MINUTE)),
                "Working · break in 1 min",
            ),
            (
                snapshot(TrayPhase::Working, Some(MILLISECONDS_PER_MINUTE - 1)),
                "Working · break in less than 1 min",
            ),
            (
                snapshot(TrayPhase::Working, Some(0)),
                "Working · break in less than 1 min",
            ),
            (
                snapshot(TrayPhase::Working, Some(120 * MILLISECONDS_PER_MINUTE)),
                "Working · break in 120 min",
            ),
            (snapshot(TrayPhase::Working, None), "Status unavailable"),
            (
                snapshot(TrayPhase::Break, Some(20_000)),
                "Break in progress",
            ),
            (snapshot(TrayPhase::Paused, None), "Paused"),
            (snapshot(TrayPhase::Stopped, None), "Workday ended"),
            (snapshot(TrayPhase::Unavailable, None), "Status unavailable"),
        ] {
            assert_eq!(snapshot.presentation().status, expected);
        }
    }

    #[test]
    fn formats_pause_expiry_with_the_same_ceiling_rule() {
        let mut paused = snapshot(TrayPhase::Paused, None);
        paused.pause_expires_in_milliseconds = Some(30 * MILLISECONDS_PER_MINUTE + 1);
        assert_eq!(paused.presentation().status, "Paused · resumes in 31 min");

        paused.pause_expires_in_milliseconds = Some(1);
        assert_eq!(
            paused.presentation().status,
            "Paused · resumes in less than 1 min"
        );
    }

    #[test]
    fn activity_distinguishes_absent_unknown_zero_and_tracked_time() {
        assert_eq!(format_activity(TrayActivity::Absent), None);
        assert_eq!(format_activity(TrayActivity::Unknown), None);
        assert_eq!(
            format_activity(TrayActivity::Tracked { seconds: 0 }),
            Some("Today · 0m tracked".into())
        );
        assert_eq!(
            format_activity(TrayActivity::Tracked {
                seconds: 3 * 60 * 60 + 42 * 60 + 59,
            }),
            Some("Today · 3h 42m tracked".into())
        );
        assert_eq!(
            format_activity(TrayActivity::Tracked {
                seconds: 4 * 60 * 60,
            }),
            Some("Today · 4h tracked".into())
        );
    }

    #[test]
    fn snapshot_is_explicitly_inspectable() {
        let serialized = serde_json::to_value(TraySnapshot::timer(
            TrayPhase::Working,
            Duration::from_secs(90),
            true,
            4,
            9,
        ))
        .unwrap();

        assert_eq!(serialized["phase"], "working");
        assert_eq!(serialized["remainingMilliseconds"], 90_000);
        assert_eq!(serialized["overlayActive"], true);
        assert_eq!(serialized["activity"]["kind"], "absent");
        assert_eq!(serialized["settingsRevision"], 4);
        assert_eq!(serialized["stateRevision"], 9);
    }

    #[test]
    fn updates_are_coalesced_at_minute_and_state_boundaries() {
        let status = TrayStatus::default();
        let subscription = status.subscribe();
        assert_eq!(subscription.recv(), Ok(()));

        status.publish(TraySnapshot::timer(
            TrayPhase::Working,
            Duration::from_secs(12 * 60),
            false,
            1,
            1,
        ));
        assert_eq!(subscription.recv(), Ok(()));

        status.publish(TraySnapshot::timer(
            TrayPhase::Working,
            Duration::from_secs(11 * 60 + 1),
            false,
            1,
            1,
        ));
        assert_eq!(subscription.try_recv(), Err(TryRecvError::Empty));

        status.publish(TraySnapshot::timer(
            TrayPhase::Working,
            Duration::from_secs(11 * 60),
            false,
            1,
            1,
        ));
        assert_eq!(subscription.recv(), Ok(()));

        status.publish(TraySnapshot::timer(
            TrayPhase::Working,
            Duration::from_secs(11 * 60),
            true,
            1,
            1,
        ));
        assert_eq!(subscription.recv(), Ok(()));

        status.publish(TraySnapshot::timer(
            TrayPhase::Working,
            Duration::from_secs(11 * 60),
            true,
            2,
            2,
        ));
        assert_eq!(subscription.recv(), Ok(()));
    }

    #[test]
    fn a_full_notification_channel_keeps_the_latest_snapshot() {
        let status = TrayStatus::default();
        let subscription = status.subscribe();

        for minute in (1..=120).rev() {
            status.publish(TraySnapshot::timer(
                TrayPhase::Working,
                Duration::from_secs(minute * 60),
                false,
                1,
                1,
            ));
        }

        assert_eq!(subscription.recv(), Ok(()));
        assert_eq!(
            subscription.current().unwrap().presentation().status,
            "Working · break in 1 min"
        );
        assert_eq!(subscription.try_recv(), Err(TryRecvError::Empty));
    }
}
