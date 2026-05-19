# 18 — App logging + crash reporting

> Tracking issue: [#138](https://github.com/yicheng47/runner/issues/138)

## Motivation

Runner today has zero app-side logging or crash reporting.

- `src-tauri/src/main.rs` is `fn main() { runner_lib::run() }`. No
  log facade init, no panic hook, no `tauri-plugin-log`, no Sentry.
- All diagnostics across `session/manager.rs`, the router, the
  reattach paths, etc. are `eprintln!` to stderr.
- On macOS, when the bundle is launched from Finder/Dock (the actual
  user path), stderr is silently dropped. The only way to see it is
  to run `/Applications/Runner.app/Contents/MacOS/runner` from a
  terminal — fine for developers, useless for any normal user.
- Default `panic::set_hook` is in effect, which prints to stderr (so:
  also dropped on Finder launches) and then aborts / unwinds. No
  panic body or backtrace lands anywhere persistent.
- The only post-crash artifact today is the OS-level CrashReporter
  `.ips` file in `~/Library/Logs/DiagnosticReports/`. That gives a
  stack trace at the crash site but no app context (what mission was
  resuming, what session id was being reattached, what tmux command
  failed). Most "the app froze" reports get diagnosed from those
  `eprintln!` lines — which the user can't see.

Concretely, every user-reported crash today is unrecoverable: we
have no log to ask the user to attach. The auto-update toast, the
reattach path, the codex_capture watcher, the router's stdin-inject
branch — all of them have informative `eprintln!`s that vanish in
production.

This is also a prereq for several other open specs:

- **#14 (human notifications)** — debugging suppression decisions
  needs persistent logs.
- **#13 (PTY silence idle)** — verifying the forwarder's transition
  emissions in the wild needs a log we can ask the user to share.
- **#10 (mission session persistence)** — the reattach branches
  (`Ok(None) | Err(_)`, mount-failed, etc.) emit `eprintln!`s today;
  without logs we can't tell what path was taken on a given user's
  machine.

This is foundational infra, not a user-facing feature. Filing as P1
because the next user-reported crash is unrecoverable without it.

## Scope

### In scope (v1)

- **`tauri-plugin-log` integration.** Add the plugin, configure a
  file target to `~/Library/Logs/com.wycstudios.runner/runner.log`
  on macOS (the plugin's `LogDir` default lands there). Rotation:
  ~10MB per file, keep last 3 files. Stdout target stays enabled in
  debug builds; disabled in release (so the dev terminal experience
  is unchanged).
- **Convert `eprintln!` to `log::` macros.** Every existing
  `eprintln!` in `src-tauri/src/` becomes one of:
  - `log::error!` for failures (forwarder errors, runtime failures,
    DB roundtrips that lost data).
  - `log::warn!` for degraded paths (mount-failed reattach,
    resume-failed wipe, orphan-stop fallback).
  - `log::info!` for lifecycle (app start, mission_attach, session
    reattach decisions).
  - `log::debug!` for chatty internals (forwarder byte counts,
    poll loops). Off by default in release.
  The plugin's filter respects `RUST_LOG` so devs can crank it up
  without rebuilding.
- **Panic hook that writes to the same log.** Register
  `std::panic::set_hook` *before* `tauri::Builder::default()`. The
  hook formats `panic_info` + `std::backtrace::Backtrace::force_capture()`
  into a single `log::error!` call so the panic body and the
  backtrace end up in the log file. Then chains to the default
  hook so the existing abort/unwind behavior is preserved.
- **"Reveal logs in Finder" menu item.** Under `Help`. Opens the
  log directory in Finder via `tauri-plugin-opener`. Same affordance
  Claude Desktop and VS Code surface — when a user files a bug,
  "click Help → Reveal Logs, drag the file in" is the entire ask.
- **Log dir path is discoverable.** Add a `Reveal logs` button on
  the existing About / Settings panel too, so users who don't think
  to check the Help menu can still find it. Both routes call the
  same command.
- **Sensible startup banner.** The first line on every app start
  logs the version, the resolved `app_data_dir`, the OS / arch, and
  the tmux version (best-effort `tmux -V`). Triage-from-log starts
  with these.

### Out of scope (deferred)

- **Remote crash reporting (Sentry, BugSnag).** The local log file
  is enough for the "ask the user to send it" flow we have today.
  Wiring a remote sink raises privacy questions (event log contents
  include agent output and user prompts) and isn't worth the
  complexity until we have hundreds of users. If we ever turn it
  on, it must be opt-in with a clear preview.
- **In-app log viewer.** Other apps (Linear, Slack) have a "view
  recent logs" panel inside the app. Useful but lots of UI for a
  rare need; v1 ships "Reveal in Finder" only.
- **Structured log envelope (JSON).** `tauri-plugin-log` supports
  custom formatters; v1 uses the plugin's default text format
  (`[YYYY-MM-DDThh:mm:ss][level][target] message`) which greps
  cleanly. JSON makes machine ingestion easier; revisit if we ever
  wire a remote sink.
- **Per-mission log files.** A separate file per mission would be
  symmetric with the per-mission NDJSON event log, but the event
  log already carries the audit-trail load. App-level logs are about
  Runner's own behavior — one global file is the right unit.
- **Sanitization / scrubbing.** Logs may contain absolute paths
  (cwd, event-log path) and tmux session names. Not credentials —
  Runner doesn't touch agent API keys. v1 logs paths verbatim; a
  later scrubber pass can normalize them if we ever ship logs
  somewhere users don't trust.

### Key decisions

1. **`tauri-plugin-log`, not a hand-rolled writer.** Rotation,
   target multiplexing, level filtering, and the `RUST_LOG` env
   integration are already solved by the plugin. Tauri's own
   docs treat it as the default; Quill uses it too. No reason to
   reinvent.
2. **Default level `Info` in release, `Debug` in debug builds.**
   `Info` produces a few lines per lifecycle event (start,
   mission_attach, reattach decision, resume, exit) — enough to
   reconstruct user actions from a bug report, not enough to blow
   up disk. Devs can crank to `Debug` via `RUST_LOG=runner=debug`.
3. **Panic hook chains to the default, doesn't replace it.** The
   goal is `panic body → log file` BEFORE the existing abort path
   runs. Replacing the default hook outright would suppress the
   stderr line that the OS CrashReporter currently keys on; chain
   instead. (`backtrace::Backtrace::force_capture()` works even
   when `RUST_BACKTRACE` is unset — important because users almost
   never set that env var.)
4. **Help menu is the canonical surface.** "Reveal logs in Finder"
   in the Help menu matches every other macOS dev tool. Settings
   page button is a secondary convenience. About dialog is too
   buried to be the only entry point.
5. **No remote anything in v1.** Opt-in remote reporting is its own
   feature with its own privacy review. Don't slip it into the
   logging spec.

## Implementation phases

### Phase 1 — plugin + log facade

- Add `tauri-plugin-log = "2"` to `src-tauri/Cargo.toml`.
- Init in `src-tauri/src/lib.rs::run`:
  ```rust
  tauri::Builder::default()
      .plugin(tauri_plugin_log::Builder::new()
          .targets([
              Target::new(TargetKind::LogDir { file_name: Some("runner".into()) }),
              #[cfg(debug_assertions)]
              Target::new(TargetKind::Stdout),
          ])
          .level(if cfg!(debug_assertions) {
              log::LevelFilter::Debug
          } else {
              log::LevelFilter::Info
          })
          .max_file_size(10 * 1024 * 1024)
          .rotation_strategy(RotationStrategy::KeepN(3))
          .build())
      // existing plugins
  ```
- Confirm `LogDir` resolves to `~/Library/Logs/com.wycstudios.runner/`
  on a fresh macOS build (matches `tauri.conf.json:identifier`).

### Phase 2 — convert `eprintln!`

- Add `log = "0.4"` to `src-tauri/Cargo.toml`.
- Sweep `src-tauri/src/`:
  - `eprintln!("runner: …error…")` → `log::error!("…")` with the
    `runner:` prefix dropped (the target field already disambiguates).
  - `eprintln!("runner: …degraded…")` → `log::warn!("…")`.
  - Lifecycle prints (app start, mount, reattach decision) →
    `log::info!`.
  - Forwarder byte counts, poll-loop chatter → `log::debug!` (gated
    so release builds don't churn).
- The `mark_session_stopped` / `reattach` paths are the highest-
  signal lines today; promote them to `log::warn!` deliberately so
  they always survive the default filter.

### Phase 3 — panic hook

- New `src-tauri/src/panic_hook.rs`:
  ```rust
  pub fn install() {
      let prev = std::panic::take_hook();
      std::panic::set_hook(Box::new(move |info| {
          let bt = std::backtrace::Backtrace::force_capture();
          log::error!("panic: {info}\n{bt}");
          prev(info);
      }));
  }
  ```
- Call `panic_hook::install()` as the first line of
  `runner_lib::run()` — *before* the Tauri builder, so even a panic
  during plugin init lands in the file (the file target initializes
  lazily on first write).

### Phase 4 — Help menu + Settings button

- Add a `runner_logs_reveal` Tauri command in
  `src-tauri/src/commands/app.rs` (or wherever the app-shell
  commands live) that resolves the log dir and opens it via
  `tauri-plugin-opener`.
- Wire it into the Help menu in the Tauri menu config (Linux/macOS
  parity: Help → "Reveal logs in Finder").
- Add a `Reveal logs` row to the Settings page under "Diagnostics".
- Both call the same command. No fancy UI — just open the dir.

### Phase 5 — verification

- Manual smoke:
  1. Fresh release build. Start app from Finder. Confirm
     `~/Library/Logs/com.wycstudios.runner/runner.log` exists and
     has the startup banner.
  2. Start a mission. Confirm `info` lines log spawn, mount, attach.
  3. Kill a mission session via tmux directly. Confirm the reattach
     path's `warn!` lines land in the file.
  4. Force a panic (debug build: `RUST_LOG=runner=debug
     RUNNER_PANIC_TEST=1` and a debug-only thread that panics
     after 5s). Confirm the panic body + backtrace lands in the
     log BEFORE the OS CrashReporter `.ips`. Both should exist.
  5. Help → "Reveal logs in Finder" opens the right directory.
  6. Run the app for long enough to roll past 10MB (or temporarily
     drop the rotation threshold). Confirm three files survive
     with `runner.log`, `runner.log.1`, `runner.log.2` (or however
     the plugin names rotated files).
- Backend tests:
  - Smoke test that `panic_hook::install` doesn't break the test
    harness (test threads panic intentionally).
  - Unit test that the log dir resolution matches the bundle
    identifier (catches an identifier rename silently breaking the
    path).

## Verification

- [ ] `tauri-plugin-log` initialized with a file target at the
      OS-conventional log path.
- [ ] Every existing `eprintln!` in `src-tauri/src/` converted to a
      `log::` macro at the right level; no plain `eprintln!`
      remaining in production paths.
- [ ] Panic hook installed before the Tauri builder; panic bodies
      land in the log file with a backtrace.
- [ ] Default level: `Info` in release, `Debug` in debug builds.
      `RUST_LOG` override respected.
- [ ] Help → "Reveal logs in Finder" opens the log directory.
- [ ] Settings → Diagnostics has a "Reveal logs" button that does
      the same.
- [ ] Startup banner logs version, app_data_dir, OS, arch, tmux
      version.
- [ ] Log file rotation works: ~10MB per file, last 3 kept.
- [ ] `cargo test --workspace` and `pnpm exec tsc --noEmit` clean.
- [ ] No agent-protocol changes; no schema changes.
