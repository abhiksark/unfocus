//! Per-user login registration. Changes apply at the next login; never launch
//! or terminate the running application when changing this preference.
use super::StartAtLoginStatus;
use std::{
    fs,
    io::Write,
    os::unix::fs::{OpenOptionsExt, PermissionsExt},
    path::{Path, PathBuf},
    process::Command,
};
use tauri::Manager;

const LABEL: &str = "com.unfocus.desktop.start-at-login";
static TEMP_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

pub(super) fn entry_path(window: &tauri::WebviewWindow) -> Result<PathBuf, String> {
    window
        .path()
        .home_dir()
        .map(|home| {
            home.join("Library/LaunchAgents")
                .join(format!("{LABEL}.plist"))
        })
        .map_err(|error| format!("could not locate login configuration: {error}"))
}

fn document(executable: &Path) -> Result<String, String> {
    let value = executable
        .to_str()
        .ok_or("Unfocus path is not valid UTF-8")?;
    if !executable.is_absolute() || value.chars().any(char::is_control) {
        return Err("Unfocus needs an absolute path without control characters".into());
    }
    // Pin the installed bundle, not a translocated/downloaded/debug executable.
    if !value.contains(".app/Contents/MacOS/")
        || value.starts_with("/Volumes/")
        || value.contains("/AppTranslocation/")
    {
        return Err(
            "Move Unfocus to Applications and reopen it before enabling start at login".into(),
        );
    }
    let escaped = value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;");
    Ok(format!(
        r#"<?xml version="1.0" encoding="UTF-8"?>
<plist version="1.0"><dict>
<key>Label</key><string>{LABEL}</string>
<key>ProgramArguments</key><array><string>{escaped}</string><string>--autostart</string></array>
<key>RunAtLoad</key><true/>
<key>LimitLoadToSessionType</key><string>Aqua</string>
</dict></plist>
"#
    ))
}

fn existing(path: &Path) -> Result<Option<serde_json::Value>, String> {
    match fs::symlink_metadata(path) {
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(error.to_string()),
        Ok(metadata) if !metadata.is_file() => {
            return Err(
                "Login configuration is not a regular file; preserve it and review manually".into(),
            )
        }
        Ok(_) => {}
    }
    let output = Command::new("/usr/bin/plutil")
        .args(["-convert", "json", "-o", "-", "--"])
        .arg(path)
        .output()
        .map_err(|error| error.to_string())?;
    if !output.status.success() {
        return Err("Cannot read login configuration; preserve it and review manually".into());
    }
    let value: serde_json::Value =
        serde_json::from_slice(&output.stdout).map_err(|error| error.to_string())?;
    if value["Label"] != LABEL {
        return Err(
            "Login configuration belongs to another application; refusing to change it".into(),
        );
    }
    let fields = value
        .as_object()
        .ok_or("Login configuration must be a dictionary")?;
    const ALLOWED_KEYS: &[&str] = &[
        "Label",
        "ProgramArguments",
        "RunAtLoad",
        "LimitLoadToSessionType",
        "Disabled",
    ];
    if fields
        .keys()
        .any(|key| !ALLOWED_KEYS.contains(&key.as_str()))
    {
        return Err(
            "Login configuration has unsupported launch settings; preserve it and review manually"
                .into(),
        );
    }
    if value["LimitLoadToSessionType"] != "Aqua" {
        return Err("Login configuration has an unsupported session type".into());
    }
    if fields
        .get("Disabled")
        .is_some_and(|disabled| !disabled.is_boolean())
    {
        return Err("Login configuration Disabled setting must be a boolean".into());
    }
    Ok(Some(value))
}

fn executable_exists(path: &Path) -> Result<bool, String> {
    match fs::metadata(path) {
        Ok(metadata) => Ok(metadata.is_file() && metadata.permissions().mode() & 0o111 != 0),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(false),
        Err(error) => Err(format!("Cannot inspect login executable: {error}")),
    }
}

pub(super) fn read(path: &Path) -> Result<StartAtLoginStatus, String> {
    let enabled = match existing(path)? {
        None => false,
        Some(value) => {
            let arguments = value["ProgramArguments"]
                .as_array()
                .ok_or("Invalid login arguments")?;
            let executable = arguments
                .first()
                .and_then(|v| v.as_str())
                .ok_or("Missing login executable")?;
            if arguments.len() != 2 || arguments[1] != "--autostart" || value["RunAtLoad"] != true {
                return Err(
                    "Login configuration was modified; review it before changing startup".into(),
                );
            }
            document(Path::new(executable))?;
            executable_exists(Path::new(executable))? && value["Disabled"] != true
        }
    };
    Ok(StartAtLoginStatus {
        supported: true,
        enabled,
    })
}

pub(super) fn write(path: &Path, executable: Option<&Path>) -> Result<(), String> {
    read(path)?; // Validate the complete entry before replacing or deleting it.
    let Some(executable) = executable else {
        return match fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.to_string()),
        };
    };
    let contents = document(executable)?;
    if !executable_exists(executable)? {
        return Err("Unfocus executable is missing".into());
    }
    let parent = path
        .parent()
        .ok_or("Missing login configuration directory")?;
    fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    let id = TEMP_ID.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let temporary = parent.join(format!(".{LABEL}.{}.{id}.tmp", std::process::id()));
    let result = (|| -> std::io::Result<()> {
        let mut file = fs::OpenOptions::new()
            .create_new(true)
            .write(true)
            .mode(0o600)
            .open(&temporary)?;
        file.write_all(contents.as_bytes())?;
        file.sync_all()?;
        fs::rename(&temporary, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(temporary);
    }
    result.map_err(|error| format!("Could not save login setting: {error}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_temporary_and_unbundled_executables() {
        for path in [
            "relative",
            "/tmp/unfocus",
            "/Volumes/Test/Unfocus.app/Contents/MacOS/Unfocus",
            "/private/tmp/AppTranslocation/id/Unfocus.app/Contents/MacOS/Unfocus",
        ] {
            assert!(document(Path::new(path)).is_err());
        }
    }
    #[test]
    fn roundtrip_registration_preserves_arguments_and_unrelated_files() {
        let root = std::env::temp_dir().join(format!("unfocus-login-test-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let executable = root.join("A & <test> ' $ .app/Contents/MacOS/Unfocus");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, "fixture").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let entry = root.join("login.plist");
        assert!(!read(&entry).unwrap().enabled);
        write(&entry, Some(&executable)).unwrap();
        assert!(read(&entry).unwrap().enabled);
        let value = existing(&entry).unwrap().unwrap();
        assert_eq!(value["ProgramArguments"][0], executable.to_str().unwrap());
        assert!(value.get("KeepAlive").is_none());
        write(&entry, None).unwrap();
        assert!(!read(&entry).unwrap().enabled);
        fs::write(&entry, "corrupt user data").unwrap();
        assert!(write(&entry, Some(&executable)).is_err());
        assert!(write(&entry, None).is_err());
        assert_eq!(fs::read_to_string(&entry).unwrap(), "corrupt user data");
        fs::remove_dir_all(root).unwrap();
    }
}

#[cfg(test)]
mod modified_entry_tests {
    use super::*;
    fn fixture(name: &str, extra: &str) -> (PathBuf, PathBuf) {
        let root =
            std::env::temp_dir().join(format!("unfocus-retest-{}-{name}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let executable = root.join("Unfocus.app/Contents/MacOS/Unfocus");
        fs::create_dir_all(executable.parent().unwrap()).unwrap();
        fs::write(&executable, "fixture").unwrap();
        fs::set_permissions(&executable, fs::Permissions::from_mode(0o700)).unwrap();
        let entry = root.join("test.plist");
        fs::write(
            &entry,
            document(&executable)
                .unwrap()
                .replace("</dict>", &format!("{extra}</dict>")),
        )
        .unwrap();
        (root, entry)
    }
    #[test]
    fn disabled_entry_must_not_report_enabled() {
        let (root, entry) = fixture("disabled", "<key>Disabled</key><true/>");
        let result = read(&entry);
        fs::remove_dir_all(root).unwrap();
        assert!(
            !result.unwrap().enabled,
            "Disabled=true still reports enabled"
        );
    }
    #[test]
    fn disabled_configuration_can_be_reenabled_and_invalid_boolean_is_preserved() {
        let (root, entry) = fixture("reenable", "<key>Disabled</key><true/>");
        let executable = root.join("Unfocus.app/Contents/MacOS/Unfocus");
        write(&entry, Some(&executable)).unwrap();
        assert!(read(&entry).unwrap().enabled);
        let bad = document(&executable).unwrap().replace(
            "</dict>",
            "<key>Disabled</key><string>false</string></dict>",
        );
        fs::write(&entry, &bad).unwrap();
        assert!(read(&entry).is_err());
        assert!(write(&entry, None).is_err());
        assert_eq!(fs::read_to_string(&entry).unwrap(), bad);
        fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn program_override_must_not_report_unfocus_enabled() {
        let (root, entry) = fixture(
            "program",
            "<key>Program</key><string>/usr/bin/false</string>",
        );
        let result = read(&entry);
        fs::remove_dir_all(root).unwrap();
        assert!(
            result.is_err() || !result.unwrap().enabled,
            "A different Program still reports Unfocus enabled"
        );
    }
    #[test]
    fn keepalive_modification_must_be_rejected() {
        let (root, entry) = fixture("keepalive", "<key>KeepAlive</key><true/>");
        let before = fs::read(&entry).unwrap();
        let result = read(&entry);
        assert!(write(&entry, None).is_err());
        let executable = root.join("Unfocus.app/Contents/MacOS/Unfocus");
        assert!(write(&entry, Some(&executable)).is_err());
        assert_eq!(fs::read(&entry).unwrap(), before);
        fs::remove_dir_all(root).unwrap();
        assert!(
            result.is_err(),
            "KeepAlive=true is accepted even though Quit must stay quit"
        );
    }
}
