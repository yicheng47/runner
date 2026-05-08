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
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;

use super::launch;
use super::runtime::{
    OutputStream, RuntimeError, RuntimeOutput, RuntimeResult, RuntimeSession, SessionRuntime,
    SpawnSpec,
};
use super::tmux::{resolve_tmux_binary, tmux_cmd, write_runner_config, DEFAULT_LABEL};

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

// ──────────────────────────────────────────────────────────────────
// SessionRuntime impl.
// ──────────────────────────────────────────────────────────────────

impl SessionRuntime for TmuxRuntime {
    fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
        validate_session_id(&spec.session_id)?;
        let sess_name = session_name(&spec.session_id);
        let sess_dir = self.session_dir(&spec.session_id);
        std::fs::create_dir_all(&sess_dir)?;

        // 1. Compose PATH + render launch script.
        let process_path = std::env::var("PATH").ok();
        let composed = launch::compose_path(
            None, // shim_dir — wired by manager in Step 9 for missions
            None, // bundled_bin_dir — same
            None, // shell_path — manager's responsibility to provide
            self.home.as_deref(),
            process_path.as_deref(),
        );
        let script = launch::LaunchScript {
            command: spec.command.clone(),
            args: spec.args.clone(),
            cwd: spec.cwd.clone(),
            env: spec
                .env
                .iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect(),
            path: composed,
        };
        let launch_path = launch::write_launch_script(&sess_dir, &script)?;

        // 2. Create per-session FIFO and open reader-side
        //    (O_NONBLOCK to avoid the open-side block; flip back to
        //    blocking for reads so the forwarder thread sleeps when
        //    the FIFO is empty).
        let fifo_path = sess_dir.join("output.fifo");
        ensure_fifo(&fifo_path)?;
        let reader = open_fifo_reader_blocking(&fifo_path)?;

        // 3. Wire the output channel. Send Replay before installing
        //    pipe-pane so xterm.js sees the snapshot first; the
        //    forwarder thread then begins flushing live Stream
        //    chunks once `cat` connects to the writer side.
        let (tx, rx) = mpsc::channel::<RuntimeOutput>();
        let forwarder_tx = tx.clone();
        thread::spawn(move || forward_fifo(reader, forwarder_tx));

        // 4. Pre-spawn: kill any stale session left over from a
        //    crashed prior process. has-session returns non-zero if
        //    missing; treat both outcomes as success.
        let _ = self
            .cmd()
            .arg("kill-session")
            .arg(target(&sess_name))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();

        // 5. tmux new-session.
        let mut new_session = self.cmd();
        new_session
            .arg("new-session")
            .arg("-d")
            .arg("-P")
            .arg("-F")
            .arg("#{pane_id}")
            .arg("-s")
            .arg(&sess_name)
            .arg("-n")
            .arg("main");
        if let Some(cwd) = &spec.cwd {
            new_session.arg("-c").arg(cwd);
        }
        // tmux's trailing positional is a shell-command string
        // (passed to default-shell -c), so quote the launch script
        // path explicitly.
        new_session
            .arg("--")
            .arg(launch::shell_quote(&launch_path.display().to_string()));
        let output = new_session.output()?;
        if !output.status.success() {
            return Err(RuntimeError::TmuxFailed {
                command: "new-session".into(),
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

        // 6. Replay snapshot, then pipe-pane install.
        emit_replay(&self.cmd(), &session, &tx)?;
        install_pipe_pane(&self.cmd(), &session, &fifo_path)?;

        Ok((session, rx))
    }

    fn resume(&self, session: &RuntimeSession) -> RuntimeResult<OutputStream> {
        // Confirm the session is still alive before doing anything
        // expensive. has-session prints nothing on success and exits
        // 1 if the target is missing.
        let status = self
            .cmd()
            .arg("has-session")
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

        // Reattach: open a fresh FIFO + forwarder, snapshot the
        // pane state for replay, install a new pipe-pane (close
        // any stale one first per Step 6's reattach-safe pattern).
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
        let reader = open_fifo_reader_blocking(&fifo_path)?;

        let (tx, rx) = mpsc::channel::<RuntimeOutput>();
        let forwarder_tx = tx.clone();
        thread::spawn(move || forward_fifo(reader, forwarder_tx));

        emit_replay(&self.cmd(), session, &tx)?;
        install_pipe_pane(&self.cmd(), session, &fifo_path)?;
        Ok(rx)
    }

    fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
        // Best-effort. kill-session against a missing target is
        // not a runtime error — the manager polls list-panes
        // separately to confirm the pane is gone (Step 8).
        let _ = self
            .cmd()
            .arg("kill-session")
            .arg(target(&session.session_name))
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
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

/// Open a FIFO with `O_RDWR`. Three properties we need from this:
///
/// 1. **Non-blocking open.** O_RDONLY blocks until a writer
///    attaches (default POSIX FIFO semantics). O_RDONLY|O_NONBLOCK
///    returns immediately but reads then return EOF until a writer
///    shows up. O_RDWR opens immediately without either of those
///    problems — we have a write end too, so the kernel sees a
///    writer attached.
/// 2. **No spurious EOF.** As long as our own fd stays open, the
///    kernel never reports EOF. That matters because the forwarder
///    thread starts before tmux's `pipe-pane | cat` writer attaches,
///    and an O_RDONLY reader sees EOF whenever there are zero
///    writers — including the ~ms window before pipe-pane installs.
///    With O_RDWR the read blocks instead, which is what we want.
/// 3. **Blocking reads.** Default for O_RDWR. The forwarder loop
///    sleeps in `read()` when the FIFO is empty.
///
/// We never write through this fd — the forwarder only reads. The
/// write side is purely a kernel-side ref-count keeper.
fn open_fifo_reader_blocking(path: &Path) -> RuntimeResult<std::fs::File> {
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)?;
    Ok(file)
}

/// Forwarder loop: read FIFO bytes and emit `RuntimeOutput::Stream`
/// chunks. Exits cleanly on EOF (writer closed — session ended) or
/// when the receiver drops (manager detached).
fn forward_fifo(mut reader: std::fs::File, tx: mpsc::Sender<RuntimeOutput>) {
    let mut buf = [0u8; 8192];
    loop {
        match reader.read(&mut buf) {
            Ok(0) => break, // EOF
            Ok(n) => {
                if tx.send(RuntimeOutput::Stream(buf[..n].to_vec())).is_err() {
                    break;
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

/// Probe alternate-screen state and run the right capture-pane
/// shape. Send the result to `tx` as a single `Replay` event.
fn emit_replay(
    cmd: &Command,
    session: &RuntimeSession,
    tx: &mpsc::Sender<RuntimeOutput>,
) -> RuntimeResult<()> {
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
    // Best-effort: a closed receiver means the manager already
    // dropped the channel; drop the snapshot rather than failing.
    let _ = tx.send(RuntimeOutput::Replay(out.stdout));
    Ok(())
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
        };
        rt.spawn(spec)
    }

    #[test]
    #[ignore]
    fn integration_spawn_and_observe() {
        let Some(rt) = test_runtime("spawn-and-observe") else {
            eprintln!("tmux not available — skipping");
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
