# Windows development

Runner supports Windows x64 from 0.8.0. macOS and Windows are developed together on `main`, with separate platform chrome and shared application behavior. The [Windows port record](../impls/archive/windows-nightly/README.md) is archived; this document covers ongoing development and packaging.

## Local Windows development

Prerequisites: Git for Windows, the repository's pinned Rust toolchain with rustfmt and Clippy, MSVC v143 x64/x86 build tools, and the Windows SDK. After installing Rust, restart the terminal to pick up the updated `PATH`. The build shortcut also finds Cargo under `%CARGO_HOME%\bin` or `%USERPROFILE%\.cargo\bin`.

From the repository root, use `.\make.cmd build` to build the app and both CLI sidecars, or `.\make.cmd run` to build and launch. Add `--release` for optimized binaries. The shortcut works in PowerShell and Command Prompt without GNU Make and defaults to 12 build jobs unless `CARGO_BUILD_JOBS` is set.

Development outputs are `target\debug\Runner.exe`, `runner-agent-cli.exe`, `runner-mcp.exe`, `conpty.dll`, and `OpenConsole.exe`; optimized local builds use `target\release`. After Cargo builds, `make.cmd` runs `script/windows/conpty.ps1` to download the pinned, checksum-verified ConPTY package into `target/tools`, verify Microsoft's signatures, and copy the two x64 files beside the executables. Run `.\target\debug\Runner.exe` to launch an existing development build without rebuilding.

Runner ships the Windows Terminal ConPTY because the inbox conhost splits Codex redraws around a delayed cursor repair. The shipped ConPTY delivers whole frames, so Runner can forward output immediately without a Windows-only hold. `portable-pty` loads `conpty.dll` beside the executable; the first PTY spawn logs whether that DLL is present or the inbox conhost is in use. See the [output latency spec](../features/archive/492-windows-terminal-output-latency.md) for measurements and the package pin.

Debug builds use `%APPDATA%\com.wycstudios.runner-dev` and a separate MCP pipe. Packaged releases use `%APPDATA%\com.wycstudios.runner`, with logs under its `logs` directory. The per-user installer places application binaries under `%LOCALAPPDATA%\Programs\Runner`; upgrades and uninstall preserve application data.

To clean build outputs, close the development app and finish other Cargo builds, then run `.\make.cmd clean`. Add `--release` to clean only release outputs. This removes Cargo outputs and packaging tools cached under the target directory; installed apps, user data, Rust/MSVC, and Cargo's shared dependency cache are retained. The next build recompiles the removed outputs.

## Validation

Run the same Clippy and test commands as Windows CI:

```powershell
cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings
if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
cargo test --locked --workspace --no-fail-fast --profile ci --timings
if ($LASTEXITCODE -ne 0) { throw 'Tests failed' }
cargo fmt --all --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed' }
```

The `ci` profile uses level 1 dependency optimization and minimal backtrace debug information. CI also runs installer tests, builds `Runner.exe` for manifest/icon verification, and uploads Cargo timing reports. Development builds retain their UI optimization settings; release builds use level 3 with thin LTO.

## Building and testing the Windows installer

Build a stable-channel installer from PowerShell:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\script\bundle-windows.ps1 -Channel production
```

The script downloads the checksum-verified portable Inno Setup compiler and pinned ConPTY package into `target/tools`, builds optimized x64 binaries, and produces `target/x86_64-pc-windows-msvc/release/Runner-Setup-<version>.<stamp>-x64.exe`. The installer contains Runner, both CLI sidecars, `conpty.dll`, `OpenConsole.exe`, and license notices including `LICENSE.conpty`. The ConPTY files sit beside `Runner.exe`, are replaced on every upgrade, and join the binaries in rename-aside handling when in use. `-Stamp YYYYMMDD.HHMM`, `-Sha <commit>`, and `-Jobs <count>` are optional; CI supplies the build identity through environment variables. Building an installer does not install or launch Runner.

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

CI signs `Runner.exe`, `runner-agent-cli.exe`, `runner-mcp.exe`, the installer, and the uninstaller with a Certum Open Source Code Signing certificate issued to **Open Source Developer Yicheng Wang** (tracked in [#497](https://github.com/yicheng47/runner/issues/497)). The private key lives in Certum's SimplySign cloud HSM; the only way to reach it is the SimplySign Desktop application, which after a login with the account name and a one-time code exposes the certificate to Windows as a virtual smart card for about two hours. There is no headless API, so `script/windows/simplysign.ps1` installs the pinned, checksum-verified SimplySign Desktop MSI on the runner, pre-sets its registry so the login dialog opens on launch, derives the one-time code from the `otpauth://` secret, types the credentials into the dialog, and waits until the certificate appears in `Cert:\CurrentUser\My` with a private key. It retries with a fresh code and fails the job if the certificate never appears.

`script/bundle-windows.ps1` signs when `CERTUM_CERTIFICATE_SHA1` (or `-SigningThumbprint`) is set: it runs `signtool sign /sha1 <thumbprint> /fd sha256 /tr http://time.certum.pl /td sha256` on the three executables before packaging, then compiles `runner.iss` with `/DSign` and a `/Sauthenticode=…` sign-tool definition so Inno Setup signs Setup and, through `SignedUninstaller=yes`, the uninstaller stub. `script/windows/test-installer.ps1` compiles its test installers the same way in payload mode and asserts that the installer, the three installed binaries, and `unins000.exe` carry a `Valid`, timestamped signature from the expected thumbprint. Without the thumbprint both scripts produce unsigned output, which is what the `Rust / Windows` CI job and local builds use.

Both `release.yml` and `nightly.yml` require the `CERTUM_USERNAME`, `CERTUM_OTP_URI`, and `CERTUM_CERTIFICATE_SHA1` secrets, connect to SimplySign after restoring the Rust cache, and fail before upload if signing or verification fails. The certificate subject is fixed by Certum's open-source product, so `AppPublisher` in `runner.iss` stays `wyc studios` as the product brand. To sign a local build, connect SimplySign Desktop on the PC, then set `$env:CERTUM_CERTIFICATE_SHA1` to the certificate thumbprint before running `bundle-windows.ps1`; `Get-AuthenticodeSignature` on the outputs shows the signer and timestamp.

## Unified nightlies

`gh workflow run nightly.yml --ref main` defaults to `platform=both`, building macOS and Windows in parallel from one dispatch SHA and UTC stamp. Use `-f platform=windows` or `-f platform=macos` for a single-platform cut. One `nightly` concurrency group cancels the entire older run, regardless of platform. One publish job accepts skipped unselected builds, requires every selected build to succeed, and gates publication on successful CI for that SHA once. Failed/cancelled selected builds and missing/failed CI prevent publication.

Nightly is an independent rolling development channel, identified by its commit and displayed as `Nightly (<short-sha>)`. Official `vX.Y.Z` tags and package-version changes belong to official releases; no nightly version bump is required. The lockstep `runner-app`, `runner-backend`, and `runner-terminal` packages retain the `0.8.2` baseline as internal metadata, including numeric Windows resource versions. `runner-core` and the CLI keep their independent versions. Windows produces `Runner-Setup-nightly.<sha>.<stamp>-x64.exe` and its `.sig`, for example `Runner-Setup-nightly.abc1234.20260908.0100-x64.exe`. The updater compares the last two timestamp components, so existing bare-version and `X.Y.Z-nightly` Windows installers can upgrade to this naming convention; the new update offer displays `Nightly (abc1234)`.

Both platforms publish to one public `nightly` prerelease with combined notes from `script/nightly-release-notes.md`. Windows reads this release’s baked updater URL and ignores macOS assets; macOS reads its signed Sparkle appcast. Stable feeds and `releases/latest` remain separate. The workflow uploads the DMG before its appcast and the installer before its signature, verifies release flags and all selected anonymous downloads, then keeps the newest two builds per platform by stamp. Windows installers and signatures prune together, and the appcast’s DMG is protected. A single-platform cut changes only that platform’s assets and retention. An upload or download failure leaves an incomplete cut without pruning either platform. The first live cut and installed upgrades remain pending after landing; `/nightly check [both|macos|windows]` verifies an authorized cut without installing it.

The installed Windows nightly still reads `nightly-win` and needs one manual install of the first unified build on the PC. After the PC reads `nightly`, delete the old release and tag by hand; there is no transition mirror. Dormant installs then reinstall from `nightly`. On macOS, install the first unified DMG over `Runner.app` and delete `Runner Nightly.app` by hand because Sparkle cannot cross bundle identifiers. Both platforms now replace the same app when switching channels; installing a production build by hand switches back to stable.

## Update behavior

Packaged Windows builds always check at startup and every six hours: production builds use the latest stable GitHub release, and nightly builds use `nightly`. Settings → Updates also offers **Check for updates**. **Automatically download updates** defaults on; turning it off leaves checks enabled and waits for **Download** in the update dialog. The older automatic-check setting is ignored on Windows. Unstamped local builds do not check automatically.

Only a completed x64 installer with its exact `<installer>.sig` asset in the same release can be installed in-app. The updater streams bytes into `<app data>\updates\<installer>.partial`, reports progress in the centered update dialog, and verifies the minisign signature against `packaging/windows-update-public-key` before renaming the staged file. Cancel deletes the partial download; retry starts from zero. Startup removes partial and obsolete files and re-verifies a complete current candidate without downloading the installer again. Releases without a signature and legacy portable ZIP releases show **View downloads** only.

The icon beside Settings appears for available, ready, and failed updates; both it and the hero card's **Update** button open the same dialog. **Install and restart** starts the verified installer detached with `/SILENT /NORESTART /WAITPID=<Runner pid> /RELAUNCH=1 /LOG=<log directory>\update-<stamp>.log`, then uses Runner's normal quit flow to preserve session auto-resume and stop PTYs. Setup waits up to 30 seconds for Runner to exit, renames any in-use application binaries to `<name>.old` (or `<name>.1.old`, etc. if a prior old image is still running), installs their replacements, and relaunches Runner through `explorer.exe` rather than as its own child: Setup runs under Windows Redirection Guard, which child processes inherit, and a Runner started that way cannot traverse user-created junctions such as Codex's `bin` directory, so agents behind one are reported as not found. The finish-page **Launch Runner** option uses the same shell hand-off for the same reason. Existing external MCP/CLI processes keep using their old images. Every install and uninstall attempts to delete leftover `.old` files; files still running are retained without blocking either operation and are cleaned up on the next install after they exit. Settings, chats, and missions are retained. The installer filename and Installed Apps version retain the build stamp, while the app displays the base version on production and `Nightly (<sha>)` on nightly.

If installation aborts, Setup restores renamed binaries whose original paths are missing after rollback, then relaunches the old app when it exists. Runner records each install attempt beside the staged installer; on the next successful check, an unfinished attempt becomes an installer failure after signature verification. The dialog offers **Install and restart** again, and **Settings → Diagnostics → Open log folder** exposes the installer log path. A successful upgrade removes the old staged installer and attempt record during startup cleanup.

To preview the update indicator in a development build, set `$env:RUNNER_DEV_UPDATE_AVAILABLE = '0.8.0.20260907.1200'` before `.\make.cmd run`. Clear it with `Remove-Item Env:RUNNER_DEV_UPDATE_AVAILABLE` before the next launch. Release builds ignore this preview variable.

## Follow-up work

The unsigned Windows port shipped in 0.8.0 and signing followed with [#497](https://github.com/yicheng47/runner/issues/497); these items were not completed by that release:

- Confirm SmartScreen behavior for the signed stable installer on a fresh Windows 11 PC and record the result in the [signing spec](../features/archive/497-windows-code-signing.md).
- Complete detailed installed-build lifecycle, crash/relaunch, IME, resize, DPI, path, and update/data-retention acceptance. The [remaining validation checklist](../impls/archive/windows-nightly/impl_log.md#todo) preserves the specific cases and prior results. TRAE remains disabled by default on Windows and native validation is deferred unless requested.
- Investigate the shutdown `window not found` diagnostic. The separate development-only DXGI debug-interface warning is an optional gpui-ce debug probe and is skipped in release builds.
- Promote `Rust / Windows` to a required branch check after a week of green merges, planned no earlier than 2026-09-12; inspect current branch protection before changing it.
