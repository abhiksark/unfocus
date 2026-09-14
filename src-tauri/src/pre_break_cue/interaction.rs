// src-tauri/src/pre_break_cue/interaction.rs

//! Keep only the small skip target interactive. Hit tests use window-local logical
//! points on both macOS and X11. No event monitor or input permission is needed.
use super::*;

#[derive(Debug, Default)]
pub(super) struct PointerState {
    enabled: bool,
    layout: Option<CueLayout>,
}

fn hits_skip(x: f64, y: f64, width: f64, layout: CueLayout) -> bool {
    let center = width - layout.shoulder_width - layout.wing_width / 2.0;
    // Matches the centered 31-point timing + 27-point button in the frontend.
    (center + 2.0..center + 29.0).contains(&x)
        && ((layout.height - 30.0) / 2.0..(layout.height + 30.0) / 2.0).contains(&y)
}

pub(super) fn set_layout(window: &WebviewWindow, layout: CueLayout) {
    if let Some(state) = window
        .state::<PreBreakCueController>()
        .state()
        .pointers
        .get(window.label())
    {
        state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .layout = Some(layout);
    }
}

pub(super) fn set_enabled(window: &WebviewWindow, enabled: bool) -> Result<(), String> {
    let controller = window.state::<PreBreakCueController>();
    let state = controller.state().pointers.get(window.label()).cloned();
    let Some(state) = state else {
        return if enabled {
            Err("cue pointer tracking has ended".into())
        } else {
            Ok(())
        };
    };
    state
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .enabled = enabled;
    if !enabled {
        set_click_through(window, true).map_err(|error| error.to_string())?;
    }
    Ok(())
}

fn set_click_through(window: &WebviewWindow, ignore: bool) -> Result<(), tauri::Error> {
    // GTK has no GdkWindow before first map; Tao's input-shape setter unwraps it.
    // Reveal applies click-through after show, before the next pointer poll.
    if cfg!(target_os = "macos") || window.is_visible()? {
        window.set_ignore_cursor_events(ignore)?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
fn pointer_hits_skip(window: &WebviewWindow, layout: CueLayout) -> Result<bool, String> {
    let panel = window
        .app_handle()
        .get_webview_panel(window.label())
        .map_err(|error| error.to_string())?;
    let native = panel.as_panel();
    let point = native.mouseLocationOutsideOfEventStream();
    Ok(native.isVisible() && hits_skip(point.x, point.y, native.frame().size.width, layout))
}

#[cfg(not(target_os = "macos"))]
fn pointer_hits_skip(window: &WebviewWindow, layout: CueLayout) -> Result<bool, tauri::Error> {
    if !window.is_visible()? {
        return Ok(false);
    }
    let point = window.cursor_position()?;
    let origin = window.inner_position()?;
    let scale = window.scale_factor()?;
    let width = f64::from(window.inner_size()?.width) / scale;
    Ok(hits_skip(
        (point.x - f64::from(origin.x)) / scale,
        (point.y - f64::from(origin.y)) / scale,
        width,
        layout,
    ))
}

pub(super) fn start(window: &WebviewWindow, cancelled: Arc<AtomicBool>) -> Result<(), String> {
    let pointer = Arc::new(Mutex::new(PointerState::default()));
    let app = window.app_handle().clone();
    let label = window.label().to_owned();
    let controller = window.state::<PreBreakCueController>().inner().clone();
    controller
        .state()
        .pointers
        .insert(label.clone(), Arc::clone(&pointer));
    std::thread::Builder::new()
        .name("unfocus-cue-pointer".into())
        .spawn(move || {
            loop {
                let (sender, receiver) = mpsc::sync_channel(1);
                let main_app = app.clone();
                let main_label = label.clone();
                let main_pointer = Arc::clone(&pointer);
                let main_cancelled = Arc::clone(&cancelled);
                if app
                    .run_on_main_thread(move || {
                        let Some(window) = main_app.get_webview_window(&main_label) else {
                            let _ = sender.send(false);
                            return;
                        };
                        let alive = !main_cancelled.load(Ordering::Acquire);
                        let state = main_pointer
                            .lock()
                            .unwrap_or_else(std::sync::PoisonError::into_inner);
                        let interactive = alive
                            && state.enabled
                            && state.layout.is_some_and(|layout| {
                                pointer_hits_skip(&window, layout).unwrap_or(false)
                            });
                        if set_click_through(&window, !interactive).is_err() {
                            let _ = window.hide();
                            request_cue_window_close(
                                &main_app,
                                main_label,
                                "cue pointer update failed",
                            );
                            let _ = sender.send(false);
                            return;
                        }
                        let _ = sender.send(alive);
                    })
                    .is_err()
                {
                    break;
                }
                // At most one main-thread job is outstanding; a blocked main loop
                // cannot accumulate pointer jobs. The poll ends with its cue.
                if receiver.recv().ok() != Some(true) {
                    break;
                }
                std::thread::sleep(Duration::from_millis(16));
            }
            controller.state().pointers.remove(&label);
        })
        .map_err(|error| format!("could not start cue pointer tracking: {error}"))?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn only_the_button_hits_in_notched_and_fallback_layouts() {
        let notch = CueLayout {
            is_notched: true,
            notch_width: 209.0,
            wing_width: 64.0,
            shoulder_width: 9.0,
            height: 38.0,
        };
        for (x, y, expected) in [
            (316.0, 4.0, true),
            (342.9, 33.9, true),
            (343.0, 20.0, false),
            (315.9, 20.0, false),
            (330.0, 34.0, false),
            (177.5, 19.0, false),
            (40.0, 19.0, false),
        ] {
            assert_eq!(hits_skip(x, y, 355.0, notch), expected);
        }
        let pill = CueLayout {
            is_notched: false,
            notch_width: 0.0,
            wing_width: 100.0,
            shoulder_width: 0.0,
            height: 36.0,
        };
        assert!(hits_skip(165.0, 18.0, 200.0, pill));
        assert!(!hits_skip(195.0, 18.0, 200.0, pill));
        assert!(!hits_skip(f64::NAN, 18.0, 200.0, pill));
    }
}
