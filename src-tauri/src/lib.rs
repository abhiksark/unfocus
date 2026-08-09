mod diagnostics;
mod probes;

use diagnostics::get_diagnostics;
use probes::{ProbeCache, ProbeSnapshot};
use serde::Serialize;
use std::{
    collections::VecDeque,
    io,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Arc, Mutex,
    },
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tauri::{
    image::Image,
    menu::{Menu, MenuItem},
    tray::TrayIconBuilder,
    AppHandle, Emitter, EventTarget, Manager, State, WebviewUrl, WebviewWindow,
    WebviewWindowBuilder,
};

static OVERLAY_RUN_ID: AtomicU64 = AtomicU64::new(1);
static OVERLAY_START_LOCK: Mutex<()> = Mutex::new(());
const WORK_INTERVAL: Duration = Duration::from_secs(20 * 60);
const BREAK_DURATION: Duration = Duration::from_secs(20);
const REMINDER_POLL_INTERVAL: Duration = Duration::from_millis(250);
const OVERLAY_TICK_INTERVAL: Duration = Duration::from_millis(250);
const OVERLAY_DISMISS_DELAY: Duration = Duration::from_millis(500);
const OVERLAY_COMPLETION_GRACE: Duration = Duration::from_millis(1_250);
const OVERLAY_COMMAND_CAPACITY: usize = 256;
const CLOSE_ORIGIN_CAPACITY: usize = 16;
const INTENTIONAL_CLOSE_SUPPRESSION: Duration = Duration::from_secs(5);
const MAX_OVERLAY_MONITORS: usize = 64;
const JAVASCRIPT_MAX_SAFE_INTEGER: u64 = 9_007_199_254_740_991;

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
    work_interval: Duration,
    break_duration: Duration,
}

impl ReminderTimer {
    fn new(now: Duration, work_interval: Duration, break_duration: Duration) -> Self {
        Self {
            phase: ReminderPhase::Working,
            phase_started_at: now,
            work_interval,
            break_duration,
        }
    }

    fn with_defaults(now: Duration) -> Self {
        Self::new(now, WORK_INTERVAL, BREAK_DURATION)
    }

    fn tick(&mut self, now: Duration) -> Option<ReminderTransition> {
        let elapsed = now.saturating_sub(self.phase_started_at);
        let phase_duration = match self.phase {
            ReminderPhase::Working => self.work_interval,
            ReminderPhase::Break => self.break_duration,
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

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayTickPayload {
    run_id: u64,
    remaining_ms: u64,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
struct OverlayRunPayload {
    run_id: u64,
}

#[derive(Debug)]
struct OverlayRunLifecycle {
    run_id: u64,
    prefix: String,
    completes_at: Instant,
    closes_at: Instant,
    dismiss_at: Option<Instant>,
    completed: bool,
    closing_emitted: bool,
}

#[derive(Debug, Default, PartialEq, Eq)]
struct OverlayLifecycleUpdate {
    emit_complete: bool,
    emit_closing: bool,
    close: bool,
}

impl OverlayRunLifecycle {
    fn automatic_closing_at(&self) -> Instant {
        self.closes_at
            .checked_sub(OVERLAY_DISMISS_DELAY)
            .unwrap_or(self.completes_at)
            .max(self.completes_at)
    }

    fn effective_close_at(&self) -> Instant {
        self.dismiss_at
            .map_or(self.closes_at, |dismiss_at| dismiss_at.min(self.closes_at))
    }

    fn begin_dismiss(&mut self, now: Instant) -> bool {
        let emit_closing = !self.closing_emitted;
        self.closing_emitted = true;
        let dismiss_at = now + OVERLAY_DISMISS_DELAY;
        self.dismiss_at = Some(
            self.dismiss_at
                .map_or(dismiss_at, |current| current.min(dismiss_at)),
        );
        emit_closing
    }

    fn advance(&mut self, now: Instant) -> OverlayLifecycleUpdate {
        let emit_complete =
            self.dismiss_at.is_none() && !self.completed && now >= self.completes_at;
        if emit_complete {
            self.completed = true;
        }

        let emit_closing = self.dismiss_at.is_none()
            && !self.closing_emitted
            && now >= self.automatic_closing_at();
        if emit_closing {
            self.closing_emitted = true;
        }

        OverlayLifecycleUpdate {
            emit_complete,
            emit_closing,
            close: now >= self.effective_close_at(),
        }
    }
}

#[derive(Debug)]
enum OverlayCommand {
    Register(OverlayRunLifecycle),
    Dismiss(u64),
    CancelAll,
    SiblingClosed(u64),
}

#[derive(Debug, Default)]
struct CloseOriginState {
    intentional: VecDeque<(u64, Instant)>,
    unexpected_pending: VecDeque<u64>,
}

impl CloseOriginState {
    fn prune_intentional(&mut self, now: Instant) {
        self.intentional.retain(|(_, marked_at)| {
            now.saturating_duration_since(*marked_at) <= INTENTIONAL_CLOSE_SUPPRESSION
        });
    }

    fn mark_intentional(&mut self, run_id: u64, now: Instant) {
        self.prune_intentional(now);
        self.intentional.retain(|(existing, _)| *existing != run_id);
        self.intentional.push_back((run_id, now));
        while self.intentional.len() > CLOSE_ORIGIN_CAPACITY {
            self.intentional.pop_front();
        }
        self.cancel_unexpected(run_id);
    }

    fn begin_unexpected(&mut self, run_id: u64, now: Instant) -> bool {
        self.prune_intentional(now);
        if self
            .intentional
            .iter()
            .any(|(intentional, _)| *intentional == run_id)
            || self.unexpected_pending.contains(&run_id)
        {
            return false;
        }

        self.unexpected_pending.push_back(run_id);
        while self.unexpected_pending.len() > CLOSE_ORIGIN_CAPACITY {
            self.unexpected_pending.pop_front();
        }
        true
    }

    fn cancel_unexpected(&mut self, run_id: u64) {
        self.unexpected_pending.retain(|pending| *pending != run_id);
    }
}

#[derive(Debug, Clone, Default)]
struct OverlayCloseOrigins {
    inner: Arc<Mutex<CloseOriginState>>,
}

impl OverlayCloseOrigins {
    fn mark_intentional(&self, run_id: u64) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .mark_intentional(run_id, Instant::now());
    }

    fn begin_unexpected(&self, run_id: u64) -> bool {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .begin_unexpected(run_id, Instant::now())
    }

    fn cancel_unexpected(&self, run_id: u64) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancel_unexpected(run_id);
    }
}

#[derive(Debug, Clone)]
struct OverlayController {
    sender: SyncSender<OverlayCommand>,
    close_origins: OverlayCloseOrigins,
}

impl OverlayController {
    fn start(app: AppHandle) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(OVERLAY_COMMAND_CAPACITY);
        let close_origins = OverlayCloseOrigins::default();
        let worker_close_origins = close_origins.clone();
        std::thread::Builder::new()
            .name("unfocus-overlays".into())
            .spawn(move || run_overlay_worker(app, receiver, worker_close_origins))?;
        Ok(Self {
            sender,
            close_origins,
        })
    }

    fn send(&self, command: OverlayCommand) -> Result<(), String> {
        self.sender.try_send(command).map_err(|error| match error {
            TrySendError::Full(_) => "overlay lifecycle queue is full".to_owned(),
            TrySendError::Disconnected(_) => "overlay lifecycle worker has stopped".to_owned(),
        })
    }

    fn register(&self, lifecycle: OverlayRunLifecycle) -> Result<(), String> {
        self.send(OverlayCommand::Register(lifecycle))
    }

    fn dismiss(&self, run_id: u64) -> Result<(), String> {
        self.send(OverlayCommand::Dismiss(run_id))
    }

    fn cancel_all(&self) -> Result<(), String> {
        self.send(OverlayCommand::CancelAll)
    }

    fn sibling_closed(&self, run_id: u64) {
        if !self.close_origins.begin_unexpected(run_id) {
            return;
        }
        match self.sender.try_send(OverlayCommand::SiblingClosed(run_id)) {
            Ok(()) => {}
            Err(TrySendError::Full(_)) => {
                self.close_origins.cancel_unexpected(run_id);
            }
            Err(TrySendError::Disconnected(_)) => {
                self.close_origins.cancel_unexpected(run_id);
                eprintln!("overlay lifecycle worker stopped before sibling cleanup");
            }
        }
    }

    fn close_windows(&self, app: &AppHandle, prefix: Option<&str>, reason: &str) {
        close_overlay_windows(app, &self.close_origins, prefix, reason);
    }
}

fn close_overlay_windows(
    app: &AppHandle,
    close_origins: &OverlayCloseOrigins,
    prefix: Option<&str>,
    reason: &str,
) {
    let windows: Vec<_> = app
        .webview_windows()
        .into_iter()
        .filter(|(label, _)| {
            label.starts_with("overlay-") && prefix.is_none_or(|prefix| label.starts_with(prefix))
        })
        .collect();

    // Mark every run before closing the first window: close events may be
    // delivered synchronously and must not look like unexpected WM teardown.
    for (label, _) in &windows {
        if let Some(run_id) = overlay_run_id_from_label(label) {
            close_origins.mark_intentional(run_id);
        }
    }

    for (label, window) in windows {
        eprintln!("closing overlay {label}: {reason}");
        if let Err(error) = window.close() {
            eprintln!("could not close overlay {label}: {error}");
        }
    }
}

fn emit_overlay_event<T: Clone + Serialize>(
    app: &AppHandle,
    prefix: &str,
    event: &str,
    payload: T,
) {
    // `Emitter::emit` delivers to every target regardless of the receiver, so
    // emitting from each window would broadcast the event once per window in
    // the run. Address the windows explicitly instead.
    for label in app.webview_windows().into_keys() {
        if label.starts_with(prefix) {
            if let Err(error) = app.emit_to(
                EventTarget::webview_window(label.clone()),
                event,
                payload.clone(),
            ) {
                eprintln!("could not emit {event} to overlay {label}: {error}");
            }
        }
    }
}

fn overlay_run_exists(app: &AppHandle, prefix: &str) -> bool {
    app.webview_windows()
        .keys()
        .any(|label| label.starts_with(prefix))
}

fn handle_overlay_command(
    app: &AppHandle,
    close_origins: &OverlayCloseOrigins,
    runs: &mut Vec<OverlayRunLifecycle>,
    command: OverlayCommand,
) {
    match command {
        OverlayCommand::Register(run) => {
            runs.retain(|existing| existing.run_id != run.run_id);
            runs.push(run);
        }
        OverlayCommand::Dismiss(run_id) => {
            if let Some(run) = runs.iter_mut().find(|run| run.run_id == run_id) {
                if run.begin_dismiss(Instant::now()) {
                    emit_overlay_event(
                        app,
                        &run.prefix,
                        "unfocus-overlay-closing",
                        OverlayRunPayload { run_id },
                    );
                }
            } else {
                // A previous native close may have failed after its lifecycle
                // entry expired. Recreate a short dismissal entry so Escape or
                // click can always retry while any window in the run exists.
                let prefix = format!("overlay-{run_id}-");
                if overlay_run_exists(app, &prefix) {
                    emit_overlay_event(
                        app,
                        &prefix,
                        "unfocus-overlay-closing",
                        OverlayRunPayload { run_id },
                    );
                    let dismiss_at = Instant::now() + OVERLAY_DISMISS_DELAY;
                    runs.push(OverlayRunLifecycle {
                        run_id,
                        prefix,
                        completes_at: Instant::now(),
                        closes_at: dismiss_at,
                        dismiss_at: Some(dismiss_at),
                        completed: true,
                        closing_emitted: true,
                    });
                }
            }
        }
        OverlayCommand::CancelAll => runs.clear(),
        OverlayCommand::SiblingClosed(run_id) => {
            let prefix = format!("overlay-{run_id}-");
            close_overlay_windows(
                app,
                close_origins,
                Some(&prefix),
                "one overlay in the run was closed",
            );
            runs.retain(|run| run.run_id != run_id);
        }
    }
}

fn process_overlay_runs(
    app: &AppHandle,
    close_origins: &OverlayCloseOrigins,
    runs: &mut Vec<OverlayRunLifecycle>,
    now: Instant,
) {
    let mut finished = Vec::new();

    for run in runs.iter_mut() {
        let update = run.advance(now);

        if update.emit_complete {
            emit_overlay_event(
                app,
                &run.prefix,
                "unfocus-overlay-tick",
                OverlayTickPayload {
                    run_id: run.run_id,
                    remaining_ms: 0,
                },
            );
            emit_overlay_event(
                app,
                &run.prefix,
                "unfocus-overlay-complete",
                OverlayRunPayload { run_id: run.run_id },
            );
        } else if !run.completed && run.dismiss_at.is_none() {
            let remaining_ms = run
                .completes_at
                .saturating_duration_since(now)
                .as_millis()
                .try_into()
                .unwrap_or(u64::MAX);
            emit_overlay_event(
                app,
                &run.prefix,
                "unfocus-overlay-tick",
                OverlayTickPayload {
                    run_id: run.run_id,
                    remaining_ms,
                },
            );
        }

        if update.emit_closing {
            emit_overlay_event(
                app,
                &run.prefix,
                "unfocus-overlay-closing",
                OverlayRunPayload { run_id: run.run_id },
            );
        }

        if update.close {
            let reason = if run.dismiss_at.is_some() {
                "overlay dismissed"
            } else {
                "overlay duration elapsed"
            };
            close_overlay_windows(app, close_origins, Some(&run.prefix), reason);
            finished.push(run.run_id);
        }
    }

    runs.retain(|run| !finished.contains(&run.run_id));
}

fn overlay_worker_timeout(runs: &[OverlayRunLifecycle], now: Instant) -> Duration {
    runs.iter().fold(OVERLAY_TICK_INTERVAL, |timeout, run| {
        let mut next = run.effective_close_at();
        if run.dismiss_at.is_none() {
            if !run.completed {
                next = next.min(run.completes_at);
            }
            if !run.closing_emitted {
                next = next.min(run.automatic_closing_at());
            }
        }
        timeout.min(next.saturating_duration_since(now))
    })
}

fn run_overlay_worker(
    app: AppHandle,
    receiver: Receiver<OverlayCommand>,
    close_origins: OverlayCloseOrigins,
) {
    let mut runs = Vec::new();

    loop {
        let timeout = overlay_worker_timeout(&runs, Instant::now());
        match receiver.recv_timeout(timeout) {
            Ok(command) => {
                handle_overlay_command(&app, &close_origins, &mut runs, command);
                for command in receiver.try_iter() {
                    handle_overlay_command(&app, &close_origins, &mut runs, command);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        process_overlay_runs(&app, &close_origins, &mut runs, Instant::now());
    }
}

fn begin_overlay_close(
    app: &AppHandle,
    controller: &OverlayController,
    run_id: u64,
) -> Result<(), String> {
    let prefix = format!("overlay-{run_id}-");
    if !overlay_run_exists(app, &prefix) {
        return Err("the overlay preview has already closed".into());
    }

    controller.dismiss(run_id)
}

fn bounded_overlay_duration(seconds: u64) -> u64 {
    seconds.clamp(3, 30)
}

fn unix_timestamp_ms() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis()
        .try_into()
        .unwrap_or(u64::MAX)
}

fn next_overlay_run_id() -> u64 {
    // Overlay starts are serialized by `OVERLAY_START_LOCK`, so this explicit
    // wrap keeps every label within JavaScript's canonical safe-integer range.
    let current = OVERLAY_RUN_ID.load(Ordering::Relaxed);
    let run_id = if (1..=JAVASCRIPT_MAX_SAFE_INTEGER).contains(&current) {
        current
    } else {
        1
    };
    let next = if run_id == JAVASCRIPT_MAX_SAFE_INTEGER {
        1
    } else {
        run_id + 1
    };
    OVERLAY_RUN_ID.store(next, Ordering::Relaxed);
    run_id
}

fn overlay_deadline_ms(duration_seconds: u64) -> Result<u64, String> {
    unix_timestamp_ms()
        .checked_add(duration_seconds.saturating_mul(1_000))
        .filter(|deadline| *deadline <= JAVASCRIPT_MAX_SAFE_INTEGER)
        .ok_or_else(|| "overlay deadline exceeds JavaScript's safe-integer range".to_owned())
}

fn overlay_label(
    run_id: u64,
    index: usize,
    total: usize,
    duration_seconds: u64,
    deadline_ms: u64,
) -> String {
    format!("overlay-{run_id}-{index}-{total}-{duration_seconds}-{deadline_ms}")
}

fn show_overlay_test_impl(
    app: &AppHandle,
    controller: &OverlayController,
    duration_seconds: u64,
) -> Result<usize, String> {
    let _start_guard = OVERLAY_START_LOCK
        .lock()
        .map_err(|_| "overlay preview start lock is poisoned".to_owned())?;

    controller.cancel_all()?;
    controller.close_windows(app, None, "superseded by a new preview");

    let monitors = app
        .available_monitors()
        .map_err(|error| format!("could not enumerate monitors: {error}"))?;
    if monitors.is_empty() {
        return Err("Tauri did not report any monitors".into());
    }
    if monitors.len() > MAX_OVERLAY_MONITORS {
        return Err(format!(
            "Tauri reported {} monitors; overlays support at most {MAX_OVERLAY_MONITORS}",
            monitors.len()
        ));
    }

    let duration_seconds = bounded_overlay_duration(duration_seconds);
    let run_id = next_overlay_run_id();
    let prefix = format!("overlay-{run_id}-");
    let total = monitors.len();
    let starts_at = Instant::now();
    let deadline_ms = overlay_deadline_ms(duration_seconds)?;
    let completes_at = starts_at + Duration::from_secs(duration_seconds);
    let closes_at = completes_at + OVERLAY_COMPLETION_GRACE;

    for (index, monitor) in monitors.iter().enumerate() {
        let scale_factor = monitor.scale_factor();
        let position = monitor.position();
        let size = monitor.size();
        let label = overlay_label(run_id, index, total, duration_seconds, deadline_ms);

        let build_result =
            WebviewWindowBuilder::new(app, label, WebviewUrl::App("index.html".into()))
                .title("Unfocus eye break")
                .position(
                    f64::from(position.x) / scale_factor,
                    f64::from(position.y) / scale_factor,
                )
                .inner_size(
                    f64::from(size.width) / scale_factor,
                    f64::from(size.height) / scale_factor,
                )
                .decorations(false)
                .resizable(false)
                .closable(false)
                .always_on_top(true)
                .skip_taskbar(true)
                .background_color(tauri::webview::Color(7, 19, 16, 255))
                .build();

        if let Err(error) = build_result {
            controller.close_windows(app, Some(&prefix), "preview startup failed");
            return Err(format!("could not create overlay {index}: {error}"));
        }
    }

    eprintln!("opened overlay run {run_id} on {total} display(s) for {duration_seconds} second(s)");

    if let Err(error) = controller.register(OverlayRunLifecycle {
        run_id,
        prefix: prefix.clone(),
        completes_at,
        closes_at,
        dismiss_at: None,
        completed: false,
        closing_emitted: false,
    }) {
        controller.close_windows(app, Some(&prefix), "overlay lifecycle registration failed");
        return Err(error);
    }

    Ok(total)
}

#[tauri::command]
fn show_overlay_test(
    window: WebviewWindow,
    controller: State<'_, OverlayController>,
    duration_seconds: u64,
) -> Result<usize, String> {
    authorize_main_caller(window.label())?;
    show_overlay_test_impl(window.app_handle(), &controller, duration_seconds)
}

#[tauri::command]
fn close_overlay_test(
    window: WebviewWindow,
    controller: State<'_, OverlayController>,
    run_id: u64,
) -> Result<(), String> {
    authorize_overlay_close_caller(window.label(), run_id)?;
    begin_overlay_close(window.app_handle(), &controller, run_id)
}

fn install_tray(app: &tauri::App) -> tauri::Result<()> {
    let open = MenuItem::with_id(app, "open", "Open Unfocus", true, None::<&str>)?;
    let overlay = MenuItem::with_id(
        app,
        "overlay",
        "Test overlays (8 seconds)",
        true,
        None::<&str>,
    )?;
    let quit = MenuItem::with_id(app, "quit", "Quit", true, None::<&str>)?;
    let menu = Menu::with_items(app, &[&open, &overlay, &quit])?;

    // macOS recolours a template image to match the menubar theme; every
    // other platform gets a fixed light glyph for their (dark) panels.
    #[cfg(target_os = "macos")]
    const TRAY_ICON: &[u8] = include_bytes!("../icons/tray/tray-template.png");
    #[cfg(not(target_os = "macos"))]
    const TRAY_ICON: &[u8] = include_bytes!("../icons/tray/tray-light.png");

    let icon = Image::from_bytes(TRAY_ICON)?;

    TrayIconBuilder::new()
        .icon(icon)
        .icon_as_template(cfg!(target_os = "macos"))
        .tooltip("Unfocus eye-break reminder")
        .menu(&menu)
        .show_menu_on_left_click(false)
        .on_menu_event(|app, event| match event.id.as_ref() {
            "open" => {
                if let Some(window) = app.get_webview_window("main") {
                    let _ = window.show();
                    let _ = window.set_focus();
                }
            }
            "overlay" => {
                let controller = app.state::<OverlayController>();
                if let Err(error) = show_overlay_test_impl(app, &controller, 8) {
                    eprintln!("overlay test failed: {error}");
                }
            }
            "quit" => app.exit(0),
            _ => {}
        })
        .build(app)?;

    Ok(())
}

fn environment_value(key: &str) -> Option<String> {
    std::env::var(key).ok().filter(|value| !value.is_empty())
}

fn schedule_automatic_overlay_test(app: &tauri::App, controller: OverlayController) {
    if environment_value("UNFOCUS_SPIKE_AUTO_OVERLAY").as_deref() != Some("1") {
        return;
    }

    let duration_seconds = environment_value("UNFOCUS_SPIKE_OVERLAY_SECONDS")
        .and_then(|value| value.parse().ok())
        .unwrap_or(8);
    let app = app.handle().clone();
    std::thread::spawn(move || {
        std::thread::sleep(Duration::from_secs(3));
        match show_overlay_test_impl(&app, &controller, duration_seconds) {
            Ok(count) => eprintln!("automatic overlay test opened {count} window(s)"),
            Err(error) => eprintln!("automatic overlay test failed: {error}"),
        }
    });
}

fn start_reminder_scheduler(
    app: AppHandle,
    probe_cache: ProbeCache,
    overlay_controller: OverlayController,
) -> io::Result<()> {
    std::thread::Builder::new()
        .name("unfocus-reminders".into())
        .spawn(move || {
            let started_at = Instant::now();
            let mut timer = ReminderTimer::with_defaults(Duration::ZERO);

            loop {
                std::thread::sleep(REMINDER_POLL_INTERVAL);
                if timer.tick(started_at.elapsed()) != Some(ReminderTransition::StartBreak) {
                    continue;
                }

                let probes = probe_cache.snapshot();
                if !should_present_break(&probes, BREAK_DURATION) {
                    if probes
                        .idle_seconds
                        .as_ref()
                        .is_ok_and(|seconds| *seconds >= BREAK_DURATION.as_secs())
                    {
                        eprintln!("scheduled break stayed hidden because the user is already idle");
                    } else {
                        eprintln!("scheduled break stayed hidden while fullscreen is active");
                    }
                    continue;
                }

                if let Err(error) =
                    show_overlay_test_impl(&app, &overlay_controller, BREAK_DURATION.as_secs())
                {
                    eprintln!("could not present scheduled break: {error}");
                }
            }
        })?;
    Ok(())
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

fn overlay_run_id_from_label(label: &str) -> Option<u64> {
    let mut parts = label.split('-');
    if parts.next()? != "overlay" {
        return None;
    }
    let run_id = parse_canonical_u64(parts.next()?)?;
    let index = parse_canonical_u64(parts.next()?)?;
    let total = parse_canonical_u64(parts.next()?)?;
    let duration = parse_canonical_u64(parts.next()?)?;
    let deadline = parse_canonical_u64(parts.next()?)?;
    if parts.next().is_some()
        || run_id == 0
        || run_id > JAVASCRIPT_MAX_SAFE_INTEGER
        || total == 0
        || total > MAX_OVERLAY_MONITORS as u64
        || index >= total
        || !(3..=30).contains(&duration)
        || deadline == 0
        || deadline > JAVASCRIPT_MAX_SAFE_INTEGER
    {
        return None;
    }

    Some(run_id)
}

fn authorize_main_caller(label: &str) -> Result<(), String> {
    if label == "main" {
        Ok(())
    } else {
        Err("this command is only available to the main window".into())
    }
}

fn authorize_overlay_close_caller(label: &str, requested_run_id: u64) -> Result<(), String> {
    match overlay_run_id_from_label(label) {
        Some(caller_run_id) if caller_run_id == requested_run_id => Ok(()),
        Some(_) => Err("an overlay can only close its own run".into()),
        None => Err("this command is only available to a valid overlay window".into()),
    }
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .setup(|app| {
            let probe_cache = ProbeCache::start()?;
            let overlay_controller = OverlayController::start(app.handle().clone())?;
            if !app.manage(probe_cache.clone()) {
                return Err(io::Error::other("probe cache was already managed").into());
            }
            if !app.manage(overlay_controller.clone()) {
                return Err(io::Error::other("overlay controller was already managed").into());
            }
            install_tray(app)?;
            schedule_automatic_overlay_test(app, overlay_controller.clone());
            start_reminder_scheduler(app.handle().clone(), probe_cache, overlay_controller)?;
            Ok(())
        })
        .on_window_event(|window, event| {
            if window.label() == "main" {
                if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                    api.prevent_close();
                    let _ = window.hide();
                }
            } else if matches!(
                event,
                tauri::WindowEvent::CloseRequested { .. } | tauri::WindowEvent::Destroyed
            ) {
                if let Some(run_id) = overlay_run_id_from_label(window.label()) {
                    window
                        .app_handle()
                        .state::<OverlayController>()
                        .sibling_closed(run_id);
                }
            }
        })
        .invoke_handler(tauri::generate_handler![
            get_diagnostics,
            show_overlay_test,
            close_overlay_test
        ])
        .run(tauri::generate_context!())
        .expect("error while running Unfocus");
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reminder_defaults_are_twenty_minutes_and_twenty_seconds() {
        let mut timer = ReminderTimer::with_defaults(Duration::ZERO);

        assert_eq!(timer.tick(WORK_INTERVAL - Duration::from_millis(1)), None);
        assert_eq!(
            timer.tick(WORK_INTERVAL),
            Some(ReminderTransition::StartBreak)
        );
        assert_eq!(
            timer.tick(WORK_INTERVAL + BREAK_DURATION),
            Some(ReminderTransition::EndBreak)
        );
        assert_eq!(
            timer.tick(WORK_INTERVAL + BREAK_DURATION + WORK_INTERVAL),
            Some(ReminderTransition::StartBreak)
        );
    }

    #[test]
    fn reminder_clock_is_injected_and_does_not_replay_missed_cycles() {
        let mut timer = ReminderTimer::new(
            Duration::from_secs(10),
            Duration::from_secs(60),
            Duration::from_secs(5),
        );

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
        let mut timer = ReminderTimer::new(
            Duration::from_secs(100),
            Duration::from_secs(60),
            Duration::from_secs(5),
        );

        assert_eq!(timer.tick(Duration::from_secs(90)), None);
        assert_eq!(timer.phase, ReminderPhase::Working);
    }

    #[test]
    fn automatic_overlay_completion_precedes_fade_and_teardown() {
        let started_at = Instant::now();
        let completes_at = started_at + Duration::from_secs(20);
        let closes_at = completes_at + OVERLAY_COMPLETION_GRACE;
        let closing_at = closes_at - OVERLAY_DISMISS_DELAY;
        let mut run = OverlayRunLifecycle {
            run_id: 1,
            prefix: "overlay-1-".into(),
            completes_at,
            closes_at,
            dismiss_at: None,
            completed: false,
            closing_emitted: false,
        };

        assert_eq!(
            run.advance(completes_at - Duration::from_millis(1)),
            OverlayLifecycleUpdate::default()
        );
        assert_eq!(
            run.advance(completes_at),
            OverlayLifecycleUpdate {
                emit_complete: true,
                emit_closing: false,
                close: false,
            }
        );
        assert_eq!(
            run.advance(closing_at - Duration::from_millis(1)),
            OverlayLifecycleUpdate::default()
        );
        assert_eq!(
            overlay_worker_timeout(&[run], closing_at - Duration::from_millis(100)),
            Duration::from_millis(100)
        );

        let mut run = OverlayRunLifecycle {
            run_id: 1,
            prefix: "overlay-1-".into(),
            completes_at,
            closes_at,
            dismiss_at: None,
            completed: true,
            closing_emitted: false,
        };
        assert_eq!(
            run.advance(closing_at),
            OverlayLifecycleUpdate {
                emit_complete: false,
                emit_closing: true,
                close: false,
            }
        );
        assert_eq!(
            run.advance(closes_at - Duration::from_millis(1)),
            OverlayLifecycleUpdate::default()
        );
        assert_eq!(
            run.advance(closes_at),
            OverlayLifecycleUpdate {
                emit_complete: false,
                emit_closing: false,
                close: true,
            }
        );
    }

    #[test]
    fn probes_only_control_break_presentation() {
        let active = ProbeSnapshot {
            idle_seconds: Ok(0),
            active_window_fullscreen: Ok(false),
        };
        let idle = ProbeSnapshot {
            idle_seconds: Ok(BREAK_DURATION.as_secs()),
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

        assert!(should_present_break(&active, BREAK_DURATION));
        assert!(!should_present_break(&idle, BREAK_DURATION));
        assert!(!should_present_break(&fullscreen, BREAK_DURATION));
        assert!(should_present_break(&failed, BREAK_DURATION));

        // Timer advancement has no probe input and is identical whether the
        // presentation decision above succeeds, suppresses, or errors.
        for probes in [&active, &idle, &fullscreen, &failed] {
            let mut timer = ReminderTimer::new(
                Duration::ZERO,
                Duration::from_secs(1),
                Duration::from_secs(1),
            );
            let _ = should_present_break(probes, Duration::from_secs(1));
            assert_eq!(
                timer.tick(Duration::from_secs(1)),
                Some(ReminderTransition::StartBreak)
            );
            assert_eq!(
                timer.tick(Duration::from_secs(2)),
                Some(ReminderTransition::EndBreak)
            );
        }
    }

    #[test]
    fn overlay_run_labels_are_parsed_strictly() {
        assert_eq!(
            overlay_run_id_from_label("overlay-7-1-2-20-1800000000000"),
            Some(7)
        );
        assert_eq!(overlay_run_id_from_label("overlay-garbage"), None);
        assert_eq!(
            overlay_run_id_from_label("overlay-07-1-2-20-1800000000000"),
            None
        );
        assert_eq!(
            overlay_run_id_from_label("overlay-7-2-2-20-1800000000000"),
            None
        );
        assert_eq!(
            overlay_run_id_from_label("overlay-7-1-2-20-1800000000000-extra"),
            None
        );
        assert_eq!(
            overlay_run_id_from_label("overlay-9007199254740992-1-2-20-1800000000000"),
            None
        );
        assert_eq!(
            overlay_run_id_from_label("overlay-7-1-65-20-1800000000000"),
            None
        );
    }

    #[test]
    fn command_callers_are_authorized_from_their_window_labels() {
        let overlay = "overlay-7-1-2-20-1800000000000";

        assert_eq!(authorize_main_caller("main"), Ok(()));
        assert!(authorize_main_caller(overlay).is_err());
        assert_eq!(authorize_overlay_close_caller(overlay, 7), Ok(()));
        assert!(authorize_overlay_close_caller(overlay, 8).is_err());
        assert!(authorize_overlay_close_caller("main", 7).is_err());
        assert!(authorize_overlay_close_caller("overlay-garbage", 7).is_err());
    }

    #[test]
    fn close_origins_deduplicate_expected_and_unexpected_teardown() {
        let now = Instant::now();
        let mut origins = CloseOriginState::default();

        assert!(origins.begin_unexpected(7, now));
        assert!(!origins.begin_unexpected(7, now));

        origins.mark_intentional(7, now);
        assert!(origins.unexpected_pending.is_empty());
        assert!(!origins.begin_unexpected(7, now + INTENTIONAL_CLOSE_SUPPRESSION));
        assert!(origins.begin_unexpected(
            7,
            now + INTENTIONAL_CLOSE_SUPPRESSION + Duration::from_millis(1)
        ));

        for run_id in 100..100 + CLOSE_ORIGIN_CAPACITY as u64 + 1 {
            origins.mark_intentional(run_id, now + Duration::from_secs(10));
        }
        assert_eq!(origins.intentional.len(), CLOSE_ORIGIN_CAPACITY);
    }

    #[test]
    fn overlay_test_duration_has_safe_bounds() {
        assert_eq!(bounded_overlay_duration(0), 3);
        assert_eq!(bounded_overlay_duration(8), 8);
        assert_eq!(bounded_overlay_duration(300), 30);
    }

    #[test]
    fn overlay_labels_share_an_absolute_deadline() {
        assert_eq!(
            overlay_label(7, 1, 2, 20, 1_800_000_000_000),
            "overlay-7-1-2-20-1800000000000"
        );
    }

    const TRAY_TEMPLATE_PNG: &[u8] = include_bytes!("../icons/tray/tray-template.png");
    const TRAY_LIGHT_PNG: &[u8] = include_bytes!("../icons/tray/tray-light.png");

    fn assert_tray_asset(bytes: &[u8], name: &str, channel_ok: impl Fn(u8) -> bool) {
        let image = tauri::image::Image::from_bytes(bytes)
            .unwrap_or_else(|error| panic!("{name} did not decode: {error}"));
        assert_eq!(
            (image.width(), image.height()),
            (32, 32),
            "{name} must be 32x32"
        );
        let rgba = image.rgba();
        assert_eq!(
            rgba.len(),
            32 * 32 * 4,
            "{name} has a truncated pixel buffer"
        );
        let mut visible = 0_usize;
        for pixel in rgba.chunks_exact(4) {
            if pixel[3] == 0 {
                continue;
            }
            visible += 1;
            assert!(
                pixel[0] == pixel[1] && pixel[1] == pixel[2] && channel_ok(pixel[0]),
                "{name} contains a non-monochrome pixel {pixel:?}"
            );
        }
        assert!(visible > 0, "{name} is fully transparent");
    }

    #[test]
    fn tray_template_asset_is_black_on_alpha() {
        assert_tray_asset(TRAY_TEMPLATE_PNG, "tray-template.png", |channel| {
            channel == 0
        });
    }

    #[test]
    fn tray_light_asset_is_white_on_alpha() {
        // The rasterizer's un-premultiply rounding can land a hair under 255
        // on anti-aliased edges; the template contract only needs "white".
        assert_tray_asset(TRAY_LIGHT_PNG, "tray-light.png", |channel| channel >= 250);
    }
}
