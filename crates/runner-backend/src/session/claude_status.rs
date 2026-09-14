use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use notify::{RecommendedWatcher, RecursiveMode, Watcher};
use serde::Deserialize;

use crate::error::{Error, Result};

use super::launch::shell_quote;
use super::runtime::RunnerStatus;

pub(crate) const PATH_ENV: &str = "RUNNER_CLAUDE_STATUS_PATH";
pub(crate) const GENERATION_ENV: &str = "RUNNER_CLAUDE_STATUS_GENERATION";
pub(crate) const HOOK_TIMEOUT_SECS: u64 = 2;
pub(crate) const CTRL_C_INTERRUPT: u8 = 1;
pub(crate) const ESCAPE_INTERRUPT: u8 = 2;
const STATUS_DIR: &str = "session-status";

// Runner-owned per-invocation helper follows cmux's hook bridge shape
// (manaflow-ai/cmux, GPL-3.0-or-later); the status records are Runner's.
const APPEND_SCRIPT: &str = r#"#!/bin/sh
notification=
if [ "$2" = Notification ]; then
    kind=$(sed -n 's/.*"notification_type"[[:space:]]*:[[:space:]]*"\([a-z_]*\)".*/\1/p')
    case "$kind" in
        ''|*[!a-z_]*) ;;
        *) notification=",\"notification_type\":\"$kind\"" ;;
    esac
fi
printf '{"generation":"%s","hook_event_name":"%s"%s}\n' "$RUNNER_CLAUDE_STATUS_GENERATION" "$2" "$notification" >> "$1" 2>/dev/null
if [ "$2" != Notification ]; then
    cat >/dev/null
fi
exit 0
"#;

pub(crate) fn status_path(app_data_dir: &Path, session_id: &str) -> PathBuf {
    app_data_dir
        .join(STATUS_DIR)
        .join(format!("{session_id}.ndjson"))
}

pub(crate) fn clear_leftovers(app_data_dir: &Path) -> Result<()> {
    let entries = match fs::read_dir(app_data_dir.join(STATUS_DIR)) {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(error) => return Err(error.into()),
    };
    for entry in entries {
        let path = match entry {
            Ok(entry) => entry.path(),
            Err(error) => {
                log::warn!("read stale Claude status entry: {error}");
                continue;
            }
        };
        if matches!(
            path.extension().and_then(|extension| extension.to_str()),
            Some("ndjson" | "sh" | "tmp")
        ) {
            if let Err(error) = fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!(
                        "remove stale Claude status file {}: {error}",
                        path.display()
                    );
                }
            }
        }
    }
    Ok(())
}

pub(crate) fn hook_command(path: &Path, event: &str) -> String {
    // Only the event and notification type enter the log; stdin is always drained.
    // One small printf append avoids interleaving records from parallel tool hooks.
    format!(
        "(sh {} {} {} || cat >/dev/null) 2>/dev/null; exit 0",
        shell_quote(&script_path(path).to_string_lossy()),
        shell_quote(&path.to_string_lossy()),
        shell_quote(event),
    )
}

fn script_path(path: &Path) -> PathBuf {
    path.with_extension("sh")
}

struct StatusFiles(PathBuf);

impl Drop for StatusFiles {
    fn drop(&mut self) {
        for path in [
            script_path(&self.0),
            self.0.with_extension("sh.tmp"),
            self.0.clone(),
        ] {
            if let Err(error) = fs::remove_file(&path) {
                if error.kind() != std::io::ErrorKind::NotFound {
                    log::warn!("remove Claude status file {}: {error}", path.display());
                }
            }
        }
    }
}

#[derive(Deserialize)]
struct StatusReport {
    generation: String,
    hook_event_name: String,
    #[serde(default)]
    notification_type: Option<String>,
}

fn parse_transition(line: &[u8], generation: &str) -> Option<RunnerStatus> {
    let report: StatusReport = serde_json::from_slice(line).ok()?;
    if report.generation != generation {
        return None;
    }
    match report.hook_event_name.as_str() {
        "UserPromptSubmit" | "PreToolUse" | "PostToolUse" => Some(RunnerStatus::Busy),
        "Stop" | "StopFailure" => Some(RunnerStatus::Idle),
        "Notification" if report.notification_type.as_deref() == Some("idle_prompt") => {
            Some(RunnerStatus::Idle)
        }
        _ => None,
    }
}

pub(crate) struct ClaudeStatusWatcher {
    reader: BufReader<File>,
    pending: Vec<u8>,
    generation: String,
    dirty: Arc<AtomicBool>,
    interrupt: Arc<AtomicU8>,
    last_read: Instant,
    _watcher: RecommendedWatcher,
    // Drop the reader and watcher before deleting their files, including on Windows.
    _files: StatusFiles,
}

impl ClaudeStatusWatcher {
    pub(crate) fn start(path: &Path, generation: String) -> Result<Self> {
        fs::create_dir_all(path.parent().expect("status file has a parent"))?;
        let files = StatusFiles(path.to_owned());
        let script = script_path(path);
        let temporary = path.with_extension("sh.tmp");
        fs::write(&temporary, APPEND_SCRIPT)?;
        fs::rename(temporary, script)?;
        let file = OpenOptions::new()
            .read(true)
            .write(true)
            .create(true)
            .truncate(true)
            .open(path)?;
        let dirty = Arc::new(AtomicBool::new(true));
        let dirty_for_watch = Arc::clone(&dirty);
        let mut watcher = notify::recommended_watcher(move |event| {
            if let Err(error) = event {
                log::warn!("Claude status notify: {error}");
            }
            dirty_for_watch.store(true, Ordering::Release);
        })
        .map_err(|error| Error::msg(format!("Claude status watcher: {error}")))?;
        watcher
            .watch(path, RecursiveMode::NonRecursive)
            .map_err(|error| {
                Error::msg(format!("watch Claude status {}: {error}", path.display()))
            })?;
        Ok(Self {
            reader: BufReader::new(file),
            pending: Vec::new(),
            generation,
            dirty,
            interrupt: Arc::new(AtomicU8::new(0)),
            last_read: Instant::now(),
            _watcher: watcher,
            _files: files,
        })
    }

    pub(crate) fn interrupt_signal(&self) -> Arc<AtomicU8> {
        Arc::clone(&self.interrupt)
    }

    pub(crate) fn drain(
        &mut self,
        mut transition: impl FnMut(RunnerStatus, &'static str),
    ) -> Result<()> {
        let interrupted = self.interrupt.swap(0, Ordering::AcqRel);
        if !self.dirty.swap(false, Ordering::AcqRel)
            && self.last_read.elapsed() < Duration::from_secs(1)
            && interrupted == 0
        {
            return Ok(());
        }
        self.last_read = Instant::now();
        while self.reader.read_until(b'\n', &mut self.pending)? != 0 {
            if self.pending.last() != Some(&b'\n') {
                break;
            }
            if let Some(state) = parse_transition(&self.pending, &self.generation) {
                transition(state, "hook");
            }
            self.pending.clear();
        }
        // Keep buffered tool hooks ahead of the interrupt on the same output channel.
        if interrupted != 0 {
            let source = if interrupted & CTRL_C_INTERRUPT != 0 {
                "input-interrupt"
            } else {
                "input-escape"
            };
            transition(RunnerStatus::Idle, source);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;
    #[cfg(unix)]
    use std::process::{Command, Stdio};

    use super::*;

    fn report(event: &str, notification: Option<&str>, generation: &str) -> String {
        serde_json::json!({
            "generation": generation,
            "hook_event_name": event,
            "notification_type": notification,
        })
        .to_string()
    }

    #[cfg(unix)]
    fn run_hook(path: &Path, event: &str, payload: &[u8]) {
        let mut child = Command::new("/bin/sh")
            .args(["-c", &hook_command(path, event)])
            .env(GENERATION_ENV, "current")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child.stdin.take().unwrap().write_all(payload).unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(output.status.success(), "{event}: {output:?}");
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }

    #[test]
    fn maps_turn_boundaries_work_and_real_idle_notifications() {
        for (event, notification, expected) in [
            ("UserPromptSubmit", None, Some(RunnerStatus::Busy)),
            ("PreToolUse", None, Some(RunnerStatus::Busy)),
            ("PostToolUse", None, Some(RunnerStatus::Busy)),
            (
                "Notification",
                Some("idle_prompt"),
                Some(RunnerStatus::Idle),
            ),
            ("Notification", Some("permission_prompt"), None),
            ("Notification", Some("agent_completed"), None),
            ("Notification", None, None),
            ("Stop", None, Some(RunnerStatus::Idle)),
            ("StopFailure", None, Some(RunnerStatus::Idle)),
            ("SubagentStop", None, None),
            ("SessionStart", None, None),
            ("unknown", None, None),
        ] {
            assert_eq!(
                parse_transition(report(event, notification, "current").as_bytes(), "current"),
                expected,
                "{event} {notification:?}"
            );
        }
        assert_eq!(parse_transition(b"not JSON\n", "current"), None);
        assert_eq!(
            parse_transition(
                report("Notification", Some("idle_prompt"), "old").as_bytes(),
                "current"
            ),
            None
        );
    }

    #[test]
    fn watcher_preserves_partial_lines_and_skips_stale_or_invalid_records() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "session");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            format!(
                "{}\n",
                report("Notification", Some("idle_prompt"), "current")
            ),
        )
        .unwrap();
        let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
        let mut file = OpenOptions::new().append(true).open(&path).unwrap();
        let prompt = report("UserPromptSubmit", None, "current");
        write!(file, "{}", &prompt[..10]).unwrap();
        let mut states = Vec::new();
        watcher.drain(|state, _| states.push(state)).unwrap();
        assert!(states.is_empty());

        writeln!(file, "{}", &prompt[10..]).unwrap();
        writeln!(file, "broken").unwrap();
        writeln!(
            file,
            "{}",
            report("Notification", Some("idle_prompt"), "old")
        )
        .unwrap();
        writeln!(file, "{}", report("Stop", None, "current")).unwrap();
        writeln!(
            file,
            "{}",
            report("Notification", Some("idle_prompt"), "current")
        )
        .unwrap();
        watcher.dirty.store(true, Ordering::Release);
        watcher.drain(|state, _| states.push(state)).unwrap();
        assert_eq!(
            states,
            [RunnerStatus::Busy, RunnerStatus::Idle, RunnerStatus::Idle]
        );
        watcher.dirty.store(true, Ordering::Release);
        watcher.drain(|state, _| states.push(state)).unwrap();
        assert_eq!(states.len(), 3);
        drop(file);
        drop(watcher);
        assert!(!path.exists());
        assert!(!script_path(&path).exists());
    }

    #[test]
    fn interrupt_drains_buffered_tool_records_before_idle_and_allows_new_work() {
        for (kind, source) in [
            (CTRL_C_INTERRUPT, "input-interrupt"),
            (ESCAPE_INTERRUPT, "input-escape"),
            (CTRL_C_INTERRUPT | ESCAPE_INTERRUPT, "input-interrupt"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let path = status_path(root.path(), "interrupt");
            let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
            watcher.drain(|_, _| panic!("no records yet")).unwrap();
            let mut file = OpenOptions::new().append(true).open(&path).unwrap();
            writeln!(file, "{}", report("PreToolUse", None, "current")).unwrap();
            watcher.dirty.store(false, Ordering::Release);
            watcher.interrupt_signal().store(kind, Ordering::Release);
            let mut transitions = Vec::new();
            watcher
                .drain(|state, source| transitions.push((state, source)))
                .unwrap();
            assert_eq!(
                transitions,
                [(RunnerStatus::Busy, "hook"), (RunnerStatus::Idle, source)]
            );
            writeln!(file, "{}", report("PreToolUse", None, "current")).unwrap();
            watcher.dirty.store(true, Ordering::Release);
            watcher
                .drain(|state, source| transitions.push((state, source)))
                .unwrap();
            assert_eq!(transitions.last(), Some(&(RunnerStatus::Busy, "hook")));
            assert_eq!(transitions.len(), 3);
        }
    }

    #[test]
    #[cfg(unix)]
    fn notification_script_uses_payload_type_even_when_matcher_is_bypassed() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "notifications");
        let _watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
        for kind in [
            None,
            Some(""),
            Some("permission_prompt"),
            Some("agent_completed"),
            Some("idle_prompt"),
        ] {
            let mut payload = serde_json::json!({
                "hook_event_name": "Notification",
                "message": "Do not trust embedded \"notification_type\":\"idle_prompt\" text",
            });
            if let Some(kind) = kind {
                payload["notification_type"] = serde_json::json!(kind);
            }
            for text in [
                serde_json::to_string(&payload).unwrap(),
                serde_json::to_string_pretty(&payload).unwrap(),
            ] {
                run_hook(&path, "Notification", text.as_bytes());
                let records = fs::read_to_string(&path).unwrap();
                let line = records.lines().last().unwrap();
                let report: StatusReport = serde_json::from_str(line).unwrap();
                assert_eq!(
                    report.notification_type.as_deref(),
                    kind.filter(|kind| !kind.is_empty())
                );
                assert_eq!(
                    parse_transition(line.as_bytes(), "current"),
                    (kind == Some("idle_prompt")).then_some(RunnerStatus::Idle),
                );
            }
        }
    }

    #[test]
    #[cfg(unix)]
    fn stop_followed_by_pre_tool_use_ends_busy() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "continuation");
        let mut watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
        for event in ["Stop", "PreToolUse"] {
            run_hook(&path, event, b"{}");
        }
        let mut states = Vec::new();
        watcher.drain(|state, _| states.push(state)).unwrap();
        assert_eq!(states, [RunnerStatus::Idle, RunnerStatus::Busy]);
    }

    #[test]
    #[cfg(unix)]
    fn tool_hooks_drain_payloads_larger_than_the_pipe_buffer() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "large-payload");
        let _watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
        let payload = serde_json::json!({"tool_response": "x".repeat(2 * 1024 * 1024)}).to_string();
        for event in [
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Stop",
            "StopFailure",
        ] {
            run_hook(&path, event, payload.as_bytes());
        }
        assert_eq!(fs::read_to_string(&path).unwrap().lines().count(), 5);
        // A failed bridge setup leaves no helper; the command must still consume stdin.
        run_hook(
            &status_path(root.path(), "missing-helper"),
            "PostToolUse",
            payload.as_bytes(),
        );
    }

    #[test]
    fn startup_clears_status_files_left_by_a_crash() {
        let root = tempfile::tempdir().unwrap();
        clear_leftovers(root.path()).unwrap();
        let path = status_path(root.path(), "stale");
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, b"stale report").unwrap();
        fs::write(script_path(&path), APPEND_SCRIPT).unwrap();
        fs::write(path.with_extension("sh.tmp"), APPEND_SCRIPT).unwrap();
        let unremovable = path.with_file_name("directory.sh");
        fs::create_dir(&unremovable).unwrap();
        clear_leftovers(root.path()).unwrap();
        assert!(unremovable.is_dir());
        assert!(!path.exists());
        assert!(!script_path(&path).exists());
        assert!(!path.with_extension("sh.tmp").exists());
    }

    #[test]
    fn failed_setup_removes_its_helper() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(root.path(), "bad-file");
        fs::create_dir_all(&path).unwrap();
        assert!(ClaudeStatusWatcher::start(&path, "current".into()).is_err());
        assert!(!script_path(&path).exists());
        assert!(!path.with_extension("sh.tmp").exists());
    }

    #[test]
    #[cfg(unix)]
    fn injected_commands_append_silently_and_fail_open() {
        let root = tempfile::tempdir().unwrap();
        let path = status_path(&root.path().join("Jason's status $dir"), "session");
        let _watcher = ClaudeStatusWatcher::start(&path, "current".into()).unwrap();
        for event in [
            "UserPromptSubmit",
            "PreToolUse",
            "PostToolUse",
            "Notification",
            "Stop",
            "StopFailure",
        ] {
            run_hook(&path, event, br#"{"notification_type":"idle_prompt"}"#);
        }
        let records = fs::read_to_string(&path).unwrap();
        let states: Vec<_> = records
            .lines()
            .filter_map(|line| parse_transition(line.as_bytes(), "current"))
            .collect();
        assert_eq!(records.lines().count(), 6);
        assert_eq!(
            states,
            [
                RunnerStatus::Busy,
                RunnerStatus::Busy,
                RunnerStatus::Busy,
                RunnerStatus::Idle,
                RunnerStatus::Idle,
                RunnerStatus::Idle,
            ]
        );

        let output = std::process::Command::new("/bin/sh")
            .args(["-c", &hook_command(root.path(), "Stop")])
            .env(GENERATION_ENV, "current")
            .output()
            .unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}
