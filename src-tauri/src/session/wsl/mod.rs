//! Windows + WSL bridge for the PTY runtime.
//!
//! On Windows the coding agents (claude, codex) live inside a WSL2
//! distro, not on the Windows host. The native PTY runtime forks a
//! ConPTY child; here we make that child `wsl.exe`, which relays the
//! pseudoconsole into the Linux side so the agent's TUI renders with a
//! real TTY, resize propagates as SIGWINCH, and the agent's exit code
//! comes back through wsl.exe. This module owns the one platform-
//! specific seam — turning a `SpawnSpec` into a `wsl.exe …`
//! `CommandBuilder`.
//!
//! ## Why `bash -lic 'exec <agent>'`
//!
//! The agents resolve as bare names (`claude`, `codex`) only on the
//! user's *interactive-login* PATH — `~/.local/bin`, nvm, linuxbrew are
//! all added by `~/.bashrc`, which bails early (`case $- in *i*) … *)
//! return`) unless the shell is interactive. Interactive mode (`-i`) in
//! turn only "takes" when bash has a real TTY — which it does here,
//! because the ConPTY gives the WSL side a genuine pty. So:
//!
//!   * `-l` sources `~/.profile`, `-i` sources `~/.bashrc` ⇒ the full
//!     user PATH is present, exactly as in the user's own terminal.
//!   * `exec` replaces bash with the agent in-place: no lingering
//!     interactive shell, and the agent's exit status propagates out
//!     through wsl.exe.
//!
//! A startup PATH *probe* was tried instead, but a probe runs over a
//! pipe (no TTY), so `-i` is dropped and the probed PATH is missing the
//! user's dirs. Letting the real, ConPTY-backed spawn be the
//! interactive shell sidesteps that entirely — verified end-to-end
//! against a live `claude` rendering its TUI.
//!
//! Teardown note: under ConPTY the `wsl.exe` relay only reaps once the
//! master pty is dropped (ClosePseudoConsole). The session manager's
//! kill path covers the common stop; graceful self-exit detection is a
//! follow-up (plan M2+).
//!
//! M1 scope: `bash -lic 'exec <agent> <args…>'`, agent inherits the
//! runtime's cwd. Working-directory translation, env injection, the
//! cross-boundary event bus, distro selection, and the
//! Windows-native-vs-WSL execution-target switch are M2+/M3 (see plan:
//! sparkling-honking-spindle.md).

pub mod install;
pub mod job;
pub mod path;

use std::path::Path;

use portable_pty::CommandBuilder;

use super::launch::{is_reserved_env_name, is_valid_env_name, shell_quote};
use super::pty_runtime::CommandShaper;
use super::runtime::{RuntimeError, RuntimeResult, SpawnSpec};
use install::RUNNER_BIN_SUBDIR;
use path::win_to_wsl;

/// Env vars whose values are Windows paths and must be translated to
/// their `/mnt/c/…` WSL form before crossing into the distro. These are
/// baked by `session::manager::spawn` for mission sessions.
const WSL_PATH_ENV_KEYS: &[&str] = &["RUNNER_EVENT_LOG", "MISSION_CWD"];

/// Absolute path to wsl.exe — not the bare name, to dodge PATH search
/// and any WoW64 file-system redirection from a 32-bit host shim.
const WSL_EXE: &str = r"C:\Windows\System32\wsl.exe";

/// Build a [`CommandShaper`] that runs each agent inside the given WSL
/// distro. See the module docs for the invocation rationale.
pub fn wsl_command_shaper(distro: String) -> CommandShaper {
    Box::new(
        move |spec: &SpawnSpec, composed_path: &str| -> RuntimeResult<CommandBuilder> {
            // Per-runner execution target: "native" runs the command
            // directly on the Windows host (a Windows-installed claude/codex,
            // powershell, …), everything else (incl. NULL) runs inside WSL.
            if spec.exec_target.as_deref() == Some("native") {
                return windows_native_shaper(spec, composed_path);
            }
            // Deliver the launch script as a FILE that bash `source`s,
            // not as an inline `bash -lic '<body>'` argument. A mission
            // lead's launch prompt carries backticks, nested quotes,
            // newlines and CJK; passed inline it gets mangled crossing
            // wsl.exe's Windows command line (backticks fire as command
            // substitution, quotes unbalance, `exec claude` breaks). In a
            // file, bash parses it directly — no Windows command-line
            // layer. The argv to wsl.exe is then just `source <path>`,
            // which is short and quote-clean.
            let inner = build_launch_inner(spec);
            let source_cmd = write_launch_script(&spec.session_id, &inner)?;
            let mut cmd = CommandBuilder::new(WSL_EXE);
            cmd.args([
                "-d",
                distro.as_str(),
                "--",
                "bash",
                "-lic",
                source_cmd.as_str(),
            ]);
            Ok(cmd)
        },
    )
}

/// Native (Windows-host) execution target. Windows agent CLIs — claude,
/// codex — usually ship as `.cmd`/`.ps1` shims (npm, nodist), which
/// `CreateProcess` (and thus portable-pty) can't launch directly: only a
/// shell resolves `PATHEXT`. Route the command through `cmd.exe /c` so a bare
/// `codex` / `claude` (or an explicit `foo.cmd`) resolves exactly as it would
/// when typed at a prompt. A real `.exe` like `powershell` also works through
/// this path, so it isn't special-cased. Mirrors `native_command_shaper`'s
/// env handling (reserved PATH from the composed result, name validation).
fn windows_native_shaper(spec: &SpawnSpec, composed_path: &str) -> RuntimeResult<CommandBuilder> {
    let comspec =
        std::env::var("ComSpec").unwrap_or_else(|_| r"C:\Windows\System32\cmd.exe".to_string());
    let mut cmd = CommandBuilder::new(comspec);
    // `/c` runs the command line then exits. command + args go as separate
    // argv entries; CommandBuilder quotes each for the Windows command line.
    cmd.arg("/c");
    cmd.arg(&spec.command);
    cmd.args(&spec.args);
    if let Some(cwd) = &spec.cwd {
        cmd.cwd(cwd);
    }
    for (k, v) in &spec.env {
        if is_reserved_env_name(k) {
            continue;
        }
        if !is_valid_env_name(k) {
            return Err(RuntimeError::Msg(format!(
                "invalid env var name {k:?}: must match [A-Za-z_][A-Za-z0-9_]*"
            )));
        }
        cmd.env(k, v);
    }
    cmd.env("PATH", composed_path);
    Ok(cmd)
}

/// Write the rendered launch-script body to a per-session file under the
/// Windows temp dir and return the `source <wsl-path>` command for
/// `bash -lic`. `source` (not `exec bash <file>`) so the script runs in
/// the login+interactive shell that already loaded the user's PATH —
/// claude/codex/runner resolve there; the script's own `exec` then
/// replaces that shell with the agent.
fn write_launch_script(session_id: &str, body: &str) -> RuntimeResult<String> {
    let mut dir = std::env::temp_dir();
    dir.push("runner-launch");
    std::fs::create_dir_all(&dir)
        .map_err(|e| RuntimeError::Msg(format!("create launch dir: {e}")))?;
    let file = dir.join(format!("{session_id}.sh"));
    std::fs::write(&file, body)
        .map_err(|e| RuntimeError::Msg(format!("write launch script: {e}")))?;
    let wsl_path = win_to_wsl(&file);
    Ok(format!("source {}", shell_quote(&wsl_path)))
}

/// Render the `bash -lic` command string:
///
/// 1. `export` each session env var (mission-bus `RUNNER_*`, the runner's
///    own env, TERM/COLORTERM). Values that are Windows paths
///    (`RUNNER_EVENT_LOG`, `MISSION_CWD`) are translated to `/mnt/c/…` so
///    the in-WSL agent and its `runner` CLI read the same NTFS event log
///    the Windows host watches. PATH is skipped (reserved) — the login
///    shell owns it.
/// 2. For mission sessions, prepend the in-distro bundled `runner` CLI
///    dir to PATH so the agent can emit signals onto the event bus.
/// 3. Optionally `cd` into the translated working directory, then
///    `exec <agent> <args…>`, every token single-quoted so arbitrary
///    flags / first-turn prompts survive the parse. `cd … &&` so a bad
///    cwd fails loud instead of silently launching in the wrong place.
///
/// Pure (no I/O) so it's unit-testable without a live WSL.
fn build_launch_inner(spec: &SpawnSpec) -> String {
    let mut s = String::new();

    for (k, v) in &spec.env {
        if is_reserved_env_name(k) {
            continue; // PATH is owned by the login shell / the line below.
        }
        let value = if WSL_PATH_ENV_KEYS.contains(&k.as_str()) {
            win_to_wsl(Path::new(v))
        } else {
            v.clone()
        };
        s.push_str("export ");
        s.push_str(k);
        s.push('=');
        s.push_str(&shell_quote(&value));
        s.push('\n');
    }

    if spec.mission {
        // `$HOME`/`$PATH` are expanded by the running login shell, so use
        // double quotes here (everything else is single-quoted literals).
        s.push_str(&format!(
            "export PATH=\"$HOME/{RUNNER_BIN_SUBDIR}:$PATH\"\n"
        ));
    }

    if let Some(cwd) = &spec.cwd {
        s.push_str("cd ");
        s.push_str(&shell_quote(&win_to_wsl(cwd)));
        s.push_str(" && ");
    }
    s.push_str("exec ");
    s.push_str(&shell_quote(&spec.command));
    for arg in &spec.args {
        s.push(' ');
        s.push_str(&shell_quote(arg));
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(command: &str, args: &[&str]) -> SpawnSpec {
        SpawnSpec {
            command: command.into(),
            args: args.iter().map(|s| s.to_string()).collect(),
            ..Default::default()
        }
    }

    #[test]
    fn mission_session_injects_translated_env_and_runner_path() {
        let mut s = spec("claude", &[]);
        s.mission = true;
        s.env.insert("RUNNER_HANDLE".into(), "architect".into());
        s.env.insert(
            "RUNNER_EVENT_LOG".into(),
            r"C:\Users\h\AppData\Roaming\app\crews\c1\missions\m1\events.ndjson".into(),
        );
        let inner = build_launch_inner(&s);
        assert!(
            inner.contains("export RUNNER_HANDLE='architect'\n"),
            "inner =\n{inner}"
        );
        // RUNNER_EVENT_LOG translated C:\ -> /mnt/c/ for the in-WSL agent.
        assert!(
            inner.contains("export RUNNER_EVENT_LOG='/mnt/c/Users/h/AppData/Roaming/app/crews/c1/missions/m1/events.ndjson'\n"),
            "inner =\n{inner}"
        );
        assert!(
            inner.contains("export PATH=\"$HOME/.local/share/runner/bin:$PATH\"\n"),
            "inner =\n{inner}"
        );
        assert!(
            inner.trim_end().ends_with("exec 'claude'"),
            "inner =\n{inner}"
        );
    }

    #[test]
    fn direct_chat_has_no_runner_path_or_bus_env() {
        // mission=false, empty env → unchanged M1 behaviour.
        let inner = build_launch_inner(&spec("claude", &[]));
        assert_eq!(inner, "exec 'claude'");
    }

    #[test]
    fn builds_exec_with_quoted_tokens() {
        let inner = build_launch_inner(&spec("claude", &["--model", "opus", "hi there"]));
        assert_eq!(inner, "exec 'claude' '--model' 'opus' 'hi there'");
    }

    #[test]
    fn builds_exec_no_args() {
        assert_eq!(build_launch_inner(&spec("codex", &[])), "exec 'codex'");
    }

    #[test]
    fn escapes_single_quotes_in_args() {
        let inner = build_launch_inner(&spec("claude", &["it's"]));
        assert_eq!(inner, r#"exec 'claude' 'it'\''s'"#);
    }

    #[test]
    fn prepends_cd_for_translated_windows_cwd() {
        let mut s = spec("claude", &["--resume", "x"]);
        s.cwd = Some(std::path::PathBuf::from(r"C:\Users\Haochen\proj"));
        let inner = build_launch_inner(&s);
        assert_eq!(
            inner,
            "cd '/mnt/c/Users/Haochen/proj' && exec 'claude' '--resume' 'x'"
        );
    }

    /// The shaper produces `wsl.exe -d <distro> -- bash -lic 'source
    /// <script>'` — the launch body rides in a file, not inline (see
    /// `write_launch_script`). The file's *contents* are covered by the
    /// `build_launch_inner` tests above.
    #[test]
    fn shaper_sources_a_launch_script_file() {
        let mut s = spec("claude", &["--resume", "abc"]);
        s.session_id = "shaper_test_session".into();
        let shaper = wsl_command_shaper("Ubuntu".into());
        let cmd = shaper(&s, "").unwrap();
        let argv: Vec<String> = cmd
            .get_argv()
            .iter()
            .map(|s| s.to_string_lossy().into_owned())
            .collect();
        assert_eq!(argv[0], WSL_EXE);
        assert_eq!(&argv[1..6], &["-d", "Ubuntu", "--", "bash", "-lic"]);
        assert!(argv[6].starts_with("source '/mnt/"), "argv6 = {}", argv[6]);
        assert!(
            argv[6].contains("runner-launch") && argv[6].ends_with("shaper_test_session.sh'"),
            "argv6 = {}",
            argv[6]
        );
        // And the written file holds the exact launch body.
        let mut p = std::env::temp_dir();
        p.push("runner-launch");
        p.push("shaper_test_session.sh");
        let body = std::fs::read_to_string(&p).unwrap();
        assert_eq!(body, "exec 'claude' '--resume' 'abc'");
        let _ = std::fs::remove_file(&p);
    }
}
