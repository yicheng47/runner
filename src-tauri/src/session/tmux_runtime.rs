#![allow(dead_code)] // Wired into manager.rs in Step 9; foundation now.

//! TmuxRuntime — implements `SessionRuntime` over tmux primitives.
//! Steps 5+6+7 of docs/impls/0004-tmux-session-runtime.md.
//!
//! Layout: this file owns the trait implementation; the foundation
//! (binary discovery, config writer, tmux_cmd helper) lives in
//! `session::tmux`. Pure helpers (argv builders, name validators)
//! sit at the top of this module and have unit tests; methods that
//! actually shell out are exercised by gated integration tests
//! that require a local tmux binary.
//!
//! Unix-only by construction. The Windows path is the future
//! native-pty runtime; `session/mod.rs` cfg-gates this module so
//! Windows builds don't trip on the FIFO/libc pieces.

use std::io::{Read, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::io::AsRawFd;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{mpsc, Arc};
use std::thread;
use std::time::{Duration, Instant};

use super::launch;
use super::runtime::{
    OutputStream, RunnerStatus, RuntimeError, RuntimeOutput, RuntimeResult, RuntimeSession,
    SessionRuntime, SessionStatus, SpawnSpec,
};
use super::tmux::{
    resolve_tmux_binary, tmux_cmd, write_runner_config, CONFIG_VERSION, DEFAULT_LABEL,
};

/// Tmux-backed session runtime. Constructed once per app process
/// from `app_data_dir`; clones are cheap (it's just paths and a
/// label).
#[derive(Debug, Clone)]
pub struct TmuxRuntime {
    tmux_bin: PathBuf,
    label: String,
    config_path: PathBuf,
    /// Parent dir for per-session subdirs (`<runtime_dir>/<session_id>/`).
    /// Each subdir holds `launch.sh` and `output.fifo`.
    runtime_dir: PathBuf,
    home: Option<PathBuf>,
}

impl TmuxRuntime {
    /// Resolve the tmux binary, write the Runner-managed config, and
    /// allocate the per-app runtime directory under
    /// `<app_data>/sessions/`. Idempotent: safe to call on every
    /// app start.
    pub fn new(app_data_dir: &Path) -> RuntimeResult<Self> {
        let tmux_bin = resolve_tmux_binary()?;
        let config_path = write_runner_config(app_data_dir)?;
        let runtime_dir = app_data_dir.join("sessions");
        std::fs::create_dir_all(&runtime_dir)?;
        let home = std::env::var_os("HOME").map(PathBuf::from);
        Ok(Self {
            tmux_bin,
            label: DEFAULT_LABEL.to_string(),
            config_path,
            runtime_dir,
            home,
        })
    }

    /// Pre-bound `tmux` Command with `-L <label> -f <config>`. Use
    /// this for every invocation so global flags can't be forgotten.
    fn cmd(&self) -> Command {
        tmux_cmd(&self.tmux_bin, &self.label, &self.config_path)
    }

    fn session_dir(&self, session_id: &str) -> PathBuf {
        self.runtime_dir.join(session_id)
    }

    /// App-start config reconciliation (Step 8 of the plan). When
    /// `exit-empty off` keeps the tmux server alive across Runner
    /// upgrades, an old server is still running with the previous
    /// config's options at next launch. `-f <runner.conf>` is
    /// only re-read on server start; running options stay stale
    /// unless we reload them explicitly.
    ///
    /// Two-step probe so the missing-stamp legacy-server case is
    /// handled correctly (the previous shape lumped "no server"
    /// and "server up but stamp missing" into a single Ok(false)
    /// branch, leaving legacy servers permanently stale):
    ///
    /// 1. `list-sessions` — is the tmux server up at all? Non-
    ///    zero = no server, no-op (next spawn boots a fresh one
    ///    with the current config).
    /// 2. `show-options -g -v @runner_config_version` — read the
    ///    stamp. Empty stdout (or non-zero on a tmux that errors
    ///    on missing user-options) means the running server is
    ///    pre-stamp legacy. Treat as stale and reload.
    ///
    /// Returns `Ok(true)` when a `source-file` reload happened,
    /// `Ok(false)` when no work was needed.
    pub fn reconcile_config(&self) -> RuntimeResult<bool> {
        // Step 1: is the server running?
        let server_up = self
            .cmd()
            .arg("list-sessions")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !server_up.success() {
            return Ok(false);
        }

        // Step 2: read the stamp. Treat both `success + empty
        // stdout` and `non-zero exit` as missing — both mean the
        // running server doesn't carry our @runner_config_version
        // user-option (legacy server, or the option was unset).
        let probe = self
            .cmd()
            .arg("show-options")
            .arg("-g")
            .arg("-v")
            .arg("@runner_config_version")
            .stdin(Stdio::null())
            .stderr(Stdio::null())
            .output()?;
        let stamped = if probe.status.success() {
            String::from_utf8_lossy(&probe.stdout).trim().to_string()
        } else {
            String::new()
        };
        if stamped == CONFIG_VERSION {
            return Ok(false);
        }
        // Stale — reload the config against the running server.
        // The config itself sets `@runner_config_version`, so a
        // successful source-file is also the re-stamp.
        run_tmux_check(
            self.cmd().arg("source-file").arg(&self.config_path),
            "source-file",
        )?;
        Ok(true)
    }
}

// ──────────────────────────────────────────────────────────────────
// Pure helpers — testable without a tmux binary.
// ──────────────────────────────────────────────────────────────────

/// Validate `session_id` against tmux's target-name rules. tmux
/// treats `:` as a session/window separator, `.` as a window/pane
/// separator, and `;` as a command terminator inside `send-keys`,
/// so anything outside `[A-Za-z0-9_-]` poisons targeting.
pub fn validate_session_id(id: &str) -> RuntimeResult<()> {
    if id.is_empty() || id.len() > 64 {
        return Err(RuntimeError::Msg(format!(
            "session id {id:?} must be 1-64 chars"
        )));
    }
    for c in id.chars() {
        if !(c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(RuntimeError::Msg(format!(
                "session id {id:?} must match [A-Za-z0-9_-]+ (tmux uses : . ; as separators)"
            )));
        }
    }
    Ok(())
}

/// Validate a tmux key name (`Enter`, `C-c`, `Up`, etc.). Permits
/// the same character set as session ids — matches what tmux key-
/// name lookup accepts in practice. Conservative: prefers
/// rejecting an unfamiliar name to letting an injection through.
pub fn validate_key_name(key: &str) -> RuntimeResult<()> {
    if key.is_empty() || key.len() > 32 {
        return Err(RuntimeError::Msg(format!(
            "key name {key:?} must be 1-32 chars"
        )));
    }
    for c in key.chars() {
        if !(c.is_ascii_alphanumeric() || c == '_' || c == '-') {
            return Err(RuntimeError::Msg(format!(
                "key name {key:?} must match [A-Za-z0-9_-]+"
            )));
        }
    }
    Ok(())
}

/// Deterministic session name. Always `runner-<session_id>`; the
/// runtime never invents an opaque tmux id.
pub fn session_name(session_id: &str) -> String {
    format!("runner-{session_id}")
}

/// Exact-match target form (`=name`) for **session/window names
/// only**. tmux's bare `-t name` does prefix matching against
/// session/window names — `runner-1` would collide with `runner-10`
/// — so this helper enforces the `=` prefix at the boundary so
/// individual call sites can never get it wrong.
///
/// **Pane ids are not session names.** Pane ids start with `%`
/// (e.g. `%0`, `%17`) and are globally unique by construction; the
/// prefix-matching footgun doesn't apply to them, and tmux rejects
/// `=%0` as "can't find pane". For pane targets, pass the raw pane
/// id directly: `cmd.arg(&session.pane)` not `cmd.arg(target(&session.pane))`.
pub fn target(name: &str) -> String {
    format!("={name}")
}

/// Window-scoped target (`=session:window`). For `resize-window`,
/// `list-panes -t=…:main`, etc.
pub fn window_target(session: &str, window: &str) -> String {
    format!("={session}:{window}")
}

/// Parse one row of `list-panes -F '#{pane_id} #{pane_dead}
/// #{pane_dead_status} #{pane_pid} #{pane_current_command}'`.
/// Returns `(pane_id, SessionStatus)` so the caller can match by
/// pane id. Returns `None` for rows that don't fit the expected
/// shape — defensive against tmux format changes; the manager
/// treats that as "no info this tick" and re-polls.
///
/// Whitespace splitting handles missing fields the way the format
/// actually emits them: `pane_dead_status` is empty (no preceding
/// space gap) when the pane is alive, and `pane_current_command`
/// is the whole tail. We require pane_id, pane_dead, and pane_pid
/// at minimum; everything else is optional.
pub fn parse_pane_status_line(line: &str) -> Option<(String, SessionStatus)> {
    let trimmed = line.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut parts = trimmed.split_whitespace();
    let pane_id = parts.next()?.to_string();
    let dead_raw = parts.next()?;
    let alive = match dead_raw {
        "0" => true,
        "1" => false,
        _ => return None,
    };
    // `pane_dead_status` is empty (no token) when the pane is
    // alive. With `split_whitespace` empty fields collapse —
    // `0  12345 sh` becomes ["0", "12345", "sh"], skipping the
    // missing dead-status. So we need to peek: the next token is
    // either a numeric dead-status (when dead=1) or the pid (when
    // alive). Disambiguate by looking at `alive`.
    let exit_code: Option<i32>;
    let pid_raw: Option<&str>;
    if alive {
        // Skip dead-status (it'll be empty/missing); next token is pid.
        exit_code = None;
        pid_raw = parts.next();
    } else {
        let next = parts.next();
        exit_code = next.and_then(|s| s.parse::<i32>().ok());
        pid_raw = parts.next();
    }
    // pid is the one tail field we treat as required — every
    // working tmux pane has a pane_pid. Reject the row otherwise
    // so callers see "no info" rather than half-populated noise.
    let pid_str = pid_raw?;
    let pid = pid_str.parse::<i32>().ok();
    pid?;
    let command_tail: Vec<&str> = parts.collect();
    let command = if command_tail.is_empty() {
        None
    } else {
        Some(command_tail.join(" "))
    };
    Some((
        pane_id,
        SessionStatus {
            alive,
            exit_code,
            pid,
            command,
        },
    ))
}

// ──────────────────────────────────────────────────────────────────
// SessionRuntime impl.
// ──────────────────────────────────────────────────────────────────

impl SessionRuntime for TmuxRuntime {
    fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
        validate_session_id(&spec.session_id)?;
        let sess_name = session_name(&spec.session_id);
        let sess_dir = self.session_dir(&spec.session_id);
        std::fs::create_dir_all(&sess_dir)?;

        // 1. Compose PATH + render launch script. Manager-provided
        //    shim_dir / bundled_bin_dir / shell_path are threaded
        //    in here; the runtime layer doesn't read DB or env-var
        //    state directly.
        let process_path = std::env::var("PATH").ok();
        let composed = launch::compose_path(
            spec.shim_dir.as_deref(),
            spec.bundled_bin_dir.as_deref(),
            spec.shell_path.as_deref(),
            self.home.as_deref(),
            process_path.as_deref(),
        );
        // Inject COLUMNS/LINES so Node-based TUIs (claude-code, ink)
        // pick up the initial grid before SIGWINCH lands. The pane
        // also gets resized to `initial_size` after new-session via
        // `resize-window`.
        let mut env: std::collections::BTreeMap<String, String> = spec
            .env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        if let Some((cols, rows)) = spec.initial_size {
            env.insert("COLUMNS".into(), cols.to_string());
            env.insert("LINES".into(), rows.to_string());
        }
        let script = launch::LaunchScript {
            command: spec.command.clone(),
            args: spec.args.clone(),
            cwd: spec.cwd.clone(),
            env,
            path: composed,
        };
        let launch_path = launch::write_launch_script(&sess_dir, &script)?;

        // 2. mkfifo + open reader O_RDONLY|O_NONBLOCK. Non-blocking
        //    avoids the open-side wait for a writer; later the
        //    forwarder uses poll() to wait for readability with a
        //    timeout, so we never spurious-EOF on a transient
        //    no-writer condition.
        let fifo_path = sess_dir.join("output.fifo");
        ensure_fifo(&fifo_path)?;
        let reader = open_fifo_reader(&fifo_path)?;

        // 3. Pre-spawn: kill any stale session left over from a
        //    crashed prior process.
        let _ = self
            .cmd()
            .arg("kill-session")
            .arg("-t")
            .arg(target(&sess_name))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        // 4. tmux new-session, chained with pipe-pane install AND
        //    a display-message pane-id readout. tmux processes
        //    chained commands sequentially in a single server
        //    event-loop iteration, so pipe-pane attaches
        //    microseconds after new-session forks the launch
        //    script — far smaller than the IPC roundtrip we'd see
        //    if we ran them as separate `tmux` invocations. This
        //    minimizes the pre-install race where a fast-startup
        //    agent could write to its PTY before pipe-pane
        //    forwards. Real agents (claude-code, codex) take
        //    100ms+ to start; chaining shrinks the race below
        //    their startup latency.
        let pipe_cmd = format!(
            "cat >> {}",
            launch::shell_quote(&fifo_path.display().to_string())
        );
        let window_tgt = window_target(&sess_name, "main");
        let mut new_session = self.cmd();
        new_session
            .arg("new-session")
            .arg("-d")
            .arg("-s")
            .arg(&sess_name)
            .arg("-n")
            .arg("main");
        if let Some(cwd) = &spec.cwd {
            new_session.arg("-c").arg(cwd);
        }
        new_session
            .arg("--")
            .arg(launch::shell_quote(&launch_path.display().to_string()));
        // \; separator between chained commands. tmux's argv
        // parser treats a bare `;` arg as the command separator.
        new_session
            .arg(";")
            .arg("pipe-pane")
            .arg("-O")
            .arg("-t")
            .arg(&window_tgt)
            .arg(&pipe_cmd);
        new_session
            .arg(";")
            .arg("display-message")
            .arg("-p")
            .arg("-t")
            .arg(&window_tgt)
            .arg("#{pane_id}");
        let output = new_session.output()?;
        if !output.status.success() {
            return Err(RuntimeError::TmuxFailed {
                command: "new-session+pipe-pane".into(),
                status: output.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&output.stderr).to_string(),
            });
        }
        let pane_id = String::from_utf8_lossy(&output.stdout).trim().to_string();
        if pane_id.is_empty() {
            return Err(RuntimeError::Msg(
                "tmux new-session returned empty pane id".into(),
            ));
        }

        let session = RuntimeSession {
            runtime: "tmux".into(),
            socket: self.label.clone(),
            session_name: sess_name,
            window: "main".into(),
            pane: pane_id,
        };

        // 4b–7. Resize + pipe-pane + capture-pane + channel.
        //       Wrap in a closure so any error after new-session
        //       triggers kill-session below — including a failed
        //       resize-window. Without resize being inside the
        //       cleanup block, a transient resize failure left the
        //       new tmux session alive with no `RuntimeSession`
        //       tracked anywhere for reconciliation.
        let setup = || -> RuntimeResult<OutputStream> {
            // 4b. Apply initial pane size if requested. The agent's
            //     COLUMNS/LINES env was set to match in step 1; this
            //     resize tells tmux about it.
            if let Some((cols, rows)) = spec.initial_size {
                run_tmux_check(
                    self.cmd()
                        .arg("resize-window")
                        .arg("-t")
                        .arg(window_target(&session.session_name, &session.window))
                        .arg("-x")
                        .arg(cols.to_string())
                        .arg("-y")
                        .arg(rows.to_string()),
                    "resize-window",
                )?;
            }
            attach_streaming(
                &self.cmd(),
                &session,
                &fifo_path,
                reader.try_clone()?,
                /*is_fresh_spawn=*/ true,
            )
        };
        match setup() {
            Ok(stream) => {
                drop(reader);
                Ok((session, stream))
            }
            Err(e) => {
                let _ = self
                    .cmd()
                    .arg("kill-session")
                    .arg("-t")
                    .arg(target(&session.session_name))
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
                Err(e)
            }
        }
    }

    fn resume(&self, session: &RuntimeSession) -> RuntimeResult<OutputStream> {
        // Confirm the session is still alive before doing anything
        // expensive. has-session prints nothing on success and exits
        // 1 if the target is missing.
        let status = self
            .cmd()
            .arg("has-session")
            .arg("-t")
            .arg(target(&session.session_name))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !status.success() {
            return Err(RuntimeError::Msg(format!(
                "tmux session {} not found (server lost it)",
                session.session_name
            )));
        }

        // Reattach: rebuild the FIFO + forwarder around the
        // existing pane, then install pipe-pane → capture-pane →
        // channel via the same `attach_streaming` helper as spawn
        // so the gap-free attach order is shared.
        let session_id = session
            .session_name
            .strip_prefix("runner-")
            .ok_or_else(|| {
                RuntimeError::Msg(format!(
                    "session name {:?} doesn't match runner-<id> shape",
                    session.session_name
                ))
            })?;
        let sess_dir = self.session_dir(session_id);
        std::fs::create_dir_all(&sess_dir)?;
        let fifo_path = sess_dir.join("output.fifo");
        ensure_fifo(&fifo_path)?;
        let reader = open_fifo_reader(&fifo_path)?;
        // Resume does NOT kill the session on attach failure —
        // unlike spawn we don't own the session. Reconciliation
        // (Step 8) handles a permanently broken pane separately.
        attach_streaming(
            &self.cmd(),
            session,
            &fifo_path,
            reader,
            /*is_fresh_spawn=*/ false,
        )
    }

    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
        // Try to kill, then verify with `has-session`. Discarding
        // kill-session's failure isn't safe: if tmux itself is
        // wedged (socket gone, daemon dead), we'd otherwise
        // return Ok and let the manager's forwarder reconcile
        // the row to "stopped" while the agent stays alive in
        // tmux with no UI/DB record to reach it.
        //
        // kill-session on a missing target ALSO returns non-zero
        // (with a "no such session" stderr) — that's a benign
        // success for our purposes, which is why we don't fail
        // on the kill-session exit code itself. The has-session
        // probe disambiguates: if the session is gone (probe
        // fails), we did our job; if the session is still there,
        // the kill genuinely didn't take and we should surface
        // that as an error.
        let _ = self
            .cmd()
            .arg("kill-session")
            .arg("-t")
            .arg(target(&session.session_name))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
        let probe = self
            .cmd()
            .arg("has-session")
            .arg("-t")
            .arg(target(&session.session_name))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if probe.success() {
            // Session still present after kill-session — tmux is
            // unwilling or unable to reap it. The manager's
            // caller (typically `kill()`) propagates this so the
            // DB row isn't flipped to stopped on a still-live
            // pane.
            return Err(RuntimeError::Msg(format!(
                "tmux refused to stop session {} (has-session still finds it)",
                session.session_name
            )));
        }
        Ok(())
    }

    fn paste(&self, session: &RuntimeSession, payload: &[u8]) -> RuntimeResult<()> {
        // Multi-line prompt paste:
        //   1. load-buffer -b <name> -  ← payload over stdin (verbatim).
        //   2. paste-buffer -p -r -d -b <name> -t=<pane>
        //        -p bracketed paste so the agent sees a paste event
        //        -r keep LF literal (default would translate LF→CR,
        //           making each newline a submit)
        //        -d delete the buffer after paste (no leak)
        //
        // Strip a single trailing newline from the payload. The
        // manager submits with a separate send_key("Enter"); leaving
        // the newline in would render as an extra blank line in the
        // paste before the submit.
        let mut trimmed = payload;
        if trimmed.ends_with(b"\n") {
            trimmed = &trimmed[..trimmed.len() - 1];
        }
        let buffer_name = format!("runner-{}", session.session_name);

        let mut load = self.cmd();
        load.arg("load-buffer")
            .arg("-b")
            .arg(&buffer_name)
            .arg("-")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped());
        let mut child = load.spawn()?;
        if let Some(mut stdin) = child.stdin.take() {
            stdin.write_all(trimmed)?;
        }
        let load_out = child.wait_with_output()?;
        if !load_out.status.success() {
            return Err(RuntimeError::TmuxFailed {
                command: "load-buffer".into(),
                status: load_out.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&load_out.stderr).to_string(),
            });
        }

        run_tmux_check(
            self.cmd()
                .arg("paste-buffer")
                .arg("-p")
                .arg("-r")
                .arg("-d")
                .arg("-b")
                .arg(&buffer_name)
                .arg("-t")
                .arg(&session.pane),
            "paste-buffer",
        )?;
        Ok(())
    }

    fn send_bytes(&self, session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()> {
        // Literal byte stream from xterm.js passthrough. tmux's
        // send-keys -l takes UTF-8 string args, not raw bytes, so
        // anything that's not valid UTF-8 is rejected at the
        // boundary — xterm.js only emits valid UTF-8 anyway.
        let s = std::str::from_utf8(bytes).map_err(|e| {
            RuntimeError::Msg(format!("send_bytes payload is not valid UTF-8: {e}"))
        })?;
        run_tmux_check(
            self.cmd()
                .arg("send-keys")
                .arg("-t")
                .arg(&session.pane)
                .arg("-l")
                .arg("--")
                .arg(s),
            "send-keys -l",
        )?;
        Ok(())
    }

    fn send_key(&self, session: &RuntimeSession, key: &str) -> RuntimeResult<()> {
        validate_key_name(key)?;
        run_tmux_check(
            self.cmd()
                .arg("send-keys")
                .arg("-t")
                .arg(&session.pane)
                .arg(key),
            "send-keys <key>",
        )?;
        Ok(())
    }

    fn resize(&self, session: &RuntimeSession, cols: u16, rows: u16) -> RuntimeResult<()> {
        run_tmux_check(
            self.cmd()
                .arg("resize-window")
                .arg("-t")
                .arg(window_target(&session.session_name, &session.window))
                .arg("-x")
                .arg(cols.to_string())
                .arg("-y")
                .arg(rows.to_string()),
            "resize-window",
        )?;
        Ok(())
    }

    fn capture_visible(&self, session: &RuntimeSession) -> RuntimeResult<Vec<u8>> {
        capture_visible_region(&self.cmd(), session)
    }

    fn status(&self, session: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
        // First gate: has-session. tmux exits non-zero if the
        // target session is gone — that's our terminal-unavailable
        // signal. We could fold this into list-panes' error
        // handling, but the explicit probe gives the manager a
        // clean Ok(None) without parsing tmux's stderr.
        let probe = self
            .cmd()
            .arg("has-session")
            .arg("-t")
            .arg(target(&session.session_name))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()?;
        if !probe.success() {
            return Ok(None);
        }

        // list-panes -s targets the whole session (vs. the bare
        // -t which interprets the target as a window). With
        // `remain-on-exit on`, dead panes stay around with
        // pane_dead=1 + pane_dead_status; without it those
        // fields are blank. The format keeps fields space-
        // separated so a dead pane with `pane_current_command`
        // empty still parses.
        let out = self
            .cmd()
            .arg("list-panes")
            .arg("-s")
            .arg("-t")
            .arg(target(&session.session_name))
            .arg("-F")
            .arg("#{pane_id} #{pane_dead} #{pane_dead_status} #{pane_pid} #{pane_current_command}")
            .output()?;
        if !out.status.success() {
            return Err(RuntimeError::TmuxFailed {
                command: "list-panes".into(),
                status: out.status.code().unwrap_or(-1),
                stderr: String::from_utf8_lossy(&out.stderr).to_string(),
            });
        }
        let stdout = String::from_utf8_lossy(&out.stdout);
        // Find the row matching our pane_id; one pane per session
        // today, but mission spawns may grow more, and we always
        // want the row tied to this RuntimeSession.
        for line in stdout.lines() {
            if let Some(parsed) = parse_pane_status_line(line) {
                if parsed.0 == session.pane {
                    return Ok(Some(parsed.1));
                }
            }
        }
        // Session exists but our pane id no longer does — treat
        // as terminal-unavailable so the manager marks the
        // session stopped without inventing exit info.
        Ok(None)
    }
}

// ──────────────────────────────────────────────────────────────────
// Internal helpers.
// ──────────────────────────────────────────────────────────────────

/// `mkfifo` if missing. tmux pipe-pane will block its `cat` writer
/// until our reader-side opens, so this is safe to leave around
/// across reattaches.
fn ensure_fifo(path: &Path) -> RuntimeResult<()> {
    if path.exists() {
        return Ok(());
    }
    let cstr = std::ffi::CString::new(path.as_os_str().as_encoded_bytes())
        .map_err(|e| RuntimeError::Msg(format!("fifo path contains NUL: {e}")))?;
    // SAFETY: cstr lives for the call; mode 0600 (owner-only) is
    // strict enough for per-app use.
    let rc = unsafe { libc::mkfifo(cstr.as_ptr(), 0o600) };
    if rc != 0 {
        return Err(RuntimeError::Io(std::io::Error::last_os_error()));
    }
    Ok(())
}

/// Open a FIFO `O_RDONLY|O_NONBLOCK`. The non-blocking flag is
/// load-bearing twice over:
///
/// 1. **Non-blocking open.** Plain `O_RDONLY` blocks until a writer
///    attaches (default POSIX FIFO semantics); we open before tmux
///    has even spawned `pipe-pane`, so we'd deadlock.
/// 2. **No spurious EOF.** A blocking O_RDONLY reader sees `read()`
///    return 0 (EOF) the moment there are zero writers — including
///    the ms-window before tmux's `cat` opens the write end. With
///    `O_NONBLOCK`, that case is `EAGAIN` instead, so the
///    forwarder keeps polling rather than exiting prematurely.
///
/// We previously tried O_RDWR (kernel sees a writer = us), but
/// then a manager-side detach left the forwarder thread blocked
/// in `read()` forever because EOF was unreachable. Non-blocking +
/// `poll()` with a timeout is the cleaner shape.
fn open_fifo_reader(path: &Path) -> RuntimeResult<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)?;
    Ok(file)
}

/// Read everything currently in the FIFO without blocking. Returns
/// the accumulated bytes and stops on the first `EAGAIN` (no more
/// data right now). The reader fd must already be `O_NONBLOCK`.
/// Used during attach to drain whatever pipe-pane has buffered
/// between the install and the snapshot capture, so those bytes
/// flush as `Stream` events after `Replay` rather than getting
/// silently dropped.
fn drain_fifo_nonblocking(reader: &mut std::fs::File) -> Vec<u8> {
    let mut out = Vec::new();
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break, // No writer + non-blocking ⇒ done.
            Ok(n) => out.extend_from_slice(&buf[..n]),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => break,
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
    out
}

/// Wire pipe-pane → capture-pane → channel for an existing tmux
/// pane. Shared between `spawn` and `resume` so the gap-free
/// attach order can't drift between the two.
///
/// The order is the v1 attach sequence from
/// docs/impls/0004-tmux-session-runtime.md (Step 6):
///
/// 1. Install `pipe-pane` so the pane's PTY output starts flowing
///    into the FIFO immediately. Bytes that arrive between this
///    and step 3 land in the FIFO buffer (kernel-managed).
/// 2. **Reattach only**: snapshot via `capture-pane` (with
///    alternate-screen branching) so the new xterm.js render
///    starts with the existing pane state. Skipped on fresh
///    spawn — there's nothing meaningful to replay (the agent
///    just started), and including the snapshot writes the
///    pane's empty top rows into xterm.js's buffer ahead of any
///    live output, leaving a big blank region above the agent's
///    actual content.
/// 3. Drain the FIFO buffer non-blockingly into a `Vec<u8>`.
///    Picks up any bytes that arrived between pipe-pane install
///    and now.
/// 4. Send `Replay(snapshot)` (when non-empty), then
///    `Stream(buffered)` (when non-empty). xterm.js applies
///    Replay as a snapshot reset and Stream as appends.
/// 5. Spawn the forwarder thread for ongoing live bytes.
fn attach_streaming(
    cmd: &Command,
    session: &RuntimeSession,
    fifo_path: &Path,
    mut reader: std::fs::File,
    is_fresh_spawn: bool,
) -> RuntimeResult<OutputStream> {
    if !is_fresh_spawn {
        // Reattach: install pipe-pane (close stale + install
        // new). For fresh spawn the chained new-session+pipe-pane
        // in `TmuxRuntime::spawn` already installed the pipe at
        // server-tick latency, so calling install_pipe_pane here
        // would just churn it unnecessarily.
        install_pipe_pane(cmd, session, fifo_path)?;
    }

    // Snapshot strategy:
    // - **Fresh spawn**: capture the *visible region only* (no
    //   `-S/-E` for full scrollback) and trim leading blank
    //   lines. This recovers any bytes the agent produced
    //   before pipe-pane attached (rare for real agents, real
    //   for fast shell commands), without dragging in the
    //   pane's empty top-rows from the agent's bottom-anchored
    //   layout. Trim is leading-only — trailing blanks may be
    //   intentional spacing in the agent's banner.
    // - **Reattach**: full alt-screen-aware capture so the
    //   resumed render sees the existing pane state (including
    //   scrollback for main-screen panes). No trimming —
    //   long-running agents produce real layouts that
    //   shouldn't be edited.
    let snapshot = if is_fresh_spawn {
        let raw = capture_visible_region(cmd, session)?;
        trim_leading_blank_lines(raw)
    } else {
        capture_replay_bytes(cmd, session)?
    };
    let buffered = drain_fifo_nonblocking(&mut reader);

    let (tx, rx) = mpsc::channel::<RuntimeOutput>();
    let stop = Arc::new(AtomicBool::new(false));

    if !snapshot.is_empty() {
        let _ = tx.send(RuntimeOutput::Replay(snapshot));
    }
    if !buffered.is_empty() {
        let _ = tx.send(RuntimeOutput::Stream(buffered));
    }

    let forwarder_stop = Arc::clone(&stop);
    thread::spawn(move || forward_fifo(reader, tx, forwarder_stop));

    Ok(OutputStream::new(rx, stop))
}

/// PTY-silence threshold for the busy→idle transition (issue #124).
/// Spec 13 settled on 750ms global; per-template tuning is deferred.
/// Combined with the 200ms `poll()` timeout below, the worst-case
/// busy→idle latency is ~950ms — well inside the spec's ~1s SLO.
const IDLE_THRESHOLD: Duration = Duration::from_millis(750);

/// Debounce window for the idle→busy edge. Bytes must sustain at
/// least this long after a quiet stretch before the detector emits
/// `Busy`. Filters out short bursts that are not "the agent is
/// actually doing something" — the xterm SIGWINCH dance on panel
/// switch (one repaint chunk + silence), focus-induced cursor
/// blinks, and similar one-shot terminal artifacts. A real
/// streaming TUI sustains output for seconds, so the debounce is
/// imperceptible in practice; the trade-off is an at-most
/// `BUSY_DEBOUNCE` lag on legitimate wake-ups, well inside the
/// human perception threshold.
const BUSY_DEBOUNCE: Duration = Duration::from_millis(200);

/// Per-session busy/idle state machine driven by PTY-byte activity
/// (issue #124). The forwarder owns one and feeds it
/// `on_bytes(n)` after every successful FIFO read and `tick()`
/// every poll iteration. The detector returns `Some(state)` only
/// at transition boundaries; latched runs return `None` so the
/// caller doesn't emit redundant events.
///
/// Initial state is `Idle` — the forwarder thread starts when the
/// runtime attaches, and no bytes have flowed yet. The first byte
/// after a quiet stretch starts a `busy_pending_since` timer; the
/// detector flips `Busy` only once activity has sustained for
/// `BUSY_DEBOUNCE`. Sustained silence past `IDLE_THRESHOLD` flips
/// back to `Idle`.
struct IdleDetector {
    /// Wall-clock at the last successful byte read. `None` until
    /// the first byte arrives so `tick()` doesn't try to flip a
    /// pane that has produced no output yet.
    last_byte: Option<Instant>,
    current: RunnerStatus,
    threshold: Duration,
    /// Set on the first byte after `Idle`; held until either the
    /// debounce elapses (→ flip `Busy`) or silence resumes (→ drop
    /// without flipping). `None` while `current == Busy` or while
    /// no bytes have arrived since the last idle stretch.
    busy_pending_since: Option<Instant>,
    busy_debounce: Duration,
}

impl IdleDetector {
    fn new() -> Self {
        Self {
            last_byte: None,
            current: RunnerStatus::Idle,
            threshold: IDLE_THRESHOLD,
            busy_pending_since: None,
            busy_debounce: BUSY_DEBOUNCE,
        }
    }

    /// Called after a successful `read()` produced `n > 0` bytes.
    /// Returns `Some(Busy)` only when activity has sustained for at
    /// least `busy_debounce` since the first byte after the last
    /// idle stretch — short bursts (SIGWINCH dance, cursor blink,
    /// focus events) are dropped without flipping state.
    fn on_bytes(&mut self, n: usize) -> Option<RunnerStatus> {
        if n == 0 {
            return None;
        }
        let now = Instant::now();
        self.last_byte = Some(now);
        if self.current == RunnerStatus::Busy {
            return None;
        }
        let pending_since = *self.busy_pending_since.get_or_insert(now);
        if now.duration_since(pending_since) >= self.busy_debounce {
            self.busy_pending_since = None;
            self.current = RunnerStatus::Busy;
            Some(RunnerStatus::Busy)
        } else {
            None
        }
    }

    /// Called from the forwarder's poll loop (every ~200ms).
    /// Drops a stale `busy_pending_since` when silence has resumed
    /// past the debounce window (the burst didn't sustain) and
    /// emits `Some(Idle)` exactly on the Busy→Idle edge when
    /// elapsed silence exceeds `threshold`.
    fn tick(&mut self) -> Option<RunnerStatus> {
        // Burst-that-didn't-sustain: if there's a pending wake but
        // bytes haven't arrived in a debounce window, the activity
        // wasn't real — drop the pending timer so the next quiet
        // stretch isn't biased by it.
        if let (Some(_pending_t), Some(last_t)) = (self.busy_pending_since, self.last_byte) {
            if last_t.elapsed() >= self.busy_debounce {
                self.busy_pending_since = None;
            }
        }
        if self.current != RunnerStatus::Busy {
            return None;
        }
        // Busy without a `last_byte` shouldn't happen — `on_bytes`
        // is the only path that sets Busy — but defending against
        // it keeps the detector honest if a future caller flips
        // state manually.
        let last = self.last_byte?;
        if last.elapsed() < self.threshold {
            return None;
        }
        self.current = RunnerStatus::Idle;
        Some(RunnerStatus::Idle)
    }
}

/// Forwarder loop. Uses `poll()` with a 200ms timeout so the
/// thread wakes regularly to check `stop` (set when the
/// `OutputStream` receiver is dropped) and can exit even when no
/// bytes are flowing through the FIFO. Reads are non-blocking
/// because the fd was opened `O_NONBLOCK`; `poll(POLLIN)` ensures
/// data is actually ready before each read.
///
/// The same 200ms tick doubles as the busy→idle check window
/// (issue #124): `IdleDetector::tick()` runs every iteration so
/// the worst-case wake-to-idle latency is `IDLE_THRESHOLD + 200ms`.
fn forward_fifo(mut reader: std::fs::File, tx: mpsc::Sender<RuntimeOutput>, stop: Arc<AtomicBool>) {
    let raw_fd = reader.as_raw_fd();
    let mut buf = [0u8; 8192];
    let mut detector = IdleDetector::new();
    loop {
        if stop.load(Ordering::Relaxed) {
            break;
        }
        let mut pfd = libc::pollfd {
            fd: raw_fd,
            events: libc::POLLIN,
            revents: 0,
        };
        // SAFETY: pfd is a single valid struct on the stack; nfds
        // = 1 matches. Timeout is in milliseconds.
        let rc = unsafe { libc::poll(&mut pfd, 1, 200) };
        if rc < 0 {
            // EINTR or other transient. Loop and re-check stop.
            // Idle tick still runs so a long EINTR storm doesn't
            // prevent the busy→idle flip.
            if let Some(state) = detector.tick() {
                if emit_status(&tx, state).is_err() {
                    break;
                }
            }
            continue;
        }
        if rc == 0 {
            // Timeout. Loop and re-check stop, but tick first so a
            // silent stretch flips to idle without needing another
            // byte to arrive.
            if let Some(state) = detector.tick() {
                if emit_status(&tx, state).is_err() {
                    break;
                }
            }
            continue;
        }
        let revents = pfd.revents;
        if revents & libc::POLLIN != 0 {
            match reader.read(&mut buf) {
                Ok(0) => {
                    // Non-blocking O_RDONLY shouldn't see EOF
                    // unless every writer has closed AND POLLIN
                    // fired against an empty FIFO — meaning the
                    // pipe-pane writer closed for good.
                    break;
                }
                Ok(n) => {
                    if tx.send(RuntimeOutput::Stream(buf[..n].to_vec())).is_err() {
                        break; // Receiver dropped — also caught by `stop`.
                    }
                    if let Some(state) = detector.on_bytes(n) {
                        if emit_status(&tx, state).is_err() {
                            break;
                        }
                    }
                }
                Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => continue,
                Err(_) => break,
            }
        } else if revents & (libc::POLLHUP | libc::POLLERR | libc::POLLNVAL) != 0 {
            // POLLHUP without POLLIN means the writer closed and
            // the FIFO is drained. Real EOF — exit.
            break;
        }
        // Even on a POLLIN iteration, check for idle: a single
        // burst followed by silence still needs the tick to fire.
        if let Some(state) = detector.tick() {
            if emit_status(&tx, state).is_err() {
                break;
            }
        }
    }
}

/// Helper so the four `tick`/`on_bytes` call sites in `forward_fifo`
/// don't each reach for the same `RuntimeOutput::StatusTransition`
/// constructor. `source` is always `"forwarder"` here — the CLI's
/// `runner status` verb takes a different path into the log and
/// stamps `source: "agent"` itself.
fn emit_status(
    tx: &mpsc::Sender<RuntimeOutput>,
    state: RunnerStatus,
) -> Result<(), mpsc::SendError<RuntimeOutput>> {
    tx.send(RuntimeOutput::StatusTransition {
        state,
        source: "forwarder",
    })
}

/// Probe alternate-screen state and run the right capture-pane
/// shape. Returns the raw snapshot bytes; the caller decides when
/// to send them as a `Replay` event over the channel — the gap-
/// free attach order in `attach_streaming` requires sending
/// `Replay` _after_ the FIFO drain, so the helper has to stay
/// channel-agnostic.
/// Capture only the pane's currently-visible region (no `-S/-E`
/// scrollback). Used at fresh spawn to recover any bytes the
/// agent produced before pipe-pane attached, without including
/// the (empty) main-screen scrollback. Always preserves SGR
/// escapes (`-e`) so xterm.js sees colors.
fn capture_visible_region(cmd: &Command, session: &RuntimeSession) -> RuntimeResult<Vec<u8>> {
    let out = clone_cmd(cmd)
        .arg("capture-pane")
        .arg("-p")
        .arg("-e")
        .arg("-t")
        .arg(&session.pane)
        .output()?;
    if !out.status.success() {
        return Err(RuntimeError::TmuxFailed {
            command: "capture-pane".into(),
            status: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        });
    }
    Ok(out.stdout)
}

/// Drop leading blank lines from a capture-pane snapshot. A line
/// is "blank" if it contains only ASCII whitespace once SGR / CSI
/// escape sequences are stripped — handles tmux's `-e` output
/// where rows might have `\e[40m  \e[0m` without any visible
/// content. Trailing blank lines are preserved (agents
/// occasionally use them as intentional spacing in banners).
fn trim_leading_blank_lines(snapshot: Vec<u8>) -> Vec<u8> {
    if snapshot.is_empty() {
        return snapshot;
    }
    let mut line_start = 0usize;
    let mut i = 0usize;
    while i < snapshot.len() {
        if snapshot[i] == b'\n' {
            let line = &snapshot[line_start..i];
            if !is_visually_blank(line) {
                return snapshot[line_start..].to_vec();
            }
            line_start = i + 1;
        }
        i += 1;
    }
    // Trailing line without a `\n` — only meaningful if non-blank.
    if line_start < snapshot.len() {
        let line = &snapshot[line_start..];
        if !is_visually_blank(line) {
            return snapshot[line_start..].to_vec();
        }
    }
    Vec::new()
}

/// True if `line` contains only whitespace once CSI escape
/// sequences (ESC `[` … final byte 0x40-0x7e) are stripped. Tmux's
/// `capture-pane -e` emits SGR escapes around colored content;
/// this lets us treat a row of "background-only" cells as blank.
fn is_visually_blank(line: &[u8]) -> bool {
    let mut i = 0;
    while i < line.len() {
        let b = line[i];
        if b == 0x1b {
            // Skip CSI escape: ESC [ <params> <final 0x40-0x7e>
            i += 1;
            if i < line.len() && line[i] == b'[' {
                i += 1;
                while i < line.len() && !(0x40..=0x7e).contains(&line[i]) {
                    i += 1;
                }
                if i < line.len() {
                    i += 1; // consume final byte
                }
            }
            continue;
        }
        if b != b' ' && b != b'\t' && b != b'\r' {
            return false;
        }
        i += 1;
    }
    true
}

fn capture_replay_bytes(cmd: &Command, session: &RuntimeSession) -> RuntimeResult<Vec<u8>> {
    let alt_on = is_alternate_on(cmd, &session.pane)?;
    let mut capture = clone_cmd(cmd);
    capture
        .arg("capture-pane")
        .arg("-p")
        .arg("-e")
        .arg("-t")
        .arg(&session.pane);
    if !alt_on {
        // Main screen: include full scrollback. Alternate has no
        // scrollback, so this branch only makes sense pre-TUI.
        capture.arg("-S").arg("-").arg("-E").arg("-");
    }
    let out = capture.output()?;
    if !out.status.success() {
        return Err(RuntimeError::TmuxFailed {
            command: "capture-pane".into(),
            status: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        });
    }
    Ok(out.stdout)
}

/// `display-message -p -t=<pane> '#{alternate_on}'` returns
/// `0`/`1` + newline. Defaults to `false` on parse failure (the
/// caller will use the main-screen capture, which is the more
/// useful default).
fn is_alternate_on(cmd: &Command, pane: &str) -> RuntimeResult<bool> {
    let out = clone_cmd(cmd)
        .arg("display-message")
        .arg("-p")
        .arg("-t")
        .arg(pane)
        .arg("#{alternate_on}")
        .output()?;
    if !out.status.success() {
        return Err(RuntimeError::TmuxFailed {
            command: "display-message".into(),
            status: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        });
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim() == "1")
}

/// Step 6's two-step reattach-safe pipe-pane install: close any
/// existing pipe (stale FIFO from a crashed prior process), then
/// install the new one *without* `-o`. tmux replaces the existing
/// pipe atomically when called this way.
fn install_pipe_pane(cmd: &Command, session: &RuntimeSession, fifo: &Path) -> RuntimeResult<()> {
    // Close.
    let _ = clone_cmd(cmd)
        .arg("pipe-pane")
        .arg("-t")
        .arg(&session.pane)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    // Install.
    let shell_cmd = format!(
        "cat >> {}",
        launch::shell_quote(&fifo.display().to_string())
    );
    run_tmux_check(
        clone_cmd(cmd)
            .arg("pipe-pane")
            .arg("-O")
            .arg("-t")
            .arg(&session.pane)
            .arg(shell_cmd),
        "pipe-pane",
    )?;
    Ok(())
}

/// Run a tmux command and propagate failures as `TmuxFailed`. The
/// caller already configured the args; this wrapper just unifies
/// exit-status checking.
fn run_tmux_check(cmd: &mut Command, name: &str) -> RuntimeResult<()> {
    let out = cmd.output()?;
    if !out.status.success() {
        return Err(RuntimeError::TmuxFailed {
            command: name.into(),
            status: out.status.code().unwrap_or(-1),
            stderr: String::from_utf8_lossy(&out.stderr).to_string(),
        });
    }
    Ok(())
}

/// Std `Command` doesn't impl `Clone`, but every helper here wants
/// to start from the pre-bound `tmux -L … -f …` shape. Roll our
/// own clone by re-running `tmux_cmd` shape from the program /
/// arg list of the source command. Internal — only used to fan out
/// helper calls from a single `cmd()` source.
fn clone_cmd(src: &Command) -> Command {
    let mut out = Command::new(src.get_program());
    out.args(src.get_args());
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validate_session_id_accepts_valid_ulids() {
        for ok in ["01HX1", "abc-123", "A_B_C", "0", "x"] {
            assert!(validate_session_id(ok).is_ok(), "{ok:?} should be valid");
        }
    }

    #[test]
    fn validate_session_id_rejects_tmux_metas() {
        for bad in [
            "",
            "a:b",
            "a.b",
            "a;b",
            "a b",
            "a/b",
            "üni",
            &"x".repeat(65),
        ] {
            assert!(
                validate_session_id(bad).is_err(),
                "{bad:?} should be invalid"
            );
        }
    }

    #[test]
    fn validate_key_name_accepts_common_keys() {
        for ok in ["Enter", "C-c", "Up", "Down", "F1", "Tab", "BSpace", "M-x"] {
            assert!(validate_key_name(ok).is_ok(), "{ok:?} should be valid");
        }
    }

    #[test]
    fn validate_key_name_rejects_metas() {
        for bad in ["", "a;b", "a.b", "a:b", " Enter", &"x".repeat(33)] {
            assert!(validate_key_name(bad).is_err(), "{bad:?} should be invalid");
        }
    }

    #[test]
    fn session_name_is_runner_prefix() {
        assert_eq!(session_name("01HX1"), "runner-01HX1");
        assert_eq!(session_name("abc"), "runner-abc");
    }

    #[test]
    fn target_uses_exact_match_form() {
        assert_eq!(target("runner-01HX1"), "=runner-01HX1");
        // Window-scoped form for resize-window etc.
        assert_eq!(window_target("runner-01HX1", "main"), "=runner-01HX1:main");
    }

    #[test]
    fn trim_leading_blank_lines_strips_padding() {
        // Typical fresh-spawn capture: agent painted at the
        // bottom of its allocated screen, pane has empty rows
        // above. Leading blanks (including ones containing only
        // SGR escape sequences) should be stripped.
        let input = b"\n\n   \n\x1b[40m  \x1b[0m\nClaude Code v2.1.133\nbody\n";
        let out = trim_leading_blank_lines(input.to_vec());
        let s = String::from_utf8_lossy(&out);
        assert!(s.starts_with("Claude Code"), "got = {s:?}");
        assert!(s.contains("body"), "trailing content preserved: {s:?}");
    }

    #[test]
    fn trim_leading_blank_lines_preserves_trailing_blanks() {
        // Trailing blank lines may be intentional spacing in the
        // banner; don't touch them.
        let input = b"banner\n\n\n";
        let out = trim_leading_blank_lines(input.to_vec());
        assert_eq!(out, b"banner\n\n\n");
    }

    #[test]
    fn trim_leading_blank_lines_returns_empty_when_all_blank() {
        let input = b"\n   \n\x1b[40m\x1b[0m\n";
        let out = trim_leading_blank_lines(input.to_vec());
        assert!(out.is_empty(), "expected empty, got {:?}", out);
    }

    #[test]
    fn trim_leading_blank_lines_no_op_when_first_line_has_content() {
        let input = b"hello\n\nworld\n";
        let out = trim_leading_blank_lines(input.to_vec());
        assert_eq!(out, input);
    }

    #[test]
    fn is_visually_blank_handles_csi_escapes() {
        assert!(is_visually_blank(b""));
        assert!(is_visually_blank(b"   "));
        assert!(is_visually_blank(b"\t \r"));
        assert!(is_visually_blank(b"\x1b[40m  \x1b[0m"));
        assert!(is_visually_blank(b"\x1b[2J"));
        assert!(!is_visually_blank(b" hi "));
        assert!(!is_visually_blank(b"\x1b[31mhello\x1b[0m"));
    }

    /// Build a detector with tiny debounce + threshold so the
    /// state machine can be exercised without sleeping for the
    /// real production constants. Tests that care specifically
    /// about the debounce/idle constants override accordingly.
    fn detector_with_threshold(threshold: Duration) -> IdleDetector {
        let mut d = IdleDetector::new();
        d.threshold = threshold;
        // Tests pre-debounce assumed the first byte flips Busy
        // immediately. Keep that contract by reducing debounce to
        // zero unless a test asks otherwise.
        d.busy_debounce = Duration::from_millis(0);
        d
    }

    /// Helper for tests that need the production-style debounce
    /// path but a short threshold for the busy→idle assertion.
    fn detector_with_debounce_and_threshold(
        debounce: Duration,
        threshold: Duration,
    ) -> IdleDetector {
        let mut d = IdleDetector::new();
        d.threshold = threshold;
        d.busy_debounce = debounce;
        d
    }

    #[test]
    fn idle_detector_first_byte_flips_busy_with_zero_debounce() {
        // With debounce=0, the first byte flips Busy immediately —
        // mirrors the pre-debounce behavior the rest of the suite
        // historically relied on.
        let mut d = detector_with_threshold(Duration::from_millis(1));
        assert_eq!(d.on_bytes(1), Some(RunnerStatus::Busy));
        assert_eq!(d.on_bytes(64), None);
    }

    #[test]
    fn idle_detector_zero_byte_read_is_noop() {
        // `read(0)` is the FIFO-EOF signal in `forward_fifo`; the
        // detector should not treat it as activity.
        let mut d = IdleDetector::new();
        assert_eq!(d.on_bytes(0), None);
        assert_eq!(d.current, RunnerStatus::Idle);
    }

    #[test]
    fn idle_detector_tick_below_threshold_returns_none() {
        let mut d = detector_with_threshold(Duration::from_millis(50));
        assert_eq!(d.on_bytes(1), Some(RunnerStatus::Busy));
        // Same tick window — no elapsed silence yet.
        assert_eq!(d.tick(), None);
    }

    #[test]
    fn idle_detector_tick_past_threshold_flips_idle() {
        // 1ms threshold + 5ms sleep is enough margin on every CI
        // host we run on; the spec's 750ms is *only* about giving
        // the agent's punctuation pauses room to breathe.
        let mut d = detector_with_threshold(Duration::from_millis(1));
        assert_eq!(d.on_bytes(1), Some(RunnerStatus::Busy));
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(d.tick(), Some(RunnerStatus::Idle));
        // Subsequent ticks while already Idle don't re-emit.
        assert_eq!(d.tick(), None);
    }

    #[test]
    fn idle_detector_byte_after_idle_flips_busy() {
        let mut d = detector_with_threshold(Duration::from_millis(1));
        assert_eq!(d.on_bytes(1), Some(RunnerStatus::Busy));
        std::thread::sleep(Duration::from_millis(5));
        assert_eq!(d.tick(), Some(RunnerStatus::Idle));
        // Next byte after the idle flip is the wake edge.
        assert_eq!(d.on_bytes(1), Some(RunnerStatus::Busy));
    }

    #[test]
    fn idle_detector_initial_tick_is_noop() {
        // Detector starts Idle. `tick()` before any bytes have
        // flowed should not emit — there's nothing to flip from.
        let mut d = IdleDetector::new();
        assert_eq!(d.tick(), None);
    }

    #[test]
    fn idle_detector_short_burst_does_not_flip_busy() {
        // A single chunk of bytes followed by silence (the SIGWINCH
        // dance, a focus-induced cursor blink, etc.) must NOT flip
        // the detector: activity didn't sustain past the debounce
        // window.
        let mut d = detector_with_debounce_and_threshold(
            Duration::from_millis(30),
            Duration::from_millis(1),
        );
        assert_eq!(d.on_bytes(64), None);
        assert_eq!(d.current, RunnerStatus::Idle);
        // No more bytes; pending wake should age out on tick.
        std::thread::sleep(Duration::from_millis(35));
        assert_eq!(d.tick(), None);
        assert!(d.busy_pending_since.is_none());
        assert_eq!(d.current, RunnerStatus::Idle);
    }

    #[test]
    fn idle_detector_sustained_activity_flips_busy_after_debounce() {
        // Real streaming activity (multiple chunks across the
        // debounce window) must flip Busy exactly when the burst
        // crosses the debounce threshold.
        let mut d = detector_with_debounce_and_threshold(
            Duration::from_millis(20),
            Duration::from_millis(1),
        );
        assert_eq!(d.on_bytes(8), None); // starts the timer
        std::thread::sleep(Duration::from_millis(25));
        assert_eq!(d.on_bytes(8), Some(RunnerStatus::Busy));
    }

    #[test]
    fn parse_pane_status_line_alive_pane() {
        // Live pane: dead=0, no dead-status token, then pid + cmd.
        let (pane, status) = parse_pane_status_line("%0 0  12345 sh").unwrap();
        assert_eq!(pane, "%0");
        assert!(status.alive);
        assert_eq!(status.exit_code, None);
        assert_eq!(status.pid, Some(12345));
        assert_eq!(status.command.as_deref(), Some("sh"));
    }

    #[test]
    fn parse_pane_status_line_dead_pane_with_exit_code() {
        // Dead pane: dead=1, dead-status=42, then pid + cmd.
        let (pane, status) = parse_pane_status_line("%3 1 42 67890 claude").unwrap();
        assert_eq!(pane, "%3");
        assert!(!status.alive);
        assert_eq!(status.exit_code, Some(42));
        assert_eq!(status.pid, Some(67890));
        assert_eq!(status.command.as_deref(), Some("claude"));
    }

    #[test]
    fn parse_pane_status_line_dead_pane_zero_exit() {
        // Clean exit (status 0) is the success path; manager
        // distinguishes "stopped" vs "crashed" via this.
        let (_, status) = parse_pane_status_line("%1 1 0 100 bash").unwrap();
        assert!(!status.alive);
        assert_eq!(status.exit_code, Some(0));
    }

    #[test]
    fn parse_pane_status_line_command_with_spaces() {
        // pane_current_command can include args / spaces (rare in
        // practice, but agents launched with -c "..." can show up
        // this way).
        let (_, status) = parse_pane_status_line("%0 0  100 my agent --flag").unwrap();
        assert_eq!(status.command.as_deref(), Some("my agent --flag"));
    }

    #[test]
    fn parse_pane_status_line_rejects_bad_shapes() {
        for bad in [
            "",     // empty
            "%0",   // missing dead
            "%0 ?", // bad dead value
            "%0 0", // missing pid
        ] {
            assert!(
                parse_pane_status_line(bad).is_none(),
                "{bad:?} should not parse"
            );
        }
    }

    // ─────── Tmux-gated integration tests ───────
    //
    // These actually shell out to a tmux server. They use a
    // per-pid `-L` label so they never touch the user's tmux
    // server, and a tempfile config so the user's ~/.tmux.conf
    // doesn't shadow remain-on-exit / window-size.
    //
    // Run with `cargo test --lib session::tmux_runtime -- --ignored`
    // when tmux is locally available.

    /// Build an isolated `TmuxRuntime` for a single test. The
    /// caller passes a unique `test_label` so each test owns its
    /// own tmux server / tempdir; sharing a label makes parallel
    /// tests step on each other's `kill-server` cleanup.
    fn test_runtime(test_label: &str) -> Option<TmuxRuntime> {
        if resolve_tmux_binary().is_err() {
            return None;
        }
        let dir = tempfile::tempdir().ok()?;
        // Leak the dir for the duration of the test process so the
        // tmux server's socket dir survives until cleanup.
        let path = Box::leak(Box::new(dir)).path().to_path_buf();
        let label = format!("runner-test-{}-{}", std::process::id(), test_label);
        let tmux_bin = resolve_tmux_binary().ok()?;
        let config_path = write_runner_config(&path).ok()?;
        let runtime_dir = path.join("sessions");
        std::fs::create_dir_all(&runtime_dir).ok()?;
        Some(TmuxRuntime {
            tmux_bin,
            label,
            config_path,
            runtime_dir,
            home: std::env::var_os("HOME").map(PathBuf::from),
        })
    }

    fn cleanup(rt: &TmuxRuntime) {
        // Tear down the per-test server so we don't leak
        // daemons across runs.
        let _ = rt.cmd().arg("kill-server").status();
    }

    fn spawn_echo(
        rt: &TmuxRuntime,
        id: &str,
        msg: &str,
    ) -> RuntimeResult<(RuntimeSession, OutputStream)> {
        // Emit `msg` continuously so the test isn't sensitive to
        // the tiny capture-pane / pipe-pane install race the plan
        // calls out (Step 6 "tiny window"). A one-shot printf can
        // land in the pane buffer before pipe-pane is installed,
        // miss the live stream, and only show in the Replay
        // snapshot — which itself races the launch-script fork.
        // Repeat-until-killed sidesteps both.
        let spec = SpawnSpec {
            session_id: id.into(),
            cwd: None,
            command: "/bin/sh".into(),
            args: vec![
                "-c".into(),
                format!("while :; do printf '%s\\n' {} ; sleep 0.2 ; done", msg),
            ],
            env: Default::default(),
            mission: false,
            ..Default::default()
        };
        rt.spawn(spec)
    }

    #[test]
    #[ignore]
    fn integration_spawn_and_observe() {
        let Some(rt) = test_runtime("spawn-and-observe") else {
            log::warn!("tmux not available — skipping");
            return;
        };
        let (session, rx) = spawn_echo(&rt, "spawnobs01", "hello-from-tmux").unwrap();
        // First event is the Replay snapshot. Then Stream events
        // arrive as `printf` writes land in pipe-pane's FIFO.
        let mut got = Vec::new();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(3);
        while std::time::Instant::now() < deadline {
            match rx.recv_timeout(std::time::Duration::from_millis(200)) {
                Ok(RuntimeOutput::Replay(b)) | Ok(RuntimeOutput::Stream(b)) => {
                    got.extend_from_slice(&b);
                    if String::from_utf8_lossy(&got).contains("hello-from-tmux") {
                        break;
                    }
                }
                Ok(RuntimeOutput::StatusTransition { .. }) => continue,
                Err(_) => continue,
            }
        }
        assert!(
            String::from_utf8_lossy(&got).contains("hello-from-tmux"),
            "missing payload; got = {:?}",
            String::from_utf8_lossy(&got)
        );
        rt.stop(&session).unwrap();
        cleanup(&rt);
    }

    #[test]
    #[ignore]
    fn integration_send_key_and_resize() {
        let Some(rt) = test_runtime("send-key-and-resize") else {
            return;
        };
        let (session, _rx) = spawn_echo(&rt, "sendkey01", "ready").unwrap();
        rt.send_key(&session, "Enter").expect("send_key Enter");
        rt.send_bytes(&session, b"echo hi\n")
            .expect("send_bytes echo");
        rt.resize(&session, 100, 30).expect("resize");
        rt.stop(&session).unwrap();
        cleanup(&rt);
    }

    #[test]
    #[ignore]
    fn integration_status_alive_then_dead() {
        let Some(rt) = test_runtime("status-alive-then-dead") else {
            return;
        };
        // Quick-exit command so the pane goes dead within the
        // test deadline. With remain-on-exit on (in the runner
        // config), the pane stays around with pane_dead=1 +
        // pane_dead_status so we can read the exit code.
        let spec = SpawnSpec {
            session_id: "statusprobe01".into(),
            cwd: None,
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "exit 7".into()],
            env: Default::default(),
            mission: false,
            ..Default::default()
        };
        let (session, _rx) = rt.spawn(spec).unwrap();

        // Poll up to 2s for pane_dead=1. The agent exits ~ms
        // after spawn but tmux's pane reaper has its own cadence.
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(2);
        let mut final_status = None;
        let mut last_seen = None;
        while std::time::Instant::now() < deadline {
            let st = rt.status(&session).unwrap();
            last_seen = Some(format!("{st:?}"));
            match st {
                Some(s) if !s.alive => {
                    final_status = Some(s);
                    break;
                }
                _ => std::thread::sleep(std::time::Duration::from_millis(50)),
            }
        }
        let s = final_status.unwrap_or_else(|| {
            panic!("pane never went dead; last status seen = {last_seen:?}");
        });
        assert!(!s.alive);
        assert_eq!(s.exit_code, Some(7), "exit code should be 7");

        // After kill-session the runtime should report None
        // (terminal-unavailable).
        rt.stop(&session).unwrap();
        std::thread::sleep(std::time::Duration::from_millis(100));
        let after = rt.status(&session).unwrap();
        assert!(after.is_none(), "expected None after kill, got {after:?}");
        cleanup(&rt);
    }

    #[test]
    #[ignore]
    fn integration_reconcile_config_reloads_legacy_server() {
        // Simulate a legacy server: start one, then explicitly
        // unset the @runner_config_version user-option so the
        // probe sees a stamp-less running server. reconcile
        // should detect that and source-file the config back
        // in, returning Ok(true).
        let Some(rt) = test_runtime("reconcile-legacy") else {
            return;
        };
        let spec = SpawnSpec {
            session_id: "reconcilelegacy01".into(),
            cwd: None,
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 5".into()],
            env: Default::default(),
            mission: false,
            ..Default::default()
        };
        let (session, _rx) = rt.spawn(spec).unwrap();
        // Unset the stamp on the running server.
        let unset = rt
            .cmd()
            .arg("set-option")
            .arg("-g")
            .arg("-u")
            .arg("@runner_config_version")
            .status()
            .unwrap();
        assert!(unset.success(), "set-option -u failed");
        // Now reconcile_config should reload (legacy path).
        let reloaded = rt.reconcile_config().unwrap();
        assert!(reloaded, "expected reload of legacy server");
        // After reload, calling again should be a no-op.
        let second = rt.reconcile_config().unwrap();
        assert!(!second, "second call should observe current stamp");
        rt.stop(&session).unwrap();
        cleanup(&rt);
    }

    #[test]
    #[ignore]
    fn integration_reconcile_config_no_op_on_fresh_server() {
        // A fresh server's first session creation already loads
        // -f <runner.conf>, so reconcile_config should observe a
        // matching stamp and return Ok(false) (no reload needed).
        let Some(rt) = test_runtime("reconcile-noop") else {
            return;
        };
        // Create a session so the server actually exists.
        let spec = SpawnSpec {
            session_id: "reconcileprobe01".into(),
            cwd: None,
            command: "/bin/sh".into(),
            args: vec!["-c".into(), "sleep 5".into()],
            env: Default::default(),
            mission: false,
            ..Default::default()
        };
        let (session, _rx) = rt.spawn(spec).unwrap();
        let reloaded = rt.reconcile_config().unwrap();
        assert!(!reloaded, "fresh server should already have current stamp");
        rt.stop(&session).unwrap();
        cleanup(&rt);
    }

    #[test]
    #[ignore]
    fn integration_paste_round_trip() {
        let Some(rt) = test_runtime("paste-round-trip") else {
            return;
        };
        let (session, _rx) = spawn_echo(&rt, "paste01", "ready").unwrap();
        // Multi-line UTF-8 with embedded `;`, embedded `'`, and a
        // trailing newline — exercises the load-buffer + paste-
        // buffer -p -r -d path.
        rt.paste(&session, b"line one;\nline two's\n")
            .expect("paste");
        rt.stop(&session).unwrap();
        cleanup(&rt);
    }
}
