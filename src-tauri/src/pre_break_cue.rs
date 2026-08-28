// src-tauri/src/pre_break_cue.rs

use crate::tray::{TrayPhase, TraySnapshot};
use std::{
    sync::{
        atomic::{AtomicBool, AtomicU64, Ordering},
        mpsc::{self, Receiver, TryRecvError},
        Arc,
    },
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tauri::{webview::PageLoadEvent, AppHandle, Manager, WebviewUrl, WebviewWindowBuilder};

const CUE_LEAD_MILLISECONDS: u64 = 60_000;
const CUE_WIDTH: f64 = 360.0;
const CUE_HEIGHT: f64 = 88.0;
const CUE_TOP_GAP: f64 = 12.0;
const CUE_PAGE_LOAD_TIMEOUT: Duration = Duration::from_secs(5);
const JAVASCRIPT_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;
static CUE_RUN_ID: AtomicU64 = AtomicU64::new(1);

#[derive(Debug, Clone, Copy, PartialEq)]
struct CueGeometry {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

fn cue_geometry(
    work_x: i32,
    work_y: i32,
    work_width: u32,
    work_height: u32,
    scale_factor: f64,
) -> Result<CueGeometry, String> {
    if !scale_factor.is_finite() || scale_factor <= 0.0 {
        return Err("primary monitor reported an invalid scale factor".into());
    }

    let work_width = f64::from(work_width) / scale_factor;
    let work_height = f64::from(work_height) / scale_factor;
    if work_width <= 0.0 || work_height <= CUE_TOP_GAP {
        return Err("primary monitor reported an unusable work area".into());
    }

    let width = work_width.min(CUE_WIDTH);
    let height = (work_height - CUE_TOP_GAP).min(CUE_HEIGHT);
    Ok(CueGeometry {
        x: f64::from(work_x) / scale_factor + (work_width - width) / 2.0,
        y: f64::from(work_y) / scale_factor + CUE_TOP_GAP,
        width,
        height,
    })
}

fn parse_canonical_u64(value: &str) -> Option<u64> {
    if value.is_empty()
        || !value.bytes().all(|byte| byte.is_ascii_digit())
        || (value.len() > 1 && value.starts_with('0'))
    {
        return None;
    }
    value.parse().ok()
}

fn cue_parameters_from_label(label: &str) -> Option<(u64, u64)> {
    let mut parts = label.split('-');
    if parts.next()? != "cue" {
        return None;
    }
    let run_id = parse_canonical_u64(parts.next()?)?;
    let deadline_ms = parse_canonical_u64(parts.next()?)?;
    if parts.next().is_some()
        || run_id == 0
        || run_id > JAVASCRIPT_MAX_SAFE_INTEGER
        || deadline_ms == 0
        || deadline_ms > JAVASCRIPT_MAX_SAFE_INTEGER
    {
        return None;
    }
    Some((run_id, deadline_ms))
}

fn next_cue_run_id() -> u64 {
    let current = CUE_RUN_ID.load(Ordering::Relaxed);
    let run_id = if (1..=JAVASCRIPT_MAX_SAFE_INTEGER).contains(&current) {
        current
    } else {
        1
    };
    CUE_RUN_ID.store(
        if run_id == JAVASCRIPT_MAX_SAFE_INTEGER {
            1
        } else {
            run_id + 1
        },
        Ordering::Relaxed,
    );
    run_id
}

fn cue_deadline_ms(wall_now: SystemTime, remaining_ms: u64) -> Result<u64, String> {
    wall_now
        .duration_since(UNIX_EPOCH)
        .map_err(|_| "system clock predates the Unix epoch".to_owned())?
        .as_millis()
        .try_into()
        .ok()
        .and_then(|now_ms: u64| now_ms.checked_add(remaining_ms))
        .filter(|deadline| (1..=JAVASCRIPT_MAX_SAFE_INTEGER).contains(deadline))
        .ok_or_else(|| "cue deadline exceeds JavaScript's safe-integer range".to_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CueReminderState {
    working: bool,
    remaining_ms: Option<u64>,
    revision: u64,
}

impl From<&TraySnapshot> for CueReminderState {
    fn from(snapshot: &TraySnapshot) -> Self {
        Self {
            working: snapshot.phase == TrayPhase::Working,
            remaining_ms: snapshot.remaining_milliseconds,
            revision: snapshot.state_revision,
        }
    }
}

impl CueReminderState {
    fn should_show(self) -> bool {
        self.working
            && self
                .remaining_ms
                .is_some_and(|remaining| (1..=CUE_LEAD_MILLISECONDS).contains(&remaining))
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CueDecision {
    None,
    Create,
    Close,
}

fn cue_decision(
    state: CueReminderState,
    attempted_revision: Option<u64>,
    occupied_revision: Option<u64>,
    cleanup_pending: bool,
) -> CueDecision {
    if let Some(occupied_revision) = occupied_revision {
        return if cleanup_pending || !state.should_show() || occupied_revision != state.revision {
            CueDecision::Close
        } else {
            CueDecision::None
        };
    }
    if state.should_show() && attempted_revision != Some(state.revision) {
        CueDecision::Create
    } else {
        CueDecision::None
    }
}

#[derive(Debug)]
struct CreatedCue {
    label: String,
    cancelled: Arc<AtomicBool>,
}

type CueCreationResult = Result<Option<CreatedCue>, String>;

#[derive(Debug)]
enum CueSlot {
    Pending {
        revision: u64,
        cancelled: Arc<AtomicBool>,
        receiver: Receiver<CueCreationResult>,
    },
    Active {
        revision: u64,
        cue: CreatedCue,
    },
}

impl CueSlot {
    fn revision(&self) -> u64 {
        match self {
            Self::Pending { revision, .. } | Self::Active { revision, .. } => *revision,
        }
    }

    fn cancel(&self) {
        match self {
            Self::Pending { cancelled, .. } => cancelled.store(true, Ordering::Release),
            Self::Active { cue, .. } => cue.cancelled.store(true, Ordering::Release),
        }
    }

    fn cleanup_pending(&self) -> bool {
        match self {
            Self::Pending { cancelled, .. } => cancelled.load(Ordering::Acquire),
            Self::Active { cue, .. } => cue.cancelled.load(Ordering::Acquire),
        }
    }
}

#[derive(Debug)]
pub(crate) struct PreBreakCue {
    enabled: bool,
    attempted_revision: Option<u64>,
    slot: Option<CueSlot>,
}

impl PreBreakCue {
    pub(crate) fn new(enabled: bool) -> Self {
        Self {
            enabled,
            attempted_revision: None,
            slot: None,
        }
    }

    pub(crate) fn reconcile(
        &mut self,
        app: &AppHandle,
        snapshot: &TraySnapshot,
        wall_now: SystemTime,
    ) {
        self.poll_creation(app);
        self.clear_absent_window(app);

        let state = CueReminderState::from(snapshot);
        let decision = cue_decision(
            state,
            self.attempted_revision,
            self.slot.as_ref().map(CueSlot::revision),
            self.slot.as_ref().is_some_and(CueSlot::cleanup_pending),
        );
        match decision {
            CueDecision::None => {}
            CueDecision::Close => self.close(app, "reminder state changed"),
            CueDecision::Create if self.enabled => {
                self.attempted_revision = Some(state.revision);
                let Some(remaining_ms) = state.remaining_ms else {
                    return;
                };
                match cue_deadline_ms(wall_now, remaining_ms)
                    .and_then(|deadline_ms| schedule_cue_creation(app, state.revision, deadline_ms))
                {
                    Ok(slot) => self.slot = Some(slot),
                    Err(error) => eprintln!("could not create pre-break cue: {error}"),
                }
            }
            CueDecision::Create => {}
        }
    }

    pub(crate) fn cancel(&mut self, app: &AppHandle, reason: &str) {
        self.close(app, reason);
    }

    fn poll_creation(&mut self, app: &AppHandle) {
        let result = match self.slot.as_ref() {
            Some(CueSlot::Pending { receiver, .. }) => receiver.try_recv(),
            _ => return,
        };
        match result {
            Ok(result) => {
                let Some(CueSlot::Pending {
                    revision,
                    cancelled,
                    ..
                }) = self.slot.take()
                else {
                    return;
                };
                match result {
                    Ok(Some(cue)) => {
                        self.slot = Some(CueSlot::Active { revision, cue });
                        if cancelled.load(Ordering::Acquire) {
                            self.close(app, "reminder state changed during cue startup");
                        }
                    }
                    Ok(None) => {}
                    Err(error) if !cancelled.load(Ordering::Acquire) => {
                        eprintln!("could not create pre-break cue: {error}");
                    }
                    Err(_) => {}
                }
            }
            Err(TryRecvError::Empty) => {}
            Err(TryRecvError::Disconnected) => {
                let cancelled = self.slot.as_ref().is_some_and(|slot| match slot {
                    CueSlot::Pending { cancelled, .. } => cancelled.load(Ordering::Acquire),
                    CueSlot::Active { .. } => false,
                });
                self.slot = None;
                if !cancelled {
                    eprintln!("could not create pre-break cue: startup worker stopped");
                }
            }
        }
    }

    fn clear_absent_window(&mut self, app: &AppHandle) {
        let absent = matches!(
            self.slot.as_ref(),
            Some(CueSlot::Active { cue, .. }) if app.get_webview_window(&cue.label).is_none()
        );
        if absent {
            self.slot = None;
        }
    }

    fn close(&mut self, app: &AppHandle, reason: &str) {
        let Some(slot) = self.slot.as_ref() else {
            return;
        };
        slot.cancel();
        let CueSlot::Active { cue, .. } = slot else {
            return;
        };
        let Some(window) = app.get_webview_window(&cue.label) else {
            self.slot = None;
            return;
        };
        if let Err(error) = window.close() {
            eprintln!(
                "could not close pre-break cue {} ({reason}): {error}",
                cue.label
            );
        }
    }
}

fn schedule_cue_creation(
    app: &AppHandle,
    revision: u64,
    deadline_ms: u64,
) -> Result<CueSlot, String> {
    let (sender, receiver) = mpsc::sync_channel(1);
    let cancelled = Arc::new(AtomicBool::new(false));
    let main_cancelled = Arc::clone(&cancelled);
    let main_app = app.clone();
    app.run_on_main_thread(move || {
        let result = create_cue_window(&main_app, deadline_ms, main_cancelled);
        let _ = sender.send(result);
    })
    .map_err(|error| format!("could not schedule cue window creation: {error}"))?;
    Ok(CueSlot::Pending {
        revision,
        cancelled,
        receiver,
    })
}

fn create_cue_window(
    app: &AppHandle,
    deadline_ms: u64,
    cancelled: Arc<AtomicBool>,
) -> CueCreationResult {
    if cancelled.load(Ordering::Acquire) {
        return Ok(None);
    }
    let monitor = app
        .primary_monitor()
        .map_err(|error| format!("could not read the primary monitor: {error}"))?
        .ok_or_else(|| "Tauri did not report a primary monitor".to_owned())?;
    let work_area = monitor.work_area();
    let geometry = cue_geometry(
        work_area.position.x,
        work_area.position.y,
        work_area.size.width,
        work_area.size.height,
        monitor.scale_factor(),
    )?;
    let run_id = next_cue_run_id();
    let label = format!("cue-{run_id}-{deadline_ms}");
    if cue_parameters_from_label(&label) != Some((run_id, deadline_ms)) {
        return Err("generated cue label was invalid".into());
    }

    let (ready_sender, ready_receiver) = mpsc::sync_channel(1);
    let window = WebviewWindowBuilder::new(app, &label, WebviewUrl::App("index.html".into()))
        .title("Unfocus eye break reminder")
        .position(geometry.x, geometry.y)
        .inner_size(geometry.width, geometry.height)
        // GTK otherwise promotes a non-resizable WebKit window to its 200 px natural height.
        // Equal constraints keep the native window fixed at the requested cue size.
        .min_inner_size(geometry.width, geometry.height)
        .max_inner_size(geometry.width, geometry.height)
        .decorations(false)
        .closable(false)
        .focusable(false)
        .always_on_top(true)
        .visible(false)
        .skip_taskbar(true)
        .shadow(false)
        .transparent(true)
        .background_color(tauri::webview::Color(0, 0, 0, 0))
        .on_page_load(move |_, payload| {
            if matches!(payload.event(), PageLoadEvent::Finished) {
                let _ = ready_sender.try_send(());
            }
        })
        .build()
        .map_err(|error| format!("could not build the cue window: {error}"))?;
    if cancelled.load(Ordering::Acquire) {
        let _ = window.close();
        return Ok(None);
    }

    let ready_app = app.clone();
    let ready_label = label.clone();
    let ready_cancelled = Arc::clone(&cancelled);
    std::thread::Builder::new()
        .name("unfocus-pre-break-cue-ready".into())
        .spawn(
            move || match ready_receiver.recv_timeout(CUE_PAGE_LOAD_TIMEOUT) {
                Ok(()) if !ready_cancelled.load(Ordering::Acquire) => {
                    if let Some(window) = ready_app.get_webview_window(&ready_label) {
                        if let Err(error) = window.show() {
                            eprintln!("could not reveal pre-break cue: {error}");
                            let _ = window.close();
                        } else if let Err(error) = window.set_ignore_cursor_events(true) {
                            eprintln!("could not make the pre-break cue click-through: {error}");
                            let _ = window.close();
                        }
                    }
                }
                Ok(()) => {
                    if let Some(window) = ready_app.get_webview_window(&ready_label) {
                        let _ = window.close();
                    }
                }
                Err(error) => {
                    if !ready_cancelled.load(Ordering::Acquire) {
                        eprintln!("pre-break cue did not finish loading: {error}");
                    }
                    if let Some(window) = ready_app.get_webview_window(&ready_label) {
                        let _ = window.close();
                    }
                }
            },
        )
        .map_err(|error| {
            let _ = window.close();
            format!("could not start the cue readiness worker: {error}")
        })?;

    Ok(Some(CreatedCue { label, cancelled }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn working(remaining_ms: u64, revision: u64) -> CueReminderState {
        CueReminderState {
            working: true,
            remaining_ms: Some(remaining_ms),
            revision,
        }
    }

    #[test]
    fn cue_starts_at_exactly_sixty_seconds_once_per_revision() {
        assert_eq!(
            cue_decision(working(60_000, 4), None, None, false),
            CueDecision::Create
        );
        assert_eq!(
            cue_decision(working(60_001, 4), None, None, false),
            CueDecision::None
        );
        assert_eq!(
            cue_decision(working(0, 4), None, None, false),
            CueDecision::None
        );
        assert_eq!(
            cue_decision(working(30_000, 4), Some(4), None, false),
            CueDecision::None
        );
    }

    #[test]
    fn startup_inside_warning_window_creates_a_cue() {
        assert_eq!(
            cue_decision(working(9_500, 0), None, None, false),
            CueDecision::Create
        );
    }

    #[test]
    fn non_working_and_revision_changes_close_the_current_cue() {
        for state in [
            CueReminderState {
                working: false,
                remaining_ms: None,
                revision: 8,
            },
            working(59_000, 9),
            working(61_000, 8),
            working(0, 8),
        ] {
            assert_eq!(
                cue_decision(state, Some(8), Some(8), false),
                CueDecision::Close
            );
        }
    }

    #[test]
    fn settings_changes_and_clock_rebases_close_the_old_revision() {
        assert_eq!(
            cue_decision(working(30_000, 21), Some(20), Some(20), false),
            CueDecision::Close
        );
        assert_eq!(
            cue_decision(working(30_000, 22), Some(21), Some(21), false),
            CueDecision::Close
        );
    }

    #[test]
    fn pause_manual_break_and_suspend_past_deadline_close_the_cue() {
        let left_working = CueReminderState {
            working: false,
            remaining_ms: None,
            revision: 31,
        };
        for _cause in ["pause", "manual break", "suspend past deadline"] {
            assert_eq!(
                cue_decision(left_working, Some(30), Some(30), false),
                CueDecision::Close
            );
        }
    }

    #[test]
    fn resume_never_reuses_the_previous_working_revision() {
        assert_eq!(
            cue_decision(working(60_000, 41), Some(40), Some(40), false),
            CueDecision::Close
        );
    }

    #[test]
    fn failed_creation_is_not_retried_and_cleanup_repeats_until_absent() {
        let state = working(20_000, 12);
        assert_eq!(
            cue_decision(state, Some(12), None, false),
            CueDecision::None
        );
        assert_eq!(
            cue_decision(state, Some(12), Some(11), false),
            CueDecision::Close
        );
        assert_eq!(
            cue_decision(state, Some(12), Some(11), false),
            CueDecision::Close
        );
        assert_eq!(
            cue_decision(state, Some(12), Some(12), true),
            CueDecision::Close
        );
    }

    #[test]
    fn cue_labels_are_strict_and_javascript_safe() {
        assert_eq!(
            cue_parameters_from_label("cue-7-1800000000000"),
            Some((7, 1_800_000_000_000))
        );
        for label in [
            "cue-garbage",
            "cue-01-1800000000000",
            "cue-0-1800000000000",
            "cue-1-0",
            "cue-1-1800000000000-extra",
            "cue-9007199254740992-1800000000000",
            "cue-1-9007199254740992",
        ] {
            assert_eq!(cue_parameters_from_label(label), None, "{label}");
        }
    }

    #[test]
    fn work_area_geometry_handles_scale_reserved_bars_negative_origins_and_narrow_displays() {
        assert_eq!(
            cue_geometry(0, 48, 2_560, 1_392, 2.0).unwrap(),
            CueGeometry {
                x: 460.0,
                y: 36.0,
                width: 360.0,
                height: 88.0,
            }
        );
        assert_eq!(
            cue_geometry(-1_920, 24, 1_920, 1_056, 1.0).unwrap().x,
            -1_140.0
        );
        assert_eq!(
            cue_geometry(0, 0, 300, 800, 1.0).unwrap(),
            CueGeometry {
                x: 0.0,
                y: 12.0,
                width: 300.0,
                height: 88.0,
            }
        );
    }
}
