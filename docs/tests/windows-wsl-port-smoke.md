# Windows (WSL) Port — Smoke Test

Release-readiness checklist for the Windows fork, where agents run **inside
WSL** rather than as native host processes (`session::wsl`). The shared
cross-platform behavior is already covered by the workspace tests and the
[v0 MVP plan](archive/v0-mvp-tests.md); this document only exercises what the
Windows port changes or adds, plus the runtime behavior that compilation
cannot prove.

Two lanes:

- **Static / build checks** — run before touching the app; catch type,
  lint, and cross-target compile regressions.
- **Windows runtime smoke** — must be done on a real Windows machine with a
  WSL distro installed. This is the lane that matters: none of it is proven
  by `cargo check`.

Check a box only when the **Expected** result is observed. If a step fails,
the **Anchor** points at the code to look at first.

---

## Static / build checks

Run from the repo root in WSL.

```sh
pnpm exec tsc --noEmit
pnpm run lint
cargo fmt --all --check
cargo test --workspace          # or: make test-rust
```

Windows cross-target compile (via the Windows MSVC toolchain — confirms the
`#[cfg(windows)]` code in `session::wsl`, `job.rs`, and the MCP stubs builds):

```sh
cmd.exe /c "cd C:\path\to\runner-windows\src-tauri && cargo check --lib --target x86_64-pc-windows-msvc"
```

- [ ] All four WSL-side checks pass.
- [ ] Windows `cargo check` finishes with no **errors** (dead-code warnings for
      Unix-only paths such as `shell_path.rs` are expected on this target).

Clippy with `-D warnings` is the upstream CI gate; run it if `cargo-clippy` is
installed (`rustup component add clippy`):

```sh
cargo clippy --workspace --all-targets -- -D warnings
```

---

## Prerequisites for the runtime lane

Because the app is built **without** the Tauri CLI's dev server (the
`custom-protocol` feature is on by default so `cargo build` embeds the
frontend — see `src-tauri/Cargo.toml`), prepare three artifacts, then build a
standalone exe.

1. **Frontend bundle** — `pnpm build` (writes `dist/`).
2. **Windows host CLIs** in `src-tauri/binaries/` (staged by the Tauri build's
   `externalBin`): `runner-agent-cli-x86_64-pc-windows-msvc.exe` and
   `runner-mcp-x86_64-pc-windows-msvc.exe`.
3. **Linux agent CLI** at `src-tauri/binaries/runner-agent-cli-linux-x86_64`
   — this is the ELF `include_bytes!`'d by `session::wsl::install` and pushed
   into the distro. Rebuild it in WSL whenever `cli/` changes:

   ```sh
   cargo build -p runner-cli --release
   cp target/release/runner-agent-cli src-tauri/binaries/runner-agent-cli-linux-x86_64
   ```

   > Stale-binary trap: `include_bytes!` bakes this file in at compile time. If
   > you change the CLI but skip this copy, the distro runs the **old** agent.

Build + launch (from Windows, or via `cmd.exe` from WSL):

```sh
cmd.exe /c "cd C:\path\to\runner-windows\src-tauri && cargo build --target x86_64-pc-windows-msvc"
# then run the exe:
cmd.exe /c "C:\path\to\runner-windows\src-tauri\target\x86_64-pc-windows-msvc\debug\runner.exe"
```

Clean-state reset (optional) — debug builds write to the `-dev` sibling dir,
so a packaged install is untouched:

```
%APPDATA%\com.wycstudios.runner-dev
```

---

## Windows runtime smoke

### A. Launch & localization

- [ ] **Window appears on launch.** The app shows a fully-rendered window (no
      indefinitely-blank/hidden window). *Anchor:* `commands::app::app_ready`
      shows the calling window; `tauri.conf.json` window `visible`.
- [ ] **Language toggle is live.** Settings → switch 中文 ⇄ English updates the
      UI immediately, no restart; the choice survives a relaunch. *Anchor:*
      `src/lib/i18n.tsx` (`LangProvider`, `app.lang` in localStorage).
- [ ] **Default language is Chinese** on first run (fresh `-dev` dir).

### B. WSL runner + mission end-to-end

- [ ] **Create a WSL runner** (execution target = WSL, the default). The
      command field locks to the runtime default (in-distro `claude`/`codex`)
      and is read-only. *Anchor:* `CreateRunnerModal` execution-target control.
- [ ] **Start a mission** with that runner. The PTY attaches and streams the
      agent's output; typing reaches the agent. *Anchor:*
      `session::wsl::wsl_command_shaper`, `session::pty_runtime`.
- [ ] **Event flow crosses the WSL boundary.** Agent activity (signals /
      messages) appears in the mission feed — confirms the agent inside the
      distro writes the NDJSON event log at a path the host watcher reads.
      *Anchor:* `session::wsl::path` mapping; `event_bus` watcher.
- [ ] **CJK IME caret is positioned at the prompt.** Switch to a Chinese IME
      and type into the terminal — the candidate window appears **at the
      cursor**, not pinned to the screen's top-left corner. *Anchor:*
      `RunnerTerminal.tsx` `syncImeCaret`.
- [ ] **No gaming-overlay popup.** Launch does **not** trigger the Windows
      `ms-gamingoverlay` "You'll need a new app" dialog. *Anchor:*
      `RunnerTerminal.tsx` `USE_WEBGL = false`.

### C. Native (host) runner

- [ ] **Create a native runner.** Execution target = Windows/native; the
      command field becomes editable (e.g. `powershell`). *Anchor:*
      `CreateRunnerModal` native branch; `pty_runtime::native_command_shaper`.
- [ ] **It runs on the host, not in WSL.** Start a session — the process is a
      Windows process (e.g. `powershell.exe` in Task Manager), no `wsl.exe`
      relay for this session.

### D. Multi-window

- [ ] **Open a second window** (Ctrl+N / File → New Window / sidebar "在新窗口
      打开"). A new window opens on the same backend. *Anchor:*
      `commands::window::window_open`; `windows.rs`.
- [ ] **Duplicate-subject overlay (localized).** Navigate both windows to the
      same mission → the non-primary window shows the "已在另一个窗口打开"
      overlay; its terminal is read-only. *Anchor:* `DuplicateSubjectOverlay`.
- [ ] **"切到那个窗口" works.** The button focuses the primary window.
      *Anchor:* `api.window.focusOther` → `window_focus_other`.
- [ ] **Primary promotion on close.** With two windows on one mission, close
      the primary — the survivor becomes primary and mounts the PTY. *Anchor:*
      `WindowEvent::Destroyed` → `unregister` + `broadcast_focus_map`.

### E. Lifecycle & cleanup (Windows-specific)

- [ ] **Main window close exits the app.** Closing `main` on Windows quits
      (no macOS-style hide-to-Dock, which would strand an invisible process).
      *Anchor:* `lib.rs` `CloseRequested` arm, `#[cfg(not(target_os = "macos"))]`.
- [ ] **No leaked relays after quit.** After a clean quit, Task Manager /
      `tasklist | findstr wsl` shows no orphaned `wsl.exe` relay from the
      session. *Anchor:* `ExitRequested` → `stop_running_sessions_on_quit`.
- [ ] **Job Object backstop.** Force-kill `runner.exe` (End Task) mid-session —
      the associated `wsl.exe` relay dies too, not left running. *Anchor:*
      `session::wsl::job` (`KILL_ON_JOB_CLOSE`).

### F. Path mapping

- [ ] **Windows working dir maps into the distro.** Set a runner's working dir
      to a Windows path (`C:\Users\...`); the agent starts in the corresponding
      `/mnt/c/Users/...` inside WSL. *Anchor:* `session::wsl::path`.

---

## Known gaps — out of scope (do not file as smoke failures)

These are intentionally deferred (see the milestone notes in code). A tester
hitting them is confirming a known limitation, not a regression.

- **MCP server disabled on the Windows host** — `mcp/mod.rs` `start` is a
  no-op; Settings → MCP integration renders but does nothing. Core mission
  coordination is unaffected (it flows through the NDJSON event log, not MCP).
  (M4+)
- **Distro is hardcoded to `"Ubuntu"`** — `lib.rs`; no detection, no Settings
  picker. A differently-named distro won't be used. (M3)
- **No graceful self-exit detection for WSL relays** — cleanup relies on the
  kill path + Job Object backstop. (M2+)
