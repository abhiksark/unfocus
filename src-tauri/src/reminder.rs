// src-tauri/src/reminder.rs

mod schedule;

use crate::{
    activity::{
        epoch_ms, ActivityPresentationContext, ActivityTrackerHandle, LONG_ACTIVE_SECONDS,
        LONG_AFK_SECONDS,
    },
    authorize_main_caller,
    break_ledger::{BreakEventKind, BreakLedgerHandle},
    overlay::{
        show_overlay, show_overlay_if_idle, OverlayController, MAX_OVERLAY_DURATION_SECONDS,
        MIN_OVERLAY_DURATION_SECONDS,
    },
    pre_break_cue::{PreBreakCue, CUE_LEAD_MILLISECONDS},
    probes::{qualified_x11_session, ProbeCache, ProbeReading, ProbeSnapshot},
    storage_recovery::{
        canonical_bytes_unchanged, create_new_file_with_permissions, existing_file_permissions,
        quarantine_invalid_hard_link, replace_file_atomically, LoadFailure, LocalSnapshot,
        StorageDiagnostic, StorageFailureCategory, StorageLoadHealth,
    },
    tray::{TrayPhase, TraySnapshot, TrayStatus},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{AppHandle, State, WebviewWindow};

const DEFAULT_WORK_MINUTES: u64 = 20;
const DEFAULT_BREAK_SECONDS: u64 = 20;
const MIN_WORK_MINUTES: u64 = 1;
const MAX_WORK_MINUTES: u64 = 120;
pub(crate) const PAUSE_DURATION_MINUTES: u64 = 30;
const PAUSE_DURATION: Duration = Duration::from_secs(PAUSE_DURATION_MINUTES * 60);
const REMINDER_POLL_INTERVAL: Duration = Duration::from_millis(250);
/// How far the wall clock and the injected monotonic clock may drift between
/// scheduler iterations before it counts as a discontinuity rather than
/// ordinary polling jitter. Mirrors `lifecycle_contract::discontinuity_observation`.
const CLOCK_DIVERGENCE_TOLERANCE: Duration = Duration::from_secs(5);
const REMINDER_CONTROL_CAPACITY: usize = 16;
const REMINDER_CONTROL_TIMEOUT: Duration = Duration::from_secs(10);
const SETTINGS_FILE_NAME: &str = "reminder-settings.json";
const SETTINGS_SCHEMA_VERSION: u32 = 4;
const MIN_GRID_OFFSET_MINUTES: i16 = -720; // UTC-12:00
const MAX_GRID_OFFSET_MINUTES: i16 = 840; //  UTC+14:00

static SETTINGS_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);
#[cfg(test)]
static TEST_SETTINGS_PERSIST_FAILURES: Mutex<Vec<PathBuf>> = Mutex::new(Vec::new());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReminderSettings {
    work_minutes: u64,
    break_seconds: u64,
    sync_across_devices: bool,
    grid_offset_minutes: i16,
    pre_break_cue_enabled: bool,
}

impl ReminderSettings {
    fn try_new(
        work_minutes: u64,
        break_seconds: u64,
        sync_across_devices: bool,
        grid_offset_minutes: i16,
    ) -> Result<Self, String> {
        if !(MIN_WORK_MINUTES..=MAX_WORK_MINUTES).contains(&work_minutes) {
            return Err(format!(
                "work duration must be between {MIN_WORK_MINUTES} and {MAX_WORK_MINUTES} minutes"
            ));
        }
        if !(MIN_OVERLAY_DURATION_SECONDS..=MAX_OVERLAY_DURATION_SECONDS).contains(&break_seconds) {
            return Err(format!(
                "break duration must be between {MIN_OVERLAY_DURATION_SECONDS} and {MAX_OVERLAY_DURATION_SECONDS} seconds"
            ));
        }
        if !(MIN_GRID_OFFSET_MINUTES..=MAX_GRID_OFFSET_MINUTES).contains(&grid_offset_minutes) {
            return Err(format!(
                "grid offset must be between {MIN_GRID_OFFSET_MINUTES} and {MAX_GRID_OFFSET_MINUTES} minutes"
            ));
        }

        Ok(Self {
            work_minutes,
            break_seconds,
            sync_across_devices,
            grid_offset_minutes,
            pre_break_cue_enabled: true,
        })
    }

    fn work_interval(self) -> Duration {
        Duration::from_secs(self.work_minutes * 60)
    }

    fn break_duration(self) -> Duration {
        Duration::from_secs(self.break_seconds)
    }

    /// The grid offset when sync is on. The timer's only view of sync state.
    fn sync_offset(self) -> Option<i16> {
        self.sync_across_devices.then_some(self.grid_offset_minutes)
    }

    fn with_pre_break_cue_enabled(mut self, enabled: bool) -> Self {
        self.pre_break_cue_enabled = enabled;
        self
    }

    fn has_same_schedule(self, other: Self) -> bool {
        self.work_minutes == other.work_minutes
            && self.break_seconds == other.break_seconds
            && self.sync_across_devices == other.sync_across_devices
            && self.grid_offset_minutes == other.grid_offset_minutes
    }
}

impl Default for ReminderSettings {
    fn default() -> Self {
        Self {
            work_minutes: DEFAULT_WORK_MINUTES,
            break_seconds: DEFAULT_BREAK_SECONDS,
            sync_across_devices: false,
            grid_offset_minutes: 0,
            pre_break_cue_enabled: true,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReminderSettingsRequest {
    work_minutes: Value,
    break_seconds: Value,
    sync_across_devices: bool,
    grid_offset_minutes: Value,
    #[serde(default = "default_pre_break_cue_enabled")]
    pre_break_cue_enabled: bool,
}

impl ReminderSettingsRequest {
    fn into_settings(self) -> Result<ReminderSettings, String> {
        let work_minutes = integer_setting(
            &self.work_minutes,
            "work duration",
            MIN_WORK_MINUTES,
            MAX_WORK_MINUTES,
            "minutes",
        )?;
        let break_seconds = integer_setting(
            &self.break_seconds,
            "break duration",
            MIN_OVERLAY_DURATION_SECONDS,
            MAX_OVERLAY_DURATION_SECONDS,
            "seconds",
        )?;
        let grid_offset_minutes = signed_integer_setting(
            &self.grid_offset_minutes,
            "grid offset",
            MIN_GRID_OFFSET_MINUTES.into(),
            MAX_GRID_OFFSET_MINUTES.into(),
            "minutes",
        )? as i16;
        ReminderSettings::try_new(
            work_minutes,
            break_seconds,
            self.sync_across_devices,
            grid_offset_minutes,
        )
        .map(|settings| settings.with_pre_break_cue_enabled(self.pre_break_cue_enabled))
    }
}

fn integer_setting(
    value: &Value,
    name: &str,
    minimum: u64,
    maximum: u64,
    unit: &str,
) -> Result<u64, String> {
    let value = value
        .as_u64()
        .ok_or_else(|| format!("{name} must be a whole number"))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(format!(
            "{name} must be between {minimum} and {maximum} {unit}"
        ));
    }
    Ok(value)
}

fn signed_integer_setting(
    value: &Value,
    name: &str,
    minimum: i64,
    maximum: i64,
    unit: &str,
) -> Result<i64, String> {
    let value = value
        .as_i64()
        .ok_or_else(|| format!("{name} must be a whole number"))?;
    if !(minimum..=maximum).contains(&value) {
        return Err(format!(
            "{name} must be between {minimum} and {maximum} {unit}"
        ));
    }
    Ok(value)
}

fn default_pre_break_cue_enabled() -> bool {
    true
}

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedReminderSettings {
    version: u32,
    work_minutes: u64,
    break_seconds: u64,
    #[serde(default)]
    pause_until_unix_milliseconds: Option<u64>,
    #[serde(default)]
    sync_across_devices: bool,
    #[serde(default)]
    grid_offset_minutes: i16,
    #[serde(default)]
    pre_break_cue_enabled: Option<bool>,
}

#[derive(Debug, Clone, Copy)]
struct PersistedReminderState {
    settings: ReminderSettings,
    pause_until: Option<SystemTime>,
}

impl PersistedReminderSettings {
    fn from_state(state: PersistedReminderState) -> io::Result<Self> {
        Ok(Self {
            version: SETTINGS_SCHEMA_VERSION,
            work_minutes: state.settings.work_minutes,
            break_seconds: state.settings.break_seconds,
            pause_until_unix_milliseconds: state
                .pause_until
                .map(system_time_to_unix_milliseconds)
                .transpose()?,
            sync_across_devices: state.settings.sync_across_devices,
            grid_offset_minutes: state.settings.grid_offset_minutes,
            pre_break_cue_enabled: Some(state.settings.pre_break_cue_enabled),
        })
    }

    fn into_state(self, now: SystemTime) -> Result<(PersistedReminderState, bool), String> {
        let mut needs_repair = match self.version {
            v if v == SETTINGS_SCHEMA_VERSION => false,
            v if (1..SETTINGS_SCHEMA_VERSION).contains(&v) => true,
            v => return Err(format!("unsupported reminder settings version {v}")),
        };
        let pre_break_cue_enabled = match self.pre_break_cue_enabled {
            Some(enabled) => enabled,
            None => {
                needs_repair = true;
                default_pre_break_cue_enabled()
            }
        };
        let settings = ReminderSettings::try_new(
            self.work_minutes,
            self.break_seconds,
            self.sync_across_devices,
            self.grid_offset_minutes,
        )?
        .with_pre_break_cue_enabled(pre_break_cue_enabled);
        let pause_until = self.pause_until_unix_milliseconds.and_then(|milliseconds| {
            let Some(pause_until) = UNIX_EPOCH.checked_add(Duration::from_millis(milliseconds))
            else {
                needs_repair = true;
                return None;
            };
            let Ok(remaining) = pause_until.duration_since(now) else {
                needs_repair = true;
                return None;
            };
            if remaining.is_zero() || remaining > PAUSE_DURATION {
                needs_repair = true;
                return None;
            }
            Some(pause_until)
        });

        Ok((
            PersistedReminderState {
                settings,
                pause_until,
            },
            needs_repair,
        ))
    }
}

fn system_time_to_unix_milliseconds(value: SystemTime) -> io::Result<u64> {
    value
        .duration_since(UNIX_EPOCH)
        .map_err(|_| io::Error::other("pause expiry predates the Unix epoch"))?
        .as_millis()
        .try_into()
        .map_err(|_| io::Error::other("pause expiry does not fit in persisted state"))
}

#[derive(Debug, Clone, Copy)]
struct ReminderSettingsSnapshot {
    settings: ReminderSettings,
    revision: u64,
    changed_at: Instant,
    pause_until: Option<SystemTime>,
}

#[derive(Debug)]
struct ReminderSettingsRuntime {
    settings: ReminderSettings,
    revision: u64,
    changed_at: Instant,
    pause_until: Option<SystemTime>,
}

#[derive(Debug)]
enum ReminderSettingsState {
    Available(ReminderSettingsRuntime),
    Unavailable(LoadFailure),
}

#[derive(Debug)]
struct ReminderSettingsInner {
    path: PathBuf,
    state: Mutex<ReminderSettingsState>,
    recovery: Mutex<()>,
}

#[derive(Debug, Clone)]
pub(crate) struct ReminderSettingsManager {
    inner: Arc<ReminderSettingsInner>,
}

impl ReminderSettingsManager {
    /// Always constructs a manager anchored to the canonical app-config path.
    /// Storage failures are retained as typed unavailable state so setup can
    /// continue with one inert scheduler and an installed tray.
    pub(crate) fn initialize(config_dir: &Path) -> Self {
        let path = config_dir.join(SETTINGS_FILE_NAME);
        let state = match load_settings_runtime(&path, SystemTime::now(), Instant::now(), 0) {
            Ok(runtime) => ReminderSettingsState::Available(runtime),
            Err(failure) => ReminderSettingsState::Unavailable(failure),
        };
        Self {
            inner: Arc::new(ReminderSettingsInner {
                path,
                state: Mutex::new(state),
                recovery: Mutex::new(()),
            }),
        }
    }

    #[cfg(test)]
    fn load(config_dir: &Path) -> io::Result<Self> {
        let manager = Self::initialize(config_dir);
        if manager.authoritative_snapshot().is_some() {
            Ok(manager)
        } else {
            Err(io::Error::other("reminder settings unavailable"))
        }
    }

    fn view(&self) -> LocalSnapshot<ReminderSettings> {
        let Ok(state) = self.inner.state.lock() else {
            return LocalSnapshot::unavailable(StorageFailureCategory::Read);
        };
        match &*state {
            ReminderSettingsState::Available(runtime) => LocalSnapshot::available(runtime.settings),
            ReminderSettingsState::Unavailable(failure) => {
                LocalSnapshot::unavailable(failure.category)
            }
        }
    }

    fn authoritative_snapshot(&self) -> Option<ReminderSettingsSnapshot> {
        let state = self.inner.state.lock().ok()?;
        let ReminderSettingsState::Available(runtime) = &*state else {
            return None;
        };
        Some(ReminderSettingsSnapshot {
            settings: runtime.settings,
            revision: runtime.revision,
            changed_at: runtime.changed_at,
            pause_until: runtime.pause_until,
        })
    }

    #[cfg(test)]
    fn current(&self) -> ReminderSettings {
        self.authoritative_snapshot()
            .expect("test settings should be available")
            .settings
    }

    #[cfg(test)]
    fn snapshot(&self) -> ReminderSettingsSnapshot {
        self.authoritative_snapshot()
            .expect("test settings should be available")
    }

    pub(crate) fn diagnostics(&self) -> StorageDiagnostic {
        let Ok(state) = self.inner.state.lock() else {
            return LoadFailure::read("reminder settings state lock is poisoned").diagnostic();
        };
        match &*state {
            ReminderSettingsState::Available(_) => StorageDiagnostic::available(),
            ReminderSettingsState::Unavailable(failure) => failure.diagnostic(),
        }
    }

    fn save(&self, settings: ReminderSettings) -> Result<ReminderSettings, String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "reminder settings are unavailable".to_owned())?;
        let ReminderSettingsState::Available(runtime) = &mut *state else {
            return Err("reminder settings are unavailable; recover them before saving".into());
        };
        persist_settings(
            &self.inner.path,
            PersistedReminderState {
                settings,
                pause_until: runtime.pause_until,
            },
        )
        .map_err(|error| format!("could not save reminder settings: {error}"))?;
        runtime.settings = settings;
        runtime.revision = runtime.revision.wrapping_add(1);
        runtime.changed_at = Instant::now();
        Ok(settings)
    }

    fn reset(&self) -> Result<ReminderSettings, String> {
        let _recovery = self
            .inner
            .recovery
            .lock()
            .map_err(|_| "reminder settings recovery is unavailable".to_owned())?;
        let category = {
            let state = self
                .inner
                .state
                .lock()
                .map_err(|_| "reminder settings recovery is unavailable".to_owned())?;
            match &*state {
                ReminderSettingsState::Available(_) => None,
                ReminderSettingsState::Unavailable(failure) => Some(failure.category),
            }
        };
        match category {
            None => self.save(ReminderSettings::default()),
            Some(StorageFailureCategory::Invalid) => self.reset_invalid_locked(),
            Some(StorageFailureCategory::Read) => Err(
                "saved reminder settings cannot be preserved right now; retry is still available"
                    .into(),
            ),
        }
    }

    fn reset_invalid_locked(&self) -> Result<ReminderSettings, String> {
        let contents = fs::read(&self.inner.path).map_err(|error| {
            self.publish_failure(LoadFailure::read(format!(
                "could not re-read {} before recovery: {error}",
                self.inner.path.display()
            )));
            "saved reminder settings could not be read; retry is still available".to_owned()
        })?;
        let now = SystemTime::now();
        if settings_state_from_bytes(&contents, now).is_ok() {
            return self
                .retry_load_locked()
                .data
                .ok_or_else(|| "reminder settings recovery did not become available".to_owned());
        }

        quarantine_invalid_hard_link(&self.inner.path, &contents).map_err(|error| {
            self.publish_failure(LoadFailure::invalid(format!(
                "could not preserve invalid {}: {error}",
                self.inner.path.display()
            )));
            "unreadable reminder settings could not be preserved".to_owned()
        })?;
        let defaults = PersistedReminderState {
            settings: ReminderSettings::default(),
            pause_until: None,
        };
        let temp_path = prepare_settings_file(&self.inner.path, defaults).map_err(|error| {
            self.publish_failure(LoadFailure::invalid(format!(
                "invalid {} was preserved, but defaults could not be prepared: {error}",
                self.inner.path.display()
            )));
            "default reminder settings could not be prepared".to_owned()
        })?;
        let unchanged = match canonical_bytes_unchanged(&self.inner.path, &contents) {
            Ok(unchanged) => unchanged,
            Err(error) => {
                let _ = fs::remove_file(&temp_path);
                self.publish_failure(LoadFailure::read(format!(
                    "could not complete the final canonical recheck for {}: {error}",
                    self.inner.path.display()
                )));
                return Err(
                    "saved reminder settings could not be rechecked; retry is still available"
                        .into(),
                );
            }
        };
        if !unchanged {
            let _ = fs::remove_file(&temp_path);
            return Err("reminder settings changed during recovery; retry to load them".into());
        }
        let mut state = self.inner.state.lock().map_err(|_| {
            let _ = fs::remove_file(&temp_path);
            "reminder settings recovery is unavailable".to_owned()
        })?;
        if let Err(error) = replace_file_atomically(&temp_path, &self.inner.path) {
            let _ = fs::remove_file(&temp_path);
            return Err(format!(
                "default reminder settings could not be restored: {error}"
            ));
        }
        *state =
            ReminderSettingsState::Available(runtime_from_persisted(defaults, Instant::now(), 0));
        Ok(ReminderSettings::default())
    }

    fn retry_load(&self) -> StorageLoadHealth {
        let Ok(_recovery) = self.inner.recovery.lock() else {
            return StorageLoadHealth::unavailable(StorageFailureCategory::Read);
        };
        if self.authoritative_snapshot().is_some() {
            return StorageLoadHealth::available();
        }
        self.retry_load_locked().load_health
    }

    fn retry_load_locked(&self) -> LocalSnapshot<ReminderSettings> {
        match load_settings_runtime(&self.inner.path, SystemTime::now(), Instant::now(), 0) {
            Ok(runtime) => {
                let settings = runtime.settings;
                if let Ok(mut state) = self.inner.state.lock() {
                    *state = ReminderSettingsState::Available(runtime);
                    LocalSnapshot::available(settings)
                } else {
                    LocalSnapshot::unavailable(StorageFailureCategory::Read)
                }
            }
            Err(failure) => {
                let category = failure.category;
                self.publish_failure(failure);
                LocalSnapshot::unavailable(category)
            }
        }
    }

    fn publish_failure(&self, failure: LoadFailure) {
        if let Ok(mut state) = self.inner.state.lock() {
            *state = ReminderSettingsState::Unavailable(failure);
        }
    }

    fn pause_for(&self, duration: Duration) -> Result<(), String> {
        let pause_until = SystemTime::now()
            .checked_add(duration)
            .ok_or_else(|| "pause expiry overflowed the system clock".to_owned())?;
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "reminder settings are unavailable".to_owned())?;
        let ReminderSettingsState::Available(runtime) = &mut *state else {
            return Err("reminder settings are unavailable".into());
        };
        persist_settings(
            &self.inner.path,
            PersistedReminderState {
                settings: runtime.settings,
                pause_until: Some(pause_until),
            },
        )
        .map_err(|error| format!("could not save the reminder pause: {error}"))?;
        runtime.pause_until = Some(pause_until);
        Ok(())
    }

    fn clear_pause(&self) -> Result<(), String> {
        let mut state = self
            .inner
            .state
            .lock()
            .map_err(|_| "reminder settings are unavailable".to_owned())?;
        let ReminderSettingsState::Available(runtime) = &mut *state else {
            return Err("reminder settings are unavailable".into());
        };
        if runtime.pause_until.is_none() {
            return Ok(());
        }
        persist_settings(
            &self.inner.path,
            PersistedReminderState {
                settings: runtime.settings,
                pause_until: None,
            },
        )
        .map_err(|error| format!("could not clear the reminder pause: {error}"))?;
        runtime.pause_until = None;
        Ok(())
    }
}

fn runtime_from_persisted(
    persisted: PersistedReminderState,
    changed_at: Instant,
    revision: u64,
) -> ReminderSettingsRuntime {
    ReminderSettingsRuntime {
        settings: persisted.settings,
        revision,
        changed_at,
        pause_until: persisted.pause_until,
    }
}

fn settings_state_from_bytes(
    contents: &[u8],
    now: SystemTime,
) -> Result<(PersistedReminderState, bool), LoadFailure> {
    let persisted =
        serde_json::from_slice::<PersistedReminderSettings>(contents).map_err(|error| {
            LoadFailure::invalid(format!("reminder settings content is malformed: {error}"))
        })?;
    persisted.into_state(now).map_err(|error| {
        LoadFailure::invalid(format!(
            "reminder settings content is unsupported or invalid: {error}"
        ))
    })
}

fn load_settings_runtime(
    path: &Path,
    now: SystemTime,
    changed_at: Instant,
    revision: u64,
) -> Result<ReminderSettingsRuntime, LoadFailure> {
    let state = match fs::read(path) {
        Ok(contents) => {
            let (state, needs_repair) = settings_state_from_bytes(&contents, now)?;
            if needs_repair {
                persist_settings(path, state).map_err(|error| {
                    LoadFailure::read(format!(
                        "could not migrate or repair {}: {error}",
                        path.display()
                    ))
                })?;
            }
            state
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let defaults = PersistedReminderState {
                settings: ReminderSettings::default(),
                pause_until: None,
            };
            persist_settings(path, defaults).map_err(|error| {
                LoadFailure::read(format!(
                    "could not persist default reminder settings at {}: {error}",
                    path.display()
                ))
            })?;
            defaults
        }
        Err(error) => {
            return Err(LoadFailure::read(format!(
                "could not read {}: {error}",
                path.display()
            )));
        }
    };
    Ok(runtime_from_persisted(state, changed_at, revision))
}

fn create_settings_temp_file(path: &Path) -> io::Result<(PathBuf, File)> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "settings path has no parent")
    })?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "settings".into());
    let permissions = existing_file_permissions(path)?;

    for _ in 0..100 {
        let id = SETTINGS_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
        let temp_path = parent.join(format!(".{name}.{}.{id}.tmp", std::process::id()));
        match create_new_file_with_permissions(&temp_path, permissions.as_ref()) {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a reminder settings temporary file",
    ))
}

fn prepare_settings_file(path: &Path, state: PersistedReminderState) -> io::Result<PathBuf> {
    #[cfg(test)]
    if TEST_SETTINGS_PERSIST_FAILURES
        .lock()
        .is_ok_and(|mut targets| {
            targets
                .iter()
                .position(|target| target == path)
                .map(|index| targets.remove(index))
                .is_some()
        })
    {
        return Err(io::Error::other("injected reminder settings write failure"));
    }

    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "settings path has no parent")
    })?;
    fs::create_dir_all(parent)?;
    let serialized = serde_json::to_vec_pretty(&PersistedReminderSettings::from_state(state)?)
        .map_err(io::Error::other)?;
    let (temp_path, mut temp_file) = create_settings_temp_file(path)?;
    let result = temp_file
        .write_all(&serialized)
        .and_then(|()| temp_file.write_all(b"\n"))
        .and_then(|()| temp_file.sync_all());
    drop(temp_file);
    if let Err(error) = result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(temp_path)
}

fn persist_settings(path: &Path, state: PersistedReminderState) -> io::Result<()> {
    let temp_path = prepare_settings_file(path, state)?;
    if let Err(error) = replace_file_atomically(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn get_reminder_settings(
    window: WebviewWindow,
    manager: State<'_, ReminderSettingsManager>,
) -> Result<LocalSnapshot<ReminderSettings>, String> {
    authorize_main_caller(window.label())?;
    Ok(manager.view())
}

#[tauri::command]
pub(crate) async fn retry_reminder_settings(
    window: WebviewWindow,
    manager: State<'_, ReminderSettingsManager>,
    control: State<'_, ReminderControl>,
) -> Result<StorageLoadHealth, String> {
    authorize_main_caller(window.label())?;
    let manager = manager.inner().clone();
    let control = control.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        retry_reminder_settings_and_notify(&manager, &control)
    })
    .await
    .map_err(|error| format!("reminder settings retry task failed: {error}"))
}

#[tauri::command]
pub(crate) fn save_reminder_settings(
    window: WebviewWindow,
    manager: State<'_, ReminderSettingsManager>,
    control: State<'_, ReminderControl>,
    settings: ReminderSettingsRequest,
) -> Result<ReminderSettings, String> {
    authorize_main_caller(window.label())?;
    save_reminder_settings_and_notify(manager.inner(), control.inner(), settings.into_settings()?)
}

#[tauri::command]
pub(crate) async fn reset_reminder_settings(
    window: WebviewWindow,
    manager: State<'_, ReminderSettingsManager>,
    control: State<'_, ReminderControl>,
) -> Result<ReminderSettings, String> {
    authorize_main_caller(window.label())?;
    let manager = manager.inner().clone();
    let control = control.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        reset_reminder_settings_and_notify(&manager, &control)
    })
    .await
    .map_err(|error| format!("reminder settings reset task failed: {error}"))?
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ReminderAction {
    Pause,
    Resume,
    TakeBreakNow,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ReminderPauseAction {
    Pause,
    Resume,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReminderStatus {
    pub(crate) phase: TrayPhase,
    pub(crate) status: String,
    pub(crate) remaining_milliseconds: Option<u64>,
    pub(crate) pause_expires_in_milliseconds: Option<u64>,
    pub(crate) overlay_active: bool,
    pub(crate) settings_revision: u64,
    pub(crate) state_revision: u64,
    pub(crate) action_error: Option<String>,
    pub(crate) pause_action: ReminderPauseAction,
    pub(crate) pause_action_label: String,
    pub(crate) pause_action_enabled: bool,
    pub(crate) take_break_enabled: bool,
    pub(crate) preview_enabled: bool,
}

impl ReminderStatus {
    pub(crate) fn from_snapshot(snapshot: TraySnapshot) -> Self {
        let (pause_action, pause_action_label, pause_action_enabled) = match snapshot.phase {
            TrayPhase::Paused => (ReminderPauseAction::Resume, "Resume reminders".into(), true),
            TrayPhase::Working => (
                ReminderPauseAction::Pause,
                format!("Pause for {PAUSE_DURATION_MINUTES} minutes"),
                true,
            ),
            TrayPhase::Break | TrayPhase::Stopped | TrayPhase::Unavailable => (
                ReminderPauseAction::Pause,
                format!("Pause for {PAUSE_DURATION_MINUTES} minutes"),
                false,
            ),
        };
        let status = snapshot.presentation().status;
        Self {
            phase: snapshot.phase,
            status,
            remaining_milliseconds: snapshot.remaining_milliseconds,
            pause_expires_in_milliseconds: snapshot.pause_expires_in_milliseconds,
            overlay_active: snapshot.overlay_active,
            settings_revision: snapshot.settings_revision,
            state_revision: snapshot.state_revision,
            action_error: snapshot.action_error,
            pause_action,
            pause_action_label,
            pause_action_enabled,
            take_break_enabled: snapshot.phase == TrayPhase::Working && !snapshot.overlay_active,
            preview_enabled: snapshot.phase != TrayPhase::Unavailable && !snapshot.overlay_active,
        }
    }

    pub(crate) fn tray_status(&self) -> &str {
        if self.action_error.is_some() {
            "Action failed · open Unfocus"
        } else {
            &self.status
        }
    }
}

type ReminderActionResponse = Result<TraySnapshot, String>;

#[derive(Debug, Clone, Copy)]
enum ReminderControlCommand {
    Action(ReminderAction),
    SynchronizeSettings,
}

#[derive(Debug)]
struct ReminderControlRequest {
    attempt_id: u64,
    command: ReminderControlCommand,
    response: Option<SyncSender<ReminderActionResponse>>,
}

impl ReminderControlRequest {
    fn cancels_pre_break_cue(&self) -> bool {
        matches!(self.command, ReminderControlCommand::Action(_))
    }
}

#[derive(Debug, Clone)]
pub(crate) struct ReminderControl {
    sender: SyncSender<ReminderControlRequest>,
    action_health: ReminderActionHealth,
    tray_status: TrayStatus,
    next_attempt_id: Arc<AtomicU64>,
}

impl ReminderControl {
    fn channel(tray_status: TrayStatus) -> (Self, Receiver<ReminderControlRequest>) {
        let (sender, receiver) = mpsc::sync_channel(REMINDER_CONTROL_CAPACITY);
        (
            Self {
                sender,
                action_health: ReminderActionHealth::default(),
                tray_status,
                next_attempt_id: Arc::new(AtomicU64::new(0)),
            },
            receiver,
        )
    }

    pub(crate) fn dispatch(&self, action: ReminderAction) -> Result<(), String> {
        self.ensure_action_available(action)?;
        self.send(ReminderControlRequest {
            attempt_id: self.next_attempt_id(),
            command: ReminderControlCommand::Action(action),
            response: None,
        })
    }

    fn request(&self, action: ReminderAction) -> Result<ReminderStatus, String> {
        self.ensure_action_available(action)?;
        let (response, receiver) = mpsc::sync_channel(1);
        let attempt_id = self.next_attempt_id();
        self.send(ReminderControlRequest {
            attempt_id,
            command: ReminderControlCommand::Action(action),
            response: Some(response),
        })?;
        let response = receiver
            .recv_timeout(REMINDER_CONTROL_TIMEOUT)
            .map_err(|error| {
                let message = format!("reminder scheduler did not answer the action: {error}");
                self.report_failure(attempt_id, message.clone());
                message
            })?;
        let snapshot = response?;
        Ok(ReminderStatus::from_snapshot(snapshot))
    }

    /// Wake the scheduler after a committed settings mutation without waiting
    /// for an acknowledgement. The production loop also reconciles on every
    /// periodic iteration, so a full or disconnected queue cannot invalidate
    /// an already authoritative storage commit.
    fn notify_settings_changed(&self) -> Result<(), String> {
        self.sender
            .try_send(ReminderControlRequest {
                attempt_id: 0,
                command: ReminderControlCommand::SynchronizeSettings,
                response: None,
            })
            .map_err(|error| match error {
                TrySendError::Full(_) => "reminder synchronization queue is full".to_owned(),
                TrySendError::Disconnected(_) => {
                    "reminder synchronization channel is disconnected".to_owned()
                }
            })
    }

    fn send(&self, request: ReminderControlRequest) -> Result<(), String> {
        let attempt_id = request.attempt_id;
        self.sender.try_send(request).map_err(|error| {
            let message = match error {
                TrySendError::Full(_) => "reminder control queue is full".to_owned(),
                TrySendError::Disconnected(_) => "reminder scheduler has stopped".to_owned(),
            };
            self.report_failure(attempt_id, message.clone());
            message
        })
    }

    fn ensure_action_available(&self, action: ReminderAction) -> Result<(), String> {
        let snapshot = self.tray_status.current();
        let available = match action {
            ReminderAction::Pause => snapshot.phase == TrayPhase::Working,
            ReminderAction::Resume => snapshot.phase == TrayPhase::Paused,
            ReminderAction::TakeBreakNow => {
                snapshot.phase == TrayPhase::Working && !snapshot.overlay_active
            }
        };
        if available {
            Ok(())
        } else if snapshot.phase == TrayPhase::Unavailable {
            Err("automatic reminders are unavailable until saved timing is recovered".into())
        } else {
            Err("that reminder action is not available in the current state".into())
        }
    }

    fn next_attempt_id(&self) -> u64 {
        self.next_attempt_id
            .fetch_add(1, Ordering::Relaxed)
            .wrapping_add(1)
    }

    fn report_failure(&self, attempt_id: u64, error: String) {
        self.action_health.record_failure(attempt_id, error);
        self.tray_status
            .publish_action_error(self.action_health.current());
    }
}

fn notify_scheduler_after_settings_commit(control: &ReminderControl) {
    if let Err(error) = control.notify_settings_changed() {
        // Fixed technical channel state only: canonical paths and persisted
        // values stay in developer diagnostics, never in consumer feedback.
        eprintln!(
            "could not notify the reminder scheduler after a committed settings update: {error}; periodic reconciliation remains active"
        );
    }
}

fn save_reminder_settings_and_notify(
    manager: &ReminderSettingsManager,
    control: &ReminderControl,
    settings: ReminderSettings,
) -> Result<ReminderSettings, String> {
    let committed = manager.save(settings)?;
    notify_scheduler_after_settings_commit(control);
    Ok(committed)
}

fn retry_reminder_settings_and_notify(
    manager: &ReminderSettingsManager,
    control: &ReminderControl,
) -> StorageLoadHealth {
    let health = manager.retry_load();
    if health.status == crate::storage_recovery::StorageLoadStatus::Available {
        notify_scheduler_after_settings_commit(control);
    }
    health
}

fn reset_reminder_settings_and_notify(
    manager: &ReminderSettingsManager,
    control: &ReminderControl,
) -> Result<ReminderSettings, String> {
    let committed = manager.reset()?;
    notify_scheduler_after_settings_commit(control);
    Ok(committed)
}

#[derive(Debug, Default)]
struct ReminderActionHealthState {
    attempt_id: u64,
    error: Option<String>,
}

#[derive(Debug, Clone, Default)]
struct ReminderActionHealth {
    inner: Arc<Mutex<ReminderActionHealthState>>,
}

impl ReminderActionHealth {
    fn current(&self) -> Option<String> {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .error
            .clone()
    }

    fn record_failure(&self, attempt_id: u64, error: String) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if attempt_id >= state.attempt_id {
            state.attempt_id = attempt_id;
            state.error = Some(error);
        }
    }

    fn clear(&self, attempt_id: u64) {
        let mut state = self
            .inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        if attempt_id >= state.attempt_id {
            state.attempt_id = attempt_id;
            state.error = None;
        }
    }
}

#[tauri::command]
pub(crate) fn get_reminder_status(
    window: WebviewWindow,
    status: State<'_, TrayStatus>,
) -> Result<ReminderStatus, String> {
    authorize_main_caller(window.label())?;
    Ok(ReminderStatus::from_snapshot(status.current()))
}

async fn request_reminder_action(
    window: WebviewWindow,
    control: State<'_, ReminderControl>,
    action: ReminderAction,
) -> Result<ReminderStatus, String> {
    authorize_main_caller(window.label())?;
    let control = control.inner().clone();
    tauri::async_runtime::spawn_blocking(move || control.request(action))
        .await
        .map_err(|error| format!("reminder action task failed: {error}"))?
}

#[tauri::command]
pub(crate) async fn pause_reminders(
    window: WebviewWindow,
    control: State<'_, ReminderControl>,
) -> Result<ReminderStatus, String> {
    request_reminder_action(window, control, ReminderAction::Pause).await
}

#[tauri::command]
pub(crate) async fn resume_reminders(
    window: WebviewWindow,
    control: State<'_, ReminderControl>,
) -> Result<ReminderStatus, String> {
    request_reminder_action(window, control, ReminderAction::Resume).await
}

#[tauri::command]
pub(crate) async fn take_break_now(
    window: WebviewWindow,
    control: State<'_, ReminderControl>,
) -> Result<ReminderStatus, String> {
    request_reminder_action(window, control, ReminderAction::TakeBreakNow).await
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReminderPhase {
    Working,
    Break,
    Paused,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReminderTransition {
    StartBreak,
    EndBreak,
    ResumeWorking,
}

fn unix_seconds(at: SystemTime) -> i64 {
    match at.duration_since(UNIX_EPOCH) {
        Ok(since) => i64::try_from(since.as_secs()).unwrap_or(i64::MAX),
        Err(before) => -i64::try_from(before.duration().as_secs()).unwrap_or(i64::MAX),
    }
}

fn system_time_from_unix_seconds(secs: i64) -> SystemTime {
    if secs >= 0 {
        UNIX_EPOCH + Duration::from_secs(secs.unsigned_abs())
    } else {
        UNIX_EPOCH - Duration::from_secs(secs.unsigned_abs())
    }
}

/// When the current Working phase ends.
///
/// `Relative` is the default path and reproduces the pre-sync behaviour
/// exactly. `Wall` is computed once on Working entry and stored, so the grace
/// rule never re-runs in steady state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum WorkDeadline {
    Relative,
    Wall(SystemTime),
}

/// The grid deadline for a Working phase starting under `settings`, observed
/// at `wall_now`. Pure and clock-injected: the caller supplies the wall clock.
fn compute_work_deadline(settings: ReminderSettings, wall_now: SystemTime) -> WorkDeadline {
    match settings.sync_offset() {
        None => WorkDeadline::Relative,
        Some(offset) => {
            let interval = settings.work_interval().as_secs() as i64;
            WorkDeadline::Wall(system_time_from_unix_seconds(
                schedule::deadline_with_grace(unix_seconds(wall_now), interval, offset),
            ))
        }
    }
}

/// The recurring reminder clock. Break and Pause advance on injected monotonic
/// time only. Working advances on monotonic time in relative mode (the
/// default) and on a stored wall-clock grid deadline in sync mode. Probe
/// results deliberately do not participate in phase advancement in either mode.
#[derive(Debug)]
struct ReminderTimer {
    phase: ReminderPhase,
    phase_started_at: Duration,
    paused_until: Option<Duration>,
    settings: ReminderSettings,
    pending_settings: Option<ReminderSettings>,
    state_revision: u64,
    work_deadline: WorkDeadline,
}

impl ReminderTimer {
    fn new(now: Duration, settings: ReminderSettings, wall_now: SystemTime) -> Self {
        Self {
            phase: ReminderPhase::Working,
            phase_started_at: now,
            paused_until: None,
            work_deadline: compute_work_deadline(settings, wall_now),
            settings,
            pending_settings: None,
            state_revision: 0,
        }
    }

    fn new_paused(
        now: Duration,
        settings: ReminderSettings,
        remaining: Duration,
        wall_now: SystemTime,
    ) -> Self {
        Self {
            phase: ReminderPhase::Paused,
            phase_started_at: now,
            paused_until: Some(now.saturating_add(remaining.min(PAUSE_DURATION))),
            work_deadline: compute_work_deadline(settings, wall_now),
            settings,
            pending_settings: None,
            state_revision: 0,
        }
    }

    #[cfg(test)]
    fn with_defaults(now: Duration) -> Self {
        Self::new(now, ReminderSettings::default(), UNIX_EPOCH)
    }

    /// The single entry point for *transitions* onto the Working phase.
    /// Computes and stores the deadline for this cycle (relative or
    /// wall-clock grid) and bumps `state_revision` once; callers that
    /// delegate here must not bump again. `new` and `new_paused` bypass this
    /// and compute the deadline directly so construction keeps
    /// `state_revision` at 0.
    fn enter_working(&mut self, now: Duration, wall_now: SystemTime) {
        self.phase = ReminderPhase::Working;
        self.phase_started_at = now;
        self.paused_until = None;
        self.work_deadline = compute_work_deadline(self.settings, wall_now);
        self.state_revision = self.state_revision.wrapping_add(1);
    }

    /// Re-derive the grid deadline after a clock discontinuity (a step, or a
    /// suspend on platforms where `Instant` observes it — see
    /// `lifecycle_contract::DiscontinuityObservation`).
    ///
    /// Deliberately narrower than `enter_working`: a clock jump is not a phase
    /// entry, so `phase`, `phase_started_at`, and `paused_until` are untouched.
    /// Only Working in sync mode has a wall-clock deadline to rebase; a
    /// relative-mode Working phase, and Break and Paused, are left completely
    /// alone so suspend-and-wake resumes rather than restarting them.
    fn rebase_work_deadline(&mut self, wall_now: SystemTime) {
        if self.phase != ReminderPhase::Working {
            return;
        }
        if let WorkDeadline::Wall(_) = self.work_deadline {
            self.work_deadline = compute_work_deadline(self.settings, wall_now);
            self.state_revision = self.state_revision.wrapping_add(1);
        }
    }

    fn apply_settings(
        &mut self,
        settings: ReminderSettings,
        changed_at: Duration,
        wall_now: SystemTime,
    ) {
        match self.phase {
            ReminderPhase::Working => {
                let schedule_changed = !self.settings.has_same_schedule(settings);
                self.settings = settings;
                self.pending_settings = None;
                if schedule_changed {
                    self.enter_working(changed_at, wall_now);
                }
            }
            ReminderPhase::Break => {
                self.pending_settings = Some(settings);
                self.state_revision = self.state_revision.wrapping_add(1);
            }
            ReminderPhase::Paused => {
                self.settings = settings;
                self.pending_settings = None;
                self.state_revision = self.state_revision.wrapping_add(1);
            }
        }
    }

    fn break_duration(&self) -> Duration {
        self.settings.break_duration()
    }

    fn pause(&mut self, now: Duration) -> bool {
        if self.phase != ReminderPhase::Working {
            return false;
        }
        self.phase = ReminderPhase::Paused;
        self.phase_started_at = now;
        self.paused_until = Some(now.saturating_add(PAUSE_DURATION));
        self.state_revision = self.state_revision.wrapping_add(1);
        true
    }

    fn resume(&mut self, now: Duration, wall_now: SystemTime) -> bool {
        if self.phase != ReminderPhase::Paused {
            return false;
        }
        self.enter_working(now, wall_now);
        true
    }

    fn take_break_now(&mut self, now: Duration) -> bool {
        if self.phase != ReminderPhase::Working {
            return false;
        }
        self.phase = ReminderPhase::Break;
        self.phase_started_at = now;
        self.paused_until = None;
        self.state_revision = self.state_revision.wrapping_add(1);
        true
    }

    /// Credit a natural break after the timer has already entered `Break`.
    /// Used when idle probes show the user already rested long enough that
    /// showing a multi-monitor overlay would be wrong — and sitting in a
    /// silent break phase would feel broken.
    fn credit_natural_break(&mut self, now: Duration, wall_now: SystemTime) -> bool {
        if self.phase != ReminderPhase::Break {
            return false;
        }
        // Pending settings must be applied before the deadline is computed:
        // the deadline depends on the interval and offset, both of which can
        // change with pending settings.
        if let Some(settings) = self.pending_settings.take() {
            self.settings = settings;
        }
        self.enter_working(now, wall_now);
        true
    }

    fn tick(&mut self, now: Duration, wall_now: SystemTime) -> Option<ReminderTransition> {
        if self.phase == ReminderPhase::Paused {
            let paused_until = self.paused_until.unwrap_or(self.phase_started_at);
            if now < paused_until {
                return None;
            }
            self.enter_working(now, wall_now);
            return Some(ReminderTransition::ResumeWorking);
        }

        let due = match self.phase {
            ReminderPhase::Working => match self.work_deadline {
                WorkDeadline::Relative => {
                    now.saturating_sub(self.phase_started_at) >= self.settings.work_interval()
                }
                WorkDeadline::Wall(due) => wall_now >= due,
            },
            ReminderPhase::Break => {
                now.saturating_sub(self.phase_started_at) >= self.settings.break_duration()
            }
            ReminderPhase::Paused => unreachable!("paused phases return before duration lookup"),
        };

        if !due {
            return None;
        }

        // Anchor the next phase at this observation rather than replaying every
        // missed cycle after a long scheduler stall.
        Some(match self.phase {
            ReminderPhase::Working => {
                self.phase = ReminderPhase::Break;
                self.phase_started_at = now;
                self.state_revision = self.state_revision.wrapping_add(1);
                ReminderTransition::StartBreak
            }
            ReminderPhase::Break => {
                // Pending settings must be applied before the deadline is
                // computed: applying afterwards would compute against the
                // stale interval and silently desync for one cycle.
                if let Some(settings) = self.pending_settings.take() {
                    self.settings = settings;
                }
                self.enter_working(now, wall_now);
                ReminderTransition::EndBreak
            }
            ReminderPhase::Paused => unreachable!("paused phases return before transition"),
        })
    }

    fn tray_snapshot(
        &self,
        now: Duration,
        wall_now: SystemTime,
        settings_revision: u64,
        overlay_active: bool,
    ) -> TraySnapshot {
        match self.phase {
            ReminderPhase::Working | ReminderPhase::Break => {
                let (phase, remaining) = match self.phase {
                    ReminderPhase::Working => (
                        TrayPhase::Working,
                        match self.work_deadline {
                            WorkDeadline::Relative => self
                                .settings
                                .work_interval()
                                .saturating_sub(now.saturating_sub(self.phase_started_at)),
                            WorkDeadline::Wall(due) => {
                                due.duration_since(wall_now).unwrap_or(Duration::ZERO)
                            }
                        },
                    ),
                    ReminderPhase::Break => (
                        TrayPhase::Break,
                        self.settings
                            .break_duration()
                            .saturating_sub(now.saturating_sub(self.phase_started_at)),
                    ),
                    ReminderPhase::Paused => unreachable!("paused phases use the paused snapshot"),
                };
                TraySnapshot::timer(
                    phase,
                    remaining,
                    overlay_active,
                    settings_revision,
                    self.state_revision,
                )
            }
            ReminderPhase::Paused => TraySnapshot::paused(
                self.paused_until
                    .unwrap_or(self.phase_started_at)
                    .saturating_sub(now),
                overlay_active,
                settings_revision,
                self.state_revision,
            ),
        }
    }
}

/// Why a due break was or was not shown. Probe data and optional activity
/// context only affect presentation (and natural-break credit for idle); they
/// never advance the pure clock.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BreakPresentation {
    Show,
    /// Idle long enough that the user already rested; credit the break.
    NaturalIdle,
    /// Fullscreen/presentation active; keep the break phase without overlay.
    SuppressFullscreen,
}

/// Probe data can suppress presentation of a due break, but it cannot mutate
/// the timer by itself. Errors fail open so an unavailable probe never
/// disables breaks. Idle long enough for a natural break is reported separately
/// so the scheduler can credit work without a silent break phase.
///
/// When `activity` history is available, issue #61 adapts presentation only:
/// long continuous active requires a real AFK for natural credit; long AFK
/// still prefers natural credit when idle ≥ break length. Fullscreen always
/// wins. Empty history falls back to the legacy idle/fullscreen rules.
fn break_presentation(
    probes: &ProbeSnapshot,
    break_duration: Duration,
    activity: Option<ActivityPresentationContext>,
) -> BreakPresentation {
    // Fullscreen is never overridden by adaptation (#61 decision 5).
    if matches!(
        &probes.active_window_fullscreen,
        ProbeReading::Available(true)
    ) {
        return BreakPresentation::SuppressFullscreen;
    }

    let ProbeReading::Available(idle_seconds) = &probes.idle_seconds else {
        // Pending and failed idle probes both fail open and show the break.
        return BreakPresentation::Show;
    };
    let idle_seconds = *idle_seconds;
    let break_secs = break_duration.as_secs();

    if let Some(ctx) = activity.filter(|ctx| ctx.history_available) {
        if ctx.continuous_active_seconds >= LONG_ACTIVE_SECONDS {
            // After deep continuous work, micro-idle equal only to a short break
            // is not enough; require a real AFK-length rest for natural credit.
            let required = break_secs.max(LONG_AFK_SECONDS);
            if idle_seconds >= required {
                return BreakPresentation::NaturalIdle;
            }
            return BreakPresentation::Show;
        }
        if ctx.recent_afk_seconds >= LONG_AFK_SECONDS {
            if idle_seconds >= break_secs {
                return BreakPresentation::NaturalIdle;
            }
            return BreakPresentation::Show;
        }
    }

    if idle_seconds >= break_secs {
        return BreakPresentation::NaturalIdle;
    }
    BreakPresentation::Show
}

// The wall clock is injected once per scheduler iteration and reused across
// every call in that pass (see `start_scheduler`), rather than sampled
// locally, which pushed this seam past clippy's default argument threshold.
#[allow(clippy::too_many_arguments)]
fn execute_reminder_action(
    action: ReminderAction,
    now: Duration,
    wall_now: SystemTime,
    timer: &mut ReminderTimer,
    settings_manager: &ReminderSettingsManager,
    app: &AppHandle,
    overlay_controller: &OverlayController,
    break_ledger: &BreakLedgerHandle,
) -> Result<(), String> {
    match action {
        ReminderAction::Pause if timer.phase == ReminderPhase::Working => {
            start_bounded_pause(timer, now, || settings_manager.pause_for(PAUSE_DURATION))?;
        }
        ReminderAction::Resume if timer.phase == ReminderPhase::Paused => {
            resume_from_pause(timer, now, wall_now, || settings_manager.clear_pause())?;
        }
        ReminderAction::TakeBreakNow if timer.phase == ReminderPhase::Working => {
            let settings = timer.settings;
            start_manual_break(timer, now, |break_duration| {
                show_overlay_if_idle(app, overlay_controller, break_duration.as_secs()).map(|_| ())
            })?;
            // Only record after the timer transition commits (overlay built).
            if timer.phase == ReminderPhase::Break {
                break_ledger.record(
                    BreakEventKind::ManualTakeBreak,
                    settings.work_minutes,
                    settings.break_seconds,
                    epoch_ms(SystemTime::now()),
                );
            }
        }
        ReminderAction::Pause | ReminderAction::Resume | ReminderAction::TakeBreakNow => {
            // Stale and repeated native events are explicit idempotent no-ops.
        }
    }
    Ok(())
}

fn start_bounded_pause(
    timer: &mut ReminderTimer,
    now: Duration,
    persist: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if timer.phase != ReminderPhase::Working {
        return Ok(());
    }
    persist()?;
    let paused = timer.pause(now);
    debug_assert!(paused);
    Ok(())
}

fn resume_from_pause(
    timer: &mut ReminderTimer,
    now: Duration,
    wall_now: SystemTime,
    persist: impl FnOnce() -> Result<(), String>,
) -> Result<(), String> {
    if timer.phase != ReminderPhase::Paused {
        return Ok(());
    }
    persist()?;
    let resumed = timer.resume(now, wall_now);
    debug_assert!(resumed);
    Ok(())
}

fn start_manual_break(
    timer: &mut ReminderTimer,
    now: Duration,
    present: impl FnOnce(Duration) -> Result<(), String>,
) -> Result<(), String> {
    if timer.phase != ReminderPhase::Working {
        return Ok(());
    }

    // Build the synchronized overlay before committing the timer transition.
    // A native window failure therefore leaves the authoritative work phase
    // and deadline unchanged.
    present(timer.break_duration())?;
    let break_started = timer.take_break_now(now);
    debug_assert!(break_started);
    Ok(())
}

/// Presentation decision after optional overlay creation.
///
/// `Show` is only returned when the overlay was created successfully so the
/// break ledger does not invent a "shown" outcome for a failed cover.
fn present_scheduled_break(
    app: &AppHandle,
    probe_cache: &ProbeCache,
    activity_tracker: &ActivityTrackerHandle,
    overlay_controller: &OverlayController,
    break_duration: Duration,
) -> Option<BreakPresentation> {
    let probes = probe_cache.snapshot();
    let activity = activity_tracker.presentation_context(epoch_ms(SystemTime::now()));
    let decision = break_presentation(&probes, break_duration, Some(activity));
    match decision {
        BreakPresentation::NaturalIdle => {
            eprintln!("scheduled break credited as natural rest because the user is already idle");
            return Some(decision);
        }
        BreakPresentation::SuppressFullscreen => {
            eprintln!("scheduled break stayed hidden while fullscreen is active");
            return Some(decision);
        }
        BreakPresentation::Show => {}
    }

    match show_overlay(app, overlay_controller, break_duration.as_secs()) {
        Ok(_) => Some(BreakPresentation::Show),
        Err(error) => {
            eprintln!("could not present scheduled break: {error}");
            None
        }
    }
}

struct ReminderSchedulerContext {
    app: AppHandle,
    probe_cache: ProbeCache,
    activity_tracker: ActivityTrackerHandle,
    break_ledger: BreakLedgerHandle,
    overlay_controller: OverlayController,
    settings_manager: ReminderSettingsManager,
    tray_status: TrayStatus,
    receiver: Receiver<ReminderControlRequest>,
    action_health: ReminderActionHealth,
    next_attempt_id: Arc<AtomicU64>,
}

struct ReminderScheduler {
    context: ReminderSchedulerContext,
    started_at: Instant,
    runtime: ReminderSchedulerRuntime,
    control_connected: bool,
    last_sample: Option<(SystemTime, Duration)>,
    pre_break_cue: PreBreakCue,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SettingsReconciliation {
    Unavailable,
    Activated,
    Unchanged,
    Updated,
}

/// Pure timer-owning portion of one scheduler thread. Production reconciliation
/// and ticking both pass through this state, so recovery cannot accidentally
/// create a second timer beside the scheduler's existing one.
struct ReminderSchedulerRuntime {
    timer: Option<ReminderTimer>,
    settings_revision: u64,
}

impl ReminderSchedulerRuntime {
    fn new(initial: Option<ReminderSettingsSnapshot>, now: Duration, wall_now: SystemTime) -> Self {
        Self {
            timer: initial.map(|snapshot| Self::recovered_timer(now, wall_now, snapshot)),
            settings_revision: initial.map_or(0, |snapshot| snapshot.revision),
        }
    }

    fn recovered_timer(
        now: Duration,
        wall_now: SystemTime,
        snapshot: ReminderSettingsSnapshot,
    ) -> ReminderTimer {
        let pause_remaining = snapshot
            .pause_until
            .and_then(|pause_until| pause_until.duration_since(wall_now).ok());
        pause_remaining.map_or_else(
            || ReminderTimer::new(now, snapshot.settings, wall_now),
            |remaining| ReminderTimer::new_paused(now, snapshot.settings, remaining, wall_now),
        )
    }

    fn reconcile_settings(
        &mut self,
        latest: Option<ReminderSettingsSnapshot>,
        now: Duration,
        wall_now: SystemTime,
        started_at: Instant,
    ) -> SettingsReconciliation {
        let Some(latest) = latest else {
            return SettingsReconciliation::Unavailable;
        };
        let Some(timer) = self.timer.as_mut() else {
            self.timer = Some(Self::recovered_timer(now, wall_now, latest));
            self.settings_revision = latest.revision;
            return SettingsReconciliation::Activated;
        };
        if latest.revision == self.settings_revision {
            return SettingsReconciliation::Unchanged;
        }
        timer.apply_settings(
            latest.settings,
            latest.changed_at.saturating_duration_since(started_at),
            wall_now,
        );
        self.settings_revision = latest.revision;
        SettingsReconciliation::Updated
    }

    fn tick(&mut self, now: Duration, wall_now: SystemTime) -> Option<ReminderTransition> {
        self.timer.as_mut()?.tick(now, wall_now)
    }

    fn timer(&self) -> Option<&ReminderTimer> {
        self.timer.as_ref()
    }

    fn timer_mut(&mut self) -> Option<&mut ReminderTimer> {
        self.timer.as_mut()
    }
}

/// The production scheduler's per-iteration settings reconciliation seam.
/// Keeping it free of Tauri objects lets tests exercise the exact unavailable,
/// activation, and unchanged-revision path without constructing fake windows.
fn reconcile_scheduler_iteration(
    runtime: &mut ReminderSchedulerRuntime,
    latest: Option<ReminderSettingsSnapshot>,
    now: Duration,
    wall_now: SystemTime,
    started_at: Instant,
) -> SettingsReconciliation {
    runtime.reconcile_settings(latest, now, wall_now, started_at)
}

impl ReminderScheduler {
    fn new(context: ReminderSchedulerContext) -> Self {
        let initial = context.settings_manager.authoritative_snapshot();
        let started_at = initial.map_or_else(Instant::now, |snapshot| snapshot.changed_at);
        let wall_now = SystemTime::now();
        let runtime = ReminderSchedulerRuntime::new(initial, Duration::ZERO, wall_now);
        let initial_snapshot = runtime
            .timer()
            .map_or_else(TraySnapshot::unavailable, |timer| {
                timer.tray_snapshot(Duration::ZERO, wall_now, runtime.settings_revision, false)
            });
        context.tray_status.publish(initial_snapshot);
        Self {
            context,
            started_at,
            runtime,
            control_connected: true,
            // Loop-local, not on `ReminderTimer`: the timer stays pure and
            // clock-injected. `None` on the first iteration, so it never
            // rebases before there is a prior sample to diverge from.
            last_sample: None,
            pre_break_cue: PreBreakCue::new(qualified_x11_session()),
        }
    }

    fn run(mut self) {
        loop {
            self.run_iteration();
        }
    }

    fn run_iteration(&mut self) {
        let request = self.receive_request();
        // Sampled once per iteration and reused everywhere below so every
        // timer call in this pass observes the same instant.
        let wall_now = SystemTime::now();
        let now = self.started_at.elapsed();
        self.apply_latest_settings(now, wall_now);
        let probes = self.observe_activity();
        self.rebase_after_clock_discontinuity(now, wall_now);

        if request
            .as_ref()
            .is_some_and(ReminderControlRequest::cancels_pre_break_cue)
        {
            self.pre_break_cue
                .cancel(&self.context.app, "reminder action requested");
        }
        let action_result = self.execute_request(request.as_ref(), now, wall_now);
        self.handle_transition(now, wall_now);
        let snapshot = self.snapshot_and_reconcile_cue(now, wall_now, &probes);
        self.context.tray_status.publish(snapshot.clone());
        Self::respond_to_request(request, action_result, snapshot);
    }

    fn receive_request(&mut self) -> Option<ReminderControlRequest> {
        if !self.control_connected {
            std::thread::sleep(REMINDER_POLL_INTERVAL);
            return None;
        }
        match self.context.receiver.recv_timeout(REMINDER_POLL_INTERVAL) {
            Ok(request) => Some(request),
            Err(RecvTimeoutError::Timeout) => None,
            Err(RecvTimeoutError::Disconnected) => {
                self.control_connected = false;
                None
            }
        }
    }

    fn apply_latest_settings(&mut self, now: Duration, wall_now: SystemTime) {
        reconcile_scheduler_iteration(
            &mut self.runtime,
            self.context.settings_manager.authoritative_snapshot(),
            now,
            wall_now,
            self.started_at,
        );
    }

    fn observe_activity(&self) -> ProbeSnapshot {
        // Activity segmentation is observe-only: probe failures freeze
        // classification and never change the pure reminder clock.
        let probes = self.context.probe_cache.snapshot();
        observe_activity_snapshot(
            &self.context.activity_tracker,
            epoch_ms(SystemTime::now()),
            &probes,
        );
        probes
    }

    fn rebase_after_clock_discontinuity(&mut self, now: Duration, wall_now: SystemTime) {
        // Detect an NTP/manual step or a suspend where `Instant` observes it.
        // Rebase before ticking so a jump cannot fire a stale deadline or
        // strand Working for the size of a backward step. This mirrors the
        // pinned test-only lifecycle discontinuity contract.
        if let Some((prev_wall, prev_mono)) = self.last_sample {
            let wall_delta_ms = match wall_now.duration_since(prev_wall) {
                Ok(forward) => i64::try_from(forward.as_millis()).unwrap_or(i64::MAX),
                Err(backward) => {
                    -i64::try_from(backward.duration().as_millis()).unwrap_or(i64::MAX)
                }
            };
            let mono_delta_ms =
                i64::try_from(now.saturating_sub(prev_mono).as_millis()).unwrap_or(i64::MAX);
            let tolerance_ms =
                i64::try_from(CLOCK_DIVERGENCE_TOLERANCE.as_millis()).unwrap_or(i64::MAX);
            if wall_delta_ms.abs_diff(mono_delta_ms) > tolerance_ms.unsigned_abs() {
                if let Some(timer) = self.runtime.timer_mut() {
                    timer.rebase_work_deadline(wall_now);
                }
            }
        }
        // Updated every iteration so one jump cannot trigger repeated rebases.
        self.last_sample = Some((wall_now, now));
    }

    fn execute_request(
        &mut self,
        request: Option<&ReminderControlRequest>,
        now: Duration,
        wall_now: SystemTime,
    ) -> Option<Result<(), String>> {
        let request = request?;
        let Some(timer) = self.runtime.timer_mut() else {
            return Some(Err(
                "automatic reminders are unavailable until saved timing is recovered".into(),
            ));
        };
        match request.command {
            ReminderControlCommand::Action(action) => {
                let result = execute_reminder_action(
                    action,
                    now,
                    wall_now,
                    timer,
                    &self.context.settings_manager,
                    &self.context.app,
                    &self.context.overlay_controller,
                    &self.context.break_ledger,
                );
                match &result {
                    Ok(()) => self.context.action_health.clear(request.attempt_id),
                    Err(error) => self
                        .context
                        .action_health
                        .record_failure(request.attempt_id, error.clone()),
                }
                Some(result)
            }
            ReminderControlCommand::SynchronizeSettings => Some(Ok(())),
        }
    }

    fn handle_transition(&mut self, now: Duration, wall_now: SystemTime) {
        match self.runtime.tick(now, wall_now) {
            Some(ReminderTransition::ResumeWorking) => self.clear_expired_pause(),
            Some(ReminderTransition::StartBreak) => self.start_scheduled_break(now, wall_now),
            Some(ReminderTransition::EndBreak) | None => {}
        }
    }

    fn clear_expired_pause(&self) {
        if let Err(error) = self.context.settings_manager.clear_pause() {
            let error =
                format!("reminder pause expired but its persisted state was not cleared: {error}");
            eprintln!("{error}");
            let attempt_id = self
                .context
                .next_attempt_id
                .fetch_add(1, Ordering::Relaxed)
                .wrapping_add(1);
            self.context.action_health.record_failure(attempt_id, error);
        }
    }

    fn start_scheduled_break(&mut self, now: Duration, wall_now: SystemTime) {
        let Some(timer) = self.runtime.timer_mut() else {
            return;
        };
        let settings = timer.settings;
        let Some(presentation) = present_scheduled_break(
            &self.context.app,
            &self.context.probe_cache,
            &self.context.activity_tracker,
            &self.context.overlay_controller,
            timer.break_duration(),
        ) else {
            return;
        };
        let kind = match presentation {
            BreakPresentation::Show => BreakEventKind::ScheduledShown,
            BreakPresentation::NaturalIdle => BreakEventKind::NaturalIdle,
            BreakPresentation::SuppressFullscreen => BreakEventKind::FullscreenSuppress,
        };
        self.context.break_ledger.record(
            kind,
            settings.work_minutes,
            settings.break_seconds,
            epoch_ms(SystemTime::now()),
        );
        if presentation == BreakPresentation::NaturalIdle {
            let credited = timer.credit_natural_break(now, wall_now);
            debug_assert!(credited);
        }
    }

    fn snapshot_and_reconcile_cue(
        &mut self,
        now: Duration,
        wall_now: SystemTime,
        probes: &ProbeSnapshot,
    ) -> TraySnapshot {
        let Some(timer) = self.runtime.timer() else {
            self.pre_break_cue
                .cancel(&self.context.app, "reminder settings unavailable");
            return TraySnapshot::unavailable()
                .with_action_error(self.context.action_health.current());
        };
        let snapshot = timer
            .tray_snapshot(
                now,
                wall_now,
                self.runtime.settings_revision,
                self.context.overlay_controller.has_active_run(),
            )
            .with_action_error(self.context.action_health.current());
        let cue_in_lead_window = snapshot.phase == TrayPhase::Working
            && snapshot
                .remaining_milliseconds
                .is_some_and(|remaining| (1..=CUE_LEAD_MILLISECONDS).contains(&remaining));
        let cue_presentation_allowed = !cue_in_lead_window
            || matches!(
                break_presentation(
                    probes,
                    timer.break_duration(),
                    Some(
                        self.context
                            .activity_tracker
                            .presentation_context(epoch_ms(wall_now)),
                    ),
                ),
                BreakPresentation::Show
            );
        self.pre_break_cue.reconcile(
            &self.context.app,
            &snapshot,
            timer.settings.pre_break_cue_enabled,
            cue_presentation_allowed,
            wall_now,
        );
        snapshot
    }

    fn respond_to_request(
        request: Option<ReminderControlRequest>,
        action_result: Option<Result<(), String>>,
        snapshot: TraySnapshot,
    ) {
        let Some(request) = request else {
            return;
        };
        let response = action_result
            .unwrap_or_else(|| Err("reminder action was not processed".to_owned()))
            .map(|()| snapshot);
        if let Some(sender) = request.response {
            let _ = sender.try_send(response);
        } else if let Err(error) = response {
            eprintln!("tray reminder action failed: {error}");
        }
    }
}

fn observe_activity_snapshot(
    activity_tracker: &ActivityTrackerHandle,
    now_ms: u64,
    probes: &ProbeSnapshot,
) {
    match &probes.idle_seconds {
        ProbeReading::Pending => {}
        ProbeReading::Available(idle_seconds) => {
            activity_tracker.observe(now_ms, Some(*idle_seconds));
        }
        ProbeReading::Failed(_) => activity_tracker.observe(now_ms, None),
    }
}

pub(crate) fn start_scheduler(
    app: AppHandle,
    probe_cache: ProbeCache,
    activity_tracker: ActivityTrackerHandle,
    break_ledger: BreakLedgerHandle,
    overlay_controller: OverlayController,
    settings_manager: ReminderSettingsManager,
    tray_status: TrayStatus,
) -> io::Result<ReminderControl> {
    let (control, receiver) = ReminderControl::channel(tray_status.clone());
    let context = ReminderSchedulerContext {
        app,
        probe_cache,
        activity_tracker,
        break_ledger,
        overlay_controller,
        settings_manager,
        tray_status,
        receiver,
        action_health: control.action_health.clone(),
        next_attempt_id: Arc::clone(&control.next_attempt_id),
    };
    std::thread::Builder::new()
        .name("unfocus-reminders".into())
        .spawn(move || ReminderScheduler::new(context).run())?;
    Ok(control)
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    static TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            for _ in 0..100 {
                let id = TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "unfocus-reminder-tests-{}-{id}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("test settings directory should be created: {error}"),
                }
            }

            panic!("could not allocate a test settings directory")
        }

        fn settings_path(&self) -> PathBuf {
            self.path.join(SETTINGS_FILE_NAME)
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    fn settings(work_minutes: u64, break_seconds: u64) -> ReminderSettings {
        ReminderSettings::try_new(work_minutes, break_seconds, false, 0)
            .expect("valid test settings")
    }

    fn request(work_minutes: Value, break_seconds: Value) -> ReminderSettingsRequest {
        ReminderSettingsRequest {
            work_minutes,
            break_seconds,
            sync_across_devices: false,
            grid_offset_minutes: json!(0),
            pre_break_cue_enabled: true,
        }
    }

    fn fill_synchronization_queue(control: &ReminderControl) {
        for _ in 0..REMINDER_CONTROL_CAPACITY {
            control
                .notify_settings_changed()
                .expect("synchronization queue has capacity");
        }
    }

    #[test]
    fn scheduler_probe_snapshot_preserves_pending_then_recovers_after_failure() {
        use crate::activity::{ActivityKind, ActivityProbeStatus};

        let directory = TestDirectory::new();
        let tracker = ActivityTrackerHandle::initialize(&directory.path);
        let cache = ProbeCache::default();
        let cache_now = Instant::now();
        let activity_now = epoch_ms(SystemTime::now());

        let pending = cache.snapshot();
        observe_activity_snapshot(&tracker, activity_now, &pending);
        let pending_summary = tracker
            .snapshot(activity_now)
            .data
            .expect("activity storage is available");
        assert_eq!(pending_summary.probe_status, ActivityProbeStatus::Pending);
        assert_eq!(pending_summary.current_kind, None);

        cache.update_idle_for_test(Err("actual idle failure".into()), cache_now);
        let failed = cache.snapshot();
        observe_activity_snapshot(&tracker, activity_now + 1, &failed);
        let failed_summary = tracker
            .snapshot(activity_now + 1)
            .data
            .expect("activity storage is available");
        assert_eq!(failed_summary.probe_status, ActivityProbeStatus::Failed);
        assert_eq!(failed_summary.current_kind, Some(ActivityKind::Unknown));

        cache.update_idle_for_test(Ok(0), cache_now);
        let available = cache.snapshot();
        observe_activity_snapshot(&tracker, activity_now + 2, &available);
        let available_summary = tracker
            .snapshot(activity_now + 2)
            .data
            .expect("activity storage is available");
        assert_eq!(
            available_summary.probe_status,
            ActivityProbeStatus::Available
        );
        assert_eq!(available_summary.current_kind, Some(ActivityKind::Active));
    }

    #[test]
    fn reminder_defaults_are_twenty_minutes_and_twenty_seconds() {
        let defaults = ReminderSettings::default();
        let mut timer = ReminderTimer::with_defaults(Duration::ZERO);

        assert_eq!(defaults, settings(20, 20));
        assert_eq!(
            timer.tick(
                defaults.work_interval() - Duration::from_millis(1),
                UNIX_EPOCH
            ),
            None
        );
        assert_eq!(
            timer.tick(defaults.work_interval(), UNIX_EPOCH),
            Some(ReminderTransition::StartBreak)
        );
        assert_eq!(
            timer.tick(
                defaults.work_interval() + defaults.break_duration(),
                UNIX_EPOCH
            ),
            Some(ReminderTransition::EndBreak)
        );
        assert_eq!(
            timer.tick(
                defaults.work_interval() + defaults.break_duration() + defaults.work_interval(),
                UNIX_EPOCH
            ),
            Some(ReminderTransition::StartBreak)
        );
    }

    #[test]
    fn settings_ranges_are_validated_at_the_rust_boundary() {
        assert_eq!(
            settings(1, 3),
            ReminderSettings::try_new(1, 3, false, 0).unwrap()
        );
        assert_eq!(
            settings(120, 30),
            ReminderSettings::try_new(120, 30, false, 0).unwrap()
        );

        for invalid in [
            request(json!(""), json!(20)),
            request(json!(1.5), json!(20)),
            request(json!("twenty"), json!(20)),
            request(json!(-1), json!(20)),
            request(json!(0), json!(20)),
            request(json!(121), json!(20)),
            request(json!(u64::MAX), json!(20)),
            request(json!(20), json!(2)),
            request(json!(20), json!(31)),
        ] {
            assert!(invalid.into_settings().is_err());
        }
    }

    #[test]
    fn sync_settings_default_to_off_with_a_zero_offset() {
        let settings = ReminderSettings::default();
        assert!(!settings.sync_across_devices);
        assert_eq!(settings.grid_offset_minutes, 0);
        assert_eq!(settings.sync_offset(), None);
    }

    #[test]
    fn sync_offset_is_exposed_only_when_sync_is_enabled() {
        let on = ReminderSettings::try_new(20, 20, true, 330).unwrap();
        assert_eq!(on.sync_offset(), Some(330));
        let off = ReminderSettings::try_new(20, 20, false, 330).unwrap();
        assert_eq!(off.sync_offset(), None);
    }

    #[test]
    fn grid_offset_is_validated_against_real_utc_bounds() {
        assert!(ReminderSettings::try_new(20, 20, true, -300).is_ok());
        assert!(ReminderSettings::try_new(20, 20, true, -720).is_ok());
        assert!(ReminderSettings::try_new(20, 20, true, 840).is_ok());
        assert!(ReminderSettings::try_new(20, 20, true, -721).is_err());
        assert!(ReminderSettings::try_new(20, 20, true, 841).is_err());
    }

    #[test]
    fn a_version_one_file_migrates_to_version_four_with_current_defaults() {
        let body = br#"{"version":1,"workMinutes":20,"breakSeconds":20}"#;
        let persisted: PersistedReminderSettings = serde_json::from_slice(body).unwrap();
        let (state, needs_repair) = persisted.into_state(SystemTime::now()).unwrap();
        assert!(needs_repair, "an older version must be rewritten");
        assert!(!state.settings.sync_across_devices);
        assert_eq!(state.settings.grid_offset_minutes, 0);
        assert!(state.settings.pre_break_cue_enabled);
    }

    #[test]
    fn a_version_two_file_migrates_to_version_four_with_current_defaults() {
        let body = br#"{"version":2,"workMinutes":25,"breakSeconds":15,"pauseUntilUnixMilliseconds":null}"#;
        let persisted: PersistedReminderSettings = serde_json::from_slice(body).unwrap();
        let (state, needs_repair) = persisted.into_state(SystemTime::now()).unwrap();
        assert!(needs_repair);
        assert_eq!(state.settings.work_minutes, 25);
        assert!(!state.settings.sync_across_devices);
    }

    #[test]
    fn a_version_three_file_migrates_with_the_cue_enabled() {
        let body = br#"{"version":3,"workMinutes":20,"breakSeconds":20,"pauseUntilUnixMilliseconds":null,"syncAcrossDevices":true,"gridOffsetMinutes":330}"#;
        let persisted: PersistedReminderSettings = serde_json::from_slice(body).unwrap();
        let (state, needs_repair) = persisted.into_state(SystemTime::now()).unwrap();
        assert!(needs_repair);
        assert!(state.settings.sync_across_devices);
        assert_eq!(state.settings.grid_offset_minutes, 330);
        assert!(state.settings.pre_break_cue_enabled);
    }

    #[test]
    fn a_version_four_file_round_trips_without_repair() {
        let body = br#"{"version":4,"workMinutes":20,"breakSeconds":20,"pauseUntilUnixMilliseconds":null,"syncAcrossDevices":true,"gridOffsetMinutes":330,"preBreakCueEnabled":false}"#;
        let persisted: PersistedReminderSettings = serde_json::from_slice(body).unwrap();
        let (state, needs_repair) = persisted.into_state(SystemTime::now()).unwrap();
        assert!(!needs_repair);
        assert!(!state.settings.pre_break_cue_enabled);
        let round_tripped = PersistedReminderSettings::from_state(state).unwrap();
        assert_eq!(round_tripped.version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(round_tripped.grid_offset_minutes, 330);
        assert_eq!(round_tripped.pre_break_cue_enabled, Some(false));
    }

    #[test]
    fn an_unsupported_future_version_fails_to_parse_into_state() {
        let body = br#"{"version":5,"workMinutes":20,"breakSeconds":20}"#;
        let persisted: PersistedReminderSettings = serde_json::from_slice(body).unwrap();
        assert!(persisted.into_state(SystemTime::now()).is_err());
    }

    #[test]
    fn a_negative_grid_offset_survives_the_command_boundary() {
        let request: ReminderSettingsRequest = serde_json::from_str(
            r#"{"workMinutes":20,"breakSeconds":20,"syncAcrossDevices":true,"gridOffsetMinutes":-300}"#,
        )
        .unwrap();
        let settings = request.into_settings().unwrap();
        assert_eq!(settings.grid_offset_minutes, -300);
    }

    #[test]
    fn missing_settings_use_defaults_and_are_persisted() {
        let directory = TestDirectory::new();
        let manager = ReminderSettingsManager::load(&directory.path).unwrap();

        assert_eq!(manager.current(), ReminderSettings::default());
        assert!(directory.settings_path().is_file());
        assert_eq!(
            ReminderSettingsManager::load(&directory.path)
                .unwrap()
                .current(),
            ReminderSettings::default()
        );
    }

    #[test]
    fn saved_settings_survive_a_manager_restart_and_reset_to_defaults() {
        let directory = TestDirectory::new();
        let manager = ReminderSettingsManager::load(&directory.path).unwrap();
        // Sync-on settings here (not the sync-off `settings(45, 12)` helper):
        // starting from defaults on sync_across_devices/grid_offset_minutes
        // would let a reset that never touches those two fields pass by
        // coincidence.
        let sync_on = ReminderSettings::try_new(45, 12, true, 330)
            .unwrap()
            .with_pre_break_cue_enabled(false);
        manager.save(sync_on).unwrap();
        drop(manager);

        let reloaded = ReminderSettingsManager::load(&directory.path).unwrap();
        assert_eq!(reloaded.current(), sync_on);
        assert_eq!(reloaded.reset().unwrap(), ReminderSettings::default());
        drop(reloaded);

        let after_reset = ReminderSettingsManager::load(&directory.path)
            .unwrap()
            .current();
        assert_eq!(after_reset, ReminderSettings::default());
        assert!(!after_reset.sync_across_devices, "reset must clear sync");
        assert_eq!(
            after_reset.grid_offset_minutes, 0,
            "reset must zero the grid offset"
        );
        assert!(
            after_reset.pre_break_cue_enabled,
            "reset must restore the default heads-up"
        );
    }

    #[test]
    fn legacy_settings_are_migrated_without_losing_values() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();
        fs::write(&path, r#"{"version":1,"workMinutes":45,"breakSeconds":12}"#).unwrap();

        let manager = ReminderSettingsManager::load(&directory.path).unwrap();
        assert_eq!(manager.current(), settings(45, 12));
        assert_eq!(manager.snapshot().pause_until, None);

        let migrated: PersistedReminderSettings =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(migrated.version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(migrated.work_minutes, 45);
        assert_eq!(migrated.break_seconds, 12);
        assert_eq!(migrated.pause_until_unix_milliseconds, None);
    }

    #[test]
    fn legacy_settings_with_a_bounded_pause_are_migrated_without_losing_values() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();
        let pause_until =
            system_time_to_unix_milliseconds(SystemTime::now() + Duration::from_secs(60)).unwrap();
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "version": 1,
                "workMinutes": 45,
                "breakSeconds": 12,
                "pauseUntilUnixMilliseconds": pause_until,
            }))
            .unwrap(),
        )
        .unwrap();

        let manager = ReminderSettingsManager::load(&directory.path).unwrap();
        assert_eq!(manager.current(), settings(45, 12));
        assert!(manager.snapshot().pause_until.is_some());

        let migrated: PersistedReminderSettings =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(migrated.version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(migrated.work_minutes, 45);
        assert_eq!(migrated.break_seconds, 12);
        assert_eq!(migrated.pause_until_unix_milliseconds, Some(pause_until));
    }

    #[test]
    fn a_bounded_pause_survives_settings_changes_and_restart_until_resumed() {
        let directory = TestDirectory::new();
        let manager = ReminderSettingsManager::load(&directory.path).unwrap();

        manager.pause_for(PAUSE_DURATION).unwrap();
        let paused_until = manager.snapshot().pause_until.unwrap();
        let remaining = paused_until.duration_since(SystemTime::now()).unwrap();
        assert!(!remaining.is_zero());
        assert!(remaining <= PAUSE_DURATION);

        manager.save(settings(45, 12)).unwrap();
        assert_eq!(manager.snapshot().pause_until, Some(paused_until));
        drop(manager);

        let reloaded = ReminderSettingsManager::load(&directory.path).unwrap();
        assert_eq!(reloaded.current(), settings(45, 12));
        assert!(reloaded.snapshot().pause_until.is_some());
        reloaded.clear_pause().unwrap();
        drop(reloaded);

        assert_eq!(
            ReminderSettingsManager::load(&directory.path)
                .unwrap()
                .snapshot()
                .pause_until,
            None
        );
    }

    #[test]
    fn expired_and_unbounded_persisted_pauses_are_cleared_without_losing_settings() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();
        let now = SystemTime::now();
        let expired = system_time_to_unix_milliseconds(now - Duration::from_secs(1)).unwrap();
        let unbounded =
            system_time_to_unix_milliseconds(now + PAUSE_DURATION + Duration::from_secs(60))
                .unwrap();

        for pause_until in [expired, unbounded] {
            fs::write(
                &path,
                serde_json::to_vec(&json!({
                    "version": SETTINGS_SCHEMA_VERSION,
                    "workMinutes": 45,
                    "breakSeconds": 12,
                    "pauseUntilUnixMilliseconds": pause_until,
                }))
                .unwrap(),
            )
            .unwrap();

            let manager = ReminderSettingsManager::load(&directory.path).unwrap();
            assert_eq!(manager.current(), settings(45, 12));
            assert_eq!(manager.snapshot().pause_until, None);
            let repaired: PersistedReminderSettings =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(repaired.pause_until_unix_milliseconds, None);
        }
    }

    #[test]
    fn a_failed_persistence_write_does_not_publish_runtime_settings() {
        let directory = TestDirectory::new();
        let manager = ReminderSettingsManager::load(&directory.path).unwrap();
        let before = manager.snapshot();
        fs::remove_file(directory.settings_path()).unwrap();
        fs::create_dir(directory.settings_path()).unwrap();

        assert!(manager.save(settings(45, 12)).is_err());
        let after = manager.snapshot();
        assert_eq!(after.settings, before.settings);
        assert_eq!(after.revision, before.revision);
        assert_eq!(after.changed_at, before.changed_at);
        assert_eq!(after.pause_until, before.pause_until);
    }

    #[test]
    fn committed_save_succeeds_with_a_full_notification_queue_and_reconciles_periodically() {
        let directory = TestDirectory::new();
        let manager = ReminderSettingsManager::load(&directory.path).unwrap();
        let initial = manager.snapshot();
        let started_at = initial.changed_at;
        let mut runtime = ReminderSchedulerRuntime::new(Some(initial), Duration::ZERO, UNIX_EPOCH);
        let (control, receiver) = ReminderControl::channel(TrayStatus::default());
        fill_synchronization_queue(&control);

        let committed = save_reminder_settings_and_notify(&manager, &control, settings(45, 12))
            .expect("storage commit is authoritative");

        assert_eq!(committed, settings(45, 12));
        assert_eq!(manager.current(), committed);
        assert_eq!(
            ReminderSettingsManager::load(&directory.path)
                .unwrap()
                .current(),
            committed
        );
        assert_eq!(receiver.try_iter().count(), REMINDER_CONTROL_CAPACITY);
        assert_eq!(
            reconcile_scheduler_iteration(
                &mut runtime,
                manager.authoritative_snapshot(),
                Duration::from_secs(10),
                UNIX_EPOCH,
                started_at
            ),
            SettingsReconciliation::Updated
        );
        assert_eq!(
            runtime.timer().expect("timer remains active").settings,
            committed
        );
    }

    #[test]
    fn successful_retry_returns_health_without_waiting_for_scheduler_acknowledgement() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();
        TEST_SETTINGS_PERSIST_FAILURES
            .lock()
            .expect("hook")
            .push(path);
        let manager = ReminderSettingsManager::initialize(&directory.path);
        let (control, receiver) = ReminderControl::channel(TrayStatus::default());

        let health = retry_reminder_settings_and_notify(&manager, &control);

        assert_eq!(health, StorageLoadHealth::available());
        assert_eq!(manager.current(), ReminderSettings::default());
        let delayed = receiver
            .try_recv()
            .expect("best-effort notification was queued without being consumed");
        assert!(matches!(
            delayed.command,
            ReminderControlCommand::SynchronizeSettings
        ));
        assert!(
            delayed.response.is_none(),
            "notification must not wait for a response"
        );
    }

    #[test]
    fn committed_normal_reset_succeeds_with_a_disconnected_scheduler() {
        let directory = TestDirectory::new();
        let manager = ReminderSettingsManager::load(&directory.path).unwrap();
        manager.save(settings(45, 12)).unwrap();
        let (control, receiver) = ReminderControl::channel(TrayStatus::default());
        drop(receiver);

        let committed = reset_reminder_settings_and_notify(&manager, &control)
            .expect("completed reset is not rolled back by notification failure");

        assert_eq!(committed, ReminderSettings::default());
        assert_eq!(manager.current(), committed);
        assert_eq!(
            ReminderSettingsManager::load(&directory.path)
                .unwrap()
                .current(),
            committed
        );
    }

    #[test]
    fn committed_invalid_file_reset_succeeds_with_a_full_notification_queue() {
        let directory = TestDirectory::new();
        fs::write(directory.settings_path(), b"invalid reminder settings").unwrap();
        let manager = ReminderSettingsManager::initialize(&directory.path);
        let (control, _receiver) = ReminderControl::channel(TrayStatus::default());
        fill_synchronization_queue(&control);

        let committed = reset_reminder_settings_and_notify(&manager, &control)
            .expect("invalid-file reset commit is authoritative");

        assert_eq!(committed, ReminderSettings::default());
        assert_eq!(manager.view().load_health, StorageLoadHealth::available());
        assert_eq!(manager.current(), committed);
    }

    #[test]
    fn precommit_persistence_failure_remains_an_error_and_sends_no_notification() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();
        let manager = ReminderSettingsManager::load(&directory.path).unwrap();
        TEST_SETTINGS_PERSIST_FAILURES
            .lock()
            .expect("hook")
            .push(path);
        let (control, receiver) = ReminderControl::channel(TrayStatus::default());

        assert!(save_reminder_settings_and_notify(&manager, &control, settings(45, 12)).is_err());
        assert_eq!(manager.current(), ReminderSettings::default());
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn a_failed_pause_write_does_not_publish_runtime_pause_state() {
        let directory = TestDirectory::new();
        let manager = ReminderSettingsManager::load(&directory.path).unwrap();
        fs::remove_file(directory.settings_path()).unwrap();
        fs::create_dir(directory.settings_path()).unwrap();

        assert!(manager.pause_for(PAUSE_DURATION).is_err());
        assert_eq!(manager.snapshot().pause_until, None);
    }

    #[test]
    fn malformed_unsupported_and_out_of_range_settings_are_preserved_unavailable() {
        for invalid in [
            "{",
            r#"{"version":1,"workMinutes":0,"breakSeconds":20}"#,
            r#"{"version":1,"workMinutes":20,"breakSeconds":31}"#,
            r#"{"version":99,"workMinutes":20,"breakSeconds":20}"#,
            r#"{"version":1,"workMinutes":20,"breakSeconds":20,"extra":true}"#,
            r#"{"version":3,"workMinutes":45,"breakSeconds":12,"syncAcrossDevices":true,"gridOffsetMinutes":841}"#,
        ] {
            let directory = TestDirectory::new();
            let path = directory.settings_path();
            fs::write(&path, invalid).unwrap();
            let original = fs::read(&path).unwrap();

            let manager = ReminderSettingsManager::initialize(&directory.path);

            assert_eq!(fs::read(&path).unwrap(), original);
            assert!(manager.authoritative_snapshot().is_none());
            assert_eq!(
                manager.view().load_health,
                StorageLoadHealth::unavailable(StorageFailureCategory::Invalid)
            );
            assert!(manager.save(settings(45, 12)).is_err());
        }
    }

    #[test]
    fn missing_settings_become_available_only_after_defaults_persist() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();
        TEST_SETTINGS_PERSIST_FAILURES
            .lock()
            .expect("hook")
            .push(path.clone());

        let manager = ReminderSettingsManager::initialize(&directory.path);

        assert!(manager.authoritative_snapshot().is_none());
        assert!(!path.exists());
        assert_eq!(
            manager.view().load_health,
            StorageLoadHealth::unavailable(StorageFailureCategory::Read)
        );
        assert_eq!(manager.retry_load(), StorageLoadHealth::available());
        assert_eq!(manager.current(), ReminderSettings::default());
        assert!(path.is_file());
    }

    #[test]
    fn failed_legacy_migration_is_unavailable_and_retry_preserves_values_and_pause() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();
        let pause_until =
            system_time_to_unix_milliseconds(SystemTime::now() + Duration::from_secs(60)).unwrap();
        fs::write(
            &path,
            serde_json::to_vec(&json!({
                "version": 2,
                "workMinutes": 45,
                "breakSeconds": 12,
                "pauseUntilUnixMilliseconds": pause_until,
            }))
            .unwrap(),
        )
        .unwrap();
        let original = fs::read(&path).unwrap();
        TEST_SETTINGS_PERSIST_FAILURES
            .lock()
            .expect("hook")
            .push(path.clone());

        let manager = ReminderSettingsManager::initialize(&directory.path);

        assert!(manager.authoritative_snapshot().is_none());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert_eq!(manager.retry_load(), StorageLoadHealth::available());
        let recovered = manager.snapshot();
        assert_eq!(recovered.settings, settings(45, 12));
        assert!(recovered.pause_until.is_some());
        let migrated: PersistedReminderSettings =
            serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
        assert_eq!(migrated.version, SETTINGS_SCHEMA_VERSION);
        assert_eq!(migrated.pause_until_unix_milliseconds, Some(pause_until));
    }

    #[test]
    fn read_failure_initializes_canonical_manager_and_allows_retry_only() {
        let directory = TestDirectory::new();
        let blocked_config = directory.path.join("blocked-config");
        fs::write(&blocked_config, b"blocker").unwrap();
        let canonical = blocked_config.join(SETTINGS_FILE_NAME);
        let manager = ReminderSettingsManager::initialize(&blocked_config);

        assert_eq!(manager.inner.path, canonical);
        assert_eq!(
            manager.view().load_health,
            StorageLoadHealth::unavailable(StorageFailureCategory::Read)
        );
        assert!(manager.save(settings(45, 12)).is_err());
        assert!(manager.reset().is_err());
        assert_eq!(fs::read(&blocked_config).unwrap(), b"blocker");

        fs::remove_file(&blocked_config).unwrap();
        fs::create_dir(&blocked_config).unwrap();
        persist_settings(
            &canonical,
            PersistedReminderState {
                settings: settings(45, 12),
                pause_until: None,
            },
        )
        .unwrap();
        assert_eq!(manager.retry_load(), StorageLoadHealth::available());
        assert_eq!(manager.current(), settings(45, 12));
    }

    #[test]
    fn invalid_reset_quarantines_exact_bytes_then_persists_v4_defaults() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();
        let original = b"invalid reminder settings\0";
        fs::write(&path, original).unwrap();
        let manager = ReminderSettingsManager::initialize(&directory.path);

        assert_eq!(manager.reset().unwrap(), ReminderSettings::default());
        assert_eq!(manager.current(), ReminderSettings::default());
        let persisted: PersistedReminderSettings =
            serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
        assert_eq!(persisted.version, SETTINGS_SCHEMA_VERSION);
        let quarantines: Vec<_> = fs::read_dir(&directory.path)
            .unwrap()
            .filter_map(Result::ok)
            .filter(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("reminder-settings.json.invalid-")
            })
            .collect();
        assert_eq!(quarantines.len(), 1);
        assert_eq!(fs::read(quarantines[0].path()).unwrap(), original);
    }

    #[test]
    fn failed_invalid_reset_preserves_canonical_bytes_and_unavailable_state() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();
        let original = b"invalid reminder settings";
        fs::write(&path, original).unwrap();
        let manager = ReminderSettingsManager::initialize(&directory.path);
        crate::storage_recovery::TEST_QUARANTINE_FAILURES
            .lock()
            .expect("hook")
            .push(path.clone());

        assert!(manager.reset().is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(manager.authoritative_snapshot().is_none());

        TEST_SETTINGS_PERSIST_FAILURES
            .lock()
            .expect("hook")
            .push(path.clone());
        assert!(manager.reset().is_err());
        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(manager.authoritative_snapshot().is_none());
    }

    #[test]
    fn canonical_replacement_failure_preserves_exact_invalid_settings_and_unavailable_state() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();
        let original = b"invalid reminder settings\0replacement";
        fs::write(&path, original).unwrap();
        let manager = ReminderSettingsManager::initialize(&directory.path);
        crate::storage_recovery::inject_replacement_failure(path.clone());

        assert!(manager.reset().is_err());

        assert_eq!(fs::read(&path).unwrap(), original);
        assert!(manager.authoritative_snapshot().is_none());
        assert_eq!(
            manager.view().load_health,
            StorageLoadHealth::unavailable(StorageFailureCategory::Invalid)
        );
    }

    #[test]
    fn invalid_reset_recovers_an_external_valid_repair_instead_of_overwriting_it() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();
        fs::write(&path, b"invalid reminder settings").unwrap();
        let manager = ReminderSettingsManager::initialize(&directory.path);
        persist_settings(
            &path,
            PersistedReminderState {
                settings: settings(45, 12),
                pause_until: None,
            },
        )
        .unwrap();

        assert_eq!(manager.reset().unwrap(), settings(45, 12));
        assert_eq!(manager.current(), settings(45, 12));
        assert_eq!(
            fs::read_dir(&directory.path)
                .unwrap()
                .filter_map(Result::ok)
                .filter(|entry| entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with("reminder-settings.json.invalid-"))
                .count(),
            0
        );
    }

    #[test]
    fn concurrent_external_settings_repair_is_not_overwritten_by_reset() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();
        fs::write(&path, b"invalid reminder settings").unwrap();
        let manager = ReminderSettingsManager::initialize(&directory.path);
        let (started, release) = crate::storage_recovery::install_replacement_barrier(path.clone());
        let recovering = manager.clone();
        let recovery = std::thread::spawn(move || recovering.reset());
        started
            .recv_timeout(Duration::from_secs(1))
            .expect("reset reaches final byte recheck");

        persist_settings(
            &path,
            PersistedReminderState {
                settings: settings(45, 12),
                pause_until: None,
            },
        )
        .unwrap();
        let repaired = fs::read(&path).unwrap();
        release.send(()).unwrap();

        assert!(recovery.join().unwrap().is_err());
        assert_eq!(fs::read(&path).unwrap(), repaired);
        assert!(manager.authoritative_snapshot().is_none());
        assert_eq!(manager.retry_load(), StorageLoadHealth::available());
        assert_eq!(manager.current(), settings(45, 12));
    }

    #[test]
    fn canonical_removal_at_final_recheck_publishes_read_failure() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();
        fs::write(&path, b"invalid reminder settings").unwrap();
        let manager = ReminderSettingsManager::initialize(&directory.path);
        let (started, release) = crate::storage_recovery::install_replacement_barrier(path.clone());
        let recovering = manager.clone();
        let recovery = std::thread::spawn(move || recovering.reset());
        started
            .recv_timeout(Duration::from_secs(1))
            .expect("reset reaches final canonical recheck");

        fs::remove_file(&path).unwrap();
        release.send(()).unwrap();

        assert!(recovery.join().unwrap().is_err());
        assert_eq!(
            manager.view().load_health,
            StorageLoadHealth::unavailable(StorageFailureCategory::Read)
        );
        assert!(
            manager.reset().is_err(),
            "read failures must remove reset capability"
        );
        assert_eq!(manager.retry_load(), StorageLoadHealth::available());
    }

    #[test]
    fn unchanged_and_cue_only_synchronization_preserve_the_current_cue_revision() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(20, 20), UNIX_EPOCH);
        let changed_at = Duration::from_secs(10 * 60);
        let before = timer.tray_snapshot(changed_at, UNIX_EPOCH, 0, false);
        let current_cue_attempt = Some(before.state_revision);
        let current_cue_revision = Some(before.state_revision);
        let synchronization = ReminderControlRequest {
            attempt_id: 0,
            command: ReminderControlCommand::SynchronizeSettings,
            response: None,
        };

        assert!(!synchronization.cancels_pre_break_cue());
        assert_eq!(current_cue_attempt, Some(before.state_revision));
        assert_eq!(current_cue_revision, Some(before.state_revision));

        timer.apply_settings(
            settings(20, 20).with_pre_break_cue_enabled(false),
            changed_at,
            UNIX_EPOCH,
        );

        let after = timer.tray_snapshot(changed_at, UNIX_EPOCH, 1, false);
        assert_eq!(after.remaining_milliseconds, before.remaining_milliseconds);
        assert_eq!(after.state_revision, before.state_revision);
        assert_eq!(current_cue_attempt, Some(after.state_revision));
        assert_eq!(current_cue_revision, Some(after.state_revision));
        assert!(!timer.settings.pre_break_cue_enabled);
    }

    #[test]
    fn every_real_user_action_request_cancels_the_current_cue() {
        for action in [
            ReminderAction::Pause,
            ReminderAction::Resume,
            ReminderAction::TakeBreakNow,
        ] {
            let request = ReminderControlRequest {
                attempt_id: 1,
                command: ReminderControlCommand::Action(action),
                response: None,
            };
            assert!(request.cancels_pre_break_cue(), "{action:?}");
        }
    }

    #[test]
    fn saving_during_work_restarts_the_countdown_at_the_save_time() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(20, 20), UNIX_EPOCH);

        assert_eq!(timer.tick(Duration::from_secs(10 * 60), UNIX_EPOCH), None);
        timer.apply_settings(settings(1, 8), Duration::from_secs(10 * 60), UNIX_EPOCH);
        let snapshot = timer.tray_snapshot(Duration::from_secs(10 * 60), UNIX_EPOCH, 1, false);
        assert_eq!(snapshot.settings_revision, 1);
        assert_eq!(snapshot.state_revision, 1);
        assert_eq!(snapshot.presentation().status, "Working · break in 1 min");
        assert_eq!(
            timer.tick(Duration::from_secs(10 * 60 + 59), UNIX_EPOCH),
            None
        );
        assert_eq!(
            timer.tick(Duration::from_secs(11 * 60), UNIX_EPOCH),
            Some(ReminderTransition::StartBreak)
        );
        assert_eq!(timer.break_duration(), Duration::from_secs(8));
    }

    #[test]
    fn saving_during_a_break_preserves_that_break_and_updates_the_next_work_phase() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(1, 3), UNIX_EPOCH);

        assert_eq!(
            timer.tick(Duration::from_secs(60), UNIX_EPOCH),
            Some(ReminderTransition::StartBreak)
        );
        timer.apply_settings(settings(2, 30), Duration::from_secs(61), UNIX_EPOCH);
        assert_eq!(timer.break_duration(), Duration::from_secs(3));
        let break_snapshot = timer.tray_snapshot(Duration::from_secs(61), UNIX_EPOCH, 1, true);
        assert_eq!(break_snapshot.phase, TrayPhase::Break);
        assert_eq!(break_snapshot.settings_revision, 1);
        assert_eq!(break_snapshot.state_revision, 2);
        assert_eq!(break_snapshot.presentation().status, "Break in progress");
        assert_eq!(timer.tick(Duration::from_secs(62), UNIX_EPOCH), None);
        assert_eq!(
            timer.tick(Duration::from_secs(63), UNIX_EPOCH),
            Some(ReminderTransition::EndBreak)
        );
        assert_eq!(
            timer
                .tray_snapshot(Duration::from_secs(63), UNIX_EPOCH, 1, false)
                .presentation()
                .status,
            "Working · break in 2 min"
        );
        assert_eq!(timer.tick(Duration::from_secs(182), UNIX_EPOCH), None);
        assert_eq!(
            timer.tick(Duration::from_secs(183), UNIX_EPOCH),
            Some(ReminderTransition::StartBreak)
        );
        assert_eq!(timer.break_duration(), Duration::from_secs(30));
    }

    #[test]
    fn pause_is_bounded_idempotent_and_resumes_with_a_fresh_work_interval() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(20, 20), UNIX_EPOCH);
        let paused_at = Duration::from_secs(10 * 60);

        assert!(timer.pause(paused_at));
        let paused_until = timer.paused_until;
        assert!(!timer.pause(paused_at + Duration::from_secs(1)));
        assert_eq!(timer.paused_until, paused_until);
        assert_eq!(timer.state_revision, 1);

        let snapshot = timer.tray_snapshot(paused_at, UNIX_EPOCH, 4, false);
        assert_eq!(snapshot.phase, TrayPhase::Paused);
        assert_eq!(snapshot.remaining_milliseconds, None);
        assert_eq!(
            snapshot.pause_expires_in_milliseconds,
            Some(PAUSE_DURATION.as_millis().try_into().unwrap())
        );
        let status = ReminderStatus::from_snapshot(snapshot);
        assert_eq!(status.status, "Paused · resumes in 30 min");
        assert_eq!(status.pause_action, ReminderPauseAction::Resume);
        assert!(status.pause_action_enabled);
        assert!(!status.take_break_enabled);
        assert!(status.preview_enabled);

        assert_eq!(
            timer.tick(
                paused_at + PAUSE_DURATION - Duration::from_millis(1),
                UNIX_EPOCH
            ),
            None
        );
        assert_eq!(
            timer.tick(paused_at + PAUSE_DURATION, UNIX_EPOCH),
            Some(ReminderTransition::ResumeWorking)
        );
        assert_eq!(timer.phase, ReminderPhase::Working);
        assert_eq!(timer.state_revision, 2);
        assert_eq!(
            timer.tick(
                paused_at + PAUSE_DURATION + Duration::from_secs(20 * 60 - 1),
                UNIX_EPOCH
            ),
            None
        );
        assert_eq!(
            timer.tick(
                paused_at + PAUSE_DURATION + Duration::from_secs(20 * 60),
                UNIX_EPOCH
            ),
            Some(ReminderTransition::StartBreak)
        );
    }

    #[test]
    fn visible_overlays_disable_actions_that_would_create_another_run() {
        let timer = ReminderTimer::new(Duration::ZERO, settings(20, 20), UNIX_EPOCH);
        let status =
            ReminderStatus::from_snapshot(timer.tray_snapshot(Duration::ZERO, UNIX_EPOCH, 0, true));

        assert!(!status.take_break_enabled);
        assert!(!status.preview_enabled);
        assert!(status.pause_action_enabled);
    }

    #[test]
    fn manual_resume_is_idempotent_and_settings_saved_while_paused_apply_afterward() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(20, 20), UNIX_EPOCH);
        assert!(timer.pause(Duration::from_secs(60)));
        timer.apply_settings(settings(1, 8), Duration::from_secs(90), UNIX_EPOCH);
        assert_eq!(timer.phase, ReminderPhase::Paused);
        assert_eq!(timer.state_revision, 2);

        assert!(timer.resume(Duration::from_secs(120), UNIX_EPOCH));
        assert!(!timer.resume(Duration::from_secs(121), UNIX_EPOCH));
        assert_eq!(timer.state_revision, 3);
        assert_eq!(timer.tick(Duration::from_secs(179), UNIX_EPOCH), None);
        assert_eq!(
            timer.tick(Duration::from_secs(180), UNIX_EPOCH),
            Some(ReminderTransition::StartBreak)
        );
        assert_eq!(timer.break_duration(), Duration::from_secs(8));
    }

    #[test]
    fn pause_persistence_failures_leave_the_timer_state_unchanged() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(20, 20), UNIX_EPOCH);
        let result = start_bounded_pause(&mut timer, Duration::from_secs(60), || {
            Err("settings write failed".into())
        });
        assert_eq!(result, Err("settings write failed".into()));
        assert_eq!(timer.phase, ReminderPhase::Working);
        assert_eq!(timer.state_revision, 0);

        assert!(timer.pause(Duration::from_secs(60)));
        let paused_until = timer.paused_until;
        let result = resume_from_pause(&mut timer, Duration::from_secs(120), UNIX_EPOCH, || {
            Err("settings write failed".into())
        });
        assert_eq!(result, Err("settings write failed".into()));
        assert_eq!(timer.phase, ReminderPhase::Paused);
        assert_eq!(timer.paused_until, paused_until);
        assert_eq!(timer.state_revision, 1);
    }

    #[test]
    fn successful_control_helpers_apply_every_transition_in_release_builds() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(20, 8), UNIX_EPOCH);

        start_bounded_pause(&mut timer, Duration::from_secs(60), || Ok(())).unwrap();
        assert_eq!(timer.phase, ReminderPhase::Paused);

        resume_from_pause(&mut timer, Duration::from_secs(120), UNIX_EPOCH, || Ok(())).unwrap();
        assert_eq!(timer.phase, ReminderPhase::Working);

        start_manual_break(&mut timer, Duration::from_secs(180), |_| Ok(())).unwrap();
        assert_eq!(timer.phase, ReminderPhase::Break);
        assert_eq!(timer.state_revision, 3);
    }

    #[test]
    fn take_break_now_is_a_single_authoritative_transition() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(20, 8), UNIX_EPOCH);
        let requested_at = Duration::from_secs(5 * 60);

        assert!(timer.take_break_now(requested_at));
        assert!(!timer.take_break_now(requested_at + Duration::from_secs(1)));
        assert_eq!(timer.phase, ReminderPhase::Break);
        assert_eq!(timer.state_revision, 1);
        assert_eq!(
            timer
                .tray_snapshot(requested_at, UNIX_EPOCH, 0, true)
                .presentation()
                .status,
            "Break in progress"
        );
        assert_eq!(
            timer.tick(requested_at + Duration::from_secs(7), UNIX_EPOCH),
            None
        );
        assert_eq!(
            timer.tick(requested_at + Duration::from_secs(8), UNIX_EPOCH),
            Some(ReminderTransition::EndBreak)
        );
        assert_eq!(timer.phase, ReminderPhase::Working);

        assert!(timer.pause(requested_at + Duration::from_secs(9)));
        assert!(!timer.take_break_now(requested_at + Duration::from_secs(10)));
        assert_eq!(timer.phase, ReminderPhase::Paused);
    }

    #[test]
    fn a_manual_overlay_failure_leaves_the_timer_unchanged() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(20, 8), UNIX_EPOCH);
        let before = timer.tray_snapshot(Duration::from_secs(5 * 60), UNIX_EPOCH, 0, false);

        let result = start_manual_break(&mut timer, Duration::from_secs(5 * 60), |duration| {
            assert_eq!(duration, Duration::from_secs(8));
            Err("monitor enumeration failed".into())
        });

        assert_eq!(result, Err("monitor enumeration failed".into()));
        assert_eq!(timer.phase, ReminderPhase::Working);
        assert_eq!(timer.state_revision, 0);
        assert_eq!(
            timer.tray_snapshot(Duration::from_secs(5 * 60), UNIX_EPOCH, 0, false),
            before
        );
    }

    #[test]
    fn a_repeated_manual_break_does_not_present_or_extend_the_active_break() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(20, 8), UNIX_EPOCH);
        assert!(timer.take_break_now(Duration::from_secs(60)));
        let before = timer.tray_snapshot(Duration::from_secs(61), UNIX_EPOCH, 0, true);

        start_manual_break(&mut timer, Duration::from_secs(62), |_| {
            panic!("an active break must not create another overlay run")
        })
        .unwrap();

        assert_eq!(timer.phase, ReminderPhase::Break);
        assert_eq!(timer.state_revision, 1);
        assert_eq!(
            timer.tray_snapshot(Duration::from_secs(61), UNIX_EPOCH, 0, true),
            before
        );
    }

    #[test]
    fn control_queue_failures_are_published_without_changing_timer_state() {
        let tray_status = TrayStatus::default();
        tray_status.publish(TraySnapshot::timer(
            TrayPhase::Working,
            Duration::from_secs(20 * 60),
            false,
            0,
            0,
        ));
        let before = tray_status.current();
        let (control, _receiver) = ReminderControl::channel(tray_status.clone());

        for _ in 0..REMINDER_CONTROL_CAPACITY {
            control.dispatch(ReminderAction::Pause).unwrap();
        }
        assert_eq!(
            control.dispatch(ReminderAction::Pause),
            Err("reminder control queue is full".into())
        );

        let failed = tray_status.current();
        assert_eq!(failed.phase, before.phase);
        assert_eq!(failed.state_revision, before.state_revision);
        assert_eq!(
            failed.action_error.as_deref(),
            Some("reminder control queue is full")
        );
        let reminder = ReminderStatus::from_snapshot(failed);
        assert_eq!(reminder.status, "Working · break in 20 min");
        assert_eq!(reminder.tray_status(), "Action failed · open Unfocus");
    }

    #[test]
    fn explicit_unavailable_settings_cannot_tick_or_fire_transitions() {
        let directory = TestDirectory::new();
        fs::write(directory.settings_path(), b"invalid reminder settings").unwrap();
        let manager = ReminderSettingsManager::initialize(&directory.path);
        assert_eq!(
            manager.view().load_health,
            StorageLoadHealth::unavailable(StorageFailureCategory::Invalid)
        );
        let started_at = Instant::now();
        let mut runtime = ReminderSchedulerRuntime::new(None, Duration::ZERO, UNIX_EPOCH);

        for now in [Duration::ZERO, Duration::from_secs(86_400)] {
            assert_eq!(
                reconcile_scheduler_iteration(
                    &mut runtime,
                    manager.authoritative_snapshot(),
                    now,
                    UNIX_EPOCH,
                    started_at
                ),
                SettingsReconciliation::Unavailable
            );
            assert_eq!(runtime.tick(now, UNIX_EPOCH), None);
            assert!(runtime.timer().is_none());
        }
    }

    #[test]
    fn unavailable_controls_and_preview_are_disabled_before_queueing() {
        let tray_status = TrayStatus::default();
        let (control, receiver) = ReminderControl::channel(tray_status.clone());
        let status = ReminderStatus::from_snapshot(tray_status.current());

        assert!(!status.pause_action_enabled);
        assert!(!status.take_break_enabled);
        assert!(!status.preview_enabled);
        assert!(control.dispatch(ReminderAction::Pause).is_err());
        assert!(control.dispatch(ReminderAction::TakeBreakNow).is_err());
        assert!(matches!(
            receiver.try_recv(),
            Err(mpsc::TryRecvError::Empty)
        ));
    }

    #[test]
    fn repeated_recovery_reconciliation_keeps_one_unchanged_active_timer() {
        let recovered_at = Duration::from_secs(10_000);
        let started_at = Instant::now();
        let snapshot = ReminderSettingsSnapshot {
            settings: settings(1, 5),
            revision: 0,
            changed_at: started_at,
            pause_until: None,
        };
        let mut runtime = ReminderSchedulerRuntime::new(None, Duration::ZERO, UNIX_EPOCH);

        assert_eq!(
            reconcile_scheduler_iteration(
                &mut runtime,
                Some(snapshot),
                recovered_at,
                UNIX_EPOCH,
                started_at
            ),
            SettingsReconciliation::Activated
        );
        let activated_at = runtime.timer().expect("active timer").phase_started_at;
        assert_eq!(runtime.tick(recovered_at, UNIX_EPOCH), None);

        for elapsed in [1, 30, 59] {
            let now = recovered_at + Duration::from_secs(elapsed);
            assert_eq!(
                reconcile_scheduler_iteration(
                    &mut runtime,
                    Some(snapshot),
                    now,
                    UNIX_EPOCH,
                    started_at
                ),
                SettingsReconciliation::Unchanged
            );
            assert_eq!(
                runtime.timer().expect("same active timer").phase_started_at,
                activated_at,
                "unchanged recovery reconciliation must not restart the timer"
            );
            assert_eq!(runtime.tick(now, UNIX_EPOCH), None);
        }
        assert_eq!(
            runtime.tick(recovered_at + Duration::from_secs(60), UNIX_EPOCH),
            Some(ReminderTransition::StartBreak)
        );
    }

    #[test]
    fn recovery_restores_only_a_still_valid_bounded_pause() {
        let wall_now = SystemTime::now();
        let started_at = Instant::now();
        let snapshot = ReminderSettingsSnapshot {
            settings: settings(20, 20),
            revision: 0,
            changed_at: started_at,
            pause_until: wall_now.checked_add(Duration::from_secs(60)),
        };
        let mut runtime = ReminderSchedulerRuntime::new(None, Duration::ZERO, wall_now);

        assert_eq!(
            reconcile_scheduler_iteration(
                &mut runtime,
                Some(snapshot),
                Duration::from_secs(500),
                wall_now,
                started_at
            ),
            SettingsReconciliation::Activated
        );
        let timer = runtime.timer().expect("recovered timer");
        assert_eq!(timer.phase, ReminderPhase::Paused);
        assert_eq!(timer.paused_until, Some(Duration::from_secs(560)));
    }

    #[test]
    fn an_older_queued_success_cannot_clear_a_newer_action_failure() {
        let health = ReminderActionHealth::default();
        health.record_failure(7, "newer action failed".into());

        health.clear(6);
        assert_eq!(health.current().as_deref(), Some("newer action failed"));

        health.clear(8);
        assert_eq!(health.current(), None);
    }

    #[test]
    fn a_restored_pause_uses_only_monotonic_elapsed_time_after_startup() {
        let mut timer = ReminderTimer::new_paused(
            Duration::from_secs(100),
            settings(20, 20),
            Duration::from_secs(90),
            UNIX_EPOCH,
        );

        assert_eq!(timer.tick(Duration::from_secs(99), UNIX_EPOCH), None);
        assert_eq!(timer.tick(Duration::from_secs(189), UNIX_EPOCH), None);
        assert_eq!(
            timer.tick(Duration::from_secs(190), UNIX_EPOCH),
            Some(ReminderTransition::ResumeWorking)
        );
    }

    #[test]
    fn reminder_clock_is_injected_and_does_not_replay_missed_cycles() {
        let mut timer = ReminderTimer::new(Duration::from_secs(10), settings(1, 5), UNIX_EPOCH);

        assert_eq!(timer.tick(Duration::from_secs(69), UNIX_EPOCH), None);
        assert_eq!(
            timer.tick(Duration::from_secs(600), UNIX_EPOCH),
            Some(ReminderTransition::StartBreak)
        );
        assert_eq!(timer.tick(Duration::from_secs(600), UNIX_EPOCH), None);
        assert_eq!(
            timer.tick(Duration::from_secs(605), UNIX_EPOCH),
            Some(ReminderTransition::EndBreak)
        );
    }

    #[test]
    fn a_clock_regression_does_not_advance_the_reminder() {
        let mut timer = ReminderTimer::new(Duration::from_secs(100), settings(1, 5), UNIX_EPOCH);

        assert_eq!(timer.tick(Duration::from_secs(90), UNIX_EPOCH), None);
        assert_eq!(timer.phase, ReminderPhase::Working);
    }

    #[test]
    fn tray_snapshot_comes_from_the_timer_and_never_goes_negative() {
        let mut timer = ReminderTimer::new(Duration::from_secs(100), settings(1, 5), UNIX_EPOCH);

        let regressed = timer.tray_snapshot(Duration::from_secs(90), UNIX_EPOCH, 4, false);
        assert_eq!(regressed.phase, TrayPhase::Working);
        assert_eq!(regressed.remaining_milliseconds, Some(60_000));
        assert_eq!(regressed.settings_revision, 4);
        assert_eq!(regressed.state_revision, 0);
        assert!(!regressed.overlay_active);

        let almost_due = timer.tray_snapshot(Duration::from_millis(159_999), UNIX_EPOCH, 4, false);
        assert_eq!(almost_due.remaining_milliseconds, Some(1));

        assert_eq!(
            timer.tick(Duration::from_secs(600), UNIX_EPOCH),
            Some(ReminderTransition::StartBreak)
        );
        let break_snapshot = timer.tray_snapshot(Duration::from_secs(600), UNIX_EPOCH, 4, true);
        assert_eq!(break_snapshot.phase, TrayPhase::Break);
        assert_eq!(break_snapshot.remaining_milliseconds, Some(5_000));
        assert_eq!(break_snapshot.state_revision, 1);
        assert!(break_snapshot.overlay_active);

        assert_eq!(
            timer.tick(Duration::from_secs(605), UNIX_EPOCH),
            Some(ReminderTransition::EndBreak)
        );
        let resumed = timer.tray_snapshot(Duration::from_secs(605), UNIX_EPOCH, 4, false);
        assert_eq!(resumed.phase, TrayPhase::Working);
        assert_eq!(resumed.remaining_milliseconds, Some(60_000));
        assert_eq!(resumed.state_revision, 2);
    }

    fn probe_readings(
        idle_seconds: ProbeReading<u64>,
        active_window_fullscreen: ProbeReading<bool>,
    ) -> ProbeSnapshot {
        ProbeSnapshot {
            idle_seconds,
            active_window_fullscreen,
        }
    }

    fn probes(idle_seconds: u64, active_window_fullscreen: bool) -> ProbeSnapshot {
        ProbeSnapshot::available(idle_seconds, active_window_fullscreen)
    }

    fn no_history() -> Option<ActivityPresentationContext> {
        None
    }

    fn long_active_context() -> Option<ActivityPresentationContext> {
        Some(ActivityPresentationContext {
            history_available: true,
            continuous_active_seconds: LONG_ACTIVE_SECONDS,
            recent_afk_seconds: 0,
        })
    }

    fn long_afk_context() -> Option<ActivityPresentationContext> {
        Some(ActivityPresentationContext {
            history_available: true,
            continuous_active_seconds: 0,
            recent_afk_seconds: LONG_AFK_SECONDS,
        })
    }

    #[test]
    fn configured_break_duration_controls_idle_suppression() {
        let idle = probes(8, false);

        assert_eq!(
            break_presentation(&idle, Duration::from_secs(8), no_history()),
            BreakPresentation::NaturalIdle
        );
        assert_eq!(
            break_presentation(&idle, Duration::from_secs(20), no_history()),
            BreakPresentation::Show
        );
    }

    #[test]
    fn probes_only_control_break_presentation() {
        let break_duration = ReminderSettings::default().break_duration();
        let active = probes(0, false);
        let idle = probes(break_duration.as_secs(), false);
        let fullscreen = probes(0, true);
        let failed = ProbeSnapshot::failed("idle failed", "fullscreen failed");
        let pending = ProbeSnapshot::pending();

        assert_eq!(
            break_presentation(&active, break_duration, no_history()),
            BreakPresentation::Show
        );
        assert_eq!(
            break_presentation(&idle, break_duration, no_history()),
            BreakPresentation::NaturalIdle
        );
        assert_eq!(
            break_presentation(&fullscreen, break_duration, no_history()),
            BreakPresentation::SuppressFullscreen
        );
        assert_eq!(
            break_presentation(&failed, break_duration, no_history()),
            BreakPresentation::Show
        );
        assert_eq!(
            break_presentation(&pending, break_duration, no_history()),
            BreakPresentation::Show
        );

        // Pure timer advancement has no probe input. Natural-break credit is a
        // separate scheduler step after StartBreak, not inside tick().
        for probes in [&active, &idle, &fullscreen, &failed, &pending] {
            let mut timer = ReminderTimer::new(Duration::ZERO, settings(1, 3), UNIX_EPOCH);
            let _ = break_presentation(probes, Duration::from_secs(3), no_history());
            assert_eq!(
                timer.tick(Duration::from_secs(60), UNIX_EPOCH),
                Some(ReminderTransition::StartBreak)
            );
            assert_eq!(
                timer.tick(Duration::from_secs(63), UNIX_EPOCH),
                Some(ReminderTransition::EndBreak)
            );
        }
    }

    #[test]
    fn pending_fullscreen_fails_open_like_an_actual_fullscreen_failure() {
        let pending = probe_readings(ProbeReading::Available(0), ProbeReading::Pending);
        let failed = probe_readings(
            ProbeReading::Available(0),
            ProbeReading::Failed("fullscreen failed".into()),
        );

        for snapshot in [&pending, &failed] {
            assert_eq!(
                break_presentation(snapshot, Duration::from_secs(20), no_history()),
                BreakPresentation::Show
            );
        }
    }

    #[test]
    fn long_active_requires_real_afk_for_natural_credit() {
        let break_duration = Duration::from_secs(20);
        let micro_idle = probes(20, false);
        let real_afk = probes(LONG_AFK_SECONDS, false);
        assert_eq!(
            break_presentation(&micro_idle, break_duration, long_active_context()),
            BreakPresentation::Show
        );
        assert_eq!(
            break_presentation(&real_afk, break_duration, long_active_context()),
            BreakPresentation::NaturalIdle
        );
    }

    #[test]
    fn long_afk_prefers_natural_when_still_idle() {
        let break_duration = Duration::from_secs(20);
        let still_idle = probes(20, false);
        let typing = probes(0, false);
        assert_eq!(
            break_presentation(&still_idle, break_duration, long_afk_context()),
            BreakPresentation::NaturalIdle
        );
        assert_eq!(
            break_presentation(&typing, break_duration, long_afk_context()),
            BreakPresentation::Show
        );
    }

    #[test]
    fn fullscreen_wins_over_long_active_adaptation() {
        let probes = probes(0, true);
        assert_eq!(
            break_presentation(&probes, Duration::from_secs(20), long_active_context()),
            BreakPresentation::SuppressFullscreen
        );
    }

    #[test]
    fn empty_history_uses_legacy_idle_rules() {
        let break_duration = Duration::from_secs(20);
        let idle = probes(20, false);
        let empty = Some(ActivityPresentationContext {
            history_available: false,
            continuous_active_seconds: LONG_ACTIVE_SECONDS,
            recent_afk_seconds: LONG_AFK_SECONDS,
        });
        assert_eq!(
            break_presentation(&idle, break_duration, empty),
            BreakPresentation::NaturalIdle
        );
    }

    #[test]
    fn natural_break_credit_returns_to_fresh_work_immediately() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(1, 20), UNIX_EPOCH);
        assert_eq!(
            timer.tick(Duration::from_secs(60), UNIX_EPOCH),
            Some(ReminderTransition::StartBreak)
        );
        assert_eq!(timer.phase, ReminderPhase::Break);

        assert!(timer.credit_natural_break(Duration::from_secs(60), UNIX_EPOCH));
        assert_eq!(timer.phase, ReminderPhase::Working);
        let snapshot = timer.tray_snapshot(Duration::from_secs(60), UNIX_EPOCH, 0, false);
        assert_eq!(snapshot.phase, TrayPhase::Working);
        assert_eq!(snapshot.remaining_milliseconds, Some(60_000));
        assert_eq!(snapshot.state_revision, 2);

        assert!(!timer.credit_natural_break(Duration::from_secs(61), UNIX_EPOCH));
    }

    #[test]
    fn natural_break_credit_applies_pending_settings() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(1, 20), UNIX_EPOCH);
        assert_eq!(
            timer.tick(Duration::from_secs(60), UNIX_EPOCH),
            Some(ReminderTransition::StartBreak)
        );
        timer.apply_settings(settings(2, 10), Duration::from_secs(61), UNIX_EPOCH);
        assert!(timer.credit_natural_break(Duration::from_secs(62), UNIX_EPOCH));
        assert_eq!(timer.phase, ReminderPhase::Working);
        assert_eq!(timer.settings, settings(2, 10));
        let snapshot = timer.tray_snapshot(Duration::from_secs(62), UNIX_EPOCH, 1, false);
        assert_eq!(snapshot.remaining_milliseconds, Some(120_000));
    }

    #[test]
    fn fullscreen_outranks_idle_for_presentation_contract() {
        // Issue #61: fullscreen is never overridden, including when idle would
        // otherwise natural-credit.
        let both = probes(30, true);
        assert_eq!(
            break_presentation(&both, Duration::from_secs(20), no_history()),
            BreakPresentation::SuppressFullscreen
        );
    }

    fn sync_settings(work_minutes: u64) -> ReminderSettings {
        ReminderSettings::try_new(work_minutes, 20, true, 330).unwrap()
    }

    /// 2026-08-20, local IST time, as a SystemTime.
    fn ist(hh: i64, mm: i64) -> SystemTime {
        system_time_from_unix_seconds(1_787_184_000 + hh * 3600 + mm * 60 - 330 * 60)
    }

    #[test]
    fn sync_mode_ends_the_work_phase_on_the_grid_not_on_the_interval() {
        let mut timer = ReminderTimer::new(Duration::ZERO, sync_settings(20), ist(10, 1));
        // 10:01 start takes the 10:20 grid point; nothing fires before it.
        assert_eq!(timer.tick(Duration::from_secs(600), ist(10, 11)), None);
        assert_eq!(
            timer.tick(Duration::from_secs(1140), ist(10, 20)),
            Some(ReminderTransition::StartBreak)
        );
    }

    #[test]
    fn sync_mode_skips_a_grid_point_that_was_not_earned() {
        let mut timer = ReminderTimer::new(Duration::ZERO, sync_settings(20), ist(10, 19));
        // 10:20 is one minute away, so it is skipped in favour of 10:40.
        assert_eq!(timer.tick(Duration::from_secs(60), ist(10, 20)), None);
        assert_eq!(
            timer.tick(Duration::from_secs(1260), ist(10, 40)),
            Some(ReminderTransition::StartBreak)
        );
    }

    #[test]
    fn a_break_ending_off_grid_re_grids_to_the_next_whole_point() {
        let mut timer = ReminderTimer::new(Duration::ZERO, sync_settings(20), ist(10, 1));
        timer.tick(Duration::from_secs(1140), ist(10, 20)).unwrap(); // StartBreak

        // The twenty-second break ends at 10:20:20; the next break is 10:40:00,
        // not 10:40:20.
        assert_eq!(
            timer.tick(
                Duration::from_secs(1160),
                ist(10, 20) + Duration::from_secs(20)
            ),
            Some(ReminderTransition::EndBreak)
        );
        assert_eq!(timer.work_deadline, WorkDeadline::Wall(ist(10, 40)));
    }

    #[test]
    fn settings_changed_during_a_break_apply_before_the_next_deadline_is_computed() {
        let mut timer = ReminderTimer::new(Duration::ZERO, sync_settings(20), ist(10, 1));
        timer.tick(Duration::from_secs(1140), ist(10, 20)).unwrap(); // StartBreak

        // Switch to a ten-minute rhythm mid-break.
        timer.apply_settings(sync_settings(10), Duration::from_secs(1145), ist(10, 20));
        timer
            .tick(
                Duration::from_secs(1160),
                ist(10, 20) + Duration::from_secs(20),
            )
            .unwrap();
        // A ten-minute grid puts the next break at 10:30, not 10:40.
        assert_eq!(timer.work_deadline, WorkDeadline::Wall(ist(10, 30)));
    }

    #[test]
    fn toggling_sync_on_then_off_mid_working_flips_the_deadline_kind() {
        let mut timer = ReminderTimer::new(Duration::ZERO, ReminderSettings::default(), ist(10, 1));
        assert_eq!(timer.work_deadline, WorkDeadline::Relative);

        // Turning sync on while Working must compute and store a grid
        // deadline immediately, not just take effect on the next natural
        // Working entry.
        timer.apply_settings(sync_settings(20), Duration::from_secs(60), ist(10, 1));
        assert_eq!(timer.work_deadline, WorkDeadline::Wall(ist(10, 20)));

        // Turning sync back off must drop the grid deadline just as
        // immediately.
        timer.apply_settings(
            ReminderSettings::default(),
            Duration::from_secs(120),
            ist(10, 1),
        );
        assert_eq!(timer.work_deadline, WorkDeadline::Relative);
    }

    #[test]
    fn a_manual_break_does_not_push_the_shared_grid() {
        let mut timer = ReminderTimer::new(Duration::ZERO, sync_settings(20), ist(10, 1));
        // `take_break_now` enters Break, never Working, so it never touches the
        // shared grid deadline and takes no wall clock.
        timer.take_break_now(Duration::from_secs(240));
        assert_eq!(
            timer.tick(
                Duration::from_secs(260),
                ist(10, 5) + Duration::from_secs(20),
            ),
            Some(ReminderTransition::EndBreak)
        );
        // Intended: the 10:20 grid break still fires after a 10:05 manual break.
        assert_eq!(timer.work_deadline, WorkDeadline::Wall(ist(10, 20)));
        // The stored field alone would still read Wall(10:20) even if the
        // EndBreak re-grid were deleted, because the constructor already set
        // that value at ist(10, 1). Driving the timer to the deadline proves
        // the re-grid actually ran and left Working armed to fire there.
        assert_eq!(
            timer.tick(Duration::from_secs(1200), ist(10, 20)),
            Some(ReminderTransition::StartBreak)
        );
    }

    #[test]
    fn a_manual_break_inside_grace_still_skips_to_the_next_grid_point() {
        let mut timer = ReminderTimer::new(Duration::ZERO, sync_settings(20), ist(10, 1));
        // A manual break at 10:15; the 20s break ends at 10:15:20, only 280s
        // before the 10:20 grid point -- inside the 600s grace threshold, so
        // grace must skip it in favour of 10:40. (The companion 10:05 case
        // above is 880s away, clear of the threshold either way, so it can't
        // by itself prove grace ran.)
        timer.take_break_now(Duration::from_secs(14 * 60));
        assert_eq!(
            timer.tick(
                Duration::from_secs(14 * 60 + 20),
                ist(10, 15) + Duration::from_secs(20),
            ),
            Some(ReminderTransition::EndBreak)
        );
        assert_eq!(timer.work_deadline, WorkDeadline::Wall(ist(10, 40)));
    }

    #[test]
    fn relative_mode_still_ends_the_work_phase_on_elapsed_time() {
        let settings = ReminderSettings::default(); // sync off
        let mut timer = ReminderTimer::new(Duration::ZERO, settings, ist(10, 1));
        assert_eq!(timer.work_deadline, WorkDeadline::Relative);
        assert_eq!(timer.tick(Duration::from_secs(1199), ist(10, 21)), None);
        assert_eq!(
            timer.tick(Duration::from_secs(1200), ist(10, 21)),
            Some(ReminderTransition::StartBreak)
        );
    }

    #[test]
    fn the_countdown_targets_the_grid_deadline_not_the_interval() {
        // Entering Working at 10:39 skips the 10:40 grid point (1 min away, under
        // half an interval) and lands on 11:00. A countdown reading the interval
        // would hit zero at 10:59 and sit there for a minute.
        let timer = ReminderTimer::new(Duration::ZERO, sync_settings(20), ist(10, 39));
        assert_eq!(timer.work_deadline, WorkDeadline::Wall(ist(11, 0)));
        let snapshot = timer.tray_snapshot(Duration::from_secs(1200), ist(10, 59), 0, false);
        assert_eq!(snapshot.remaining_milliseconds, Some(60_000));
    }

    #[test]
    fn a_clock_jump_re_arms_on_the_next_grid_point_without_a_backlog() {
        let mut timer = ReminderTimer::new(Duration::ZERO, sync_settings(20), ist(10, 1));
        // Two hours pass on the wall clock. Rebase first, then evaluate.
        timer.rebase_work_deadline(ist(12, 1));
        assert_eq!(timer.tick(Duration::from_secs(60), ist(12, 1)), None);
        assert_eq!(timer.work_deadline, WorkDeadline::Wall(ist(12, 20)));
    }

    #[test]
    fn a_clock_jump_leaves_a_relative_timer_completely_unchanged() {
        // The regression guard for the promise that sync changes nothing by default.
        let mut timer = ReminderTimer::new(Duration::ZERO, ReminderSettings::default(), ist(10, 1));
        let before = timer.phase_started_at;
        let revision_before = timer.state_revision;
        timer.rebase_work_deadline(ist(12, 1));
        assert_eq!(timer.work_deadline, WorkDeadline::Relative);
        assert_eq!(timer.phase_started_at, before);
        assert_eq!(timer.state_revision, revision_before);
    }

    #[test]
    fn resuming_from_pause_in_sync_mode_rejoins_the_shared_grid_with_grace() {
        // m3: the tracked-doc claim ("with cross-device sync enabled it
        // instead rejoins the shared grid, skipping a grid point less than
        // half an interval away") had no test until now.
        let mut timer = ReminderTimer::new(Duration::ZERO, sync_settings(20), ist(10, 1));
        assert!(timer.pause(Duration::from_secs(60)));
        // 10:31 is nine minutes from the 10:40 grid point, under half the
        // twenty-minute interval, so resume skips it and rejoins at 11:00.
        assert!(timer.resume(Duration::from_secs(120), ist(10, 31)));
        assert_eq!(timer.work_deadline, WorkDeadline::Wall(ist(11, 0)));
    }

    #[test]
    fn natural_break_credit_applies_pending_settings_before_the_grid_deadline() {
        // m4: `settings_changed_during_a_break_apply_before_the_next_deadline_is_computed`
        // covers this ordering on the EndBreak path; `credit_natural_break` has
        // the same ordering comment but was untested until now.
        let mut timer = ReminderTimer::new(Duration::ZERO, sync_settings(20), ist(10, 1));
        timer.tick(Duration::from_secs(1140), ist(10, 20)).unwrap(); // StartBreak

        // Switch to a ten-minute rhythm mid-break, then credit a natural break
        // instead of waiting for the break timer to expire.
        timer.apply_settings(sync_settings(10), Duration::from_secs(1145), ist(10, 20));
        assert!(timer.credit_natural_break(
            Duration::from_secs(1146),
            ist(10, 20) + Duration::from_secs(20)
        ));
        // A ten-minute grid puts the next break at 10:30, not 10:40.
        assert_eq!(timer.work_deadline, WorkDeadline::Wall(ist(10, 30)));
    }
}
