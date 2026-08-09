use crate::{
    authorize_main_caller,
    overlay::{
        show_overlay, OverlayController, MAX_OVERLAY_DURATION_SECONDS, MIN_OVERLAY_DURATION_SECONDS,
    },
    probes::{ProbeCache, ProbeSnapshot},
};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    fs::{self, File, OpenOptions},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicU64, Ordering},
        Arc, Mutex, MutexGuard,
    },
    time::{Duration, Instant},
};
use tauri::{AppHandle, State, WebviewWindow};

const DEFAULT_WORK_MINUTES: u64 = 20;
const DEFAULT_BREAK_SECONDS: u64 = 20;
const MIN_WORK_MINUTES: u64 = 1;
const MAX_WORK_MINUTES: u64 = 120;
const REMINDER_POLL_INTERVAL: Duration = Duration::from_millis(250);
const SETTINGS_FILE_NAME: &str = "reminder-settings.json";
const SETTINGS_SCHEMA_VERSION: u32 = 1;

static SETTINGS_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ReminderSettings {
    work_minutes: u64,
    break_seconds: u64,
}

impl ReminderSettings {
    fn try_new(work_minutes: u64, break_seconds: u64) -> Result<Self, String> {
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

        Ok(Self {
            work_minutes,
            break_seconds,
        })
    }

    fn work_interval(self) -> Duration {
        Duration::from_secs(self.work_minutes * 60)
    }

    fn break_duration(self) -> Duration {
        Duration::from_secs(self.break_seconds)
    }
}

impl Default for ReminderSettings {
    fn default() -> Self {
        Self {
            work_minutes: DEFAULT_WORK_MINUTES,
            break_seconds: DEFAULT_BREAK_SECONDS,
        }
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub(crate) struct ReminderSettingsRequest {
    work_minutes: Value,
    break_seconds: Value,
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
        ReminderSettings::try_new(work_minutes, break_seconds)
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

#[derive(Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedReminderSettings {
    version: u32,
    work_minutes: u64,
    break_seconds: u64,
}

impl From<ReminderSettings> for PersistedReminderSettings {
    fn from(settings: ReminderSettings) -> Self {
        Self {
            version: SETTINGS_SCHEMA_VERSION,
            work_minutes: settings.work_minutes,
            break_seconds: settings.break_seconds,
        }
    }
}

impl PersistedReminderSettings {
    fn into_settings(self) -> Result<ReminderSettings, String> {
        if self.version != SETTINGS_SCHEMA_VERSION {
            return Err(format!(
                "unsupported reminder settings version {}",
                self.version
            ));
        }
        ReminderSettings::try_new(self.work_minutes, self.break_seconds)
    }
}

#[derive(Debug, Clone, Copy)]
struct ReminderSettingsSnapshot {
    settings: ReminderSettings,
    revision: u64,
    changed_at: Instant,
}

#[derive(Debug)]
struct ReminderSettingsRuntime {
    settings: ReminderSettings,
    revision: u64,
    changed_at: Instant,
}

#[derive(Debug)]
struct ReminderSettingsInner {
    path: PathBuf,
    runtime: Mutex<ReminderSettingsRuntime>,
}

#[derive(Debug, Clone)]
pub(crate) struct ReminderSettingsManager {
    inner: Arc<ReminderSettingsInner>,
}

impl ReminderSettingsManager {
    pub(crate) fn load(config_dir: &Path) -> io::Result<Self> {
        let path = config_dir.join(SETTINGS_FILE_NAME);
        let settings = load_or_repair_settings(&path)?;
        Ok(Self {
            inner: Arc::new(ReminderSettingsInner {
                path,
                runtime: Mutex::new(ReminderSettingsRuntime {
                    settings,
                    revision: 0,
                    changed_at: Instant::now(),
                }),
            }),
        })
    }

    fn runtime(&self) -> MutexGuard<'_, ReminderSettingsRuntime> {
        self.inner
            .runtime
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn current(&self) -> ReminderSettings {
        self.runtime().settings
    }

    fn snapshot(&self) -> ReminderSettingsSnapshot {
        let runtime = self.runtime();
        ReminderSettingsSnapshot {
            settings: runtime.settings,
            revision: runtime.revision,
            changed_at: runtime.changed_at,
        }
    }

    fn save(&self, settings: ReminderSettings) -> Result<ReminderSettings, String> {
        let mut runtime = self.runtime();
        persist_settings(&self.inner.path, settings)
            .map_err(|error| format!("could not save reminder settings: {error}"))?;
        runtime.settings = settings;
        runtime.revision = runtime.revision.wrapping_add(1);
        runtime.changed_at = Instant::now();
        Ok(settings)
    }

    fn reset(&self) -> Result<ReminderSettings, String> {
        self.save(ReminderSettings::default())
    }
}

fn load_or_repair_settings(path: &Path) -> io::Result<ReminderSettings> {
    match fs::read(path) {
        Ok(contents) => {
            let parsed = serde_json::from_slice::<PersistedReminderSettings>(&contents)
                .ok()
                .and_then(|persisted| persisted.into_settings().ok());
            if let Some(settings) = parsed {
                return Ok(settings);
            }

            let defaults = ReminderSettings::default();
            persist_settings(path, defaults)?;
            Ok(defaults)
        }
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            let defaults = ReminderSettings::default();
            persist_settings(path, defaults)?;
            Ok(defaults)
        }
        Err(error) => Err(error),
    }
}

fn create_settings_temp_file(path: &Path) -> io::Result<(PathBuf, File)> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "settings path has no parent")
    })?;
    let name = path
        .file_name()
        .map(|name| name.to_string_lossy())
        .unwrap_or_else(|| "settings".into());

    for _ in 0..100 {
        let id = SETTINGS_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
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
        "could not allocate a reminder settings temporary file",
    ))
}

#[cfg(not(target_os = "windows"))]
fn replace_settings_file(temp_path: &Path, path: &Path) -> io::Result<()> {
    fs::rename(temp_path, path)
}

#[cfg(target_os = "windows")]
fn replace_settings_file(temp_path: &Path, path: &Path) -> io::Result<()> {
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

fn persist_settings(path: &Path, settings: ReminderSettings) -> io::Result<()> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(io::ErrorKind::InvalidInput, "settings path has no parent")
    })?;
    fs::create_dir_all(parent)?;

    let serialized = serde_json::to_vec_pretty(&PersistedReminderSettings::from(settings))
        .map_err(io::Error::other)?;
    let (temp_path, mut temp_file) = create_settings_temp_file(path)?;
    let write_result = temp_file
        .write_all(&serialized)
        .and_then(|()| temp_file.write_all(b"\n"))
        .and_then(|()| temp_file.sync_all());
    drop(temp_file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }

    if let Err(error) = replace_settings_file(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

#[tauri::command]
pub(crate) fn get_reminder_settings(
    window: WebviewWindow,
    manager: State<'_, ReminderSettingsManager>,
) -> Result<ReminderSettings, String> {
    authorize_main_caller(window.label())?;
    Ok(manager.current())
}

#[tauri::command]
pub(crate) fn save_reminder_settings(
    window: WebviewWindow,
    manager: State<'_, ReminderSettingsManager>,
    settings: ReminderSettingsRequest,
) -> Result<ReminderSettings, String> {
    authorize_main_caller(window.label())?;
    manager.save(settings.into_settings()?)
}

#[tauri::command]
pub(crate) fn reset_reminder_settings(
    window: WebviewWindow,
    manager: State<'_, ReminderSettingsManager>,
) -> Result<ReminderSettings, String> {
    authorize_main_caller(window.label())?;
    manager.reset()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReminderPhase {
    Working,
    Break,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ReminderTransition {
    StartBreak,
    EndBreak,
}

/// The recurring reminder clock. Its only input is monotonic elapsed time;
/// probe results deliberately do not participate in phase advancement.
#[derive(Debug)]
struct ReminderTimer {
    phase: ReminderPhase,
    phase_started_at: Duration,
    settings: ReminderSettings,
    pending_settings: Option<ReminderSettings>,
}

impl ReminderTimer {
    fn new(now: Duration, settings: ReminderSettings) -> Self {
        Self {
            phase: ReminderPhase::Working,
            phase_started_at: now,
            settings,
            pending_settings: None,
        }
    }

    #[cfg(test)]
    fn with_defaults(now: Duration) -> Self {
        Self::new(now, ReminderSettings::default())
    }

    fn apply_settings(&mut self, settings: ReminderSettings, changed_at: Duration) {
        match self.phase {
            ReminderPhase::Working => {
                self.settings = settings;
                self.pending_settings = None;
                self.phase_started_at = changed_at;
            }
            ReminderPhase::Break => self.pending_settings = Some(settings),
        }
    }

    fn break_duration(&self) -> Duration {
        self.settings.break_duration()
    }

    fn tick(&mut self, now: Duration) -> Option<ReminderTransition> {
        let elapsed = now.saturating_sub(self.phase_started_at);
        let phase_duration = match self.phase {
            ReminderPhase::Working => self.settings.work_interval(),
            ReminderPhase::Break => self.settings.break_duration(),
        };

        if elapsed < phase_duration {
            return None;
        }

        // Anchor the next phase at this observation rather than replaying every
        // missed cycle after a long scheduler stall.
        self.phase_started_at = now;
        Some(match self.phase {
            ReminderPhase::Working => {
                self.phase = ReminderPhase::Break;
                ReminderTransition::StartBreak
            }
            ReminderPhase::Break => {
                self.phase = ReminderPhase::Working;
                if let Some(settings) = self.pending_settings.take() {
                    self.settings = settings;
                }
                ReminderTransition::EndBreak
            }
        })
    }
}

/// Probe data can suppress presentation of a due break, but it cannot mutate
/// the timer. Errors fail open so an unavailable probe never disables breaks.
fn should_present_break(probes: &ProbeSnapshot, break_duration: Duration) -> bool {
    let user_is_already_resting = probes
        .idle_seconds
        .as_ref()
        .is_ok_and(|seconds| *seconds >= break_duration.as_secs());
    let presentation_is_active = probes
        .active_window_fullscreen
        .as_ref()
        .is_ok_and(|fullscreen| *fullscreen);

    !user_is_already_resting && !presentation_is_active
}

pub(crate) fn start_scheduler(
    app: AppHandle,
    probe_cache: ProbeCache,
    overlay_controller: OverlayController,
    settings_manager: ReminderSettingsManager,
) -> io::Result<()> {
    std::thread::Builder::new()
        .name("unfocus-reminders".into())
        .spawn(move || {
            let initial = settings_manager.snapshot();
            let started_at = initial.changed_at;
            let mut settings_revision = initial.revision;
            let mut timer = ReminderTimer::new(Duration::ZERO, initial.settings);

            loop {
                std::thread::sleep(REMINDER_POLL_INTERVAL);

                let latest = settings_manager.snapshot();
                if latest.revision != settings_revision {
                    timer.apply_settings(
                        latest.settings,
                        latest.changed_at.saturating_duration_since(started_at),
                    );
                    settings_revision = latest.revision;
                }

                if timer.tick(started_at.elapsed()) != Some(ReminderTransition::StartBreak) {
                    continue;
                }

                let break_duration = timer.break_duration();
                let probes = probe_cache.snapshot();
                if !should_present_break(&probes, break_duration) {
                    if probes
                        .idle_seconds
                        .as_ref()
                        .is_ok_and(|seconds| *seconds >= break_duration.as_secs())
                    {
                        eprintln!("scheduled break stayed hidden because the user is already idle");
                    } else {
                        eprintln!("scheduled break stayed hidden while fullscreen is active");
                    }
                    continue;
                }

                if let Err(error) =
                    show_overlay(&app, &overlay_controller, break_duration.as_secs())
                {
                    eprintln!("could not present scheduled break: {error}");
                }
            }
        })?;
    Ok(())
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
        ReminderSettings::try_new(work_minutes, break_seconds).expect("valid test settings")
    }

    fn request(work_minutes: Value, break_seconds: Value) -> ReminderSettingsRequest {
        ReminderSettingsRequest {
            work_minutes,
            break_seconds,
        }
    }

    #[test]
    fn reminder_defaults_are_twenty_minutes_and_twenty_seconds() {
        let defaults = ReminderSettings::default();
        let mut timer = ReminderTimer::with_defaults(Duration::ZERO);

        assert_eq!(defaults, settings(20, 20));
        assert_eq!(
            timer.tick(defaults.work_interval() - Duration::from_millis(1)),
            None
        );
        assert_eq!(
            timer.tick(defaults.work_interval()),
            Some(ReminderTransition::StartBreak)
        );
        assert_eq!(
            timer.tick(defaults.work_interval() + defaults.break_duration()),
            Some(ReminderTransition::EndBreak)
        );
        assert_eq!(
            timer.tick(
                defaults.work_interval() + defaults.break_duration() + defaults.work_interval()
            ),
            Some(ReminderTransition::StartBreak)
        );
    }

    #[test]
    fn settings_ranges_are_validated_at_the_rust_boundary() {
        assert_eq!(settings(1, 3), ReminderSettings::try_new(1, 3).unwrap());
        assert_eq!(
            settings(120, 30),
            ReminderSettings::try_new(120, 30).unwrap()
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
        manager.save(settings(45, 12)).unwrap();
        drop(manager);

        let reloaded = ReminderSettingsManager::load(&directory.path).unwrap();
        assert_eq!(reloaded.current(), settings(45, 12));
        assert_eq!(reloaded.reset().unwrap(), ReminderSettings::default());
        drop(reloaded);
        assert_eq!(
            ReminderSettingsManager::load(&directory.path)
                .unwrap()
                .current(),
            ReminderSettings::default()
        );
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
    }

    #[test]
    fn malformed_and_out_of_range_settings_are_replaced_with_defaults() {
        let directory = TestDirectory::new();
        let path = directory.settings_path();

        for invalid in [
            "{",
            r#"{"version":1,"workMinutes":0,"breakSeconds":20}"#,
            r#"{"version":1,"workMinutes":20,"breakSeconds":31}"#,
            r#"{"version":2,"workMinutes":20,"breakSeconds":20}"#,
            r#"{"version":1,"workMinutes":20,"breakSeconds":20,"extra":true}"#,
        ] {
            fs::write(&path, invalid).unwrap();
            let manager = ReminderSettingsManager::load(&directory.path).unwrap();
            assert_eq!(manager.current(), ReminderSettings::default());

            let repaired: PersistedReminderSettings =
                serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
            assert_eq!(repaired.version, SETTINGS_SCHEMA_VERSION);
            assert_eq!(
                repaired.into_settings().unwrap(),
                ReminderSettings::default()
            );
        }
    }

    #[test]
    fn saving_during_work_restarts_the_countdown_at_the_save_time() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(20, 20));

        assert_eq!(timer.tick(Duration::from_secs(10 * 60)), None);
        timer.apply_settings(settings(1, 8), Duration::from_secs(10 * 60));
        assert_eq!(timer.tick(Duration::from_secs(10 * 60 + 59)), None);
        assert_eq!(
            timer.tick(Duration::from_secs(11 * 60)),
            Some(ReminderTransition::StartBreak)
        );
        assert_eq!(timer.break_duration(), Duration::from_secs(8));
    }

    #[test]
    fn saving_during_a_break_preserves_that_break_and_updates_the_next_work_phase() {
        let mut timer = ReminderTimer::new(Duration::ZERO, settings(1, 3));

        assert_eq!(
            timer.tick(Duration::from_secs(60)),
            Some(ReminderTransition::StartBreak)
        );
        timer.apply_settings(settings(2, 30), Duration::from_secs(61));
        assert_eq!(timer.break_duration(), Duration::from_secs(3));
        assert_eq!(timer.tick(Duration::from_secs(62)), None);
        assert_eq!(
            timer.tick(Duration::from_secs(63)),
            Some(ReminderTransition::EndBreak)
        );
        assert_eq!(timer.tick(Duration::from_secs(182)), None);
        assert_eq!(
            timer.tick(Duration::from_secs(183)),
            Some(ReminderTransition::StartBreak)
        );
        assert_eq!(timer.break_duration(), Duration::from_secs(30));
    }

    #[test]
    fn reminder_clock_is_injected_and_does_not_replay_missed_cycles() {
        let mut timer = ReminderTimer::new(Duration::from_secs(10), settings(1, 5));

        assert_eq!(timer.tick(Duration::from_secs(69)), None);
        assert_eq!(
            timer.tick(Duration::from_secs(600)),
            Some(ReminderTransition::StartBreak)
        );
        assert_eq!(timer.tick(Duration::from_secs(600)), None);
        assert_eq!(
            timer.tick(Duration::from_secs(605)),
            Some(ReminderTransition::EndBreak)
        );
    }

    #[test]
    fn a_clock_regression_does_not_advance_the_reminder() {
        let mut timer = ReminderTimer::new(Duration::from_secs(100), settings(1, 5));

        assert_eq!(timer.tick(Duration::from_secs(90)), None);
        assert_eq!(timer.phase, ReminderPhase::Working);
    }

    #[test]
    fn configured_break_duration_controls_idle_suppression() {
        let idle = ProbeSnapshot {
            idle_seconds: Ok(8),
            active_window_fullscreen: Ok(false),
        };

        assert!(!should_present_break(&idle, Duration::from_secs(8)));
        assert!(should_present_break(&idle, Duration::from_secs(20)));
    }

    #[test]
    fn probes_only_control_break_presentation() {
        let break_duration = ReminderSettings::default().break_duration();
        let active = ProbeSnapshot {
            idle_seconds: Ok(0),
            active_window_fullscreen: Ok(false),
        };
        let idle = ProbeSnapshot {
            idle_seconds: Ok(break_duration.as_secs()),
            active_window_fullscreen: Ok(false),
        };
        let fullscreen = ProbeSnapshot {
            idle_seconds: Ok(0),
            active_window_fullscreen: Ok(true),
        };
        let failed = ProbeSnapshot {
            idle_seconds: Err("idle failed".into()),
            active_window_fullscreen: Err("fullscreen failed".into()),
        };

        assert!(should_present_break(&active, break_duration));
        assert!(!should_present_break(&idle, break_duration));
        assert!(!should_present_break(&fullscreen, break_duration));
        assert!(should_present_break(&failed, break_duration));

        // Timer advancement has no probe input and is identical whether the
        // presentation decision above succeeds, suppresses, or errors.
        for probes in [&active, &idle, &fullscreen, &failed] {
            let mut timer = ReminderTimer::new(Duration::ZERO, settings(1, 3));
            let _ = should_present_break(probes, Duration::from_secs(3));
            assert_eq!(
                timer.tick(Duration::from_secs(60)),
                Some(ReminderTransition::StartBreak)
            );
            assert_eq!(
                timer.tick(Duration::from_secs(63)),
                Some(ReminderTransition::EndBreak)
            );
        }
    }
}
