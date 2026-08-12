# Install Unfocus

This guide covers installing Unfocus alpha builds on each supported package
type, verifying downloads, first-run steps, uninstall, and common problems.

Unfocus is a local-first **break** and **reflection** app: it covers your
monitors when it is time to look far away, and it can show a calm **Your day**
summary of presence and break outcomes on this device. It does not require an
account or network after download.

| Document | Purpose |
| --- | --- |
| This page | Install, verify, first run, uninstall |
| [README](../README.md) | Product overview and platform status |
| [CHANGELOG](../CHANGELOG.md) | What changed in each version |
| [Security policy](../.github/SECURITY.md) | Private vulnerability reporting |

## Before you install

### Platform status (read this first)

Labels describe tested behavior, not only whether a package exists.

| Platform | Status | Install notes |
| --- | --- | --- |
| Linux **X11** | **Qualified** | Preferred Linux path. Tray needs an AppIndicator host (see below). |
| Linux **Wayland** | **Unsupported** | Packages may start, but idle/fullscreen probes are not qualified. Do not treat Wayland as supported. |
| Windows x64 | Early build | Installers ship; interactive multi-monitor qualification is still pending. |
| macOS (Apple silicon and Intel) | Preview | Multi-monitor acceptance has not completed. Builds are not notarized. |

### Preinstalls

| Audience | Linux | macOS | Windows |
| --- | --- | --- | --- |
| **End user (release package)** | No extra libraries for the `.deb` / `.rpm` / AppImage. Tray host recommended on GNOME (below). | **None.** No Xcode. | **None** beyond a normal Windows 10/11 x64 system. |
| **Developer (build from source)** | WebKitGTK 4.1, AppIndicator, libxdo, and related deps; see [Tauri Linux prerequisites](https://v2.tauri.app/start/prerequisites/#linux). | Xcode Command Line Tools; see [Tauri macOS prerequisites](https://v2.tauri.app/start/prerequisites/#macos). | MSVC C++ build tools and WebView2 where required; see [Tauri Windows prerequisites](https://v2.tauri.app/start/prerequisites/#windows). |

Release packages are **not code-signed or notarized**. Install only from the
official GitHub Releases page for this repository.

### Choose a package

Always take the **latest published prerelease** (or stable release when one
exists) from:

**https://github.com/abhiksark/unfocus/releases**

Filenames embed the full version (example: `0.3.1-alpha.1`). Replace
`VERSION` in the commands below with that string, or download the matching
file from the release page in a browser.

| You have | Download |
| --- | --- |
| Debian, Ubuntu, or similar (X11) | `Unfocus_VERSION_amd64.deb` |
| Fedora, RHEL, or similar (X11) | `Unfocus-VERSION-1.x86_64.rpm` |
| Portable Linux (X11) | `Unfocus_VERSION_amd64.AppImage` |
| macOS Apple silicon | `Unfocus_VERSION_aarch64.dmg` |
| macOS Intel | `Unfocus_VERSION_x64.dmg` |
| Windows (normal install) | `Unfocus_VERSION_x64-setup.exe` |
| Windows (managed / MSI) | `Unfocus_VERSION_x64_en-US.msi` |
| Checksums for every asset | `SHA256SUMS` |
| License texts for dependencies | `THIRD_PARTY_NOTICES.txt` |
| Software bill of materials | `unfocus.cdx.json` |

macOS can also install the alpha through Homebrew (see [macOS](#macos)).

Unfocus does **not** update itself yet. Check the releases page for new builds.

---

## Verify every download

Do this for every package, on every OS, before you install or run it.

### 1. Download the package and `SHA256SUMS`

From the same release tag as the package (example tag `v0.3.1-alpha.1`):

```sh
# Example version; use the version from the release page.
VERSION=0.3.1-alpha.1
BASE="https://github.com/abhiksark/unfocus/releases/download/v${VERSION}"

curl -fsSL -O "${BASE}/SHA256SUMS"
# Also download the package you need, e.g.:
# curl -fsSL -O "${BASE}/Unfocus_${VERSION}_amd64.deb"
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
Get-FileHash .\Unfocus_0.3.1-alpha.1_x64-setup.exe -Algorithm SHA256
# Compare the hash to the matching line in SHA256SUMS
```

### 3. Optional: build provenance

GitHub Actions attaches **build provenance attestations** to release assets.
You can inspect them on the release UI or with the GitHub CLI if you use it.
Checksums are the minimum bar; attestations add supply-chain evidence.

If a checksum does not match, do not install the file. Re-download from the
official release page or open a bug report.

---

## Linux (X11)

Qualified backend. Use an **X11** session (`echo $XDG_SESSION_TYPE` should
print `x11`). Wayland is unsupported for probes and is not a qualified install
target.

### Debian / Ubuntu (`.deb`)

```sh
VERSION=0.3.1-alpha.1
# After downloading Unfocus_${VERSION}_amd64.deb and verifying SHA256SUMS:

sudo apt install "./Unfocus_${VERSION}_amd64.deb"
# or:
# sudo dpkg -i "./Unfocus_${VERSION}_amd64.deb"
# sudo apt-get install -f   # only if dpkg reports missing dependencies
```

Launch from the application menu as **Unfocus**, or:

```sh
unfocus
```

**Upgrade:** install a newer `.deb` the same way. Prerelease Debian versions
are ordered so later alphas and stables can upgrade normally
(for example `0.3.1-alpha.1` embeds as a Debian-ordered version).

**Remove:**

```sh
sudo apt remove unfocus
# or: sudo dpkg -r unfocus
```

### Fedora / RHEL (`.rpm`)

```sh
VERSION=0.3.1-alpha.1
# After downloading Unfocus-${VERSION}-1.x86_64.rpm and verifying SHA256SUMS:

sudo dnf install "./Unfocus-${VERSION}-1.x86_64.rpm"
# older hosts may use: sudo rpm -Uvh "./Unfocus-${VERSION}-1.x86_64.rpm"
```

**Remove:**

```sh
sudo dnf remove unfocus
```

### Portable AppImage

```sh
VERSION=0.3.1-alpha.1
chmod +x "./Unfocus_${VERSION}_amd64.AppImage"
"./Unfocus_${VERSION}_amd64.AppImage"
```

No root install. Keep the file where you want it; delete the file to “uninstall.”
Some desktops need FUSE for AppImages; if the image fails to start, install
your distribution’s AppImage / FUSE support, or use the `.deb` / `.rpm` instead.

### Tray icon on Linux

The reminder keeps running from the system tray when you close the dashboard.
The desktop must provide a **StatusNotifier / AppIndicator** host.

- **Ubuntu GNOME:** enable or install the **Ubuntu AppIndicators** extension
  (or equivalent), then restart Unfocus if the icon is missing.
- If tray construction fails, Unfocus shows a known setup error on the
  dashboard and does not hide into an unreachable background process. Keep the
  dashboard open until the tray host works, then restart Unfocus.

### Linux troubleshooting

| Symptom | What to try |
| --- | --- |
| No tray icon | Enable AppIndicator host; restart Unfocus; check developer mode diagnostics |
| Wayland session | Unsupported for probes; switch to an X11 session for qualified behavior |
| AppImage will not run | `chmod +x`; install FUSE/AppImage support; or use `.deb` / `.rpm` |
| Break never appears while “away” or fullscreen | Expected when probes work; check **Your day** / developer diagnostics for idle and fullscreen |

Local settings and day history live under the app’s config directory on this
machine (next to other Unfocus settings). They are not uploaded.

---

## macOS

Preview platform. Multi-monitor behavior has not completed an acceptance run.
**No Xcode or other developer tools are required** to install a release DMG.

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

### First launch (unsigned / not notarized)

Alpha builds are **not code-signed or notarized**. Gatekeeper will block a
normal double-click the first time.

1. In **Finder**, open **Applications**.
2. **Control-click** (or right-click) **Unfocus**.
3. Choose **Open**, then confirm **Open** again.

Alternatively: open Unfocus once, then **System Settings → Privacy & Security**
and choose **Open Anyway** if macOS offers it.

Later launches can use a normal double-click or Spotlight.

### Homebrew (alpha cask)

If you use Homebrew:

```sh
brew install --cask abhiksark/unfocus/unfocus@alpha
```

That needs Homebrew itself. It does not install Xcode. First-run Gatekeeper
steps may still apply depending on how the cask delivers the app; use
Control-click → Open if macOS blocks the app.

Upgrade when a new alpha is published:

```sh
brew update
brew upgrade --cask abhiksark/unfocus/unfocus@alpha
```

### Permissions

Current idle and fullscreen probes are designed **not** to require Screen
Recording consent. Unfocus should not prompt for Screen Recording for those
probes. If a future change needs a new permission, it will be documented
explicitly.

### Remove on macOS

- Drag **Unfocus** from Applications to the Trash, or  
- If installed via Homebrew: `brew uninstall --cask abhiksark/unfocus/unfocus@alpha`

### macOS troubleshooting

| Symptom | What to try |
| --- | --- |
| “App can’t be opened because it is from an unidentified developer” | Control-click → Open (unsigned alpha) |
| Wrong architecture | Use `aarch64` vs `x64` DMG for your Mac |
| Tray or multi-monitor oddities | Expected gaps while status is Preview; report with the platform report form |

---

## Windows

Early build. Packages are produced for **64-bit Windows**. Interactive
multi-monitor qualification is still pending. Idle and fullscreen probes are
implemented in current alphas; treat overall Windows support as early, not
fully qualified.

### Setup installer (`.exe`)

1. Download `Unfocus_VERSION_x64-setup.exe` and `SHA256SUMS`; verify the hash.
2. Run the setup executable.
3. If **SmartScreen** warns that the app is unrecognized (unsigned alpha):
   choose **More info**, then **Run anyway** only if you verified the checksum
   from the official release.
4. Finish the wizard and start Unfocus from the Start menu.

### MSI (managed install)

1. Download `Unfocus_VERSION_x64_en-US.msi` and verify `SHA256SUMS`.
2. Double-click the MSI, or for scripted install:

```powershell
msiexec /i Unfocus_0.3.1-alpha.1_x64_en-US.msi
```

The MSI **ProductVersion** is the numeric core only (for example `0.3.1` for
`0.3.1-alpha.1`) because Windows requires that format. The filename still
carries the full prerelease version.

### SmartScreen and unsigned builds

Alpha installers are **not code-signed**. SmartScreen warnings are expected.
Always verify `SHA256SUMS` before choosing **Run anyway**.

### Remove on Windows

- **Settings → Apps → Installed apps → Unfocus → Uninstall**, or  
- Use the uninstaller entry from the Start menu if present, or  
- For MSI: `msiexec /x Unfocus_VERSION_x64_en-US.msi`

### Windows troubleshooting

| Symptom | What to try |
| --- | --- |
| SmartScreen block | Verify checksum; More info → Run anyway |
| Installer will not start | Confirm 64-bit Windows; re-download and re-verify |
| Probes or multi-monitor issues | Early platform; capture details with the platform report form |

---

## Build from source (all platforms)

Use this for development, not as the primary end-user install.

1. Install [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) for
   your OS (Linux libraries, macOS Xcode CLT, or Windows MSVC/WebView2 as
   documented there).
2. Install **Bun** and **Rust** versions pinned in `.bun-version` and
   `rust-toolchain.toml` at the repository root.
3. Clone the repository and run:

```sh
bun install --frozen-lockfile
bun run tauri dev
```

Optional Linux helper when native headers are missing but you have a host X11
display: `./scripts/run-linux-spike-container.sh` (development convenience, not
a sandbox; see the README).

Contributor checks and branching rules are in the [README](../README.md) and
`AGENTS.md`.

---

## After install

1. Start Unfocus. The consumer dashboard shows the next break and, where the
   idle probe works, **Your day** reflection (presence only, local only).
2. Closing the dashboard leaves the reminder in the tray when the tray is
   available.
3. Expand **Advanced** in the timing editor and open **developer mode** only if
   you need raw probe and monitor diagnostics.
4. For bugs or platform evidence, use the issue templates on GitHub (bug report
   or platform report).

## Privacy reminder

Unfocus has no accounts, telemetry, cloud dependency, or packaged-app runtime
network calls. Timing, day history, and break outcomes stay on this device.
Download and install steps use GitHub only to fetch the package you chose.

## Getting help

- [Report a bug](https://github.com/abhiksark/unfocus/issues/new?template=bug_report.yml)
- [Platform report](https://github.com/abhiksark/unfocus/issues/new?template=platform_report.yml)
  (acceptance evidence on a given OS or monitor setup)
- [Security policy](https://github.com/abhiksark/unfocus/security/policy) for
  vulnerabilities (private report, not a public issue)
