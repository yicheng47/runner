// I2 — `runner` CLI ↔ event log roundtrip.
// Mirrors docs/tests/archive/v0-mvp-tests.md `## I2`.
//
// Each test sets up a tempdir mission directory, seeds the roster
// sidecar (the part `mission_start` still writes), spawns the
// just-built `runner` binary with the four `RUNNER_*` env vars
// pointing at it, and asserts on the resulting NDJSON.

use std::path::{Path, PathBuf};
use std::process::Command;

use runner_core::event_log::EventLog;
use runner_core::model::EventKind;

/// Locate the agent CLI binary. Its source-side name remains
/// `runner-agent-cli`; at runtime, `cli_install` renames it to `runner`
/// when copying into
/// `$APPDATA/runner/bin/`. Integration tests invoke the source artifact
/// directly via `CARGO_BIN_EXE_runner-agent-cli`.
fn runner_bin() -> PathBuf {
    PathBuf::from(env!("CARGO_BIN_EXE_runner-agent-cli"))
}

struct Fixture {
    _root: tempfile::TempDir,
    mission_dir: PathBuf,
    events_log: PathBuf,
}

impl Fixture {
    fn new(crew_id: &str, mission_id: &str) -> Self {
        let root = tempfile::tempdir().unwrap();
        let crew_dir = root.path().join("crews").join(crew_id);
        let mission_dir = crew_dir.join("missions").join(mission_id);
        std::fs::create_dir_all(&mission_dir).unwrap();
        let events_log = mission_dir.join("events.ndjson");
        Self {
            _root: root,
            mission_dir,
            events_log,
        }
    }

    fn write_roster(&self, members: &[(&str, bool)]) {
        let entries: Vec<serde_json::Value> = members
            .iter()
            .enumerate()
            .map(|(position, (h, lead))| serde_json::json!({ "id": h, "role_id": h, "position": position, "handle": h, "lead": lead }))
            .collect();
        std::fs::write(
            self.mission_dir.join("roster.json"),
            serde_json::to_vec(&serde_json::json!({"version": 2, "crew_name": "Pair", "crew_addendum": null, "slots": entries})).unwrap(),
        )
        .unwrap();
    }

    fn cmd(&self, handle: &str, args: &[&str]) -> Command {
        let mut cmd = Command::new(runner_bin());
        cmd.args(args);
        cmd.env("RUNNER_CREW_ID", "C");
        cmd.env("RUNNER_MISSION_ID", "M");
        cmd.env("RUNNER_HANDLE", handle);
        cmd.env("RUNNER_EVENT_LOG", &self.events_log);
        cmd
    }

    fn read_log(&self) -> Vec<runner_core::model::Event> {
        if !self.events_log.exists() {
            return vec![];
        }
        let log = EventLog::open(self.events_log.parent().unwrap()).unwrap();
        log.read_from(0)
            .unwrap()
            .into_iter()
            .map(|e| e.event)
            .collect()
    }

    fn line_count(&self) -> usize {
        std::fs::read_to_string(&self.events_log)
            .map(|s| s.lines().filter(|l| !l.is_empty()).count())
            .unwrap_or(0)
    }
}

#[test]
fn i2_1_signal_appends_one_line_with_correct_envelope() {
    let f = Fixture::new("C", "M");

    let out = f
        .cmd(
            "impl",
            &["signal", "mission_goal", "--payload", r#"{"text":"go"}"#],
        )
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let events = f.read_log();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert!(matches!(ev.kind, EventKind::Signal));
    assert_eq!(ev.from, "impl");
    assert!(ev.recipients().is_empty());
    assert_eq!(ev.signal_type.as_ref().unwrap().as_str(), "mission_goal");
    assert_eq!(ev.payload["text"], "go");
}

#[test]
fn ask_shortcuts_append_directly_inside_a_mission() {
    let f = Fixture::new("C", "M");

    let lead = f
        .cmd("impl", &["ask", "Ship?", "--context", "Checks pass"])
        .output()
        .unwrap();
    assert_eq!(lead.status.code(), Some(2));
    assert!(
        String::from_utf8_lossy(&lead.stderr).contains(runner_core::model::ASK_LEAD_REMOVED_HINT)
    );
    let human = f
        .cmd("impl", &["ask", "--human", "Ship?", "--choices", "yes,no"])
        .output()
        .unwrap();
    assert!(human.status.success());

    let events = f.read_log();
    assert_eq!(events.len(), 1);
    assert_eq!(
        events[0].signal_type.as_ref().unwrap().as_str(),
        "ask_human"
    );
    assert_eq!(
        events[0].payload,
        serde_json::json!({"prompt": "Ship?", "choices": ["yes", "no"]})
    );
    let removed = f.cmd("impl", &["signal", "ask_lead"]).output().unwrap();
    assert_eq!(removed.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&removed.stderr)
        .contains(runner_core::model::ASK_LEAD_REMOVED_HINT));
}

#[test]
fn i2_2_signal_rejects_unknown_type_and_does_not_append() {
    let f = Fixture::new("C", "M");

    let out = f
        .cmd("impl", &["signal", "not_a_real_type"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("unknown type") && stderr.contains("Known types"),
        "stderr should name the unknown type and list known ones; got: {stderr}",
    );
    assert_eq!(f.line_count(), 0);
}

#[test]
fn i2_3_msg_post_to_handle_routes_directed() {
    let f = Fixture::new("C", "M");
    f.write_roster(&[("lead", true), ("impl", false)]);

    let out = f
        .cmd("lead", &["msg", "post", "ready for review", "--to", "impl"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let events = f.read_log();
    assert_eq!(events.len(), 1);
    let ev = &events[0];
    assert!(matches!(ev.kind, EventKind::Message));
    assert_eq!(ev.from, "lead");
    assert_eq!(ev.recipients().first().map(String::as_str), Some("impl"));
    assert_eq!(ev.payload["text"], "ready for review");
}

#[test]
fn i2_4_msg_post_to_unknown_handle_is_rejected() {
    let f = Fixture::new("C", "M");
    f.write_roster(&[("lead", true), ("impl", false)]);

    let out = f
        .cmd("lead", &["msg", "post", "hi", "--to", "ghost"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("@ghost") || stderr.contains("roster"),
        "stderr should mention the unknown handle; got: {stderr}",
    );
    assert_eq!(f.line_count(), 0);
}

#[test]
fn msg_post_rejects_over_cap_for_broadcast_and_directed() {
    const MESSAGE_LIMIT_BYTES: usize = 32 * 1024;

    let f = Fixture::new("C", "M");
    f.write_roster(&[("lead", true), ("impl", false)]);
    #[cfg(unix)]
    let oversized = "x".repeat(MESSAGE_LIMIT_BYTES + 1);
    #[cfg(windows)]
    let oversized = "€".repeat((MESSAGE_LIMIT_BYTES + 1) / 3);

    for recipient in [None, Some("impl")] {
        let mut args = vec!["msg", "post", oversized.as_str()];
        if let Some(handle) = recipient {
            args.extend(["--to", handle]);
        }

        let out = f.cmd("lead", &args).output().unwrap();
        assert_eq!(out.status.code(), Some(1));
        let stderr = String::from_utf8_lossy(&out.stderr);
        assert_eq!(
            stderr.trim(),
            "runner msg post: message is 33KB, limit is 32KB. \
             Write large content to a file and reference its path instead."
        );
    }

    assert_eq!(f.line_count(), 0);
}

#[test]
fn msg_post_accepts_messages_at_and_under_cap() {
    const MESSAGE_LIMIT_BYTES: usize = 32 * 1024;

    let f = Fixture::new("C", "M");
    f.write_roster(&[("lead", true), ("impl", false)]);
    let at_limit = "é".repeat(MESSAGE_LIMIT_BYTES / "é".len());
    #[cfg(unix)]
    let under_limit = "x".repeat(MESSAGE_LIMIT_BYTES - 1);
    #[cfg(windows)]
    let under_limit = format!("{}x", "é".repeat(MESSAGE_LIMIT_BYTES / 2 - 1));
    assert_eq!(at_limit.len(), MESSAGE_LIMIT_BYTES);

    let broadcast = f
        .cmd("lead", &["msg", "post", at_limit.as_str()])
        .output()
        .unwrap();
    assert!(
        broadcast.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&broadcast.stderr)
    );

    let directed = f
        .cmd(
            "lead",
            &["msg", "post", under_limit.as_str(), "--to", "impl"],
        )
        .output()
        .unwrap();
    assert!(
        directed.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&directed.stderr)
    );

    let events = f.read_log();
    assert_eq!(events.len(), 2);
    assert!(events[0].recipients().is_empty());
    assert_eq!(events[0].payload["text"].as_str().unwrap(), at_limit);
    assert_eq!(
        events[1].recipients().first().map(String::as_str),
        Some("impl")
    );
    assert_eq!(events[1].payload["text"].as_str().unwrap(), under_limit);
}

#[test]
fn msg_post_to_human_is_rejected_with_channel_guidance() {
    let f = Fixture::new("C", "M");
    f.write_roster(&[("lead", true), ("impl", false)]);

    let out = f
        .cmd("lead", &["msg", "post", "hi", "--to", "human"])
        .output()
        .unwrap();
    assert!(!out.status.success());
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("@human is not a message recipient")
            && stderr.contains("valid handles: @lead, @impl"),
        "stderr should teach the agent to broadcast into the channel; got: {stderr}",
    );
    assert_eq!(f.line_count(), 0);
}

#[test]
fn i2_5_msg_read_prints_inbox_in_order_and_emits_inbox_read() {
    let f = Fixture::new("C", "M");
    f.write_roster(&[("lead", true), ("impl", false)]);

    // Pre-populate two directed messages to @impl from @lead with the
    // caller's own broadcast between them.
    let log = EventLog::open(&f.mission_dir).unwrap();
    use runner_core::model::EventDraft;
    let m1 = log
        .append(EventDraft::message(
            Some("C"),
            "M",
            "lead",
            Some("impl".into()),
            "first",
        ))
        .unwrap();
    log.append(EventDraft::message(
        Some("C"),
        "M",
        "impl",
        None,
        "own broadcast must be excluded",
    ))
    .unwrap();
    let m2 = log
        .append(EventDraft::message(
            Some("C"),
            "M",
            "lead",
            Some("impl".into()),
            "second",
        ))
        .unwrap();

    let out = f.cmd("impl", &["msg", "read"]).output().unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(!stdout.contains("own broadcast must be excluded"));
    let first_pos = stdout.find("first").expect("first message in stdout");
    let second_pos = stdout.find("second").expect("second message in stdout");
    assert!(
        first_pos < second_pos,
        "messages must print in append order"
    );

    // The CLI appended one inbox_read with up_to = m2.id.
    let events = f.read_log();
    let last = events.last().unwrap();
    assert_eq!(last.signal_type.as_ref().unwrap().as_str(), "inbox_read");
    assert_eq!(last.from, "impl");
    assert_eq!(last.payload["up_to"], m2.id);
    assert_ne!(last.payload["up_to"], m1.id);
}

#[test]
fn i2_5c_msg_read_with_from_filter_does_not_emit_inbox_read() {
    // Regression: `--from` only filters which messages are *printed*,
    // but `inbox_read` advances a global per-handle watermark in the
    // bus. If the CLI emitted inbox_read on filtered reads, every
    // message with id ≤ the printed last would be marked read —
    // including unread messages from other senders that the user
    // never saw. The CLI must suppress watermark emission on filtered
    // reads.
    let f = Fixture::new("C", "M");
    f.write_roster(&[("lead", true), ("impl", false), ("reviewer", false)]);

    let log = EventLog::open(&f.mission_dir).unwrap();
    use runner_core::model::EventDraft;
    log.append(EventDraft::message(
        Some("C"),
        "M",
        "lead",
        Some("impl".into()),
        "from lead first",
    ))
    .unwrap();
    log.append(EventDraft::message(
        Some("C"),
        "M",
        "reviewer",
        Some("impl".into()),
        "from reviewer (must stay unread)",
    ))
    .unwrap();
    log.append(EventDraft::message(
        Some("C"),
        "M",
        "lead",
        Some("impl".into()),
        "from lead second",
    ))
    .unwrap();

    let out = f
        .cmd("impl", &["msg", "read", "--from", "lead"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "stderr: {}",
        String::from_utf8_lossy(&out.stderr)
    );

    // Critical: no inbox_read appended. The log must still contain
    // exactly the three pre-seeded messages.
    let signals: Vec<_> = f
        .read_log()
        .into_iter()
        .filter(|e| matches!(e.kind, EventKind::Signal))
        .collect();
    assert!(
        !signals
            .iter()
            .any(|s| s.signal_type.as_ref().map(|t| t.as_str()) == Some("inbox_read")),
        "filtered read must NOT emit inbox_read; signals: {signals:?}",
    );
}

#[test]
fn filtered_reads_report_unread_on_stderr_until_inbox_is_consumed() {
    let f = Fixture::new("C", "M");
    let log = EventLog::open(&f.mission_dir).unwrap();
    use runner_core::model::EventDraft;
    log.append(EventDraft::message(
        Some("C"),
        "M",
        "lead",
        Some("impl".into()),
        "first",
    ))
    .unwrap();
    log.append(EventDraft::message(
        Some("C"),
        "M",
        "reviewer",
        Some("other".into()),
        "other inbox",
    ))
    .unwrap();
    log.append(EventDraft::message(
        Some("C"),
        "M",
        "impl",
        None,
        "own broadcast",
    ))
    .unwrap();
    let last = log
        .append(EventDraft::message(
            Some("C"),
            "M",
            "reviewer",
            None,
            "second",
        ))
        .unwrap();
    let initial_count = f.line_count();
    let filters = [
        vec!["--from", "lead"],
        vec!["--since", last.id.as_str()],
        vec!["--from", "nobody"],
        vec!["--since", last.id.as_str(), "--from", "lead"],
    ];
    for all in [false, true] {
        for filter in &filters {
            let mut args = vec!["msg", "read"];
            if all {
                args.push("--all");
            }
            args.extend(filter.iter().copied());
            let out = f.cmd("impl", &args).output().unwrap();
            assert!(out.status.success());
            let stdout = String::from_utf8_lossy(&out.stdout);
            let stderr = String::from_utf8_lossy(&out.stderr);
            assert!(stderr.contains("filtered reads do not mark messages read"));
            assert!(stderr.contains("unread inbox messages: 2"));
            assert!(stderr.contains("`runner msg read` without --since or --from"));
            assert!(!stdout.contains("unread inbox messages:"));
            if filter.contains(&"--since") || filter.contains(&"nobody") {
                assert!(stdout.is_empty());
            }
            assert_eq!(f.line_count(), initial_count);
        }
    }

    let consumed = f.cmd("impl", &["msg", "read"]).output().unwrap();
    assert!(consumed.status.success());
    assert!(consumed.stderr.is_empty());
    let events = f.read_log();
    assert_eq!(events.last().unwrap().payload["up_to"], last.id);
    let history = f
        .cmd("impl", &["msg", "read", "--all", "--from", "lead"])
        .output()
        .unwrap();
    assert!(history.status.success());
    assert!(String::from_utf8_lossy(&history.stdout).contains("first"));
    assert!(history.stderr.is_empty());
    assert_eq!(f.line_count(), initial_count + 1);
}

#[test]
fn i2_5b_msg_read_with_empty_inbox_does_not_emit_inbox_read() {
    let f = Fixture::new("C", "M");
    f.write_roster(&[("lead", true), ("impl", false)]);

    let out = f.cmd("impl", &["msg", "read"]).output().unwrap();
    assert!(out.status.success());
    assert_eq!(f.line_count(), 0, "empty inbox must not emit inbox_read");
}

#[test]
fn status_busy_idle_alias_is_removed_without_appending() {
    let f = Fixture::new("C", "M");

    let out = f
        .cmd("impl", &["status", "idle", "--note", "ready for next task"])
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert_eq!(f.line_count(), 0);
}

#[test]
fn i2_6b_status_rejects_unknown_state() {
    let f = Fixture::new("C", "M");

    let out = f.cmd("impl", &["status", "sleeping"]).output().unwrap();
    assert!(!out.status.success());
    assert_eq!(f.line_count(), 0);
}

#[test]
fn i2_7_concurrent_writers_interleave_atomically() {
    let f = Fixture::new("C", "M");

    // 5 workers × 20 invocations = 100 lines. Cut down from the spec's
    // 10×100 because we're spawning real processes; 100 is enough to
    // exercise the flock contract without burning CI minutes on
    // process spawn overhead.
    let workers = 5usize;
    let per_worker = 20usize;
    let bin = runner_bin();
    let log = f.events_log.clone();
    let handles: Vec<_> = (0..workers)
        .map(|w| {
            let bin = bin.clone();
            let log = log.clone();
            std::thread::spawn(move || {
                for _ in 0..per_worker {
                    let status = Command::new(&bin)
                        .args(["signal", "human_said", "--payload", r#"{"text":"x"}"#])
                        .env("RUNNER_CREW_ID", "C")
                        .env("RUNNER_MISSION_ID", "M")
                        .env("RUNNER_HANDLE", format!("worker{w}"))
                        .env("RUNNER_EVENT_LOG", &log)
                        .status()
                        .unwrap();
                    assert!(status.success(), "worker {w} signal failed");
                }
            })
        })
        .collect();
    for h in handles {
        h.join().unwrap();
    }

    let total = workers * per_worker;
    assert_eq!(
        f.line_count(),
        total,
        "every invocation must produce one line"
    );
    let events = f.read_log();
    assert_eq!(
        events.len(),
        total,
        "every line must parse as a complete event"
    );

    // ULIDs strictly monotonic — the EventLog flock contract.
    for w in events.windows(2) {
        assert!(
            w[0].id.as_bytes() < w[1].id.as_bytes(),
            "ULIDs must be strictly monotonic: {} ≥ {}",
            w[0].id,
            w[1].id,
        );
    }
}

#[test]
fn i2_8_partial_env_fails_fast_with_pointer() {
    // RUNNER_EVENT_LOG missing while the other three are set should
    // produce a non-zero exit and a stderr message naming the missing
    // var. Off-bus (none-set) is a separate test below.
    let f = Fixture::new("C", "M");

    let mut cmd = Command::new(runner_bin());
    cmd.args(["signal", "mission_goal"]);
    cmd.env("RUNNER_CREW_ID", "C");
    cmd.env("RUNNER_MISSION_ID", "M");
    cmd.env("RUNNER_HANDLE", "impl");
    // Deliberately do not set RUNNER_EVENT_LOG; also unset any inherited
    // one to avoid cross-test bleed-through.
    cmd.env_remove("RUNNER_EVENT_LOG");

    let out = cmd.output().unwrap();
    assert!(!out.status.success(), "expected non-zero exit");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("RUNNER_EVENT_LOG"),
        "stderr should name the missing env var; got: {stderr}",
    );
    assert_eq!(f.line_count(), 0);
}

#[test]
fn off_bus_mission_write_requires_mission() {
    let mut cmd = Command::new(runner_bin());
    cmd.args(["msg", "post", "hello"]);
    cmd.env_remove("RUNNER_CREW_ID");
    cmd.env_remove("RUNNER_MISSION_ID");
    cmd.env_remove("RUNNER_HANDLE");
    cmd.env_remove("RUNNER_EVENT_LOG");

    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(
        stderr.contains("--mission"),
        "stderr should name the required flag; got: {stderr}",
    );
}

#[test]
fn local_usage_errors_win_over_a_missing_app() {
    let mut cmd = Command::new(runner_bin());
    cmd.args(["role", "create", "coder"]);
    cmd.env_remove("RUNNER_CREW_ID");
    cmd.env_remove("RUNNER_MISSION_ID");
    cmd.env_remove("RUNNER_HANDLE");
    cmd.env_remove("RUNNER_EVENT_LOG");

    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(2));
    assert!(String::from_utf8_lossy(&out.stderr).contains("--runtime is required"));
}

#[test]
fn role_runtime_shell_is_a_usage_error() {
    for args in [
        ["role", "create", "smoke-shell", "--runtime", "shell"],
        ["role", "update", "smoke-shell", "--runtime", "shell"],
    ] {
        let mut cmd = Command::new(runner_bin());
        cmd.args(args);
        cmd.env_remove("RUNNER_CREW_ID");
        cmd.env_remove("RUNNER_MISSION_ID");
        cmd.env_remove("RUNNER_HANDLE");
        cmd.env_remove("RUNNER_EVENT_LOG");

        let out = cmd.output().unwrap();
        assert_eq!(out.status.code(), Some(2), "{args:?}");
        assert_eq!(
            String::from_utf8_lossy(&out.stderr),
            "unknown role runtime \"shell\"; expected codex, claude-code, antigravity, pi, copilot, trae, cursor, or grok\n",
            "{args:?}"
        );
    }
}

#[cfg(unix)]
#[test]
fn health_status_reports_not_running_with_exit_three() {
    let home = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(runner_bin());
    cmd.arg("status");
    cmd.env("HOME", home.path());
    cmd.env("XDG_DATA_HOME", home.path());
    cmd.env_remove("RUNNER_CREW_ID");
    cmd.env_remove("RUNNER_MISSION_ID");
    cmd.env_remove("RUNNER_HANDLE");
    cmd.env_remove("RUNNER_EVENT_LOG");

    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!("{}\n", runner_cli::client::NOT_RUNNING_MESSAGE)
    );
}

#[cfg(unix)]
#[test]
fn direct_chat_handle_only_reaches_not_running_path() {
    let home = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(runner_bin());
    cmd.arg("status");
    cmd.env("HOME", home.path());
    cmd.env("XDG_DATA_HOME", home.path());
    cmd.env_remove("RUNNER_CREW_ID");
    cmd.env_remove("RUNNER_MISSION_ID");
    cmd.env("RUNNER_HANDLE", "coder");
    cmd.env_remove("RUNNER_EVENT_LOG");

    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(3));
    assert_eq!(
        String::from_utf8_lossy(&out.stderr),
        format!("{}\n", runner_cli::client::NOT_RUNNING_MESSAGE)
    );
}

#[test]
fn help_works_without_env_vars() {
    let mut cmd = Command::new(runner_bin());
    cmd.args(["help"]);
    cmd.env_remove("RUNNER_CREW_ID");
    cmd.env_remove("RUNNER_MISSION_ID");
    cmd.env_remove("RUNNER_HANDLE");
    cmd.env_remove("RUNNER_EVENT_LOG");

    let out = cmd.output().unwrap();
    assert!(out.status.success());
    let stdout = String::from_utf8_lossy(&out.stdout);
    assert!(stdout.contains("USAGE"));
    assert!(stdout.contains("runner signal"));
    assert!(stdout.contains("runner msg"));
    assert!(!stdout.contains("status busy"));
}

#[test]
fn command_help_describes_verbs_and_flags() {
    let cases = [
        (
            vec!["project", "--help"],
            vec!["List projects", "Show one project", "--json", "-q"],
        ),
        (
            vec!["mission", "start", "--help"],
            vec![
                "--goal-file",
                "Read the mission goal from a file",
                "--cwd",
                "defaults to the current directory",
            ],
        ),
        (
            vec!["msg", "post", "--help"],
            vec![
                "--mission",
                "required outside a mission",
                "--as",
                "roster handle",
            ],
        ),
    ];
    for (args, expected) in cases {
        let mut cmd = Command::new(runner_bin());
        cmd.args(args);
        let out = cmd.output().unwrap();
        assert!(out.status.success());
        let stdout = String::from_utf8_lossy(&out.stdout);
        for needle in expected {
            assert!(
                stdout.contains(needle),
                "help output did not contain {needle:?}:\n{stdout}"
            );
        }
    }
}

// Avoid unused-warning on Path import when the file body changes.
#[allow(dead_code)]
fn _path_marker(_p: &Path) {}

#[test]
fn targeted_posts_resolve_leading_mentions_and_repeatable_to() {
    let f = Fixture::new("C", "M");
    f.write_roster(&[("coder", false), ("reviewer", false), ("qa", false)]);
    for args in [
        vec!["msg", "post", "@reviewer @qa please check"],
        vec![
            "msg",
            "post",
            "@human explicit wins",
            "--to",
            "reviewer",
            "--to",
            "qa",
        ],
        vec!["msg", "post", "please ask @qa"],
    ] {
        let result = f.cmd("coder", &args).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
    }
    let events = f.read_log();
    for event in &events[..2] {
        assert_eq!(event.recipients(), &["reviewer", "qa"]);
    }
    assert!(events[2].recipients().is_empty());
    for text in ["@human forbidden", "@missing unknown"] {
        let result = f.cmd("coder", &["msg", "post", text]).output().unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&result.stderr)
            .contains("valid handles: @coder, @reviewer, @qa"));
    }
    assert_eq!(f.read_log().len(), 3);
}

#[test]
fn leading_address_tokens_accept_one_colon_or_comma_and_stop_at_other_text() {
    let f = Fixture::new("C", "M");
    f.write_roster(&[
        ("coder", false),
        ("reviewer", false),
        ("qa", false),
        ("impl", false),
    ]);
    let cases = [
        ("@coder: please check", vec!["coder"]),
        ("@coder, please check", vec!["coder"]),
        ("@coder, @qa: hi", vec!["coder", "qa"]),
        ("@impl's fix", vec![]),
        ("@reviewer's audit @qa", vec![]),
        ("@", vec![]),
        ("@coder @impl's fix @qa", vec!["coder"]),
    ];
    for (text, expected) in &cases {
        let result = f.cmd("reviewer", &["msg", "post", text]).output().unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let events = f.read_log();
        let event = events.last().unwrap();
        assert_eq!(event.recipients(), expected);
        assert_eq!(event.payload["text"], *text);
    }
    for text in [
        "@missing: check",
        "@missing, check",
        "@human: check",
        "@human, check",
    ] {
        let result = f.cmd("reviewer", &["msg", "post", text]).output().unwrap();
        assert_eq!(result.status.code(), Some(1));
        assert!(String::from_utf8_lossy(&result.stderr).contains("valid handles:"));
    }
    assert_eq!(f.read_log().len(), cases.len());
}
