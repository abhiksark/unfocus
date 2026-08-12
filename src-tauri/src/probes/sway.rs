//! Sway Wayland candidate probes (issue #25).
//!
//! Default packages leave Wayland unsupported. The opt-in `wayland-sway` Cargo
//! feature enables a **candidate** backend only after positive runtime checks
//! for Sway 1.11+, `seat0`, and `ext_idle_notifier_v1` version 2+. Nothing here
//! claims packaged Wayland support or multi-monitor qualification.
//!
//! Design source: issue #25 implementation proposal (Sway IPC + input-only idle).
//!
//! Pure helpers below are shared by unit tests and the feature-gated runtime.
//! Without `wayland-sway` they are intentionally unused in the production lib.

#![cfg_attr(
    not(all(target_os = "linux", feature = "wayland-sway")),
    allow(dead_code)
)]

use std::time::{Duration, Instant};

/// Minimum Sway major.minor accepted for the candidate row.
pub(super) const MIN_SWAY_MAJOR: u32 = 1;
pub(super) const MIN_SWAY_MINOR: u32 = 11;

/// Required Wayland global name and minimum interface version.
pub(super) const EXT_IDLE_NOTIFIER_NAME: &str = "ext_idle_notifier_v1";
pub(super) const EXT_IDLE_NOTIFIER_MIN_VERSION: u32 = 2;

/// Seat name required by the candidate matrix.
pub(super) const REQUIRED_SEAT_NAME: &str = "seat0";

/// Sway / i3 IPC magic (native endian length + type follow).
pub(super) const SWAY_IPC_MAGIC: &[u8; 6] = b"i3-ipc";

pub(super) const SWAY_IPC_GET_TREE: u32 = 4;
pub(super) const SWAY_IPC_GET_VERSION: u32 = 7;
pub(super) const SWAY_IPC_GET_SEATS: u32 = 101;

/// Reject IPC payloads larger than this before allocation.
pub(super) const SWAY_IPC_MAX_PAYLOAD_BYTES: usize = 16 * 1024 * 1024;

/// Bound for connect/read/write on the Sway socket.
pub(super) const SWAY_IPC_TIMEOUT: Duration = Duration::from_millis(500);

/// Parsed Sway version from IPC `GET_VERSION`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct SwayVersion {
    pub(super) major: u32,
    pub(super) minor: u32,
    pub(super) patch: u32,
    pub(super) human_readable: String,
}

impl SwayVersion {
    pub(super) fn meets_minimum(&self) -> bool {
        self.major > MIN_SWAY_MAJOR
            || (self.major == MIN_SWAY_MAJOR && self.minor >= MIN_SWAY_MINOR)
    }
}

/// Environment hints used only for discovery; never establish support alone.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) struct SessionHints<'a> {
    pub(super) session_type: Option<&'a str>,
    pub(super) current_desktop: Option<&'a str>,
    pub(super) sway_sock: Option<&'a str>,
}

/// Why a session is not the Sway candidate row.
#[derive(Debug, Clone, PartialEq, Eq)]
#[allow(dead_code)] // some variants only apply with or without wayland-sway
pub(super) enum QualificationError {
    NotWayland,
    DesktopNotSway,
    MissingSwaySock,
    EmptySwaySock,
    SwayTooOld { version: SwayVersion },
    MissingIdleNotifier,
    IdleNotifierVersionTooOld { version: u32 },
    MissingSeat0,
    FeatureDisabled,
}

impl QualificationError {
    pub(super) fn message(&self) -> String {
        match self {
            Self::NotWayland => {
                "Linux session is not Wayland; Sway candidate probes require XDG_SESSION_TYPE=wayland"
                    .into()
            }
            Self::DesktopNotSway => {
                "Linux Wayland desktop is not Sway; only the Sway candidate row is implemented"
                    .into()
            }
            Self::MissingSwaySock => "SWAYSOCK is unset; Sway IPC is required for the candidate".into(),
            Self::EmptySwaySock => "SWAYSOCK is empty".into(),
            Self::SwayTooOld { version } => format!(
                "Sway {} is below the candidate floor {}.{}",
                version.human_readable, MIN_SWAY_MAJOR, MIN_SWAY_MINOR
            ),
            Self::MissingIdleNotifier => format!(
                "Wayland compositor does not advertise {EXT_IDLE_NOTIFIER_NAME}"
            ),
            Self::IdleNotifierVersionTooOld { version } => format!(
                "{EXT_IDLE_NOTIFIER_NAME} version {version} is below the candidate floor {EXT_IDLE_NOTIFIER_MIN_VERSION}"
            ),
            Self::MissingSeat0 => format!(
                "Wayland seat {REQUIRED_SEAT_NAME} is unavailable for input-idle tracking"
            ),
            Self::FeatureDisabled => {
                "Linux Wayland is unsupported in default builds; rebuild with --features wayland-sway for the Sway 1.11+ candidate"
                    .into()
            }
        }
    }
}

/// True when `XDG_CURRENT_DESKTOP` contains a discrete `sway` token (colon-separated).
pub(super) fn desktop_has_sway_token(current_desktop: &str) -> bool {
    current_desktop
        .split(':')
        .any(|token| token.eq_ignore_ascii_case("sway"))
}

/// Environment-only gates before opening sockets. Missing runtime capabilities
/// are checked separately after IPC / registry discovery.
pub(super) fn qualify_session_hints(hints: SessionHints<'_>) -> Result<(), QualificationError> {
    match hints.session_type {
        Some(session) if session.eq_ignore_ascii_case("wayland") => {}
        Some(_) | None => return Err(QualificationError::NotWayland),
    }

    match hints.current_desktop {
        Some(desktop) if desktop_has_sway_token(desktop) => {}
        _ => return Err(QualificationError::DesktopNotSway),
    }

    match hints.sway_sock {
        None => return Err(QualificationError::MissingSwaySock),
        Some("") => return Err(QualificationError::EmptySwaySock),
        Some(_) => {}
    }

    Ok(())
}

/// Parse `GET_VERSION` JSON (`major` / `minor` / `patch` / `human_readable`).
pub(super) fn parse_sway_version_json(payload: &str) -> Result<SwayVersion, String> {
    let value: serde_json::Value = serde_json::from_str(payload)
        .map_err(|error| format!("Sway GET_VERSION JSON is malformed: {error}"))?;
    let major = json_u32(&value, "major")?;
    let minor = json_u32(&value, "minor")?;
    let patch = json_u32(&value, "patch").unwrap_or(0);
    let human_readable = value
        .get("human_readable")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .unwrap_or_else(|| format!("{major}.{minor}.{patch}"));
    Ok(SwayVersion {
        major,
        minor,
        patch,
        human_readable,
    })
}

fn json_u32(value: &serde_json::Value, field: &str) -> Result<u32, String> {
    value
        .get(field)
        .and_then(|v| {
            v.as_u64()
                .and_then(|n| u32::try_from(n).ok())
                .or_else(|| v.as_i64().and_then(|n| u32::try_from(n).ok()))
        })
        .ok_or_else(|| format!("Sway version field `{field}` is missing or not a u32"))
}

/// Encode one Sway IPC request (header + payload).
pub(super) fn encode_ipc_request(message_type: u32, payload: &[u8]) -> Result<Vec<u8>, String> {
    if payload.len() > SWAY_IPC_MAX_PAYLOAD_BYTES {
        return Err(format!(
            "Sway IPC request payload exceeds {SWAY_IPC_MAX_PAYLOAD_BYTES} bytes"
        ));
    }
    let mut bytes = Vec::with_capacity(14 + payload.len());
    bytes.extend_from_slice(SWAY_IPC_MAGIC);
    bytes.extend_from_slice(&(payload.len() as u32).to_ne_bytes());
    bytes.extend_from_slice(&message_type.to_ne_bytes());
    bytes.extend_from_slice(payload);
    Ok(bytes)
}

/// Decode one Sway IPC reply header; returns (payload_len, reply_type).
pub(super) fn decode_ipc_header(header: &[u8; 14]) -> Result<(usize, u32), String> {
    if &header[..6] != SWAY_IPC_MAGIC.as_slice() {
        return Err("Sway IPC reply has an invalid magic header".into());
    }
    let payload_len = u32::from_ne_bytes(header[6..10].try_into().unwrap()) as usize;
    let reply_type = u32::from_ne_bytes(header[10..14].try_into().unwrap());
    if payload_len > SWAY_IPC_MAX_PAYLOAD_BYTES {
        return Err(format!(
            "Sway IPC reply payload length {payload_len} exceeds cap {SWAY_IPC_MAX_PAYLOAD_BYTES}"
        ));
    }
    Ok((payload_len, reply_type))
}

/// Validate that a complete reply matches the expected type and has no trailing bytes.
pub(super) fn validate_ipc_reply(
    expected_type: u32,
    reply_type: u32,
    payload: &[u8],
    declared_len: usize,
) -> Result<(), String> {
    if reply_type != expected_type {
        return Err(format!(
            "Sway IPC reply type {reply_type} does not match expected {expected_type}"
        ));
    }
    if payload.len() != declared_len {
        return Err(format!(
            "Sway IPC reply payload length {} does not match declared {declared_len}",
            payload.len()
        ));
    }
    Ok(())
}

/// Seat focus from `GET_SEATS`: require `seat0` with a nonzero focused node id.
pub(super) fn focused_node_id_from_seats_json(payload: &str) -> Result<u64, String> {
    let seats: Vec<serde_json::Value> = serde_json::from_str(payload)
        .map_err(|error| format!("Sway GET_SEATS JSON is malformed: {error}"))?;
    let mut seat0_focus: Option<u64> = None;
    for seat in &seats {
        let name = seat
            .get("name")
            .and_then(|v| v.as_str())
            .ok_or_else(|| "Sway seat entry is missing a name".to_owned())?;
        if !name.eq_ignore_ascii_case(REQUIRED_SEAT_NAME) {
            continue;
        }
        if seat0_focus.is_some() {
            return Err("Sway GET_SEATS returned duplicate seat0 entries".into());
        }
        let focus = seat
            .get("focus")
            .and_then(|v| {
                v.as_u64()
                    .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
            })
            .ok_or_else(|| "Sway seat0 focus is missing or not an integer".to_owned())?;
        seat0_focus = Some(focus);
    }
    let focus = seat0_focus.ok_or_else(|| "Sway GET_SEATS does not include seat0".to_owned())?;
    if focus == 0 {
        return Err("Sway seat0 has no focused node".into());
    }
    Ok(focus)
}

/// Walk the complete tree for `focus_id` and report fullscreen from that node
/// and every ancestor. Mode `1` (workspace) and `2` (global) are fullscreen;
/// `0` is not (maximized stays false).
pub(super) fn focused_window_is_fullscreen_from_tree(
    payload: &str,
    focus_id: u64,
) -> Result<bool, String> {
    let root: serde_json::Value = serde_json::from_str(payload)
        .map_err(|error| format!("Sway GET_TREE JSON is malformed: {error}"))?;
    let mut matches = Vec::new();
    collect_focus_paths(&root, focus_id, &mut Vec::new(), &mut matches)?;
    match matches.len() {
        0 => Err(format!(
            "Sway tree does not contain focused node id {focus_id}"
        )),
        1 => Ok(matches[0]),
        _ => Err(format!(
            "Sway tree contains focused node id {focus_id} more than once"
        )),
    }
}

fn collect_focus_paths(
    node: &serde_json::Value,
    focus_id: u64,
    ancestors_fullscreen: &mut Vec<bool>,
    matches: &mut Vec<bool>,
) -> Result<(), String> {
    let id = node.get("id").and_then(|v| {
        v.as_u64()
            .or_else(|| v.as_i64().and_then(|n| u64::try_from(n).ok()))
    });
    let mode = node
        .get("fullscreen_mode")
        .map(parse_fullscreen_mode)
        .transpose()?;
    let self_fs = mode.unwrap_or(0) != 0;
    let path_fs = self_fs || ancestors_fullscreen.iter().copied().any(|v| v);

    if id == Some(focus_id) {
        matches.push(path_fs);
    }

    ancestors_fullscreen.push(self_fs);
    for child in node_children(node) {
        collect_focus_paths(child, focus_id, ancestors_fullscreen, matches)?;
    }
    ancestors_fullscreen.pop();
    Ok(())
}

fn parse_fullscreen_mode(value: &serde_json::Value) -> Result<u32, String> {
    let mode = value
        .as_u64()
        .or_else(|| value.as_i64().and_then(|n| u64::try_from(n).ok()))
        .ok_or_else(|| "Sway fullscreen_mode is not an integer".to_owned())?;
    let mode =
        u32::try_from(mode).map_err(|_| "Sway fullscreen_mode is out of range".to_owned())?;
    if mode > 2 {
        return Err(format!("Sway fullscreen_mode {mode} is not 0, 1, or 2"));
    }
    Ok(mode)
}

fn node_children(node: &serde_json::Value) -> impl Iterator<Item = &serde_json::Value> {
    let nodes = node
        .get("nodes")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    let floating = node
        .get("floating_nodes")
        .and_then(|v| v.as_array())
        .map(|a| a.as_slice())
        .unwrap_or(&[]);
    nodes.iter().chain(floating.iter())
}

/// Input-idle baseline driven by `ext_idle_notification_v1` events.
///
/// The protocol starts "not idle" and only emits `resumed` after an `idled`.
/// Until the first event, readings error rather than inventing a duration.
/// After that: active → 0s idle; idle → whole seconds since the `idled` event.
#[derive(Debug, Clone, Default)]
pub(super) struct InputIdleBaseline {
    state: IdleBaselineState,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
enum IdleBaselineState {
    #[default]
    Unknown,
    Active,
    Idle {
        since: Instant,
    },
}

impl InputIdleBaseline {
    pub(super) fn clear(&mut self) {
        self.state = IdleBaselineState::Unknown;
    }

    pub(super) fn on_idled(&mut self, now: Instant) {
        self.state = IdleBaselineState::Idle { since: now };
    }

    pub(super) fn on_resumed(&mut self) {
        self.state = IdleBaselineState::Active;
    }

    pub(super) fn idle_seconds(&self, now: Instant) -> Result<u64, String> {
        match self.state {
            IdleBaselineState::Unknown => Err(
                "Wayland input-idle baseline is not established yet; waiting for ext_idle_notification_v1 events"
                    .into(),
            ),
            IdleBaselineState::Active => Ok(0),
            IdleBaselineState::Idle { since } => {
                Ok(now.saturating_duration_since(since).as_secs())
            }
        }
    }
}

/// Check advertised idle-notifier version against the candidate floor.
pub(super) fn qualify_idle_notifier_version(version: u32) -> Result<(), QualificationError> {
    if version < EXT_IDLE_NOTIFIER_MIN_VERSION {
        return Err(QualificationError::IdleNotifierVersionTooOld { version });
    }
    Ok(())
}

/// Check parsed Sway version against the candidate floor.
pub(super) fn qualify_sway_version(version: &SwayVersion) -> Result<(), QualificationError> {
    if version.meets_minimum() {
        Ok(())
    } else {
        Err(QualificationError::SwayTooOld {
            version: version.clone(),
        })
    }
}

// --- Linux runtime (IPC + optional Wayland idle worker) ---------------------

#[cfg(all(target_os = "linux", feature = "wayland-sway"))]
mod runtime {
    use super::*;
    use std::{
        io::{Read, Write},
        os::unix::{fs::MetadataExt, net::UnixStream},
        path::Path,
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc, Mutex, OnceLock,
        },
        thread,
        time::Instant,
    };
    use wayland_client::{
        protocol::{wl_registry, wl_seat},
        Connection, Dispatch, EventQueue, QueueHandle,
    };
    use wayland_protocols::ext::idle_notify::v1::client::{
        ext_idle_notification_v1, ext_idle_notifier_v1,
    };

    static IDLE_WORKER: OnceLock<IdleWorkerHandle> = OnceLock::new();

    struct IdleWorkerHandle {
        baseline: Arc<Mutex<InputIdleBaseline>>,
        session_live: Arc<AtomicBool>,
        last_error: Arc<Mutex<Option<String>>>,
    }

    fn worker() -> &'static IdleWorkerHandle {
        IDLE_WORKER.get_or_init(|| {
            let baseline = Arc::new(Mutex::new(InputIdleBaseline::default()));
            let session_live = Arc::new(AtomicBool::new(false));
            let last_error = Arc::new(Mutex::new(Some(
                "Sway Wayland idle worker has not connected yet".into(),
            )));
            let b = Arc::clone(&baseline);
            let live = Arc::clone(&session_live);
            let e = Arc::clone(&last_error);
            let _ = thread::Builder::new()
                .name("unfocus-sway-idle".into())
                .spawn(move || idle_worker_loop(b, live, e));
            IdleWorkerHandle {
                baseline,
                session_live,
                last_error,
            }
        })
    }

    pub fn idle_seconds() -> Result<u64, String> {
        ensure_candidate_session()?;
        let handle = worker();
        if !handle.session_live.load(Ordering::Acquire) {
            let error = handle
                .last_error
                .lock()
                .ok()
                .and_then(|guard| guard.clone())
                .unwrap_or_else(|| "Sway Wayland idle worker is not connected".into());
            return Err(error);
        }
        let baseline = handle
            .baseline
            .lock()
            .map_err(|_| "Sway idle baseline lock is poisoned".to_owned())?;
        baseline.idle_seconds(Instant::now())
    }

    pub fn active_window_fullscreen() -> Result<bool, String> {
        let sock = ensure_candidate_session()?;
        let version_payload = ipc_roundtrip(&sock, SWAY_IPC_GET_VERSION, b"")?;
        let version = parse_sway_version_json(&version_payload)?;
        qualify_sway_version(&version).map_err(|error| error.message())?;

        let seats_payload = ipc_roundtrip(&sock, SWAY_IPC_GET_SEATS, b"")?;
        let focus_id = focused_node_id_from_seats_json(&seats_payload)?;

        let tree_payload = ipc_roundtrip(&sock, SWAY_IPC_GET_TREE, b"")?;
        focused_window_is_fullscreen_from_tree(&tree_payload, focus_id)
    }

    pub fn backend_label() -> Option<(String, bool)> {
        let Ok(sock) = ensure_candidate_session() else {
            return None;
        };
        let Ok(payload) = ipc_roundtrip(&sock, SWAY_IPC_GET_VERSION, b"") else {
            return None;
        };
        let Ok(version) = parse_sway_version_json(&payload) else {
            return None;
        };
        if qualify_sway_version(&version).is_err() {
            return None;
        }
        Some((version.human_readable, true))
    }

    fn ensure_candidate_session() -> Result<String, String> {
        let session_type = std::env::var("XDG_SESSION_TYPE").ok();
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
        let sway_sock = std::env::var("SWAYSOCK").ok();
        qualify_session_hints(SessionHints {
            session_type: session_type.as_deref(),
            current_desktop: desktop.as_deref(),
            sway_sock: sway_sock.as_deref(),
        })
        .map_err(|error| error.message())?;
        let sock = sway_sock.expect("qualify_session_hints requires SWAYSOCK");
        validate_sway_sock_path(&sock)?;
        Ok(sock)
    }

    fn validate_sway_sock_path(path: &str) -> Result<(), String> {
        let path = Path::new(path);
        if !path.exists() {
            return Err(format!("SWAYSOCK path {} does not exist", path.display()));
        }
        let meta =
            std::fs::metadata(path).map_err(|error| format!("could not stat SWAYSOCK: {error}"))?;
        let euid = rustix::process::geteuid().as_raw();
        if meta.uid() != euid {
            return Err("SWAYSOCK is not owned by the effective user".into());
        }
        Ok(())
    }

    fn ipc_roundtrip(sock_path: &str, message_type: u32, payload: &[u8]) -> Result<String, String> {
        let mut stream = UnixStream::connect(sock_path)
            .map_err(|error| format!("Sway IPC connect failed: {error}"))?;
        stream
            .set_read_timeout(Some(SWAY_IPC_TIMEOUT))
            .map_err(|error| format!("Sway IPC set_read_timeout failed: {error}"))?;
        stream
            .set_write_timeout(Some(SWAY_IPC_TIMEOUT))
            .map_err(|error| format!("Sway IPC set_write_timeout failed: {error}"))?;

        let request = encode_ipc_request(message_type, payload)?;
        stream
            .write_all(&request)
            .map_err(|error| format!("Sway IPC write failed: {error}"))?;

        let mut header = [0u8; 14];
        stream
            .read_exact(&mut header)
            .map_err(|error| format!("Sway IPC header read failed: {error}"))?;
        let (payload_len, reply_type) = decode_ipc_header(&header)?;
        if reply_type != message_type {
            return Err(format!(
                "Sway IPC reply type {reply_type} does not match expected {message_type}"
            ));
        }

        let mut body = vec![0u8; payload_len];
        if payload_len > 0 {
            stream
                .read_exact(&mut body)
                .map_err(|error| format!("Sway IPC payload read failed: {error}"))?;
        }
        validate_ipc_reply(message_type, reply_type, &body, payload_len)?;
        String::from_utf8(body).map_err(|error| format!("Sway IPC payload is not UTF-8: {error}"))
    }

    fn idle_worker_loop(
        baseline: Arc<Mutex<InputIdleBaseline>>,
        session_live: Arc<AtomicBool>,
        last_error: Arc<Mutex<Option<String>>>,
    ) {
        loop {
            match run_idle_session(&baseline, &session_live) {
                Ok(()) => {
                    session_live.store(false, Ordering::Release);
                    if let Ok(mut idle) = baseline.lock() {
                        idle.clear();
                    }
                    if let Ok(mut error) = last_error.lock() {
                        *error = Some("Sway Wayland idle session ended".into());
                    }
                }
                Err(message) => {
                    session_live.store(false, Ordering::Release);
                    if let Ok(mut idle) = baseline.lock() {
                        idle.clear();
                    }
                    if let Ok(mut error) = last_error.lock() {
                        *error = Some(message);
                    }
                }
            }
            thread::sleep(Duration::from_secs(2));
        }
    }

    fn run_idle_session(
        baseline: &Arc<Mutex<InputIdleBaseline>>,
        session_live: &Arc<AtomicBool>,
    ) -> Result<(), String> {
        let session_type = std::env::var("XDG_SESSION_TYPE").ok();
        let desktop = std::env::var("XDG_CURRENT_DESKTOP").ok();
        let sway_sock = std::env::var("SWAYSOCK").ok();
        qualify_session_hints(SessionHints {
            session_type: session_type.as_deref(),
            current_desktop: desktop.as_deref(),
            sway_sock: sway_sock.as_deref(),
        })
        .map_err(|error| error.message())?;

        let conn = Connection::connect_to_env()
            .map_err(|error| format!("Wayland connection failed: {error}"))?;
        let mut event_queue: EventQueue<IdleApp> = conn.new_event_queue();
        let qh = event_queue.handle();
        let display = conn.display();
        display.get_registry(&qh, ());

        let mut app = IdleApp {
            unbound_seats: Vec::new(),
            seat0: None,
            notifier: None,
            notification: None,
            baseline: Arc::clone(baseline),
        };

        event_queue
            .roundtrip(&mut app)
            .map_err(|error| format!("Wayland registry roundtrip failed: {error}"))?;
        // Seat names arrive as events after bind.
        event_queue
            .roundtrip(&mut app)
            .map_err(|error| format!("Wayland seat name roundtrip failed: {error}"))?;

        let seat = app
            .seat0
            .clone()
            .ok_or_else(|| QualificationError::MissingSeat0.message())?;
        let notifier = app
            .notifier
            .clone()
            .ok_or_else(|| QualificationError::MissingIdleNotifier.message())?;

        app.notification = Some(notifier.get_input_idle_notification(0, &seat, &qh, ()));

        if let Ok(mut idle) = baseline.lock() {
            idle.clear();
        }
        session_live.store(true, Ordering::Release);

        loop {
            event_queue
                .blocking_dispatch(&mut app)
                .map_err(|error| format!("Wayland dispatch failed: {error}"))?;
        }
    }

    struct IdleApp {
        unbound_seats: Vec<wl_seat::WlSeat>,
        seat0: Option<wl_seat::WlSeat>,
        notifier: Option<ext_idle_notifier_v1::ExtIdleNotifierV1>,
        notification: Option<ext_idle_notification_v1::ExtIdleNotificationV1>,
        baseline: Arc<Mutex<InputIdleBaseline>>,
    }

    impl Dispatch<wl_registry::WlRegistry, ()> for IdleApp {
        fn event(
            state: &mut Self,
            registry: &wl_registry::WlRegistry,
            event: wl_registry::Event,
            _: &(),
            _: &Connection,
            qh: &QueueHandle<Self>,
        ) {
            if let wl_registry::Event::Global {
                name,
                interface,
                version,
            } = event
            {
                if interface == "wl_seat" && version >= 2 {
                    // Version 2+ sends the seat name event required for seat0 matching.
                    let seat: wl_seat::WlSeat = registry.bind(name, version.min(9), qh, ());
                    state.unbound_seats.push(seat);
                } else if interface == EXT_IDLE_NOTIFIER_NAME
                    && qualify_idle_notifier_version(version).is_ok()
                    && state.notifier.is_none()
                {
                    let notifier = registry.bind::<ext_idle_notifier_v1::ExtIdleNotifierV1, _, _>(
                        name,
                        version.min(2),
                        qh,
                        (),
                    );
                    state.notifier = Some(notifier);
                }
            }
        }
    }

    impl Dispatch<wl_seat::WlSeat, ()> for IdleApp {
        fn event(
            state: &mut Self,
            seat: &wl_seat::WlSeat,
            event: wl_seat::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let wl_seat::Event::Name { name } = event {
                if name.eq_ignore_ascii_case(REQUIRED_SEAT_NAME) && state.seat0.is_none() {
                    state.seat0 = Some(seat.clone());
                }
                state.unbound_seats.retain(|pending| pending != seat);
            }
        }
    }

    impl Dispatch<ext_idle_notifier_v1::ExtIdleNotifierV1, ()> for IdleApp {
        fn event(
            _: &mut Self,
            _: &ext_idle_notifier_v1::ExtIdleNotifierV1,
            _: ext_idle_notifier_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
        }
    }

    impl Dispatch<ext_idle_notification_v1::ExtIdleNotificationV1, ()> for IdleApp {
        fn event(
            state: &mut Self,
            _: &ext_idle_notification_v1::ExtIdleNotificationV1,
            event: ext_idle_notification_v1::Event,
            _: &(),
            _: &Connection,
            _: &QueueHandle<Self>,
        ) {
            if let Ok(mut idle) = state.baseline.lock() {
                match event {
                    ext_idle_notification_v1::Event::Idled => {
                        idle.on_idled(Instant::now());
                    }
                    ext_idle_notification_v1::Event::Resumed => {
                        idle.on_resumed();
                    }
                    _ => {}
                }
            }
        }
    }
}

#[cfg(all(target_os = "linux", feature = "wayland-sway"))]
pub(super) use runtime::{active_window_fullscreen, backend_label, idle_seconds};

#[cfg(all(target_os = "linux", not(feature = "wayland-sway")))]
pub(super) fn wayland_feature_disabled_error() -> String {
    QualificationError::FeatureDisabled.message()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_hints_require_wayland_sway_and_swaysock() {
        assert!(qualify_session_hints(SessionHints {
            session_type: Some("wayland"),
            current_desktop: Some("sway"),
            sway_sock: Some("/run/user/1000/sway-ipc.sock"),
        })
        .is_ok());
        assert_eq!(
            qualify_session_hints(SessionHints {
                session_type: Some("x11"),
                current_desktop: Some("sway"),
                sway_sock: Some("/tmp/s"),
            }),
            Err(QualificationError::NotWayland)
        );
        assert_eq!(
            qualify_session_hints(SessionHints {
                session_type: Some("wayland"),
                current_desktop: Some("GNOME"),
                sway_sock: Some("/tmp/s"),
            }),
            Err(QualificationError::DesktopNotSway)
        );
        assert!(desktop_has_sway_token("sway:wlroots"));
        assert!(desktop_has_sway_token("SWAY"));
        assert!(!desktop_has_sway_token("swayfoo"));
        assert!(!desktop_has_sway_token("notsway"));
    }

    #[test]
    fn sway_version_floor_is_1_11() {
        let old = SwayVersion {
            major: 1,
            minor: 10,
            patch: 1,
            human_readable: "1.10.1".into(),
        };
        let ok = SwayVersion {
            major: 1,
            minor: 11,
            patch: 0,
            human_readable: "1.11".into(),
        };
        assert!(qualify_sway_version(&old).is_err());
        assert!(qualify_sway_version(&ok).is_ok());
        assert!(ok.meets_minimum());
        assert!(!old.meets_minimum());
    }

    #[test]
    fn parse_version_json_reads_human_readable() {
        let version = parse_sway_version_json(
            r#"{"major":1,"minor":11,"patch":0,"human_readable":"1.11.0"}"#,
        )
        .unwrap();
        assert_eq!(version.major, 1);
        assert_eq!(version.minor, 11);
        assert_eq!(version.human_readable, "1.11.0");
    }

    #[test]
    fn ipc_header_roundtrip_rejects_bad_magic_and_oversize() {
        let encoded = encode_ipc_request(SWAY_IPC_GET_VERSION, b"").unwrap();
        assert_eq!(&encoded[..6], b"i3-ipc");
        let mut header = [0u8; 14];
        header.copy_from_slice(&encoded[..14]);
        let (len, ty) = decode_ipc_header(&header).unwrap();
        assert_eq!((len, ty), (0, SWAY_IPC_GET_VERSION));

        header[0] = b'x';
        assert!(decode_ipc_header(&header).is_err());
    }

    #[test]
    fn seats_json_requires_unique_seat0_with_focus() {
        let payload = r#"[
            {"name":"seat0","focus":42},
            {"name":"seat1","focus":0}
        ]"#;
        assert_eq!(focused_node_id_from_seats_json(payload).unwrap(), 42);
        assert!(focused_node_id_from_seats_json(r#"[{"name":"seat0","focus":0}]"#).is_err());
        assert!(focused_node_id_from_seats_json(r#"[{"name":"seat1","focus":1}]"#).is_err());
    }

    #[test]
    fn tree_fullscreen_follows_focus_and_ancestors() {
        // focus 2 is fullscreen_mode 1 on the leaf.
        let tree = r#"{
            "id": 1,
            "fullscreen_mode": 0,
            "nodes": [
                {
                    "id": 2,
                    "fullscreen_mode": 1,
                    "nodes": [],
                    "floating_nodes": []
                }
            ],
            "floating_nodes": []
        }"#;
        assert!(focused_window_is_fullscreen_from_tree(tree, 2).unwrap());

        // Maximized is mode 0.
        let maximized = r#"{
            "id": 1,
            "fullscreen_mode": 0,
            "nodes": [{"id": 9, "fullscreen_mode": 0, "nodes": [], "floating_nodes": []}],
            "floating_nodes": []
        }"#;
        assert!(!focused_window_is_fullscreen_from_tree(maximized, 9).unwrap());

        // Ancestor workspace fullscreen counts for a non-fullscreen leaf.
        let ancestor = r#"{
            "id": 1,
            "fullscreen_mode": 1,
            "nodes": [{"id": 3, "fullscreen_mode": 0, "nodes": [], "floating_nodes": []}],
            "floating_nodes": []
        }"#;
        assert!(focused_window_is_fullscreen_from_tree(ancestor, 3).unwrap());

        assert!(focused_window_is_fullscreen_from_tree(tree, 99).is_err());
    }

    #[test]
    fn floating_nodes_are_searched_for_focus() {
        let tree = r#"{
            "id": 1,
            "fullscreen_mode": 0,
            "nodes": [],
            "floating_nodes": [
                {"id": 7, "fullscreen_mode": 2, "nodes": [], "floating_nodes": []}
            ]
        }"#;
        assert!(focused_window_is_fullscreen_from_tree(tree, 7).unwrap());
    }

    #[test]
    fn input_idle_baseline_errors_until_first_event() {
        let mut baseline = InputIdleBaseline::default();
        let t0 = Instant::now();
        assert!(baseline.idle_seconds(t0).is_err());

        baseline.on_idled(t0);
        assert_eq!(baseline.idle_seconds(t0).unwrap(), 0);
        assert_eq!(
            baseline.idle_seconds(t0 + Duration::from_secs(5)).unwrap(),
            5
        );

        baseline.on_resumed();
        assert_eq!(
            baseline.idle_seconds(t0 + Duration::from_secs(9)).unwrap(),
            0
        );

        baseline.clear();
        assert!(baseline.idle_seconds(t0).is_err());
    }

    #[test]
    fn idle_notifier_version_floor_is_2() {
        assert!(qualify_idle_notifier_version(2).is_ok());
        assert!(qualify_idle_notifier_version(1).is_err());
    }

    #[test]
    fn unexpected_fullscreen_mode_is_rejected() {
        let tree = r#"{
            "id": 1,
            "fullscreen_mode": 9,
            "nodes": [],
            "floating_nodes": []
        }"#;
        assert!(focused_window_is_fullscreen_from_tree(tree, 1).is_err());
    }
}
