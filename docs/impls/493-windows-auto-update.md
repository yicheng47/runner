# 493 — Automatic updates on Windows

Tracking issue: [#493](https://github.com/yicheng47/runner/issues/493). Feature, P2, Windows. Spec: [`docs/features/493-windows-auto-update.md`](../features/493-windows-auto-update.md). Design: `design/windows-updates.pen` (frame ids in [`design/README.md`](../../design/README.md)). Baseline `main` at `6f2918d` (2026-09-07).

## What ships

A packaged Windows build finds the next build on its own channel, downloads it in the background, verifies a minisign signature against a key compiled into the app, and offers **Install and restart** in one centered dialog. Pressing it quits Runner into a silent Inno Setup run that waits for the old process, installs, and relaunches the new version. Settings → Updates takes the macOS pane's shape: a hero card with the version, a one-line status, and one button; a download toggle; Last checked. macOS and Sparkle are untouched.

Three phases, in order, each reviewed before the next starts. Phase 1 is CI and installer work with no app change and can be exercised on a `nightly-win` build by itself.

## Working tree at start

The tree already holds three uncommitted files that belong to this feature: `docs/features/493-windows-auto-update.md`, `design/README.md`, and `design/windows-updates.pen`, plus this brief. They are not unrelated changes. First action on the feature branch: commit them as `docs(features): specify #493 Windows auto-update` before touching anything else.

## Phase 1 — Signed assets and installer handoff

### CI

- Both Windows packaging jobs sign the installer right after `test-installer.ps1` passes: `build-windows` in `.github/workflows/release.yml` and `nightly-windows` in `.github/workflows/nightly.yml`. The step runs `npx --yes @tauri-apps/cli@2.10.1 signer sign --private-key "$TAURI_SIGNING_PRIVATE_KEY" --password "$TAURI_SIGNING_PRIVATE_KEY_PASSWORD" <installer>` (the same invocation the macOS bridge step already uses, `release.yml` "Build and sign the v0.6.0 Tauri bridge") and produces `<installer>.sig` beside it. `windows-latest` has Node; run the step under `shell: bash` so the key-id check below can reuse the bridge step's pipeline.
- Both jobs require the two secrets up front, the way the macOS job requires the Sparkle key ("Require release signing keys"), and assert the signing key id is `23fee1fa29746d59` with the bridge step's decode: `base64 --decode < "$sig" | sed -n 2p | base64 --decode | xxd -p -s2 -l8`.
- `release.yml`: the `.sig` joins the `release-windows` artifact (`path:` becomes a two-line list); `publish` already uploads `release-artifacts/*`.
- `nightly.yml`: `gh release upload nightly-win "$new_setup" "$new_setup.sig" --clobber`. The pruning loop keys on installers, not on every asset: collect `Runner-Setup-*-x64.exe` names sorted by stamp, keep the newest ten, and for each stale installer delete both the `.exe` and its `.sig`. The existing loop already excludes signatures from its count; paired deletion prevents orphaned signatures when installers expire.
- Commit `packaging/windows-update-public-key` with exactly this content (the key 0.5.2's `tauri.conf.json` trusted; the same key id as above):

  ```
  untrusted comment: minisign public key: 596D7429FAE1FE23
  RWQj/uH6KXRtWbyExZjR12wAuu86bmQB2NpXjhhJ9fxK8JBy5oqh/obP
  ```

### Installer

`script/windows/runner.iss` gains two switches read with `{param:WAITPID|0}` and `{param:RELAUNCH|0}`:

- `PrepareToInstall`: when `WAITPID` is non-zero, `OpenProcess(SYNCHRONIZE, False, pid)` (import `OpenProcess@kernel32.dll stdcall`, likewise `WaitForSingleObject` and `CloseHandle`), wait up to 30 000 ms, close the handle, then fall through to the existing `ApplicationFilesInUse` check. A pid that no longer exists is not an error.
- A second `[Run]` entry, `Filename: "{app}\Runner.exe"; WorkingDir: "{%USERPROFILE}"; Flags: nowait; Check: RelaunchRequested`, with no `postinstall` and no `skipifsilent`, so it runs after a silent install. The existing checked-by-hand entry stays.
- `CurStepChanged(ssDone)` sets an `InstallCompleted` flag. `DeinitializeSetup`: when `RELAUNCH=1`, the install did not complete, and `{app}\Runner.exe` exists, `Exec` it with `ewNoWait`. This is what keeps the old app usable after an in-use refusal or a mid-copy abort.
- Do not touch `CloseApplications`, `RestartApplications`, `PrivilegesRequired`, the in-use check, or the uninstall path.

### Installer tests

`script/windows/test-installer.ps1` gains three cases using the same fixture executables and temporary identity as today:

1. `/WAITPID=<pid>` where the pid belongs to a helper that holds the fixture `Runner.exe` open and exits after about three seconds: the install succeeds, and the log shows it waited rather than refused.
2. `/RELAUNCH=1` on a successful silent install: a process running from the installed `Runner.exe` path exists afterwards; the test kills it.
3. `/RELAUNCH=1` with a helper holding the file open for the whole run: the install is refused as today, and a process running from the previously installed `Runner.exe` exists afterwards; the test kills both.

`-SourceDir` payload mode keeps working unchanged.

### Phase 1 verification

- `powershell.exe -NoProfile -ExecutionPolicy Bypass -File .\script\windows\test-installer.ps1` passes locally in fixture mode. Payload mode is not required locally (it needs a release build); CI runs it.
- Both workflow files parse (`gh workflow view` or a YAML linter). Do not dispatch a nightly or a release; the human cuts the nightly.
- Reviewer checks: the pruning arithmetic, that `.sig` is uploaded in both workflows, the key-id assertion, that the `[Run]` relaunch entry is not `postinstall`, and that `DeinitializeSetup` cannot relaunch after a completed install.

## Phase 2 — Updater core

All in `crates/runner-app/src/updater/windows.rs` and `crates/runner-app/src/updater.rs`, Windows-only behind the existing `cfg(windows)` split.

- **Dependency**: `minisign-verify = "0.2"` under `[target.'cfg(windows)'.dependencies]` in `crates/runner-app/Cargo.toml`. `base64` is already a dependency. Nothing else new.
- **States**: replace `available: Option<UpdateInfo>` plus the error string with an enum the spec's table names: `UpToDate { checking: bool }`, `Available { info, installer_url, sig_url: Option<String> }`, `Downloading { received, total }`, `Ready { path, info }`, `Failed { step, message, info }`. Keep `Updater::available()` returning `Option<&UpdateInfo>` for the sidebar hint: on Windows it answers `Some` only in Available, Ready, and Failed, which is the spec's icon-visibility rule. Add a `state()` accessor for the pane and the dialog. macOS keeps its `available` field and Sparkle transitions exactly as they are.
- **Pairing**: `Asset` gains `browser_download_url`. `available_update` still picks the newest completed `Runner-Setup-*-x64.exe`, and reports it installable only when a completed `<name>.sig` sits in the same release. The `Runner-Nightly-*.zip` form stays recognized for the version comparison and is never installable.
- **Download**: the existing blocking `reqwest` client on the background executor, streamed in chunks into `<app data>\updates\<installer name>.partial`. `Updater::new` gains the updates directory (`NativePaths.app_data_dir` joined with `updates`, from `main.rs` where the updater is built). Progress is an `Arc<AtomicU64>` pair the reader updates and the entity reads on a 250 ms timer; cancel is an `Arc<AtomicBool>` the reader checks per chunk, after which the partial file is deleted. Retry restarts from zero.
- **Verify**: fetch the `.sig` asset, base64-decode it to the minisign text, `minisign_verify::Signature::decode`, `PublicKey::decode(include_str!("../../../../packaging/windows-update-public-key"))`, verify the whole file, then rename `.partial` to the final name. A failure deletes the file and lands in `Failed { step: Verify }`. Nothing is executed in this phase.
- **Sweep** on `start`: delete `*.partial`, delete installers whose stamp is not the current candidate's or is not newer than the installed stamp, and re-verify a complete installer for the current candidate into `Ready` without downloading.
- **Setting**: `AppSettings.automatically_download_updates: bool`, `#[serde(default = "true")]`-style default on, persisted like its neighbours. When on, `finish_windows_check` starts the download as soon as an installable update is found. `automatically_check_for_updates` is ignored by the Windows updater: packaged Windows builds always check; the field stays for macOS.
- **Tests** next to the existing ones: pairing with and without a sig, sweep behaviour against a temp dir, the state transitions on check success, download failure, verify failure, and cancel. For verification, generate a throwaway keypair with `npx --yes @tauri-apps/cli@2.10.1 signer generate -w <temp>` and sign a small fixture file; commit the public key, the fixture, and its `.sig` under `crates/runner-app/tests/fixtures/updates/`. Never commit a private key. `RUNNER_DEV_UPDATE_AVAILABLE` keeps working for the dev preview.

### Phase 2 verification

- `cargo test -p runner-app` green, `cargo clippy --locked --workspace --all-targets --profile ci -- -D warnings` clean, `cargo fmt --all --check` clean.
- Reviewer checks: no state reachable where an unverified file has the final name; cancel deletes the partial; the sweep never deletes the current candidate's verified file; the macOS module and the macOS tests are byte-identical.

## Phase 3 — UI and handoff

- **Update dialog**: a new overlay following the `ConfirmDialog` pattern in `crates/runner-app/src/ui/overlay.rs` (same 60% scrim, same 420 px card, click outside and Escape close it). One dialog entity held on `NativeRoot` like the other confirms; a second open request while it is up is a no-op. Content per state from design frame `rZRA3`: Available (View downloads, Download; no Download when not installable, with the notify-only copy), Downloading (4 px accent bar, "18.4 MB of 41.2 MB · 45%", Cancel), Ready (Later, Install and restart), Failed (body names the step, View downloads, Retry; after an installer failure the primary is Install and restart). Subtitle copy is in the frame.
- **Triggers**: `platform_ui::activate_update_hint` on Windows opens the dialog instead of the browser; `update_hint_tooltip` returns "Runner <v> is ready to install" for Ready and "Runner <v> is available" otherwise. The hero button in the Updates pane opens the same dialog. macOS `activate_update_hint` stays as it is.
- **Updates pane** (`crates/runner-app/src/surfaces/settings/updates_windows.rs`): the hero card copied from the macOS pane's layout in `updates.rs` (56 px app icon, "Runner" with the version chip, status line, button on the right: **Check for updates** in UpToDate, spinning and disabled while checking, **Update** otherwise), then one card with **Automatically download updates** and **Last checked**. Remove the Installed version, Update status, Windows downloads, and Automatically check rows. Header line: "Runner checks for new Windows builds at startup and every six hours, downloads them in the background, and installs when you choose." Design frames `uDGHz` and `Jr0d9`.
- **Handoff** on Install and restart: spawn the staged installer with `std::process::Command`, `creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)` via `std::os::windows::process::CommandExt` (see `crates/runner-backend/src/session/process/windows.rs` for the pattern), stdio null, arguments `/SILENT /NORESTART /WAITPID=<std::process::id()> /RELAUNCH=1 /LOG=<log_dir>\update-<stamp>.log`, then `cx.quit()`. Nothing else changes about quit: `on_app_quit` stamps sessions for auto-resume and stops PTYs as today.
- **Docs**: the Update behavior section of `docs/arch/windows.md`, the Windows paragraph in `README.md`, `script/windows/release-notes.md`, and the release-notes string in `release.yml` stop telling people to close Runner and run the installer by hand.

### Phase 3 verification

- Same three commands as phase 2, plus `cargo test -p runner-app` covering the tooltip and hint-visibility functions.
- Reviewer checks: the dialog is the only place that spawns the installer; the spawn is detached and inherits no handles; the macOS pane, `updates.rs`, and the macOS `platform_ui` are unchanged; no progress bar on the pane.

## Rules of the road

- Windows-only. `crates/runner-app/src/updater.rs`'s macOS module, `surfaces/settings/updates.rs`, `platform_ui/macos.rs`, and everything Sparkle stay byte-identical.
- No new dependencies beyond `minisign-verify`. No async runtime, no new HTTP client.
- Follow `AGENTS.md`. Match the surrounding code; comments only for non-obvious intent.
- Do not launch the app (`.\make.cmd run`) and do not dispatch workflows or touch GitHub releases; the human does the nightly and the smoke test. Do not run the installer against the real per-user installation; `test-installer.ps1` uses its own identity.
- This machine is Windows. Use PowerShell for the scripts and the native Cargo commands from [Local Windows development](../arch/windows.md#local-windows-development).
- Mission authorization: after the reviewer reports each phase clean, commit that phase on the feature branch with an imperative subject and the scopes in `AGENTS.md` (`ci`, `validation`, `updater`, `ui`, `docs`). No push, no pull request, no merge: the human reviews the branch and decides the PR boundaries.

## Jason's smoke test (after landing)

1. Dispatch a Windows nightly from the branch's merge; confirm `nightly-win` carries `Runner-Setup-<v>-x64.exe` and `.sig`. Install it by hand once.
2. Dispatch a second Windows nightly. In the installed app, expect the sidebar icon within a check cycle (or press Check for updates), then Update → Install and restart. Expect: Runner closes, Setup's progress window runs a few seconds, Runner reopens, About shows the new stamp, chats and missions are intact, no SmartScreen interstitial.
3. With an external `runner-mcp.exe` held open (this Claude Code session's MCP proxy will do), Install and restart: Setup refuses with its message, the old Runner relaunches, the dialog shows Failed with the log path in Diagnostics.
4. Turn Automatically download updates off, cut a third nightly: the icon appears, the dialog offers Download, nothing is fetched until pressed.

## Non-goals

Authenticode signing (#497), a WinVerifyTrust gate, resumable downloads, channel switching, portable ZIP installs, ARM64, CLI self-updates (#475), any macOS change, and a running-work warning in the dialog (#491 closed; quit stamps sessions for auto-resume as it does today).
