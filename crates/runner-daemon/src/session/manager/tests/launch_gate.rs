use super::*;
use crate::runtimes::claude_code::launch::compute_gate_wait;

#[test]
fn compute_gate_wait_returns_zero_when_no_prior_spawn() {
    // First claude through the gate — `last` is None, so the
    // caller pays nothing. This is the property that makes
    // single direct chats / cold mission starts feel instant.
    let now = Instant::now();
    assert_eq!(
        compute_gate_wait(None, now, Duration::from_millis(1500)),
        Duration::ZERO
    );
}

#[test]
fn compute_gate_wait_returns_remaining_grace_when_prior_recent() {
    // Mid-grace case: a prior claude spawned 400ms ago and the
    // grace is 1500ms → caller waits the remaining 1100ms.
    let now = Instant::now();
    let last = now - Duration::from_millis(400);
    assert_eq!(
        compute_gate_wait(Some(last), now, Duration::from_millis(1500)),
        Duration::from_millis(1100)
    );
}

#[test]
fn compute_gate_wait_returns_zero_when_grace_already_elapsed() {
    // Prior spawn is older than the grace window → no wait. This
    // is what keeps single chats opened minutes apart from
    // paying any tax for a long-stale prior spawn.
    let now = Instant::now();
    let last = now - Duration::from_millis(5000);
    assert_eq!(
        compute_gate_wait(Some(last), now, Duration::from_millis(1500)),
        Duration::ZERO
    );
}

#[test]
fn compute_gate_wait_handles_clock_skew_without_panic() {
    // `last` being slightly in the future (Instant arithmetic
    // shouldn't underflow). saturating_duration_since clamps to
    // zero, so we treat a "future" last the same as "just now"
    // and return the full grace. Defensive only — Instant is
    // monotonic on every platform we target, so this shouldn't
    // happen in practice.
    let now = Instant::now();
    let last = now + Duration::from_millis(100);
    assert_eq!(
        compute_gate_wait(Some(last), now, Duration::from_millis(1500)),
        Duration::from_millis(1500)
    );
}

#[test]
fn enter_launch_gate_records_timestamp_only_for_claude_code() {
    // Non-claude runtimes must not touch `last_spawn_at` —
    // otherwise a codex spawn would unnecessarily delay a
    // subsequent claude. Sanity-check that the runtime-string
    // discriminator is wired correctly.
    let mgr = mgr_with_fake(None, fake_runtime());
    assert!(mgr.launch_gate.lock().unwrap().is_none());

    // Shell / codex / empty string: state stays None.
    mgr.enter_launch_gate("s1", Runtime::Shell.key());
    mgr.enter_launch_gate("s2", Runtime::Codex.key());
    mgr.enter_launch_gate("s3", "");
    assert!(
        mgr.launch_gate.lock().unwrap().is_none(),
        "non-claude runtimes must not advance the gate"
    );

    // claude-code stamps the field.
    mgr.enter_launch_gate("s4", Runtime::ClaudeCode.key());
    assert!(
        mgr.launch_gate.lock().unwrap().is_some(),
        "claude-code spawn must advance the gate"
    );
}

#[test]
fn enter_launch_gate_first_claude_does_not_sleep() {
    // First claude-code spawn through the gate (no prior) must
    // return nearly immediately — the deadline-based design's
    // whole point. Even at the production GRACE (1500ms), a
    // cold start should take << 100ms here.
    let mgr = mgr_with_fake(None, fake_runtime());
    let started = Instant::now();
    mgr.enter_launch_gate("first", Runtime::ClaudeCode.key());
    let elapsed = started.elapsed();
    assert!(
        elapsed < ci_scaled_budget(Duration::from_millis(100)),
        "first claude must not wait — actual elapsed {}ms",
        elapsed.as_millis()
    );
}

#[test]
fn custom_claude_settings_do_not_prepare_a_status_watcher() {
    for args in [
        vec!["--settings".to_owned(), "custom.json".to_owned()],
        vec!["--settings=custom.json".to_owned()],
    ] {
        let mut role = role("/bin/cat", &[]);
        role.runtime = "claude-code".into();
        role.args = args.clone();
        let root = tempfile::tempdir().unwrap();
        let stale_rekey = crate::session::claude_rekey::drop_path(root.path(), "custom-settings");
        std::fs::create_dir_all(stale_rekey.parent().unwrap()).unwrap();
        std::fs::write(&stale_rekey, "stale report").unwrap();
        let mut spec = SpawnSpec {
            agent_runtime: None,
            pending_turn: None,
            session_id: "custom-settings".into(),
            cwd: None,
            command: role.command.clone(),
            args,
            env: BTreeMap::new(),
            mission: false,
            shim_dir: None,
            bundled_bin_dir: None,
            shell_path: None,
            initial_size: Some((80, 24)),
        };
        mgr_with_fake(None, fake_runtime()).apply_runtime_args(
            &mut spec,
            &role,
            &crate::runtimes::adapter(Runtime::ClaudeCode).resume_plan(None),
            root.path(),
            None,
            None,
            None,
            None,
        );
        assert!(!spec.env.contains_key("RUNNER_CLAUDE_STATUS_PATH"));
        assert!(!spec
            .env
            .contains_key(runner_core::protocol::hook::GENERATION_ENV));
        assert!(stale_rekey.exists());
    }
}

#[test]
fn spawn_argv_injects_runtime_settings_for_fresh_and_resume() {
    let compose = |runtime: &str, plan: router::runtime::ResumePlan| {
        let mut role = role("/bin/cat", &["--debug"]);
        role.runtime = runtime.into();
        let mut spec = SpawnSpec {
            agent_runtime: None,
            pending_turn: None,
            session_id: "settings-argv".into(),
            cwd: None,
            command: role.command.clone(),
            args: role.args.clone(),
            env: BTreeMap::new(),
            mission: false,
            shim_dir: None,
            bundled_bin_dir: None,
            shell_path: None,
            initial_size: Some((80, 24)),
        };
        mgr_with_fake(None, fake_runtime()).apply_runtime_args(
            &mut spec,
            &role,
            &plan,
            &fixture_tmp_dir().join("runner-app-data"),
            None,
            Some("first turn"),
            None,
            None,
        );
        if runtime == "claude-code" {
            let generation = spec.env[runner_core::protocol::hook::GENERATION_ENV].clone();
            assert!(uuid::Uuid::parse_str(&generation).is_ok());
            assert_eq!(
                spec.env[runner_core::protocol::hook::SESSION_ENV],
                spec.session_id
            );
        }
        spec.args
    };

    let fresh = compose(
        "claude-code",
        crate::runtimes::adapter(Runtime::ClaudeCode).resume_plan(None),
    );
    let settings = fresh
        .windows(2)
        .find(|pair| pair[0] == "--settings")
        .map(|pair| serde_json::from_str::<serde_json::Value>(&pair[1]).unwrap())
        .expect("Claude spawn should carry --settings");
    assert_eq!(settings["tui"], "fullscreen");
    assert!(settings["hooks"]["SessionStart"].is_array());
    assert_eq!(fresh.last().map(String::as_str), Some("first turn"));

    let prior = uuid::Uuid::new_v4().to_string();
    let resumed = compose(
        "claude-code",
        crate::runtimes::adapter(Runtime::ClaudeCode).resume_plan(Some(&prior)),
    );
    assert!(resumed.windows(2).any(|pair| pair[0] == "--settings"));

    let codex = compose(
        "codex",
        crate::runtimes::adapter(Runtime::Codex).resume_plan(None),
    );
    assert!(!codex.iter().any(|arg| arg == "--settings"));
    assert!(codex
        .windows(2)
        .any(|args| args == ["-c", "check_for_update_on_startup=false"]));
    assert!(codex
        .windows(2)
        .any(|args| args == ["-c", crate::runtimes::codex::CODEX_TERMINAL_TITLE]));
    assert_eq!(codex.last().map(String::as_str), Some("first turn"));

    let resumed = compose(
        "codex",
        crate::runtimes::adapter(Runtime::Codex).resume_plan(Some(&prior)),
    );
    assert_eq!(&resumed[..2], &["resume", prior.as_str()]);
    assert!(resumed
        .windows(2)
        .any(|args| args == ["-c", "check_for_update_on_startup=false"]));
    assert!(resumed
        .windows(2)
        .any(|args| args == ["-c", crate::runtimes::codex::CODEX_TERMINAL_TITLE]));
    assert!(!resumed.iter().any(|arg| arg == "first turn"));

    for runtime in ["claude-code", "trae", "copilot", "pi", "antigravity"] {
        let args = compose(runtime, crate::runtimes::for_key(runtime).resume_plan(None));
        assert!(!args
            .iter()
            .any(|arg| arg.contains("check_for_update_on_startup")));
    }

    let agy_fresh = compose(
        "antigravity",
        crate::runtimes::adapter(Runtime::Antigravity).resume_plan(None),
    );
    let agy_settings = agy_fresh
        .windows(2)
        .find(|pair| pair[0] == "--settings")
        .map(|pair| serde_json::from_str::<serde_json::Value>(&pair[1]).unwrap())
        .expect("Antigravity spawn should carry --settings");
    assert_eq!(agy_settings["title"]["type"], "command");
    assert_eq!(agy_settings["title"]["enabled"], true);
    assert!(agy_settings["title"]["command"]
        .as_str()
        .unwrap()
        .contains("hook title"));
    assert_eq!(&agy_fresh[agy_fresh.len() - 2..], ["-i", "first turn"]);

    let agy_resumed = compose(
        "antigravity",
        crate::runtimes::adapter(Runtime::Antigravity).resume_plan(Some(&prior)),
    );
    assert!(has_arg_pair(&agy_resumed, "--conversation", &prior));
    assert!(agy_resumed.windows(2).any(|pair| pair[0] == "--settings"));
    assert!(!agy_resumed.iter().any(|arg| arg == "-i"));

    for custom_flag in [
        &["--settings", r#"{"toolPermission":"request-review"}"#][..],
        &["-settings", r#"{"toolPermission":"request-review"}"#][..],
        &[r#"--settings={"toolPermission":"request-review"}"#][..],
        &[r#"-settings={"toolPermission":"request-review"}"#][..],
    ] {
        let mut custom_role = role("agy", custom_flag);
        custom_role.runtime = "antigravity".into();
        let mut spec = SpawnSpec {
            session_id: "settings-opt-out".into(),
            command: custom_role.command.clone(),
            args: custom_role.args.clone(),
            ..Default::default()
        };
        mgr_with_fake(None, fake_runtime()).apply_runtime_args(
            &mut spec,
            &custom_role,
            &crate::runtimes::adapter(Runtime::Antigravity).resume_plan(None),
            &fixture_tmp_dir().join("runner-app-data"),
            None,
            Some("first turn"),
            None,
            None,
        );
        assert_eq!(
            spec.args
                .iter()
                .filter(|arg| arg.starts_with("--settings") || arg.starts_with("-settings"))
                .count(),
            1,
            "{custom_flag:?} -> {:?}",
            spec.args
        );
    }
}
