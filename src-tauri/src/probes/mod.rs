// src-tauri/src/probes/mod.rs

#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(any(target_os = "linux", test))]
mod sway;
#[cfg(any(target_os = "windows", test))]
mod windows;

use std::{
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const PROBE_POLL_INTERVAL: Duration = Duration::from_secs(2);
const PROBE_STALE_AFTER: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProbeReading<T> {
    Pending,
    Available(T),
    Failed(String),
}

#[derive(Debug, Clone)]
pub(crate) struct ProbeSnapshot {
    pub(crate) idle_seconds: ProbeReading<u64>,
    pub(crate) active_window_fullscreen: ProbeReading<bool>,
}

#[cfg(test)]
impl ProbeSnapshot {
    pub(crate) fn pending() -> Self {
        Self {
            idle_seconds: ProbeReading::Pending,
            active_window_fullscreen: ProbeReading::Pending,
        }
    }

    pub(crate) fn available(idle_seconds: u64, active_window_fullscreen: bool) -> Self {
        Self {
            idle_seconds: ProbeReading::Available(idle_seconds),
            active_window_fullscreen: ProbeReading::Available(active_window_fullscreen),
        }
    }

    pub(crate) fn failed(idle_error: &str, fullscreen_error: &str) -> Self {
        Self {
            idle_seconds: ProbeReading::Failed(idle_error.to_owned()),
            active_window_fullscreen: ProbeReading::Failed(fullscreen_error.to_owned()),
        }
    }
}

/// Discriminated probe backend for diagnostics (never infers support from env alone).
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize)]
#[serde(tag = "kind")]
#[allow(dead_code)] // variants are selected per target OS
pub(crate) enum ProbeBackend {
    #[serde(rename = "x11")]
    X11,
    #[serde(rename = "quartz")]
    Quartz,
    #[serde(rename = "win32")]
    Win32,
    /// Opt-in Sway candidate only (`wayland-sway` feature + runtime gates).
    #[serde(rename = "sway", rename_all = "camelCase")]
    Sway { version: String, candidate: bool },
    #[serde(rename = "unsupported")]
    Unsupported,
}

/// Report which probe backend this process is using for the current session.
pub(crate) fn probe_backend() -> ProbeBackend {
    #[cfg(target_os = "macos")]
    {
        ProbeBackend::Quartz
    }
    #[cfg(target_os = "windows")]
    {
        ProbeBackend::Win32
    }
    #[cfg(target_os = "linux")]
    {
        linux_probe_backend()
    }
    #[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
    {
        ProbeBackend::Unsupported
    }
}

pub(crate) fn qualified_x11_session() -> bool {
    #[cfg(target_os = "linux")]
    {
        linux::validate_session(
            std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
            std::env::var("DISPLAY").ok().as_deref(),
        )
        .is_ok()
    }
    #[cfg(not(target_os = "linux"))]
    {
        false
    }
}

#[cfg(target_os = "linux")]
fn linux_probe_backend() -> ProbeBackend {
    let session = std::env::var("XDG_SESSION_TYPE").ok();
    match session.as_deref() {
        Some(session) if session.eq_ignore_ascii_case("x11") => ProbeBackend::X11,
        Some(session) if session.eq_ignore_ascii_case("wayland") => {
            #[cfg(feature = "wayland-sway")]
            {
                if let Some((version, candidate)) = sway::backend_label() {
                    return ProbeBackend::Sway { version, candidate };
                }
            }
            ProbeBackend::Unsupported
        }
        _ => ProbeBackend::Unsupported,
    }
}

#[derive(Debug, Clone)]
struct CachedProbe<T> {
    reading: Option<Result<T, String>>,
    updated_at: Option<Instant>,
}

impl<T> Default for CachedProbe<T> {
    fn default() -> Self {
        Self {
            reading: None,
            updated_at: None,
        }
    }
}

impl<T: Clone> CachedProbe<T> {
    fn read(&self, now: Instant, name: &str) -> ProbeReading<T> {
        let Some(updated_at) = self.updated_at else {
            return ProbeReading::Pending;
        };
        if now.saturating_duration_since(updated_at) > PROBE_STALE_AFTER {
            return ProbeReading::Failed(format!("{name} probe result is stale"));
        }

        match self.reading.clone() {
            Some(Ok(reading)) => ProbeReading::Available(reading),
            Some(Err(error)) => ProbeReading::Failed(error),
            None => ProbeReading::Failed(format!("{name} probe cache is inconsistent")),
        }
    }
}

#[derive(Debug, Default)]
struct ProbeCacheInner {
    idle_seconds: CachedProbe<u64>,
    active_window_fullscreen: CachedProbe<bool>,
}

#[derive(Debug, Clone, Default)]
pub(crate) struct ProbeCache {
    inner: Arc<Mutex<ProbeCacheInner>>,
}

impl ProbeCache {
    pub(crate) fn start() -> io::Result<Self> {
        let cache = Self::default();
        let idle_cache = cache.clone();
        std::thread::Builder::new()
            .name("unfocus-idle-probe".into())
            .spawn(move || {
                run_probe_worker(
                    idle_cache,
                    "idle",
                    platform_probe::idle_seconds,
                    ProbeCache::update_idle,
                );
            })?;
        let fullscreen_cache = cache.clone();
        std::thread::Builder::new()
            .name("unfocus-fullscreen-probe".into())
            .spawn(move || {
                run_probe_worker(
                    fullscreen_cache,
                    "fullscreen",
                    platform_probe::active_window_fullscreen,
                    ProbeCache::update_fullscreen,
                );
            })?;
        Ok(cache)
    }

    fn update_idle(&self, reading: Result<u64, String>, now: Instant) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.idle_seconds = CachedProbe {
                reading: Some(reading),
                updated_at: Some(now),
            };
        }
    }

    fn update_fullscreen(&self, reading: Result<bool, String>, now: Instant) {
        if let Ok(mut inner) = self.inner.lock() {
            inner.active_window_fullscreen = CachedProbe {
                reading: Some(reading),
                updated_at: Some(now),
            };
        }
    }

    #[cfg(test)]
    pub(crate) fn update_idle_for_test(&self, reading: Result<u64, String>, now: Instant) {
        self.update_idle(reading, now);
    }

    pub(crate) fn snapshot(&self) -> ProbeSnapshot {
        self.snapshot_at(Instant::now())
    }

    fn snapshot_at(&self, now: Instant) -> ProbeSnapshot {
        let Ok(inner) = self.inner.lock() else {
            let error = "probe cache lock is poisoned".to_owned();
            return ProbeSnapshot {
                idle_seconds: ProbeReading::Failed(error.clone()),
                active_window_fullscreen: ProbeReading::Failed(error),
            };
        };

        ProbeSnapshot {
            idle_seconds: inner.idle_seconds.read(now, "idle"),
            active_window_fullscreen: inner.active_window_fullscreen.read(now, "fullscreen"),
        }
    }
}

fn guarded_probe<T>(name: &str, probe: impl FnOnce() -> Result<T, String>) -> Result<T, String> {
    std::panic::catch_unwind(std::panic::AssertUnwindSafe(probe))
        .unwrap_or_else(|_| Err(format!("{name} probe panicked")))
}

fn log_probe_transition<T>(name: &str, result: &Result<T, String>, unavailable: &mut bool) {
    match result {
        Err(error) if !*unavailable => {
            eprintln!("{name} probe unavailable: {error}");
            *unavailable = true;
        }
        Ok(_) if std::mem::take(unavailable) => {
            eprintln!("{name} probe recovered");
        }
        _ => {}
    }
}

fn run_probe_worker<T>(
    cache: ProbeCache,
    name: &'static str,
    probe: fn() -> Result<T, String>,
    update: fn(&ProbeCache, Result<T, String>, Instant),
) {
    let mut unavailable = false;
    loop {
        let reading = guarded_probe(name, probe);
        log_probe_transition(name, &reading, &mut unavailable);
        update(&cache, reading, Instant::now());

        std::thread::sleep(PROBE_POLL_INTERVAL);
    }
}

#[cfg(target_os = "linux")]
mod platform_probe {
    pub(super) use super::linux::{active_window_fullscreen, idle_seconds};
}

#[cfg(target_os = "macos")]
mod platform_probe {
    pub(super) use super::macos::{active_window_fullscreen, idle_seconds};
}

#[cfg(target_os = "windows")]
mod platform_probe {
    pub(super) use super::windows::{active_window_fullscreen, idle_seconds};
}

#[cfg(not(any(target_os = "linux", target_os = "macos", target_os = "windows")))]
mod platform_probe {
    pub(super) fn idle_seconds() -> Result<u64, String> {
        Err(format!(
            "no idle probe is implemented for {}",
            std::env::consts::OS
        ))
    }

    pub(super) fn active_window_fullscreen() -> Result<bool, String> {
        Err(format!(
            "no fullscreen probe is implemented for {}",
            std::env::consts::OS
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_cache_is_non_blocking_and_rejects_stale_results() {
        let cache = ProbeCache::default();
        let updated_at = Instant::now();

        let pending = cache.snapshot_at(updated_at);
        assert_eq!(pending.idle_seconds, ProbeReading::Pending);
        assert_eq!(pending.active_window_fullscreen, ProbeReading::Pending);

        cache.update_idle(Ok(7), updated_at);
        cache.update_fullscreen(Ok(true), updated_at);
        let fresh = cache.snapshot_at(updated_at + PROBE_STALE_AFTER);
        assert_eq!(fresh.idle_seconds, ProbeReading::Available(7));
        assert_eq!(
            fresh.active_window_fullscreen,
            ProbeReading::Available(true)
        );

        let stale = cache.snapshot_at(updated_at + PROBE_STALE_AFTER + Duration::from_millis(1));
        assert_eq!(
            stale.idle_seconds,
            ProbeReading::Failed("idle probe result is stale".into())
        );
        assert_eq!(
            stale.active_window_fullscreen,
            ProbeReading::Failed("fullscreen probe result is stale".into())
        );
    }

    #[test]
    fn probe_cache_tracks_idle_and_fullscreen_liveness_independently() {
        let cache = ProbeCache::default();
        let now = Instant::now();

        cache.update_idle(Ok(11), now);
        let only_idle = cache.snapshot_at(now);
        assert_eq!(only_idle.idle_seconds, ProbeReading::Available(11));
        assert_eq!(only_idle.active_window_fullscreen, ProbeReading::Pending);

        let cache = ProbeCache::default();
        cache.update_fullscreen(Ok(false), now);
        let only_fullscreen = cache.snapshot_at(now);
        assert_eq!(only_fullscreen.idle_seconds, ProbeReading::Pending);
        assert_eq!(
            only_fullscreen.active_window_fullscreen,
            ProbeReading::Available(false)
        );
    }

    #[test]
    fn cached_and_lock_failures_are_explicitly_failed() {
        let cache = ProbeCache::default();
        let now = Instant::now();
        cache.update_idle(Err("idle failed".into()), now);
        assert_eq!(
            cache.snapshot_at(now).idle_seconds,
            ProbeReading::Failed("idle failed".into())
        );

        let poisoned = ProbeCache::default();
        let inner = Arc::clone(&poisoned.inner);
        let _ = std::thread::spawn(move || {
            let _guard = inner.lock().expect("cache lock");
            panic!("poison cache lock");
        })
        .join();
        let snapshot = poisoned.snapshot_at(now);
        assert_eq!(
            snapshot.idle_seconds,
            ProbeReading::Failed("probe cache lock is poisoned".into())
        );
        assert_eq!(
            snapshot.active_window_fullscreen,
            ProbeReading::Failed("probe cache lock is poisoned".into())
        );
    }

    #[test]
    fn probe_failures_are_tracked_as_one_unavailable_period() {
        let mut unavailable = false;

        log_probe_transition::<()>("test", &Err("first".into()), &mut unavailable);
        assert!(unavailable);
        log_probe_transition::<()>("test", &Err("changed".into()), &mut unavailable);
        assert!(unavailable);
        log_probe_transition("test", &Ok(()), &mut unavailable);
        assert!(!unavailable);
    }
}
