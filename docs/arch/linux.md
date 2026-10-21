# Linux development and packaging

Linux support for ARM64 (`aarch64`) and x86_64 is being prepared in [#823](https://github.com/yicheng47/runner/issues/823). This branch adds source/build/package definitions; it does not publish downloads. See [validation](../tests/823-linux-support.md) for observed results and remaining gaps. A local Ubuntu 26.04 ARM64 build is evidence for that machine only.

## Platform requirements

Linux kernel 5.3 or later is required for retained pidfds during descendant cleanup. The app uses GPUI 0.3.7 with X11 and Wayland enabled, fontconfig, an EGL/Vulkan-capable graphics stack and a desktop notification service. Noto Sans CJK provides Chinese/Japanese/Korean fallback. A working Vulkan cube alone does not establish terminal/IME or app acceptance. Software Vulkan rendering (Mesa llvmpipe) may work but is not a performance guarantee. The CLI/daemon do not need a display or prior GUI launch.

CI and release definitions build natively on x86_64 and ARM64 runners inside Ubuntu 22.04 containers, targeting glibc 2.35 or later. The current-main compatibility checks use this baseline on ARM64; see `docs/impls/982-linux-remote-compat.md` for automated results and remaining acceptance gaps. This automated coverage does not establish native desktop, installation or x86_64 acceptance. The bundler extracts the highest required GLIBC symbol from both executables and raises package dependency floors accordingly. Locally built Ubuntu 26 binaries can therefore require newer glibc. `lib/runner/build-platform.txt` records the actual floor and build distro. Debian C++ dependencies conservatively require the build host's installed libstdc++ package; RPM declares the required GLIBCXX symbol capability. Other distros need the equivalent runtime libraries and graphics drivers; package metadata is not a substitute for native validation.

## Local development

Use the pinned Rust toolchain and native architecture. On Ubuntu install the build libraries with:

```sh
sudo ./script/linux/install-build-deps
```

The script installs compiler/pkg-config/OpenSSL, fontconfig/FreeType, X11/XCB, Wayland/xkbcommon, EGL/Vulkan headers, clang, CJK fonts and packaging tools. A desktop notification daemon and appropriate xdg-desktop-portal backend are desktop runtime prerequisites; Ubuntu GNOME can use `xdg-desktop-portal-gtk`. Do not run apt concurrently with another package manager.

```sh
make run
cargo check --locked --workspace --all-targets --profile ci
cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings
cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci
python3 script/linux/test-packaging.py
```

`make run` builds the CLI sidecar before the app and uses the development namespace. `ci` also retains debug assertions and development paths. Release packaging requires release binaries. Set `CARGO_BUILD_JOBS=2` on a small VM; each worktree keeps its own target directory.

Workspace tests require Node.js for the embedded pi extension protocol fixture; this needs no agent install, account or inference. Linux CI installs Node 22.14.0 with `actions/setup-node`, independently of Ubuntu's older apt version. For local checks, provide a compatible Node on the test process's PATH; it is not a CLI/daemon runtime prerequisite.

Linux PR CI runs formatting, workspace Clippy/tests, generated CLI reference checks,
installer fixtures and desktop-entry validation on both architectures. It downloads
pinned prebuilt nextest binaries, saves successful build caches for subsequent runs
of the same PR, and lets Cargo choose concurrency from the available CPUs. Tests
run as an ordinary user with `C.UTF-8` inside an init-enabled container. PR CI does
not build release binaries or installation packages; the release and nightly
workflows build and verify tar.gz, deb and rpm packages before uploading them.

## Window controls and initial bounds

Linux requests client-side decorations and draws its own caption controls. Normal windows reserve a 6 logical pixel border outside the caption/content for native edge and corner resize requests. Maximized/fullscreen windows have no resize border; tiled edges omit their corresponding margin and resize target. The compositor owns native resize and maximize/restore; Runner does not resize the terminal model to implement these controls.

Initial main and secondary windows select an enumerated display even when Wayland reports no primary display. Their requested normal size and cascaded origin are constrained to that display's reported visible logical bounds. Existing saved windows continue through the established restore/clamp path. If no display is available, Runner retains the requested fallback bounds. GPUI's pinned Wayland display exposes full logical output bounds as visible bounds, not a separate panel/taskbar work area; this fallback cannot promise that desktop-reserved areas are excluded. Native verification remains required on each desktop.

The contract follows the locked [GPUI 0.3.7 source](https://static.crates.io/crates/gpui-pre/0.3.7/download) and [Linux backend 0.3.7 source](https://static.crates.io/crates/gpui-pre-linux/0.3.7/download), both from Zed `1a28cff4b409169bac058bca40dfbfeb7621d19b`. The official `window_shadow.rs` example uses explicit resize requests for client decorations. The Wayland backend reports no primary display and restores its retained normal bounds when unmaximizing, so initial restore bounds must be usable before maximize.

Linux uses the existing custom desktop titlebar and Ctrl shortcuts. Text copy/paste uses GPUI; clipboard file paths are currently unsupported. Window material is solid. Hide/Hide Others and the macOS wake observer do not apply. Signed application updates are described below. Notifications are submitted to GPUI's desktop backend; Runner cannot query a universal Linux permission status or open a universal notification-settings URL, so Settings describes desktop-managed delivery without claiming permission is granted. Agent CLI Update remains separate from app updates.

Linux windows use the desktop application ID `runner`, matching `runner.desktop`
and its `StartupWMClass`. GPUI forwards this window option to Wayland's app ID
and X11's WM_CLASS so the desktop can associate running windows with the installed
Runner name and icon. This desktop identity does not change the production or
development data identifiers.

Linux process discovery reads `/proc`: descendant PPIDs are parsed after the command name's final parenthesis, disappeared processes are skipped, and zombie/exited states are not treated as live executable descendants. Cleanup retains pidfds after verifying each proc start-time token and signals through those handles, so PID reuse cannot target a replacement process. Unavailable pidfds fail closed with a logged identity-capture error. PID-assisted Codex capture uses exact open rollout paths beneath the canonical sessions root; existing metadata/marker, unique-claim, ownership and timeout guards still apply. It never guesses the newest session.

## Packages

Build on a matching Linux architecture. The bundler always builds both inputs with an explicit native target, release profile and disabled debug assertions; prebuilt/debug input mode is intentionally unavailable:

```sh
./script/bundle-linux --channel production
# Native architecture may also be selected explicitly:
./script/bundle-linux --arch aarch64
python3 script/linux/test-packaging.py --packages target/linux-packages
```

Outputs are `Runner-<version>-linux-<x86_64|aarch64>.tar.gz`, `.deb` and `.rpm`. Nightlies substitute `nightly.<sha>.<UTC stamp>` for the version label. Debian/RPM metadata keeps the UTC stamp before the commit for upgrade ordering. Debian metadata uses `amd64`/`arm64`, RPM uses `x86_64`/`aarch64`; ELF machine validation prevents mislabelling. All formats share one staging tree and include the app, sidecar, real same-byte `runnerd`, desktop entry, icon and user service. Packaging uses `dpkg-deb` and `rpmbuild`, without nfpm or a runtime download. Local packaging alone does not sign artifacts. Release/nightly builds run `script/linux/sign-updates` and publish a detached `.sig` for each package using the existing updater signing key.

The tarball installs per user:

```sh
tar -xzf Runner-<version>-linux-aarch64.tar.gz
cd Runner-<version>-linux-aarch64
./install.sh
# ~/.local/bin must be on PATH
~/.local/bin/runner --version
~/.local/bin/runner-app
# Remove application files, retaining data:
~/.local/lib/runner/install.sh --uninstall
```

The user-installed desktop icon is an owned relative link into `lib/runner`, so an atomic application-directory update also changes its image without losing reinstall/uninstall ownership. A manual install migrates a legacy copied icon only when its bytes match the currently managed package; foreign regular icons and foreign symlink targets are preserved.

`--prefix /absolute/user/prefix` supports isolated/custom installs; add its `bin` to PATH. The installer refuses foreign commands, desktop entries, icons and units before changing files. Upgrades replace executable inodes, preserving already running processes until normal daemon replacement. The installer never runs or enables a service. Uninstall stops no sessions and retains data; stop/disable the optional service first. Do not install a tarball into `/usr` or `/usr/local`.

Install a `.deb` with `sudo apt install ./Runner-…-linux-….deb`, or an `.rpm` with your distro's package manager. System packages install payloads to `/usr/lib/runner` and expose `/usr/bin/runner`, `runnerd` and `runner-app` through ownership-checked lifecycle scripts; the tarball uses the same layout under its prefix. Headless `runner` starts a local daemon and installs the per-user shim target on first use. Settings recognizes package-managed commands and directs removal to the installer/package manager; it cannot delete package-owned links. An earlier unrelated executable on PATH remains visible as a shadowing warning. The aliases are not ordinary package payload files: install/upgrade refuses foreign existing commands, and remove/purge preserves replacements. RPM upgrade keeps aliases until the new package post-install step; erase removes only exact owned links. No package operation traverses user homes or deletes their databases.

## Optional systemd user service

The system package ships `/usr/lib/systemd/user/runnerd.service`; the tarball writes the unit under `${XDG_CONFIG_HOME:-~/.config}/systemd/user`. Installation leaves it disabled. After installing or changing the unit:

```sh
systemctl --user daemon-reload
systemctl --user start runnerd
# Optional persistence when the user manager starts:
systemctl --user enable --now runnerd
# Optional user-manager lifetime after logout (requires system authorization):
loginctl enable-linger "$USER"
```

`Restart=no` is deliberate: duplicate endpoint ownership already exits successfully, normal Stop must remain stopped, and an app replacing a mismatched hash must not race systemd restarting old bytes. Service failure therefore requires an explicit start. There is one daemon lock per data root, shared by app/CLI/service launches. The unit executes the same CLI bytes as the app's sidecar hash. The app's normal mismatch path stamps/stops the old daemon, waits for lock release, installs and launches its current sidecar; systemd stays inactive instead of reclaiming the endpoint. A service started while another daemon owns that root exits successfully. `KillMode=mixed` allows the daemon's SIGTERM shutdown to stop/stamp children before systemd's 20-second final cgroup deadline.

A service environment may lack `SHELL`; Linux resolves the current account's passwd shell with bounded `getpwuid_r` buffers, then uses existing login-shell discovery and timeout rules. New terminal uses that same account shell fallback. It does not assume zsh is installed. Opening the app or an ordinary local CLI request can start the daemon. A CLI launched with `SSH_CONNECTION` never starts it: start the user service explicitly first, then run `runner session list` over SSH.

To remove a service installation, use `systemctl --user disable --now runnerd` before uninstalling, then `systemctl --user daemon-reload`. User data remains under `${XDG_DATA_HOME:-~/.local/share}/com.wycstudios.runner`; development uses the `-dev` identifier. Disabling a service does not disable CLI/app auto-start.

## WSL2 and WSLg

WSL is a documented setup path with native acceptance still pending. Use current Store WSL (`wsl --update`) and WSL2. For a distro without systemd, add this to `/etc/wsl.conf`, save work, and run `wsl --shutdown` from Windows before reopening it:

```ini
[boot]
systemd=true
```

Then check `systemctl status`, install Runner inside the distro, and opt into the user service as above. Microsoft explicitly states that systemd services do not keep a WSL instance alive; linger is not a guarantee of continuous SSH reachability. `wsl --shutdown`, distro termination and Windows reboot end the daemon and sessions. Enabling the unit starts it when the distro/user manager next starts, subject to WSL lifetime. See [Microsoft systemd guidance](https://learn.microsoft.com/en-us/windows/wsl/systemd/) and [WSL configuration](https://learn.microsoft.com/en-us/windows/wsl/wsl-config).

The app additionally needs WSLg, current Windows GPU drivers and a working Vulkan/EGL backend inside the distro. WSLg supplies Wayland/X11 integration but its presence alone does not prove Runner rendering, clipboard or IME. See [Microsoft WSLg](https://github.com/microsoft/wslg). Test only the distro's Linux binaries; the native Windows Runner remains a separate application.

## Validation gaps

Consult the feature record for exact candidate/commands and outcomes. ARM64 native app/package/service results, x86_64 execution, older-distro compatibility, WSLg, real agent hooks and Chinese IME must each be established independently. Synthetic shell/PTY/IPC tests do not establish a real Claude/Codex chat or crew round trip. No release or nightly has been dispatched by this implementation task.

## In-app updates

Linux owns its updater state, scheduling, transport and verification in
`updater/linux.rs`, installation in `updater/linux/install.rs`, and its update
settings/dialog in `updates_linux.rs` and `update_dialog_linux.rs`. Windows keeps
its existing updater and views; macOS retains Sparkle. The application-facing
updater entry points select the implementation at compile time.
Stable updates compare semantic versions; nightlies compare UTC build stamps. The retired remote-nightly channel is not produced or selected. Each architecture/format needs its own signature; unsigned older releases cannot be installed by the updater.

A user tar installation is recognized by its managed `lib/runner` directory.
The verified archive is copied to private staging, safely unpacked with bounded
sizes and no links/special paths, checked against signed release identity and
ELF architecture, then the entire app/CLI/daemon directory is swapped with
`renameat2(RENAME_EXCHANGE)` on the same filesystem. No unsafe fallback to
per-file replacement is attempted if atomic exchange is unavailable. Failure
before the exchange preserves the old installation. User data stays outside it;
this does not promise database rollback after future schema migrations.

System package ownership is queried for `/usr/lib/runner/Runner`. The GUI starts
that root-owned, non-writable executable through `pkexec` in its narrow
`--runner-install-update` mode. It starts no GUI, daemon, agents or user database
as root. The authorized process copies package/signature inputs into root-owned
staging and rechecks the compiled-in trust key, version, channel, architecture
and runner package metadata before invoking APT/DNF. Neither shell command text
nor arbitrary executable paths come from a feed. DNF checks repository dependency
signatures; only the already Runner-verified local RPM uses the detached trust
chain instead of RPM signing. APT refuses a transaction requiring removals.

Polkit/pkexec and an interactive desktop authentication agent are required for
system packages; RPM automatic install currently requires DNF. A failed or
cancelled authorization/transaction keeps the current GUI running, with log and
retry access. Successful install marks the existing update-quit lifecycle and
relaunches the installed application as the original user after the old GUI
exits; new-build daemon replacement/resume follows existing policy. An attempt
marker survives interrupted installation so the next check can report it.
PR CI never signs or builds release packages; publishing signatures requires the
maintainer's existing signing secret, and no release is dispatched by local tests.

## Remote control on current main

The Linux integration follows Runner 0.18.0's remote host and client contracts. A headless `runnerd` serves encrypted Runner clients through the hosted relay; the browser handles enrollment and account management, not terminals. The Linux app can open another computer through the picker, and `runner remote` uses the same daemon connection. Real Linux hosted-service acceptance has not been verified by the compatibility work; see `docs/impls/982-linux-remote-compat.md` for exact automated coverage.

Release and development remote identity, PIN, enrollment credentials and outgoing pairing keys use plaintext files with mode 0600 under `${XDG_DATA_HOME:-~/.local/share}/com.wycstudios.runner` and `com.wycstudios.runner-dev` respectively. When the secret store creates a missing data root, it uses mode 0700; a root already created by the daemon or user can retain its existing safe permissions. An existing data root must be a real directory owned by the current user and not writable by group or other users; remote secret operations reject unsafe roots without changing existing permissions. Keep its parent directories under the user's control as well. This backend needs no desktop keyring or session-bus unlock; a future Secret Service backend would require a migration policy. Do not copy these files between computers or publish them with logs.

The app, CLI and systemd user manager must use the same `XDG_DATA_HOME` to share an identity. If using a custom value, set it consistently in the user service's environment and desktop/shell launches. The packaged user service runs release bytes and uses the release root. Development and `ci` builds use `-dev`; never point the production service at development data. Over SSH, explicitly start `runnerd` first, then run `runner remote login` and `runner remote pin` in your own interactive terminal. Login prints an approval link to open on another device and asks for account confirmation. It never opens a browser on the host. Closing the GUI leaves the daemon's remote connection running. Stopping or replacing the daemon interrupts it; a new daemon reconnects from the saved root. `Restart=no` and WSL lifetime limits still apply as described above.

Linux uses Ctrl for PIN paste and app shortcuts. Ctrl+C copies a terminal selection; with no selection it reaches the terminal's interrupt path. Remote terminal paste accepts text, and input is dropped while disconnected or controlled elsewhere. Pairing attempts desktop notification delivery and also shows an actionable in-app toast, because Linux cannot query whether the desktop delivered the notification. Native clipboard/browser, X11/Wayland chrome, pairing notification and systemd/linger behavior still need checks on real Linux machines.

Nightly publication uses one `nightly` release, one stamp and one SHA across macOS, Windows and both Linux architectures. The default `all` includes every platform; `linux` selects both Linux architectures, and legacy `both` remains macOS+Windows. Six Linux packages and six signatures are validated and publicly checked before pruning. Nightly does not wait for main CI; production release publication retains its CI gate and draft-release handoff.
