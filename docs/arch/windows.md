# Windows development

Runner supports Windows x64 from 0.8.0. macOS and Windows are developed together on `main`, with separate platform chrome and shared application behavior. The [Windows port record](../impls/archive/windows-nightly/README.md) is archived; this document covers ongoing development and packaging.

## Local Windows development

Prerequisites: Git for Windows, the repository's pinned Rust toolchain with rustfmt and Clippy, MSVC v143 x64/x86 build tools, and the Windows SDK. After installing Rust, restart the terminal to pick up the updated `PATH`. The build shortcut also finds Cargo under `%CARGO_HOME%\bin` or `%USERPROFILE%\.cargo\bin`.

From the repository root, use `.\make.cmd build` to build the app and both CLI sidecars, or `.\make.cmd run` to build and launch. Add `--release` for optimized binaries. The shortcut works in PowerShell and Command Prompt without GNU Make and defaults to 12 build jobs unless `CARGO_BUILD_JOBS` is set.

Development outputs are `target\debug\Runner.exe`, `runner-agent-cli.exe`, and `runner-mcp.exe`; optimized local builds use `target\release`. Run `.\target\debug\Runner.exe` to launch an existing development build without rebuilding.

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

The script downloads the checksum-verified portable Inno Setup compiler into `target/tools`, builds optimized x64 binaries, and produces `target/x86_64-pc-windows-msvc/release/Runner-Setup-<version>.<stamp>-x64.exe`. The installer contains Runner, both CLI sidecars, and license notices. `-Stamp YYYYMMDD.HHMM`, `-Sha <commit>`, and `-Jobs <count>` are optional; CI supplies the build identity through environment variables. Building an installer does not install or launch Runner.

The packaging build links the C runtime statically and verifies with MSVC's `dumpbin` that no separate Visual C++ runtime is needed and the app uses the GUI subsystem. End users do not need Rust, MSVC, or PowerShell 7. Installers are not yet Authenticode-signed; SmartScreen may require **More info → Run anyway** for the initial download and installation. Both Windows packaging jobs publish a minisign `.sig` alongside each installer for in-app update verification.

Run installer checks without launching Runner:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\script\windows\test-installer.ps1
if ($LASTEXITCODE -ne 0) { throw 'Installer fixture tests failed' }
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\script\windows\test-installer.ps1 -SourceDir .\target\x86_64-pc-windows-msvc\release
if ($LASTEXITCODE -ne 0) { throw 'Installer payload tests failed' }
```

The first command uses generated test executables; the second uses the actual release payload. Each uses a separate temporary installation identity and checks install, upgrade, locked-file refusal, uninstall, and reinstall without modifying the installed Runner app or its data. Fixture mode also checks `/WAITPID` and successful/refused `/RELAUNCH` handoffs, launching and stopping only the temporary fixture executables.

## Update behavior

Packaged Windows builds always check at startup and every six hours: production builds use the latest stable GitHub release, and nightly builds use `nightly-win`. Settings → Updates also offers **Check for updates**. **Automatically download updates** defaults on; turning it off leaves checks enabled and waits for **Download** in the update dialog. The older automatic-check setting is ignored on Windows. Unstamped local builds do not check automatically.

Only a completed x64 installer with its exact `<installer>.sig` asset in the same release can be installed in-app. The updater streams bytes into `<app data>\updates\<installer>.partial`, reports progress in the centered update dialog, and verifies the minisign signature against `packaging/windows-update-public-key` before renaming the staged file. Cancel deletes the partial download; retry starts from zero. Startup removes partial and obsolete files and re-verifies a complete current candidate without downloading the installer again. Releases without a signature and legacy portable ZIP releases show **View downloads** only.

The icon beside Settings appears for available, ready, and failed updates; both it and the hero card's **Update** button open the same dialog. **Install and restart** starts the verified installer detached with `/SILENT /NORESTART /WAITPID=<Runner pid> /RELAUNCH=1 /LOG=<log directory>\update-<stamp>.log`, then uses Runner's normal quit flow to preserve session auto-resume and stop PTYs. Setup waits up to 30 seconds for Runner to exit before the existing in-use check, installs, and relaunches Runner. Settings, chats, and missions are retained. The installer filename and Installed Apps version retain the build stamp, while the app displays the base release version.

If installation is refused or aborted, Setup relaunches the old app when it still exists. Runner records each install attempt beside the staged installer; on the next successful check, an unfinished attempt becomes an installer failure after signature verification. The dialog offers **Install and restart** again, and **Settings → Diagnostics → Open log folder** exposes the installer log path. External Runner CLI processes that hold installed files open must be closed before retrying. A successful upgrade removes the old staged installer and attempt record during startup cleanup.

To preview the update indicator in a development build, set `$env:RUNNER_DEV_UPDATE_AVAILABLE = '0.8.0.20260907.1200'` before `.\make.cmd run`. Clear it with `Remove-Item Env:RUNNER_DEV_UPDATE_AVAILABLE` before the next launch. Release builds ignore this preview variable.

## Follow-up work

The unsigned Windows port shipped in 0.8.0; these items were not completed by that release:

- Windows signing, tracked in [#497](https://github.com/yicheng47/runner/issues/497) with its [feature spec](../features/497-windows-code-signing.md): choose the provider, configure credentials, and sign/verify the app, sidecars, installer, and uninstaller. The [archived signing plan](../impls/archive/windows-nightly/plan.md#phase-4--windows-installer-and-upgrades-remaining) retains the original scope and provider research.
- Complete detailed installed-build lifecycle, crash/relaunch, IME, resize, DPI, path, and update/data-retention acceptance. The [remaining validation checklist](../impls/archive/windows-nightly/impl_log.md#todo) preserves the specific cases and prior results. TRAE remains disabled by default on Windows and native validation is deferred unless requested.
- Investigate the shutdown `window not found` diagnostic. The separate development-only DXGI debug-interface warning is an optional gpui-ce debug probe and is skipped in release builds.
- Promote `Rust / Windows` to a required branch check after a week of green merges, planned no earlier than 2026-09-12; inspect current branch protection before changing it.
