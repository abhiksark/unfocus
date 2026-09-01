//! Local-only ledger of break outcomes (shown, natural, suppressed, manual).
//!
//! Observe-only for the timer: write failures never stop reminders. No
//! keylogging, window titles, or telemetry.

use crate::storage_recovery::{
    canonical_bytes_unchanged, create_new_file_with_permissions, existing_file_permissions,
    quarantine_invalid_hard_link, replace_file_atomically, LoadFailure, LocalSnapshot,
    StorageDiagnostic, StorageFailureCategory, StorageLoadHealth,
};
use serde::{Deserialize, Serialize};
use std::{
    fs::{self, File},
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
static TEST_LEDGER_PERSIST_FAILURE: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(test)]
struct TestPersistBarrier {
    path: PathBuf,
    started: std::sync::mpsc::Sender<()>,
    release: std::sync::mpsc::Receiver<()>,
}

#[cfg(test)]
static TEST_PERSIST_BARRIER: Mutex<Option<TestPersistBarrier>> = Mutex::new(None);
#[cfg(test)]
static TEST_WORKER_START_FAILURES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

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
struct BreakLedgerRuntime {
    events: Vec<BreakEvent>,
    persistence: Arc<PersistenceWorker>,
}

#[derive(Debug)]
enum BreakLedgerStorageState {
    Available(BreakLedgerRuntime),
    Unavailable(LoadFailure),
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
    inner: Arc<Mutex<BreakLedgerStorageState>>,
    path: Arc<PathBuf>,
    recovery: Arc<Mutex<()>>,
}

impl BreakLedgerHandle {
    pub(crate) fn initialize(config_dir: &Path) -> Self {
        Self::initialize_at(config_dir, epoch_ms(SystemTime::now()))
    }

    fn initialize_at(config_dir: &Path, now_ms: u64) -> Self {
        let path = config_dir.join(LEDGER_FILE_NAME);
        let state = match load_ledger_runtime(&path, now_ms) {
            Ok(runtime) => BreakLedgerStorageState::Available(runtime),
            Err(failure) => BreakLedgerStorageState::Unavailable(failure),
        };
        Self {
            inner: Arc::new(Mutex::new(state)),
            path: Arc::new(path),
            recovery: Arc::new(Mutex::new(())),
        }
    }

    #[cfg(test)]
    fn load(config_dir: &Path) -> io::Result<Self> {
        let handle = Self::initialize(config_dir);
        if handle.is_available() {
            Ok(handle)
        } else {
            Err(io::Error::other("break ledger unavailable"))
        }
    }

    #[cfg(test)]
    fn new_with_path(path: PathBuf) -> Self {
        let runtime = start_persistence_worker(path.clone(), Vec::new()).expect("worker starts");
        Self {
            inner: Arc::new(Mutex::new(BreakLedgerStorageState::Available(runtime))),
            path: Arc::new(path),
            recovery: Arc::new(Mutex::new(())),
        }
    }

    /// Append one outcome in memory and queue it for persistence. While the
    /// canonical ledger is unavailable, outcomes are deliberately not
    /// invented in memory and no worker exists to write another file.
    pub(crate) fn record(
        &self,
        kind: BreakEventKind,
        work_minutes: u64,
        break_seconds: u64,
        now_ms: u64,
    ) {
        let Ok(mut storage) = self.inner.lock() else {
            return;
        };
        let BreakLedgerStorageState::Available(runtime) = &mut *storage else {
            return;
        };
        let event = BreakEvent {
            at_ms: now_ms,
            kind,
            work_minutes,
            break_seconds,
        };
        runtime.events.push(event.clone());
        prune_events(&mut runtime.events, now_ms);
        let queued = runtime
            .persistence
            .sender
            .send(PersistenceMessage::Record(event))
            .is_ok();
        drop(storage);
        if !queued {
            eprintln!("could not queue break event for persistence; keeping it in memory");
        }
    }

    pub(crate) fn snapshot(&self, now_ms: u64) -> LocalSnapshot<BreakLedgerSummary> {
        let Ok(storage) = self.inner.lock() else {
            return LocalSnapshot::unavailable(StorageFailureCategory::Read);
        };
        match &*storage {
            BreakLedgerStorageState::Available(runtime) => {
                LocalSnapshot::available(summarize_events(&runtime.events, now_ms))
            }
            BreakLedgerStorageState::Unavailable(failure) => {
                LocalSnapshot::unavailable(failure.category)
            }
        }
    }

    pub(crate) fn diagnostics(&self) -> StorageDiagnostic {
        let Ok(storage) = self.inner.lock() else {
            return LoadFailure::read("break ledger state lock is poisoned").diagnostic();
        };
        match &*storage {
            BreakLedgerStorageState::Available(_) => StorageDiagnostic::available(),
            BreakLedgerStorageState::Unavailable(failure) => failure.diagnostic(),
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
            .map_err(|_| "break history recovery is unavailable".to_owned())?;
        if self.failure_category()? != StorageFailureCategory::Invalid {
            return Err("start-new is only available for invalid break history".into());
        }

        let contents = fs::read(&*self.path).map_err(|error| {
            self.publish_failure(LoadFailure::read(format!(
                "could not re-read {} before recovery: {error}",
                self.path.display()
            )));
            "break history could not be read; retry is still available".to_owned()
        })?;

        if ledger_from_bytes(&contents, now_ms).is_ok() {
            return Ok(self.retry_load_locked(now_ms));
        }

        quarantine_invalid_hard_link(&self.path, &contents).map_err(|error| {
            self.publish_failure(LoadFailure::invalid(format!(
                "could not preserve invalid {}: {error}",
                self.path.display()
            )));
            "invalid break history could not be preserved".to_owned()
        })?;
        let empty = PersistedBreakLedger {
            version: LEDGER_SCHEMA_VERSION,
            events: Vec::new(),
        };
        let temp_path = prepare_ledger_file(&self.path, &empty).map_err(|error| {
            self.publish_failure(LoadFailure::invalid(format!(
                "invalid {} was preserved, but a new ledger could not be prepared: {error}",
                self.path.display()
            )));
            "a new break history could not be started".to_owned()
        })?;
        let unchanged = match canonical_bytes_unchanged(&self.path, &contents) {
            Ok(unchanged) => unchanged,
            Err(error) => {
                let _ = fs::remove_file(&temp_path);
                self.publish_failure(LoadFailure::read(format!(
                    "could not complete the final canonical recheck for {}: {error}",
                    self.path.display()
                )));
                return Err(
                    "break history could not be rechecked; retry is still available".into(),
                );
            }
        };
        if !unchanged {
            let _ = fs::remove_file(&temp_path);
            return Err(
                "break history changed while recovery was being prepared; retry to load it"
                    .to_owned(),
            );
        }
        let runtime =
            start_persistence_worker((*self.path).clone(), Vec::new()).map_err(|_error| {
                let _ = fs::remove_file(&temp_path);
                "break history persistence could not start".to_owned()
            })?;
        let mut storage = self.inner.lock().map_err(|_| {
            let _ = fs::remove_file(&temp_path);
            "break history recovery is unavailable".to_owned()
        })?;
        if let Err(error) = replace_file_atomically(&temp_path, &self.path) {
            let _ = fs::remove_file(&temp_path);
            return Err(format!("a new break history could not be started: {error}"));
        }
        *storage = BreakLedgerStorageState::Available(runtime);
        Ok(StorageLoadHealth::available())
    }

    fn retry_load_locked(&self, now_ms: u64) -> StorageLoadHealth {
        match load_ledger_runtime(&self.path, now_ms) {
            Ok(runtime) => {
                if let Ok(mut storage) = self.inner.lock() {
                    *storage = BreakLedgerStorageState::Available(runtime);
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
            .is_ok_and(|storage| matches!(&*storage, BreakLedgerStorageState::Available(_)))
    }

    fn failure_category(&self) -> Result<StorageFailureCategory, String> {
        let storage = self
            .inner
            .lock()
            .map_err(|_| "break history recovery is unavailable".to_owned())?;
        match &*storage {
            BreakLedgerStorageState::Available(_) => {
                Err("break history is already available".into())
            }
            BreakLedgerStorageState::Unavailable(failure) => Ok(failure.category),
        }
    }

    fn publish_failure(&self, failure: LoadFailure) {
        if let Ok(mut storage) = self.inner.lock() {
            *storage = BreakLedgerStorageState::Unavailable(failure);
        }
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

        let storage = self
            .inner
            .lock()
            .map_err(|_| "break history is unavailable".to_owned())?;
        let BreakLedgerStorageState::Available(runtime) = &*storage else {
            return Err("break history is unavailable".into());
        };
        Ok(events_in_range(&runtime.events, start_ms, end_ms))
    }
}

fn start_persistence_worker(
    path: PathBuf,
    events: Vec<BreakEvent>,
) -> io::Result<BreakLedgerRuntime> {
    #[cfg(test)]
    if TEST_WORKER_START_FAILURES.lock().is_ok_and(|mut targets| {
        targets
            .iter()
            .position(|target| target == &path)
            .map(|index| targets.remove(index))
            .is_some()
    }) {
        return Err(io::Error::other("injected worker startup failure"));
    }

    let (sender, receiver) = mpsc::channel();
    let worker_events = events.clone();
    let join = thread::Builder::new()
        .name("break-ledger-persistence".into())
        .spawn(move || persistence_worker(path, worker_events, receiver))?;
    Ok(BreakLedgerRuntime {
        events,
        persistence: Arc::new(PersistenceWorker {
            sender,
            join: Some(join),
        }),
    })
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

fn ledger_from_bytes(contents: &[u8], now_ms: u64) -> Result<Vec<BreakEvent>, LoadFailure> {
    let ledger = serde_json::from_slice::<PersistedBreakLedger>(contents).map_err(|error| {
        LoadFailure::invalid(format!("break ledger content is malformed: {error}"))
    })?;
    events_from_persisted(ledger, now_ms)
        .map_err(|()| LoadFailure::invalid("break ledger content is unsupported or invalid"))
}

fn load_ledger(path: &Path, now_ms: u64) -> Result<Vec<BreakEvent>, LoadFailure> {
    match fs::read(path) {
        Ok(contents) => ledger_from_bytes(&contents, now_ms),
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(error) => Err(LoadFailure::read(format!(
            "could not read {}: {error}",
            path.display()
        ))),
    }
}

fn load_ledger_runtime(path: &Path, now_ms: u64) -> Result<BreakLedgerRuntime, LoadFailure> {
    let events = load_ledger(path, now_ms)?;
    start_persistence_worker(path.to_path_buf(), events).map_err(|error| {
        LoadFailure::read(format!(
            "could not start persistence for {}: {error}",
            path.display()
        ))
    })
}

#[cfg(test)]
fn load_or_repair_ledger(path: &Path, now_ms: u64) -> Result<Vec<BreakEvent>, LoadFailure> {
    load_ledger(path, now_ms)
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
    let permissions = existing_file_permissions(path)?;

    for _ in 0..100 {
        let id = LEDGER_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
        let temp_path = parent.join(format!(".{name}.{}.{id}.tmp", std::process::id()));
        match create_new_file_with_permissions(&temp_path, permissions.as_ref()) {
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

fn prepare_ledger_file(path: &Path, ledger: &PersistedBreakLedger) -> io::Result<PathBuf> {
    #[cfg(test)]
    if TEST_LEDGER_PERSIST_FAILURE.lock().is_ok_and(|mut target| {
        if target.as_deref() == Some(path) {
            target.take();
            true
        } else {
            false
        }
    }) {
        return Err(io::Error::other("injected break ledger write failure"));
    }

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
    Ok(temp_path)
}

fn persist_ledger(path: &Path, ledger: &PersistedBreakLedger) -> io::Result<()> {
    let temp_path = prepare_ledger_file(path, ledger)?;
    if let Err(error) = replace_file_atomically(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn get_break_summary(
    window: tauri::WebviewWindow,
    ledger: tauri::State<'_, BreakLedgerHandle>,
) -> Result<LocalSnapshot<BreakLedgerSummary>, String> {
    crate::authorize_main_caller(window.label())?;
    Ok(ledger.snapshot(epoch_ms(SystemTime::now())))
}

#[tauri::command]
pub(crate) async fn retry_break_ledger(
    window: tauri::WebviewWindow,
    ledger: tauri::State<'_, BreakLedgerHandle>,
) -> Result<StorageLoadHealth, String> {
    crate::authorize_main_caller(window.label())?;
    let ledger = ledger.inner().clone();
    tauri::async_runtime::spawn_blocking(move || ledger.retry_load(epoch_ms(SystemTime::now())))
        .await
        .map_err(|_| "break history retry could not run".to_owned())
}

#[tauri::command]
pub(crate) async fn start_new_break_ledger(
    window: tauri::WebviewWindow,
    ledger: tauri::State<'_, BreakLedgerHandle>,
) -> Result<StorageLoadHealth, String> {
    crate::authorize_main_caller(window.label())?;
    let ledger = ledger.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        ledger.start_new_after_invalid(epoch_ms(SystemTime::now()))
    })
    .await
    .map_err(|_| "break history recovery could not run".to_owned())?
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
    fn malformed_ledger_is_preserved_and_unavailable() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        let original = b"{nope";
        fs::write(&path, original).expect("garbage");

        let handle = BreakLedgerHandle::initialize_at(&dir.path, 1_700_000_000_000);

        assert_eq!(fs::read(&path).expect("read"), original);
        let snapshot = handle.snapshot(1_700_000_000_000);
        assert!(snapshot.data.is_none());
        assert_eq!(
            snapshot.load_health.recovery,
            crate::storage_recovery::StorageRecovery::RetryOrStartNew
        );
        assert!(handle.range(1, 2).is_err());
    }

    #[test]
    fn read_failure_stays_unavailable_until_retry_starts_one_canonical_worker() {
        let dir = TestDirectory::new();
        let blocked_config = dir.path.join("blocked-config");
        fs::write(&blocked_config, b"blocker").expect("block config directory");
        let canonical = blocked_config.join(LEDGER_FILE_NAME);
        let now = 1_700_000_000_000;
        let handle = BreakLedgerHandle::initialize_at(&blocked_config, now);

        assert_eq!(&*handle.path, &canonical);
        assert_eq!(
            handle.snapshot(now).load_health.recovery,
            crate::storage_recovery::StorageRecovery::Retry
        );
        handle.record(BreakEventKind::ScheduledShown, 20, 20, now);
        assert_eq!(
            fs::read(&blocked_config).expect("blocker untouched"),
            b"blocker"
        );
        assert!(handle.range(now, now + 1).is_err());
        assert!(handle.start_new_after_invalid(now).is_err());

        fs::remove_file(&blocked_config).expect("remove blocker");
        fs::create_dir(&blocked_config).expect("create config directory");
        persist_ledger(
            &canonical,
            &PersistedBreakLedger {
                version: LEDGER_SCHEMA_VERSION,
                events: Vec::new(),
            },
        )
        .expect("write repaired ledger");

        assert_eq!(handle.retry_load(now), StorageLoadHealth::available());
        assert!(handle.snapshot(now).data.is_some());
        handle.record(BreakEventKind::ManualTakeBreak, 20, 20, now);
        drop(handle);
        let persisted: PersistedBreakLedger =
            serde_json::from_slice(&fs::read(canonical).expect("persisted record"))
                .expect("valid ledger");
        assert_eq!(persisted.events.len(), 1);
    }

    #[test]
    fn failed_retry_preserves_invalid_ledger_bytes_and_state() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        let original = b"invalid ledger";
        fs::write(&path, original).expect("seed invalid ledger");
        let now = 1_700_000_000_000;
        let handle = BreakLedgerHandle::initialize_at(&dir.path, now);

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
    fn invalid_start_new_quarantines_exact_bytes_and_writes_empty_ledger() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        let original = b"invalid ledger bytes\0";
        fs::write(&path, original).expect("seed invalid ledger");
        let now = 1_700_000_000_000;
        let handle = BreakLedgerHandle::initialize_at(&dir.path, now);

        assert_eq!(
            handle.start_new_after_invalid(now).expect("start new"),
            StorageLoadHealth::available()
        );

        let persisted: PersistedBreakLedger =
            serde_json::from_slice(&fs::read(&path).expect("new canonical")).expect("valid empty");
        assert_eq!(persisted.version, LEDGER_SCHEMA_VERSION);
        assert!(persisted.events.is_empty());
        let quarantines: Vec<_> = fs::read_dir(&dir.path)
            .expect("siblings")
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("break-events.json.invalid-")
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
        let path = dir.path.join(LEDGER_FILE_NAME);
        fs::write(&path, b"invalid ledger bytes").expect("seed invalid ledger");
        fs::set_permissions(&path, fs::Permissions::from_mode(0o600))
            .expect("restrict canonical permissions");
        let now = 1_700_000_000_000;
        let handle = BreakLedgerHandle::initialize_at(&dir.path, now);

        handle
            .start_new_after_invalid(now)
            .expect("start new break history");

        assert_eq!(
            fs::metadata(path).expect("metadata").permissions().mode() & 0o7777,
            0o600
        );
    }

    #[test]
    fn failed_quarantine_or_replacement_preserves_invalid_ledger_bytes() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        let original = b"invalid ledger bytes";
        fs::write(&path, original).expect("seed invalid ledger");
        let now = 1_700_000_000_000;
        let handle = BreakLedgerHandle::initialize_at(&dir.path, now);
        crate::storage_recovery::TEST_QUARANTINE_FAILURES
            .lock()
            .expect("hook")
            .push(path.clone());

        assert!(handle.start_new_after_invalid(now).is_err());
        assert_eq!(fs::read(&path).expect("after quarantine failure"), original);

        *TEST_LEDGER_PERSIST_FAILURE.lock().expect("hook") = Some(path.clone());
        assert!(handle.start_new_after_invalid(now).is_err());
        assert_eq!(
            fs::read(&path).expect("after replacement failure"),
            original
        );
        assert!(handle.snapshot(now).data.is_none());
    }

    #[test]
    fn start_new_worker_failure_preserves_invalid_canonical_and_unavailable_state() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        let original = b"invalid ledger bytes";
        fs::write(&path, original).expect("seed invalid ledger");
        let now = 1_700_000_000_000;
        let handle = BreakLedgerHandle::initialize_at(&dir.path, now);
        let before = handle.diagnostics();
        TEST_WORKER_START_FAILURES
            .lock()
            .expect("hook")
            .push(path.clone());

        assert!(handle.start_new_after_invalid(now).is_err());

        assert_eq!(fs::read(&path).expect("canonical unchanged"), original);
        assert!(handle.snapshot(now).data.is_none());
        let after = handle.diagnostics();
        assert_eq!(after.status, before.status);
        assert_eq!(after.recovery, before.recovery);
        assert_eq!(after.category, before.category);
        assert_eq!(after.error, before.error);
    }

    #[test]
    fn concurrent_external_ledger_repair_is_not_overwritten() {
        use std::time::Duration;

        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        fs::write(&path, b"invalid ledger bytes").expect("seed invalid ledger");
        let now = 1_700_000_000_000;
        let handle = BreakLedgerHandle::initialize_at(&dir.path, now);
        let repaired = PersistedBreakLedger {
            version: LEDGER_SCHEMA_VERSION,
            events: Vec::new(),
        };
        let (started, release) = crate::storage_recovery::install_replacement_barrier(path.clone());
        let recovering = handle.clone();
        let recovery = std::thread::spawn(move || recovering.start_new_after_invalid(now));
        started
            .recv_timeout(Duration::from_secs(1))
            .expect("recovery reaches final validation");

        persist_ledger(&path, &repaired).expect("external repair replaces canonical file");
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
        let path = dir.path.join(LEDGER_FILE_NAME);
        fs::write(&path, b"invalid ledger bytes").expect("seed invalid ledger");
        let now = 1_700_000_000_000;
        let handle = BreakLedgerHandle::initialize_at(&dir.path, now);
        let (started, release) = crate::storage_recovery::install_replacement_barrier(path.clone());
        let recovering = handle.clone();
        let recovery = std::thread::spawn(move || recovering.start_new_after_invalid(now));
        started
            .recv_timeout(Duration::from_secs(1))
            .expect("recovery reaches final canonical recheck");

        fs::remove_file(&path).expect("remove canonical ledger");
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
    fn worker_start_failure_is_unavailable_and_retry_recovers() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        persist_ledger(
            &path,
            &PersistedBreakLedger {
                version: LEDGER_SCHEMA_VERSION,
                events: Vec::new(),
            },
        )
        .expect("seed valid ledger");
        TEST_WORKER_START_FAILURES
            .lock()
            .expect("hook")
            .push(path.clone());

        let handle = BreakLedgerHandle::initialize_at(&dir.path, 1_700_000_000_000);

        assert!(handle.snapshot(1_700_000_000_000).data.is_none());
        assert_eq!(
            handle.retry_load(1_700_000_000_000),
            StorageLoadHealth::available()
        );
        assert!(handle.snapshot(1_700_000_000_000).data.is_some());
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
        let original = fs::read(&path).expect("read future bytes");
        let failure = load_or_repair_ledger(&path, now).expect_err("future event is invalid");
        assert_eq!(failure.category, StorageFailureCategory::Invalid);
        assert_eq!(fs::read(&path).expect("bytes remain"), original);
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

        let mut storage = handle.inner.lock().expect("lock");
        let BreakLedgerStorageState::Available(runtime) = &mut *storage else {
            panic!("test ledger should be available");
        };
        runtime.events = vec![
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
        drop(storage);

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
