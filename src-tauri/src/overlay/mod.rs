mod labels;
mod lifecycle;
#[cfg(target_os = "macos")]
mod macos;
mod windows;

pub(crate) use labels::{
    overlay_run_id_from_label, MAX_OVERLAY_DURATION_SECONDS, MIN_OVERLAY_DURATION_SECONDS,
};
#[cfg(debug_assertions)]
pub(crate) use windows::schedule_automatic_overlay_test;
pub(crate) use windows::{show_overlay, show_overlay_if_idle};

fn prepare_unexpected_overlay_teardown(
    app: &tauri::AppHandle,
    window_label: &str,
) -> Result<(), String> {
    #[cfg(target_os = "macos")]
    {
        macos::prepare_unexpected_overlay_teardown(app, window_label)
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = (app, window_label);
        Ok(())
    }
}

use crate::{
    authorize_main_caller,
    tray::{TrayPhase, TrayStatus},
};
use labels::authorize_overlay_close_caller;
use lifecycle::{
    overlay_close_retry_delay, overlay_worker_timeout, OverlayRunLifecycle,
    OVERLAY_CLOSE_FAILURE_LIMIT, OVERLAY_DISMISS_DELAY,
};
use serde::Serialize;
use std::{
    collections::VecDeque,
    io,
    sync::{
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Arc, Mutex,
    },
    time::Instant,
};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use windows::{
    close_overlay_windows, close_overlay_windows_confirmed, emit_overlay_event,
    mark_overlay_scene_ready, overlay_run_exists,
};

const OVERLAY_COMMAND_CAPACITY: usize = 256;
const CLOSE_ORIGIN_CAPACITY: usize = 16;
#[cfg(target_os = "macos")]
const INTENTIONAL_CLOSE_SUPPRESSION: std::time::Duration =
    windows::MACOS_INTENTIONAL_CLOSE_SUPPRESSION;
#[cfg(not(target_os = "macos"))]
const INTENTIONAL_CLOSE_SUPPRESSION: std::time::Duration = std::time::Duration::from_secs(5);
const OVERLAY_RETRY_THREAD_NAME: &str = "unfocus-overlay-command-retry";

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
enum OverlayCommand {
    Register {
        run: OverlayRunLifecycle,
        startup: OverlayStartupReservation,
    },
    Dismiss(u64),
    CancelAll,
    SiblingClosed {
        run_id: u64,
        window_label: String,
    },
    RetryStartupCleanup {
        run_id: u64,
        prefix: String,
    },
}

#[derive(Debug)]
struct PendingCleanup {
    run_id: u64,
    target: String,
    attempts: usize,
    next_attempt_at: Option<Instant>,
}

fn enqueue_pending_cleanup(pending: &mut Vec<PendingCleanup>, cleanup: PendingCleanup) -> bool {
    if pending.iter().any(|item| item.run_id == cleanup.run_id) {
        false
    } else {
        pending.push(cleanup);
        true
    }
}

fn process_pending_cleanups<Cleanup>(
    pending: &mut Vec<PendingCleanup>,
    now: Instant,
    mut cleanup: Cleanup,
) -> (Vec<u64>, Vec<u64>)
where
    Cleanup: FnMut(&PendingCleanup) -> Result<(), String>,
{
    let mut completed = Vec::new();
    let mut exhausted = Vec::new();
    pending.retain_mut(|item| {
        if item.next_attempt_at.is_some_and(|retry_at| retry_at > now) {
            return true;
        }
        item.attempts += 1;
        match cleanup(item) {
            Ok(()) => {
                completed.push(item.run_id);
                false
            }
            Err(_) if item.attempts >= OVERLAY_CLOSE_FAILURE_LIMIT => {
                exhausted.push(item.run_id);
                false
            }
            Err(_) => {
                item.next_attempt_at = Some(now + overlay_close_retry_delay(item.attempts));
                true
            }
        }
    });
    (completed, exhausted)
}

#[derive(Debug, Default)]
struct CloseOriginState {
    intentional: VecDeque<(u64, Instant)>,
    unexpected_pending: VecDeque<u64>,
}

#[derive(Debug, Clone, Copy)]
pub(crate) enum OverlayCloseEvent {
    Requested,
    Destroyed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OverlayCloseDecision {
    prevent_close: bool,
    queue_cleanup: bool,
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

    fn decide_close(
        &mut self,
        run_id: u64,
        event: OverlayCloseEvent,
        now: Instant,
    ) -> OverlayCloseDecision {
        let queue_cleanup = self.begin_unexpected(run_id, now);
        OverlayCloseDecision {
            prevent_close: cfg!(target_os = "macos")
                && matches!(event, OverlayCloseEvent::Requested)
                && (queue_cleanup || self.unexpected_pending.contains(&run_id)),
            queue_cleanup,
        }
    }

    fn cancel_unexpected(&mut self, run_id: u64) {
        self.unexpected_pending.retain(|pending| *pending != run_id);
    }

    fn cancel_intentional(&mut self, run_id: u64) {
        self.intentional
            .retain(|(intentional, _)| *intentional != run_id);
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

    fn decide_close(&self, run_id: u64, event: OverlayCloseEvent) -> OverlayCloseDecision {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .decide_close(run_id, event, Instant::now())
    }

    fn cancel_unexpected(&self, run_id: u64) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancel_unexpected(run_id);
    }

    fn cancel_intentional(&self, run_id: u64) {
        self.inner
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .cancel_intentional(run_id);
    }
}

fn send_without_blocking<T, ReportFailure>(
    sender: &SyncSender<T>,
    command: T,
    retry_thread_name: &str,
    report_failure: ReportFailure,
) -> Result<(), String>
where
    T: Send + 'static,
    ReportFailure: Fn(String) + Send + Sync + 'static,
{
    match sender.try_send(command) {
        Ok(()) => Ok(()),
        Err(TrySendError::Disconnected(_)) => {
            let error = "overlay lifecycle worker has stopped".to_owned();
            report_failure(error.clone());
            Err(error)
        }
        Err(TrySendError::Full(command)) => {
            let retry_sender = sender.clone();
            let report_failure = Arc::new(report_failure);
            let worker_report_failure = Arc::clone(&report_failure);
            std::thread::Builder::new()
                .name(retry_thread_name.to_owned())
                .spawn(move || {
                    if retry_sender.send(command).is_err() {
                        worker_report_failure(
                            "overlay lifecycle worker stopped before deferred command delivery"
                                .to_owned(),
                        );
                    }
                })
                .map(|_| ())
                .map_err(|error| {
                    let error = format!("could not start overlay lifecycle retry sender: {error}");
                    report_failure(error.clone());
                    error
                })
        }
    }
}

#[derive(Debug, Default)]
struct OverlaySurfaceOccupancy {
    state: Mutex<OverlaySurfaceState>,
}

#[derive(Debug, Default)]
struct OverlaySurfaceState {
    active: bool,
    starting: usize,
}

impl OverlaySurfaceOccupancy {
    fn begin_startup(self: &Arc<Self>) -> OverlayStartupReservation {
        self.state().starting += 1;
        OverlayStartupReservation {
            occupancy: Arc::clone(self),
            starting: true,
        }
    }

    fn is_occupied(&self) -> bool {
        let state = self.state();
        state.active || state.starting > 0
    }

    fn set_active(&self, active: bool) {
        self.state().active = active;
    }

    fn promote_startup(&self) {
        let mut state = self.state();
        state.active = true;
        debug_assert!(state.starting > 0);
        state.starting = state.starting.saturating_sub(1);
    }

    fn release_startup(&self) {
        let mut state = self.state();
        debug_assert!(state.starting > 0);
        state.starting = state.starting.saturating_sub(1);
    }

    fn state(&self) -> std::sync::MutexGuard<'_, OverlaySurfaceState> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    #[cfg(test)]
    fn snapshot(&self) -> (usize, bool) {
        let state = self.state();
        (state.starting, state.active)
    }
}

#[derive(Debug)]
struct OverlayStartupReservation {
    occupancy: Arc<OverlaySurfaceOccupancy>,
    starting: bool,
}

impl OverlayStartupReservation {
    fn promote(mut self) {
        if self.starting {
            self.occupancy.promote_startup();
            self.starting = false;
        }
    }

    fn release_startup(&mut self) {
        if self.starting {
            self.occupancy.release_startup();
            self.starting = false;
        }
    }
}

impl Drop for OverlayStartupReservation {
    fn drop(&mut self) {
        self.release_startup();
    }
}

#[derive(Debug, Clone)]
pub(crate) struct OverlayController {
    sender: SyncSender<OverlayCommand>,
    close_origins: OverlayCloseOrigins,
    occupancy: Arc<OverlaySurfaceOccupancy>,
}

impl OverlayController {
    pub(crate) fn start(app: AppHandle) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(OVERLAY_COMMAND_CAPACITY);
        let close_origins = OverlayCloseOrigins::default();
        let worker_close_origins = close_origins.clone();
        let occupancy = Arc::new(OverlaySurfaceOccupancy::default());
        let worker_occupancy = Arc::clone(&occupancy);
        std::thread::Builder::new()
            .name("unfocus-overlays".into())
            .spawn(move || {
                run_overlay_worker(app, receiver, worker_close_origins, worker_occupancy);
            })?;
        Ok(Self {
            sender,
            close_origins,
            occupancy,
        })
    }

    pub(crate) fn has_active_run(&self) -> bool {
        self.occupancy.is_occupied()
    }

    fn begin_startup(&self) -> OverlayStartupReservation {
        self.occupancy.begin_startup()
    }

    fn send(&self, command: OverlayCommand) -> Result<(), String> {
        self.sender.try_send(command).map_err(|error| match error {
            TrySendError::Full(_) => "overlay lifecycle queue is full".to_owned(),
            TrySendError::Disconnected(_) => "overlay lifecycle worker has stopped".to_owned(),
        })
    }

    fn register(
        &self,
        lifecycle: OverlayRunLifecycle,
        startup: OverlayStartupReservation,
    ) -> Result<(), String> {
        self.send(OverlayCommand::Register {
            run: lifecycle,
            startup,
        })
    }

    fn dismiss(&self, run_id: u64) -> Result<(), String> {
        self.send(OverlayCommand::Dismiss(run_id))
    }

    fn cancel_all(&self) -> Result<(), String> {
        self.send(OverlayCommand::CancelAll)
    }

    pub(super) fn retain_startup_cleanup(&self, run_id: u64, prefix: String) -> Result<(), String> {
        send_without_blocking(
            &self.sender,
            OverlayCommand::RetryStartupCleanup { run_id, prefix },
            OVERLAY_RETRY_THREAD_NAME,
            move |error| {
                eprintln!("could not retain startup cleanup for overlay run {run_id}: {error}");
            },
        )
    }

    pub(crate) fn sibling_closed(
        &self,
        run_id: u64,
        window_label: String,
        event: OverlayCloseEvent,
    ) -> bool {
        // Window events run on the application thread. Decide the close origin
        // and interception here; native cleanup belongs to the worker.
        let decision = self.close_origins.decide_close(run_id, event);
        if !decision.queue_cleanup {
            return decision.prevent_close;
        }
        let failed_close_origins = self.close_origins.clone();
        let _ = send_without_blocking(
            &self.sender,
            OverlayCommand::SiblingClosed {
                run_id,
                window_label,
            },
            OVERLAY_RETRY_THREAD_NAME,
            move |error| {
                failed_close_origins.cancel_unexpected(run_id);
                eprintln!("{error}");
            },
        );
        decision.prevent_close
    }

    fn close_windows(
        &self,
        app: &AppHandle,
        prefix: Option<&str>,
        reason: &str,
    ) -> Result<(), String> {
        close_overlay_windows_confirmed(app, &self.close_origins, prefix, reason)
    }
}

fn cleanup_before_sibling_close<Cleanup, ReportFailure, CloseSiblings>(
    cleanup: Cleanup,
    mut report_failure: ReportFailure,
    close_siblings: CloseSiblings,
) -> Result<(), String>
where
    Cleanup: FnOnce() -> Result<(), String>,
    ReportFailure: FnMut(&str),
    CloseSiblings: FnOnce() -> Result<(), String>,
{
    if let Err(error) = cleanup() {
        report_failure(&error);
        return Err(error);
    }

    match close_siblings() {
        Ok(()) => Ok(()),
        Err(error) => {
            report_failure(&error);
            Err(format!("sibling cleanup failed: {error}"))
        }
    }
}

fn register_overlay_run(
    runs: &mut Vec<OverlayRunLifecycle>,
    run: OverlayRunLifecycle,
    startup: OverlayStartupReservation,
    run_exists: bool,
) {
    if !run_exists {
        runs.retain(|existing| existing.run_id != run.run_id);
        return;
    }

    // A revealed overlay can accept dismissal before startup registers it.
    // Keep that lifecycle's fade, emitted events, and close retry state intact.
    if !runs.iter().any(|existing| existing.run_id == run.run_id) {
        runs.push(run);
    }
    startup.promote();
}

fn handle_overlay_command(
    app: &AppHandle,
    runs: &mut Vec<OverlayRunLifecycle>,
    pending_cleanups: &mut Vec<PendingCleanup>,
    pending_startup_cleanups: &mut Vec<PendingCleanup>,
    command: OverlayCommand,
) {
    match command {
        OverlayCommand::Register { run, startup } => {
            let run_exists = overlay_run_exists(app, &run.prefix);
            register_overlay_run(runs, run, startup, run_exists);
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
                        next_close_attempt_at: None,
                        close_failures: 0,
                        completed: true,
                        closing_emitted: true,
                    });
                }
            }
        }
        OverlayCommand::CancelAll => runs.clear(),
        OverlayCommand::SiblingClosed {
            run_id,
            window_label,
        } => {
            enqueue_pending_cleanup(
                pending_cleanups,
                PendingCleanup {
                    run_id,
                    target: window_label,
                    attempts: 0,
                    next_attempt_at: None,
                },
            );
        }
        OverlayCommand::RetryStartupCleanup { run_id, prefix } => {
            enqueue_pending_cleanup(
                pending_startup_cleanups,
                PendingCleanup {
                    run_id,
                    target: prefix,
                    attempts: 0,
                    next_attempt_at: None,
                },
            );
        }
    }
}

fn process_startup_overlay_cleanups(
    app: &AppHandle,
    close_origins: &OverlayCloseOrigins,
    pending_cleanups: &mut Vec<PendingCleanup>,
) {
    let (_, exhausted) = process_pending_cleanups(pending_cleanups, Instant::now(), |pending| {
        let result = close_overlay_windows_confirmed(
            app,
            close_origins,
            Some(&pending.target),
            "retrying failed overlay startup cleanup",
        );
        if let Err(error) = &result {
            let disposition = if pending.attempts >= OVERLAY_CLOSE_FAILURE_LIMIT {
                "abandoned"
            } else {
                "will be retried"
            };
            eprintln!(
                "overlay startup cleanup for run {} {disposition}: {error}",
                pending.run_id
            );
        }
        result
    });
    for run_id in exhausted {
        close_origins.cancel_intentional(run_id);
        eprintln!(
            "overlay startup cleanup for run {run_id} stopped after {OVERLAY_CLOSE_FAILURE_LIMIT} failed attempts"
        );
    }
}

fn process_unexpected_overlay_cleanups(
    app: &AppHandle,
    close_origins: &OverlayCloseOrigins,
    runs: &mut Vec<OverlayRunLifecycle>,
    pending_cleanups: &mut Vec<PendingCleanup>,
) {
    let (completed, exhausted) =
        process_pending_cleanups(pending_cleanups, Instant::now(), |pending| {
            let prefix = format!("overlay-{}-", pending.run_id);
            cleanup_before_sibling_close(
                || prepare_unexpected_overlay_teardown(app, &pending.target),
                |error| {
                    eprintln!(
                        "overlay cleanup after unexpected close of {} failed: {error}",
                        pending.target
                    );
                },
                || {
                    close_overlay_windows_confirmed(
                        app,
                        close_origins,
                        Some(&prefix),
                        "one overlay in the run was closed",
                    )
                },
            )
        });

    for run_id in completed.into_iter().chain(exhausted.iter().copied()) {
        close_origins.cancel_unexpected(run_id);
        runs.retain(|run| run.run_id != run_id);
    }
    for run_id in exhausted {
        close_origins.cancel_intentional(run_id);
        eprintln!(
            "unexpected overlay cleanup for run {run_id} stopped after {OVERLAY_CLOSE_FAILURE_LIMIT} failed attempts"
        );
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
            match close_overlay_windows(app, close_origins, Some(&run.prefix), reason) {
                Ok(()) => finished.push(run.run_id),
                Err(error) => {
                    if run.defer_close_retry(Instant::now()) {
                        eprintln!(
                            "overlay run {} teardown will be retried: {error}",
                            run.run_id
                        );
                    } else {
                        finished.push(run.run_id);
                        close_origins.cancel_intentional(run.run_id);
                        eprintln!(
                            "overlay run {} teardown stopped after {OVERLAY_CLOSE_FAILURE_LIMIT} failed attempts: {error}",
                            run.run_id
                        );
                    }
                }
            }
        }
    }

    runs.retain(|run| !finished.contains(&run.run_id));
}

fn run_overlay_worker(
    app: AppHandle,
    receiver: Receiver<OverlayCommand>,
    close_origins: OverlayCloseOrigins,
    occupancy: Arc<OverlaySurfaceOccupancy>,
) {
    let mut runs = Vec::new();
    let mut pending_cleanups = Vec::new();
    let mut pending_startup_cleanups = Vec::new();

    loop {
        let timeout = overlay_worker_timeout(&runs, Instant::now());
        match receiver.recv_timeout(timeout) {
            Ok(command) => {
                handle_overlay_command(
                    &app,
                    &mut runs,
                    &mut pending_cleanups,
                    &mut pending_startup_cleanups,
                    command,
                );
                for command in receiver.try_iter() {
                    handle_overlay_command(
                        &app,
                        &mut runs,
                        &mut pending_cleanups,
                        &mut pending_startup_cleanups,
                        command,
                    );
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                occupancy.set_active(false);
                break;
            }
        }

        occupancy.set_active(
            !runs.is_empty()
                || !pending_cleanups.is_empty()
                || !pending_startup_cleanups.is_empty(),
        );
        process_startup_overlay_cleanups(&app, &close_origins, &mut pending_startup_cleanups);
        process_unexpected_overlay_cleanups(&app, &close_origins, &mut runs, &mut pending_cleanups);
        process_overlay_runs(&app, &close_origins, &mut runs, Instant::now());
        occupancy.set_active(
            !runs.is_empty()
                || !pending_cleanups.is_empty()
                || !pending_startup_cleanups.is_empty(),
        );
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

fn ensure_overlay_preview_available(phase: TrayPhase, overlay_active: bool) -> Result<(), String> {
    if phase == TrayPhase::Unavailable {
        return Err("break preview is unavailable until reminder timing is available".into());
    }
    if overlay_active {
        return Err("another break or preview is already active".into());
    }
    Ok(())
}

#[tauri::command]
pub(crate) async fn show_overlay_test(
    window: WebviewWindow,
    controller: State<'_, OverlayController>,
    tray_status: State<'_, TrayStatus>,
    duration_seconds: u64,
) -> Result<usize, String> {
    authorize_main_caller(window.label())?;
    let snapshot = tray_status.current();
    ensure_overlay_preview_available(snapshot.phase, snapshot.overlay_active)?;
    let app = window.app_handle().clone();
    let controller = controller.inner().clone();
    tauri::async_runtime::spawn_blocking(move || {
        show_overlay_if_idle(&app, &controller, duration_seconds)
    })
    .await
    .map_err(|error| format!("overlay preview task failed: {error}"))?
}

#[tauri::command]
pub(crate) fn close_overlay_test(
    window: WebviewWindow,
    controller: State<'_, OverlayController>,
    run_id: u64,
) -> Result<(), String> {
    authorize_overlay_close_caller(window.label(), run_id)?;
    begin_overlay_close(window.app_handle(), &controller, run_id)
}

#[tauri::command]
pub(crate) fn overlay_scene_ready(window: WebviewWindow) -> Result<(), String> {
    if overlay_run_id_from_label(window.label()).is_none() {
        return Err("this command is only available to a valid overlay window".into());
    }
    mark_overlay_scene_ready(window.label());
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        cell::RefCell,
        sync::{Arc, Mutex},
        time::Duration,
    };

    fn registration_run(run_id: u64, starts_at: Instant) -> OverlayRunLifecycle {
        let completes_at = starts_at + Duration::from_secs(20);
        OverlayRunLifecycle {
            run_id,
            prefix: format!("overlay-{run_id}-"),
            completes_at,
            closes_at: completes_at + lifecycle::OVERLAY_COMPLETION_GRACE,
            dismiss_at: None,
            next_close_attempt_at: None,
            close_failures: 0,
            completed: false,
            closing_emitted: false,
        }
    }

    #[test]
    fn registration_preserves_a_dismissal_accepted_before_registration() {
        let now = Instant::now();
        let occupancy = Arc::new(OverlaySurfaceOccupancy::default());
        let mut pending = registration_run(7, now);
        assert!(pending.begin_dismiss(now));
        let mut runs = vec![pending];

        register_overlay_run(
            &mut runs,
            registration_run(7, now),
            occupancy.begin_startup(),
            true,
        );

        assert_eq!(runs.len(), 1);
        assert_eq!(occupancy.snapshot(), (0, true));
        assert_eq!(
            runs[0].advance(now + OVERLAY_DISMISS_DELAY),
            lifecycle::OverlayLifecycleUpdate {
                emit_complete: false,
                emit_closing: false,
                close: true,
            }
        );
    }

    #[test]
    fn registration_preserves_pending_close_retry_and_emitted_events() {
        let now = Instant::now();
        let occupancy = Arc::new(OverlaySurfaceOccupancy::default());
        let mut pending = registration_run(7, now);
        pending.completed = true;
        pending.begin_dismiss(now);
        let dismissed_at = now + OVERLAY_DISMISS_DELAY;
        assert!(pending.defer_close_retry(dismissed_at));
        let mut runs = vec![pending];

        register_overlay_run(
            &mut runs,
            registration_run(7, now),
            occupancy.begin_startup(),
            true,
        );

        assert_eq!(runs[0].close_failures, 1);
        assert!(runs[0].completed);
        assert!(runs[0].closing_emitted);
        assert!(!runs[0].advance(dismissed_at).close);
        assert!(
            runs[0]
                .advance(dismissed_at + overlay_close_retry_delay(1))
                .close
        );
    }

    #[test]
    fn registration_of_another_run_preserves_its_original_deadlines() {
        let now = Instant::now();
        let occupancy = Arc::new(OverlaySurfaceOccupancy::default());
        let mut pending = registration_run(7, now);
        pending.begin_dismiss(now);
        let mut runs = vec![pending];
        let new_run = registration_run(8, now + Duration::from_secs(3));
        let completes_at = new_run.completes_at;
        let closes_at = new_run.closes_at;

        register_overlay_run(&mut runs, new_run, occupancy.begin_startup(), true);

        assert_eq!(runs.len(), 2);
        assert!(runs[0].advance(now + OVERLAY_DISMISS_DELAY).close);
        assert_eq!(runs[1].run_id, 8);
        assert_eq!(runs[1].completes_at, completes_at);
        assert_eq!(runs[1].closes_at, closes_at);
        assert_eq!(runs[1].dismiss_at, None);
        assert!(!runs[1].advance(now + OVERLAY_DISMISS_DELAY).close);
        assert!(runs[1].advance(completes_at).emit_complete);
        assert!(runs[1].advance(closes_at).close);
    }

    #[test]
    fn registration_after_windows_close_releases_startup_without_resurrection() {
        let now = Instant::now();
        let occupancy = Arc::new(OverlaySurfaceOccupancy::default());
        let mut runs = Vec::new();
        let startup = occupancy.begin_startup();
        assert!(occupancy.is_occupied());

        register_overlay_run(&mut runs, registration_run(7, now), startup, false);

        assert!(runs.is_empty());
        assert_eq!(occupancy.snapshot(), (0, false));
    }

    #[test]
    fn registration_of_a_closed_run_removes_only_its_stale_lifecycle() {
        let now = Instant::now();
        let occupancy = Arc::new(OverlaySurfaceOccupancy::default());
        let mut pending = registration_run(7, now);
        pending.begin_dismiss(now);
        let mut runs = vec![pending, registration_run(8, now)];

        register_overlay_run(
            &mut runs,
            registration_run(7, now),
            occupancy.begin_startup(),
            false,
        );

        assert_eq!(runs.len(), 1);
        assert_eq!(runs[0].run_id, 8);
        assert_eq!(runs[0].dismiss_at, None);
        assert_eq!(occupancy.snapshot().0, 0);
    }

    #[test]
    fn overlay_startup_counts_as_occupied_before_lifecycle_registration() {
        let occupancy = Arc::new(OverlaySurfaceOccupancy::default());

        let startup = occupancy.begin_startup();

        assert!(occupancy.is_occupied());
        startup.promote();
        assert!(occupancy.is_occupied());
        occupancy.set_active(false);
        assert!(!occupancy.is_occupied());
    }

    #[test]
    fn startup_promotion_is_one_linearized_occupancy_transition() {
        let occupancy = Arc::new(OverlaySurfaceOccupancy::default());
        let startup = occupancy.begin_startup();
        let state_guard = occupancy.state();
        let reader_occupancy = Arc::clone(&occupancy);
        let (ready_sender, ready_receiver) = mpsc::sync_channel(2);
        let reader_ready = ready_sender.clone();

        std::thread::scope(|scope| {
            let promoter = scope.spawn(move || {
                ready_sender.send(()).unwrap();
                startup.promote();
            });
            let reader = scope.spawn(move || {
                reader_ready.send(()).unwrap();
                reader_occupancy.is_occupied()
            });
            ready_receiver.recv().unwrap();
            ready_receiver.recv().unwrap();
            drop(state_guard);

            promoter.join().unwrap();
            assert!(reader.join().unwrap());
        });

        assert_eq!(occupancy.snapshot(), (0, true));
        assert!(occupancy.is_occupied());
    }

    #[test]
    fn preview_gate_distinguishes_unavailable_timing_from_an_active_overlay() {
        assert_eq!(
            ensure_overlay_preview_available(TrayPhase::Unavailable, false),
            Err("break preview is unavailable until reminder timing is available".into())
        );
        assert_eq!(
            ensure_overlay_preview_available(TrayPhase::Working, true),
            Err("another break or preview is already active".into())
        );
        assert_eq!(
            ensure_overlay_preview_available(TrayPhase::Working, false),
            Ok(())
        );
    }

    #[test]
    fn a_full_lifecycle_queue_delivers_the_command_from_a_background_sender() {
        let (sender, receiver) = mpsc::sync_channel(1);
        sender.send("already queued").unwrap();
        let failures = Arc::new(Mutex::new(Vec::new()));
        let reported_failures = Arc::clone(&failures);

        send_without_blocking(
            &sender,
            "sibling closed",
            "unfocus-overlay-command-retry",
            move |error| {
                reported_failures
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(error);
            },
        )
        .unwrap();

        assert_eq!(receiver.recv().unwrap(), "already queued");
        assert_eq!(
            receiver.recv_timeout(Duration::from_secs(1)).unwrap(),
            "sibling closed"
        );
        assert!(failures
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
            .is_empty());
    }

    #[test]
    fn a_disconnected_lifecycle_queue_reports_terminal_delivery_failure() {
        let (sender, receiver) = mpsc::sync_channel(1);
        drop(receiver);
        let failures = Arc::new(Mutex::new(Vec::new()));
        let reported_failures = Arc::clone(&failures);

        let result = send_without_blocking(
            &sender,
            "sibling closed",
            "unfocus-overlay-command-retry",
            move |error| {
                reported_failures
                    .lock()
                    .unwrap_or_else(std::sync::PoisonError::into_inner)
                    .push(error);
            },
        );

        assert_eq!(result.unwrap_err(), "overlay lifecycle worker has stopped");
        assert_eq!(
            failures
                .lock()
                .unwrap_or_else(std::sync::PoisonError::into_inner)
                .as_slice(),
            ["overlay lifecycle worker has stopped"]
        );
    }

    #[test]
    fn unexpected_native_cleanup_precedes_sibling_close() {
        let events = RefCell::new(Vec::new());

        cleanup_before_sibling_close(
            || {
                events.borrow_mut().push("cleanup");
                Ok(())
            },
            |_| panic!("successful cleanup must not be reported as an error"),
            || {
                events.borrow_mut().push("close-siblings");
                Ok(())
            },
        )
        .unwrap();

        assert_eq!(events.into_inner(), ["cleanup", "close-siblings"]);
    }

    #[test]
    fn native_cleanup_failure_defers_sibling_close() {
        let events = RefCell::new(Vec::new());

        let result = cleanup_before_sibling_close(
            || {
                events.borrow_mut().push("cleanup-failed".to_owned());
                Err("cleanup timeout".to_owned())
            },
            |error| events.borrow_mut().push(error.to_owned()),
            || {
                events.borrow_mut().push("close-siblings".to_owned());
                Ok(())
            },
        );

        assert_eq!(result.unwrap_err(), "cleanup timeout");
        assert_eq!(events.into_inner(), ["cleanup-failed", "cleanup timeout"]);
    }

    #[test]
    fn sibling_cleanup_failure_is_reported_after_native_cleanup_succeeds() {
        let reported = RefCell::new(Vec::new());

        let result = cleanup_before_sibling_close(
            || Ok(()),
            |error| reported.borrow_mut().push(error.to_owned()),
            || Err("sibling close failed".to_owned()),
        );

        assert_eq!(
            result.unwrap_err(),
            "sibling cleanup failed: sibling close failed"
        );
        assert_eq!(reported.into_inner(), ["sibling close failed"]);
    }

    #[test]
    fn unexpected_cleanup_state_is_retained_until_cleanup_succeeds() {
        let now = Instant::now();
        let mut pending = vec![PendingCleanup {
            run_id: 7,
            target: "overlay-7-0-1-8-9".to_owned(),
            attempts: 0,
            next_attempt_at: None,
        }];

        let (completed, exhausted) = process_pending_cleanups(&mut pending, now, |_| {
            Err("native teardown timed out".to_owned())
        });
        assert!(completed.is_empty());
        assert!(exhausted.is_empty());
        assert_eq!(pending.len(), 1);

        let (completed, exhausted) = process_pending_cleanups(
            &mut pending,
            now + overlay_close_retry_delay(1) - std::time::Duration::from_millis(1),
            |_| panic!("cleanup must wait for its backoff"),
        );
        assert!(completed.is_empty());
        assert!(exhausted.is_empty());

        let (completed, exhausted) =
            process_pending_cleanups(&mut pending, now + overlay_close_retry_delay(1), |_| Ok(()));
        assert_eq!(completed, [7]);
        assert!(exhausted.is_empty());
        assert!(pending.is_empty());
    }

    #[test]
    fn startup_cleanup_state_is_deduplicated_and_retained_until_absent() {
        let mut pending = Vec::new();
        assert!(enqueue_pending_cleanup(
            &mut pending,
            PendingCleanup {
                run_id: 17,
                target: "overlay-17-".to_owned(),
                attempts: 0,
                next_attempt_at: None,
            }
        ));
        assert!(!enqueue_pending_cleanup(
            &mut pending,
            PendingCleanup {
                run_id: 17,
                target: "overlay-17-duplicate-".to_owned(),
                attempts: 0,
                next_attempt_at: None,
            }
        ));

        let now = Instant::now();
        let (completed, exhausted) =
            process_pending_cleanups(&mut pending, now, |_| Err("panels remain".to_owned()));
        assert!(completed.is_empty());
        assert!(exhausted.is_empty());
        assert_eq!(pending.len(), 1);
        assert_eq!(pending[0].target, "overlay-17-");

        let (completed, exhausted) =
            process_pending_cleanups(&mut pending, now + overlay_close_retry_delay(1), |_| Ok(()));
        assert_eq!(completed, [17]);
        assert!(exhausted.is_empty());
        assert!(pending.is_empty());
    }

    #[test]
    fn permanently_failing_cleanup_is_dropped_at_the_shared_limit() {
        let mut pending = vec![PendingCleanup {
            run_id: 17,
            target: "overlay-17-".to_owned(),
            attempts: 0,
            next_attempt_at: None,
        }];

        let started_at = Instant::now();
        for attempt in 1..OVERLAY_CLOSE_FAILURE_LIMIT {
            let now = started_at + std::time::Duration::from_secs(attempt as u64);
            assert_eq!(
                process_pending_cleanups(&mut pending, now, |_| { Err("still open".to_owned()) }),
                (Vec::new(), Vec::new())
            );
        }

        let final_attempt =
            started_at + std::time::Duration::from_secs(OVERLAY_CLOSE_FAILURE_LIMIT as u64);
        assert_eq!(
            process_pending_cleanups(&mut pending, final_attempt, |_| {
                Err("still open".to_owned())
            }),
            (Vec::new(), vec![17])
        );
        assert!(pending.is_empty());
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
            now + INTENTIONAL_CLOSE_SUPPRESSION + std::time::Duration::from_millis(1)
        ));

        for run_id in 100..100 + CLOSE_ORIGIN_CAPACITY as u64 + 1 {
            origins.mark_intentional(run_id, now + std::time::Duration::from_secs(10));
        }
        assert_eq!(origins.intentional.len(), CLOSE_ORIGIN_CAPACITY);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn intentional_close_outlives_the_confirmed_macos_close_budget() {
        assert!(
            INTENTIONAL_CLOSE_SUPPRESSION
                > std::time::Duration::from_millis(windows::MACOS_CONFIRMED_CLOSE_BUDGET_MILLIS)
        );
    }

    #[test]
    fn close_origin_decision_intercepts_requests_without_duplicate_cleanup() {
        let now = Instant::now();
        let mut origins = CloseOriginState::default();

        let first = origins.decide_close(7, OverlayCloseEvent::Requested, now);
        let repeated = origins.decide_close(7, OverlayCloseEvent::Requested, now);
        assert!(first.queue_cleanup);
        assert!(!repeated.queue_cleanup);
        assert_eq!(first.prevent_close, cfg!(target_os = "macos"));
        assert_eq!(repeated.prevent_close, cfg!(target_os = "macos"));

        origins.mark_intentional(7, now);
        let intentional = origins.decide_close(7, OverlayCloseEvent::Requested, now);
        assert!(!intentional.queue_cleanup);
        assert!(!intentional.prevent_close);

        let destroyed = origins.decide_close(8, OverlayCloseEvent::Destroyed, now);
        assert!(destroyed.queue_cleanup);
        assert!(!destroyed.prevent_close);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn macos_unexpected_close_requests_remain_intercepted_while_cleanup_is_pending() {
        let now = Instant::now();
        let mut origins = CloseOriginState::default();

        assert!(
            origins
                .decide_close(11, OverlayCloseEvent::Requested, now)
                .prevent_close
        );
        assert!(
            origins
                .decide_close(11, OverlayCloseEvent::Requested, now)
                .prevent_close
        );
    }

    #[test]
    fn intentional_close_suppresses_sibling_cascade_for_the_same_run() {
        // Multi-monitor dismiss marks every window intentional before close so
        // Destroyed events do not queue a second sibling-teardown pass.
        let now = Instant::now();
        let mut origins = CloseOriginState::default();
        origins.mark_intentional(3, now);
        origins.mark_intentional(3, now);
        assert!(!origins.begin_unexpected(3, now));
        assert!(origins.unexpected_pending.is_empty());
        assert_eq!(origins.intentional.len(), 1);
    }

    #[test]
    fn cancel_unexpected_clears_pending_sibling_cleanup() {
        let now = Instant::now();
        let mut origins = CloseOriginState::default();
        assert!(origins.begin_unexpected(9, now));
        origins.cancel_unexpected(9);
        assert!(origins.begin_unexpected(9, now));
    }

    #[test]
    fn cancelled_intentional_close_can_be_recovered_as_unexpected() {
        let now = Instant::now();
        let mut origins = CloseOriginState::default();
        origins.mark_intentional(9, now);
        origins.cancel_intentional(9);

        assert!(origins.begin_unexpected(9, now));
    }
}
