# 437 — Windows nightly: program record

Implementation program for [feature 437 — Windows nightly](../../features/437-windows-nightly.md) ([#437](https://github.com/yicheng47/runner/issues/437)). The spec says *what*; this directory says *how, in what order, and what has landed*. Same shape as the [local-skills](../local-skills/README.md) record: this file is the condensed state and the decisions that bind, [plan.md](plan.md) covers Phases 0–4 with the per-file touch list, and [impl_log.md](impl_log.md) is the dated log and the Windows nightly stamps. Mission briefs are sent verbatim as the mission goal and are recorded in the log entry for that phase.

## Status (2026-09-06)

The Phase 0–3 implementation has landed on `nightly-windows`; remaining acceptance and Phase 3 validation continue, and Phase 4 installer/upgrades is planned. The September 6 checkpoint includes the chrome, sidebar, font, keybinding, home-directory, event-log, shutdown, cursor, rename, and update-notification fixes plus review follow-ups. Final native Windows validation passed 1,000 workspace tests (one ignored), workspace check, Clippy, formatting, and diff checks. The checkpoint is for `nightly-windows`; these fixes are not yet in a published nightly.

Jason has run the app and interacted with Codex; his latest feedback is that cursor drift seems fixed and the crew mission smoke test passed. The first local mission pass exposed a sidebar rename panic, now fixed with a regression that reproduced the original failure; the short Settings footer is also fixed. All 285 app tests and workspace Clippy passed after those follow-ups, and the executable was rebuilt. Separate direct-session lifecycle and crash/relaunch tests, IME/resize/DPI, final UI/path checks, the remaining shutdown `window not found` diagnostic, and native macOS validation are outstanding. See the ordered [Todo list](impl_log.md#todo) and [review dispositions](review_log.md#2026-09-06--review-follow-up-codex-inline).

The Windows update indicator and manual-download page are now implemented in separate Windows files. The Settings footer keeps the download button on the right and fills the remaining space with Settings. The existing debug environment variable previews the indicator. All 287 app tests passed with that preview enabled; workspace Clippy, formatting, diff checks, and the build passed. Native footer/update-page acceptance remains pending.

The latest public zip is still `Runner-Nightly-0.7.5.20260905.1853-x64.zip`, before the local fixes; the public API was rechecked during the update-indicator work. A fresh nightly follows acceptance, an authorized commit/push, and CI. Required Windows branch checks follow the planned week of green merges.

**Phase 4 — Windows installer and upgrades remains to be implemented.** Jason requested the per-user Inno Setup EXE installer as the primary download, signed app/sidecars and installer, Start Menu/uninstall integration, and upgrades that preserve settings and missions. The update indicator will lead to the newer installer, which the user downloads and runs; the checker must recognize installer assets. ZIP can remain an optional portable download. See the [phase and acceptance checklist](plan.md#phase-4--windows-installer-and-upgrades-remaining). Automatic download/installation and a production Windows channel remain later scope.

## End state

A public `nightly-win` pre-release built from `nightly-windows`, with a signed x64 per-user installer as the primary download after Phase 4 and an optional portable ZIP. The installer keeps `Runner.exe`, `runner-agent-cli.exe`, and `runner-mcp.exe` together, registers shortcuts/uninstall, and preserves user data across upgrades. Runner checks for newer nightlies and links to user-run installer upgrades. Direct chats and crew missions run natively on ConPTY with Job Objects, the `runner` CLI reaches agents through both shims, and MCP uses a named pipe. macOS behavior and its Sparkle updater stay unchanged.

## Local Windows development

Prerequisites: Git for Windows, Rust 1.97.1 through rustup (including rustfmt and Clippy), MSVC v143 x64/x86 build tools, and the Windows 11 SDK. These are installed on JASONPC; the first native build and workspace tests passed on 2026-09-06. Use the `nightly-windows` branch. After installing Rust, fully restart the terminal application to pick up the saved user `PATH`, or run `$env:Path = "$env:USERPROFILE\.cargo\bin;$env:Path"` in the current PowerShell for direct Cargo commands. The shortcut also finds Cargo under `%CARGO_HOME%\bin` or the default `%USERPROFILE%\.cargo\bin` when it is absent from `PATH`.

```powershell
Set-Location C:\Users\ROG\repos\yicheng47\runner
.\make.cmd build
```

Use `.\make.cmd run` for the daily build-and-run loop, or `.\make.cmd build --release` for optimized binaries. The shortcut works in PowerShell and Command Prompt without changing PowerShell's script execution policy. It builds the workspace with Cargo, including both CLI sidecars, stops if the build fails, and defaults to 12 build jobs unless `CARGO_BUILD_JOBS` is already set. GNU Make is not required; `cargo build --workspace` remains the direct build command.

The development build produces `target\debug\Runner.exe`, `runner-agent-cli.exe`, and `runner-mcp.exe` together. Launch `.\target\debug\Runner.exe` to start the existing build without rebuilding. Debug builds use `%APPDATA%\com.wycstudios.runner-dev` and its separate MCP pipe, so they can be tested separately from the downloaded release. Release builds put the three executables under `target\release`.

Windows update notifications use the `nightly-win` release's uploaded x64 ZIP timestamps. Packaged nightlies check at startup and every six hours when automatic checking is enabled. A newer build shows the download icon beside Settings; clicking it opens the Windows release page. Settings → Updates also offers Check for updates and View downloads. Download the ZIP, close Runner, then extract and launch the new copy with both CLI sidecars beside it. Automatic installation is not implemented. Unstamped local builds cannot be compared with published nightlies and do not check automatically.

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

## Decisions that bind

The plan's [Decisions](plan.md#decisions) section is the full list. The ones a mission is most likely to trip over:

1. **`nightly-windows` is the base branch, not `main`.** Created 2026-09-05 from `main` at `7cf7dd1`, checked out in the `../runner-windows` worktree. Every phase and fix PR bases on it and merges into it; `nightly.yml` builds from it; `ci.yaml` triggers on it. `main` stays the macOS product. It has no branch protection, so `gh pr merge --auto` merges at once there.
2. **The release tag is `nightly-win`, never `nightly-windows`.** A tag with the branch's name makes `git push origin nightly-windows` an ambiguous refspec after the first cut. Decided during the Phase 1 review.
3. **Workflow-file PRs land with a local `--no-ff` merge.** The `gh` token lacks the `workflow` scope, so `gh pr merge` refuses any PR that touches `.github/workflows/`. #482, #483 and #485 all landed as `git checkout nightly-windows && git merge --no-ff <branch> && git push`; GitHub still records the PR as merged.
4. **Keymap: replace Cmd with Ctrl, preserving existing Shift modifiers.** Jason requested this on 2026-09-06, superseding the earlier extra-Shift mapping. New chat is `Ctrl+N`, new window `Ctrl+Shift+N`, search `Ctrl+K`, split right/down `Ctrl+D` / `Ctrl+Shift+D`, and close `Ctrl+W`. Pane and mission-tab cycling use `Ctrl+[` / `Ctrl+]`; page history adds Shift. Terminal copy/paste use `Ctrl+C/V`; without a selection, `Ctrl+C` reaches the PTY as an interrupt. App shortcuts take precedence over matching terminal shortcuts. Fullscreen remains `F11`; Quit and both Hide bindings are omitted on Windows. Labels follow active bindings, links use Ctrl-click, and holding Ctrl reveals sidebar tab numbers.
5. **Runner draws its own title row on Windows.** The OS bar is hidden (`appears_transparent: true`). Jason's 2026-09-06 local UI pass uses a full-width 32 px bar above the sidebar and workspace, with sidebar/history controls on the left and caption buttons on the right. Chrome lives in separate `platform_ui/windows.rs` and `platform_ui/macos.rs` implementations, with font mappings in the adjacent `fonts_windows.rs` and `fonts_macos.rs`; the macOS layout is retained. Caption buttons stay above overlays, and gpui-ce performs minimize, maximize and close on non-client mouse-up. The published `1853` nightly still carries #484's earlier 44 px layout; the local redesign is awaiting visual confirmation.
6. **Native local builds are authorized on JASONPC.** Jason requested this on 2026-09-06. Use the Windows checkout at `C:\Users\ROG\repos\yicheng47\runner` with Rust and MSVC; CI remains the shared verification gate. Missions never launch `Runner.exe` over ssh and never install unrelated tools on the PC unprompted.
7. **cfg-split, verbatim moves, no macOS change.** Platform code is selected at compile time; unix bodies move without edits; every phase leaves the macOS build, tests and `Rust / macOS` unchanged. Since Phase 2, the Windows CI job runs the workspace tests as well as check and Clippy.
