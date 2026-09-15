use crate::storage_recovery::{
    create_new_file_with_permissions, existing_file_permissions, replace_file_atomically,
};
use serde::{Deserialize, Serialize};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};
use tauri::Manager;

pub(crate) const AUTOSTART_ARGUMENT: &str = "--unfocus-autostart";
pub(crate) const AUTOSTART_STATE_FILE_NAME: &str = "autostart-state.json";
const AUTOSTART_STATE_VERSION: u8 = 1;
#[cfg(target_os = "windows")]
const WINDOWS_AUTOSTART_NAME: &str = "Unfocus";
#[cfg(any(target_os = "macos", test))]
const MACOS_LAUNCH_AGENT_LABEL: &str = "com.unfocus.desktop";
#[cfg(target_os = "macos")]
const MACOS_BUNDLE_IDENTIFIER: &str = "com.unfocus.desktop";

static AUTOSTART_TEMP_FILE_ID: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
static TEST_PERSIST_FAILURES: std::sync::Mutex<Vec<PathBuf>> = std::sync::Mutex::new(Vec::new());

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum AutostartStatus {
    Enabled,
    Disabled,
    SkippedDevelopment,
    Failed,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct AutostartDiagnostics {
    pub(crate) status: AutostartStatus,
    pub(crate) error: Option<String>,
}

impl AutostartDiagnostics {
    const fn enabled() -> Self {
        Self {
            status: AutostartStatus::Enabled,
            error: None,
        }
    }

    const fn disabled() -> Self {
        Self {
            status: AutostartStatus::Disabled,
            error: None,
        }
    }

    const fn skipped_development() -> Self {
        Self {
            status: AutostartStatus::SkippedDevelopment,
            error: None,
        }
    }

    fn failed(error: impl std::fmt::Display) -> Self {
        Self {
            status: AutostartStatus::Failed,
            error: Some(format!("Launch at login registration failed: {error}")),
        }
    }
}

#[derive(Debug, Clone)]
pub(crate) struct AutostartRuntime {
    diagnostics: AutostartDiagnostics,
}

impl AutostartRuntime {
    pub(crate) fn initialize(app: &tauri::App, config_dir: &Path) -> Self {
        let diagnostics = initialize_for_build(cfg!(debug_assertions), || {
            initialize_release(app, config_dir)
        });
        if let Some(error) = &diagnostics.error {
            eprintln!("{error}");
        }
        Self { diagnostics }
    }

    pub(crate) fn diagnostics(&self) -> AutostartDiagnostics {
        self.diagnostics.clone()
    }
}

fn initialize_for_build(
    is_development_build: bool,
    initialize_release_build: impl FnOnce() -> AutostartDiagnostics,
) -> AutostartDiagnostics {
    if is_development_build {
        AutostartDiagnostics::skipped_development()
    } else {
        initialize_release_build()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "lowercase")]
enum RegistrationState {
    Pending,
    Complete,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct PersistedAutostartState {
    version: u8,
    registration: RegistrationState,
    executable_path: String,
}

impl PersistedAutostartState {
    fn pending(executable_path: &str) -> Self {
        Self {
            version: AUTOSTART_STATE_VERSION,
            registration: RegistrationState::Pending,
            executable_path: executable_path.to_owned(),
        }
    }

    fn complete(executable_path: &str) -> Self {
        Self {
            version: AUTOSTART_STATE_VERSION,
            registration: RegistrationState::Complete,
            executable_path: executable_path.to_owned(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ExistingRegistrationAction {
    PromotePending,
    Refresh,
    KeepEnabled,
    KeepDisabled,
    FailPendingRecovery,
}

fn existing_registration_action(
    state: &PersistedAutostartState,
    registration_enabled: bool,
    executable_path: &str,
) -> ExistingRegistrationAction {
    if !registration_enabled {
        return match state.registration {
            RegistrationState::Pending => ExistingRegistrationAction::FailPendingRecovery,
            RegistrationState::Complete => ExistingRegistrationAction::KeepDisabled,
        };
    }

    match (state.registration, state.executable_path == executable_path) {
        (RegistrationState::Pending, true) => ExistingRegistrationAction::PromotePending,
        (RegistrationState::Complete, true) => ExistingRegistrationAction::KeepEnabled,
        (_, false) => ExistingRegistrationAction::Refresh,
    }
}

fn state_file_path(config_dir: &Path) -> PathBuf {
    config_dir.join(AUTOSTART_STATE_FILE_NAME)
}

fn load_state(path: &Path) -> Result<Option<PersistedAutostartState>, String> {
    let contents = match fs::read(path) {
        Ok(contents) => contents,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(format!("could not read {}: {error}", path.display())),
    };
    let state: PersistedAutostartState = serde_json::from_slice(&contents)
        .map_err(|error| format!("could not parse {}: {error}", path.display()))?;
    if state.version != AUTOSTART_STATE_VERSION {
        return Err(format!(
            "{} has unsupported schema version {}",
            path.display(),
            state.version
        ));
    }
    if state.executable_path.is_empty() {
        return Err(format!("{} has an empty executable path", path.display()));
    }
    Ok(Some(state))
}

fn create_state_temp_file(path: &Path) -> io::Result<(PathBuf, File)> {
    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "autostart state path has no parent directory",
        )
    })?;
    let file_name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or(AUTOSTART_STATE_FILE_NAME);
    let permissions = existing_file_permissions(path)?;

    for _ in 0..100 {
        let id = AUTOSTART_TEMP_FILE_ID.fetch_add(1, Ordering::Relaxed);
        let temp_path = parent.join(format!(".{file_name}.{}.{id}.tmp", std::process::id()));
        match create_new_file_with_permissions(&temp_path, permissions.as_ref()) {
            Ok(file) => return Ok((temp_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate an autostart state temporary file",
    ))
}

fn persist_state(path: &Path, state: &PersistedAutostartState) -> io::Result<()> {
    #[cfg(test)]
    if TEST_PERSIST_FAILURES.lock().is_ok_and(|mut targets| {
        targets
            .iter()
            .position(|target| target == path)
            .map(|index| targets.remove(index))
            .is_some()
    }) {
        return Err(io::Error::other(
            "injected autostart state persistence failure",
        ));
    }

    let parent = path.parent().ok_or_else(|| {
        io::Error::new(
            io::ErrorKind::InvalidInput,
            "autostart state path has no parent directory",
        )
    })?;
    fs::create_dir_all(parent)?;
    let serialized = serde_json::to_vec_pretty(state).map_err(io::Error::other)?;
    let (temp_path, mut temp_file) = create_state_temp_file(path)?;
    let write_result = temp_file
        .write_all(&serialized)
        .and_then(|()| temp_file.write_all(b"\n"))
        .and_then(|()| temp_file.sync_all());
    drop(temp_file);
    if let Err(error) = write_result {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    if let Err(error) = replace_file_atomically(&temp_path, path) {
        let _ = fs::remove_file(&temp_path);
        return Err(error);
    }
    Ok(())
}

#[cfg(test)]
fn inject_persist_failure(path: PathBuf) {
    TEST_PERSIST_FAILURES
        .lock()
        .expect("autostart persistence failure hook lock")
        .push(path);
}

fn is_autostart_argument(argument: &OsStr) -> bool {
    argument == OsStr::new(AUTOSTART_ARGUMENT)
}

pub(crate) fn primary_launch_is_autostart() -> bool {
    std::env::args_os()
        .skip(1)
        .any(|argument| is_autostart_argument(&argument))
}

pub(crate) fn secondary_launch_is_autostart(arguments: &[String]) -> bool {
    arguments
        .iter()
        .any(|argument| is_autostart_argument(OsStr::new(argument)))
}

trait RegistrationBackend {
    fn enable(&self) -> Result<(), String>;
    fn is_effectively_enabled(&self) -> Result<bool, String>;
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
impl RegistrationBackend for auto_launch::AutoLaunch {
    fn enable(&self) -> Result<(), String> {
        self.enable()
            .map_err(|error| format!("could not enable the OS entry: {error}"))
    }

    fn is_effectively_enabled(&self) -> Result<bool, String> {
        registration_is_enabled(self)
    }
}

#[cfg(target_os = "linux")]
struct LinuxRegistration {
    path: PathBuf,
    entry: String,
}

#[cfg(target_os = "linux")]
impl RegistrationBackend for LinuxRegistration {
    fn enable(&self) -> Result<(), String> {
        let parent = self.path.parent().ok_or("autostart entry has no parent")?;
        fs::create_dir_all(parent)
            .and_then(|()| fs::write(&self.path, &self.entry))
            .map_err(|error| format!("could not write {}: {error}", self.path.display()))
    }

    fn is_effectively_enabled(&self) -> Result<bool, String> {
        let entry = match fs::read_to_string(&self.path) {
            Ok(entry) => entry,
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(format!("could not read {}: {error}", self.path.display())),
        };
        Ok(!desktop_entry_is_disabled(&entry))
    }
}

#[cfg(target_os = "linux")]
fn linux_registration_path(
    xdg_config_home: Option<&OsStr>,
    home: Option<&OsStr>,
) -> Result<PathBuf, String> {
    let config = xdg_config_home
        .map(Path::new)
        .filter(|path| path.is_absolute())
        .map(Path::to_path_buf)
        .or_else(|| {
            home.map(Path::new)
                .filter(|path| path.is_absolute())
                .map(|path| path.join(".config"))
        })
        .ok_or("no absolute XDG_CONFIG_HOME or HOME is available")?;
    Ok(config.join("autostart/Unfocus.desktop"))
}

#[cfg(target_os = "linux")]
fn build_launcher(executable_path: &str) -> Result<LinuxRegistration, String> {
    let path = linux_registration_path(
        std::env::var_os("XDG_CONFIG_HOME").as_deref(),
        std::env::var_os("HOME").as_deref(),
    )?;
    let executable = desktop_entry_executable_path(executable_path)?;
    Ok(LinuxRegistration {
        path,
        entry: format!(
            "[Desktop Entry]\nType=Application\nVersion=1.0\nName=Unfocus\n\
             Exec={executable} {AUTOSTART_ARGUMENT}\nStartupNotify=false\nTerminal=false\n"
        ),
    })
}

fn initialize_registration<L, E>(
    config_dir: &Path,
    executable_path: &str,
    launcher: Result<&L, E>,
) -> AutostartDiagnostics
where
    L: RegistrationBackend,
    E: std::fmt::Display,
{
    let launcher = match launcher {
        Ok(launcher) => launcher,
        Err(error) => return AutostartDiagnostics::failed(error),
    };
    let path = state_file_path(config_dir);
    let state = match load_state(&path) {
        Ok(state) => state,
        Err(error) => return AutostartDiagnostics::failed(error),
    };

    let Some(state) = state else {
        let pending = PersistedAutostartState::pending(executable_path);
        if let Err(error) = persist_state(&path, &pending) {
            return AutostartDiagnostics::failed(format!(
                "could not persist initial enrollment state at {}: {error}",
                path.display()
            ));
        }
        return enable_and_complete(launcher, &path, executable_path);
    };

    let enabled = match launcher.is_effectively_enabled() {
        Ok(enabled) => enabled,
        Err(error) => return AutostartDiagnostics::failed(error),
    };
    match existing_registration_action(&state, enabled, executable_path) {
        ExistingRegistrationAction::KeepDisabled => AutostartDiagnostics::disabled(),
        ExistingRegistrationAction::FailPendingRecovery => AutostartDiagnostics::failed(format!(
            "incomplete enrollment at {} has no enabled OS entry; delete {} to retry",
            path.display(),
            AUTOSTART_STATE_FILE_NAME
        )),
        ExistingRegistrationAction::KeepEnabled => AutostartDiagnostics::enabled(),
        ExistingRegistrationAction::PromotePending => {
            let complete = PersistedAutostartState::complete(executable_path);
            match persist_state(&path, &complete) {
                Ok(()) => AutostartDiagnostics::enabled(),
                Err(error) => AutostartDiagnostics::failed(format!(
                    "could not mark existing enrollment complete at {}: {error}",
                    path.display()
                )),
            }
        }
        ExistingRegistrationAction::Refresh => {
            enable_and_complete(launcher, &path, executable_path)
        }
    }
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn initialize_release(app: &tauri::App, config_dir: &Path) -> AutostartDiagnostics {
    let executable = match executable_path(app) {
        Ok(path) => path,
        Err(error) => return AutostartDiagnostics::failed(error),
    };
    let executable = match executable.to_str() {
        Some(path) if !path.is_empty() => path,
        _ => {
            return AutostartDiagnostics::failed(
                "the application executable path is not valid Unicode",
            );
        }
    };
    let launcher = build_launcher(executable);
    initialize_registration(config_dir, executable, launcher.as_ref())
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
fn initialize_release(_app: &tauri::App, _config_dir: &Path) -> AutostartDiagnostics {
    AutostartDiagnostics::failed("launch at login is unsupported on this platform")
}

#[cfg(any(target_os = "linux", target_os = "macos", target_os = "windows"))]
fn executable_path(app: &tauri::App) -> Result<PathBuf, String> {
    let path = tauri::process::current_binary(&app.env())
        .map_err(|error| format!("could not resolve the application executable: {error}"))?;
    if !path.is_absolute() {
        return Err(format!(
            "the application executable path is not absolute: {}",
            path.display()
        ));
    }
    Ok(path)
}

fn enable_and_complete(
    launcher: &impl RegistrationBackend,
    state_path: &Path,
    executable_path: &str,
) -> AutostartDiagnostics {
    if let Err(error) = launcher.enable() {
        return AutostartDiagnostics::failed(error);
    }
    match launcher.is_effectively_enabled() {
        Ok(true) => {}
        Ok(false) => {
            return AutostartDiagnostics::failed(
                "the OS entry was not enabled after registration completed",
            );
        }
        Err(error) => return AutostartDiagnostics::failed(error),
    }

    let complete = PersistedAutostartState::complete(executable_path);
    match persist_state(state_path, &complete) {
        Ok(()) => AutostartDiagnostics::enabled(),
        Err(error) => AutostartDiagnostics::failed(format!(
            "could not mark enrollment complete at {}: {error}",
            state_path.display()
        )),
    }
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn build_launcher(executable_path: &str) -> Result<auto_launch::AutoLaunch, String> {
    use auto_launch::AutoLaunchBuilder;

    let mut builder = AutoLaunchBuilder::new();
    builder.set_args(&[AUTOSTART_ARGUMENT]);

    #[cfg(target_os = "macos")]
    {
        use auto_launch::MacOSLaunchMode;

        if executable_path.chars().any(is_xml_metacharacter) {
            return Err("the macOS executable path contains unsupported XML characters".into());
        }
        builder
            .set_app_name(MACOS_LAUNCH_AGENT_LABEL)
            .set_app_path(executable_path)
            .set_macos_launch_mode(MacOSLaunchMode::LaunchAgent)
            .set_bundle_identifiers(&[MACOS_BUNDLE_IDENTIFIER]);
    }
    #[cfg(target_os = "windows")]
    {
        use auto_launch::WindowsEnableMode;

        builder
            .set_app_name(WINDOWS_AUTOSTART_NAME)
            .set_app_path(&windows_command_line_quote(executable_path)?)
            .set_windows_enable_mode(WindowsEnableMode::CurrentUser);
    }

    builder
        .build()
        .map_err(|error| format!("could not construct the OS entry: {error}"))
}

#[cfg(any(target_os = "macos", target_os = "windows"))]
fn registration_is_enabled(launcher: &auto_launch::AutoLaunch) -> Result<bool, String> {
    let enabled = launcher
        .is_enabled()
        .map_err(|error| format!("could not inspect the OS entry: {error}"))?;
    if !enabled {
        return Ok(false);
    }

    #[cfg(target_os = "macos")]
    if macos_launch_agent_disabled()? {
        return Ok(false);
    }

    Ok(true)
}

#[cfg(any(target_os = "linux", target_os = "windows", test))]
fn reject_line_breaks(value: &str, platform: &str) -> Result<(), String> {
    if value.contains(['\r', '\n']) {
        return Err(format!(
            "the {platform} executable path contains a line break"
        ));
    }
    Ok(())
}

#[cfg(any(target_os = "linux", test))]
fn desktop_entry_executable_path(executable_path: &str) -> Result<String, String> {
    reject_line_breaks(executable_path, "Linux")?;
    let mut escaped = String::with_capacity(executable_path.len() + 2);
    escaped.push('"');
    for character in executable_path.chars() {
        match character {
            '\\' => escaped.push_str(r"\\\\"),
            '"' | '`' | '$' => {
                escaped.push_str(r"\\");
                escaped.push(character);
            }
            '%' => escaped.push_str("%%"),
            _ => escaped.push(character),
        }
    }
    escaped.push('"');
    Ok(escaped)
}

#[cfg(any(target_os = "windows", test))]
fn windows_command_line_quote(executable_path: &str) -> Result<String, String> {
    reject_line_breaks(executable_path, "Windows")?;
    let mut quoted = String::with_capacity(executable_path.len() + 2);
    quoted.push('"');
    let mut backslash_count = 0;
    for character in executable_path.chars() {
        match character {
            '\\' => backslash_count += 1,
            '"' => {
                quoted.extend(std::iter::repeat_n('\\', backslash_count * 2 + 1));
                quoted.push('"');
                backslash_count = 0;
            }
            _ => {
                quoted.extend(std::iter::repeat_n('\\', backslash_count));
                quoted.push(character);
                backslash_count = 0;
            }
        }
    }
    quoted.extend(std::iter::repeat_n('\\', backslash_count * 2));
    quoted.push('"');
    Ok(quoted)
}

#[cfg(any(target_os = "macos", test))]
fn is_xml_metacharacter(character: char) -> bool {
    matches!(character, '&' | '<' | '>' | '"' | '\'')
}

#[cfg(target_os = "macos")]
fn home_directory() -> Result<PathBuf, String> {
    std::env::var_os("HOME")
        .filter(|path| !path.is_empty())
        .map(PathBuf::from)
        .ok_or_else(|| "the home directory is unavailable".into())
}

#[cfg(any(target_os = "linux", test))]
fn desktop_entry_is_disabled(entry: &str) -> bool {
    let mut desktop_section = false;
    for line in entry.lines() {
        let line = line.trim();
        if line.starts_with('[') && line.ends_with(']') {
            desktop_section = line == "[Desktop Entry]";
            continue;
        }
        if !desktop_section {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        if (key == "Hidden" && value.eq_ignore_ascii_case("true"))
            || (key == "X-GNOME-Autostart-enabled" && value.eq_ignore_ascii_case("false"))
        {
            return true;
        }
    }
    false
}

#[cfg(any(target_os = "macos", test))]
fn launchctl_output_disables_label(output: &str, label: &str) -> bool {
    output.lines().any(|line| {
        line.split_once("=>").is_some_and(|(service, value)| {
            service.trim().trim_matches('"') == label && value.trim_start().starts_with("true")
        })
    })
}

#[cfg(target_os = "macos")]
fn macos_launch_agent_disabled() -> Result<bool, String> {
    use std::{os::unix::fs::MetadataExt, process::Command};

    let home = home_directory()?;
    let uid = fs::metadata(&home)
        .map_err(|error| format!("could not inspect {}: {error}", home.display()))?
        .uid();
    let output = Command::new("launchctl")
        .args(["print-disabled", &format!("gui/{uid}")])
        .output()
        .map_err(|error| format!("could not inspect launchctl disabled state: {error}"))?;
    if !output.status.success() {
        return Err(format!(
            "launchctl could not inspect disabled state: {}",
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    let output = String::from_utf8(output.stdout)
        .map_err(|error| format!("launchctl returned non-UTF-8 disabled state: {error}"))?;
    Ok(launchctl_output_disables_label(
        &output,
        MACOS_LAUNCH_AGENT_LABEL,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::{Cell, RefCell},
        collections::VecDeque,
        rc::Rc,
        sync::atomic::{AtomicU64, Ordering},
    };

    static TEST_DIRECTORY_ID: AtomicU64 = AtomicU64::new(0);

    struct TestDirectory {
        path: PathBuf,
    }

    impl TestDirectory {
        fn new() -> Self {
            for _ in 0..100 {
                let id = TEST_DIRECTORY_ID.fetch_add(1, Ordering::Relaxed);
                let path = std::env::temp_dir().join(format!(
                    "unfocus-autostart-tests-{}-{id}",
                    std::process::id()
                ));
                match fs::create_dir(&path) {
                    Ok(()) => return Self { path },
                    Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
                    Err(error) => panic!("test directory should be created: {error}"),
                }
            }
            panic!("could not allocate an autostart test directory")
        }
    }

    impl Drop for TestDirectory {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.path);
        }
    }

    struct FakeRegistration {
        checks: RefCell<VecDeque<Result<bool, String>>>,
        check_calls: Cell<usize>,
        enable_calls: Cell<usize>,
        enable_error: Option<String>,
        before_enable: RefCell<Option<Box<dyn FnOnce()>>>,
    }

    impl FakeRegistration {
        fn new(
            checks: impl IntoIterator<Item = Result<bool, &'static str>>,
            enable_error: Option<&str>,
        ) -> Self {
            Self {
                checks: RefCell::new(
                    checks
                        .into_iter()
                        .map(|result| result.map_err(str::to_owned))
                        .collect(),
                ),
                check_calls: Cell::new(0),
                enable_calls: Cell::new(0),
                enable_error: enable_error.map(str::to_owned),
                before_enable: RefCell::new(None),
            }
        }

        fn observe_state_before_enable(self, observer: impl FnOnce() + 'static) -> Self {
            self.before_enable.replace(Some(Box::new(observer)));
            self
        }
    }

    impl RegistrationBackend for FakeRegistration {
        fn enable(&self) -> Result<(), String> {
            self.enable_calls.set(self.enable_calls.get() + 1);
            if let Some(observer) = self.before_enable.borrow_mut().take() {
                observer();
            }
            self.enable_error.clone().map_or(Ok(()), Err)
        }

        fn is_effectively_enabled(&self) -> Result<bool, String> {
            self.check_calls.set(self.check_calls.get() + 1);
            self.checks
                .borrow_mut()
                .pop_front()
                .unwrap_or_else(|| Err("unexpected registration check".into()))
        }
    }

    fn initialize_with_fake(
        config_dir: &Path,
        executable_path: &str,
        registration: &FakeRegistration,
    ) -> AutostartDiagnostics {
        initialize_registration(
            config_dir,
            executable_path,
            Ok::<&FakeRegistration, &str>(registration),
        )
    }

    fn assert_failed(diagnostics: AutostartDiagnostics) {
        assert_eq!(diagnostics.status, AutostartStatus::Failed);
        assert!(diagnostics
            .error
            .as_deref()
            .is_some_and(|error| error.starts_with("Launch at login registration failed:")));
    }

    fn state(registration: RegistrationState, executable_path: &str) -> PersistedAutostartState {
        PersistedAutostartState {
            version: AUTOSTART_STATE_VERSION,
            registration,
            executable_path: executable_path.into(),
        }
    }

    #[test]
    fn launch_markers_require_an_exact_argument() {
        assert!(is_autostart_argument(OsStr::new(AUTOSTART_ARGUMENT)));
        assert!(!is_autostart_argument(OsStr::new(
            "--unfocus-autostart-now"
        )));
        assert!(!is_autostart_argument(OsStr::new("--Unfocus-autostart")));
        assert!(secondary_launch_is_autostart(&[
            "unfocus".into(),
            AUTOSTART_ARGUMENT.into()
        ]));
    }

    #[test]
    fn missing_state_enrolls_before_enabling_and_verifies_before_completion() {
        let root = TestDirectory::new();
        let path = state_file_path(&root.path);
        let observed_pending = Rc::new(RefCell::new(None));
        let observed_state = Rc::clone(&observed_pending);
        let state_to_observe = path.clone();
        let registration =
            FakeRegistration::new([Ok(true)], None).observe_state_before_enable(move || {
                *observed_state.borrow_mut() =
                    load_state(&state_to_observe).expect("pending state should load");
            });

        let diagnostics = initialize_with_fake(&root.path, "/opt/Unfocus", &registration);

        assert_eq!(diagnostics.status, AutostartStatus::Enabled);
        assert_eq!(
            observed_pending.borrow().clone(),
            Some(state(RegistrationState::Pending, "/opt/Unfocus"))
        );
        assert_eq!(registration.enable_calls.get(), 1);
        assert_eq!(registration.check_calls.get(), 1);
        assert_eq!(
            load_state(&path).expect("state should load"),
            Some(state(RegistrationState::Complete, "/opt/Unfocus"))
        );
    }

    #[test]
    fn pending_enabled_same_path_promotes_without_enabling_again() {
        let root = TestDirectory::new();
        let path = state_file_path(&root.path);
        persist_state(&path, &state(RegistrationState::Pending, "/opt/Unfocus"))
            .expect("pending state should persist");
        let registration = FakeRegistration::new([Ok(true)], None);

        let diagnostics = initialize_with_fake(&root.path, "/opt/Unfocus", &registration);

        assert_eq!(diagnostics.status, AutostartStatus::Enabled);
        assert_eq!(registration.enable_calls.get(), 0);
        assert_eq!(registration.check_calls.get(), 1);
        assert_eq!(
            load_state(&path).expect("state should load"),
            Some(state(RegistrationState::Complete, "/opt/Unfocus"))
        );
    }

    #[test]
    fn pending_enabled_old_path_refreshes_after_effective_state_check() {
        let root = TestDirectory::new();
        let path = state_file_path(&root.path);
        persist_state(
            &path,
            &state(RegistrationState::Pending, "/opt/old-unfocus"),
        )
        .expect("pending state should persist");
        let registration = FakeRegistration::new([Ok(true), Ok(true)], None);

        let diagnostics = initialize_with_fake(&root.path, "/opt/new-unfocus", &registration);

        assert_eq!(diagnostics.status, AutostartStatus::Enabled);
        assert_eq!(registration.enable_calls.get(), 1);
        assert_eq!(registration.check_calls.get(), 2);
        assert_eq!(
            load_state(&path).expect("state should load"),
            Some(state(RegistrationState::Complete, "/opt/new-unfocus"))
        );
    }

    #[test]
    fn pending_disabled_registration_fails_closed_without_mutation() {
        let root = TestDirectory::new();
        let path = state_file_path(&root.path);
        persist_state(&path, &state(RegistrationState::Pending, "/opt/Unfocus"))
            .expect("pending state should persist");
        let original = fs::read(&path).expect("pending state bytes should load");
        let registration = FakeRegistration::new([Ok(false)], None);

        assert_failed(initialize_with_fake(
            &root.path,
            "/opt/Unfocus",
            &registration,
        ));

        assert_eq!(registration.enable_calls.get(), 0);
        assert_eq!(registration.check_calls.get(), 1);
        assert_eq!(
            fs::read(&path).expect("pending state bytes should remain"),
            original
        );
    }

    #[test]
    fn completed_disabled_registration_remains_an_intentional_opt_out() {
        let root = TestDirectory::new();
        let path = state_file_path(&root.path);
        persist_state(&path, &state(RegistrationState::Complete, "/opt/Unfocus"))
            .expect("complete state should persist");
        let original = fs::read(&path).expect("complete state bytes should load");
        let registration = FakeRegistration::new([Ok(false)], None);

        let diagnostics = initialize_with_fake(&root.path, "/opt/Unfocus", &registration);

        assert_eq!(diagnostics.status, AutostartStatus::Disabled);
        assert_eq!(diagnostics.error, None);
        assert_eq!(registration.enable_calls.get(), 0);
        assert_eq!(
            fs::read(&path).expect("complete state bytes should remain"),
            original
        );
    }

    #[test]
    fn completed_enabled_same_path_is_a_no_op() {
        let root = TestDirectory::new();
        let path = state_file_path(&root.path);
        persist_state(&path, &state(RegistrationState::Complete, "/opt/Unfocus"))
            .expect("complete state should persist");
        let registration = FakeRegistration::new([Ok(true)], None);

        let diagnostics = initialize_with_fake(&root.path, "/opt/Unfocus", &registration);

        assert_eq!(diagnostics.status, AutostartStatus::Enabled);
        assert_eq!(registration.enable_calls.get(), 0);
        assert_eq!(registration.check_calls.get(), 1);
    }

    #[test]
    fn completed_enabled_old_path_refreshes_and_updates_the_saved_path() {
        let root = TestDirectory::new();
        let path = state_file_path(&root.path);
        persist_state(
            &path,
            &state(RegistrationState::Complete, "/opt/old-unfocus"),
        )
        .expect("complete state should persist");
        let registration = FakeRegistration::new([Ok(true), Ok(true)], None);

        let diagnostics = initialize_with_fake(&root.path, "/opt/new-unfocus", &registration);

        assert_eq!(diagnostics.status, AutostartStatus::Enabled);
        assert_eq!(registration.enable_calls.get(), 1);
        assert_eq!(registration.check_calls.get(), 2);
        assert_eq!(
            load_state(&path).expect("state should load"),
            Some(state(RegistrationState::Complete, "/opt/new-unfocus"))
        );
    }

    #[test]
    fn construction_failure_is_nonfatal_and_does_not_write_state() {
        let root = TestDirectory::new();

        assert_failed(initialize_registration(
            &root.path,
            "/opt/Unfocus",
            Err::<&FakeRegistration, _>("launcher construction failed"),
        ));

        assert!(!state_file_path(&root.path).exists());
    }

    #[test]
    fn malformed_state_is_nonfatal_and_does_not_touch_registration() {
        let root = TestDirectory::new();
        let path = state_file_path(&root.path);
        let original =
            br#"{"version":2,"registration":"complete","executablePath":"/opt/Unfocus"}"#;
        fs::write(&path, original).expect("malformed state fixture should persist");
        let registration = FakeRegistration::new([Err("unexpected registration check")], None);

        assert_failed(initialize_with_fake(
            &root.path,
            "/opt/Unfocus",
            &registration,
        ));

        assert_eq!(registration.enable_calls.get(), 0);
        assert_eq!(registration.check_calls.get(), 0);
        assert_eq!(
            fs::read(&path).expect("state bytes should remain"),
            original
        );
    }

    #[test]
    fn enable_failure_leaves_the_pending_state_for_fail_closed_recovery() {
        let root = TestDirectory::new();
        let path = state_file_path(&root.path);
        let registration =
            FakeRegistration::new([Err("unexpected registration check")], Some("denied"));

        assert_failed(initialize_with_fake(
            &root.path,
            "/opt/Unfocus",
            &registration,
        ));

        assert_eq!(registration.enable_calls.get(), 1);
        assert_eq!(registration.check_calls.get(), 0);
        assert_eq!(
            load_state(&path).expect("pending state should load"),
            Some(state(RegistrationState::Pending, "/opt/Unfocus"))
        );
    }

    #[test]
    fn verification_failure_leaves_the_pending_state_for_fail_closed_recovery() {
        let root = TestDirectory::new();
        let path = state_file_path(&root.path);
        let registration = FakeRegistration::new([Ok(false)], None);

        assert_failed(initialize_with_fake(
            &root.path,
            "/opt/Unfocus",
            &registration,
        ));

        assert_eq!(registration.enable_calls.get(), 1);
        assert_eq!(registration.check_calls.get(), 1);
        assert_eq!(
            load_state(&path).expect("pending state should load"),
            Some(state(RegistrationState::Pending, "/opt/Unfocus"))
        );
    }

    #[test]
    fn state_is_atomically_persisted_and_reloaded() {
        let root = TestDirectory::new();
        let path = state_file_path(&root.path);
        let expected = PersistedAutostartState::complete("/opt/Unfocus");

        persist_state(&path, &expected).expect("state should persist");

        assert_eq!(
            load_state(&path).expect("state should load"),
            Some(expected)
        );
        assert!(!root
            .path
            .join(format!(".{AUTOSTART_STATE_FILE_NAME}.tmp"))
            .exists());
    }

    #[test]
    fn initial_state_persistence_failure_is_nonfatal_and_does_not_enable() {
        let root = TestDirectory::new();
        let path = state_file_path(&root.path);
        inject_persist_failure(path.clone());
        let registration = FakeRegistration::new([Err("unexpected registration check")], None);

        assert_failed(initialize_with_fake(
            &root.path,
            "/opt/Unfocus",
            &registration,
        ));

        assert_eq!(registration.enable_calls.get(), 0);
        assert_eq!(registration.check_calls.get(), 0);
        assert!(!path.exists());
    }

    #[test]
    fn desktop_entry_escaping_keeps_paths_as_one_argument() {
        assert_eq!(
            desktop_entry_executable_path("/opt/Unfocus %% `night` $HOME\\bin\"test")
                .expect("path should escape"),
            r#""/opt/Unfocus %%%% \\`night\\` \\$HOME\\\\bin\\"test""#
        );
        assert!(desktop_entry_executable_path("/opt/Unfocus\nnext").is_err());
    }

    #[test]
    fn windows_quoting_preserves_spaces_quotes_and_trailing_backslashes() {
        assert_eq!(
            windows_command_line_quote(r#"C:\Program Files\Unfocus\unfocus.exe"#)
                .expect("path should quote"),
            r#""C:\Program Files\Unfocus\unfocus.exe""#
        );
        assert_eq!(
            windows_command_line_quote(r#"C:\a\"quoted"#).expect("path should quote"),
            r#""C:\a\\\"quoted""#
        );
        assert_eq!(
            windows_command_line_quote(r#"C:\ends-with-slash\"#).expect("path should quote"),
            r#""C:\ends-with-slash\\""#
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_registration_uses_xdg_config_home_and_preserves_its_disable_flags() {
        let root = TestDirectory::new();
        let home = root.path.join("home");
        let xdg = root.path.join("xdg");
        let entry_path = linux_registration_path(Some(xdg.as_os_str()), Some(home.as_os_str()))
            .expect("absolute XDG directory should resolve");
        assert_eq!(entry_path, xdg.join("autostart/Unfocus.desktop"));
        let registration = LinuxRegistration {
            path: entry_path.clone(),
            entry: "[Desktop Entry]\nType=Application\nExec=/opt/Unfocus\n".into(),
        };
        let config = xdg.join("com.unfocus.desktop");
        let diagnostics = initialize_with_linux(&config, &registration);
        assert_eq!(diagnostics.status, AutostartStatus::Enabled);
        assert!(!home.join(".config/autostart/Unfocus.desktop").exists());
        let disabled = format!("{}Hidden=true\n", registration.entry);
        fs::write(&entry_path, &disabled).expect("disable fixture should persist");
        assert_eq!(
            initialize_with_linux(&config, &registration).status,
            AutostartStatus::Disabled
        );
        assert_eq!(
            fs::read_to_string(&entry_path).expect("entry should remain"),
            disabled
        );
        fs::remove_file(&entry_path).expect("entry should be removable");
        assert_eq!(
            initialize_with_linux(&config, &registration).status,
            AutostartStatus::Disabled
        );
        assert!(!entry_path.exists());
    }

    #[cfg(target_os = "linux")]
    fn initialize_with_linux(
        config: &Path,
        registration: &LinuxRegistration,
    ) -> AutostartDiagnostics {
        initialize_registration(config, "/opt/Unfocus", Ok::<_, &str>(registration))
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn linux_registration_ignores_empty_or_relative_xdg_directories() {
        let root = TestDirectory::new();
        let home = root.path.as_os_str();
        let expected = root.path.join(".config/autostart/Unfocus.desktop");
        for xdg in [
            None,
            Some(OsStr::new("")),
            Some(OsStr::new("relative/config")),
        ] {
            assert_eq!(
                linux_registration_path(xdg, Some(home)),
                Ok(expected.clone())
            );
        }
        assert!(linux_registration_path(None, None).is_err());
        assert!(linux_registration_path(None, Some(OsStr::new("relative/home"))).is_err());
        assert_eq!(
            linux_registration_path(Some(home), None),
            Ok(root.path.join("autostart/Unfocus.desktop"))
        );
    }

    #[test]
    fn linux_desktop_disable_flags_are_limited_to_the_desktop_section() {
        assert!(desktop_entry_is_disabled(
            "[Desktop Entry]\nHidden=true\nExec=unfocus\n[Other]\nX-GNOME-Autostart-enabled=true"
        ));
        assert!(desktop_entry_is_disabled(
            "[Desktop Entry]\nX-GNOME-Autostart-enabled=false\nExec=unfocus"
        ));
        assert!(!desktop_entry_is_disabled(
            "[Desktop Entry]\nHidden=false\n[Other]\nX-GNOME-Autostart-enabled=false"
        ));
    }

    #[test]
    fn launchctl_disabled_map_matches_only_enabled_overrides() {
        assert!(launchctl_output_disables_label(
            "disabled services = {\n  \"com.unfocus.desktop\" => true;\n}",
            MACOS_LAUNCH_AGENT_LABEL
        ));
        assert!(!launchctl_output_disables_label(
            "disabled services = {\n  \"com.unfocus.desktop\" => false;\n}",
            MACOS_LAUNCH_AGENT_LABEL
        ));
        assert!(!launchctl_output_disables_label(
            "disabled services = {\n  \"com.unfocus.desktop.other\" => true;\n}",
            MACOS_LAUNCH_AGENT_LABEL
        ));
    }

    #[test]
    fn macos_xml_metacharacters_are_rejected() {
        assert!(is_xml_metacharacter('&'));
        assert!(is_xml_metacharacter('\''));
        assert!(!is_xml_metacharacter('/'));
    }

    #[test]
    fn development_builds_skip_release_initialization_without_state_mutation() {
        let root = TestDirectory::new();
        let path = state_file_path(&root.path);
        let initialized = Cell::new(false);

        let diagnostics = initialize_for_build(true, || {
            initialized.set(true);
            persist_state(&path, &PersistedAutostartState::pending("/opt/Unfocus"))
                .expect("release initialization fixture should persist");
            AutostartDiagnostics::enabled()
        });

        assert_eq!(diagnostics.status, AutostartStatus::SkippedDevelopment);
        assert_eq!(diagnostics.error, None);
        assert!(!initialized.get());
        assert!(!path.exists());
    }
}
