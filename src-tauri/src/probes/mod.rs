#[cfg(any(target_os = "linux", test))]
mod linux;
#[cfg(any(target_os = "macos", test))]
mod macos;
#[cfg(any(target_os = "windows", test))]
mod windows;

use std::{
    io,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

const PROBE_POLL_INTERVAL: Duration = Duration::from_secs(2);
const PROBE_STALE_AFTER: Duration = Duration::from_secs(10);

#[derive(Debug, Clone)]
pub(crate) struct ProbeSnapshot {
    pub(crate) idle_seconds: Result<u64, String>,
    pub(crate) active_window_fullscreen: Result<bool, String>,
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
    fn read(&self, now: Instant, name: &str) -> Result<T, String> {
        let Some(updated_at) = self.updated_at else {
            return Err(format!("{name} probe result is not available yet"));
        };
        if now.saturating_duration_since(updated_at) > PROBE_STALE_AFTER {
            return Err(format!("{name} probe result is stale"));
        }

        self.reading
            .clone()
            .unwrap_or_else(|| Err(format!("{name} probe result is not available yet")))
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

    pub(crate) fn snapshot(&self) -> ProbeSnapshot {
        self.snapshot_at(Instant::now())
    }

    fn snapshot_at(&self, now: Instant) -> ProbeSnapshot {
        let Ok(inner) = self.inner.lock() else {
            let error = "probe cache lock is poisoned".to_owned();
            return ProbeSnapshot {
                idle_seconds: Err(error.clone()),
                active_window_fullscreen: Err(error),
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
        assert!(pending.idle_seconds.is_err());
        assert!(pending.active_window_fullscreen.is_err());

        cache.update_idle(Ok(7), updated_at);
        cache.update_fullscreen(Ok(true), updated_at);
        let fresh = cache.snapshot_at(updated_at + PROBE_STALE_AFTER);
        assert_eq!(fresh.idle_seconds, Ok(7));
        assert_eq!(fresh.active_window_fullscreen, Ok(true));

        let stale = cache.snapshot_at(updated_at + PROBE_STALE_AFTER + Duration::from_millis(1));
        assert_eq!(stale.idle_seconds, Err("idle probe result is stale".into()));
        assert_eq!(
            stale.active_window_fullscreen,
            Err("fullscreen probe result is stale".into())
        );
    }

    #[test]
    fn probe_cache_tracks_idle_and_fullscreen_liveness_independently() {
        let cache = ProbeCache::default();
        let now = Instant::now();

        cache.update_idle(Ok(11), now);
        let only_idle = cache.snapshot_at(now);
        assert_eq!(only_idle.idle_seconds, Ok(11));
        assert!(only_idle.active_window_fullscreen.is_err());

        let cache = ProbeCache::default();
        cache.update_fullscreen(Ok(false), now);
        let only_fullscreen = cache.snapshot_at(now);
        assert!(only_fullscreen.idle_seconds.is_err());
        assert_eq!(only_fullscreen.active_window_fullscreen, Ok(false));
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
