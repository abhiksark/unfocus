//! Continuous activity and AFK segmentation from idle-probe samples.
//!
//! Privacy boundary: only presence of input (OS idle seconds) is used. No
//! keylogging, mouse paths, app titles, or window content.
//!
//! Segment history is stored locally next to app config as JSON. Writes are
//! atomic (temp file + replace). Probe or write failures never mutate the
//! reminder timer.

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
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
/// Half-hour buckets across the rolling window (48 × 30 min = 24 h).
pub(crate) const STRIP_BUCKET_COUNT: usize = 48;
/// Minimum spacing between background history writes while a segment extends.
const PERSIST_MIN_INTERVAL_MS: u64 = 30_000;
const HISTORY_FILE_NAME: &str = "activity-history.json";
const HISTORY_SCHEMA_VERSION: u32 = 1;
const MILLIS_PER_SECOND: u64 = 1_000;

static HISTORY_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ActivityKind {
    Active,
    Afk,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
enum PersistedKind {
    Active,
    Afk,
}

impl PersistedKind {
    fn from_activity(kind: ActivityKind) -> Option<Self> {
        match kind {
            ActivityKind::Active => Some(Self::Active),
            ActivityKind::Afk => Some(Self::Afk),
            ActivityKind::Unknown => None,
        }
    }

    fn into_activity(self) -> ActivityKind {
        match self {
            Self::Active => ActivityKind::Active,
            Self::Afk => ActivityKind::Afk,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Segment {
    kind: ActivityKind,
    start_ms: u64,
    end_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedSegment {
    kind: PersistedKind,
    start_ms: u64,
    end_ms: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedActivityHistory {
    version: u32,
    segments: Vec<PersistedSegment>,
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

    /// Replace live segments with a pruned history snapshot.
    ///
    /// Future wall-clock ends (clock skew) clear history rather than inventing
    /// multi-day spans. Restored state is not treated as a live probe success.
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
            self.last_probe_ok = false;
            return;
        }
        self.prune(now_ms);
        self.last_sample_ms = self.segments.last().map(|segment| segment.end_ms);
        self.last_kind = self.segments.last().map(|segment| segment.kind);
        self.last_probe_ok = false;
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
    pub(crate) probe_available: bool,
    pub(crate) strip: Vec<StripBucket>,
}

#[derive(Debug)]
struct ActivityTrackerState {
    tracker: ActivityTracker,
    path: PathBuf,
    last_persisted_at_ms: Option<u64>,
}

#[derive(Debug, Clone)]
pub(crate) struct ActivityTrackerHandle {
    inner: Arc<Mutex<ActivityTrackerState>>,
}

impl Default for ActivityTrackerHandle {
    fn default() -> Self {
        // Tests and fallbacks only; production uses `load`.
        Self {
            inner: Arc::new(Mutex::new(ActivityTrackerState {
                tracker: ActivityTracker::default(),
                path: PathBuf::from(HISTORY_FILE_NAME),
                last_persisted_at_ms: None,
            })),
        }
    }
}

impl ActivityTrackerHandle {
    /// Load pruned history from `config_dir` or start empty after repair.
    pub(crate) fn load(config_dir: &Path) -> io::Result<Self> {
        Self::load_at(config_dir, epoch_ms(SystemTime::now()))
    }

    fn load_at(config_dir: &Path, now_ms: u64) -> io::Result<Self> {
        let path = config_dir.join(HISTORY_FILE_NAME);
        let mut tracker = ActivityTracker::default();
        let segments = load_or_repair_history(&path, now_ms, tracker.window_seconds)?;
        tracker.restore_segments(segments, now_ms);
        Ok(Self {
            inner: Arc::new(Mutex::new(ActivityTrackerState {
                tracker,
                path,
                last_persisted_at_ms: None,
            })),
        })
    }

    #[cfg(test)]
    fn new_with_path(tracker: ActivityTracker, path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(ActivityTrackerState {
                tracker,
                path,
                last_persisted_at_ms: None,
            })),
        }
    }

    pub(crate) fn observe(&self, now_ms: u64, idle_seconds: Option<u64>) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        let previous_kind = state.tracker.last_kind;
        state.tracker.observe(now_ms, idle_seconds);
        let kind_changed = state.tracker.last_kind != previous_kind;
        let due = match state.last_persisted_at_ms {
            None => true,
            Some(previous) => now_ms.saturating_sub(previous) >= PERSIST_MIN_INTERVAL_MS,
        };
        if !kind_changed && !due {
            return;
        }
        if let Err(error) = persist_history(&state.path, &state.tracker.to_persisted()) {
            eprintln!("could not persist activity history: {error}");
            return;
        }
        state.last_persisted_at_ms = Some(now_ms);
    }

    pub(crate) fn summary(&self, now_ms: u64) -> ActivitySummary {
        self.inner
            .lock()
            .map(|state| state.tracker.summary(now_ms))
            .unwrap_or_else(|_| ActivityTracker::default().summary(now_ms))
    }

    pub(crate) fn presentation_context(&self, now_ms: u64) -> ActivityPresentationContext {
        self.inner
            .lock()
            .map(|state| state.tracker.presentation_context(now_ms))
            .unwrap_or(ActivityPresentationContext {
                history_available: false,
                continuous_active_seconds: 0,
                recent_afk_seconds: 0,
            })
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
    window_seconds: u64,
) -> Result<Vec<Segment>, ()> {
    if history.version != HISTORY_SCHEMA_VERSION {
        return Err(());
    }
    let window_ms = window_seconds.saturating_mul(MILLIS_PER_SECOND);
    let cutoff = now_ms.saturating_sub(window_ms);
    let mut segments = Vec::with_capacity(history.segments.len());
    for item in history.segments {
        if item.end_ms < item.start_ms {
            return Err(());
        }
        if item.end_ms > now_ms || item.start_ms > now_ms {
            // Future timestamps: treat as clock skew, drop whole history.
            return Err(());
        }
        if item.end_ms <= cutoff {
            continue;
        }
        let start_ms = item.start_ms.max(cutoff);
        segments.push(Segment {
            kind: item.kind.into_activity(),
            start_ms,
            end_ms: item.end_ms,
        });
    }
    Ok(segments)
}

fn load_or_repair_history(
    path: &Path,
    now_ms: u64,
    window_seconds: u64,
) -> io::Result<Vec<Segment>> {
    match fs::read(path) {
        Ok(contents) => {
            let parsed = serde_json::from_slice::<PersistedActivityHistory>(&contents)
                .ok()
                .and_then(|history| segments_from_persisted(history, now_ms, window_seconds).ok());
            if let Some(segments) = parsed {
                return Ok(segments);
            }

            // Invalid or untrusted content: write a complete empty history so
            // the next launch does not re-parse corruption silently as success.
            let empty = PersistedActivityHistory {
                version: HISTORY_SCHEMA_VERSION,
                segments: Vec::new(),
            };
            persist_history(path, &empty)?;
            Ok(Vec::new())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
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

    for _ in 0..100 {
        let id = HISTORY_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
        let temp_path = parent.join(format!(".{name}.{}.{id}.tmp", std::process::id()));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)
        {
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

#[cfg(not(target_os = "windows"))]
fn replace_history_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    fs::rename(temp_path, path)
}

#[cfg(target_os = "windows")]
fn replace_history_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    match fs::rename(temp_path, path) {
        Ok(()) => Ok(()),
        Err(_error) if path.exists() => {
            fs::copy(temp_path, path)?;
            OpenOptions::new().write(true).open(path)?.sync_all()?;
            fs::remove_file(temp_path)
        }
        Err(error) => Err(error),
    }
}

fn persist_history(path: &Path, history: &PersistedActivityHistory) -> io::Result<()> {
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

    if let Err(error) = replace_history_file(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
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
        // Restored history is not treated as a live probe success.
        assert!(!after.probe_available);
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
    fn future_timestamps_clear_history_instead_of_inventing_spans() {
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

        let loaded = load_or_repair_history(&path, now, ACTIVITY_WINDOW_SECONDS).expect("load");
        assert!(loaded.is_empty());

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
    fn malformed_history_is_replaced_with_empty_complete_file() {
        let dir = TestDirectory::new();
        let path = dir.path.join(HISTORY_FILE_NAME);
        fs::write(&path, b"{not-json").expect("seed garbage");
        let loaded = load_or_repair_history(&path, 1_700_000_000_000, ACTIVITY_WINDOW_SECONDS)
            .expect("repair");
        assert!(loaded.is_empty());
        let repaired: PersistedActivityHistory =
            serde_json::from_slice(&fs::read(&path).expect("read")).expect("valid json");
        assert_eq!(repaired.version, HISTORY_SCHEMA_VERSION);
        assert!(repaired.segments.is_empty());
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
