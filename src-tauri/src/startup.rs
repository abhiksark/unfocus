// src-tauri/src/startup.rs

use serde::Serialize;
#[cfg(target_os = "linux")]
use tauri::Manager;
#[cfg(target_os = "macos")]
mod macos;
use tauri::WebviewWindow;

#[derive(Debug, PartialEq, Eq, Serialize)]
pub(crate) struct StartAtLoginStatus {
    supported: bool,
    enabled: bool,
}

#[tauri::command]
pub(crate) fn get_start_at_login(window: WebviewWindow) -> Result<StartAtLoginStatus, String> {
    crate::authorize_main_caller(window.label())?;
    #[cfg(target_os = "linux")]
    {
        linux::read(&linux::entry_path(&window)?)
    }
    #[cfg(target_os = "macos")]
    {
        macos::read(&macos::entry_path(&window)?)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    Ok(StartAtLoginStatus {
        supported: false,
        enabled: false,
    })
}

#[tauri::command]
pub(crate) fn set_start_at_login(
    window: WebviewWindow,
    enabled: bool,
) -> Result<StartAtLoginStatus, String> {
    crate::authorize_main_caller(window.label())?;
    #[cfg(target_os = "linux")]
    {
        let path = linux::entry_path(&window)?;
        let executable = if enabled {
            Some(
                tauri::process::current_binary(&window.app_handle().env())
                    .map_err(|error| format!("could not locate Unfocus: {error}"))?,
            )
        } else {
            None
        };
        linux::write(&path, executable.as_deref()).map_err(|error| error.to_string())?;
        linux::read(&path)
    }
    #[cfg(target_os = "macos")]
    {
        let path = macos::entry_path(&window)?;
        let executable = std::env::current_exe().map_err(|error| error.to_string())?;
        macos::write(&path, enabled.then_some(executable.as_path()))?;
        macos::read(&path)
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        let _ = enabled;
        Ok(StartAtLoginStatus {
            supported: false,
            enabled: false,
        })
    }
}

#[cfg(target_os = "linux")]
mod linux {
    use super::StartAtLoginStatus;
    use std::{
        collections::HashMap,
        ffi::OsStr,
        fs::{self, OpenOptions},
        io::{self, Write},
        os::unix::fs::PermissionsExt,
        path::{Path, PathBuf},
        sync::atomic::{AtomicU64, Ordering},
    };
    use tauri::Manager;

    const ENTRY_NAME: &str = "com.unfocus.desktop.desktop";
    static TEMP_ID: AtomicU64 = AtomicU64::new(0);

    pub(super) fn entry_path(window: &tauri::WebviewWindow) -> Result<PathBuf, String> {
        window
            .path()
            .config_dir()
            .map(|directory| directory.join("autostart").join(ENTRY_NAME))
            .map_err(|error| format!("could not locate startup configuration: {error}"))
    }

    fn invalid(message: &str) -> io::Error {
        io::Error::new(io::ErrorKind::InvalidData, message)
    }

    fn escape_value(value: &str) -> String {
        value.replace('\\', "\\\\")
    }

    fn desktop_entry(executable: &Path) -> io::Result<String> {
        let executable_text = executable
            .to_str()
            .ok_or_else(|| invalid("the Unfocus executable path is not valid UTF-8"))?;
        // env recognizes assignments containing '=' even after its '--' separator.
        if executable_text.contains('=') {
            return Err(invalid(
                "move Unfocus to a path without '=' before enabling start at login",
            ));
        }
        if !executable.is_absolute() || executable_text.chars().any(char::is_control) {
            return Err(invalid(
                "the Unfocus executable path cannot be used in a desktop entry",
            ));
        }
        let mut quoted = String::from("\"");
        for character in executable_text.chars() {
            if matches!(character, '\\' | '"' | '$' | '`') {
                quoted.push('\\');
            }
            if character == '%' {
                quoted.push('%');
            }
            quoted.push(character);
        }
        quoted.push('"');
        // GLib checks Exec's executable before expanding %% to a literal percent.
        // env passes the expanded path as an argument, then replaces itself without a shell.
        Ok(format!(
            "[Desktop Entry]\nType=Application\nName=Unfocus\nExec=/usr/bin/env -- {} --autostart\nTryExec={}\nTerminal=false\n",
            escape_value(&quoted), escape_value(executable_text).replace(' ', "\\s")
        ))
    }

    fn entry_values(contents: &str) -> io::Result<HashMap<&str, &str>> {
        let mut in_entry = false;
        let mut found_entry = false;
        let mut values = HashMap::new();
        for line in contents.lines().map(str::trim) {
            if line.is_empty() || line.starts_with('#') {
                continue;
            }
            if line.starts_with('[') && line.ends_with(']') {
                in_entry = line == "[Desktop Entry]";
                if in_entry && found_entry {
                    return Err(invalid("duplicate Desktop Entry group"));
                }
                found_entry |= in_entry;
            } else if in_entry {
                let (key, value) = line
                    .split_once('=')
                    .ok_or_else(|| invalid("invalid startup entry line"))?;
                if values.insert(key.trim(), value.trim()).is_some() {
                    return Err(invalid("duplicate startup entry key"));
                }
            }
        }
        if !found_entry {
            return Err(invalid("startup file has no Desktop Entry group"));
        }
        Ok(values)
    }

    fn boolean(values: &HashMap<&str, &str>, key: &str, default: bool) -> io::Result<bool> {
        match values.get(key).copied() {
            None => Ok(default),
            Some("true") => Ok(true),
            Some("false") => Ok(false),
            Some(_) => Err(invalid("invalid startup entry boolean")),
        }
    }

    fn executable_exists(path: &Path) -> io::Result<bool> {
        match fs::metadata(path) {
            Ok(metadata) => Ok(metadata.is_file() && metadata.permissions().mode() & 0o111 != 0),
            Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(false),
            Err(error) => Err(error),
        }
    }

    fn try_exec_exists(value: &str, search_path: &OsStr) -> io::Result<bool> {
        let mut decoded = String::new();
        let mut characters = value.chars();
        while let Some(character) = characters.next() {
            decoded.push(if character == '\\' {
                match characters.next() {
                    Some('\\') => '\\',
                    Some('s') => ' ',
                    Some('n') => '\n',
                    Some('t') => '\t',
                    Some('r') => '\r',
                    _ => return Err(invalid("invalid TryExec escape")),
                }
            } else {
                character
            });
        }
        let path = Path::new(&decoded);
        if path.is_absolute() {
            return executable_exists(path);
        }
        if decoded.contains('/') {
            return Err(invalid(
                "TryExec must be an absolute path or an executable name",
            ));
        }
        for directory in std::env::split_paths(search_path) {
            if executable_exists(&directory.join(path))? {
                return Ok(true);
            }
        }
        Ok(false)
    }

    fn entry_enabled(contents: &str, desktop: &str, search_path: &OsStr) -> io::Result<bool> {
        let values = entry_values(contents)?;
        if boolean(&values, "Hidden", false)?
            || !boolean(&values, "X-GNOME-Autostart-enabled", true)?
        {
            return Ok(false);
        }
        if values.get("Type") != Some(&"Application")
            || values.get("Name").is_none_or(|value| value.is_empty())
            || values.get("Exec").is_none_or(|value| value.is_empty())
        {
            return Err(invalid(
                "startup entry is missing its application type, name, or command",
            ));
        }
        let matches_desktop = |list: &&str| {
            list.split(';')
                .filter(|name| !name.is_empty())
                .any(|name| desktop.split(':').any(|current| current == name))
        };
        if values.contains_key("OnlyShowIn") && values.contains_key("NotShowIn") {
            return Err(invalid(
                "startup entry contains conflicting desktop restrictions",
            ));
        }
        if values
            .get("OnlyShowIn")
            .is_some_and(|list| !matches_desktop(list))
            || values.get("NotShowIn").is_some_and(matches_desktop)
        {
            return Ok(false);
        }
        if let Some(value) = values.get("TryExec").filter(|value| !value.is_empty()) {
            return try_exec_exists(value, search_path);
        }
        Ok(true)
    }

    pub(super) fn read(path: &Path) -> Result<StartAtLoginStatus, String> {
        let enabled = match fs::read_to_string(path) {
            Ok(contents) => entry_enabled(
                &contents,
                &std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default(),
                &std::env::var_os("PATH").unwrap_or_default(),
            )
            .map_err(|error| format!("could not read startup registration: {error}"))?,
            Err(error) if error.kind() == io::ErrorKind::NotFound => false,
            Err(error) => return Err(format!("could not read startup registration: {error}")),
        };
        Ok(StartAtLoginStatus {
            supported: true,
            enabled,
        })
    }

    pub(super) fn write(path: &Path, executable: Option<&Path>) -> io::Result<()> {
        let Some(executable) = executable else {
            return match fs::remove_file(path) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
                result => result,
            };
        };
        let contents = desktop_entry(executable)?;
        if !executable_exists(Path::new("/usr/bin/env"))? {
            return Err(invalid(
                "the system startup launcher /usr/bin/env is unavailable",
            ));
        }
        if !executable_exists(executable)? {
            return Err(invalid(
                "the Unfocus executable is missing or is not executable",
            ));
        }
        let parent = path
            .parent()
            .ok_or_else(|| invalid("startup entry has no parent"))?;
        fs::create_dir_all(parent)?;
        let temp_path = parent.join(format!(
            ".{ENTRY_NAME}.{}-{}.tmp",
            std::process::id(),
            TEMP_ID.fetch_add(1, Ordering::Relaxed)
        ));
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temp_path)?;
        let result = file
            .write_all(contents.as_bytes())
            .and_then(|()| file.sync_all());
        drop(file);
        let result = result
            .and_then(|()| crate::storage_recovery::replace_file_atomically(&temp_path, path));
        if result.is_err() {
            let _ = fs::remove_file(temp_path);
        }
        result
    }

    #[cfg(test)]
    mod tests {
        use super::*;

        fn test_directory() -> PathBuf {
            let directory = std::env::temp_dir().join(format!(
                "unfocus-startup-test-{}-{}",
                std::process::id(),
                TEMP_ID.fetch_add(1, Ordering::Relaxed)
            ));
            fs::create_dir(&directory).unwrap();
            directory
        }

        #[test]
        fn exec_escapes_reserved_characters_and_preserves_appimage_path() {
            let mut environment = tauri::Env::default();
            environment.appimage = Some("/opt/Unfocus $`\"\\%f's.AppImage".into());
            let executable = tauri::process::current_binary(&environment).unwrap();
            let contents = desktop_entry(&executable).unwrap();
            assert!(contents.contains(
                r#"Exec=/usr/bin/env -- "/opt/Unfocus \\$\\`\\"\\\\%%f's.AppImage" --autostart"#
            ));
            assert!(contents.contains(r#"TryExec=/opt/Unfocus\s$`"\\%f's.AppImage"#));
            for path in [
                "relative",
                "/tmp/a=b",
                "/tmp/a=b/unfocus",
                "/tmp/a\nExec=evil",
            ] {
                assert!(desktop_entry(Path::new(path)).is_err());
            }
        }

        #[test]
        fn desktop_disabled_entries_and_invalid_files_are_not_enabled() {
            let base =
                "[Desktop Entry]\nType=Application\nName=Unfocus\nExec=unfocus --autostart\n";
            for (extra, enabled) in [
                ("", true),
                ("Hidden=true\n", false),
                ("X-GNOME-Autostart-enabled=false\n", false),
                ("OnlyShowIn=KDE;\n", false),
                ("OnlyShowIn=GNOME;\n", true),
                ("NotShowIn=GNOME;\n", false),
                ("NotShowIn=KDE;\n", true),
                ("TryExec=/nonexistent/unfocus\n", false),
                ("NoDisplay=true\n", true),
            ] {
                assert_eq!(
                    entry_enabled(&format!("{base}{extra}"), "ubuntu:GNOME", OsStr::new(""))
                        .unwrap(),
                    enabled,
                    "{extra}"
                );
            }
            for contents in [
                "garbage",
                "[Desktop Entry]\nHidden=maybe",
                "[Desktop Entry]\nExec=foo",
                "[Desktop Entry]\nHidden=false\nHidden=true",
            ] {
                assert!(entry_enabled(contents, "GNOME", OsStr::new("")).is_err());
            }
        }

        #[test]
        fn registration_is_atomic_and_disable_removes_only_its_entry() {
            let directory = test_directory();
            let path = directory.join(ENTRY_NAME);
            let other = directory.join("other.desktop");
            fs::write(&other, "unrelated").unwrap();
            assert!(!read(&path).unwrap().enabled);
            let executable = std::env::current_exe().unwrap();
            write(&path, Some(&executable)).unwrap();
            assert!(read(&path).unwrap().enabled);
            let original = fs::read(&path).unwrap();
            crate::storage_recovery::inject_replacement_failure(path.clone());
            assert!(write(&path, Some(&executable)).is_err());
            assert_eq!(fs::read(&path).unwrap(), original);
            assert_eq!(fs::read_dir(&directory).unwrap().count(), 2);
            write(&path, None).unwrap();
            write(&path, None).unwrap();
            assert!(!read(&path).unwrap().enabled);
            assert_eq!(fs::read_to_string(other).unwrap(), "unrelated");
            fs::remove_dir_all(directory).unwrap();
        }

        #[test]
        fn filesystem_and_executable_failures_are_reported() {
            let directory = test_directory();
            let blocker = directory.join("blocker");
            fs::write(&blocker, "file").unwrap();
            let executable = std::env::current_exe().unwrap();
            assert!(write(&blocker.join(ENTRY_NAME), Some(&executable)).is_err());
            assert!(read(&blocker.join(ENTRY_NAME)).is_err());
            assert!(read(&directory).is_err());
            assert!(write(&directory, None).is_err());
            assert!(write(&directory.join(ENTRY_NAME), Some(&blocker)).is_err());
            assert!(!try_exec_exists("blocker", directory.as_os_str()).unwrap());
            fs::set_permissions(&blocker, fs::Permissions::from_mode(0o700)).unwrap();
            assert!(try_exec_exists("blocker", directory.as_os_str()).unwrap());
            let unusual = directory.join("Unfocus $`\"\\%f's.AppImage ");
            fs::rename(&blocker, &unusual).unwrap();
            let registration = directory.join(ENTRY_NAME);
            write(&registration, Some(&unusual)).unwrap();
            assert!(read(&registration).unwrap().enabled);
            fs::remove_dir_all(directory).unwrap();
        }
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn startup_commands_require_the_main_window() {
        assert!(crate::authorize_main_caller("main").is_ok());
        for label in [
            "overlay-1-0-1-20-1000",
            "cue-1-1000",
            "tray-panel",
            "main-extra",
            "",
        ] {
            assert!(crate::authorize_main_caller(label).is_err());
        }
    }
}
