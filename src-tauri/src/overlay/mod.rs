mod labels;
mod lifecycle;
mod windows;

pub(crate) use labels::{
    overlay_run_id_from_label, MAX_OVERLAY_DURATION_SECONDS, MIN_OVERLAY_DURATION_SECONDS,
};
#[cfg(debug_assertions)]
pub(crate) use windows::schedule_automatic_overlay_test;
pub(crate) use windows::{show_overlay, show_overlay_if_idle};

use crate::authorize_main_caller;
use labels::authorize_overlay_close_caller;
use lifecycle::{overlay_worker_timeout, OverlayRunLifecycle, OVERLAY_DISMISS_DELAY};
use serde::Serialize;
use std::{
    collections::VecDeque,
    io,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError},
        Arc, Mutex,
    },
    time::Instant,
};
use tauri::{AppHandle, Manager, State, WebviewWindow};
use windows::{close_overlay_windows, emit_overlay_event, overlay_run_exists};

const OVERLAY_COMMAND_CAPACITY: usize = 256;
const CLOSE_ORIGIN_CAPACITY: usize = 16;
const INTENTIONAL_CLOSE_SUPPRESSION: std::time::Duration = std::time::Duration::from_secs(5);

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
pub(crate) struct OverlayController {
    sender: SyncSender<OverlayCommand>,
    close_origins: OverlayCloseOrigins,
    active_runs: Arc<AtomicUsize>,
}

impl OverlayController {
    pub(crate) fn start(app: AppHandle) -> io::Result<Self> {
        let (sender, receiver) = mpsc::sync_channel(OVERLAY_COMMAND_CAPACITY);
        let close_origins = OverlayCloseOrigins::default();
        let worker_close_origins = close_origins.clone();
        let active_runs = Arc::new(AtomicUsize::new(0));
        let worker_active_runs = Arc::clone(&active_runs);
        std::thread::Builder::new()
            .name("unfocus-overlays".into())
            .spawn(move || {
                run_overlay_worker(app, receiver, worker_close_origins, worker_active_runs);
            })?;
        Ok(Self {
            sender,
            close_origins,
            active_runs,
        })
    }

    pub(crate) fn has_active_run(&self) -> bool {
        self.active_runs.load(Ordering::Acquire) > 0
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

    pub(crate) fn sibling_closed(&self, run_id: u64) {
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

fn run_overlay_worker(
    app: AppHandle,
    receiver: Receiver<OverlayCommand>,
    close_origins: OverlayCloseOrigins,
    active_runs: Arc<AtomicUsize>,
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
            Err(RecvTimeoutError::Disconnected) => {
                active_runs.store(0, Ordering::Release);
                break;
            }
        }

        process_overlay_runs(&app, &close_origins, &mut runs, Instant::now());
        active_runs.store(runs.len(), Ordering::Release);
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

#[tauri::command]
pub(crate) async fn show_overlay_test(
    window: WebviewWindow,
    controller: State<'_, OverlayController>,
    duration_seconds: u64,
) -> Result<usize, String> {
    authorize_main_caller(window.label())?;
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

#[cfg(test)]
mod tests {
    use super::*;

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
}
