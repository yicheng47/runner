//! Shell integration for Runner's terminal shells (#575): a prompt hook that
//! reports the shell's working directory through OSC 7, so a split opens
//! where the shell is. zsh gets it through a `ZDOTDIR` wrapper, bash through
//! a `PROMPT_COMMAND` bootstrap. Both only add environment variables; argv
//! and the user's own startup files are untouched, and the hook removes
//! itself when the user's configuration already sends OSC 7.

use std::path::{Path, PathBuf};

use crate::session::runtime::SpawnSpec;

const ZSH_ENV: &str = include_str!("../shell-integration/zsh/.zshenv");
const BASH_SCRIPT: &str = include_str!("../shell-integration/bash/runner.bash");

/// The `PROMPT_COMMAND` entry that sources the bash script at the first
/// prompt. The script finds and replaces this exact text.
const BASH_BOOTSTRAP: &str = ". \"$RUNNER_BASH_INTEGRATION\"";

/// Adds the OSC 7 hook to a zsh or bash terminal shell's spawn. Any other
/// command, and every platform but Unix, is left as it is. `inherited` reads
/// a variable from the environment the child inherits beside `spec.env`.
pub(crate) fn inject(
    spec: &mut SpawnSpec,
    app_data_dir: &Path,
    inherited: impl Fn(&str) -> Option<String>,
) {
    // A relative ZDOTDIR or script path would resolve against the shell's cwd.
    if !cfg!(unix) || !app_data_dir.is_absolute() {
        return;
    }
    let root = app_data_dir.join("shell-integration");
    let shell = Path::new(&spec.command)
        .file_name()
        .and_then(|name| name.to_str());
    let current =
        |spec: &SpawnSpec, name: &str| spec.env.get(name).cloned().or_else(|| inherited(name));
    match shell {
        Some("zsh") => {
            let dir = root.join("zsh");
            if !install(&dir.join(".zshenv"), ZSH_ENV, &spec.session_id) {
                return;
            }
            if let Some(user) = current(spec, "ZDOTDIR") {
                spec.env.insert("RUNNER_ZSH_ZDOTDIR".into(), user);
            }
            spec.env
                .insert("ZDOTDIR".into(), dir.to_string_lossy().into_owned());
        }
        Some("bash") => {
            let script = root.join("bash").join("runner.bash");
            if !install(&script, BASH_SCRIPT, &spec.session_id) {
                return;
            }
            let prompt_command = match current(spec, "PROMPT_COMMAND") {
                Some(existing) if !existing.is_empty() => format!("{existing}\n{BASH_BOOTSTRAP}"),
                _ => BASH_BOOTSTRAP.to_owned(),
            };
            spec.env.insert("PROMPT_COMMAND".into(), prompt_command);
            spec.env.insert(
                "RUNNER_BASH_INTEGRATION".into(),
                script.to_string_lossy().into_owned(),
            );
        }
        _ => {}
    }
}

/// Writes `body` to `path` unless it is already there, through a temporary
/// file and a rename so a shell starting concurrently never reads half a
/// script. False, logged, if it could not be written.
fn install(path: &Path, body: &str, session_id: &str) -> bool {
    if std::fs::read(path).is_ok_and(|current| current == body.as_bytes()) {
        return true;
    }
    let result = (|| -> std::io::Result<()> {
        let dir = path.parent().expect("integration script has a parent");
        std::fs::create_dir_all(dir)?;
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        let temporary: PathBuf = dir.join(format!(".{name}.{session_id}.tmp"));
        std::fs::write(&temporary, body)?;
        std::fs::rename(&temporary, path).inspect_err(|_| {
            let _ = std::fs::remove_file(&temporary);
        })
    })();
    match result {
        Ok(()) => true,
        Err(error) => {
            log::warn!(
                "shell integration unavailable, could not write {}: {error}",
                path.display()
            );
            false
        }
    }
}

/// Whether an OSC 7 report's host names this machine: empty, `localhost`,
/// this machine's hostname, or its first label, ignoring ASCII case. zsh's
/// `$HOST` and bash's `$HOSTNAME` both come from `gethostname`.
pub fn is_local_host(host: &str) -> bool {
    if host.is_empty() || host.eq_ignore_ascii_case("localhost") {
        return true;
    }
    local_hostname().is_some_and(|name| {
        host.eq_ignore_ascii_case(&name)
            || name
                .split('.')
                .next()
                .is_some_and(|label| host.eq_ignore_ascii_case(label))
    })
}

#[cfg(unix)]
pub fn local_hostname() -> Option<String> {
    let mut buf = [0u8; 256];
    // SAFETY: the buffer outlives the call and its length is passed with it.
    let status = unsafe { libc::gethostname(buf.as_mut_ptr().cast(), buf.len()) };
    if status != 0 {
        return None;
    }
    let end = buf.iter().position(|&byte| byte == 0).unwrap_or(buf.len());
    String::from_utf8(buf[..end].to_vec())
        .ok()
        .filter(|name| !name.is_empty())
}

#[cfg(windows)]
pub fn local_hostname() -> Option<String> {
    std::env::var("COMPUTERNAME")
        .ok()
        .filter(|name| !name.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell_spec(command: &str) -> SpawnSpec {
        SpawnSpec {
            session_id: "01TEST".into(),
            command: command.into(),
            args: vec!["-l".into()],
            ..SpawnSpec::default()
        }
    }

    fn nothing_inherited(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn local_host_accepts_this_machine_and_rejects_others() {
        assert!(is_local_host(""));
        assert!(is_local_host("localhost"));
        assert!(is_local_host("LocalHost"));
        let name = local_hostname().expect("hostname");
        assert!(is_local_host(&name));
        assert!(is_local_host(&name.to_ascii_uppercase()));
        assert!(is_local_host(name.split('.').next().unwrap()));
        assert!(!is_local_host("runner-575-elsewhere.invalid"));
    }

    #[cfg(unix)]
    #[test]
    fn zsh_gets_a_zdotdir_wrapper_and_keeps_argv() {
        let data = tempfile::tempdir().unwrap();
        let mut spec = shell_spec("/bin/zsh");
        inject(&mut spec, data.path(), nothing_inherited);
        let dir = data.path().join("shell-integration/zsh");
        assert_eq!(spec.args, vec!["-l".to_string()]);
        assert_eq!(spec.env.get("ZDOTDIR").map(String::as_str), dir.to_str());
        assert!(!spec.env.contains_key("RUNNER_ZSH_ZDOTDIR"));
        assert_eq!(
            std::fs::read_to_string(dir.join(".zshenv")).unwrap(),
            ZSH_ENV
        );
        assert_eq!(spec.env.len(), 1, "{:?}", spec.env);
    }

    #[cfg(unix)]
    #[test]
    fn zsh_passes_the_users_zdotdir_through() {
        let data = tempfile::tempdir().unwrap();
        let mut spec = shell_spec("/opt/homebrew/bin/zsh");
        inject(&mut spec, data.path(), |name| {
            (name == "ZDOTDIR").then(|| "/Users/me/.config/zsh".to_string())
        });
        assert_eq!(
            spec.env.get("RUNNER_ZSH_ZDOTDIR").map(String::as_str),
            Some("/Users/me/.config/zsh")
        );

        let mut spec = shell_spec("zsh");
        spec.env
            .insert("ZDOTDIR".into(), "/from/the/spawn/env".into());
        inject(&mut spec, data.path(), nothing_inherited);
        assert_eq!(
            spec.env.get("RUNNER_ZSH_ZDOTDIR").map(String::as_str),
            Some("/from/the/spawn/env")
        );
        assert_ne!(
            spec.env.get("ZDOTDIR").map(String::as_str),
            Some("/from/the/spawn/env")
        );
    }

    #[cfg(unix)]
    #[test]
    fn bash_gets_a_prompt_command_bootstrap_and_keeps_argv() {
        let data = tempfile::tempdir().unwrap();
        let script = data.path().join("shell-integration/bash/runner.bash");
        let mut spec = shell_spec("/bin/bash");
        inject(&mut spec, data.path(), nothing_inherited);
        assert_eq!(spec.args, vec!["-l".to_string()]);
        assert_eq!(
            spec.env,
            std::collections::BTreeMap::from([
                ("PROMPT_COMMAND".to_string(), BASH_BOOTSTRAP.to_string()),
                (
                    "RUNNER_BASH_INTEGRATION".to_string(),
                    script.to_string_lossy().into_owned()
                ),
            ])
        );
        assert_eq!(std::fs::read_to_string(&script).unwrap(), BASH_SCRIPT);
        assert!(BASH_SCRIPT.contains(&format!("'{BASH_BOOTSTRAP}'")));

        let mut spec = shell_spec("bash");
        inject(&mut spec, data.path(), |name| {
            (name == "PROMPT_COMMAND").then(|| "history -a".to_string())
        });
        assert_eq!(
            spec.env.get("PROMPT_COMMAND").map(String::as_str),
            Some(format!("history -a\n{BASH_BOOTSTRAP}").as_str())
        );
    }

    #[test]
    fn other_shells_are_left_alone() {
        let data = tempfile::tempdir().unwrap();
        for command in ["/opt/homebrew/bin/fish", "/bin/sh", "nu", "pwsh", "zsh5"] {
            let mut spec = shell_spec(command);
            inject(&mut spec, data.path(), nothing_inherited);
            assert!(spec.env.is_empty(), "{command}: {:?}", spec.env);
            assert_eq!(spec.args, vec!["-l".to_string()]);
        }
        assert!(!data.path().join("shell-integration").exists());
    }

    #[cfg(unix)]
    #[test]
    fn a_stale_script_is_rewritten_and_an_unwritable_one_skips_injection() {
        let data = tempfile::tempdir().unwrap();
        let path = data.path().join("shell-integration/zsh/.zshenv");
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, "# an older Runner's script").unwrap();
        let mut spec = shell_spec("zsh");
        inject(&mut spec, data.path(), nothing_inherited);
        assert_eq!(std::fs::read_to_string(&path).unwrap(), ZSH_ENV);
        let leftovers: Vec<_> = std::fs::read_dir(path.parent().unwrap())
            .unwrap()
            .map(|entry| entry.unwrap().file_name())
            .collect();
        assert_eq!(leftovers, vec![std::ffi::OsString::from(".zshenv")]);

        let blocked = tempfile::tempdir().unwrap();
        std::fs::write(blocked.path().join("shell-integration"), "not a directory").unwrap();
        let mut spec = shell_spec("bash");
        inject(&mut spec, blocked.path(), nothing_inherited);
        assert!(spec.env.is_empty(), "{:?}", spec.env);
    }
}

/// Real zsh and bash in a temporary `HOME`, spawned through the PTY runtime
/// with the injection, as a terminal pane would spawn them.
#[cfg(all(test, unix))]
mod real_shells {
    use super::*;
    use crate::session::pty_runtime::PtyRuntime;
    use crate::session::runtime::{OutputStream, RuntimeOutput, RuntimeSession, SessionRuntime};
    use std::collections::BTreeMap;
    use std::time::{Duration, Instant};

    /// `sub dir/中文`, typed as ASCII so neither line editor depends on the locale.
    const CD_ENCODED_DIR: &str = "cd $'sub dir/\\xe4\\xb8\\xad\\xe6\\x96\\x87'\r";
    const ENCODED_DIR: &str = "/sub%20dir/%E4%B8%AD%E6%96%87";

    struct Shell {
        runtime: PtyRuntime,
        session: RuntimeSession,
        stream: OutputStream,
        output: Vec<u8>,
    }

    impl Shell {
        fn start(
            command: &str,
            home: &Path,
            inherited: impl Fn(&str) -> Option<String>,
        ) -> Option<Self> {
            if !Path::new(command).exists() {
                eprintln!("skipping: {command} is not installed");
                return None;
            }
            std::fs::create_dir_all(home.join("sub dir/中文")).unwrap();
            let data = home.join("app-data");
            let mut spec = SpawnSpec {
                session_id: ulid::Ulid::new().to_string(),
                command: command.into(),
                args: crate::shell_path::shell_login_args(command),
                cwd: Some(home.to_path_buf()),
                env: BTreeMap::from([
                    ("HOME".into(), home.to_string_lossy().into_owned()),
                    ("TERM".into(), "xterm-256color".into()),
                    // Keeps macOS's /etc/zshrc and /etc/bashrc from loading
                    // Terminal.app's own OSC 7 hook when the tests run there.
                    ("TERM_PROGRAM".into(), "runner-test".into()),
                    ("BASH_SILENCE_DEPRECATION_WARNING".into(), "1".into()),
                ]),
                initial_size: Some((160, 40)),
                ..SpawnSpec::default()
            };
            inject(&mut spec, &data, inherited);
            let runtime = PtyRuntime::new();
            let (session, stream) = runtime.spawn(spec).unwrap();
            let mut shell = Self {
                runtime,
                session,
                stream,
                output: Vec::new(),
            };
            assert!(
                shell.wait_for(b"\x1b]7;"),
                "no report at the first prompt: {}",
                shell.text()
            );
            Some(shell)
        }

        fn send(&self, line: &str) {
            self.runtime
                .send_bytes(&self.session, line.as_bytes())
                .unwrap();
        }

        fn wait_for(&mut self, needle: &[u8]) -> bool {
            let deadline = Instant::now() + Duration::from_secs(15);
            while Instant::now() < deadline {
                if self
                    .output
                    .windows(needle.len())
                    .any(|window| window == needle)
                {
                    return true;
                }
                if let Ok(RuntimeOutput::Stream(bytes)) =
                    self.stream.recv_timeout(Duration::from_millis(100))
                {
                    self.output.extend_from_slice(&bytes);
                }
            }
            false
        }

        fn expect(&mut self, needle: &str) {
            assert!(
                self.wait_for(needle.as_bytes()),
                "expected {needle:?} in: {}",
                self.text()
            );
        }

        /// Every OSC 7 payload so far, with whether it ended in ST (Runner's
        /// hook) rather than BEL (the test's own emitters).
        fn reports(&self) -> Vec<(String, bool)> {
            let mut reports = Vec::new();
            let mut rest = self.output.as_slice();
            while let Some(start) = rest.windows(4).position(|window| window == b"\x1b]7;") {
                rest = &rest[start + 4..];
                let Some(end) = rest.iter().position(|&byte| byte == 0x07 || byte == 0x1b) else {
                    break;
                };
                reports.push((
                    String::from_utf8_lossy(&rest[..end]).into_owned(),
                    rest[end] == 0x1b,
                ));
                rest = &rest[end..];
            }
            reports
        }

        fn text(&self) -> String {
            String::from_utf8_lossy(&self.output).into_owned()
        }
    }

    impl Drop for Shell {
        fn drop(&mut self) {
            let _ = self.runtime.stop(&self.session);
        }
    }

    fn write(path: &Path, body: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, body).unwrap();
    }

    fn nothing_inherited(_: &str) -> Option<String> {
        None
    }

    #[test]
    fn zsh_reports_each_cd_and_loads_the_users_startup_files() {
        let home = tempfile::tempdir().unwrap();
        for file in ["zshenv", "zprofile", "zshrc", "zlogin"] {
            write(
                &home.path().join(format!(".{file}")),
                &format!("echo MARK-{file}\nPS1='%# '\n"),
            );
        }
        let Some(mut shell) = Shell::start("/bin/zsh", home.path(), nothing_inherited) else {
            return;
        };
        shell.send(CD_ENCODED_DIR);
        shell.expect(&format!("{ENCODED_DIR}\x1b\\"));
        for file in ["zshenv", "zprofile", "zshrc", "zlogin"] {
            shell.expect(&format!("MARK-{file}"));
        }
        shell
            .send("echo MARK-env-${ZDOTDIR-unset}-${RUNNER_ZSH_ZDOTDIR-unset}-$precmd_functions\r");
        shell.expect("MARK-env-unset-unset-__runner_osc7");
        let (report, st) = shell.reports().pop().unwrap();
        assert!(st);
        assert!(report.starts_with("file:///"), "{report}");
        assert!(report.ends_with(ENCODED_DIR), "{report}");
    }

    #[test]
    fn zsh_loads_startup_files_from_the_users_zdotdir() {
        let home = tempfile::tempdir().unwrap();
        let zdotdir = home.path().join(".config/zsh");
        write(&home.path().join(".zshrc"), "echo MARK-wrong-home-zshrc\n");
        for file in ["zshenv", "zprofile", "zshrc", "zlogin"] {
            write(
                &zdotdir.join(format!(".{file}")),
                &format!("echo MARK-zdotdir-{file}\nPS1='%# '\n"),
            );
        }
        let user_zdotdir = zdotdir.to_string_lossy().into_owned();
        let Some(mut shell) = Shell::start("/bin/zsh", home.path(), |name| {
            (name == "ZDOTDIR").then(|| user_zdotdir.clone())
        }) else {
            return;
        };
        shell.send(CD_ENCODED_DIR);
        shell.expect(&format!("{ENCODED_DIR}\x1b\\"));
        for file in ["zshenv", "zprofile", "zshrc", "zlogin"] {
            shell.expect(&format!("MARK-zdotdir-{file}"));
        }
        shell.send("echo MARK-zdotdir-is-$ZDOTDIR\r");
        shell.expect(&format!("MARK-zdotdir-is-{user_zdotdir}"));
        assert!(!shell.text().contains("MARK-wrong"), "{}", shell.text());
    }

    #[test]
    fn zsh_removes_its_hook_for_a_helper_emitter() {
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join(".zshrc"),
            "PS1='%# '\n\
             _emit_cwd() { printf '\\e]7;file://%s%s\\a' \"$HOST\" \"$PWD\" }\n\
             _prompt_hook() { _emit_cwd }\n\
             precmd_functions+=(_prompt_hook)\n",
        );
        let Some(mut shell) = Shell::start("/bin/zsh", home.path(), nothing_inherited) else {
            return;
        };
        shell.send(CD_ENCODED_DIR);
        shell.expect("/sub dir/中文\x07");
        shell.send("echo MARK-hooks-$precmd_functions-end\r");
        shell.expect("MARK-hooks-_prompt_hook-end");
        let reports = shell.reports();
        assert!(reports.iter().all(|(_, st)| !st), "{reports:?}");
    }

    #[test]
    fn zsh_removes_its_hook_when_the_prompt_starts_sending_osc_7() {
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join(".zshrc"),
            "PS1='%# '\nsetopt prompt_subst\n\
             _emit_cwd() { printf '%%{\\e]7;file://%s%s\\a%%}' \"$HOST\" \"$PWD\" }\n",
        );
        let Some(mut shell) = Shell::start("/bin/zsh", home.path(), nothing_inherited) else {
            return;
        };
        shell.send("PS1='$(_emit_cwd)%# '\r");
        shell.send(CD_ENCODED_DIR);
        shell.expect("/sub dir/中文\x07");
        shell.send("echo MARK-hooks-${#precmd_functions}-end\r");
        shell.expect("MARK-hooks-0-end");
        let reports = shell.reports();
        let first_user_report = reports.iter().position(|(_, st)| !st).unwrap();
        assert!(
            reports[first_user_report..].iter().all(|(_, st)| !st),
            "{reports:?}"
        );
    }

    /// Runner's hook runs first, while the user's autoloaded hook is still a
    /// stub, so the scan has to load it to see its body.
    #[test]
    fn zsh_removes_its_hook_for_an_autoloaded_emitter() {
        let home = tempfile::tempdir().unwrap();
        let functions = home.path().join("functions");
        write(
            &functions.join("_emit_cwd_hook"),
            "printf '\\e]7;file://%s%s\\a' \"$HOST\" \"$PWD\"\n",
        );
        write(
            &home.path().join(".zshrc"),
            &format!(
                "PS1='%# '\nfpath=('{}' $fpath)\nautoload -Uz _emit_cwd_hook\n\
                 precmd_functions+=(_emit_cwd_hook)\n",
                functions.display()
            ),
        );
        let Some(mut shell) = Shell::start("/bin/zsh", home.path(), nothing_inherited) else {
            return;
        };
        shell.send(CD_ENCODED_DIR);
        shell.expect("/sub dir/中文\x07");
        shell.send("echo MARK-hooks-$precmd_functions-end\r");
        shell.expect("MARK-hooks-_emit_cwd_hook-end");
        let reports = shell.reports();
        assert!(reports.iter().all(|(_, st)| !st), "{reports:?}");
    }

    #[test]
    fn bash_reports_each_cd_and_loads_the_users_startup_files() {
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join(".bash_profile"),
            "echo MARK-bash_profile\n[ -r ~/.bashrc ] && . ~/.bashrc\n",
        );
        write(
            &home.path().join(".bashrc"),
            "echo MARK-bashrc\nPS1='\\$ '\nPROMPT_COMMAND=\"history -a; $PROMPT_COMMAND\"\n",
        );
        write(&home.path().join(".profile"), "echo MARK-wrong-profile\n");
        let Some(mut shell) = Shell::start("/bin/bash", home.path(), nothing_inherited) else {
            return;
        };
        shell.send(CD_ENCODED_DIR);
        shell.expect(&format!("{ENCODED_DIR}\x1b\\"));
        shell.expect("MARK-bash_profile");
        shell.expect("MARK-bashrc");
        shell.send("false\r");
        shell.send("echo MARK-status-$?-login-$(shopt -q login_shell && echo y)-exported-$(env | grep -c '^PROMPT_COMMAND=')-[$PROMPT_COMMAND]-${RUNNER_BASH_INTEGRATION-unset}\r");
        shell.expect("MARK-status-1-login-y-exported-0-[history -a; __runner_osc7]-unset");
        assert!(!shell.text().contains("MARK-wrong"), "{}", shell.text());
        let reports = shell.reports();
        assert!(
            reports
                .iter()
                .all(|(report, st)| *st && report.starts_with("file:///")),
            "{reports:?}"
        );
    }

    #[test]
    fn bash_removes_its_hook_for_a_helper_emitter() {
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join(".bash_profile"),
            "PS1='\\$ '\n\
             _emit_cwd() { printf '\\e]7;file://%s%s\\a' \"$HOSTNAME\" \"$PWD\"; }\n\
             _prompt_hook() { _emit_cwd; }\n\
             PROMPT_COMMAND=\"_prompt_hook${PROMPT_COMMAND:+; $PROMPT_COMMAND}\"\n",
        );
        let Some(mut shell) = Shell::start("/bin/bash", home.path(), nothing_inherited) else {
            return;
        };
        shell.send(CD_ENCODED_DIR);
        shell.expect("/sub dir/中文\x07");
        shell.send("echo MARK-pc-[$PROMPT_COMMAND]\r");
        shell.expect("MARK-pc-[_prompt_hook]");
        let reports = shell.reports();
        assert!(reports.iter().all(|(_, st)| !st), "{reports:?}");
    }

    #[test]
    fn bash_removes_its_hook_for_an_emitter_added_later() {
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join(".bash_profile"),
            "PS1='\\$ '\n_emit_cwd() { printf '\\e]7;file://%s%s\\a' \"$HOSTNAME\" \"$PWD\"; }\n",
        );
        let Some(mut shell) = Shell::start("/bin/bash", home.path(), nothing_inherited) else {
            return;
        };
        shell.send("PROMPT_COMMAND=\"$PROMPT_COMMAND; _emit_cwd\"\r");
        shell.send(CD_ENCODED_DIR);
        shell.expect("/sub dir/中文\x07");
        shell.send("echo MARK-pc-[$PROMPT_COMMAND]\r");
        shell.expect("MARK-pc-[_emit_cwd]");
        let reports = shell.reports();
        let first_user_report = reports.iter().position(|(_, st)| !st).unwrap();
        assert!(
            reports[first_user_report..].iter().all(|(_, st)| !st),
            "{reports:?}"
        );
    }

    /// Only the prompt string changes here, so this is the prompt-only rescan.
    #[test]
    fn bash_removes_its_hook_when_the_prompt_starts_sending_osc_7() {
        let home = tempfile::tempdir().unwrap();
        write(
            &home.path().join(".bash_profile"),
            "PS1='\\$ '\n_emit_cwd() { printf '\\e]7;file://%s%s\\a' \"$HOSTNAME\" \"$PWD\"; }\n",
        );
        let Some(mut shell) = Shell::start("/bin/bash", home.path(), nothing_inherited) else {
            return;
        };
        shell.send("PS1='$(_emit_cwd)\\$ '\r");
        shell.send(CD_ENCODED_DIR);
        shell.expect("/sub dir/中文\x07");
        shell.send("echo MARK-pc-[$PROMPT_COMMAND]\r");
        shell.expect("MARK-pc-[]");
        let reports = shell.reports();
        let first_user_report = reports.iter().position(|(_, st)| !st).unwrap();
        assert!(
            reports[first_user_report..].iter().all(|(_, st)| !st),
            "{reports:?}"
        );
    }
}
