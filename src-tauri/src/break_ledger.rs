//! Local-only ledger of break outcomes (shown, natural, suppressed, manual).
//!
//! Observe-only for the timer: write failures never stop reminders. No
//! keylogging, window titles, or telemetry.

use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{SystemTime, UNIX_EPOCH},
};

const LEDGER_FILE_NAME: &str = "break-events.json";
const LEDGER_SCHEMA_VERSION: u32 = 1;
/// Keep at least ninety days of outcomes, matching the activity history
/// retention (`activity.rs`'s `HISTORY_RETENTION_SECONDS`) so week/day counts
/// stay backed by data for as long as the day-by-hour history the frontend
/// will eventually read alongside them.
const RETENTION_SECONDS: u64 = 90 * 24 * 60 * 60;
/// The dashboard's week summary window, deliberately independent of how long
/// events are retained. Retention is storage; this is what "last seven days"
/// means on screen.
const WEEK_SECONDS: u64 = 7 * 24 * 60 * 60;
const MILLIS_PER_SECOND: u64 = 1_000;
const MAX_RANGE_MS: u64 = 31 * 24 * 60 * 60 * MILLIS_PER_SECOND;

static LEDGER_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
struct TestPersistBarrier {
    path: PathBuf,
    started: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

#[cfg(test)]
static TEST_PERSIST_BARRIER: Mutex<Option<TestPersistBarrier>> = Mutex::new(None);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum BreakEventKind {
    /// Scheduled break presented with a multi-monitor overlay.
    ScheduledShown,
    /// Idle long enough that the break was credited without an overlay.
    NaturalIdle,
    /// Fullscreen active; break phase kept without showing the overlay.
    FullscreenSuppress,
    /// User started the configured break immediately.
    ManualTakeBreak,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct BreakEvent {
    at_ms: u64,
    kind: BreakEventKind,
    work_minutes: u64,
    break_seconds: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedBreakLedger {
    version: u32,
    events: Vec<BreakEvent>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BreakLedgerSummary {
    pub(crate) window_label: String,
    pub(crate) window_seconds: u64,
    pub(crate) scheduled_shown: u64,
    pub(crate) natural_idle: u64,
    pub(crate) fullscreen_suppress: u64,
    pub(crate) manual_take_break: u64,
    pub(crate) week_scheduled_shown: u64,
    pub(crate) week_natural_idle: u64,
    pub(crate) week_fullscreen_suppress: u64,
    pub(crate) week_manual_take_break: u64,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct BreakRangeRecord {
    pub(crate) at_ms: u64,
    pub(crate) kind: BreakEventKind,
}

#[derive(Debug)]
struct BreakLedgerState {
    events: Vec<BreakEvent>,
}

#[derive(Debug)]
enum PersistenceMessage {
    Record(BreakEvent),
    Shutdown,
}

#[derive(Debug)]
struct PersistenceWorker {
    sender: mpsc::Sender<PersistenceMessage>,
    join: Option<thread::JoinHandle<()>>,
}

impl Drop for PersistenceWorker {
    fn drop(&mut self) {
        let _ = self.sender.send(PersistenceMessage::Shutdown);
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct BreakLedgerHandle {
    inner: Arc<Mutex<BreakLedgerState>>,
    persistence: Arc<PersistenceWorker>,
}

impl Default for BreakLedgerHandle {
    fn default() -> Self {
        Self::start(PathBuf::from(LEDGER_FILE_NAME), Vec::new())
    }
}

impl BreakLedgerHandle {
    pub(crate) fn load(config_dir: &Path) -> io::Result<Self> {
        let path = config_dir.join(LEDGER_FILE_NAME);
        let now_ms = epoch_ms(SystemTime::now());
        let events = load_or_repair_ledger(&path, now_ms)?;
        Ok(Self::start(path, events))
    }

    #[cfg(test)]
    fn new_with_path(path: PathBuf) -> Self {
        Self::start(path, Vec::new())
    }

    fn start(path: PathBuf, events: Vec<BreakEvent>) -> Self {
        let (sender, receiver) = mpsc::channel();
        let worker_events = events.clone();
        let path_display = path.display().to_string();
        let join = thread::Builder::new()
            .name("break-ledger-persistence".into())
            .spawn(move || persistence_worker(path, worker_events, receiver))
            .map_err(|error| {
                eprintln!(
                    "could not start break ledger persistence worker for {path_display}: {error}"
                );
                error
            })
            .ok();

        Self {
            inner: Arc::new(Mutex::new(BreakLedgerState { events })),
            persistence: Arc::new(PersistenceWorker { sender, join }),
        }
    }

    /// Append one outcome in memory and queue it for persistence.
    /// Persistence failures are logged; memory still updates.
    pub(crate) fn record(
        &self,
        kind: BreakEventKind,
        work_minutes: u64,
        break_seconds: u64,
        now_ms: u64,
    ) {
        let Ok(mut state) = self.inner.lock() else {
            return;
        };
        let event = BreakEvent {
            at_ms: now_ms,
            kind,
            work_minutes,
            break_seconds,
        };
        state.events.push(event.clone());
        prune_events(&mut state.events, now_ms);
        let queued = self
            .persistence
            .sender
            .send(PersistenceMessage::Record(event))
            .is_ok();
        drop(state);
        if !queued {
            eprintln!("could not queue break event for persistence; keeping it in memory");
        }
    }

    pub(crate) fn summary(&self, now_ms: u64) -> BreakLedgerSummary {
        let events = self
            .inner
            .lock()
            .map(|state| state.events.clone())
            .unwrap_or_default();
        summarize_events(&events, now_ms)
    }

    pub(crate) fn range(
        &self,
        start_ms: u64,
        end_ms: u64,
    ) -> Result<Vec<BreakRangeRecord>, String> {
        if end_ms <= start_ms {
            return Err("range must have endMs greater than startMs".into());
        }
        if end_ms.saturating_sub(start_ms) > MAX_RANGE_MS {
            return Err(format!(
                "range must not exceed {MAX_RANGE_MS} elapsed milliseconds"
            ));
        }

        let events = self
            .inner
            .lock()
            .map(|state| state.events.clone())
            .unwrap_or_default();
        Ok(events_in_range(&events, start_ms, end_ms))
    }
}

fn persistence_worker(
    path: PathBuf,
    mut events: Vec<BreakEvent>,
    receiver: mpsc::Receiver<PersistenceMessage>,
) {
    while let Ok(first) = receiver.recv() {
        let mut changed = false;
        let mut shutdown = false;
        for message in std::iter::once(first).chain(receiver.try_iter()) {
            match message {
                PersistenceMessage::Record(event) => {
                    let now_ms = event.at_ms;
                    events.push(event);
                    prune_events(&mut events, now_ms);
                    changed = true;
                }
                PersistenceMessage::Shutdown => shutdown = true,
            }
        }

        if changed {
            let ledger = PersistedBreakLedger {
                version: LEDGER_SCHEMA_VERSION,
                events: std::mem::take(&mut events),
            };
            if let Err(error) = persist_ledger(&path, &ledger) {
                eprintln!("could not persist break event ledger: {error}");
            }
            events = ledger.events;
        }
        if shutdown {
            return;
        }
    }
}

fn epoch_ms(now: SystemTime) -> u64 {
    now.duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

fn retention_ms() -> u64 {
    RETENTION_SECONDS.saturating_mul(MILLIS_PER_SECOND)
}

fn day_ms() -> u64 {
    24 * 60 * 60 * MILLIS_PER_SECOND
}

fn week_ms() -> u64 {
    WEEK_SECONDS.saturating_mul(MILLIS_PER_SECOND)
}

fn prune_events(events: &mut Vec<BreakEvent>, now_ms: u64) {
    let cutoff = now_ms.saturating_sub(retention_ms());
    events.retain(|event| event.at_ms >= cutoff && event.at_ms <= now_ms);
}

fn summarize_events(events: &[BreakEvent], now_ms: u64) -> BreakLedgerSummary {
    let day_cutoff = now_ms.saturating_sub(day_ms());
    let week_cutoff = now_ms.saturating_sub(week_ms());
    let mut day = Counts::default();
    let mut week = Counts::default();
    for event in events {
        if event.at_ms > now_ms {
            continue;
        }
        if event.at_ms >= week_cutoff {
            week.add(event.kind);
        }
        if event.at_ms >= day_cutoff {
            day.add(event.kind);
        }
    }
    BreakLedgerSummary {
        window_label: "Last 24 hours".into(),
        window_seconds: 24 * 60 * 60,
        scheduled_shown: day.scheduled_shown,
        natural_idle: day.natural_idle,
        fullscreen_suppress: day.fullscreen_suppress,
        manual_take_break: day.manual_take_break,
        week_scheduled_shown: week.scheduled_shown,
        week_natural_idle: week.natural_idle,
        week_fullscreen_suppress: week.fullscreen_suppress,
        week_manual_take_break: week.manual_take_break,
    }
}

fn events_in_range(events: &[BreakEvent], start_ms: u64, end_ms: u64) -> Vec<BreakRangeRecord> {
    let mut records: Vec<_> = events
        .iter()
        .filter(|event| event.at_ms >= start_ms && event.at_ms < end_ms)
        .map(|event| BreakRangeRecord {
            at_ms: event.at_ms,
            kind: event.kind,
        })
        .collect();
    records.sort_by_key(|record| record.at_ms);
    records
}

#[derive(Default)]
struct Counts {
    scheduled_shown: u64,
    natural_idle: u64,
    fullscreen_suppress: u64,
    manual_take_break: u64,
}

impl Counts {
    fn add(&mut self, kind: BreakEventKind) {
        match kind {
            BreakEventKind::ScheduledShown => {
                self.scheduled_shown = self.scheduled_shown.saturating_add(1);
            }
            BreakEventKind::NaturalIdle => {
                self.natural_idle = self.natural_idle.saturating_add(1);
            }
            BreakEventKind::FullscreenSuppress => {
                self.fullscreen_suppress = self.fullscreen_suppress.saturating_add(1);
            }
            BreakEventKind::ManualTakeBreak => {
                self.manual_take_break = self.manual_take_break.saturating_add(1);
            }
        }
    }
}

fn events_from_persisted(ledger: PersistedBreakLedger, now_ms: u64) -> Result<Vec<BreakEvent>, ()> {
    if ledger.version != LEDGER_SCHEMA_VERSION {
        return Err(());
    }
    let mut events = ledger.events;
    // Reject obviously corrupt entries without inventing history.
    if events
        .iter()
        .any(|event| event.at_ms > now_ms || event.break_seconds == 0)
    {
        return Err(());
    }
    prune_events(&mut events, now_ms);
    Ok(events)
}

fn load_or_repair_ledger(path: &Path, now_ms: u64) -> io::Result<Vec<BreakEvent>> {
    match fs::read(path) {
        Ok(contents) => {
            let parsed = serde_json::from_slice::<PersistedBreakLedger>(&contents)
                .ok()
                .and_then(|ledger| events_from_persisted(ledger, now_ms).ok());
            if let Some(events) = parsed {
                return Ok(events);
            }
            let empty = PersistedBreakLedger {
                version: LEDGER_SCHEMA_VERSION,
                events: Vec::new(),
            };
            persist_ledger(path, &empty)?;
            Ok(Vec::new())
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(error),
    }
}

fn create_ledger_temp_file(path: &Path) -> io::Result<(PathBuf, File)> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "break ledger path has no parent",
        )
    })?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "break-events".into());

    for _ in 0..100 {
        let id = LEDGER_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
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
        "could not allocate a break ledger temporary file",
    ))
}

#[cfg(not(target_os = "windows"))]
fn replace_ledger_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    fs::rename(temp_path, path)
}

#[cfg(target_os = "windows")]
fn replace_ledger_file(temp_path: &Path, path: &Path) -> io::Result<()> {
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

fn persist_ledger(path: &Path, ledger: &PersistedBreakLedger) -> io::Result<()> {
    #[cfg(test)]
    {
        let barrier = TEST_PERSIST_BARRIER.lock().ok().and_then(|mut slot| {
            if slot.as_ref().is_some_and(|barrier| barrier.path == path) {
                slot.take()
            } else {
                None
            }
        });
        if let Some(barrier) = barrier {
            let _ = barrier.started.send(());
            let _ = barrier.release.recv();
        }
    }

    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "break ledger path has no parent",
        )
    })?;
    fs::create_dir_all(parent)?;

    let serialized = serde_json::to_vec_pretty(ledger).map_err(io::Error::other)?;
    let (temp_path, mut temp_file) = create_ledger_temp_file(path)?;
    let write_result = temp_file
        .write_all(&serialized)
        .and_then(|()| temp_file.write_all(b"\n"))
        .and_then(|()| temp_file.sync_all());
    drop(temp_file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    if let Err(error) = replace_ledger_file(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn get_break_summary(
    window: tauri::WebviewWindow,
    ledger: tauri::State<'_, BreakLedgerHandle>,
) -> Result<BreakLedgerSummary, String> {
    crate::authorize_main_caller(window.label())?;
    Ok(ledger.summary(epoch_ms(SystemTime::now())))
}

#[tauri::command]
pub(crate) fn get_break_range(
    window: tauri::WebviewWindow,
    ledger: tauri::State<'_, BreakLedgerHandle>,
    start_ms: u64,
    end_ms: u64,
) -> Result<Vec<BreakRangeRecord>, String> {
    crate::authorize_main_caller(window.label())?;
    ledger.range(start_ms, end_ms)
}

#[cfg(test)]
mod tests {
    use super::*;

    static TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            for _ in 0..100 {
                let id = TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "unfocus-break-ledger-tests-{}-{id}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("test directory should be created: {error}"),
                }
            }
            panic!("could not allocate a break ledger test directory")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    #[test]
    fn records_distinguishable_outcomes_and_counts_windows() {
        let t0 = 1_700_000_000_000_u64;
        let events = vec![
            BreakEvent {
                at_ms: t0,
                kind: BreakEventKind::ScheduledShown,
                work_minutes: 20,
                break_seconds: 20,
            },
            BreakEvent {
                at_ms: t0 + 60_000,
                kind: BreakEventKind::NaturalIdle,
                work_minutes: 20,
                break_seconds: 20,
            },
            BreakEvent {
                at_ms: t0 + 120_000,
                kind: BreakEventKind::FullscreenSuppress,
                work_minutes: 20,
                break_seconds: 20,
            },
            BreakEvent {
                at_ms: t0 + 180_000,
                kind: BreakEventKind::ManualTakeBreak,
                work_minutes: 20,
                break_seconds: 20,
            },
            // Older than one day but inside the week.
            BreakEvent {
                at_ms: t0 + 180_000 - (2 * day_ms()),
                kind: BreakEventKind::ScheduledShown,
                work_minutes: 20,
                break_seconds: 20,
            },
        ];
        let summary = summarize_events(&events, t0 + 180_000);
        assert_eq!(summary.scheduled_shown, 1);
        assert_eq!(summary.natural_idle, 1);
        assert_eq!(summary.fullscreen_suppress, 1);
        assert_eq!(summary.manual_take_break, 1);
        assert_eq!(summary.week_scheduled_shown, 2);
        assert_eq!(summary.week_natural_idle, 1);
    }

    #[test]
    fn week_counts_exclude_events_older_than_seven_days() {
        // Ten days old: older than the seven-day week window this test
        // guards, but well inside the ninety-day retention window, so the
        // event must remain in storage yet be excluded from every `week_*`
        // count. This is exactly the gap `records_distinguishable_outcomes_and_counts_windows`
        // could not catch: its two-day-old event passes under both a 7-day
        // and a 90-day cutoff, which is why the regression (week_cutoff
        // derived from retention instead of a dedicated week window) shipped
        // unnoticed. If `week_cutoff` reverts to `now_ms.saturating_sub(retention_ms())`,
        // this event falls back inside the (90-day) window and every
        // `week_*` assertion below fails.
        let now_ms = 1_700_000_000_000_u64;
        let ten_days_ago = now_ms - (10 * day_ms());
        let mut events = vec![BreakEvent {
            at_ms: ten_days_ago,
            kind: BreakEventKind::ScheduledShown,
            work_minutes: 20,
            break_seconds: 20,
        }];

        // Storage: retention is ninety days, so a ten-day-old event must not
        // be pruned.
        prune_events(&mut events, now_ms);
        assert_eq!(
            events.len(),
            1,
            "an event within the ninety-day retention window must remain in storage"
        );

        // Presentation: the dashboard's week window is seven days, so the
        // same event must not be counted there.
        let summary = summarize_events(&events, now_ms);
        assert_eq!(summary.week_scheduled_shown, 0);
        assert_eq!(summary.week_natural_idle, 0);
        assert_eq!(summary.week_fullscreen_suppress, 0);
        assert_eq!(summary.week_manual_take_break, 0);
    }

    #[test]
    fn restart_restores_events() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        let t0 = 1_700_000_000_000_u64;
        let first = BreakLedgerHandle::new_with_path(path);
        first.record(BreakEventKind::ScheduledShown, 20, 20, t0);
        first.record(BreakEventKind::NaturalIdle, 20, 20, t0 + 1_000);
        first.record(BreakEventKind::ManualTakeBreak, 25, 30, t0 + 2_000);
        drop(first);

        let reloaded = BreakLedgerHandle::load(&dir.path).expect("load");
        // load uses wall clock; re-read file at synthetic time via summarize
        // after loading raw path contents.
        let events = load_or_repair_ledger(&dir.path.join(LEDGER_FILE_NAME), t0 + 2_000)
            .expect("reload at synthetic clock");
        let summary = summarize_events(&events, t0 + 2_000);
        assert_eq!(summary.scheduled_shown, 1);
        assert_eq!(summary.natural_idle, 1);
        assert_eq!(summary.manual_take_break, 1);
        let _ = reloaded;
    }

    #[test]
    fn delayed_persistence_does_not_delay_record() {
        use std::{sync::mpsc, thread, time::Duration};

        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        let handle = BreakLedgerHandle::new_with_path(path.clone());
        let (started_tx, started_rx) = mpsc::channel();
        let (release_tx, release_rx) = mpsc::channel();
        *TEST_PERSIST_BARRIER.lock().expect("barrier lock") = Some(TestPersistBarrier {
            path,
            started: started_tx,
            release: release_rx,
        });
        let (recorded_tx, recorded_rx) = mpsc::channel();

        let recorder = thread::spawn(move || {
            handle.record(BreakEventKind::ScheduledShown, 20, 20, 1_700_000_000_000);
            let _ = recorded_tx.send(());
        });

        started_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("persistence should start");
        let returned_while_persistence_was_blocked =
            recorded_rx.recv_timeout(Duration::from_millis(100)).is_ok();
        release_tx.send(()).expect("release persistence");
        recorder.join().expect("record thread");

        assert!(
            returned_while_persistence_was_blocked,
            "record must return without waiting for persistence"
        );
    }

    #[test]
    fn prune_drops_events_outside_retention() {
        let t0 = 1_700_000_000_000_u64;
        let mut events = vec![
            BreakEvent {
                at_ms: t0,
                kind: BreakEventKind::ScheduledShown,
                work_minutes: 20,
                break_seconds: 20,
            },
            BreakEvent {
                at_ms: t0.saturating_sub(retention_ms() + 1_000),
                kind: BreakEventKind::NaturalIdle,
                work_minutes: 20,
                break_seconds: 20,
            },
        ];

        prune_events(&mut events, t0);

        assert_eq!(events.len(), 1);
        assert_eq!(events[0].kind, BreakEventKind::ScheduledShown);
    }

    #[test]
    fn retention_keeps_ninety_days_at_minimum_work_interval() {
        const EXPECTED_EVENTS: usize = 90 * 24 * 60;
        const ONE_MINUTE_MS: u64 = 60_000;
        let t0 = 1_700_000_000_000_u64;
        let mut events = Vec::with_capacity(EXPECTED_EVENTS);
        for index in 0..EXPECTED_EVENTS {
            events.push(BreakEvent {
                at_ms: t0 + index as u64 * ONE_MINUTE_MS,
                kind: BreakEventKind::ScheduledShown,
                work_minutes: 1,
                break_seconds: 20,
            });
        }
        let now_ms = t0 + EXPECTED_EVENTS as u64 * ONE_MINUTE_MS;

        prune_events(&mut events, now_ms);

        assert_eq!(events.len(), EXPECTED_EVENTS);
    }

    #[test]
    fn failed_write_leaves_previous_complete_ledger() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        let t0 = 1_700_000_000_000_u64;
        let original = PersistedBreakLedger {
            version: LEDGER_SCHEMA_VERSION,
            events: vec![BreakEvent {
                at_ms: t0,
                kind: BreakEventKind::ScheduledShown,
                work_minutes: 20,
                break_seconds: 20,
            }],
        };
        persist_ledger(&path, &original).expect("seed");
        let bytes = fs::read(&path).expect("read");

        let blocker = dir.path.join("not-a-directory");
        fs::write(&blocker, b"x").expect("blocker");
        let nested = blocker.join(LEDGER_FILE_NAME);
        let result = persist_ledger(
            &nested,
            &PersistedBreakLedger {
                version: LEDGER_SCHEMA_VERSION,
                events: vec![BreakEvent {
                    at_ms: t0 + 1,
                    kind: BreakEventKind::NaturalIdle,
                    work_minutes: 20,
                    break_seconds: 20,
                }],
            },
        );
        assert!(result.is_err());
        assert_eq!(fs::read(&path).expect("previous remains"), bytes);
    }

    #[test]
    fn malformed_ledger_is_replaced_with_empty_complete_file() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        fs::write(&path, b"{nope").expect("garbage");
        let events = load_or_repair_ledger(&path, 1_700_000_000_000).expect("repair");
        assert!(events.is_empty());
        let repaired: PersistedBreakLedger =
            serde_json::from_slice(&fs::read(&path).expect("read")).expect("json");
        assert_eq!(repaired.version, LEDGER_SCHEMA_VERSION);
        assert!(repaired.events.is_empty());
    }

    #[test]
    fn future_timestamps_are_rejected_on_load() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        let now = 1_700_000_000_000_u64;
        persist_ledger(
            &path,
            &PersistedBreakLedger {
                version: LEDGER_SCHEMA_VERSION,
                events: vec![BreakEvent {
                    at_ms: now + 60_000,
                    kind: BreakEventKind::ScheduledShown,
                    work_minutes: 20,
                    break_seconds: 20,
                }],
            },
        )
        .expect("seed future");
        let events = load_or_repair_ledger(&path, now).expect("repair");
        assert!(events.is_empty());
    }

    #[test]
    fn range_rejects_empty_or_reversed_windows() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        let handle = BreakLedgerHandle::new_with_path(path);
        let t0 = 1_700_000_000_000_u64;

        assert!(
            handle.range(t0, t0).is_err(),
            "empty windows must be rejected"
        );
        assert!(
            handle.range(t0 + 1, t0).is_err(),
            "reversed windows must be rejected"
        );
    }

    #[test]
    fn range_rejects_windows_longer_than_thirty_one_days() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        let handle = BreakLedgerHandle::new_with_path(path);
        let t0 = 1_700_000_000_000_u64;
        let thirty_one_days_ms = 31 * day_ms();

        assert!(
            handle.range(t0, t0 + thirty_one_days_ms).is_ok(),
            "exactly thirty-one elapsed days must be accepted"
        );
        assert!(
            handle.range(t0, t0 + thirty_one_days_ms + 1).is_err(),
            "windows wider than thirty-one elapsed days must be rejected"
        );
    }

    #[test]
    fn range_returns_filtered_chronological_privacy_preserving_records() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        let handle = BreakLedgerHandle::new_with_path(path);
        let t0 = 1_700_000_000_000_u64;

        handle.inner.lock().expect("lock").events = vec![
            BreakEvent {
                at_ms: t0 + 120_000,
                kind: BreakEventKind::ManualTakeBreak,
                work_minutes: 25,
                break_seconds: 30,
            },
            BreakEvent {
                at_ms: t0 + 20_000,
                kind: BreakEventKind::ScheduledShown,
                work_minutes: 20,
                break_seconds: 20,
            },
            BreakEvent {
                at_ms: t0 + 80_000,
                kind: BreakEventKind::NaturalIdle,
                work_minutes: 15,
                break_seconds: 10,
            },
            BreakEvent {
                at_ms: t0 + 200_000,
                kind: BreakEventKind::FullscreenSuppress,
                work_minutes: 20,
                break_seconds: 20,
            },
        ];

        let events = handle
            .range(t0 + 10_000, t0 + 150_000)
            .expect("range succeeds");
        let serialized = serde_json::to_value(&events).expect("serialize range");

        assert_eq!(
            serialized,
            serde_json::json!([
                {
                    "atMs": t0 + 20_000,
                    "kind": "scheduledShown"
                },
                {
                    "atMs": t0 + 80_000,
                    "kind": "naturalIdle"
                },
                {
                    "atMs": t0 + 120_000,
                    "kind": "manualTakeBreak"
                }
            ])
        );
    }
}
