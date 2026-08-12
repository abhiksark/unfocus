use crate::{
    authorize_main_caller,
    probes::{probe_backend, ProbeBackend, ProbeCache},
    tray::{TrayDiagnostics, TrayRuntime},
};
use serde::Serialize;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::{Manager, PhysicalPosition, PhysicalSize, State, WebviewWindow};

static MONITOR_ENUMERATION_UNAVAILABLE: AtomicBool = AtomicBool::new(false);

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
struct MonitorReport {
    name: Option<String>,
    x: i32,
    y: i32,
    width: u32,
    height: u32,
    scale_factor: f64,
}

#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct DiagnosticsReport {
    operating_system: &'static str,
    session_type: Option<String>,
    desktop: Option<String>,
    display: Option<String>,
    monitors: Vec<MonitorReport>,
    monitor_error: Option<String>,
    idle_seconds: Option<u64>,
    idle_error: Option<String>,
    active_window_fullscreen: Option<bool>,
    fullscreen_error: Option<String>,
    probe_backend: ProbeBackend,
    tray: TrayDiagnostics,
}

fn environment_value(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

/// The windowing session, reported per platform rather than left blank.
///
/// The XDG variables these read on Linux do not exist on macOS, where the
/// session is always Quartz, so returning `None` there would read as a failed
/// probe rather than a different platform.
fn session_type() -> Option<String> {
    if cfg!(target_os = "macos") {
        return Some("quartz".to_owned());
    }
    if cfg!(target_os = "windows") {
        return Some("win32".to_owned());
    }

    environment_value("XDG_SESSION_TYPE")
}

fn desktop() -> Option<String> {
    if cfg!(target_os = "macos") {
        return Some("Aqua".to_owned());
    }
    if cfg!(target_os = "windows") {
        return Some("Windows".to_owned());
    }

    environment_value("XDG_CURRENT_DESKTOP")
}

fn display() -> Option<String> {
    if cfg!(target_os = "macos") {
        return Some("Quartz Compositor".to_owned());
    }
    if cfg!(target_os = "windows") {
        return Some("Desktop Window Manager".to_owned());
    }

    environment_value("DISPLAY")
}

fn monitor_report(
    name: Option<&str>,
    position: &PhysicalPosition<i32>,
    size: &PhysicalSize<u32>,
    scale_factor: f64,
) -> MonitorReport {
    MonitorReport {
        name: name.map(str::to_owned),
        x: position.x,
        y: position.y,
        width: size.width,
        height: size.height,
        scale_factor,
    }
}

#[tauri::command]
pub(crate) fn get_diagnostics(
    window: WebviewWindow,
    probe_cache: State<'_, ProbeCache>,
    tray_runtime: State<'_, TrayRuntime>,
) -> Result<DiagnosticsReport, String> {
    authorize_main_caller(window.label())?;
    let app = window.app_handle();
    let (monitors, monitor_error) = match app.available_monitors() {
        Ok(monitors) => {
            if MONITOR_ENUMERATION_UNAVAILABLE.swap(false, Ordering::AcqRel) {
                eprintln!("monitor enumeration recovered");
            }
            let reports = monitors
                .iter()
                .map(|monitor| {
                    monitor_report(
                        monitor.name().map(String::as_str),
                        monitor.position(),
                        monitor.size(),
                        monitor.scale_factor(),
                    )
                })
                .collect();
            (reports, None)
        }
        Err(error) => {
            let message = format!("monitor enumeration failed: {error}");
            if !MONITOR_ENUMERATION_UNAVAILABLE.swap(true, Ordering::AcqRel) {
                eprintln!("{message}");
            }
            (Vec::new(), Some(message))
        }
    };

    let probes = probe_cache.snapshot();
    let (idle_seconds, idle_error) = match probes.idle_seconds {
        Ok(seconds) => (Some(seconds), None),
        Err(error) => (None, Some(error)),
    };
    let (active_window_fullscreen, fullscreen_error) = match probes.active_window_fullscreen {
        Ok(fullscreen) => (Some(fullscreen), None),
        Err(error) => (None, Some(error)),
    };

    Ok(DiagnosticsReport {
        operating_system: std::env::consts::OS,
        session_type: session_type(),
        desktop: desktop(),
        display: display(),
        monitors,
        monitor_error,
        idle_seconds,
        idle_error,
        active_window_fullscreen,
        fullscreen_error,
        probe_backend: probe_backend(),
        tray: tray_runtime.diagnostics(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn monitor_reports_preserve_physical_topology() {
        let report = monitor_report(
            Some("DP-4"),
            &PhysicalPosition::new(-1920, 240),
            &PhysicalSize::new(1920, 1080),
            1.25,
        );

        assert_eq!(report.name.as_deref(), Some("DP-4"));
        assert_eq!((report.x, report.y), (-1920, 240));
        assert_eq!((report.width, report.height), (1920, 1080));
        assert_eq!(report.scale_factor, 1.25);
    }
}
