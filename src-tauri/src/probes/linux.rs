#[cfg(target_os = "linux")]
use x11rb::{
    connection::Connection,
    protocol::{
        screensaver::ConnectionExt as ScreensaverConnectionExt,
        xproto::{AtomEnum, ConnectionExt as XprotoConnectionExt, GetPropertyReply, Window},
    },
    rust_connection::RustConnection,
};

const SPIKE_FORCE_PROBE_FAILURE: &str = "UNFOCUS_SPIKE_FORCE_PROBE_FAILURE";

fn validate_spike_probe_override(value: Option<&str>) -> Result<(), String> {
    if value == Some("1") {
        return Err(format!(
            "Linux probe failure injected by {SPIKE_FORCE_PROBE_FAILURE}"
        ));
    }
    Ok(())
}

pub(super) fn validate_session(
    session_type: Option<&str>,
    display: Option<&str>,
) -> Result<(), String> {
    match session_type {
        Some(session) if session.eq_ignore_ascii_case("x11") => {}
        Some(session) => {
            return Err(format!(
                "Linux {session} sessions are unsupported; probes require a qualified X11 session"
            ));
        }
        None => {
            return Err(
                "Linux session type is unavailable; probes require XDG_SESSION_TYPE=x11".into(),
            );
        }
    }

    if display.is_none_or(str::is_empty) {
        return Err("X11 DISPLAY is unavailable".into());
    }

    Ok(())
}

pub(super) fn validate_property(
    name: &str,
    actual_type: u32,
    expected_type: u32,
    format: u8,
    value_len: u32,
    byte_len: usize,
    bytes_after: u32,
) -> Result<(), String> {
    if actual_type != expected_type {
        return Err(format!("{name} has an unexpected X11 property type"));
    }
    if format != 32 {
        return Err(format!("{name} has X11 format {format}, expected 32"));
    }
    let expected_bytes = usize::try_from(value_len)
        .unwrap_or(usize::MAX)
        .saturating_mul(4);
    if byte_len != expected_bytes {
        return Err(format!("{name} has an inconsistent X11 value length"));
    }
    if bytes_after != 0 {
        return Err(format!("{name} returned a truncated X11 value"));
    }

    Ok(())
}

#[cfg(target_os = "linux")]
fn connect() -> Result<(RustConnection, usize), String> {
    validate_spike_probe_override(std::env::var(SPIKE_FORCE_PROBE_FAILURE).ok().as_deref())?;
    validate_session(
        std::env::var("XDG_SESSION_TYPE").ok().as_deref(),
        std::env::var("DISPLAY").ok().as_deref(),
    )?;
    x11rb::connect(None).map_err(|error| format!("X11 connection failed: {error}"))
}

#[cfg(target_os = "linux")]
fn atom(connection: &RustConnection, name: &[u8]) -> Result<u32, String> {
    let atom = connection
        .intern_atom(true, name)
        .map_err(|error| format!("could not request X11 atom: {error}"))?
        .reply()
        .map(|reply| reply.atom)
        .map_err(|error| format!("could not read X11 atom: {error}"))?;
    if atom == 0 {
        return Err(format!(
            "the X11 window manager does not advertise {}",
            String::from_utf8_lossy(name)
        ));
    }
    Ok(atom)
}

#[cfg(target_os = "linux")]
fn root(connection: &RustConnection, screen_number: usize) -> Result<Window, String> {
    connection
        .setup()
        .roots
        .get(screen_number)
        .map(|screen| screen.root)
        .ok_or_else(|| format!("X11 screen {screen_number} is unavailable"))
}

#[cfg(target_os = "linux")]
fn property_values(
    name: &str,
    reply: &GetPropertyReply,
    expected_type: u32,
) -> Result<Vec<u32>, String> {
    validate_property(
        name,
        reply.type_,
        expected_type,
        reply.format,
        reply.value_len,
        reply.value.len(),
        reply.bytes_after,
    )?;
    reply
        .value32()
        .map(|values| values.collect())
        .ok_or_else(|| format!("{name} does not contain 32-bit X11 values"))
}

#[cfg(target_os = "linux")]
fn verify_ewmh_support(
    connection: &RustConnection,
    root: Window,
    required_atoms: &[u32],
) -> Result<(), String> {
    let supported_atom = atom(connection, b"_NET_SUPPORTED")?;
    let reply = connection
        .get_property(false, root, supported_atom, AtomEnum::ATOM, 0, u32::MAX)
        .map_err(|error| format!("EWMH support query failed: {error}"))?
        .reply()
        .map_err(|error| format!("EWMH support reply failed: {error}"))?;
    let supported = property_values("_NET_SUPPORTED", &reply, AtomEnum::ATOM.into())?;

    if let Some(missing) = required_atoms
        .iter()
        .find(|required| !supported.contains(required))
    {
        return Err(format!(
            "the X11 window manager does not advertise required EWMH atom {missing}"
        ));
    }

    Ok(())
}

#[cfg(target_os = "linux")]
pub(super) fn idle_seconds() -> Result<u64, String> {
    let (connection, screen_number) = connect()?;
    let reply = connection
        .screensaver_query_info(root(&connection, screen_number)?)
        .map_err(|error| format!("XScreenSaver query failed: {error}"))?
        .reply()
        .map_err(|error| format!("XScreenSaver reply failed: {error}"))?;

    Ok(u64::from(reply.ms_since_user_input) / 1_000)
}

#[cfg(target_os = "linux")]
pub(super) fn active_window_fullscreen() -> Result<bool, String> {
    let (connection, screen_number) = connect()?;
    let root = root(&connection, screen_number)?;
    let active_window_atom = atom(&connection, b"_NET_ACTIVE_WINDOW")?;
    let window_state_atom = atom(&connection, b"_NET_WM_STATE")?;
    let fullscreen_atom = atom(&connection, b"_NET_WM_STATE_FULLSCREEN")?;
    verify_ewmh_support(
        &connection,
        root,
        &[active_window_atom, window_state_atom, fullscreen_atom],
    )?;

    let active_window_reply = connection
        .get_property(false, root, active_window_atom, AtomEnum::WINDOW, 0, 1)
        .map_err(|error| format!("active-window query failed: {error}"))?
        .reply()
        .map_err(|error| format!("active-window reply failed: {error}"))?;
    let active_windows = property_values(
        "_NET_ACTIVE_WINDOW",
        &active_window_reply,
        AtomEnum::WINDOW.into(),
    )?;
    if active_windows.len() != 1 {
        return Err("_NET_ACTIVE_WINDOW must contain exactly one window".into());
    }
    let active_window = active_windows[0];
    if active_window == 0 {
        return Ok(false);
    }

    let state_reply = connection
        .get_property(
            false,
            active_window,
            window_state_atom,
            AtomEnum::ATOM,
            0,
            u32::MAX,
        )
        .map_err(|error| format!("window-state query failed: {error}"))?
        .reply()
        .map_err(|error| format!("window-state reply failed: {error}"))?;
    let states = property_values("_NET_WM_STATE", &state_reply, AtomEnum::ATOM.into())?;

    Ok(states.contains(&fullscreen_atom))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn linux_probe_gate_accepts_only_explicit_x11_sessions() {
        assert_eq!(validate_session(Some("x11"), Some(":0")), Ok(()));
        assert_eq!(validate_session(Some("X11"), Some(":1")), Ok(()));
        assert!(validate_session(Some("wayland"), Some(":0")).is_err());
        assert!(validate_session(None, Some(":0")).is_err());
        assert!(validate_session(Some("x11"), None).is_err());
        assert!(validate_session(Some("x11"), Some("")).is_err());
    }

    #[test]
    fn linux_probe_failure_can_be_injected_without_changing_the_session_backend() {
        assert_eq!(validate_spike_probe_override(None), Ok(()));
        assert_eq!(validate_spike_probe_override(Some("0")), Ok(()));
        assert!(validate_spike_probe_override(Some("1")).is_err());
    }

    #[test]
    fn x11_property_metadata_must_be_typed_and_well_formed() {
        assert_eq!(validate_property("property", 4, 4, 32, 2, 8, 0), Ok(()));
        assert!(validate_property("property", 0, 4, 32, 2, 8, 0).is_err());
        assert!(validate_property("property", 4, 4, 8, 2, 8, 0).is_err());
        assert!(validate_property("property", 4, 4, 32, 2, 4, 0).is_err());
        assert!(validate_property("property", 4, 4, 32, 2, 8, 4).is_err());
    }
}
