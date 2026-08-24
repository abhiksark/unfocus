# Install Unfocus

This guide covers installing Unfocus prerelease builds on each supported package
type, verifying downloads, first-run steps, local data, uninstall, and common
problems.

Unfocus is a local-first **break** and **reflection** app: it covers your
monitors when it is time to look far away, and it can show a calm **Your day**
summary of presence and break outcomes on this device. It does not require an
account or network after download.

| Document | Purpose |
| --- | --- |
| This page | Install, verify, first run, local data, uninstall |
| [README](../README.md) | Product overview and platform status |
| [CHANGELOG](../CHANGELOG.md) | What changed in each version |
| [Security policy](../.github/SECURITY.md) | Private vulnerability reporting |

## Contents

1. [Before you install](#before-you-install)
2. [Verify every download](#verify-every-download)
3. [Linux (X11)](#linux-x11)
4. [macOS](#macos)
5. [Windows](#windows)
6. [Build from source](#build-from-source-all-platforms)
7. [After install](#after-install)
8. [Local data and clean uninstall](#local-data-and-clean-uninstall)
9. [Privacy reminder](#privacy-reminder)
10. [Getting help](#getting-help)

---

## Before you install

### Platform status (read this first)

Labels describe tested behavior, not only whether a package exists.

| Platform | Status | Install notes |
| --- | --- | --- |
| **Ubuntu / Debian on Linux X11** | **Qualified** | Preferred install: signed APT repo at [apt.abhik.ai](https://apt.abhik.ai). Tray needs an AppIndicator host (on Ubuntu GNOME: **Ubuntu AppIndicators**; see [Tray icon on Linux](#tray-icon-on-linux)). |
| Other Linux **X11** | **Qualified** backend | Same probes and overlays as Ubuntu; install via release `.deb` / `.rpm` / AppImage and verify `SHA256SUMS`. |
| Linux **Wayland** | **Unsupported** | Packages may start, but idle and fullscreen probes are not qualified. Do not treat Wayland as supported. |
| Windows x64 | Early build | Installers ship; interactive multi-monitor qualification is still pending. Idle and fullscreen probes are implemented in current prereleases. Not application code-signed. |
| macOS 11+ (Apple silicon and Intel) | Preview | Intentionally tray/menu-bar-only while running: no Dock icon or application menu. Reopen or focus the dashboard from the tray icon. Physical multi-monitor acceptance is unfinished. Builds are ad-hoc signed but not Developer ID-signed or notarized. |

### Package trust and install sources

| Path | Trust model | Where to get it |
| --- | --- | --- |
| **Ubuntu / Debian APT** (preferred on those systems) | Archive signed with the Unfocus OpenPGP key; `apt` verifies `InRelease` on every update. Optional `.deb.asc` next to each pool package. | [apt.abhik.ai](https://apt.abhik.ai); see [Debian / Ubuntu (APT)](#debian--ubuntu-apt-preferred) |
| GitHub release assets (`.deb`, `.rpm`, AppImage, DMG, setup/MSI) | Verify `SHA256SUMS` (required). Optional GitHub build-provenance attestations. | [GitHub Releases](https://github.com/abhiksark/unfocus/releases); **published** prereleases only, not draft releases |
| macOS Homebrew cask | Homebrew’s cask install path | `brew install --cask abhiksark/unfocus/unfocus@beta` |

macOS app bundles are ad-hoc signed but not Developer ID-signed or notarized.
Windows packages are **not** application code-signed. Ubuntu/Debian APT is the
path that is archive-signed today. That is normal Linux third-party repository
signing, not Apple Developer ID or Windows Authenticode.

### Preinstalls

| Audience | Linux | macOS | Windows |
| --- | --- | --- | --- |
| **End user (release package)** | APT, `.deb`, and `.rpm` installs resolve their GTK 3, WebKitGTK 4.1, AppIndicator, and libc runtime packages automatically; no manual preinstall. AppImage may need your distribution's FUSE / AppImage support. A tray host is recommended on GNOME (below). | No separate runtime install on macOS 11 or later; Unfocus uses macOS-provided AppKit and WebKit. No Xcode. | No separate runtime install on a normal Windows 10/11 x64 system. Unfocus uses WebView2; if it is missing, the setup installer downloads Microsoft's bootstrapper. |
| **Developer (build from source)** | WebKitGTK 4.1, AppIndicator, libxdo, and related deps; see [Build from source](#build-from-source-all-platforms) and [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/#linux). | Xcode Command Line Tools; see [Tauri macOS prerequisites](https://v2.tauri.app/start/prerequisites/#macos). | MSVC C++ build tools and WebView2 where required; see [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows). |

### Choose a package

**Ubuntu or Debian (X11):** use the [APT source](#debian--ubuntu-apt-preferred)
first. You do not need a GitHub download for the normal path.

**Everyone else:** take the **latest published prerelease** (or stable release
when one exists) from:

**https://github.com/abhiksark/unfocus/releases**

Filenames embed the full version (example: `0.6.0-beta.1`). Replace
`VERSION` in the commands below with that string, or download the matching
file from the release page in a browser. The release tag is the same string
with a `v` prefix (example: `v0.6.0-beta.1`).

| You have | Download or install path |
| --- | --- |
| **Ubuntu or Debian (X11)** | **APT** (`apt install unfocus` via [apt.abhik.ai](https://apt.abhik.ai)) preferred; or `Unfocus_VERSION_amd64.deb` from releases |
| Fedora, RHEL, or similar (X11) | `Unfocus-VERSION-1.x86_64.rpm` |
| Portable Linux (X11) | `Unfocus_VERSION_amd64.AppImage` |
| macOS Apple silicon | `Unfocus_VERSION_aarch64.dmg` or Homebrew |
| macOS Intel | `Unfocus_VERSION_x64.dmg` or Homebrew |
| Windows (normal install) | `Unfocus_VERSION_x64-setup.exe` |
| Windows (managed / MSI) | `Unfocus_VERSION_x64_en-US.msi` |
| Checksums for every GitHub asset | `SHA256SUMS` |
| License texts for dependencies | `THIRD_PARTY_NOTICES.txt` |
| Software bill of materials | `unfocus.cdx.json` |

Unfocus does **not** update itself in-app. On Ubuntu/Debian use `apt upgrade`
(or `apt install --only-upgrade unfocus`). Elsewhere use `brew upgrade` or a
newer package from the releases page.

---

## Verify every download

**APT users on Ubuntu/Debian:** `apt update` already checks the signed
repository metadata. You can skip this section for the normal `apt install
unfocus` path. Use the optional [`.deb.asc` offline check](#debian--ubuntu-apt-preferred)
if you download a pool package by hand. For GitHub release assets (manual
`.deb`, `.rpm`, AppImage, DMG, Windows installers), do the steps below before
you install or run anything.

### 1. Download the package and `SHA256SUMS`

From the same **published** release tag as the package (example tag
`v0.6.0-beta.1`):

```sh
# Example version; use the version from the release page.
VERSION=0.6.0-beta.1
BASE="https://github.com/abhiksark/unfocus/releases/download/v${VERSION}"

curl -fsSL -O "${BASE}/SHA256SUMS"
# Also download the package you need, e.g.:
# curl -fsSL -O "${BASE}/Unfocus_${VERSION}_amd64.deb"
# curl -fsSL -O "${BASE}/Unfocus_${VERSION}_amd64.AppImage"
# curl -fsSL -O "${BASE}/Unfocus-${VERSION}-1.x86_64.rpm"
# curl -fsSL -O "${BASE}/Unfocus_${VERSION}_aarch64.dmg"
# curl -fsSL -O "${BASE}/Unfocus_${VERSION}_x64-setup.exe"
```

### 2. Check the checksum

**Linux / macOS:**

```sh
# Verifies only files present in the current directory
sha256sum -c SHA256SUMS --ignore-missing
# macOS may need:
# shasum -a 256 -c SHA256SUMS --ignore-missing
```

**Windows (PowerShell), for one file:**

```powershell
Get-FileHash .\Unfocus_0.6.0-beta.1_x64-setup.exe -Algorithm SHA256
# Compare the hash to the matching line in SHA256SUMS
```

If a checksum does not match, **do not install** the file. Re-download from the
official release page or open a bug report.

### 3. Optional: build provenance

GitHub Actions attaches **build provenance attestations** to release assets.
Checksums are the minimum bar; attestations add supply-chain evidence that the
file was built by the repository’s release workflow.

You can inspect attestations on the release page in the GitHub UI. With a
recent [GitHub CLI](https://cli.github.com/) that supports attestations:

```sh
# Example for a Debian package; use the file you actually downloaded.
gh attestation verify "./Unfocus_${VERSION}_amd64.deb" --repo abhiksark/unfocus
```

If your `gh` build does not include `attestation`, use the release UI or upgrade
the CLI. Checksum verification remains required either way.

---

## Linux (X11)

**Ubuntu on X11** is the primary qualified install target. Other Linux X11
desktops share the same qualified probes and multi-monitor overlay path when
they provide an AppIndicator host. Use an **X11** session. Wayland is
unsupported for probes and is not a qualified install target.

### Confirm an X11 session

```sh
echo "$XDG_SESSION_TYPE"
# Expect: x11
```

If the value is `wayland` (or empty) and you need qualified probe behavior,
sign out and choose an **X11 / Xorg** session at the display manager login
screen. On Ubuntu GNOME, that is often a gear menu on the password screen with
a label such as **Ubuntu on Xorg**. Exact labels vary by distribution and
desktop.

### Debian / Ubuntu (APT, preferred)

This is the supported day-to-day install path on Ubuntu and Debian.

Install the beta from the public APT repository at
[https://apt.abhik.ai](https://apt.abhik.ai). The suite name is **`beta`**
(architecture **amd64** only for now).

**What “signed” means here:** the **APT archive** is signed with the Unfocus
archive OpenPGP key (`public-key.asc`). That is not Apple Developer ID or
Windows Authenticode application code signing. `apt update` checks the
`InRelease` signature before it trusts package lists. Each pool `.deb` also
has a matching detached signature (`.deb.asc`) for optional offline checks.
That is the normal Linux trust model for third-party repositories.

```sh
curl -fsSL https://apt.abhik.ai/public-key.asc \
  | sudo gpg --dearmor -o /usr/share/keyrings/unfocus-archive-keyring.gpg

echo 'deb [arch=amd64 signed-by=/usr/share/keyrings/unfocus-archive-keyring.gpg] https://apt.abhik.ai beta main' \
  | sudo tee /etc/apt/sources.list.d/unfocus-beta.list

sudo apt update
sudo apt install unfocus
```

**Optional offline check** of a downloaded package (after importing the archive
key once):

```sh
DEB_VERSION=0.6.0~beta.1-1
DEB_FILE="unfocus_${DEB_VERSION}_amd64.deb"
curl -fsSL -O "https://apt.abhik.ai/pool/beta/u/unfocus/${DEB_FILE}"
curl -fsSL -O "https://apt.abhik.ai/pool/beta/u/unfocus/${DEB_FILE}.asc"
gpg --no-default-keyring \
  --keyring /usr/share/keyrings/unfocus-archive-keyring.gpg \
  --verify "${DEB_FILE}.asc" "$DEB_FILE"
```

Launch from the application menu as **Unfocus**, or:

```sh
unfocus
```

**Upgrade:**

```sh
sudo apt update
sudo apt install --only-upgrade unfocus
```

#### Migrate from the alpha APT channel

The alpha suite remains frozen and does not receive beta packages. Existing
alpha users can explicitly move to beta with:

```sh
sudo rm -f /etc/apt/sources.list.d/unfocus-alpha.list
echo 'deb [arch=amd64 signed-by=/usr/share/keyrings/unfocus-archive-keyring.gpg] https://apt.abhik.ai beta main' \
  | sudo tee /etc/apt/sources.list.d/unfocus-beta.list
sudo apt update
sudo apt install --only-upgrade unfocus
```

**Remove the package:**

```sh
sudo apt remove unfocus
```

To remove the APT source as well:

```sh
sudo rm -f /etc/apt/sources.list.d/unfocus-beta.list
sudo rm -f /usr/share/keyrings/unfocus-archive-keyring.gpg
sudo apt update
```

Package removal does not always delete local settings. See
[Local data and clean uninstall](#local-data-and-clean-uninstall).

Repository automation and operator notes live in
[abhiksark/unfocus-apt](https://github.com/abhiksark/unfocus-apt). Use the
manual `.deb` path below for offline install or when verifying a specific
release asset yourself.

### Debian / Ubuntu (manual `.deb`)

Use this for offline install or when you want to verify a specific release
asset yourself.

```sh
VERSION=0.6.0-beta.1
# After downloading Unfocus_${VERSION}_amd64.deb and verifying SHA256SUMS:

sudo apt install "./Unfocus_${VERSION}_amd64.deb"
# or:
# sudo dpkg -i "./Unfocus_${VERSION}_amd64.deb"
# sudo apt-get install -f   # only if dpkg reports missing dependencies
```

**Upgrade:** install a newer `.deb` the same way. Prerelease Debian versions
are ordered so later prereleases and stables can upgrade normally. The filename
keeps the full SemVer (for example `0.6.0-beta.1`); the package’s embedded
Debian `Version` uses tilde ordering (for example `0.6.0~beta.1-1`) so
upgrades sort correctly.

**Remove the package:**

```sh
sudo apt remove unfocus
# or: sudo dpkg -r unfocus
```

### Fedora / RHEL (`.rpm`)

```sh
VERSION=0.6.0-beta.1
# After downloading Unfocus-${VERSION}-1.x86_64.rpm and verifying SHA256SUMS:

sudo dnf install "./Unfocus-${VERSION}-1.x86_64.rpm"
# older hosts may use: sudo rpm -Uvh "./Unfocus-${VERSION}-1.x86_64.rpm"
```

**Remove the package:**

```sh
sudo dnf remove unfocus
```

### Portable AppImage

```sh
VERSION=0.6.0-beta.1
chmod +x "./Unfocus_${VERSION}_amd64.AppImage"
"./Unfocus_${VERSION}_amd64.AppImage"
```

No root install. Keep the file where you want it. To upgrade, download a newer
AppImage, verify it, and replace the old file. Delete the file to remove the
app binary.

Some desktops need FUSE for AppImages. If the image fails to start, install
your distribution’s AppImage / FUSE support, or use the `.deb` / `.rpm` instead.

### Tray icon on Linux

The reminder keeps running from the system tray when you close the dashboard.
The desktop must provide a **StatusNotifier / AppIndicator** host.

- **Ubuntu GNOME:** enable or install the **Ubuntu AppIndicators** extension
  (or equivalent), then restart Unfocus if the icon is missing.
- If tray construction fails, Unfocus shows a known setup error on the
  dashboard and does **not** hide into an unreachable background process.
  Closing the dashboard exits the app in that case so a silent process cannot
  keep running without a way to open it. Keep the dashboard open until the tray
  host works, then restart Unfocus.

### Linux troubleshooting

| Symptom | What to try |
| --- | --- |
| `apt update` rejects the Unfocus source | Confirm the keyring path and `signed-by=` line match the install commands; re-import `public-key.asc` from https://apt.abhik.ai/public-key.asc |
| Checksum mismatch (GitHub asset) | Do not install; re-download from the official release and re-verify |
| No tray icon | On Ubuntu GNOME enable **Ubuntu AppIndicators**; restart Unfocus; check developer mode diagnostics |
| Closing the dashboard exits the app | Tray setup failed; fix the tray host, then restart |
| Wayland session | Unsupported for probes; switch to an X11 session (on Ubuntu: **Ubuntu on Xorg**) for qualified behavior |
| AppImage will not run | `chmod +x`; install FUSE/AppImage support; or use APT / `.deb` / `.rpm` |
| Break never appears while away or fullscreen | Expected when probes work; check **Your day** / developer diagnostics for idle and fullscreen |

---

## macOS

Preview platform. Physical multi-monitor behavior has not completed an
acceptance run. While running, macOS intentionally uses Accessory activation:
Unfocus is tray/menu-bar-only, with no Dock icon or application menu.
Release packages require macOS 11 or later and use the AppKit and WebKit
frameworks included with macOS. **No Xcode or other developer tools are
required** to install a release DMG.

### Which DMG

| Mac | File |
| --- | --- |
| Apple silicon (M1, M2, M3, …) | `Unfocus_VERSION_aarch64.dmg` |
| Intel | `Unfocus_VERSION_x64.dmg` |

Check **Apple menu → About This Mac** if you are unsure.

### Install from the DMG

1. Download the correct DMG and `SHA256SUMS`; verify the checksum (above).
2. Open the DMG and drag **Unfocus** to **Applications** (or your preferred
   location).
3. Eject the DMG.

### First launch (ad-hoc signed / not notarized)

Prerelease builds are ad-hoc signed so the app bundle passes local codesign
verification, but they are **not** Developer ID-signed or notarized.
Gatekeeper will still block a normal double-click the first time.

1. In **Finder**, open **Applications**.
2. **Control-click** (or right-click) **Unfocus**.
3. Choose **Open**, then confirm **Open** again.

Alternatively: open Unfocus once, then **System Settings → Privacy & Security**
and choose **Open Anyway** if macOS offers it.

Later launches can use a normal double-click or Spotlight.

### Tray-only operation

On macOS, the running app intentionally has no Dock icon or application menu.
Use the Unfocus tray/menu-bar icon and choose **Open Unfocus** to reopen or
focus the dashboard. The tray menu also exposes the reminder controls and quit
action.

### Homebrew (beta cask)

If you use Homebrew:

```sh
brew install --cask abhiksark/unfocus/unfocus@beta
```

That needs Homebrew itself. It does not install Xcode. First-run Gatekeeper
steps may still apply depending on how the cask delivers the app; use
Control-click → Open if macOS blocks the app.

Upgrade when a new beta is published:

```sh
brew update
brew upgrade --cask abhiksark/unfocus/unfocus@beta
```

Existing alpha cask installs remain pinned. Migrate explicitly without
`--zap`, which preserves local application data:

```sh
brew uninstall --cask abhiksark/unfocus/unfocus@alpha
brew install --cask abhiksark/unfocus/unfocus@beta
```

### Permissions

Current idle and fullscreen probes are designed **not** to require Screen
Recording consent. Unfocus should not prompt for Screen Recording for those
probes. If a future change needs a new permission, it will be documented
explicitly.

### Remove on macOS

- Drag **Unfocus** from Applications to the Trash, or
- If installed via Homebrew: `brew uninstall --cask abhiksark/unfocus/unfocus@beta`

Removing the app does not always delete local settings. See
[Local data and clean uninstall](#local-data-and-clean-uninstall).

### macOS troubleshooting

| Symptom | What to try |
| --- | --- |
| Checksum mismatch | Do not install; re-download and re-verify |
| “App can’t be opened because it is from an unidentified developer” | Control-click → Open (ad-hoc-signed, unnotarized prerelease) |
| Wrong architecture | Use `aarch64` vs `x64` DMG for your Mac |
| Tray or multi-monitor oddities | Expected gaps while status is Preview; report with the platform report form |

---

## Windows

Early build. Packages are produced for **64-bit Windows**. Interactive
multi-monitor qualification is still pending. Idle and fullscreen probes are
implemented in current prereleases; treat overall Windows support as early, not
fully qualified.

Modern Windows 10 and 11 systems usually already include the WebView2 runtime
that Tauri uses. If WebView2 is missing, the setup installer downloads
Microsoft's bootstrapper; the installed Unfocus application still makes no
application-originated runtime network calls.

### Setup installer (`.exe`)

1. Download `Unfocus_VERSION_x64-setup.exe` and `SHA256SUMS`; verify the hash.
2. Run the setup executable.
3. If **SmartScreen** warns that the app is unrecognized (unsigned prerelease):
   choose **More info**, then **Run anyway** only if you verified the checksum
   from the official release.
4. Finish the wizard and start Unfocus from the Start menu.

**Upgrade:** run a newer setup installer from a published release after
verifying its checksum.

### MSI (managed install)

1. Download `Unfocus_VERSION_x64_en-US.msi` and verify `SHA256SUMS`.
2. Double-click the MSI, or for scripted install:

```powershell
msiexec /i Unfocus_0.6.0-beta.1_x64_en-US.msi
```

The MSI **ProductVersion** is the numeric core only (for example `0.6.0` for
`0.6.0-beta.1`) because Windows requires that format. The filename still
carries the full prerelease version.

### SmartScreen and unsigned builds

Prerelease installers are **not code-signed**. SmartScreen warnings are expected.
Always verify `SHA256SUMS` before choosing **Run anyway**.

### Remove on Windows

- **Settings → Apps → Installed apps → Unfocus → Uninstall**, or
- Use the uninstaller entry from the Start menu if present, or
- For MSI: `msiexec /x Unfocus_VERSION_x64_en-US.msi`

Uninstalling the package does not always delete local settings. See
[Local data and clean uninstall](#local-data-and-clean-uninstall).

### Windows troubleshooting

| Symptom | What to try |
| --- | --- |
| Checksum mismatch | Do not install; re-download and re-verify |
| SmartScreen block | Verify checksum; More info → Run anyway |
| Installer will not start | Confirm 64-bit Windows; re-download and re-verify |
| Probes or multi-monitor issues | Early platform; capture details with the platform report form |

---

## Build from source (all platforms)

Use this for development, not as the primary end-user install. End users
should prefer a [release package](#choose-a-package).

### Toolchain pins

| Tool | Pin | Source |
| --- | --- | --- |
| Bun | `1.3.5` | `.bun-version` |
| Rust | `1.98.0` | `rust-toolchain.toml` |

Install those versions (or let `rustup` pick up `rust-toolchain.toml` when you
build in the repo). Use Bun for all JavaScript and TypeScript work in this
repository; do not substitute npm, pnpm, or Yarn for project scripts.

### System prerequisites

Follow the official [Tauri 2 prerequisites](https://v2.tauri.app/start/prerequisites/)
for your OS, then add the project-specific notes below.

**Linux (Debian/Ubuntu-style example, aligned with the repo’s spike container):**

```sh
sudo apt-get update
sudo apt-get install -y \
  build-essential \
  curl \
  file \
  libayatana-appindicator3-dev \
  librsvg2-dev \
  libssl-dev \
  libwebkit2gtk-4.1-dev \
  libxdo-dev \
  pkg-config
```

Other distributions need the equivalent WebKitGTK 4.1, AppIndicator, and libxdo
development packages. Development still expects an **X11** session for the
qualified Linux path.

**macOS:** install the Xcode Command Line Tools.

**Windows:** install the MSVC C++ build tools and WebView2 components described
in the Tauri Windows prerequisites.

### Clone and run

```sh
git clone https://github.com/abhiksark/unfocus.git
cd unfocus
bun install --frozen-lockfile
bun run tauri dev
```

### Optional Linux container helper

When native headers are missing on the host but you have a real X11 display:

```sh
./scripts/run-linux-spike-container.sh
```

That runner is a **development convenience, not a sandbox**. Run it only from a
revision you trust. It can access the host X11 display and session bus. The
repository is mounted read-only except for ignored `src-tauri/target` and
`src-tauri/gen` build directories. The frontend build also runs on the host
before the container starts.

### Contributor checks

Branching rules and the full local application gate are in the
[README](../README.md) and `AGENTS.md`. Do not treat a successful `tauri dev` on
one machine as cross-platform qualification.

---

## After install

### First run

1. Start Unfocus from the application menu, Start menu, Spotlight, or the
   `unfocus` binary (Linux packages).
2. The consumer dashboard shows the next break. Where the idle probe works,
   **Your day** shows a local, presence-only summary of continuous computer use
   versus time away. History exposes the newest 90 local days as a
   compact Monday-aligned calendar of confirmed active minutes. Selecting a day
   reveals its hourly activity and break outcomes, aligned to the saved **Day
   starts** preference (no keylogging, no cloud).
3. Defaults are a **20-minute** work interval and a **20-second** break. Valid
   ranges are work **1-120** minutes and break **3-30** seconds. Changes are
   stored only on this device.
4. Closing the dashboard leaves the reminder in the **tray** when the tray is
   available. If tray setup failed, closing the dashboard **exits** so the
   process cannot keep running without a reachable UI.
5. A second launch focuses the existing window; it does not start a second tray
   or timer.
6. Expand **Advanced** in the timing editor and open **developer mode** only if
   you need raw probe and monitor diagnostics. Developer mode is optional and
   remembered on the device.

### Day-to-day controls

From the dashboard or tray (where available) you can pause reminders for thirty
minutes, resume into a fresh work interval, or start the configured break
immediately. Pause expiry is local and bounded.

### Updates

Unfocus does not auto-update.

**Ubuntu / Debian (APT):**

```sh
sudo apt update
sudo apt install --only-upgrade unfocus
```

**macOS (Homebrew):**

```sh
brew update
brew upgrade --cask abhiksark/unfocus/unfocus@beta
```

**GitHub release packages:** download a newer published asset and verify
`SHA256SUMS` before installing.

---

## Local data and clean uninstall

Unfocus stores settings and reflection data under the app config directory for
bundle identifier `com.unfocus.desktop`. Paths below are the usual locations;
Linux may honor `XDG_CONFIG_HOME` if you set it.

| OS | Typical config directory |
| --- | --- |
| Linux | `~/.config/com.unfocus.desktop/` |
| macOS | `~/Library/Application Support/com.unfocus.desktop/` |
| Windows | `%APPDATA%\com.unfocus.desktop\` |

Files written there include:

| File | Purpose |
| --- | --- |
| `reminder-settings.json` | Work and break timing, pause state |
| `activity-history.json` | Hot local presence / AFK segments for the last 24 hours, feeding **Your day** |
| `activity-archive-<key>.json` | Older presence / AFK segments in fixed 30-day epoch chunks, kept at least 90 days |
| `break-events.json` | Local break outcome ledger, kept at least 90 days |

`activity-archive-<key>.json` files hold the same presence-only data as
`activity-history.json` (no keylogging, window titles, or telemetry), just
older than 24 hours. Each archive file covers one fixed 30-day epoch block, so
effective retention is at least 90 days rather than an exact cutoff. An older
partial chunk can remain until its complete 30-day block expires. Retention
fills forward only: compatible existing local data is preserved, but activity
from before History was installed cannot be backfilled. There may be more than
one archive file as history accumulates. These files stay on this device.
Removing the application package, DMG app bundle, or AppImage does **not**
always delete them.

### Optional full wipe after uninstall

Only after you have uninstalled or deleted the app, remove the config directory
if you want a clean slate:

**Linux:**

```sh
rm -rf ~/.config/com.unfocus.desktop
```

**macOS:**

```sh
rm -rf "$HOME/Library/Application Support/com.unfocus.desktop"
```

**Windows (PowerShell):**

```powershell
Remove-Item -Recurse -Force "$env:APPDATA\com.unfocus.desktop"
```

---

## Privacy reminder

Unfocus has no accounts, telemetry, cloud dependency, or application-originated
runtime network calls. Timing, day history, and break outcomes stay on this
device. Only explicit activation of the linked author name asks the system
browser to open the fixed `https://abhik.ai` address.
Install steps may contact a download host only to fetch the package you chose:
APT users contact [apt.abhik.ai](https://apt.abhik.ai); GitHub release
downloads use GitHub; Homebrew users also contact GitHub (and the Homebrew
infrastructure) for the cask.

---

## Getting help

- [Report a bug](https://github.com/abhiksark/unfocus/issues/new?template=bug_report.yml)
- [Platform report](https://github.com/abhiksark/unfocus/issues/new?template=platform_report.yml)
  (acceptance evidence on a given OS or monitor setup)
- [Security policy](https://github.com/abhiksark/unfocus/security/policy) for
  vulnerabilities (private report, not a public issue)
