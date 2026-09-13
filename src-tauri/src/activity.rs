//! Continuous activity and AFK segmentation from idle-probe samples.
//!
//! Privacy boundary: only presence of input (OS idle seconds) is used. No
//! keylogging, mouse paths, app titles, or window content.
//!
//! Segment history is stored locally next to app config as JSON. Writes are
//! atomic (temp file + replace). Probe or write failures never mutate the
//! reminder timer.
//!
//! Segments that age out of the rolling window are archived to cold storage
//! (`activity_archive`) before being dropped from the hot set. A failed
//! archive write leaves a segment hot so it is retried on a later prune;
//! nothing is lost to a failed write or a crash between the two.

use crate::{
    activity_archive::{archive_segments, prune_chunks, read_range, ARCHIVE_BLOCK_MS},
    storage_recovery::{
        canonical_bytes_unchanged, create_new_file_with_permissions, existing_file_permissions,
        quarantine_invalid_hard_link, replace_file_atomically, LoadFailure, LocalSnapshot,
        StorageDiagnostic, StorageFailureCategory, StorageLoadHealth,
    },
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

/// No keyboard/mouse input for this long means AFK (industry default ~5 min).
pub(crate) const AFK_THRESHOLD_SECONDS: u64 = 5 * 60;
/// Continuous active stretch at least this long counts as a deep block.
pub(crate) const DEEP_BLOCK_MIN_SECONDS: u64 = 25 * 60;
/// Long continuous active threshold for break presentation adaptation (#61).
pub(crate) const LONG_ACTIVE_SECONDS: u64 = DEEP_BLOCK_MIN_SECONDS;
/// Long AFK threshold for break presentation adaptation (#61).
pub(crate) const LONG_AFK_SECONDS: u64 = AFK_THRESHOLD_SECONDS;
/// Recent AFK still influences presentation this long after it ends (#61).
pub(crate) const RECENT_AFK_GRACE_SECONDS: u64 = 2 * 60;
/// Rolling observation window for the dashboard strip and totals.
pub(crate) const ACTIVITY_WINDOW_SECONDS: u64 = 24 * 60 * 60;
/// How long raw history is kept on disk, distinct from the live window.
/// Whole chunks are deleted only once fully expired, so effective retention
/// runs between this and this plus one archive block.
const HISTORY_RETENTION_SECONDS: u64 = 90 * 24 * 60 * 60;
/// Half-hour buckets across the rolling window (48 × 30 min = 24 h).
pub(crate) const STRIP_BUCKET_COUNT: usize = 48;
/// Minimum spacing between background history writes while a segment extends.
const PERSIST_MIN_INTERVAL_MS: u64 = 30_000;
const HISTORY_FILE_NAME: &str = "activity-history.json";
pub(crate) const HISTORY_SCHEMA_VERSION: u32 = 1;
const MILLIS_PER_SECOND: u64 = 1_000;
/// Upper bound on buckets per query. A 30-day grid at hourly resolution needs
/// 720 buckets, so 721 boundaries.
const MAX_RANGE_BUCKETS: usize = 1_024;

static HISTORY_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static TEST_HISTORY_PERSIST_FAILURE: Mutex<Option<PathBuf>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ActivityKind {
    Active,
    Afk,
    Unknown,
}

/// Runtime lifecycle of the idle probe used for activity classification.
/// This is serialized in summaries only and is never persisted to history.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ActivityProbeStatus {
    Pending,
    Available,
    Failed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum PersistedKind {
    Active,
    Afk,
}

impl PersistedKind {
    pub(crate) fn from_activity(kind: ActivityKind) -> Option<Self> {
        match kind {
            ActivityKind::Active => Some(Self::Active),
            ActivityKind::Afk => Some(Self::Afk),
            ActivityKind::Unknown => None,
        }
    }

    pub(crate) fn into_activity(self) -> ActivityKind {
        match self {
            Self::Active => ActivityKind::Active,
            Self::Afk => ActivityKind::Afk,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Segment {
    pub(crate) kind: ActivityKind,
    pub(crate) start_ms: u64,
    pub(crate) end_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersistedSegment {
    pub(crate) kind: PersistedKind,
    pub(crate) start_ms: u64,
    pub(crate) end_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct PersistedActivityHistory {
    pub(crate) version: u32,
    pub(crate) segments: Vec<PersistedSegment>,
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
    probe_status: ActivityProbeStatus,
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
            probe_status: ActivityProbeStatus::Pending,
        }
    }

    /// Replace live segments with a loaded history snapshot.
    ///
    /// The caller (the loader in `load_history`) has already
    /// archived-before-dropping anything aged out of the window; this must
    /// not re-drop anything itself, or a segment kept hot for a later
    /// archive retry (because the write failed at load time) would be
    /// silently destroyed here instead. Future wall-clock ends (clock skew)
    /// still clear history rather than inventing multi-day spans. Restored
    /// state is not treated as a live probe success.
    fn restore_segments(&mut self, segments: Vec<Segment>, now_ms: u64) {
        self.segments = segments;
        if self
            .segments
            .iter()
            .any(|segment| segment.end_ms > now_ms || segment.start_ms > now_ms)
        {
            self.segments.clear();
            self.last_sample_ms = None;
            self.last_kind = None;
            self.probe_status = ActivityProbeStatus::Pending;
            return;
        }
        self.last_sample_ms = self.segments.last().map(|segment| segment.end_ms);
        self.last_kind = self.segments.last().map(|segment| segment.kind);
        self.probe_status = ActivityProbeStatus::Pending;
    }

    /// Record one idle reading at `now_ms` (Unix epoch milliseconds).
    ///
    /// `idle_seconds = None` means the probe failed; classification freezes and
    /// does not invent active or AFK time (fail open for the timer elsewhere).
    ///
    /// This never drops or archives aged-out segments itself: archiving is
    /// I/O and must succeed before a segment leaves the hot set, so that
    /// decision belongs to the caller (see `archivable_segments` /
    /// `drop_archived`, driven by `ActivityTrackerHandle`).
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
                self.probe_status = ActivityProbeStatus::Available;
                ActivityKind::Afk
            }
            Some(_) => {
                self.probe_status = ActivityProbeStatus::Available;
                ActivityKind::Active
            }
            None => {
                self.probe_status = ActivityProbeStatus::Failed;
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

    fn window_cutoff(&self, now_ms: u64) -> u64 {
        let window_ms = self.window_seconds.saturating_mul(MILLIS_PER_SECOND);
        now_ms.saturating_sub(window_ms)
    }

    /// Segments that have fully aged out of the rolling window: exactly the
    /// segments a drop would remove, `end_ms <= cutoff`. Read-only; a
    /// segment straddling the cutoff is never included and never truncated,
    /// it stays hot and whole.
    fn archivable_segments(&self, now_ms: u64) -> Vec<Segment> {
        let cutoff = self.window_cutoff(now_ms);
        self.segments
            .iter()
            .filter(|segment| segment.end_ms <= cutoff)
            .copied()
            .collect()
    }

    /// Drop segments already confirmed archived. Mirrors
    /// `archivable_segments`'s predicate exactly, so callers must archive
    /// first; a segment straddling the cutoff is never truncated.
    fn drop_archived(&mut self, now_ms: u64) {
        let cutoff = self.window_cutoff(now_ms);
        self.segments.retain(|segment| segment.end_ms > cutoff);
    }

    fn to_persisted(&self) -> PersistedActivityHistory {
        let segments = self
            .segments
            .iter()
            .filter_map(|segment| {
                let kind = PersistedKind::from_activity(segment.kind)?;
                if segment.end_ms < segment.start_ms {
                    return None;
                }
                Some(PersistedSegment {
                    kind,
                    start_ms: segment.start_ms,
                    end_ms: segment.end_ms,
                })
            })
            .collect();
        PersistedActivityHistory {
            version: HISTORY_SCHEMA_VERSION,
            segments,
        }
    }

    /// Snapshot for pure break-presentation adaptation (issue #61).
    pub(crate) fn presentation_context(&self, now_ms: u64) -> ActivityPresentationContext {
        if self.segments.is_empty() {
            return ActivityPresentationContext {
                history_available: false,
                continuous_active_seconds: 0,
                recent_afk_seconds: 0,
            };
        }

        let continuous_active_seconds = self
            .segments
            .last()
            .filter(|segment| segment.kind == ActivityKind::Active)
            .map(|segment| {
                let end = now_ms.max(segment.end_ms);
                end.saturating_sub(segment.start_ms) / MILLIS_PER_SECOND
            })
            .unwrap_or(0);

        let grace_ms = RECENT_AFK_GRACE_SECONDS.saturating_mul(MILLIS_PER_SECOND);
        let mut recent_afk_seconds = 0_u64;
        for segment in self.segments.iter().rev() {
            if segment.kind != ActivityKind::Afk {
                continue;
            }
            let end = if self.segments.last().is_some_and(|last| {
                last.start_ms == segment.start_ms && last.end_ms == segment.end_ms
            }) {
                now_ms.max(segment.end_ms)
            } else {
                segment.end_ms
            };
            if now_ms.saturating_sub(end) > grace_ms {
                break;
            }
            recent_afk_seconds = end.saturating_sub(segment.start_ms) / MILLIS_PER_SECOND;
            break;
        }

        ActivityPresentationContext {
            history_available: true,
            continuous_active_seconds,
            recent_afk_seconds,
        }
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
            probe_status: self.probe_status,
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

/// Presence signals used only for break presentation (never the pure clock).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ActivityPresentationContext {
    pub(crate) history_available: bool,
    pub(crate) continuous_active_seconds: u64,
    pub(crate) recent_afk_seconds: u64,
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
    pub(crate) probe_status: ActivityProbeStatus,
    pub(crate) strip: Vec<StripBucket>,
}

/// One bucket of summed occupancy for `get_activity_range`. The frontend
/// computes bucket boundaries (local days/hours); Rust only sums between
/// them, see the module-level contract on `get_activity_range`.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct RangeBucket {
    pub(crate) active_ms: u64,
    pub(crate) afk_ms: u64,
    pub(crate) longest_active_ms: u64,
}

/// Sum active/afk overlap between `[start, end)` and `segments`, using the
/// same clamped-overlap arithmetic as `ActivityTracker::strip_buckets`.
fn bucket_occupancy(segments: &[Segment], start: u64, end: u64) -> RangeBucket {
    let mut active_ms = 0_u64;
    let mut afk_ms = 0_u64;
    let mut longest_active_ms = 0_u64;
    for segment in segments {
        let overlap_start = segment.start_ms.max(start);
        let overlap_end = segment.end_ms.min(end);
        if overlap_end <= overlap_start {
            continue;
        }
        let overlap = overlap_end - overlap_start;
        match segment.kind {
            ActivityKind::Active => {
                active_ms = active_ms.saturating_add(overlap);
                longest_active_ms = longest_active_ms.max(overlap);
            }
            ActivityKind::Afk => afk_ms = afk_ms.saturating_add(overlap),
            ActivityKind::Unknown => {}
        }
    }
    RangeBucket {
        active_ms,
        afk_ms,
        longest_active_ms,
    }
}

#[derive(Debug)]
struct ActivityTrackerState {
    tracker: ActivityTracker,
    path: PathBuf,
    /// Directory `path` lives in; archives live beside the hot file in the
    /// same directory.
    config_dir: PathBuf,
    last_persisted_at_ms: Option<u64>,
    last_retention_pruned_at_ms: Option<u64>,
}

#[derive(Debug)]
enum ActivityStorageState {
    Available(ActivityTrackerState),
    Unavailable(LoadFailure),
}

#[derive(Debug, Clone)]
pub(crate) struct ActivityTrackerHandle {
    inner: Arc<Mutex<ActivityStorageState>>,
    path: Arc<PathBuf>,
    config_dir: Arc<PathBuf>,
    recovery: Arc<Mutex<()>>,
}

impl ActivityTrackerHandle {
    /// Initialize against the canonical app-config path. A load failure is
    /// retained as unavailable state; it is never replaced with an empty,
    /// cwd-relative tracker.
    pub(crate) fn initialize(config_dir: &Path) -> Self {
        Self::initialize_at(config_dir, epoch_ms(SystemTime::now()))
    }

    fn initialize_at(config_dir: &Path, now_ms: u64) -> Self {
        let path = config_dir.join(HISTORY_FILE_NAME);
        let state = match load_activity_state(&path, config_dir, now_ms) {
            Ok(state) => ActivityStorageState::Available(state),
            Err(failure) => ActivityStorageState::Unavailable(failure),
        };
        Self {
            inner: Arc::new(Mutex::new(state)),
            path: Arc::new(path),
            config_dir: Arc::new(config_dir.to_path_buf()),
            recovery: Arc::new(Mutex::new(())),
        }
    }

    #[cfg(test)]
    fn load_at(config_dir: &Path, now_ms: u64) -> io::Result<Self> {
        let handle = Self::initialize_at(config_dir, now_ms);
        let available = handle
            .inner
            .lock()
            .map_err(|_| io::Error::other("lock poisoned"))?
            .as_available()
            .is_some();
        if available {
            Ok(handle)
        } else {
            Err(io::Error::other("activity history unavailable"))
        }
    }

    #[cfg(test)]
    fn new_with_path(tracker: ActivityTracker, path: PathBuf) -> Self {
        let config_dir = path.parent().map(Path::to_path_buf).unwrap_or_default();
        Self {
            inner: Arc::new(Mutex::new(ActivityStorageState::Available(
                ActivityTrackerState {
                    tracker,
                    path: path.clone(),
                    config_dir: config_dir.clone(),
                    last_persisted_at_ms: None,
                    last_retention_pruned_at_ms: None,
                },
            ))),
            path: Arc::new(path),
            config_dir: Arc::new(config_dir),
            recovery: Arc::new(Mutex::new(())),
        }
    }

    pub(crate) fn observe(&self, now_ms: u64, idle_seconds: Option<u64>) {
        let Ok(mut storage) = self.inner.lock() else {
            return;
        };
        let ActivityStorageState::Available(state) = &mut *storage else {
            return;
        };
        let previous_kind = state.tracker.last_kind;
        state.tracker.observe(now_ms, idle_seconds);
        let kind_changed = state.tracker.last_kind != previous_kind;

        let due = match state.last_persisted_at_ms {
            None => true,
            Some(previous) => now_ms.saturating_sub(previous) >= PERSIST_MIN_INTERVAL_MS,
        };
        let retention_due = match state.last_retention_pruned_at_ms {
            None => true,
            Some(previous) => {
                now_ms < previous || now_ms.saturating_sub(previous) >= PERSIST_MIN_INTERVAL_MS
            }
        };

        // Flush before prune: a segment leaves the hot set only once it is
        // safely archived. A write failure leaves it hot for a later retry;
        // nothing is lost, and this never blocks or mutates the timer. Only
        // attempt the archive when the persist throttle below would already
        // do work (a kind change or the throttle interval elapsing): under a
        // persistent archive failure (full disk, blocked chunk path), the
        // same segments would otherwise be retried, and rewritten in full,
        // on every observation, which is exactly the unbounded hot-set
        // growth this design exists to prevent. Segments simply wait for the
        // next throttled attempt; nothing already archived is re-attempted,
        // and nothing hot is ever lost.
        let archivable = state.tracker.archivable_segments(now_ms);
        if !archivable.is_empty() && (kind_changed || due) {
            match archive_segments(&state.config_dir, &archivable) {
                Ok(()) => state.tracker.drop_archived(now_ms),
                Err(error) => {
                    eprintln!("could not archive activity history: {error}; will retry");
                }
            }
        }

        if retention_due {
            prune_expired_archives(&state.config_dir, now_ms);
            state.last_retention_pruned_at_ms = Some(now_ms);
        }

        if !kind_changed && !due {
            return;
        }
        if let Err(error) = persist_history(&state.path, &state.tracker.to_persisted()) {
            eprintln!("could not persist activity history: {error}");
            return;
        }
        state.last_persisted_at_ms = Some(now_ms);
    }

    pub(crate) fn snapshot(&self, now_ms: u64) -> LocalSnapshot<ActivitySummary> {
        let Ok(storage) = self.inner.lock() else {
            return LocalSnapshot::unavailable(StorageFailureCategory::Read);
        };
        match &*storage {
            ActivityStorageState::Available(state) => {
                LocalSnapshot::available(state.tracker.summary(now_ms))
            }
            ActivityStorageState::Unavailable(failure) => {
                LocalSnapshot::unavailable(failure.category)
            }
        }
    }

    #[cfg(test)]
    fn summary(&self, now_ms: u64) -> ActivitySummary {
        self.snapshot(now_ms)
            .data
            .expect("test tracker should be available")
    }

    pub(crate) fn presentation_context(&self, now_ms: u64) -> ActivityPresentationContext {
        self.inner
            .lock()
            .ok()
            .and_then(|storage| match &*storage {
                ActivityStorageState::Available(state) => {
                    Some(state.tracker.presentation_context(now_ms))
                }
                ActivityStorageState::Unavailable(_) => None,
            })
            .unwrap_or(ActivityPresentationContext {
                history_available: false,
                continuous_active_seconds: 0,
                recent_afk_seconds: 0,
            })
    }

    pub(crate) fn diagnostics(&self) -> StorageDiagnostic {
        let Ok(storage) = self.inner.lock() else {
            return LoadFailure::read("activity history state lock is poisoned").diagnostic();
        };
        match &*storage {
            ActivityStorageState::Available(_) => StorageDiagnostic::available(),
            ActivityStorageState::Unavailable(failure) => failure.diagnostic(),
        }
    }

    pub(crate) fn retry_load(&self, now_ms: u64) -> StorageLoadHealth {
        let Ok(_recovery) = self.recovery.lock() else {
            return StorageLoadHealth::unavailable(StorageFailureCategory::Read);
        };
        if self.is_available() {
            return StorageLoadHealth::available();
        }
        self.retry_load_locked(now_ms)
    }

    pub(crate) fn start_new_after_invalid(&self, now_ms: u64) -> Result<StorageLoadHealth, String> {
        let _recovery = self
            .recovery
            .lock()
            .map_err(|_| "activity history recovery is unavailable".to_owned())?;
        let category = self.failure_category()?;
        if category != StorageFailureCategory::Invalid {
            return Err("start-new is only available for invalid activity history".into());
        }

        let contents = fs::read(&*self.path).map_err(|error| {
            self.publish_failure(LoadFailure::read(format!(
                "could not re-read {} before recovery: {error}",
                self.path.display()
            )));
            "activity history could not be read; retry is still available".to_owned()
        })?;

        if history_from_bytes(&contents, now_ms).is_ok() {
            return Ok(self.retry_load_locked(now_ms));
        }

        quarantine_invalid_hard_link(&self.path, &contents).map_err(|error| {
            self.publish_failure(LoadFailure::invalid(format!(
                "could not preserve invalid {}: {error}",
                self.path.display()
            )));
            "invalid activity history could not be preserved".to_owned()
        })?;
        let empty = PersistedActivityHistory {
            version: HISTORY_SCHEMA_VERSION,
            segments: Vec::new(),
        };
        let temp_path = prepare_history_file(&self.path, &empty).map_err(|error| {
            self.publish_failure(LoadFailure::invalid(format!(
                "invalid {} was preserved, but a new history could not be prepared: {error}",
                self.path.display()
            )));
            "a new activity history could not be started".to_owned()
        })?;
        let candidate = new_activity_state(&self.path, &self.config_dir, Vec::new(), now_ms);
        let unchanged = match canonical_bytes_unchanged(&self.path, &contents) {
            Ok(unchanged) => unchanged,
            Err(error) => {
                let _ = fs::remove_file(&temp_path);
                self.publish_failure(LoadFailure::read(format!(
                    "could not complete the final canonical recheck for {}: {error}",
                    self.path.display()
                )));
                return Err(
                    "activity history could not be rechecked; retry is still available".into(),
                );
            }
        };
        if !unchanged {
            let _ = fs::remove_file(&temp_path);
            return Err(
                "activity history changed while recovery was being prepared; retry to load it"
                    .to_owned(),
            );
        }
        let mut storage = self.inner.lock().map_err(|_| {
            let _ = fs::remove_file(&temp_path);
            "activity history recovery is unavailable".to_owned()
        })?;
        if let Err(error) = replace_file_atomically(&temp_path, &self.path) {
            let _ = fs::remove_file(&temp_path);
            return Err(format!(
                "a new activity history could not be started: {error}"
            ));
        }
        *storage = ActivityStorageState::Available(candidate);
        Ok(StorageLoadHealth::available())
    }

    fn retry_load_locked(&self, now_ms: u64) -> StorageLoadHealth {
        match load_activity_state(&self.path, &self.config_dir, now_ms) {
            Ok(candidate) => {
                if let Ok(mut storage) = self.inner.lock() {
                    *storage = ActivityStorageState::Available(candidate);
                    StorageLoadHealth::available()
                } else {
                    StorageLoadHealth::unavailable(StorageFailureCategory::Read)
                }
            }
            Err(failure) => {
                let health = failure.health();
                self.publish_failure(failure);
                health
            }
        }
    }

    fn is_available(&self) -> bool {
        self.inner
            .lock()
            .is_ok_and(|storage| matches!(&*storage, ActivityStorageState::Available(_)))
    }

    fn failure_category(&self) -> Result<StorageFailureCategory, String> {
        let storage = self
            .inner
            .lock()
            .map_err(|_| "activity history recovery is unavailable".to_owned())?;
        match &*storage {
            ActivityStorageState::Available(_) => {
                Err("activity history is already available".into())
            }
            ActivityStorageState::Unavailable(failure) => Ok(failure.category),
        }
    }

    fn publish_failure(&self, failure: LoadFailure) {
        if let Ok(mut storage) = self.inner.lock() {
            *storage = ActivityStorageState::Unavailable(failure);
        }
    }

    /// Sum occupancy into `boundaries.len() - 1` buckets, one per adjacent
    /// pair, from the union of the hot segments and the archive. Fails
    /// closed (a descriptive error, never a partial or empty success) when
    /// storage is unavailable, `boundaries` has fewer than two entries, is
    /// not strictly increasing, or would produce too many buckets.
    ///
    /// `boundaries` are caller-computed epoch milliseconds: Rust has no
    /// calendar and must not gain one, so it never buckets by local day or
    /// hour itself, only sums between the boundaries it is given.
    pub(crate) fn range(&self, boundaries: &[u64]) -> Result<Vec<RangeBucket>, String> {
        if boundaries.len() < 2 {
            return Err("boundaries must contain at least two values".into());
        }
        if boundaries.len() > MAX_RANGE_BUCKETS + 1 {
            return Err(format!(
                "boundaries must not exceed {} entries ({MAX_RANGE_BUCKETS} buckets)",
                MAX_RANGE_BUCKETS + 1
            ));
        }
        if !boundaries.windows(2).all(|pair| pair[1] > pair[0]) {
            return Err("boundaries must be strictly increasing".into());
        }

        let span_start = boundaries[0];
        let span_end = boundaries[boundaries.len() - 1];
        // Bound the span itself, not just the bucket count: two boundaries
        // spanning the full u64 range pass every check above (one bucket,
        // strictly increasing) yet would ask `read_range` to walk from the
        // first archive chunk key to the last, roughly seven billion reads.
        // Retained history can never exceed retention plus one archive block
        // (a chunk is only deleted once fully expired), so a wider span can
        // never hold real data and is rejected outright.
        let max_span_ms = HISTORY_RETENTION_SECONDS
            .saturating_mul(MILLIS_PER_SECOND)
            .saturating_add(ARCHIVE_BLOCK_MS);
        if span_end.saturating_sub(span_start) > max_span_ms {
            return Err(format!(
                "boundaries must not span more than {max_span_ms} ms of retained history"
            ));
        }
        let segments = self.segments_in_range(span_start, span_end)?;

        Ok(boundaries
            .windows(2)
            .map(|pair| bucket_occupancy(&segments, pair[0], pair[1]))
            .collect())
    }

    /// Every segment overlapping `[start_ms, end_ms)`, merged from the hot
    /// set and the archive.
    ///
    /// A segment is archived, then dropped from the hot set, under the same
    /// lock (see `observe`), so within one `observe` call it is never in
    /// both places at once. But this method takes its own snapshot of the
    /// hot set and only afterwards reads the archive from disk; a concurrent
    /// `observe` can archive-and-drop a segment in between those two steps,
    /// so the snapshot and archive read can both contain it. Sorting and
    /// deduping by exact value handles that race without heuristics.
    fn segments_in_range(&self, start_ms: u64, end_ms: u64) -> Result<Vec<Segment>, String> {
        let (hot, config_dir) = {
            let storage = self
                .inner
                .lock()
                .map_err(|_| "activity history is unavailable".to_owned())?;
            match &*storage {
                ActivityStorageState::Available(state) => {
                    (state.tracker.segments.clone(), state.config_dir.clone())
                }
                ActivityStorageState::Unavailable(_) => {
                    return Err("activity history is unavailable".into());
                }
            }
        };

        let mut combined: Vec<Segment> = hot
            .into_iter()
            .filter(|segment| segment.end_ms > start_ms && segment.start_ms < end_ms)
            .collect();
        combined.extend(read_range(&config_dir, start_ms, end_ms));
        combined.sort_by_key(|segment| (segment.start_ms, segment.end_ms, segment.kind as u8));
        combined.dedup();
        Ok(combined)
    }
}

impl ActivityStorageState {
    #[cfg(test)]
    fn as_available(&self) -> Option<&ActivityTrackerState> {
        match self {
            Self::Available(state) => Some(state),
            Self::Unavailable(_) => None,
        }
    }
}

fn new_activity_state(
    path: &Path,
    config_dir: &Path,
    segments: Vec<Segment>,
    now_ms: u64,
) -> ActivityTrackerState {
    let mut tracker = ActivityTracker::default();
    tracker.restore_segments(segments, now_ms);
    ActivityTrackerState {
        tracker,
        path: path.to_path_buf(),
        config_dir: config_dir.to_path_buf(),
        last_persisted_at_ms: None,
        last_retention_pruned_at_ms: Some(now_ms),
    }
}

fn load_activity_state(
    path: &Path,
    config_dir: &Path,
    now_ms: u64,
) -> Result<ActivityTrackerState, LoadFailure> {
    let segments = load_history(path, now_ms, ACTIVITY_WINDOW_SECONDS)?;
    prune_expired_archives(config_dir, now_ms);
    Ok(new_activity_state(path, config_dir, segments, now_ms))
}

fn prune_expired_archives(config_dir: &Path, now_ms: u64) {
    let retention_cutoff =
        now_ms.saturating_sub(HISTORY_RETENTION_SECONDS.saturating_mul(MILLIS_PER_SECOND));
    if let Err(error) = prune_chunks(config_dir, retention_cutoff) {
        eprintln!("could not prune expired activity archives: {error}");
    }
}

pub(crate) fn epoch_ms(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn segments_from_persisted(
    history: PersistedActivityHistory,
    now_ms: u64,
) -> Result<Vec<Segment>, ()> {
    if history.version != HISTORY_SCHEMA_VERSION {
        return Err(());
    }
    let mut segments = Vec::with_capacity(history.segments.len());
    for item in history.segments {
        if item.end_ms < item.start_ms {
            return Err(());
        }
        if item.end_ms > now_ms || item.start_ms > now_ms {
            // Future timestamps: treat as clock skew, drop whole history.
            return Err(());
        }
        segments.push(Segment {
            kind: item.kind.into_activity(),
            start_ms: item.start_ms,
            end_ms: item.end_ms,
        });
    }
    Ok(segments)
}

/// Archive-before-drop for segments loaded from the hot file, mirroring the
/// live prune contract in `ActivityTrackerHandle::observe` exactly: a
/// segment fully aged out of the window (`end_ms <= cutoff`) is archived
/// before it leaves the returned set. If the archive write fails, it stays
/// in the returned set instead of being discarded — a later live prune will
/// retry it, so a failed write here never destroys data. A segment
/// straddling the cutoff is always returned whole, never truncated.
///
/// This closes the one path the live prune can never cover: the app closed
/// for longer than the window and reopened, so the aged-out segments never
/// reach memory for a live prune to archive at all.
fn archive_before_drop(
    path: &Path,
    segments: Vec<Segment>,
    now_ms: u64,
    window_seconds: u64,
) -> Vec<Segment> {
    let window_ms = window_seconds.saturating_mul(MILLIS_PER_SECOND);
    let cutoff = now_ms.saturating_sub(window_ms);
    let (archivable, mut retained): (Vec<Segment>, Vec<Segment>) = segments
        .into_iter()
        .partition(|segment| segment.end_ms <= cutoff);

    if archivable.is_empty() {
        return retained;
    }

    let config_dir = path.parent().unwrap_or_else(|| Path::new(""));
    match archive_segments(config_dir, &archivable) {
        Ok(()) => retained,
        Err(error) => {
            eprintln!(
                "could not archive aged-out activity history on load: {error}; keeping it hot for a later retry"
            );
            retained.extend(archivable);
            retained.sort_by_key(|segment| segment.start_ms);
            retained
        }
    }
}

fn history_from_bytes(contents: &[u8], now_ms: u64) -> Result<Vec<Segment>, LoadFailure> {
    let history =
        serde_json::from_slice::<PersistedActivityHistory>(contents).map_err(|error| {
            LoadFailure::invalid(format!("activity history content is malformed: {error}"))
        })?;
    segments_from_persisted(history, now_ms)
        .map_err(|()| LoadFailure::invalid("activity history content is unsupported or invalid"))
}

fn load_history(
    path: &Path,
    now_ms: u64,
    window_seconds: u64,
) -> Result<Vec<Segment>, LoadFailure> {
    match fs::read(path) {
        Ok(contents) => history_from_bytes(&contents, now_ms)
            .map(|segments| archive_before_drop(path, segments, now_ms, window_seconds)),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(LoadFailure::read(format!(
            "could not read {}: {error}",
            path.display()
        ))),
    }
}

#[cfg(test)]
fn load_or_repair_history(
    path: &Path,
    now_ms: u64,
    window_seconds: u64,
) -> Result<Vec<Segment>, LoadFailure> {
    load_history(path, now_ms, window_seconds)
}

fn create_history_temp_file(path: &Path) -> io::Result<(PathBuf, File)> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "activity history path has no parent",
        )
    })?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "activity-history".into());
    let permissions = existing_file_permissions(path)?;

    for _ in 0..100 {
        let id = HISTORY_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
        let temp_path = parent.join(format!(".{name}.{}.{id}.tmp", std::process::id()));
        match create_new_file_with_permissions(&temp_path, permissions.as_ref()) {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate an activity history temporary file",
    ))
}

fn prepare_history_file(path: &Path, history: &PersistedActivityHistory) -> io::Result<PathBuf> {
    #[cfg(test)]
    if TEST_HISTORY_PERSIST_FAILURE.lock().is_ok_and(|mut target| {
        if target.as_deref() == Some(path) {
            target.take();
            true
        } else {
            false
        }
    }) {
        return Err(io::Error::other("injected activity history write failure"));
    }

    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "activity history path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;

    let serialized = serde_json::to_vec_pretty(history).map_err(io::Error::other)?;
    let (temp_path, mut temp_file) = create_history_temp_file(path)?;
    let write_result = temp_file
        .write_all(&serialized)
        .and_then(|()| temp_file.write_all(b"\n"))
        .and_then(|()| temp_file.sync_all());
    drop(temp_file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(temp_path)
}

pub(crate) fn persist_history(path: &Path, history: &PersistedActivityHistory) -> io::Result<()> {
    let temp_path = prepare_history_file(path, history)?;
    if let Err(error) = replace_file_atomically(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn get_today_activity(
    window: tauri::WebviewWindow,
    tracker: tauri::State<'_, ActivityTrackerHandle>,
) -> Result<LocalSnapshot<ActivitySummary>, String> {
    crate::authorize_main_caller(window.label())?;
    Ok(tracker.snapshot(epoch_ms(SystemTime::now())))
}

#[tauri::command]
pub(crate) async fn retry_activity_history(
    window: tauri::WebviewWindow,
    tracker: tauri::State<'_, ActivityTrackerHandle>,
) -> Result<StorageLoadHealth, String> {
    crate::authorize_main_caller(window.label())?;
    let tracker = tracker.inner().clone();
    tauri::async_runtime::spawn_blocking(move || tracker.retry_load(epoch_ms(SystemTime::now())))
        .await
        .map_err(|_| "activity history retry could not run".to_owned())
}

#[tauri::command]
pub(crate) async fn start_new_activity_history(
    window: tauri::WebviewWindow,
    tracker: tauri::State<'_, ActivityTrackerHandle>,
) -> Result<StorageLoadHealth, String> {
    crate::authorize_main_caller(window.label())?;
    let tracker = tracker.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        tracker.start_new_after_invalid(epoch_ms(SystemTime::now()))
    })
    .await
    .map_err(|_| "activity history recovery could not run".to_owned())?
}

/// Historical occupancy across an arbitrary span, bucketed by the caller.
///
/// Rust has no calendar and must not gain one: the eventual grid's rows are
/// local days and columns are local hours, which do not align to uniform
/// UTC steps in half-hour-offset zones or on daylight-saving days. The
/// frontend therefore computes `boundaries` (strictly increasing epoch
/// milliseconds) and this command only sums between them, over the union of
/// the hot segments and the archive. See `ActivityTrackerHandle::range` for
/// the validation contract and the hot/archive merge.
#[tauri::command]
pub(crate) fn get_activity_range(
    window: tauri::WebviewWindow,
    tracker: tauri::State<'_, ActivityTrackerHandle>,
    boundaries: Vec<u64>,
) -> Result<Vec<RangeBucket>, String> {
    crate::authorize_main_caller(window.label())?;
    tracker.range(&boundaries)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::activity_archive;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            for _ in 0..100 {
                let id = TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "unfocus-activity-tests-{}-{id}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("test directory should be created: {error}"),
                }
            }
            panic!("could not allocate a test activity directory")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

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
        assert_eq!(summary.probe_status, ActivityProbeStatus::Available);
        assert_eq!(summary.current_kind, Some(ActivityKind::Active));
        assert!(summary.active_seconds >= 120);
        assert!(summary.afk_seconds >= 60);
        assert_eq!(summary.strip.len(), 48);
        assert_eq!(summary.afk_threshold_seconds, 300);
        assert_eq!(summary.deep_block_min_seconds, 25 * 60);
    }

    #[test]
    fn new_tracker_serializes_pending_before_the_first_probe_result() {
        let summary = tracker().summary(1_700_000_000_000);

        assert_eq!(summary.probe_status, ActivityProbeStatus::Pending);
        assert_eq!(
            serde_json::to_value(&summary).expect("serialize summary")["probeStatus"],
            "pending"
        );
    }

    #[test]
    fn probe_failure_does_not_invent_active_or_afk_time_and_success_recovers() {
        let mut tracker = tracker();
        let t0 = 1_700_000_000_000_u64;
        tracker.observe(t0, Some(0));
        tracker.observe(t0 + 60_000, Some(10));
        let before = tracker.summary(t0 + 60_000);

        tracker.observe(t0 + 120_000, None);
        tracker.observe(t0 + 180_000, None);
        let failed = tracker.summary(t0 + 180_000);

        assert_eq!(failed.probe_status, ActivityProbeStatus::Failed);
        assert_eq!(failed.current_kind, Some(ActivityKind::Unknown));
        // Known classified time must not grow while the probe is dark.
        assert_eq!(failed.active_seconds, before.active_seconds);
        assert_eq!(failed.afk_seconds, before.afk_seconds);

        tracker.observe(t0 + 180_001, Some(0));
        assert_eq!(
            tracker.summary(t0 + 180_001).probe_status,
            ActivityProbeStatus::Available
        );
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
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let handle = ActivityTrackerHandle::new_with_path(tracker(), path);
        let t0 = 1_700_000_000_000_u64;
        handle.observe(t0, Some(0));
        handle.observe(t0 + 90_000, Some(2));
        let summary = handle.summary(t0 + 90_000);
        assert_eq!(summary.window_label, "Last 24 hours");
        assert!(summary.active_seconds >= 90);
        assert_eq!(summary.current_kind, Some(ActivityKind::Active));
    }

    #[test]
    fn restart_restores_segments_within_the_rolling_window() {
        let dir = TestDirectory::new();
        let t0 = 1_700_000_000_000_u64;
        let path = dir.path.join(HISTORY_FILE_NAME);

        let first = ActivityTrackerHandle::new_with_path(tracker(), path);
        first.observe(t0, Some(0));
        first.observe(t0 + 120_000, Some(0));
        first.observe(t0 + 180_000, Some(400));
        first.observe(t0 + 240_000, Some(450));

        let before = first.summary(t0 + 240_000);
        assert!(before.active_seconds >= 120);
        assert!(before.afk_seconds >= 60);

        // Load at the same synthetic clock the samples used so the rolling
        // window still contains them (wall clock now would prune 2023-era ids).
        let reloaded =
            ActivityTrackerHandle::load_at(&dir.path, t0 + 240_000).expect("history loads");
        let after = reloaded.summary(t0 + 240_000);
        assert_eq!(after.active_seconds, before.active_seconds);
        assert_eq!(after.afk_seconds, before.afk_seconds);
        // Restored history awaits a live probe result rather than reporting failure.
        assert_eq!(after.probe_status, ActivityProbeStatus::Pending);
    }

    #[test]
    fn failed_write_leaves_previous_complete_history() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let t0 = 1_700_000_000_000_u64;
        let original_history = PersistedActivityHistory {
            version: HISTORY_SCHEMA_VERSION,
            segments: vec![PersistedSegment {
                kind: PersistedKind::Active,
                start_ms: t0,
                end_ms: t0 + 60_000,
            }],
        };
        persist_history(&path, &original_history).expect("seed history");
        let original = fs::read(&path).expect("read seeded history");

        // Parent path is a file, so create_dir_all / temp create must fail and
        // must not rewrite the good history at `path`.
        let blocker = dir.path.join("not-a-directory");
        fs::write(&blocker, b"x").expect("blocker file");
        let nested = blocker.join(HISTORY_FILE_NAME);
        let result = persist_history(
            &nested,
            &PersistedActivityHistory {
                version: HISTORY_SCHEMA_VERSION,
                segments: vec![PersistedSegment {
                    kind: PersistedKind::Afk,
                    start_ms: t0,
                    end_ms: t0 + 999_000,
                }],
            },
        );
        assert!(result.is_err(), "persist under a file parent must fail");
        assert_eq!(fs::read(&path).expect("previous history remains"), original);

        // Observe still advances in memory when disk writes fail.
        let handle = ActivityTrackerHandle::new_with_path(tracker(), nested);
        handle.observe(t0, Some(0));
        handle.observe(t0 + 90_000, Some(0));
        assert!(handle.summary(t0 + 90_000).active_seconds >= 90);
        assert_eq!(fs::read(&path).expect("unrelated history intact"), original);
    }

    #[test]
    fn persisted_file_prunes_to_the_rolling_window() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let t0 = 1_700_000_000_000_u64;
        let mut tracker = ActivityTracker::new(60, 120, 300, 3);
        tracker.observe(t0, Some(0));
        tracker.observe(t0 + 100_000, Some(0));
        tracker.observe(t0 + 1_000_000, Some(0));
        persist_history(&path, &tracker.to_persisted()).expect("persist");

        let loaded = load_or_repair_history(&path, t0 + 1_000_000, 300).expect("load");
        assert!(loaded.len() <= 2);
        for segment in &loaded {
            assert!(segment.end_ms > (t0 + 1_000_000) - 300_000);
        }
        let bytes = fs::metadata(&path).expect("meta").len();
        assert!(bytes < 16_384, "history file should stay small after prune");
    }

    #[test]
    fn future_timestamps_make_storage_unavailable_without_inventing_spans() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let now = 1_700_000_000_000_u64;
        let history = PersistedActivityHistory {
            version: HISTORY_SCHEMA_VERSION,
            segments: vec![PersistedSegment {
                kind: PersistedKind::Active,
                start_ms: now + 60_000,
                end_ms: now + 120_000,
            }],
        };
        persist_history(&path, &history).expect("persist skewed");

        let original = fs::read(&path).expect("read skewed bytes");
        let failure = load_or_repair_history(&path, now, ACTIVITY_WINDOW_SECONDS)
            .expect_err("future timestamps are invalid");
        assert_eq!(failure.category, StorageFailureCategory::Invalid);
        assert_eq!(fs::read(&path).expect("bytes remain"), original);

        let mut tracker = tracker();
        tracker.restore_segments(
            vec![Segment {
                kind: ActivityKind::Active,
                start_ms: now + 1,
                end_ms: now + 2,
            }],
            now,
        );
        assert!(tracker.segments.is_empty());
        assert_eq!(tracker.last_kind, None);
    }

    #[test]
    fn malformed_history_is_preserved_and_unavailable() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let original = b"{not-json";
        fs::write(&path, original).expect("seed garbage");

        let handle = ActivityTrackerHandle::initialize_at(&dir.path, 1_700_000_000_000);

        assert_eq!(fs::read(&path).expect("read"), original);
        let snapshot = handle.snapshot(1_700_000_000_000);
        assert!(snapshot.data.is_none());
        assert_eq!(
            snapshot.load_health.recovery,
            crate::storage_recovery::StorageRecovery::RetryOrStartNew
        );
        assert!(handle.range(&[1, 2]).is_err());
        assert_eq!(
            handle.presentation_context(1_700_000_000_000),
            ActivityPresentationContext {
                history_available: false,
                continuous_active_seconds: 0,
                recent_afk_seconds: 0,
            }
        );
    }

    #[test]
    fn read_failure_stays_unavailable_until_retry_recovers_canonical_path() {
        let dir = TestDirectory::new();
        let blocked_config = dir.path.join("blocked-config");
        fs::write(&blocked_config, b"blocker").expect("block config directory");
        let canonical = blocked_config.join(HISTORY_FILE_NAME);
        let now = 1_700_000_000_000;
        let handle = ActivityTrackerHandle::initialize_at(&blocked_config, now);

        assert_eq!(&*handle.path, &canonical);
        assert_eq!(
            handle.snapshot(now).load_health.recovery,
            crate::storage_recovery::StorageRecovery::Retry
        );
        handle.observe(now, Some(0));
        assert_eq!(
            fs::read(&blocked_config).expect("blocker untouched"),
            b"blocker"
        );
        assert!(handle.range(&[now, now + 1]).is_err());
        assert!(handle.start_new_after_invalid(now).is_err());

        fs::remove_file(&blocked_config).expect("remove blocker");
        fs::create_dir(&blocked_config).expect("create config directory");
        persist_history(
            &canonical,
            &PersistedActivityHistory {
                version: HISTORY_SCHEMA_VERSION,
                segments: Vec::new(),
            },
        )
        .expect("write repaired history");

        assert_eq!(handle.retry_load(now), StorageLoadHealth::available());
        assert!(handle.snapshot(now).data.is_some());
    }

    #[test]
    fn failed_retry_preserves_invalid_bytes_and_unavailable_state() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let original = b"not valid activity history";
        fs::write(&path, original).expect("seed invalid history");
        let now = 1_700_000_000_000;
        let handle = ActivityTrackerHandle::initialize_at(&dir.path, now);

        let health = handle.retry_load(now);

        assert_eq!(
            health.status,
            crate::storage_recovery::StorageLoadStatus::Unavailable
        );
        assert_eq!(
            health.recovery,
            crate::storage_recovery::StorageRecovery::RetryOrStartNew
        );
        assert_eq!(fs::read(path).expect("canonical unchanged"), original);
        assert!(handle.snapshot(now).data.is_none());
    }

    #[test]
    fn invalid_start_new_quarantines_exact_bytes_and_writes_empty_history() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let original = b"invalid activity bytes\0";
        fs::write(&path, original).expect("seed invalid history");
        let now = 1_700_000_000_000;
        let handle = ActivityTrackerHandle::initialize_at(&dir.path, now);

        assert_eq!(
            handle.start_new_after_invalid(now).expect("start new"),
            StorageLoadHealth::available()
        );

        let persisted: PersistedActivityHistory =
            serde_json::from_slice(&fs::read(&path).expect("new canonical")).expect("valid empty");
        assert_eq!(persisted.version, HISTORY_SCHEMA_VERSION);
        assert!(persisted.segments.is_empty());
        let quarantines: Vec<_> = fs::read_dir(&dir.path)
            .expect("siblings")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("activity-history.json.invalid-")
            })
            .collect();
        assert_eq!(quarantines.len(), 1);
        assert_eq!(
            fs::read(quarantines[0].path()).expect("quarantine"),
            original
        );
        assert!(handle.snapshot(now).data.is_some());
    }

    #[cfg(unix)]
    #[test]
    fn start_new_preserves_restrictive_canonical_permissions() {
        use std::os::unix::fs::PermissionsExt;

        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        fs::write(&path, b"invalid activity bytes").expect("seed invalid history");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("restrict canonical permissions");
        let now = 1_700_000_000_000;
        let handle = ActivityTrackerHandle::initialize_at(&dir.path, now);

        handle
            .start_new_after_invalid(now)
            .expect("start new activity history");

        assert_eq!(
            fs::metadata(path).expect("metadata").permissions().mode() & 0o7777,
            0o600
        );
    }

    #[test]
    fn failed_quarantine_or_replacement_preserves_invalid_canonical_bytes() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let original = b"invalid activity bytes";
        fs::write(&path, original).expect("seed invalid history");
        let now = 1_700_000_000_000;
        let handle = ActivityTrackerHandle::initialize_at(&dir.path, now);
        crate::storage_recovery::TEST_QUARANTINE_FAILURES
            .lock()
            .expect("hook")
            .push(path.clone());

        assert!(handle.start_new_after_invalid(now).is_err());
        assert_eq!(fs::read(&path).expect("after quarantine failure"), original);

        *TEST_HISTORY_PERSIST_FAILURE.lock().expect("hook") = Some(path.clone());
        assert!(handle.start_new_after_invalid(now).is_err());
        assert_eq!(
            fs::read(&path).expect("after replacement failure"),
            original
        );
        assert!(handle.snapshot(now).data.is_none());
    }

    #[test]
    fn concurrent_external_activity_repair_is_not_overwritten() {
        use std::time::Duration;

        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let original = b"invalid activity bytes";
        fs::write(&path, original).expect("seed invalid history");
        let now = 1_700_000_000_000;
        let handle = ActivityTrackerHandle::initialize_at(&dir.path, now);
        let repaired = PersistedActivityHistory {
            version: HISTORY_SCHEMA_VERSION,
            segments: Vec::new(),
        };
        let (started, release) = crate::storage_recovery::install_replacement_barrier(path.clone());
        let recovering = handle.clone();
        let recovery = std::thread::spawn(move || recovering.start_new_after_invalid(now));
        started
            .recv_timeout(Duration::from_secs(1))
            .expect("recovery reaches final validation");

        persist_history(&path, &repaired).expect("external repair replaces canonical file");
        let repaired_bytes = fs::read(&path).expect("repaired bytes");
        release.send(()).expect("release recovery");

        assert!(recovery.join().expect("recovery thread").is_err());
        assert_eq!(
            fs::read(&path).expect("canonical remains repaired"),
            repaired_bytes
        );
        assert!(handle.snapshot(now).data.is_none());
        assert_eq!(handle.retry_load(now), StorageLoadHealth::available());
    }

    #[test]
    fn canonical_removal_at_final_recheck_publishes_read_failure() {
        use std::time::Duration;

        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        fs::write(&path, b"invalid activity bytes").expect("seed invalid history");
        let now = 1_700_000_000_000;
        let handle = ActivityTrackerHandle::initialize_at(&dir.path, now);
        let (started, release) = crate::storage_recovery::install_replacement_barrier(path.clone());
        let recovering = handle.clone();
        let recovery = std::thread::spawn(move || recovering.start_new_after_invalid(now));
        started
            .recv_timeout(Duration::from_secs(1))
            .expect("recovery reaches final canonical recheck");

        fs::remove_file(&path).expect("remove canonical history");
        release.send(()).expect("release recovery");

        assert!(recovery.join().expect("recovery thread").is_err());
        assert_eq!(
            handle.snapshot(now).load_health,
            StorageLoadHealth::unavailable(StorageFailureCategory::Read)
        );
        assert!(
            handle.start_new_after_invalid(now).is_err(),
            "read failures must remove start-new capability"
        );
        assert_eq!(handle.retry_load(now), StorageLoadHealth::available());
    }

    #[test]
    fn hot_history_recovery_does_not_rewrite_archive_chunks() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        fs::write(&path, b"invalid hot history").expect("invalid hot file");
        let archived = Segment {
            kind: ActivityKind::Active,
            start_ms: 1_000,
            end_ms: 2_000,
        };
        activity_archive::archive_segments(&dir.path, &[archived]).expect("seed archive");
        let archive_path = activity_archive::chunk_path(&dir.path, 0);
        let archive_bytes = fs::read(&archive_path).expect("archive bytes");
        let handle = ActivityTrackerHandle::initialize_at(&dir.path, 1_700_000_000_000);

        handle
            .start_new_after_invalid(1_700_000_000_000)
            .expect("start new hot history");

        assert_eq!(
            fs::read(archive_path).expect("archive remains"),
            archive_bytes
        );
    }

    #[test]
    fn load_archives_segments_aged_out_while_closed() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let t0 = 1_700_000_000_000_u64;
        let old_segment = PersistedSegment {
            kind: PersistedKind::Active,
            start_ms: t0,
            end_ms: t0 + 60_000,
        };
        persist_history(
            &path,
            &PersistedActivityHistory {
                version: HISTORY_SCHEMA_VERSION,
                segments: vec![old_segment],
            },
        )
        .expect("seed hot file with an aged segment");

        // Reopen several days later, as if the app had been closed the whole
        // time: the live prune never ran, so this segment never reached
        // memory until now.
        let reopened_at = t0 + 60_000 + 5 * 24 * 60 * 60 * 1_000;
        let loaded = load_or_repair_history(&path, reopened_at, ACTIVITY_WINDOW_SECONDS)
            .expect("load on reopen");

        assert!(
            loaded.is_empty(),
            "the aged-out segment must leave the live set on load"
        );

        let archived = activity_archive::read_range(&dir.path, t0, t0 + 60_000 + 1);
        assert_eq!(
            archived,
            vec![Segment {
                kind: ActivityKind::Active,
                start_ms: t0,
                end_ms: t0 + 60_000,
            }],
            "the aged-out segment must be archived on load instead of discarded"
        );
    }

    #[test]
    fn load_prunes_expired_archive_chunks_without_hot_segments() {
        let dir = TestDirectory::new();
        let ancient = Segment {
            kind: ActivityKind::Active,
            start_ms: 500,
            end_ms: 900,
        };
        activity_archive::archive_segments(&dir.path, &[ancient]).expect("seed expired chunk");
        let expired_chunk = activity_archive::chunk_path(&dir.path, 0);

        let now = HISTORY_RETENTION_SECONDS
            .saturating_mul(MILLIS_PER_SECOND)
            .saturating_add(ARCHIVE_BLOCK_MS)
            .saturating_add(1);
        let handle =
            ActivityTrackerHandle::load_at(&dir.path, now).expect("load empty hot history");

        assert!(
            !expired_chunk.exists(),
            "startup must prune expired archive chunks even when hot history is empty"
        );
        assert_eq!(handle.summary(now).active_seconds, 0);
    }

    #[test]
    fn load_retains_straddling_segment_whole() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let t0 = 1_700_000_000_000_u64;
        let straddler = PersistedSegment {
            kind: PersistedKind::Active,
            start_ms: t0,
            end_ms: t0 + 250_000,
        };
        persist_history(
            &path,
            &PersistedActivityHistory {
                version: HISTORY_SCHEMA_VERSION,
                segments: vec![straddler],
            },
        )
        .expect("seed hot file with a straddling segment");

        // window is 300_000 ms, so cutoff = now - 300_000 = t0 + 100_000,
        // which falls inside the segment's span.
        let now = t0 + 400_000;
        let loaded = load_or_repair_history(&path, now, 300).expect("load");

        assert_eq!(
            loaded,
            vec![Segment {
                kind: ActivityKind::Active,
                start_ms: t0,
                end_ms: t0 + 250_000,
            }],
            "a straddling segment must load with its original start_ms, not truncated"
        );
    }

    #[test]
    fn load_keeps_segments_hot_when_archive_write_fails() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let t0 = 1_700_000_000_000_u64;
        let old_segment_start_ms = t0;
        let old_segment = PersistedSegment {
            kind: PersistedKind::Active,
            start_ms: old_segment_start_ms,
            end_ms: t0 + 60_000,
        };
        persist_history(
            &path,
            &PersistedActivityHistory {
                version: HISTORY_SCHEMA_VERSION,
                segments: vec![old_segment],
            },
        )
        .expect("seed hot file with an aged segment");

        // `path` must sit under a real, readable directory for `fs::read` to
        // succeed at all, so the config-dir-under-a-file trick
        // `prune_keeps_segments_hot_when_archive_write_fails` uses can't
        // transfer here directly. The equivalent forced failure for a real
        // config_dir is blocking the exact chunk file the segment would
        // archive into with a directory, so `archive_segments`'s rename onto
        // it fails the same way — same narrative (segment stays hot when the
        // archive write fails), the closest available mechanism given a
        // real, readable config_dir is required for the read to succeed.
        let chunk_key = activity_archive::chunk_key(old_segment_start_ms);
        fs::create_dir(activity_archive::chunk_path(&dir.path, chunk_key))
            .expect("blocker directory in place of the chunk file");

        // Go through `ActivityTrackerHandle::load_at`, not just
        // `load_or_repair_history` directly: this is the path that also
        // calls `restore_segments`, which is exactly what pins the bug this
        // test exists for. `load_or_repair_history` alone correctly keeps a
        // failed-to-archive segment in its returned set; the bug this
        // guards against is `restore_segments` re-dropping that same
        // segment a moment later via the `drop_archived` call this task
        // removed, using the identical `end_ms <= cutoff` predicate at the
        // identical `now_ms`. A test that stopped at `load_or_repair_history`
        // would not observe that second drop at all.
        let reopened_at = t0 + 60_000 + 5 * 24 * 60 * 60 * 1_000;
        let handle =
            ActivityTrackerHandle::load_at(&dir.path, reopened_at).expect("load on reopen");

        let locked = handle.inner.lock().expect("lock tracker state");
        assert!(
            locked
                .as_available()
                .expect("available")
                .tracker
                .segments
                .contains(&Segment {
                    kind: ActivityKind::Active,
                    start_ms: t0,
                    end_ms: t0 + 60_000,
                }),
            "a failed archive write at load time must keep the segment hot for a later retry, \
             not be silently re-dropped by restore_segments"
        );
    }

    #[test]
    fn reverse_wall_clock_drops_live_history() {
        let mut tracker = tracker();
        let t0 = 1_700_000_000_000_u64;
        tracker.observe(t0 + 60_000, Some(0));
        tracker.observe(t0 + 120_000, Some(0));
        tracker.observe(t0, Some(0));
        assert!(tracker.summary(t0).active_seconds < 60);
    }

    #[test]
    fn prune_archives_segments_before_dropping_them() {
        let dir = TestDirectory::new();
        let t0 = 1_700_000_000_000_u64;
        let old_segment = Segment {
            kind: ActivityKind::Active,
            start_ms: t0,
            end_ms: t0 + 60_000,
        };
        let buffer_segment = Segment {
            kind: ActivityKind::Afk,
            start_ms: t0 + 60_000,
            end_ms: t0 + 61_000,
        };
        let mut seeded = tracker();
        seeded.restore_segments(vec![old_segment, buffer_segment], t0 + 61_000);

        let handle = ActivityTrackerHandle::new_with_path(seeded, dir.path.join(HISTORY_FILE_NAME));

        // Advance far past the 24-hour window so `old_segment` is fully
        // expired and archivable. The observation itself only ever extends
        // the buffer segment (still hot); `old_segment` is untouched.
        let later = t0 + 61_000 + 90_000_000;
        handle.observe(later, Some(400));

        let archived = activity_archive::read_range(&dir.path, t0, t0 + 60_000 + 1);
        assert_eq!(
            archived,
            vec![old_segment],
            "the expired segment must be archived"
        );

        let locked = handle.inner.lock().expect("lock tracker state");
        assert!(
            !locked
                .as_available()
                .expect("available")
                .tracker
                .segments
                .contains(&old_segment),
            "an archived segment must be dropped from the hot set"
        );
    }

    #[test]
    fn prune_keeps_segments_hot_when_archive_write_fails() {
        let dir = TestDirectory::new();
        let blocker = dir.path.join("not-a-directory");
        fs::write(&blocker, b"x").expect("blocker file");
        let broken_path = blocker.join(HISTORY_FILE_NAME);

        let t0 = 1_700_000_000_000_u64;
        let old_segment = Segment {
            kind: ActivityKind::Active,
            start_ms: t0,
            end_ms: t0 + 60_000,
        };
        let buffer_segment = Segment {
            kind: ActivityKind::Afk,
            start_ms: t0 + 60_000,
            end_ms: t0 + 61_000,
        };
        let mut seeded = tracker();
        seeded.restore_segments(vec![old_segment, buffer_segment], t0 + 61_000);

        // config_dir is derived from `broken_path`'s parent, which is a
        // file, so both the archive write and the hot persist must fail.
        let handle = ActivityTrackerHandle::new_with_path(seeded, broken_path);
        let later = t0 + 61_000 + 90_000_000;
        handle.observe(later, Some(400));

        let locked = handle.inner.lock().expect("lock tracker state");
        assert!(
            locked
                .as_available()
                .expect("available")
                .tracker
                .segments
                .contains(&old_segment),
            "a failed archive write must keep the segment hot for a later retry"
        );
        assert_eq!(
            locked
                .as_available()
                .expect("available")
                .tracker
                .segments
                .len(),
            2,
            "nothing is lost when the archive write fails"
        );
    }

    #[test]
    fn archive_attempt_respects_the_persist_throttle() {
        let dir = TestDirectory::new();
        let t0 = 1_700_000_000_000_u64;

        // A one-second window so a segment can age into "archivable" within
        // milliseconds, without waiting anywhere near the 30-second persist
        // throttle. `closed_segment` never gets extended again (its kind
        // differs from the segment that follows it), so it stays fixed while
        // the clock advances around it.
        let closed_segment = Segment {
            kind: ActivityKind::Afk,
            start_ms: t0,
            end_ms: t0 + 50,
        };
        let open_segment = Segment {
            kind: ActivityKind::Active,
            start_ms: t0 + 50,
            end_ms: t0 + 50,
        };
        let mut seeded = ActivityTracker::new(60, 120, 1, 3);
        seeded.restore_segments(vec![closed_segment, open_segment], t0 + 50);

        let handle = ActivityTrackerHandle::new_with_path(seeded, dir.path.join(HISTORY_FILE_NAME));

        // First observation: `last_persisted_at_ms` is still `None`, so this
        // call is due regardless of `kind_changed`. `closed_segment` has not
        // aged out of the 1-second window yet, so nothing is archivable here
        // — this call only establishes the persisted-at baseline.
        let first_at = t0 + 100;
        handle.observe(first_at, Some(0));

        // Second observation: same kind (idle stays below the 60s AFK
        // threshold, so both calls classify Active) and well inside the
        // 30-second persist throttle, so this call is neither due nor a kind
        // change. But 1.2 seconds have now passed, more than the 1-second
        // window, so `closed_segment` has aged into archivable territory.
        let second_at = first_at + 1_200;
        handle.observe(second_at, Some(0));

        let locked = handle.inner.lock().expect("lock tracker state");
        assert!(
            locked
                .as_available()
                .expect("available")
                .tracker
                .segments
                .contains(&closed_segment),
            "an archivable segment must stay hot when the observation that saw it \
             was neither due nor a kind change"
        );
        drop(locked);

        let archived = activity_archive::read_range(&dir.path, t0, t0 + 50 + 1);
        assert!(
            archived.is_empty(),
            "no archive write must occur on an observation that is neither due nor a kind change"
        );
    }

    #[test]
    fn prune_retains_straddling_segment_whole() {
        let mut tracker = ActivityTracker::new(60, 120, 300, 3);
        let t0 = 1_700_000_000_000_u64;
        let straddler = Segment {
            kind: ActivityKind::Active,
            start_ms: t0,
            end_ms: t0 + 250_000,
        };
        // window is 300_000 ms, so cutoff = now - 300_000 = t0 + 100_000,
        // which falls inside the segment's span.
        let now = t0 + 400_000;
        tracker.restore_segments(vec![straddler], now);

        assert!(
            tracker.archivable_segments(now).is_empty(),
            "a straddling segment must not be treated as archivable"
        );
        tracker.drop_archived(now);
        assert_eq!(
            tracker.segments,
            vec![straddler],
            "a straddling segment must stay hot with its original start_ms, not truncated"
        );
    }

    #[test]
    fn straddling_segment_summary_matches_clamped_value() {
        let mut tracker = ActivityTracker::new(60, 120, 300, 3);
        let t0 = 1_700_000_000_000_u64;
        let straddler = Segment {
            kind: ActivityKind::Active,
            start_ms: t0,
            end_ms: t0 + 250_000,
        };
        let now = t0 + 400_000;
        tracker.restore_segments(vec![straddler], now);

        let summary = tracker.summary(now);
        // window_start = now - 300_000 = t0 + 100_000; the clamped duration
        // is (t0 + 250_000) - (t0 + 100_000) = 150_000 ms, identical to what
        // truncating start_ms to the cutoff used to produce.
        assert_eq!(summary.active_seconds, 150);
    }

    #[test]
    fn prune_writes_nothing_when_no_segment_expired() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let handle = ActivityTrackerHandle::new_with_path(tracker(), path);
        let t0 = 1_700_000_000_000_u64;
        handle.observe(t0, Some(0));
        handle.observe(t0 + 90_000, Some(2));

        let archive_files: Vec<_> = fs::read_dir(&dir.path)
            .expect("read config dir")
            .filter_map(|entry| entry.ok())
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("activity-archive-")
            })
            .collect();
        assert!(
            archive_files.is_empty(),
            "an idle prune with nothing expired must not write any archive chunk"
        );
    }

    #[test]
    fn prune_deletes_expired_chunks() {
        let dir = TestDirectory::new();

        // A chunk from long ago, seeded directly so it predates anything the
        // handle itself will write.
        let ancient = Segment {
            kind: ActivityKind::Active,
            start_ms: 500,
            end_ms: 900,
        };
        activity_archive::archive_segments(&dir.path, &[ancient]).expect("seed old chunk");
        assert!(activity_archive::chunk_path(&dir.path, 0).exists());

        // A segment that will become archivable once `later` is reached, so
        // the same observation both archives fresh data and runs the
        // retention prune alongside it.
        let recent = Segment {
            kind: ActivityKind::Active,
            start_ms: 8_000_000_000,
            end_ms: 8_000_060_000,
        };
        let buffer = Segment {
            kind: ActivityKind::Afk,
            start_ms: 8_000_060_000,
            end_ms: 8_000_061_000,
        };
        let mut seeded = tracker();
        seeded.restore_segments(vec![recent, buffer], 8_000_061_000);

        let handle = ActivityTrackerHandle::new_with_path(seeded, dir.path.join(HISTORY_FILE_NAME));

        // Past the 24h window (so `recent` is archivable) and past the
        // 90-day retention cutoff for chunk 0's block, but not for the
        // fresh chunk's block.
        let later = 12_000_000_000_u64;
        handle.observe(later, Some(0));

        assert!(
            !activity_archive::chunk_path(&dir.path, 0).exists(),
            "chunk 0 is fully older than the retention cutoff and must be deleted"
        );
        assert!(
            activity_archive::chunk_path(&dir.path, 3).exists(),
            "the freshly archived chunk must survive the same prune pass"
        );
    }

    #[test]
    fn observe_prunes_expired_chunks_without_archivable_segments() {
        let dir = TestDirectory::new();
        let now = HISTORY_RETENTION_SECONDS
            .saturating_mul(MILLIS_PER_SECOND)
            .saturating_add(ARCHIVE_BLOCK_MS)
            .saturating_add(1);
        let handle =
            ActivityTrackerHandle::new_with_path(tracker(), dir.path.join(HISTORY_FILE_NAME));

        // Establish the live retention cadence before an expired chunk
        // appears; the only hot segment remains open and cannot be archived.
        handle.observe(now, Some(0));
        let ancient = Segment {
            kind: ActivityKind::Active,
            start_ms: 500,
            end_ms: 900,
        };
        activity_archive::archive_segments(&dir.path, &[ancient]).expect("seed expired chunk");
        let expired_chunk = activity_archive::chunk_path(&dir.path, 0);

        handle.observe(now + PERSIST_MIN_INTERVAL_MS, Some(0));

        assert!(
            !expired_chunk.exists(),
            "the live retention cadence must prune without a newly archivable segment"
        );
        assert!(
            activity_archive::read_range(&dir.path, now, now + 1).is_empty(),
            "the observing segment must remain hot rather than being archived"
        );
    }

    #[test]
    fn observe_prunes_expired_chunks_on_clock_reversal() {
        let dir = TestDirectory::new();
        let now = HISTORY_RETENTION_SECONDS
            .saturating_mul(MILLIS_PER_SECOND)
            .saturating_add(ARCHIVE_BLOCK_MS)
            .saturating_add(1_000);
        let handle =
            ActivityTrackerHandle::new_with_path(tracker(), dir.path.join(HISTORY_FILE_NAME));

        handle.observe(now, Some(0));
        let ancient = Segment {
            kind: ActivityKind::Active,
            start_ms: 500,
            end_ms: 900,
        };
        activity_archive::archive_segments(&dir.path, &[ancient]).expect("seed expired chunk");
        let expired_chunk = activity_archive::chunk_path(&dir.path, 0);

        handle.observe(now - 1, Some(0));

        assert!(
            !expired_chunk.exists(),
            "a clock reversal must immediately retry retention pruning"
        );
    }

    #[test]
    fn range_rejects_fewer_than_two_boundaries() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let handle = ActivityTrackerHandle::new_with_path(tracker(), path);

        assert!(
            handle.range(&[]).is_err(),
            "empty boundaries must be rejected"
        );
        assert!(
            handle.range(&[1_700_000_000_000]).is_err(),
            "a single boundary must be rejected"
        );
    }

    #[test]
    fn range_rejects_non_increasing_boundaries() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let handle = ActivityTrackerHandle::new_with_path(tracker(), path);
        let t0 = 1_700_000_000_000_u64;

        assert!(
            handle.range(&[t0, t0]).is_err(),
            "equal adjacent boundaries must be rejected"
        );
        assert!(
            handle.range(&[t0, t0 - 1]).is_err(),
            "decreasing boundaries must be rejected"
        );
        assert!(
            handle.range(&[t0, t0 + 10_000, t0 + 5_000]).is_err(),
            "a later non-increasing pair must be rejected too"
        );
    }

    #[test]
    fn range_rejects_more_than_the_bucket_cap() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let handle = ActivityTrackerHandle::new_with_path(tracker(), path);
        let t0 = 1_700_000_000_000_u64;

        let at_cap: Vec<u64> = (0..=MAX_RANGE_BUCKETS as u64).map(|i| t0 + i).collect();
        assert!(
            handle.range(&at_cap).is_ok(),
            "exactly MAX_RANGE_BUCKETS buckets must be accepted"
        );

        let over_cap: Vec<u64> = (0..=(MAX_RANGE_BUCKETS as u64 + 1))
            .map(|i| t0 + i)
            .collect();
        assert!(
            handle.range(&over_cap).is_err(),
            "one more than the cap must be rejected"
        );
    }

    #[test]
    fn range_rejects_a_span_wider_than_retention() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let handle = ActivityTrackerHandle::new_with_path(tracker(), path);

        // Two boundaries spanning almost the entire u64 range: exactly one
        // bucket, strictly increasing, so this passes every check except the
        // span cap. Without that cap, `read_range` would walk from the first
        // archive chunk key to the last, roughly seven billion reads, inside
        // a single command call.
        let result = handle.range(&[0, u64::MAX]);
        assert!(
            result.is_err(),
            "a span wider than retention plus one archive block must be rejected"
        );

        // A span that exactly matches the maximum allowed width must still
        // be accepted, confirming the cap targets the span, not the request.
        let t0 = 1_700_000_000_000_u64;
        let max_span_ms = HISTORY_RETENTION_SECONDS
            .saturating_mul(MILLIS_PER_SECOND)
            .saturating_add(ARCHIVE_BLOCK_MS);
        assert!(
            handle.range(&[t0, t0 + max_span_ms]).is_ok(),
            "a span exactly at the cap must still be accepted"
        );
        assert!(
            handle.range(&[t0, t0 + max_span_ms + 1]).is_err(),
            "one millisecond past the cap must be rejected"
        );
    }

    #[test]
    fn range_returns_one_bucket_per_adjacent_pair() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let handle = ActivityTrackerHandle::new_with_path(tracker(), path);
        let t0 = 1_700_000_000_000_u64;

        let boundaries = vec![t0, t0 + 10_000, t0 + 20_000, t0 + 45_000];
        let buckets = handle.range(&boundaries).expect("range succeeds");
        assert_eq!(buckets.len(), boundaries.len() - 1);
    }

    #[test]
    fn range_reports_zero_for_an_empty_bucket() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let handle = ActivityTrackerHandle::new_with_path(tracker(), path);
        let t0 = 1_700_000_000_000_u64;

        let buckets = handle
            .range(&[t0, t0 + 60_000])
            .expect("range succeeds for a window with no data");
        assert_eq!(buckets.len(), 1);
        assert_eq!(buckets[0].active_ms, 0);
        assert_eq!(buckets[0].afk_ms, 0);
    }

    #[test]
    fn range_sums_hot_and_archived_segments_once() {
        let dir = TestDirectory::new();
        let t0 = 1_700_000_000_000_u64;
        let segment = Segment {
            kind: ActivityKind::Active,
            start_ms: t0,
            end_ms: t0 + 60_000,
        };
        let same_start = Segment {
            kind: ActivityKind::Afk,
            start_ms: t0,
            end_ms: t0 + 30_000,
        };

        // Simulate the archive-before-drop race: `observe` archives a segment
        // to disk and only then drops it from the hot set, under the same
        // lock. A reader that snapshots the hot set just before that drop and
        // reads the archive just after it can observe the identical segment
        // in both places. `range` must still count its duration once.
        activity_archive::archive_segments(&dir.path, &[segment]).expect("seed archive");
        let mut seeded = tracker();
        seeded.restore_segments(vec![segment, same_start], t0 + 60_000);
        let handle = ActivityTrackerHandle::new_with_path(seeded, dir.path.join(HISTORY_FILE_NAME));

        let buckets = handle.range(&[t0, t0 + 60_000]).expect("range succeeds");

        assert_eq!(buckets.len(), 1);
        assert_eq!(
            buckets[0].active_ms, 60_000,
            "a segment present in both the hot set and the archive must be counted once"
        );
        assert_eq!(buckets[0].afk_ms, 30_000);
    }

    #[test]
    fn range_serializes_longest_active_ms_clamped_to_bucket_overlap() {
        let dir = TestDirectory::new();
        let t0 = 1_700_000_000_000_u64;
        let mut seeded = tracker();
        seeded.restore_segments(
            vec![Segment {
                kind: ActivityKind::Active,
                start_ms: t0.saturating_sub(20_000),
                end_ms: t0 + 50_000,
            }],
            t0 + 50_000,
        );
        let handle = ActivityTrackerHandle::new_with_path(seeded, dir.path.join(HISTORY_FILE_NAME));

        let buckets = handle.range(&[t0, t0 + 20_000]).expect("range succeeds");
        let serialized = serde_json::to_value(&buckets).expect("serialize buckets");

        assert_eq!(
            serialized,
            serde_json::json!([{
                "activeMs": 20_000,
                "afkMs": 0,
                "longestActiveMs": 20_000
            }])
        );
    }

    #[test]
    fn range_matches_strip_buckets_for_an_equivalent_window() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        let handle = ActivityTrackerHandle::new_with_path(tracker(), path);
        let t0 = 1_700_000_000_000_u64;

        // Spread active and afk time across more than one bucket.
        handle.observe(t0, Some(0));
        handle.observe(t0 + 30 * 60_000, Some(0));
        handle.observe(t0 + 30 * 60_000 + 1_000, Some(400));
        handle.observe(t0 + 90 * 60_000, Some(400));
        handle.observe(t0 + 90 * 60_000 + 1_000, Some(0));

        let now_ms = t0 + ACTIVITY_WINDOW_SECONDS * MILLIS_PER_SECOND;
        let summary = handle.summary(now_ms);
        assert_eq!(summary.strip.len(), STRIP_BUCKET_COUNT);

        let window_ms = ACTIVITY_WINDOW_SECONDS * MILLIS_PER_SECOND;
        let window_start = now_ms - window_ms;
        let bucket_ms = window_ms / STRIP_BUCKET_COUNT as u64;
        let boundaries: Vec<u64> = (0..=STRIP_BUCKET_COUNT as u64)
            .map(|index| window_start + bucket_ms * index)
            .collect();

        let range_buckets = handle.range(&boundaries).expect("range succeeds");
        assert_eq!(range_buckets.len(), STRIP_BUCKET_COUNT);

        for (index, (strip_bucket, range_bucket)) in
            summary.strip.iter().zip(range_buckets.iter()).enumerate()
        {
            let expected_active = (strip_bucket.active_ratio * bucket_ms as f64).round() as u64;
            let expected_afk = (strip_bucket.afk_ratio * bucket_ms as f64).round() as u64;
            assert!(
                range_bucket.active_ms.abs_diff(expected_active) <= 1,
                "bucket {index}: active_ms mismatch: strip-derived={expected_active} range={}",
                range_bucket.active_ms
            );
            assert!(
                range_bucket.afk_ms.abs_diff(expected_afk) <= 1,
                "bucket {index}: afk_ms mismatch: strip-derived={expected_afk} range={}",
                range_bucket.afk_ms
            );
        }
    }

    #[test]
    fn presentation_context_reports_continuous_active_and_recent_afk() {
        let mut tracker = tracker();
        let t0 = 1_700_000_000_000_u64;
        // 30 minutes active.
        tracker.observe(t0, Some(0));
        tracker.observe(t0 + 30 * 60_000, Some(0));
        let active_ctx = tracker.presentation_context(t0 + 30 * 60_000);
        assert!(active_ctx.history_available);
        assert!(active_ctx.continuous_active_seconds >= 30 * 60);
        assert_eq!(active_ctx.recent_afk_seconds, 0);

        // Switch to AFK for 6 minutes.
        tracker.observe(t0 + 30 * 60_000 + 1_000, Some(400));
        tracker.observe(t0 + 36 * 60_000, Some(400));
        let afk_ctx = tracker.presentation_context(t0 + 36 * 60_000);
        assert_eq!(afk_ctx.continuous_active_seconds, 0);
        assert!(afk_ctx.recent_afk_seconds >= 5 * 60);

        // Return to active; AFK still recent within grace.
        tracker.observe(t0 + 36 * 60_000 + 30_000, Some(0));
        let grace_ctx = tracker.presentation_context(t0 + 36 * 60_000 + 30_000);
        assert!(grace_ctx.continuous_active_seconds < 60);
        assert!(grace_ctx.recent_afk_seconds >= 5 * 60);
    }
}
