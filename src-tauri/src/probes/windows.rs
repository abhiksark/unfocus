//! Windows idle and fullscreen probes (issue #24).
//!
//! Idle uses `GetLastInputInfo` + tick count with wrapping-safe subtraction.
//! Fullscreen compares the foreground top-level window's outer rect to the
//! monitor it occupies (`rcMonitor`, not the work area), so maximized
//! work-area windows are not reported as fullscreen.

#[cfg(target_os = "windows")]
use windows_sys::Win32::{
    Foundation::{FALSE, HWND, RECT},
    Graphics::Gdi::{GetMonitorInfoW, MonitorFromWindow, MONITORINFO, MONITOR_DEFAULTTONEAREST},
    System::SystemInformation::GetTickCount,
    UI::{
        Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO},
        WindowsAndMessaging::{
            GetForegroundWindow, GetWindowRect, GetWindowTextW, IsIconic, IsWindowVisible,
        },
    },
};

/// Integer screen rectangle in device pixels (Win32 screen coordinates).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ScreenRect {
    left: i32,
    top: i32,
    right: i32,
    bottom: i32,
}

impl ScreenRect {
    fn width(self) -> i32 {
        self.right.saturating_sub(self.left)
    }

    fn height(self) -> i32 {
        self.bottom.saturating_sub(self.top)
    }
}

/// Pixel tolerance for DPI/rounding when matching window outer bounds to a
/// monitor. One physical pixel is enough for typical borderless fullscreen.
const FULLSCREEN_TOLERANCE_PX: i32 = 2;

/// True when `window` covers `monitor` edge to edge within tolerance.
///
/// Maximized-to-work-area windows leave the taskbar free and therefore fail
/// this check against the full monitor rectangle.
fn window_covers_monitor(window: ScreenRect, monitor: ScreenRect) -> bool {
    if window.width() <= 0 || window.height() <= 0 {
        return false;
    }
    if monitor.width() <= 0 || monitor.height() <= 0 {
        return false;
    }
    (window.left - monitor.left).abs() <= FULLSCREEN_TOLERANCE_PX
        && (window.top - monitor.top).abs() <= FULLSCREEN_TOLERANCE_PX
        && (window.right - monitor.right).abs() <= FULLSCREEN_TOLERANCE_PX
        && (window.bottom - monitor.bottom).abs() <= FULLSCREEN_TOLERANCE_PX
}

/// Wrapping-safe idle seconds from last-input tick and current tick.
///
/// `GetTickCount` wraps about every 49.7 days; subtraction in wrapping
/// arithmetic yields the correct elapsed milliseconds.
fn idle_seconds_from_ticks(last_input_tick: u32, now_tick: u32) -> u64 {
    let elapsed_ms = now_tick.wrapping_sub(last_input_tick);
    u64::from(elapsed_ms) / 1_000
}

#[cfg(target_os = "windows")]
pub(super) fn idle_seconds() -> Result<u64, String> {
    let mut info = LASTINPUTINFO {
        cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32,
        dwTime: 0,
    };
    // SAFETY: `info` is a valid LASTINPUTINFO with cbSize set.
    let ok = unsafe { GetLastInputInfo(&mut info) };
    if ok == FALSE {
        return Err("GetLastInputInfo failed".into());
    }
    // SAFETY: GetTickCount is a trivial kernel query.
    let now = unsafe { GetTickCount() };
    Ok(idle_seconds_from_ticks(info.dwTime, now))
}

#[cfg(target_os = "windows")]
pub(super) fn active_window_fullscreen() -> Result<bool, String> {
    // SAFETY: GetForegroundWindow returns a handle or null; no ownership.
    let hwnd = unsafe { GetForegroundWindow() };
    if hwnd.is_null() {
        return Ok(false);
    }
    // SAFETY: IsWindowVisible / IsIconic accept any HWND; null already excluded.
    if unsafe { IsWindowVisible(hwnd) } == FALSE {
        return Ok(false);
    }
    if unsafe { IsIconic(hwnd) } != FALSE {
        return Ok(false);
    }
    if is_unfocus_overlay_window(hwnd) {
        // Our own break cover must not suppress the next break.
        return Ok(false);
    }

    let window = window_rect(hwnd)?;
    let monitor = monitor_rect_for_window(hwnd)?;
    Ok(window_covers_monitor(window, monitor))
}

#[cfg(target_os = "windows")]
fn window_rect(hwnd: HWND) -> Result<ScreenRect, String> {
    let mut rect = RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    };
    // SAFETY: `rect` is a writable RECT for a valid HWND.
    let ok = unsafe { GetWindowRect(hwnd, &mut rect) };
    if ok == FALSE {
        return Err("GetWindowRect failed for the foreground window".into());
    }
    Ok(ScreenRect {
        left: rect.left,
        top: rect.top,
        right: rect.right,
        bottom: rect.bottom,
    })
}

#[cfg(target_os = "windows")]
fn monitor_rect_for_window(hwnd: HWND) -> Result<ScreenRect, String> {
    // SAFETY: MonitorFromWindow with DEFAULTTONEAREST always returns a monitor
    // for a real window on a desktop session.
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_null() {
        return Err("MonitorFromWindow returned no monitor".into());
    }
    let mut info = MONITORINFO {
        cbSize: std::mem::size_of::<MONITORINFO>() as u32,
        rcMonitor: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        rcWork: RECT {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        },
        dwFlags: 0,
    };
    // SAFETY: `info` has cbSize set; GetMonitorInfoW fills monitor geometry.
    let ok = unsafe { GetMonitorInfoW(monitor, &mut info) };
    if ok == FALSE {
        return Err("GetMonitorInfoW failed".into());
    }
    Ok(ScreenRect {
        left: info.rcMonitor.left,
        top: info.rcMonitor.top,
        right: info.rcMonitor.right,
        bottom: info.rcMonitor.bottom,
    })
}

#[cfg(target_os = "windows")]
fn is_unfocus_overlay_window(hwnd: HWND) -> bool {
    let mut buf = [0u16; 256];
    // SAFETY: buffer is valid for GetWindowTextW.
    let len = unsafe { GetWindowTextW(hwnd, buf.as_mut_ptr(), buf.len() as i32) };
    if len <= 0 {
        return false;
    }
    let title = String::from_utf16_lossy(&buf[..len as usize]);
    // Matches the product title used on overlay windows ("Unfocus eye break")
    // and the main window ("Unfocus"). Only the overlay must be ignored for
    // break suppression; treating the dashboard as non-fullscreen is also fine.
    title.to_ascii_lowercase().contains("unfocus eye break")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn idle_tick_subtraction_handles_wraparound() {
        // last near u32::MAX, now just after wrap: 1000 ms elapsed → 1 s.
        let last = u32::MAX - 500;
        let now = 499;
        assert_eq!(idle_seconds_from_ticks(last, now), 1);
        // 2000 ms before wrap + 1000 ms after = 3001 ms → 3 s.
        let last = u32::MAX - 2_000;
        let now = 1_000;
        assert_eq!(idle_seconds_from_ticks(last, now), 3);
    }

    #[test]
    fn idle_tick_subtraction_normal_elapsed() {
        assert_eq!(idle_seconds_from_ticks(1_000, 6_000), 5);
        assert_eq!(idle_seconds_from_ticks(0, 999), 0);
    }

    #[test]
    fn exact_monitor_coverage_is_fullscreen() {
        let monitor = ScreenRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let window = monitor;
        assert!(window_covers_monitor(window, monitor));
    }

    #[test]
    fn maximized_work_area_is_not_fullscreen() {
        // Work area leaves ~40px for the taskbar.
        let monitor = ScreenRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let maximized = ScreenRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1040,
        };
        assert!(!window_covers_monitor(maximized, monitor));
    }

    #[test]
    fn negative_origin_monitor_fullscreen() {
        let monitor = ScreenRect {
            left: -1920,
            top: 0,
            right: 0,
            bottom: 1080,
        };
        let window = monitor;
        assert!(window_covers_monitor(window, monitor));
        let offset = ScreenRect {
            left: -1910,
            top: 0,
            right: 10,
            bottom: 1080,
        };
        assert!(!window_covers_monitor(offset, monitor));
    }

    #[test]
    fn tolerance_allows_one_pixel_of_slack() {
        let monitor = ScreenRect {
            left: 100,
            top: 200,
            right: 2020,
            bottom: 1280,
        };
        let window = ScreenRect {
            left: 101,
            top: 199,
            right: 2021,
            bottom: 1281,
        };
        assert!(window_covers_monitor(window, monitor));
    }

    #[test]
    fn empty_or_inverted_rects_are_not_fullscreen() {
        let monitor = ScreenRect {
            left: 0,
            top: 0,
            right: 1920,
            bottom: 1080,
        };
        let empty = ScreenRect {
            left: 0,
            top: 0,
            right: 0,
            bottom: 0,
        };
        assert!(!window_covers_monitor(empty, monitor));
    }
}
