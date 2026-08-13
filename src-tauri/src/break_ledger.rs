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
        Arc, Mutex,
    },
    time::{SystemTime, UNIX_EPOCH},
};

const LEDGER_FILE_NAME: &str = "break-events.json";
const LEDGER_SCHEMA_VERSION: u32 = 1;
/// Keep at least ninety days of outcomes, matching the activity history
/// retention (`activity.rs`'s `HISTORY_RETENTION_SECONDS`) so week/day counts
/// stay backed by data for as long as the day-by-hour history the frontend
/// will eventually read alongside them.
const RETENTION_SECONDS: u64 = 90 * 24 * 60 * 60;
/// Bounds a pathological event rate; retention above is what normally bounds
/// the file. `512` was a live defect: a 20-minute work interval yields up to
/// 72 scheduled breaks a day, so with natural-idle and fullscreen-suppress
/// events the old cap could bite inside even the prior 7-day window, silently
/// undercounting the dashboard's week totals. At the densest plausible rate
/// (a 1-minute work interval, 1,440 scheduled breaks a day) retention still
/// bounds the file for the first several days before this cap would; a
/// genuinely pathological rate is still stopped.
const MAX_EVENTS: usize = 16_384;
const MILLIS_PER_SECOND: u64 = 1_000;

static LEDGER_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

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

#[derive(Debug)]
struct BreakLedgerState {
    path: PathBuf,
    events: Vec<BreakEvent>,
}

#[derive(Debug, Clone)]
pub(crate) struct BreakLedgerHandle {
    inner: Arc<Mutex<BreakLedgerState>>,
}

impl Default for BreakLedgerHandle {
    fn default() -> Self {
        Self {
            inner: Arc::new(Mutex::new(BreakLedgerState {
                path: PathBuf::from(LEDGER_FILE_NAME),
                events: Vec::new(),
            })),
        }
    }
}

impl BreakLedgerHandle {
    pub(crate) fn load(config_dir: &Path) -> io::Result<Self> {
        let path = config_dir.join(LEDGER_FILE_NAME);
        let now_ms = epoch_ms(SystemTime::now());
        let events = load_or_repair_ledger(&path, now_ms)?;
        Ok(Self {
            inner: Arc::new(Mutex::new(BreakLedgerState { path, events })),
        })
    }

    #[cfg(test)]
    fn new_with_path(path: PathBuf) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BreakLedgerState {
                path,
                events: Vec::new(),
            })),
        }
    }

    /// Append one outcome. Persist failures are logged; memory still updates.
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
        state.events.push(BreakEvent {
            at_ms: now_ms,
            kind,
            work_minutes,
            break_seconds,
        });
        prune_events(&mut state.events, now_ms);
        if let Err(error) = persist_ledger(
            &state.path,
            &PersistedBreakLedger {
                version: LEDGER_SCHEMA_VERSION,
                events: state.events.clone(),
            },
        ) {
            eprintln!("could not persist break event ledger: {error}");
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

fn prune_events(events: &mut Vec<BreakEvent>, now_ms: u64) {
    let cutoff = now_ms.saturating_sub(retention_ms());
    events.retain(|event| event.at_ms >= cutoff && event.at_ms <= now_ms);
    if events.len() > MAX_EVENTS {
        let drop = events.len() - MAX_EVENTS;
        events.drain(0..drop);
    }
}

fn summarize_events(events: &[BreakEvent], now_ms: u64) -> BreakLedgerSummary {
    let day_cutoff = now_ms.saturating_sub(day_ms());
    let week_cutoff = now_ms.saturating_sub(retention_ms());
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
    fn restart_restores_events() {
        let dir = TestDirectory::new();
        let path = dir.path.join(LEDGER_FILE_NAME);
        let t0 = 1_700_000_000_000_u64;
        let first = BreakLedgerHandle::new_with_path(path);
        first.record(BreakEventKind::ScheduledShown, 20, 20, t0);
        first.record(BreakEventKind::NaturalIdle, 20, 20, t0 + 1_000);
        first.record(BreakEventKind::ManualTakeBreak, 25, 30, t0 + 2_000);

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
    fn prune_drops_events_outside_retention_and_caps_length() {
        let t0 = 1_700_000_000_000_u64;
        let mut events = Vec::new();
        for index in 0..(MAX_EVENTS + 40) {
            events.push(BreakEvent {
                at_ms: t0 + index as u64,
                kind: BreakEventKind::ScheduledShown,
                work_minutes: 20,
                break_seconds: 20,
            });
        }
        // One event far outside retention.
        events.push(BreakEvent {
            at_ms: t0.saturating_sub(retention_ms() + 1_000),
            kind: BreakEventKind::NaturalIdle,
            work_minutes: 20,
            break_seconds: 20,
        });
        prune_events(&mut events, t0 + MAX_EVENTS as u64 + 40);
        assert!(events.len() <= MAX_EVENTS);
        assert!(events
            .iter()
            .all(|event| event.kind != BreakEventKind::NaturalIdle
                || event.at_ms >= (t0 + MAX_EVENTS as u64 + 40).saturating_sub(retention_ms())));
    }

    #[test]
    fn retention_keeps_ninety_days_at_a_realistic_rate() {
        // ~80 events a day (scheduled breaks plus natural-idle and
        // fullscreen-suppress outcomes) is a realistic heavy-use rate, well
        // above a 20-minute interval's ~72 scheduled breaks alone. Ninety
        // days at that rate is 7,200 events, far under MAX_EVENTS, so
        // retention alone must decide what stays; nothing inside the window
        // may be dropped by the cap.
        const EVENTS_PER_DAY: u64 = 80;
        const DAYS: u64 = 90;
        let spacing_ms = day_ms() / EVENTS_PER_DAY;
        let t0 = 1_700_000_000_000_u64;
        let total = EVENTS_PER_DAY * DAYS;
        let mut events = Vec::with_capacity(total as usize);
        for index in 0..total {
            events.push(BreakEvent {
                at_ms: t0 + index * spacing_ms,
                kind: BreakEventKind::ScheduledShown,
                work_minutes: 20,
                break_seconds: 20,
            });
        }
        let now_ms = t0 + (total - 1) * spacing_ms;

        prune_events(&mut events, now_ms);

        assert!(
            (total as usize) < MAX_EVENTS,
            "test rate must stay realistic, not already exceed the cap"
        );
        assert_eq!(
            events.len(),
            total as usize,
            "no event inside the ninety-day window should be dropped"
        );
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
}
