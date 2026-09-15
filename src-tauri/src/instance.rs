// src-tauri/src/instance.rs

use std::fmt;

use tauri::{AppHandle, Manager, UserAttentionType, WebviewWindow};

const MAIN_WINDOW_LABEL: &str = "main";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ActivationStep {
    Show,
    Unminimize,
    Focus,
    RequestAttention,
}

impl fmt::Display for ActivationStep {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self {
            Self::Show => "show",
            Self::Unminimize => "restore",
            Self::Focus => "focus",
            Self::RequestAttention => "request attention for",
        };
        formatter.write_str(label)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ActivationFailure {
    step: ActivationStep,
    error: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ActivationReport {
    MissingWindow,
    Completed(Vec<ActivationFailure>),
}

trait DashboardWindow {
    fn show(&self) -> Result<(), String>;
    fn unminimize(&self) -> Result<(), String>;
    fn focus(&self) -> Result<(), String>;
    fn request_attention(&self) -> Result<(), String>;
}

impl DashboardWindow for WebviewWindow {
    fn show(&self) -> Result<(), String> {
        WebviewWindow::show(self).map_err(|error| error.to_string())
    }

    fn unminimize(&self) -> Result<(), String> {
        WebviewWindow::unminimize(self).map_err(|error| error.to_string())
    }

    fn focus(&self) -> Result<(), String> {
        self.set_focus().map_err(|error| error.to_string())
    }

    fn request_attention(&self) -> Result<(), String> {
        self.request_user_attention(Some(UserAttentionType::Informational))
            .map_err(|error| error.to_string())
    }
}

fn record_step(
    failures: &mut Vec<ActivationFailure>,
    step: ActivationStep,
    result: Result<(), String>,
) {
    if let Err(error) = result {
        failures.push(ActivationFailure { step, error });
    }
}

fn activate_dashboard(window: Option<&impl DashboardWindow>) -> ActivationReport {
    let Some(window) = window else {
        return ActivationReport::MissingWindow;
    };

    let mut failures = Vec::new();
    record_step(&mut failures, ActivationStep::Show, window.show());
    record_step(
        &mut failures,
        ActivationStep::Unminimize,
        window.unminimize(),
    );

    if let Err(error) = window.focus() {
        failures.push(ActivationFailure {
            step: ActivationStep::Focus,
            error,
        });
        record_step(
            &mut failures,
            ActivationStep::RequestAttention,
            window.request_attention(),
        );
    }

    ActivationReport::Completed(failures)
}

fn report_activation(report: ActivationReport) {
    match report {
        ActivationReport::MissingWindow => {
            eprintln!(
                "could not reveal the existing Unfocus dashboard: the main window is unavailable"
            );
        }
        ActivationReport::Completed(failures) => {
            for failure in failures {
                eprintln!(
                    "could not {} the existing Unfocus dashboard: {}",
                    failure.step, failure.error
                );
            }
        }
    }
}

pub(crate) fn reveal_dashboard(app: &AppHandle) {
    let app = app.clone();
    let dispatcher = app.clone();
    if let Err(error) = dispatcher.run_on_main_thread(move || {
        let window = app.get_webview_window(MAIN_WINDOW_LABEL);
        report_activation(activate_dashboard(window.as_ref()));
    }) {
        eprintln!("could not schedule activation of the existing Unfocus dashboard: {error}");
    }
}

/// Handles a notification from an already-terminated secondary process.
/// Automatic login duplicates stay quiet; a manual launch reveals the dashboard.
pub(crate) fn handle_secondary_launch(app: &AppHandle, arguments: &[String]) {
    if !is_automatic_launch(arguments) {
        reveal_dashboard(app);
    }
}

pub(crate) fn is_automatic_launch(arguments: &[impl AsRef<std::ffi::OsStr>]) -> bool {
    cfg!(target_os = "linux")
        && arguments
            .iter()
            .skip(1)
            .any(|argument| argument.as_ref() == "--autostart")
}

pub(crate) fn configure_initial_window(config: &mut tauri::utils::config::Config, automatic: bool) {
    if automatic {
        if let Some(main) = config
            .app
            .windows
            .iter_mut()
            .find(|window| window.label == "main")
        {
            main.visible = false;
            main.focus = false;
        }
    }
}

pub(crate) fn reveal_after_tray_install(automatic: bool, tray_available: bool) -> bool {
    automatic && !tray_available
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;

    #[derive(Default)]
    struct FakeWindow {
        calls: RefCell<Vec<ActivationStep>>,
        failures: Vec<ActivationStep>,
    }

    impl FakeWindow {
        fn failing(failures: &[ActivationStep]) -> Self {
            Self {
                calls: RefCell::default(),
                failures: failures.to_vec(),
            }
        }

        fn perform(&self, step: ActivationStep) -> Result<(), String> {
            self.calls.borrow_mut().push(step);
            if self.failures.contains(&step) {
                Err(format!("{step} failed"))
            } else {
                Ok(())
            }
        }
    }

    impl DashboardWindow for FakeWindow {
        fn show(&self) -> Result<(), String> {
            self.perform(ActivationStep::Show)
        }

        fn unminimize(&self) -> Result<(), String> {
            self.perform(ActivationStep::Unminimize)
        }

        fn focus(&self) -> Result<(), String> {
            self.perform(ActivationStep::Focus)
        }

        fn request_attention(&self) -> Result<(), String> {
            self.perform(ActivationStep::RequestAttention)
        }
    }

    #[test]
    fn automatic_launches_are_quiet_and_manual_duplicates_reveal() {
        assert_eq!(
            is_automatic_launch(&["unfocus", "--autostart"]),
            cfg!(target_os = "linux")
        );
        assert!(!is_automatic_launch(&["unfocus"]));
        assert!(!is_automatic_launch(&["unfocus", "--autostart=false"]));
        assert!(!is_automatic_launch(&["--autostart"]));
    }

    #[test]
    fn automatic_window_is_hidden_before_creation_and_tray_failure_requires_reveal() {
        let mut config = tauri::utils::config::Config::default();
        config
            .app
            .windows
            .push(tauri::utils::config::WindowConfig::default());
        config.app.windows[0].label = "main".into();
        configure_initial_window(&mut config, false);
        assert!(config.app.windows[0].visible);
        configure_initial_window(&mut config, true);
        assert!(!config.app.windows[0].visible);
        assert!(!config.app.windows[0].focus);
        assert!(reveal_after_tray_install(true, false));
        assert!(!reveal_after_tray_install(true, true));
        assert!(!reveal_after_tray_install(false, false));
    }

    #[test]
    fn activation_shows_restores_and_focuses_the_existing_window() {
        let window = FakeWindow::default();

        assert_eq!(
            activate_dashboard(Some(&window)),
            ActivationReport::Completed(Vec::new())
        );
        assert_eq!(
            *window.calls.borrow(),
            [
                ActivationStep::Show,
                ActivationStep::Unminimize,
                ActivationStep::Focus,
            ]
        );
    }

    #[test]
    fn repeated_activation_is_idempotent_and_only_repeats_window_operations() {
        let window = FakeWindow::default();

        for _ in 0..3 {
            assert_eq!(
                activate_dashboard(Some(&window)),
                ActivationReport::Completed(Vec::new())
            );
        }

        assert_eq!(window.calls.borrow().len(), 9);
        assert!(window
            .calls
            .borrow()
            .as_chunks::<3>()
            .0
            .iter()
            .all(|calls| {
                calls
                    == &[
                        ActivationStep::Show,
                        ActivationStep::Unminimize,
                        ActivationStep::Focus,
                    ]
            }));
    }

    #[test]
    fn failed_focus_requests_attention_without_skipping_other_steps() {
        let window = FakeWindow::failing(&[ActivationStep::Focus]);

        let report = activate_dashboard(Some(&window));

        assert_eq!(
            report,
            ActivationReport::Completed(vec![ActivationFailure {
                step: ActivationStep::Focus,
                error: "focus failed".into(),
            }])
        );
        assert_eq!(
            *window.calls.borrow(),
            [
                ActivationStep::Show,
                ActivationStep::Unminimize,
                ActivationStep::Focus,
                ActivationStep::RequestAttention,
            ]
        );
    }

    #[test]
    fn activation_reports_every_recoverable_window_failure() {
        let window = FakeWindow::failing(&[
            ActivationStep::Show,
            ActivationStep::Unminimize,
            ActivationStep::Focus,
            ActivationStep::RequestAttention,
        ]);

        let report = activate_dashboard(Some(&window));

        let ActivationReport::Completed(failures) = report else {
            panic!("the existing window should have been used");
        };
        assert_eq!(failures.len(), 4);
        assert_eq!(
            failures
                .iter()
                .map(|failure| failure.step)
                .collect::<Vec<_>>(),
            [
                ActivationStep::Show,
                ActivationStep::Unminimize,
                ActivationStep::Focus,
                ActivationStep::RequestAttention,
            ]
        );
    }

    #[test]
    fn a_missing_dashboard_has_an_explicit_recovery_report() {
        let window: Option<&FakeWindow> = None;
        assert_eq!(activate_dashboard(window), ActivationReport::MissingWindow);
    }
}
