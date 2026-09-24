use super::*;

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
fn enter_claude_launch_gate_records_timestamp_only_for_claude_code() {
    // Non-claude runtimes must not touch `last_spawn_at` —
    // otherwise a codex spawn would unnecessarily delay a
    // subsequent claude. Sanity-check that the runtime-string
    // discriminator is wired correctly.
    let mgr = mgr_with_fake(None, fake_runtime());
    assert!(mgr.claude_launch_gate.lock().unwrap().is_none());

    // Shell / codex / empty string: state stays None.
    mgr.enter_claude_launch_gate("s1", Some(Runtime::Shell));
    mgr.enter_claude_launch_gate("s2", Some(Runtime::Codex));
    mgr.enter_claude_launch_gate("s3", Runtime::parse(""));
    assert!(
        mgr.claude_launch_gate.lock().unwrap().is_none(),
        "non-claude runtimes must not advance the gate"
    );

    // claude-code stamps the field.
    mgr.enter_claude_launch_gate("s4", Some(Runtime::ClaudeCode));
    assert!(
        mgr.claude_launch_gate.lock().unwrap().is_some(),
        "claude-code spawn must advance the gate"
    );
}

#[test]
fn enter_claude_launch_gate_first_claude_does_not_sleep() {
    // First claude-code spawn through the gate (no prior) must
    // return nearly immediately — the deadline-based design's
    // whole point. Even at the production GRACE (1500ms), a
    // cold start should take << 100ms here.
    let mgr = mgr_with_fake(None, fake_runtime());
    let started = Instant::now();
    mgr.enter_claude_launch_gate("first", Some(Runtime::ClaudeCode));
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
            codex_pending_turn: None,
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
        SessionManager::apply_runtime_args(
            &mut spec,
            &role,
            &router::runtime::resume_plan(Some(Runtime::ClaudeCode), None),
            root.path(),
            None,
            None,
            None,
        );
        assert!(!spec
            .env
            .contains_key(crate::session::claude_status::PATH_ENV));
        assert!(!spec
            .env
            .contains_key(crate::session::claude_status::GENERATION_ENV));
        assert!(!stale_rekey.exists());
    }
}

#[test]
fn spawn_argv_injects_runtime_settings_for_fresh_and_resume() {
    let compose = |runtime: &str, plan: router::runtime::ResumePlan| {
        let mut role = role("/bin/cat", &["--debug"]);
        role.runtime = runtime.into();
        let mut spec = SpawnSpec {
            codex_pending_turn: None,
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
        SessionManager::apply_runtime_args(
            &mut spec,
            &role,
            &plan,
            &fixture_tmp_dir().join("runner-app-data"),
            None,
            Some("first turn"),
            None,
        );
        if runtime == "claude-code" {
            let generation = spec.env[crate::session::claude_status::GENERATION_ENV].clone();
            assert!(uuid::Uuid::parse_str(&generation).is_ok());
            assert_eq!(
                spec.env[crate::session::claude_status::PATH_ENV],
                crate::session::hook_feed::hook_path(&crate::session::claude_status::status_path(
                    &fixture_tmp_dir().join("runner-app-data"),
                    "settings-argv",
                )),
            );
        } else {
            assert!(!spec
                .env
                .contains_key(crate::session::claude_status::PATH_ENV));
            assert!(!spec
                .env
                .contains_key(crate::session::claude_status::GENERATION_ENV));
        }
        spec.args
    };

    let fresh = compose(
        "claude-code",
        router::runtime::resume_plan(Some(Runtime::ClaudeCode), None),
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
        router::runtime::resume_plan(Some(Runtime::ClaudeCode), Some(&prior)),
    );
    assert!(resumed.windows(2).any(|pair| pair[0] == "--settings"));

    let codex = compose(
        "codex",
        router::runtime::resume_plan(Some(Runtime::Codex), None),
    );
    assert!(!codex.iter().any(|arg| arg == "--settings"));
    assert!(codex
        .windows(2)
        .any(|args| args == ["-c", "check_for_update_on_startup=false"]));
    assert_eq!(codex.last().map(String::as_str), Some("first turn"));

    let resumed = compose(
        "codex",
        router::runtime::resume_plan(Some(Runtime::Codex), Some(&prior)),
    );
    assert_eq!(&resumed[..2], &["resume", prior.as_str()]);
    assert!(resumed
        .windows(2)
        .any(|args| args == ["-c", "check_for_update_on_startup=false"]));
    assert!(!resumed.iter().any(|arg| arg == "first turn"));

    for runtime in [
        "claude-code",
        "trae",
        "copilot",
        "pi",
        "antigravity",
        "opencode",
    ] {
        let args = compose(
            runtime,
            router::runtime::resume_plan(Runtime::parse(runtime), None),
        );
        assert!(!args
            .iter()
            .any(|arg| arg.contains("check_for_update_on_startup")));
    }
}
