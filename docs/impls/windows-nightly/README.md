# 437 — Windows nightly: program record

Implementation program for [feature 437 — Windows nightly](../../features/437-windows-nightly.md) ([#437](https://github.com/yicheng47/runner/issues/437)). The spec says *what*; this directory says *how, in what order, and what has landed*. Same shape as the [local-skills](../local-skills/README.md) record: this file is the condensed state and the decisions that bind, [plan.md](plan.md) covers Phases 0–4 with the per-file touch list, and [impl_log.md](impl_log.md) is the dated log and the Windows nightly stamps. Mission briefs are sent verbatim as the mission goal and are recorded in the log entry for that phase.

## Status (2026-09-06)

The Phase 0–3 implementation and Phase 4a unsigned installer are published on the public [`nightly-win` prerelease](https://github.com/yicheng47/runner/releases/tag/nightly-win) from `nightly-windows`. The first installer release, `0.7.5.20260906.0842` from `31ac40c`, contains the September 6 chrome, sidebar, font, keybinding, home-directory, event-log, shutdown, cursor, rename, and update-notification fixes. Both macOS and Windows CI jobs, real-payload installer checks, and anonymous installer/ZIP downloads passed. See the [nightly record](impl_log.md#windows-nightlies) for the latest published build. No changes have been merged to `main`.

Jason reports that cursor drift seems fixed and the crew mission and MCP smoke tests passed. Claude Resume works in the updated development app; the older installed/public build predates its transcript-path fix. Separate direct-session lifecycle and crash/relaunch tests, IME/resize/DPI, final UI/path checks, the remaining shutdown `window not found` diagnostic, and native macOS validation are outstanding. See the ordered [Todo list](impl_log.md#todo) and [review dispositions](review_log.md#2026-09-06--review-follow-up-codex-inline).

The Windows update indicator and manual-download page use separate Windows files. The Settings footer keeps the download button on the right and fills the remaining space with Settings. The existing debug environment variable previews the indicator. Native footer/update-page acceptance and a real old-nightly-to-new-nightly upgrade through the icon remain pending.

The accepted follow-up fixes Windows Claude transcript lookup and the collapsing Start Chat selector, initializes MCP for detected/enabled Windows clients while preserving opt-outs, enables TRAE by default only on macOS when detected, and adds `make.cmd clean`. Direct now appears first and is the default when no valid mode preference is saved; explicit saved choices and Runner-specific launch actions are preserved on both platforms. Local verification passed 1,005 workspace tests (one ignored), Clippy, formatting, and the development build; all 291 app tests passed again after the selector reorder. Jason accepted the rebuilt selector and authorized landing and releasing this batch; publication and cleanup results are recorded in the log. Required Windows branch checks follow the planned week of green merges.

**Phase 4 — Windows installer and upgrades is in progress.** Stage 4a's unsigned per-user Inno Setup installer, Start Menu/uninstall integration, update detection, and publication workflow have shipped. The build uses a checksum-pinned portable Inno Setup 6.7.3 compiler under `target/tools`. Automated smoke tests cover registration, shortcuts, same-version binary replacement, refusal when files are locked, uninstall, and reinstall. Jason confirmed that the local installer worked smoothly on JASONPC with no warning. A GitHub-downloaded upgrade preserving existing data still needs acceptance. Signing is Stage 4b; the artifacts remain unsigned. See the [phase and acceptance checklist](plan.md#phase-4--windows-installer-and-upgrades-remaining). Automatic download/installation and a production Windows channel remain later scope.

## End state

A public `nightly-win` pre-release built from `nightly-windows`, distributed only as an x64 per-user installer, signed after Phase 4b. The installer keeps `Runner.exe`, `runner-agent-cli.exe`, and `runner-mcp.exe` together, registers shortcuts/uninstall, and preserves user data across upgrades. Runner checks for newer nightlies and links to user-run installer upgrades. Direct chats and crew missions run natively on ConPTY with Job Objects, the `runner` CLI reaches agents through both shims, and MCP uses a named pipe. macOS behavior and its Sparkle updater stay unchanged.

## Local Windows development

Prerequisites: Git for Windows, Rust 1.97.1 through rustup (including rustfmt and Clippy), MSVC v143 x64/x86 build tools, and the Windows 11 SDK. These are installed on JASONPC; the first native build and workspace tests passed on 2026-09-06. Use the `nightly-windows` branch. After installing Rust, fully restart the terminal application to pick up the saved user `PATH`, or run `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"` in the current PowerShell for direct Cargo commands. The shortcut also finds Cargo under `%CARGO_HOME%\bin` or the default `%USERPROFILE%\.cargo\bin` when it is absent from `PATH`.

```powershell
Set-Location C:\Users\ROG\repos\yicheng47\runner
.\make.cmd build
```

Use `.\make.cmd run` for the daily build-and-run loop, or `.\make.cmd build --release` for optimized binaries. The shortcut works in PowerShell and Command Prompt without changing PowerShell's script execution policy. It builds the workspace with Cargo, including both CLI sidecars, stops if the build fails, and defaults to 12 build jobs unless `CARGO_BUILD_JOBS` is already set. GNU Make is not required; `cargo build --workspace` remains the direct build command.

The development build produces `target\debug\Runner.exe`, `runner-agent-cli.exe`, and `runner-mcp.exe` together. Launch `.\target\debug\Runner.exe` to start the existing build without rebuilding. Debug builds use `%APPDATA%\com.wycstudios.runner-dev` and its separate MCP pipe, so they can be tested separately from the downloaded release. Release builds put the three executables under `target\release`.

To clean local build outputs, close the development app, finish any Cargo builds, then run `.\make.cmd clean`. This delegates to `cargo clean` and removes the Cargo target directory, including compiled binaries, intermediate files, locally packaged installers/ZIPs, and packaging tools cached there. `.\make.cmd clean --release` limits cleanup to Cargo's release outputs. Installed Runner, both AppData directories, Rust/MSVC, and Cargo's shared dependency cache are retained. The next local build recompiles the removed outputs.

Windows update notifications use the `nightly-win` release's uploaded x64 installer timestamps; the checker also understands legacy ZIP assets. Packaged nightlies check at startup and every six hours when automatic checking is enabled. A newer build shows the download icon beside Settings; clicking it opens the Windows release page. Settings → Updates also offers Check for updates and View downloads. Download `Runner-Setup-…-x64.exe`, close Runner normally, and run the installer over the existing installation. Portable ZIP distribution was retired on 2026-09-06. Automatic installation is not implemented. Unstamped local builds cannot be compared with published nightlies and do not check automatically.

To preview the update icon and footer layout in a debug build, set the existing environment variable before launching:

```powershell
$env:RUNNER_DEV_UPDATE_AVAILABLE = '0.7.5.20260907.1200'
.\make.cmd run
```

The preview remains visible after a manual check and opens the same Windows downloads page. Before the next launch, clear it with `Remove-Item Env:RUNNER_DEV_UPDATE_AVAILABLE`. Release builds ignore this preview variable.

The development-only `HRESULT(0x887A002D)` followed by “Failed to get DXGI debug interface” comes from gpui-ce 0.3.3 probing the optional DirectX debug interface. On JASONPC the same startup then logs the RTX 4090 D and successful Direct3D 11.1 device creation. GPUI's release build skips that probe. This diagnostic does not require a renderer change; the separate shutdown `window not found` message is still being tracked.

For a checkout reused from before Phase 2, check `git ls-files --eol` if SQL migration or terminal snapshot tests differ only in line endings. The existing `.gitattributes` requires LF for migration SQL, system-prompt fixtures, and terminal text snapshots, but Git can leave old CRLF files in place when switching branches. Normalize those working files to LF without changing their contents or re-blessing snapshots.

Run the same Windows checks as CI with native Cargo commands; GNU Make is optional:

```powershell
cargo check --workspace --all-targets
if ($LASTEXITCODE -ne 0) { throw 'Cargo check failed' }
cargo clippy --workspace --all-targets -- -D warnings
if ($LASTEXITCODE -ne 0) { throw 'Clippy failed' }
cargo test --workspace --no-fail-fast
if ($LASTEXITCODE -ne 0) { throw 'Tests failed' }
cargo fmt --all --check
if ($LASTEXITCODE -ne 0) { throw 'Formatting check failed' }
```

## Building and testing the Windows installer

From PowerShell in the repository, run:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\script\bundle-windows.ps1
```

This downloads the checksum-verified portable Inno Setup 6.7.3 compiler into `target/tools` on first use, builds optimized x64 binaries with a UTC nightly stamp, and produces `target/x86_64-pc-windows-msvc/release/Runner-Setup-<version>.<stamp>-x64.exe`. The installer includes the app, CLI sidecars, and license notices; no portable ZIP is produced. The Windows packaging build links the C runtime statically and verifies with MSVC's `dumpbin` that no Visual C++ runtime DLL is required and the app uses the GUI subsystem. `-Stamp YYYYMMDD.HHMM`, `-Sha <commit>`, and `-Jobs <count>` are optional; CI supplies the stamp and SHA through environment variables. Building an installer does not install or launch Runner. The script requires the existing Rust/MSVC prerequisites; end users do not need the build tools, a separate Visual C++ runtime installation, or PowerShell 7.

Run the installer directly on the PC to install under `%LOCALAPPDATA%\Programs\Runner`, then launch Runner from the Start Menu. These testing installers are unsigned, so Windows may show a warning. The installer adds an Installed Apps entry; uninstall removes its files and shortcut but retains `%APPDATA%\com.wycstudios.runner`. Release and portable builds share that data directory. Development builds use `%APPDATA%\com.wycstudios.runner-dev` and are separate. Settings and missions need no export/import when moving from a portable release to the installer.

For automated installer checks without launching Runner:

```powershell
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\script\windows\test-installer.ps1
powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\script\windows\test-installer.ps1 -SourceDir .\target\x86_64-pc-windows-msvc\release
```

The first command uses small generated test executables; the second tests the actual release payload. Each uses a unique test installation identity, a directory under `%TEMP%`, and a temporary Start Menu/uninstall entry. Neither launches the packaged app or reads/writes Runner's data. Tests verify same-version upgrades (including older payload timestamps), each locked binary, refusal to uninstall locked files, remembered installation paths, shortcut targets/home cwd, uninstall cleanup, retention of unowned files, and reinstall. Logs and payloads remain under the printed temporary path; successful tests remove their registration and shortcut. Live agent/MCP and saved-mission acceptance still belongs to the PC pass.

## Decisions that bind

The plan's [Decisions](plan.md#decisions) section is the full list. The ones a mission is most likely to trip over:

1. **`nightly-windows` is the base branch, not `main`.** Created 2026-09-05 from `main` at `7cf7dd1`, checked out in the `../runner-windows` worktree. Every phase and fix PR bases on it and merges into it; `nightly.yml` builds from it; `ci.yaml` triggers on it. `main` stays the macOS product. It has no branch protection, so `gh pr merge --auto` merges at once there.
2. **The release tag is `nightly-win`, never `nightly-windows`.** A tag with the branch's name makes `git push origin nightly-windows` an ambiguous refspec after the first cut. Decided during the Phase 1 review.
3. **Workflow-file PRs land with a local `--no-ff` merge.** The `gh` token lacks the `workflow` scope, so `gh pr merge` refuses any PR that touches `.github/workflows/`. #482, #483 and #485 all landed as `git checkout nightly-windows && git merge --no-ff <branch> && git push`; GitHub still records the PR as merged.
4. **Keymap: replace Cmd with Ctrl, preserving existing Shift modifiers.** Jason requested this on 2026-09-06, superseding the earlier extra-Shift mapping. New chat is `Ctrl+N`, new window `Ctrl+Shift+N`, search `Ctrl+K`, split right/down `Ctrl+D` / `Ctrl+Shift+D`, and close `Ctrl+W`. Pane and mission-tab cycling use `Ctrl+[` / `Ctrl+]`; page history adds Shift. Terminal copy/paste use `Ctrl+C/V`; without a selection, `Ctrl+C` reaches the PTY as an interrupt. App shortcuts take precedence over matching terminal shortcuts. Fullscreen remains `F11`; Quit and both Hide bindings are omitted on Windows. Labels follow active bindings, links use Ctrl-click, and holding Ctrl reveals sidebar tab numbers.
5. **Runner draws its own title row on Windows.** The OS bar is hidden (`appears_transparent: true`). Jason's 2026-09-06 local UI pass uses a full-width 32 px bar above the sidebar and workspace, with sidebar/history controls on the left and caption buttons on the right. Chrome lives in separate `platform_ui/windows.rs` and `platform_ui/macos.rs` implementations, with font mappings in the adjacent `fonts_windows.rs` and `fonts_macos.rs`; the macOS layout is retained. Caption buttons stay above overlays, and gpui-ce performs minimize, maximize and close on non-client mouse-up. The published `1853` nightly still carries #484's earlier 44 px layout; the local redesign is awaiting visual confirmation.
6. **Native local builds are authorized on JASONPC.** Jason requested this on 2026-09-06. Use the Windows checkout at `C:\Users\ROG\repos\yicheng47\runner` with Rust and MSVC; CI remains the shared verification gate. Missions never launch `Runner.exe` over ssh and never install unrelated tools on the PC unprompted.
7. **cfg-split, verbatim moves, no macOS change.** Platform code is selected at compile time; unix bodies move without edits; every phase leaves the macOS build, tests and `Rust / macOS` unchanged. Since Phase 2, the Windows CI job runs the workspace tests as well as check and Clippy.
