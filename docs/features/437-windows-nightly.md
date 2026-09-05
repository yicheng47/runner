# Windows nightly — a pre-release a friend can download

Tracking issue: [#437](https://github.com/yicheng47/runner/issues/437) (closed won't-do 2026-08-27; reopened for this scope). Status: gap analysis, planned. Priority P2.

## Motivation

A friend wants to run Runner on Windows, and Jason now has a Windows PC to test on, which was the missing piece when #437 was closed: the objection was a second platform with no test loop, not the port itself. The target is deliberately small — a **nightly pre-release** built by CI that anyone can download from the releases page without a GitHub account, run from a zip, and report on. No installer, no signing, no updater, no production channel. Those are a later decision once the nightly has users.

This spec replaces the #437 inventory (surveyed 2026-08-23 on `b9cbb9b`) with a measured one from `main` at `49543b2` (0.7.4, 2026-09-04). The measurement changes the shape of the work: the compile gap is about seventeen errors, the app already builds against GPUI's Windows backend, and the effort sits in three behavioural areas the compiler cannot see — process lifecycle, agent spawning, and the MCP transport — plus a CI job.

## Where main stands

Measured with `cargo check --target x86_64-pc-windows-msvc` from macOS, with a stub C compiler standing in for the bundled SQLite build (a check never links, so an empty object is enough). Temporary edits were used to reach the crates behind each failure and then reverted; nothing from that exercise is on disk.

| Crate | Errors | Where |
| --- | --- | --- |
| `runner-core` | 0 | — |
| `runner-cli` | 2 | `cli/src/mcp.rs`: `tokio::net::UnixStream`, and no Windows branch in `socket_path`. The `runner-agent-cli` binary is clean. |
| `runner-backend` | 5 | `mcp/mod.rs`, `mcp/server.rs`: the Unix-socket listener and stream; `ops/mcp.rs`: `OpenOptionsExt::mode(0o600)`. |
| `runner-app` + `runner-terminal` | 3 | `bootstrap.rs` names `session::pty_runtime`, which is `#[cfg(unix)]`. Everything else, including `gpui-ce` 0.3.3's DirectX/DirectWrite backend, type-checks. |
| `pty_runtime.rs` with its gate lifted | 7 | `libc::kill` and `libc::SIGKILL` in `signal_process_group`, `signal_process`, `is_pid_alive`, plus `spawn.rs::terminate_headless_fork`. `portable-pty` already builds its ConPTY backend. |

What the #437 inventory got wrong, in both directions:

- The mac glue in the app already has non-mac fallbacks: `mac_chrome::sync_traffic_lights` is a no-op stub, `window_state` falls back to `cx.displays()`, the Sparkle updater is behind `all(target_os = "macos", feature = "updater")`, the app icon and wake observer are gated in `main.rs`, and `archived.rs` has a `chrono` timestamp fallback. `cli_install.rs` already carries `cfg!(windows)` `.exe` names from the Tauri era.
- Hook injection (spec 52) never shipped, so it is not a gap. `launch.rs::write_launch_script` is referenced only by its tests; agents are spawned directly through `CommandBuilder`, so there is no `launch.sh` to port.
- The inventory missed the MCP transport entirely. The app listener, the `runner-mcp` proxy, and the settings page all assume a Unix domain socket at `<app-data>/mcp.sock`. This is the one genuinely new item.

## The gap, by layer

Everything below compiles or nearly compiles. The list is what would be wrong at runtime.

### 1. Process lifecycle — the real port

`pty_runtime.rs` is built on `portable-pty`, which on Windows opens a ConPTY, resolves the command through `PATH` with `PATHEXT` (so `claude` finds `claude.cmd`), and kills with `TerminateProcess` on the direct child only. Runner's own lifecycle helpers are all POSIX signals:

- **Stop and escalation.** `SIGHUP` to the process group, a grace window, then `SIGKILL` to the group, then a `SIGTERM`/`SIGKILL` sweep over descendants snapshotted before the stop. Windows has none of this. The replacement is one Job Object per session: assign the child at spawn with `JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE`, and stopping becomes `TerminateJobObject`, which takes every descendant with it, including the tool shells claude-code spawns. The descendant snapshot and the two-stage sweep disappear on Windows. Closing the pseudoconsole is the "graceful" step; there is no `SIGHUP` handler to give time to.
- **Liveness and identity.** `is_pid_alive` (`kill(pid, 0)`) becomes `OpenProcess` + `GetExitCodeProcess`. The command-line identity check that guards the startup orphan sweep uses `sysctl` on macOS and `/proc` on Linux; the `ps` fallback that would compile on Windows returns nothing useful. Windows needs `QueryFullProcessImageNameW` or a toolhelp snapshot. Pids are reused aggressively on Windows, so the identity check matters more there, not less.
- **Orphan sweep at startup.** Same two primitives; once they exist the sweep works unchanged.
- **Resize.** ConPTY repaints the full screen on resize, so the `runtime_clears_on_resize` behaviour the terminal layer assumes becomes platform-dependent. Expect a fixture pass on the PC.

### 2. Spawning agents and the bundled CLI

- **`PATH` composition.** `launch::compose_path` and its tests split and join on `:`. Use `std::env::join_paths`/`split_paths`, which pick `;` on Windows.
- **Login-shell discovery.** `shell_path::resolve_login_shell_env` is already a `NoShell` stub off unix. On Windows that is the right answer, not a gap: a GUI app launched from Explorer inherits the user's full registry environment, so there is no minimal-`PATH` problem to solve. The work is making `runtime_status` present `NoShell` as "inherited environment" in Settings → Agents instead of as a discovery failure, and making `find_executable` try `PATHEXT`.
- **Home and app data.** Fifteen call sites read `HOME`, which Windows does not set for GUI processes. One `home_dir()` helper reading `USERPROFILE` first. `bootstrap::paths_for_home` hardcodes `Library/Application Support` and `Library/Logs`; Windows gets `%APPDATA%\com.wycstudios.runner` and a `logs` directory under it, and `cli/src/mcp.rs` needs the same branch. Runtime config paths (`~/.claude`, `~/.codex/config.toml`) are the same relative shape under `USERPROFILE`, which is where both CLIs put them on Windows.
- **The `runner` shim.** `install_session_runner_shim` writes a `#!/bin/sh` script. Claude Code on Windows runs its Bash tool through Git Bash, which honours shebangs and finds extension-less scripts on `PATH`; codex and PowerShell-spawned tools do not. Write both a `runner` sh shim and a `runner.cmd` shim into the same directory. Windows paths inside the sh shim need forward slashes.
- **Shell pane.** `session_start_shell` resolves `$SHELL`. On Windows fall back to `pwsh.exe`, then `powershell.exe`, then `ComSpec`.
- **Codex resume-key capture.** `open_rollout_paths_for_pid` is `lsof` on macOS and returns `Unsupported` elsewhere, and the caller already falls back to the marker scan. Degrades gracefully; leave it.
- **Headless fork.** `spawn.rs` sets `process_group(0)` and kills the group; on Windows `child.kill()` on its own is enough because nothing else is in the group.

### 3. MCP transport

Three places assume a Unix domain socket: the app listener in `mcp/mod.rs` and `server.rs`, the `runner-mcp` stdio proxy in `cli/src/mcp.rs`, and the settings page that shows the socket path. The rmcp transport only needs `AsyncRead` + `AsyncWrite` halves, so the fix is a small `ipc` module with a cfg-split `IpcListener`/`IpcStream`: Unix socket as today, and on Windows a named pipe at `\\.\pipe\com.wycstudios.runner` through `tokio::net::windows::named_pipe`. Named pipes are per-user by default ACL and need no port. `ops/mcp.rs`'s `mode(0o600)` on the written config goes behind `cfg(unix)`; Windows relies on the `USERPROFILE` ACL.

### 4. App shell

- **Keymap.** GPUI parses `cmd` as the platform modifier, which is the Windows key on Windows. Every `cmd-*` binding in `keymap.rs` would bind to Win+key. Runner needs a per-platform default keymap. The Windows Terminal convention is the safe answer: app chords on Ctrl (Ctrl+N, Ctrl+W, Ctrl+K, Ctrl+,), terminal copy and paste on Ctrl+Shift+C / Ctrl+Shift+V, and Ctrl+C always reaching the PTY. The `⌘` glyph in shortcut hints becomes `Ctrl+`.
- **Window chrome.** The window opens with `appears_transparent` and Runner draws its own 44 px title bar around the traffic lights. On Windows GPUI can do client-side decorations, but for the nightly the standard OS title bar is fine and removes a whole class of first-run bugs. Revisit once it runs.
- **Reveal and open.** `mission_workspace` shells out to `open -R` and `file_links` to `/usr/bin/open`. Windows: `explorer /select,<path>` and `cmd /c start "" <path>`.
- **Icon.** `install_app_icon` is AppKit. Windows wants an `.ico` compiled in as a resource (`gpui-ce`'s `windows-manifest` feature plus a build script). Cosmetic; the nightly can ship without.
- **Already fine.** Fonts are bundled through `add_fonts`; the updater is compiled out; image paste and file-URL paste are already no-ops off macOS; the wake observer is macOS-only and resume-after-sleep simply will not fire.
- **Unknowns to test on the PC.** IME for CJK in the terminal (`terminal_ime.rs`), DPI scaling across monitors, and ConPTY's resize repaint.

### 5. Pipeline — what "a friend can download directly" needs

The current `nightly` release is a **draft**: invisible on the releases page and fetchable only with `gh release download` by someone with repo access. A friend cannot download it. The Windows nightly must be a **published pre-release** on its own rolling tag, `nightly-win`, so the macOS draft policy is untouched.

- `nightly.yml` gains a `windows-latest` job behind a `platform` dispatch input: `cargo build --release -p runner-app -p runner-cli` for `x86_64-pc-windows-msvc`, stage `Runner.exe`, `runner-agent-cli.exe`, and `runner-mcp.exe` side by side (`cli_install` looks next to the running exe), zip as `Runner-Nightly-<version>.<stamp>-x64.zip`, keep the last ten.
- The release step mirrors the macOS one with the draft flag inverted: `gh release create nightly-win --target nightly-windows --prerelease` on first run, `gh release edit nightly-win --prerelease --draft=false` after, `gh release upload … --clobber`. Because the tag is public, its verify step asserts the opposite of the macOS check: `isPrerelease` is true, `isDraft` is false, and an unauthenticated `curl --head` of `https://github.com/yicheng47/runner/releases/download/nightly-win/<zip>` succeeds. The friend's download link is that URL, or the releases page where pre-releases are listed with the badge.
- The `nightly` skill's check and stamp steps get a `windows` variant that reads `nightly-win` instead.
- The build identity step calls `script/bundle-mac --print-version`; factor the version read into something the Windows job can run (`cargo metadata` is enough).
- Unsigned. SmartScreen will show "Windows protected your PC"; the release notes say "More info → Run anyway". Signing (Azure Trusted Signing or an OV certificate) is a production decision, not a nightly blocker.
- `ci.yaml` gains a `windows-latest` job running `cargo check` and `cargo clippy` for the workspace, **not** a required check at first. Fourteen test files spawn `/bin/sh`-style commands; they get `cfg(unix)` gates rather than Windows equivalents, and `cargo test` on Windows runs whatever is left.
- `release.yml` is untouched until Windows leaves the nightly channel.

## Decisions

- **Native spawn, never WSL.** Native is what a Windows user expects and what the friend will have installed; Claude Code's own Windows support requires Git Bash, which also makes the sh shim work. WSL would keep every unix assumption but move the product onto a second machine the user has to set up.
- **Job Objects replace signal escalation rather than emulating it.** A job kills the whole tree in one call, so the snapshot-then-sweep design that exists to catch descendants on macOS has no Windows counterpart to port. The lifecycle code gets a platform trait with two implementations, not one implementation with two branches.
- **Named pipe, not TCP.** It is the platform's equivalent of the Unix socket, per-user by default, and needs no port or token file. rmcp does not care which.
- **Windows Terminal keymap conventions.** Ctrl for app chords, Ctrl+Shift for terminal copy and paste, Ctrl+C untouched. Anything else surprises the target user.
- **Portable zip, standard title bar, no icon.** Every item that is cosmetic or an installer concern stays out until someone other than Jason has run the nightly.
- **Separate public pre-release tag.** `nightly-win` is a published pre-release, never a draft — confirmed by Jason 2026-09-04; renamed from the planned `nightly-windows` tag on 2026-09-05 to avoid colliding with the long-lived branch of that name. `nightly` stays a draft; changing the macOS nightly's visibility is a different decision and not needed for this one.

## Phases

Each phase is one PR that leaves the macOS build, tests, and the `Rust / macOS` required check green.

0. **Compile gate.** Add the non-required `windows-latest` check job to CI and make it green: the `ipc` module with the named-pipe implementation, the `home_dir` and app-data helpers, `pty_runtime` un-gated with a Windows lifecycle module that compiles (stubs are acceptable here), `cfg(unix)` on the shell-spawning tests. Deliverable: `Runner.exe` builds. About one session.
1. **It opens on the PC.** The `nightly-windows` job and its first zip. Jason runs it: the window opens with the OS title bar, the database is created under `%APPDATA%`, Settings renders, the Ctrl keymap works, the shell pane opens PowerShell. Fix what breaks. This is already something the friend can download. One to two sessions plus PC time.
2. **Agents run.** Job Object lifecycle, liveness and identity, orphan sweep, `PATHEXT` discovery, both `runner` shims, `;` `PATH`, the MCP pipe end to end from a Claude Code session. Acceptance: a claude direct chat, a codex direct chat, then a two-slot crew mission with `runner signal` reaching the feed and Stop leaving no orphans. The real port. Three to five sessions, most of it PC testing.
3. **Nightly polish.** `explorer /select`, icon resource, long-path manifest, ConPTY resize fixtures, a CJK IME pass, the Windows CI job promoted to required, release notes with the SmartScreen step. Signing and an installer are filed as their own issue if the nightly earns them.

## Non-goals

- A production Windows release: installer (MSI/NSIS), code signing, an updater, or a Windows entry in `release.yml`.
- Spawning agents through WSL.
- ARM64 Windows. x64 only until someone asks.
- Linux, which stays a non-goal in `docs/product/vision.md`.
- Windows-specific UI beyond the keymap and title bar; the design canvas is unchanged.

## Housekeeping

- Reopen #437 so this spec keeps its number.
- `docs/product/vision.md` §6 and §8 still say "Windows is targeted for 0.7.0". The commit that recorded the won't-do (`c49b575`, 2026-08-27) never landed on `main`; it exists only in the reflog. Update both sections to this nightly-only scope in Phase 0.
- `crates/runner-backend/Cargo.toml` comments still describe a tmux fallback on Windows that no longer exists; fix when the gate moves.
