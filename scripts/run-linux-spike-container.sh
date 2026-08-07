#!/usr/bin/env bash
set -euo pipefail

if [[ "${XDG_SESSION_TYPE:-}" != "x11" ]]; then
  echo "This spike runner requires an active X11 session." >&2
  exit 1
fi

project_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
host_uid="$(id -u)"
host_gid="$(id -g)"
runtime_dir="/run/user/${host_uid}"
xauthority_file="${XAUTHORITY:-${runtime_dir}/gdm/Xauthority}"
container_name="unfocus-linux-spike-running"
display_number="${DISPLAY#:}"
display_number="${display_number%%.*}"

if [[ ! -S "/tmp/.X11-unix/X${display_number}" ]]; then
  echo "The X11 socket for DISPLAY=${DISPLAY:-unset} is unavailable." >&2
  exit 1
fi

if [[ ! -r "${xauthority_file}" ]]; then
  echo "Cannot read Xauthority file: ${xauthority_file}" >&2
  exit 1
fi

if [[ ! -S "${runtime_dir}/bus" ]]; then
  echo "The session D-Bus socket is unavailable: ${runtime_dir}/bus" >&2
  exit 1
fi

if docker container inspect "${container_name}" >/dev/null 2>&1; then
  echo "${container_name} already exists; stop it before starting another copy." >&2
  exit 1
fi

cd "${project_dir}"
echo "Warning: this development container can access your X11 session and session bus." >&2
echo "Run it only from a revision you trust; it is not a desktop security sandbox." >&2
bun run build
mkdir -p src-tauri/target src-tauri/gen

docker build \
  --build-arg "HOST_UID=${host_uid}" \
  --build-arg "HOST_GID=${host_gid}" \
  --file Dockerfile.linux-spike \
  --tag unfocus-linux-spike \
  .

# Docker's default AppArmor profile blocks access to the host session bus. This local,
# development-only container is unconfined so the AppIndicator tray item can register.
exec docker run --rm \
  --name "${container_name}" \
  --security-opt apparmor=unconfined \
  --env "DISPLAY=${DISPLAY}" \
  --env "XAUTHORITY=/tmp/unfocus.Xauthority" \
  --env "XDG_SESSION_TYPE=${XDG_SESSION_TYPE}" \
  --env "XDG_CURRENT_DESKTOP=${XDG_CURRENT_DESKTOP:-}" \
  --env "DBUS_SESSION_BUS_ADDRESS=unix:path=${runtime_dir}/bus" \
  --env "NO_AT_BRIDGE=1" \
  --env "WEBKIT_DISABLE_COMPOSITING_MODE=1" \
  --volume "/tmp/.X11-unix:/tmp/.X11-unix:rw" \
  --volume "${xauthority_file}:/tmp/unfocus.Xauthority:ro" \
  --volume "${runtime_dir}/bus:${runtime_dir}/bus" \
  --volume "${project_dir}:/workspace:ro" \
  --volume "${project_dir}/src-tauri/target:/workspace/src-tauri/target:rw" \
  --volume "${project_dir}/src-tauri/gen:/workspace/src-tauri/gen:rw" \
  unfocus-linux-spike
