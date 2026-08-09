#[cfg(target_os = "macos")]
use objc2_core_foundation::{CFDictionary, CFNumber, CFNumberType, CGPoint, CGRect, CGSize};
#[cfg(target_os = "macos")]
use objc2_core_graphics::{
    kCGWindowBounds, kCGWindowLayer, CGDirectDisplayID, CGDisplayBounds, CGError, CGEventSource,
    CGEventSourceStateID, CGEventType, CGGetActiveDisplayList,
    CGRectMakeWithDictionaryRepresentation, CGWindowListCopyWindowInfo, CGWindowListOption,
};
#[cfg(target_os = "macos")]
use std::ffi::c_void;

/// A rectangle in Quartz screen coordinates (points, top-left origin).
///
/// Only the macOS probe produces these, but the comparison logic below stays
/// platform-independent so it can be unit tested on any host.
#[derive(Debug, Clone, Copy, PartialEq)]
struct ScreenRect {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

/// Quartz reports geometry as floating point points, and a fullscreen window's
/// bounds are not always bit-identical to its display's. Allow a point of slack
/// rather than demanding exact equality.
const FULLSCREEN_TOLERANCE: f64 = 1.0;

/// True when `window` covers `display` edge to edge.
///
/// A macOS window that is merely zoomed still leaves the menu bar visible, so
/// its height falls short of the display and it is correctly not fullscreen.
fn window_covers_display(window: ScreenRect, display: ScreenRect) -> bool {
    (window.x - display.x).abs() <= FULLSCREEN_TOLERANCE
        && (window.y - display.y).abs() <= FULLSCREEN_TOLERANCE
        && (window.width - display.width).abs() <= FULLSCREEN_TOLERANCE
        && (window.height - display.height).abs() <= FULLSCREEN_TOLERANCE
}

/// True when `window` covers any one of `displays` entirely.
fn window_is_fullscreen(window: ScreenRect, displays: &[ScreenRect]) -> bool {
    displays
        .iter()
        .any(|display| window_covers_display(window, *display))
}

#[cfg(target_os = "macos")]
/// `kCGAnyInputEventType`, which Apple defines as `(uint32_t)~0`. The
/// generated bindings do not export the constant, so it is named here.
const ANY_INPUT_EVENT: CGEventType = CGEventType(u32::MAX);

#[cfg(target_os = "macos")]
/// Normal application windows sit on layer 0; menu bar, dock and overlay
/// windows sit above it. Only layer 0 can be the active window.
const NORMAL_WINDOW_LAYER: i64 = 0;

#[cfg(target_os = "macos")]
/// `kCGNullWindowID` — list windows relative to no particular window.
const NULL_WINDOW: u32 = 0;

#[cfg(target_os = "macos")]
pub(super) fn idle_seconds() -> Result<u64, String> {
    let seconds = CGEventSource::seconds_since_last_event_type(
        CGEventSourceStateID::HIDSystemState,
        ANY_INPUT_EVENT,
    );

    if !seconds.is_finite() || seconds < 0.0 {
        return Err(format!(
            "Quartz reported an implausible idle time: {seconds}"
        ));
    }

    // Rust saturates float-to-integer casts, and the value is already
    // known finite and non-negative.
    Ok(seconds as u64)
}

#[cfg(target_os = "macos")]
pub(super) fn active_window_fullscreen() -> Result<bool, String> {
    let displays = active_display_bounds()?;
    if displays.is_empty() {
        return Err("Quartz reported no active displays".into());
    }

    // No frontmost normal window at all (empty desktop) is not fullscreen.
    let Some(window) = frontmost_window_bounds()? else {
        return Ok(false);
    };

    Ok(window_is_fullscreen(window, &displays))
}

#[cfg(target_os = "macos")]
fn screen_rect(rect: CGRect) -> ScreenRect {
    ScreenRect {
        x: rect.origin.x,
        y: rect.origin.y,
        width: rect.size.width,
        height: rect.size.height,
    }
}

#[cfg(target_os = "macos")]
fn active_display_bounds() -> Result<Vec<ScreenRect>, String> {
    let mut count: u32 = 0;
    // SAFETY: a null list pointer asks Quartz for the count only.
    let status = unsafe { CGGetActiveDisplayList(0, std::ptr::null_mut(), &mut count) };
    if status != CGError::Success {
        return Err(format!("could not count Quartz displays: {status:?}"));
    }

    let mut ids: Vec<CGDirectDisplayID> = vec![0; count as usize];
    let mut filled: u32 = 0;
    // SAFETY: `ids` has room for exactly `count` identifiers.
    let status = unsafe { CGGetActiveDisplayList(count, ids.as_mut_ptr(), &mut filled) };
    if status != CGError::Success {
        return Err(format!("could not list Quartz displays: {status:?}"));
    }
    ids.truncate(filled as usize);

    Ok(ids
        .into_iter()
        .map(|id| screen_rect(CGDisplayBounds(id)))
        .collect())
}

/// Bounds of the frontmost normal window, or `None` when none is on screen.
///
/// `CGWindowListCopyWindowInfo` returns on-screen windows in front-to-back
/// order, so the first layer-0 entry is the active window.
#[cfg(target_os = "macos")]
fn frontmost_window_bounds() -> Result<Option<ScreenRect>, String> {
    let options =
        CGWindowListOption::OptionOnScreenOnly | CGWindowListOption::ExcludeDesktopElements;
    let windows = CGWindowListCopyWindowInfo(options, NULL_WINDOW)
        .ok_or("Quartz did not return a window list")?;

    for index in 0..windows.count() {
        // SAFETY: `index` is within the array we just received.
        let entry = unsafe { windows.value_at_index(index) };
        if entry.is_null() {
            continue;
        }
        // SAFETY: the window list holds CFDictionary entries.
        let entry = unsafe { &*entry.cast::<CFDictionary>() };

        let layer = dictionary_i64(entry, window_key_layer())
            .ok_or_else(|| format!("Quartz window {index} has no valid layer"))?;
        if layer != NORMAL_WINDOW_LAYER {
            continue;
        }

        let bounds = dictionary_rect(entry, window_key_bounds())
            .ok_or_else(|| format!("Quartz layer-0 window {index} has invalid bounds"))?;
        return Ok(Some(bounds));
    }

    Ok(None)
}

#[cfg(target_os = "macos")]
fn window_key_layer() -> *const c_void {
    // SAFETY: reading an immutable framework constant.
    std::ptr::from_ref(unsafe { kCGWindowLayer }).cast()
}

#[cfg(target_os = "macos")]
fn window_key_bounds() -> *const c_void {
    // SAFETY: reading an immutable framework constant.
    std::ptr::from_ref(unsafe { kCGWindowBounds }).cast()
}

#[cfg(target_os = "macos")]
fn dictionary_i64(entry: &CFDictionary, key: *const c_void) -> Option<i64> {
    // SAFETY: `key` is a valid CFString constant.
    let value = unsafe { entry.value(key) };
    if value.is_null() {
        return None;
    }

    // SAFETY: the layer entry is a CFNumber.
    let number = unsafe { &*value.cast::<CFNumber>() };
    let mut result: i64 = 0;
    // SAFETY: the out pointer matches the requested SInt64 type.
    let read = unsafe {
        number.value(
            CFNumberType::SInt64Type,
            std::ptr::from_mut(&mut result).cast(),
        )
    };

    read.then_some(result)
}

#[cfg(target_os = "macos")]
fn dictionary_rect(entry: &CFDictionary, key: *const c_void) -> Option<ScreenRect> {
    // SAFETY: `key` is a valid CFString constant.
    let value = unsafe { entry.value(key) };
    if value.is_null() {
        return None;
    }

    // SAFETY: the bounds entry is a CFDictionary rect representation.
    let bounds = unsafe { &*value.cast::<CFDictionary>() };
    let mut rect = CGRect {
        origin: CGPoint { x: 0.0, y: 0.0 },
        size: CGSize {
            width: 0.0,
            height: 0.0,
        },
    };
    // SAFETY: `rect` is a valid out pointer for the duration of the call.
    let read = unsafe {
        CGRectMakeWithDictionaryRepresentation(Some(bounds), std::ptr::from_mut(&mut rect))
    };

    read.then(|| screen_rect(rect))
}

#[cfg(test)]
mod tests {
    use super::*;

    const LAPTOP: ScreenRect = ScreenRect {
        x: 0.0,
        y: 0.0,
        width: 1710.0,
        height: 1112.0,
    };
    const EXTERNAL: ScreenRect = ScreenRect {
        x: 1710.0,
        y: 0.0,
        width: 2560.0,
        height: 1440.0,
    };

    #[test]
    fn a_window_filling_its_display_is_fullscreen() {
        assert!(window_is_fullscreen(LAPTOP, &[LAPTOP, EXTERNAL]));
    }

    #[test]
    fn a_zoomed_window_below_the_menu_bar_is_not_fullscreen() {
        // macOS keeps the menu bar visible for a merely zoomed window, so the
        // window starts lower and is shorter than the display.
        let zoomed = ScreenRect {
            x: 0.0,
            y: 25.0,
            width: 1710.0,
            height: 1087.0,
        };

        assert!(!window_is_fullscreen(zoomed, &[LAPTOP]));
    }

    #[test]
    fn fullscreen_is_detected_on_a_secondary_display() {
        assert!(window_is_fullscreen(EXTERNAL, &[LAPTOP, EXTERNAL]));
    }

    #[test]
    fn sub_pixel_differences_still_count_as_fullscreen() {
        let rounded = ScreenRect {
            x: 0.0,
            y: 0.0,
            width: 1709.6,
            height: 1112.4,
        };

        assert!(window_is_fullscreen(rounded, &[LAPTOP]));
    }

    #[test]
    fn a_window_matching_no_display_is_not_fullscreen() {
        let floating = ScreenRect {
            x: 400.0,
            y: 300.0,
            width: 900.0,
            height: 600.0,
        };

        assert!(!window_is_fullscreen(floating, &[LAPTOP, EXTERNAL]));
    }

    #[test]
    fn a_window_with_no_displays_to_match_is_not_fullscreen() {
        assert!(!window_is_fullscreen(LAPTOP, &[]));
    }
}
