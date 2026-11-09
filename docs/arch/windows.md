# Windows development

Runner supports Windows x64 from 0.8.0. macOS and Windows are developed together on `main`, with separate platform chrome and shared application behavior. The [Windows port record](../impls/archive/windows-nightly/README.md) is archived; this document covers ongoing development and packaging.

GitHub Copilot CLI and pi are enabled by default on Windows as well as macOS. Runner detects their native binaries or npm `.cmd` shims. Copilot native binaries receive their first turn through `-i`, while batch shims use the existing verified paste fallback; Copilot requires a subscription and npm installations require Node.js 22. Pi receives its system prompt from Runner's per-session file through `--append-system-prompt`, its lead goal behind `--`, and caller-assigned `--session-id` values on fresh and resumed sessions; Runner passes `--approve` only to mission slots to clear pi's project-trust prompt. Pi's bash tool requires Git Bash. Pi reports hook-based status through a Runner-owned Node extension mounted with `-e`, the same reporter on Windows and macOS, while `/new`, `/resume`, and `/fork` rekey the Runner session row through the same typed daemon admission used for status and editor drafts. Copilot's caller-assigned session keys and JSONC-preserving folder-trust seed work on both platforms. Copilot, Claude Code and Codex report hook-based status on Windows as on macOS ([#610](../impls/archive/610-windows-hook-status.md)); Claude Code runs its status hooks under Git Bash and Copilot under PowerShell, invoking the bundled connect-only `runner hook report`. Codex uses the optional persistent `runner hook serve` stdio adapter, started with executable/argv and no per-event shell. All reporters use the secured daemon pipe with per-launch generation checks, bounded FIFO admission and a 250 ms transport deadline ([#797](../features/archive/797-hook-status-ipc.md)); native Escape A/B and actual-runtime smoke remain pending in the [validation record](../tests/archive/797-hook-status-ipc.md).

Runner enables Antigravity CLI when detected on Windows, unless the user switches it off in Settings → Agents. Its session-key capture from `--log-file`, `--conversation` resume, folder-trust seed and permission flags have no platform branch, but its hook status is macOS-only and a Windows session stays on the terminal-activity baseline. The binary path agy's docs give (`%LOCALAPPDATA%\Antigravity\`), its behaviour under ConPTY, and its `cmd /c` hook commands are unverified.

## Local Windows development

Prerequisites: Git for Windows, the repository's pinned Rust toolchain with rustfmt and Clippy, MSVC v143 x64/x86 build tools, and the Windows SDK. After installing Rust, restart the terminal to pick up the updated `PATH`. The build shortcut also finds Cargo under `%CARGO_HOME%\bin` or `%USERPROFILE%\.cargo\bin`.

From the repository root, use `.\make.cmd build` to build the app and CLI sidecar, or `.\make.cmd run` to build and launch. Add `--release` for optimized binaries. The shortcut works in PowerShell and Command Prompt without GNU Make and defaults to 12 build jobs unless `CARGO_BUILD_JOBS` is set.

Development outputs are `target\debug\Runner.exe`, `runner-agent-cli.exe`, `conpty.dll`, and `OpenConsole.exe`; optimized local builds use `target\release`. After Cargo builds, `make.cmd` runs `script/windows/conpty.ps1` to download the pinned, checksum-verified ConPTY package into `target/tools`, verify Microsoft's signatures, and copy the two x64 files beside the executables. Run `.\target\debug\Runner.exe` to launch an existing development build without rebuilding.

Runner ships the Windows Terminal ConPTY because the inbox conhost splits Codex redraws around a delayed cursor repair. The shipped ConPTY delivers whole frames, so Runner can forward output immediately without a Windows-only hold. `portable-pty` loads `conpty.dll` beside the executable; the first PTY spawn logs whether that DLL is present or the inbox conhost is in use. See the [output latency spec](../features/archive/492-windows-terminal-output-latency.md) for measurements and the package pin.

Debug builds use `%APPDATA%\com.wycstudios.runner-dev` and separate daemon and app-activation pipes. The old MCP endpoint is only a compatibility sentinel. Packaged releases use `%APPDATA%\com.wycstudios.runner`, with logs under its `logs` directory. The per-user installer places application binaries under `%LOCALAPPDATA%\Programs\Runner`; upgrades and uninstall preserve application data.

To clean build outputs, close the development app and finish other Cargo builds, then run `.\make.cmd clean`. Add `--release` to clean only release outputs. This removes Cargo outputs and packaging tools cached under the target directory; installed apps, user data, Rust/MSVC, and Cargo's shared dependency cache are retained. The next build recompiles the removed outputs.

## Validation

Install the same pinned prebuilt nextest used by CI into Cargo's bin directory:

```powershell
$nextestArchive = Join-Path ([IO.Path]::GetTempPath()) ('nextest-' + [guid]::NewGuid() + '.zip')
Invoke-WebRequest -OutFile $nextestArchive https://get.nexte.st/0.9.148/windows
$cargoBin = if ($env:CARGO_HOME) { Join-Path $env:CARGO_HOME 'bin' } else { Join-Path $env:USERPROFILE '.cargo\bin' }
Expand-Archive $nextestArchive -DestinationPath $cargoBin -Force
Remove-Item $nextestArchive
```

Run the same Clippy and test commands as Windows CI:

```powershell
cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings
if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
cargo nextest run --locked --workspace --no-fail-fast --cargo-profile ci --timings
if ($LASTEXITCODE -ne 0) { throw 'Tests failed' }
cargo fmt --all --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed' }
```

The `ci` Cargo profile uses level 1 dependency optimization and minimal backtrace debug information. `make test` and `cargo test --locked --workspace --no-fail-fast --profile ci` still run the plain Cargo suite. CI also runs installer tests, builds `Runner.exe` for manifest/icon verification, and uploads Cargo timing reports. Development builds retain their UI optimization settings; release builds use level 3 with thin LTO.

## Building and testing the Windows installer

Build a stable-channel installer from PowerShell:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\script\bundle-windows.ps1 -Channel production
```

The script downloads the checksum-verified portable Inno Setup compiler and pinned ConPTY package into `target/tools`, builds optimized x64 binaries, and produces `target/x86_64-pc-windows-msvc/release/Runner-Setup-<version>.<stamp>-x64.exe`. The installer contains Runner, the CLI sidecar, `conpty.dll`, `OpenConsole.exe`, and license notices including `LICENSE.conpty`. The ConPTY files sit beside `Runner.exe`, are replaced on every upgrade, and join the binaries in rename-aside handling when in use. `-Stamp YYYYMMDD.HHMM`, `-Sha <commit>`, and `-Jobs <count>` are optional; CI supplies the build identity through environment variables. Building an installer does not install or launch Runner.

The packaging build links the C runtime statically and verifies with MSVC's `dumpbin` that no separate Visual C++ runtime is needed and the app uses the GUI subsystem. End users do not need Rust, MSVC, or PowerShell 7. Local builds are unsigned unless a signing certificate is available; see [Code signing](#code-signing). Both Windows packaging jobs publish a minisign `.sig` alongside each installer for in-app update verification.

Run installer checks without launching Runner:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\script\windows\test-installer.ps1
if ($LASTEXITCODE -ne 0) { throw 'Installer fixture tests failed' }
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\script\windows\test-installer.ps1 -SourceDir .\target\x86_64-pc-windows-msvc\release
if ($LASTEXITCODE -ne 0) { throw 'Installer payload tests failed' }
```

The first command uses generated test executables; the second uses the actual release payload. Each uses a separate temporary installation identity and checks install, upgrade, uninstall, and reinstall without modifying the installed Runner app or its data. Fixture mode also checks `/WAITPID`, rename-aside and numbered suffixes for running binaries, stale-file cleanup, uninstall with a running `.old`, cancellation at the uninstall confirmation, and `/RELAUNCH` after success or a genuine copy failure. It launches and stops only temporary fixture executables. Payload mode retains checks for handles that prevent renaming, without launching the payload.

## Code signing

CI signs `Runner.exe`, `runner-agent-cli.exe`, the installer, and the uninstaller with a Certum Open Source Code Signing certificate issued to **Open Source Developer Yicheng Wang** (tracked in [#497](https://github.com/yicheng47/runner/issues/497)). The private key lives in Certum's SimplySign cloud HSM; the only way to reach it is the SimplySign Desktop application, which after a login with the account name and a one-time code exposes the certificate to Windows as a virtual smart card for about two hours. There is no headless API, so `script/windows/simplysign.ps1` installs the pinned, checksum-verified SimplySign Desktop MSI on the runner, pre-sets its registry so the login dialog opens on launch, derives the one-time code from the `otpauth://` secret, types the credentials into the dialog, and waits until the certificate appears in `Cert:\CurrentUser\My` with a private key. It retries with a fresh code and fails the job if the certificate never appears.

`script/bundle-windows.ps1` signs when `CERTUM_CERTIFICATE_SHA1` (or `-SigningThumbprint`) is set: it runs `signtool sign /sha1 <thumbprint> /fd sha256 /tr http://time.certum.pl /td sha256` on the two executables before packaging, then compiles `runner.iss` with `/DSign` and a `/Sauthenticode=…` sign-tool definition so Inno Setup signs Setup and, through `SignedUninstaller=yes`, the uninstaller stub. `script/windows/test-installer.ps1` compiles its test installers the same way in payload mode and asserts that the installer, the two installed binaries, and `unins000.exe` carry a `Valid`, timestamped signature from the expected thumbprint. Without the thumbprint both scripts produce unsigned output, which is what the `Rust / Windows` CI job and local builds use.

Both `release.yml` and `nightly.yml` require the `CERTUM_USERNAME`, `CERTUM_OTP_URI`, and `CERTUM_CERTIFICATE_SHA1` secrets, connect to SimplySign after restoring the Rust cache, and fail before upload if signing or verification fails. The certificate subject is fixed by Certum's open-source product, so `AppPublisher` in `runner.iss` stays `wyc studios` as the product brand. To sign a local build, connect SimplySign Desktop on the PC, then set `$env:CERTUM_CERTIFICATE_SHA1` to the certificate thumbprint before running `bundle-windows.ps1`; `Get-AuthenticodeSignature` on the outputs shows the signer and timestamp.

## Unified nightlies

`gh workflow run nightly.yml --ref main` defaults to `platform=both`, building macOS and Windows in parallel from one dispatch SHA and UTC stamp. Use `-f platform=windows` or `-f platform=macos` for a single-platform cut. Each channel's concurrency group cancels the entire older run of that channel, regardless of platform. One publish job accepts skipped unselected builds and requires every selected build to succeed; failed/cancelled selected builds prevent publication. It does not wait for `main`'s CI, since code reaches `main` only through pull requests whose checks passed.

Nightly is an independent rolling development channel, identified by its commit and displayed as `Nightly (<short-sha>)`. Official `vX.Y.Z` tags and package-version changes belong to official releases; no nightly version bump is required. All five packages inherit the root workspace version since #635; production releases bump it together, while nightly artifact names and update comparisons use the build stamp. Windows produces `Runner-Setup-nightly.<sha>.<stamp>-x64.exe` and its `.sig`, for example `Runner-Setup-nightly.abc1234.20260908.0100-x64.exe`. The updater compares the last two timestamp components, so existing bare-version and `X.Y.Z-nightly` Windows installers can upgrade to this naming convention; the new update offer displays `Nightly (abc1234)`.

Both platforms publish to one public `nightly` prerelease with combined notes from `script/nightly-release-notes.md`. Windows reads this release’s baked updater URL and ignores macOS assets; macOS reads its signed Sparkle appcast. Stable feeds and `releases/latest` remain separate. The workflow uploads the DMG before its appcast and the installer before its signature, verifies release flags and all selected anonymous downloads, then keeps the newest two builds per platform by stamp. Windows installers and signatures prune together, and the appcast’s DMG is protected. A single-platform cut changes only that platform’s assets and retention. An upload or download failure leaves an incomplete cut without pruning either platform. Unified nightlies have been published since 2026-09-08; a particular installed-upgrade pass still needs its own native evidence. `/nightly check [both|macos|windows]` verifies an authorized cut without installing it. The [preview nightly](./arch.md#14-program-state--line-landing-channels), cut from a listed umbrella branch, publishes `Runner-Setup-preview-nightly.<sha>.<stamp>-x64.exe` and its `.sig` to its own `preview-nightly` release instead, and its builds display `Preview Nightly (<sha>)`.

The `nightly-win` release and tag no longer exist, so a dormant Windows install that missed the hop reinstalls from `nightly`; there was no transition mirror. On macOS, install the first unified DMG over `Runner.app` and delete `Runner Nightly.app` by hand because Sparkle cannot cross bundle identifiers. Both platforms now replace the same app when switching channels; installing a production build by hand switches back to stable.

## Update behavior

Packaged Windows builds always check at startup and every six hours: production builds use the latest stable GitHub release, nightly builds use `nightly`, and preview nightlies use `preview-nightly`. Settings → Updates also offers **Check for updates**. **Automatically download updates** defaults on; turning it off leaves checks enabled and waits for a click on the update icon or **Download** in Settings → Updates. The older automatic-check setting is ignored on Windows. Unstamped local builds do not check automatically.

Only a completed x64 installer with its exact `<installer>.sig` asset in the same release can be installed in-app. The updater streams bytes into `<app data>\updates\<installer>.partial`, reports progress through the sidebar ring and Settings → Updates, and verifies the minisign signature against `packaging/windows-update-public-key` before renaming the staged file. Cancel deletes the partial download; retry starts from zero. A download asked for during a check shows as downloading at once and starts if the check succeeds, because the check can replace the candidate; a failed check reports its error instead, and Cancel holds off an automatic download for that check. Startup removes partial and obsolete files and re-verifies a complete current candidate without downloading the installer again. Releases without a signature and legacy portable ZIP releases show **View downloads** only.

The icon beside the Runner name appears while an update is available, downloading, ready or failed. Clicking it starts the download at Available, opens the popover at Ready, opens Settings → Updates at Failed and Manual download only, and does nothing during Downloading, Verifying and Installing. The popover and the Settings → Updates hero install through the same code. **Restart to update** starts the verified installer detached with `/VERYSILENT /SUPPRESSMSGBOXES /NORESTART /WAITPID=<Runner pid> /RELAUNCH=1 /LOG=<log directory>\update-<stamp>.log`, so no installer window appears, then marks the quit as an update and exits without the quit dialog, leaving `runnerd` running. Both the popover and the hero show an amber “N agents working” note while turns are active. On relaunch the new build detects the daemon hash mismatch, stamps and stops its sessions, replaces the sidecar and starts the new daemon; the restart toast counts successful launch resumes. The old `runnerd` releases its lock before its process exits, and Windows cannot replace a running executable, so the new build also waits for the old process before replacing `runnerd.exe`. A sidecar still running anyway, such as a `runner` command started outside Runner, is renamed to `<name>.old` (or `<name>.1.old`, etc.) and deleted on a later replacement once it has exited. All of this happens before the first window opens; if it fails, Runner writes the error to `runner.log` and shows it in an error dialog instead of exiting silently. Setup waits up to 30 seconds for Runner to exit, renames any in-use application binaries to `<name>.old` (or `<name>.1.old`, etc. if a prior old image is still running), installs their replacements, and relaunches Runner through `explorer.exe` rather than as its own child: Setup runs under Windows Redirection Guard, which child processes inherit, and a Runner started that way cannot traverse user-created junctions such as Codex's `bin` directory, so agents behind one are reported as not found. The finish-page **Launch Runner** option uses the same shell hand-off for the same reason. Existing external MCP/CLI processes keep using their old images. Every install and uninstall attempts to delete leftover `.old` files; files still running are retained without blocking either operation and are cleaned up on the next install after they exit. Settings, chats, and missions are retained. The installer filename and Installed Apps version retain the build stamp, while the app displays the base version on production and `Nightly (<sha>)` on nightly.

If installation aborts, Setup restores renamed binaries whose original paths are missing after rollback, then relaunches the old app when it exists. Runner records each install attempt beside the staged installer; on the next successful check, an unfinished attempt becomes an installer failure after signature verification. With no installer window, this is where a failed install is reported. The icon shows the failure and opens Settings → Updates, which offers **Retry**, installing the staged update again, and **Settings → About** shows the installer log path beside **Open log folder**. A successful upgrade removes the old staged installer and attempt record during startup cleanup.

To preview the update flow in a development build, set `$env:RUNNER_DEV_UPDATE_AVAILABLE = '0.8.0.20260907.1200'` before `.\make.cmd run`, optionally followed by `:<state>`, one of `available`, `manual`, `downloading`, `downloading-unknown`, `verifying`, `ready`, `installing`, `failed`, `failed-check` or `uptodate`. Its actions step through the states without downloading, staging or installing anything. Clear it with `Remove-Item Env:RUNNER_DEV_UPDATE_AVAILABLE` before the next launch. Release builds ignore this preview variable, on every platform.

## Background service and quitting

The app, `runnerd` and the `runner` CLI are separate processes. `runnerd.exe` runs from app data, with `conpty.dll` and `OpenConsole.exe` beside it. It owns the sessions, SQLite, routers and sockets. Its owner-only named pipe is `\\.\pipe\com.wycstudios.runnerd` (`-dev` for debug builds). It starts with a hidden console, a new process group and job breakaway; the app retries without breakaway when its launcher forbids it, while a CLI inside an agent job refuses to start it.

Closing the last window follows Settings → General → When Runner quits: Ask, Keep running or Stop sessions. Cancel leaves the window open. Windows has no quit key binding. OS and installer quits never ask and leave sessions to the daemon. A hidden top-level window receives shutdown and sign-out messages even when loaded GUI libraries suppress console events; it waits for the resume stamp before returning from a confirmed end-session. Console handlers remain available, and PTY cleanup after the stamp is best-effort within Windows' shutdown deadline. See the [daemon lifecycle](process-model.md#lifecycle) for shutdown ordering and cancellation. A crash closes agent Job Object handles and kills those process trees. The app reconnects, retaining a notice with Resume instructions; after repeated failures, Open log reveals `runnerd.log`.

## App relaunch

A second Windows launch hands activation to the running app through an app-identity and login-session-scoped named pipe, foregrounds it, restores a minimized main window or reopens main when another window survives. The activation path runs before daemon setup, so an ordinary secondary launch does not register another main app window. Development and installed builds use separate identities. A launch while the primary quits waits for handoff or exit, with the documented 10-second fallback to a new app process. See the [user guide](../guide/guides/chats-and-windows.md#use-another-window).

[#981](https://github.com/yicheng47/runner/pull/981) added deterministic Windows pipe tests for early client disconnects and subsequent activations. CI passed, but native relaunch, minimize/restore, surviving-secondary-window and installer-relaunch checks remain unverified; [WS-CHROME-05](../tests/regression/workspace.md#sidebar-windows-and-platform-chrome) maintains that live check.

## Terminal working directory

Terminal splits follow a shell's live working directory through OSC 7 ([#575](../guide/guides/chats-and-windows.md#split-and-navigate)). On macOS Runner injects the reporting hook into zsh and bash; on Windows it injects nothing, so a split opens at the split-from shell's spawn directory unless the user's own prompt sends OSC 7. The parser accepts the Windows form `file://<COMPUTERNAME>/C:/…`, so oh-my-posh with `"pwd": "osc7"`, or this snippet placed at the end of a PowerShell profile, is enough. It has not been verified on native Windows yet:

```powershell
$runnerPrompt = $function:prompt
function prompt {
    $location = $executionContext.SessionState.Path.CurrentLocation
    if ($location.Provider.Name -eq 'FileSystem') {
        $path = ($location.ProviderPath -split '\\' | ForEach-Object { [uri]::EscapeDataString($_) }) -join '/'
        $esc = [char]27
        [Console]::Write("$esc]7;file://$env:COMPUTERNAME/$path$esc\")
    }
    & $runnerPrompt
}
```

## Follow-up work

The unsigned Windows port shipped in 0.8.0 and signing followed with [#497](https://github.com/yicheng47/runner/issues/497); these items were not completed by that release:

- Confirm SmartScreen behavior for the signed stable installer on a fresh Windows 11 PC and record the result in the [signing spec](../features/archive/497-windows-code-signing.md).
- Complete detailed installed-build lifecycle, crash/relaunch, IME, resize, DPI, path, and update/data-retention acceptance. The [remaining validation checklist](../impls/archive/windows-nightly/impl_log.md#todo) preserves the specific cases and prior results. Native TRAE and Antigravity CLI validation is still pending.
- Investigate the shutdown `window not found` diagnostic. The separate development-only DXGI debug-interface warning is an optional `gpui-pre-windows` debug probe (`check_debug_layer_available` in `src/directx_devices.rs`, under `debug_assertions`) and is skipped in release builds.
- Inject the OSC 7 prompt hook into PowerShell as Runner does for zsh and bash on macOS, through `-NoExit -Command` and a `prompt` wrapper that coexists with oh-my-posh and starship, and consider Windows Terminal's OSC 9;9 ([#575](../guide/guides/chats-and-windows.md#split-and-navigate)); validate the snippet above on native Windows first.

Both `Rust / macOS` and `Rust / Windows` are required branch checks, verified on 2026-10-22. Keep native Windows validation separate from those automated gates.
