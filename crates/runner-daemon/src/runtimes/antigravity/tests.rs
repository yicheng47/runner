use super::*;
use crate::runtimes::test_support::*;

#[test]
fn antigravity_fresh_plan_waits_for_agy_and_resume_passes_the_conversation() {
    let fresh = resume_plan(Runtime::Antigravity.key(), None);
    assert!(fresh.args.is_empty());
    assert!(fresh.assigned_key.is_none());
    assert!(!fresh.resuming);
    assert!(resume_plan(Runtime::Antigravity.key(), Some("not-a-uuid"))
        .args
        .is_empty());

    let prior = "019fa1b9-a133-7841-b4dd-730d376ab1d1";
    let plan = resume_plan(Runtime::Antigravity.key(), Some(prior));
    assert_eq!(plan.args, ["--conversation", prior]);
    assert!(!plan.prepend, "--conversation is a trailing flag");
    assert_eq!(plan.assigned_key.as_deref(), Some(prior));
    assert!(plan.resuming);
    assert!(fork_plan(Runtime::Antigravity.key(), prior, "Source").is_none());
}

#[test]
fn antigravity_trailing_args_carry_the_session_log_and_first_turn_on_i() {
    let app_data = Path::new("/tmp/runner-app-data");
    let log = crate::runtimes::antigravity::agy_capture::log_path(app_data, "runner-session")
        .to_string_lossy()
        .into_owned();
    let expected_settings = antigravity_settings_args(&[], app_data);
    assert_eq!(expected_settings.len(), 2);
    assert_eq!(expected_settings[0], "--settings");
    let fresh = trailing_runtime_args(
        Runtime::Antigravity.key(),
        &[],
        app_data,
        "runner-session",
        false,
        Some("gemini-3.8-flash"),
        Some("medium"),
        None,
        Some("persona"),
        Some("first turn"),
    );
    assert_eq!(
        fresh,
        [
            "--model",
            "gemini-3.8-flash",
            "--effort",
            "medium",
            "--log-file",
            log.as_str(),
            "--settings",
            expected_settings[1].as_str(),
            "-i",
            "first turn",
        ]
    );
    let resumed = trailing_runtime_args(
        Runtime::Antigravity.key(),
        &[],
        app_data,
        "runner-session",
        true,
        None,
        None,
        None,
        Some("persona"),
        Some("first turn"),
    );
    assert_eq!(
        resumed,
        [
            "--log-file",
            log.as_str(),
            "--settings",
            expected_settings[1].as_str(),
        ]
    );
    assert!(system_prompt_args(Runtime::Antigravity.key(), Some("persona")).is_empty());
    assert_eq!(
        first_turn_argv(Runtime::Antigravity.key(), Some("body")),
        ["-i", "body"]
    );
}

#[test]
fn antigravity_settings_inject_title_hook_across_launches_and_respect_role_settings_flags() {
    let app_data = Path::new("/Users/jason/Library/Application Support/Jason's Runner");
    let env = std::collections::BTreeMap::new();
    for (resuming, mission, first_turn) in [
        (false, false, Some("chat prompt")),
        (true, false, None),
        (false, true, Some("mission brief")),
        (true, true, None),
    ] {
        let args = Antigravity.launch_args(&LaunchContext {
            default_model: None,
            env: &env,
            cwd: None,
            role_args: &[],
            app_data_dir: app_data,
            session_id: "runner-session",
            resuming,
            mission,
            model: None,
            effort: None,
            codex_speed: None,
            system_prompt: None,
            first_turn,
        });
        let settings_str = args
            .windows(2)
            .find(|pair| pair[0] == "--settings")
            .map(|pair| pair[1].as_str())
            .expect("Antigravity launch_args should inject --settings");
        let parsed: serde_json::Value = serde_json::from_str(settings_str).unwrap();
        assert_eq!(parsed["title"]["type"], "command");
        assert_eq!(parsed["title"]["enabled"], true);
        let cmd = parsed["title"]["command"].as_str().unwrap();
        if cfg!(windows) {
            assert_eq!(
                cmd,
                format!("\"{}\" hook title", hook_executable(app_data).display())
            );
        } else {
            assert_eq!(
                cmd,
                "'/Users/jason/Library/Application Support/Jason'\\''s Runner/bin/runner' hook title"
            );
        }
        if let Some(body) = first_turn {
            assert_eq!(&args[args.len() - 2..], ["-i", body]);
        } else {
            assert!(!args.iter().any(|arg| arg == "-i"));
        }
    }

    assert_eq!(
        antigravity_title_command(
            Path::new("/Users/jason/Library/Application Support/Jason's Runner/bin/runner"),
            false,
        ),
        "'/Users/jason/Library/Application Support/Jason'\\''s Runner/bin/runner' hook title"
    );
    let win_cmd = antigravity_title_command(
        Path::new(r"C:\Users\Jason Wang\AppData\Roaming\com.wycstudios.runner\bin\runner.exe"),
        true,
    );
    assert_eq!(
        win_cmd,
        r#""C:\Users\Jason Wang\AppData\Roaming\com.wycstudios.runner\bin\runner.exe" hook title"#
    );
    assert_eq!(win_cmd.matches('"').count(), 2);

    for custom in [
        vec!["--settings", r#"{"toolPermission":"request-review"}"#],
        vec!["-settings", r#"{"toolPermission":"request-review"}"#],
        vec![r#"--settings={"toolPermission":"request-review"}"#],
        vec![r#"-settings={"toolPermission":"request-review"}"#],
    ] {
        let role_args: Vec<String> = custom.iter().map(|s| (*s).to_owned()).collect();
        assert!(!inject_antigravity_settings(&role_args), "{custom:?}");
        assert!(
            antigravity_settings_args(&role_args, app_data).is_empty(),
            "{custom:?}"
        );
        let fake_home = tempfile::tempdir().unwrap();
        let role_env = std::collections::HashMap::new();
        crate::runtimes::with_conversation_home(fake_home.path(), || {
            assert_eq!(
                antigravity_summaries_db_path(&[], &role_env),
                Some(
                    fake_home
                        .path()
                        .join(".gemini/antigravity-cli/conversation_summaries.db")
                )
            );
            assert_eq!(
                antigravity_summaries_db_path(&role_args, &role_env),
                None,
                "{custom:?}"
            );
        });
        let args = Antigravity.launch_args(&LaunchContext {
            default_model: None,
            env: &env,
            cwd: None,
            role_args: &role_args,
            app_data_dir: app_data,
            session_id: "runner-session",
            resuming: false,
            mission: false,
            model: None,
            effort: None,
            codex_speed: None,
            system_prompt: None,
            first_turn: Some("first turn"),
        });
        assert!(
            !args.iter().any(|arg| arg.contains("settings")),
            "{custom:?} -> {args:?}"
        );
    }
}

#[test]
fn antigravity_uses_program_status_without_status_hooks() {
    let adapter = Antigravity;
    assert!(adapter.status_hooks().is_none());
    assert!(adapter.status_semantics().program_status_authority);
    assert!(!adapter.startup_delivery_uses_hooks(false));
    assert!(!adapter.startup_delivery_uses_hooks(true));
}

#[test]
fn antigravity_cleanup_removes_legacy_hooks_dir_safely() {
    let root = tempfile::tempdir().unwrap();
    let hooks_root = root.path().join("antigravity-hooks");
    let agents_dir = hooks_root.join(".agents");
    std::fs::create_dir_all(&agents_dir).unwrap();
    std::fs::write(agents_dir.join("hooks.json"), "{}").unwrap();
    std::fs::write(hooks_root.join("report.sh"), "#!/bin/sh\n").unwrap();

    let active_log = agy_capture::log_path(root.path(), "active-session");
    std::fs::create_dir_all(active_log.parent().unwrap()).unwrap();
    std::fs::write(&active_log, "keep\n").unwrap();

    remove_legacy_hooks_dir(root.path());

    assert!(!hooks_root.exists());
    assert!(active_log.is_file());

    // Unexpected user files inside antigravity-hooks are preserved while known Runner files go.
    std::fs::create_dir_all(&agents_dir).unwrap();
    std::fs::write(agents_dir.join("hooks.json"), "{}").unwrap();
    std::fs::write(hooks_root.join("report.sh"), "#!/bin/sh\n").unwrap();
    let extra = hooks_root.join("keep.txt");
    std::fs::write(&extra, "user note").unwrap();
    remove_legacy_hooks_dir(root.path());
    assert!(!agents_dir.exists());
    assert!(!hooks_root.join("report.sh").exists());
    assert!(extra.is_file());
    std::fs::remove_file(&extra).unwrap();
    std::fs::remove_dir(&hooks_root).unwrap();

    #[cfg(unix)]
    {
        let target = tempfile::tempdir().unwrap();
        let target_agents = target.path().join(".agents");
        std::fs::create_dir_all(&target_agents).unwrap();
        let target_hooks = target_agents.join("hooks.json");
        std::fs::write(&target_hooks, "{}").unwrap();
        std::os::unix::fs::symlink(target.path(), &hooks_root).unwrap();
        remove_legacy_hooks_dir(root.path());
        assert!(target_hooks.is_file(), "must not follow symlinks");
        assert!(hooks_root
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        std::fs::remove_file(&hooks_root).unwrap();

        // Intermediate .agents symlink inside a real antigravity-hooks directory must not be followed.
        std::fs::create_dir_all(&hooks_root).unwrap();
        std::os::unix::fs::symlink(&target_agents, &agents_dir).unwrap();
        std::fs::write(hooks_root.join("report.sh"), "#!/bin/sh\n").unwrap();
        remove_legacy_hooks_dir(root.path());
        assert!(
            target_hooks.is_file(),
            "must not follow intermediate .agents symlink"
        );
        assert!(agents_dir
            .symlink_metadata()
            .unwrap()
            .file_type()
            .is_symlink());
        assert!(!hooks_root.join("report.sh").exists());
    }
}

#[test]
fn antigravity_conversation_probe_reads_the_global_store() {
    let home = tempfile::tempdir().unwrap();
    let key = uuid::Uuid::new_v4().to_string();
    assert!(!antigravity_conversation_exists_at(home.path(), &key));
    let store = home.path().join(".gemini/antigravity-cli/conversations");
    std::fs::create_dir_all(store.join(format!("{key}.db"))).unwrap();
    assert!(
        !antigravity_conversation_exists_at(home.path(), &key),
        "a directory is not a conversation"
    );
    let key = uuid::Uuid::new_v4().to_string();
    std::fs::write(store.join(format!("{key}.db")), "").unwrap();
    assert!(with_conversation_home(home.path(), || {
        antigravity_conversation_exists(&key)
    }));
}

#[test]
fn antigravity_model_and_effort_respect_catalog_and_dynamic_models() {
    let args = |model: Option<&str>, effort: Option<&str>| {
        model_effort_args(Runtime::Antigravity.key(), model, effort)
    };
    assert_eq!(
        args(Some("gemini-4-hydrogen"), Some(" XHigh ")),
        ["--model", "gemini-4-hydrogen", "--effort", "xhigh"]
    );
    assert_eq!(
        args(Some("gemini-3.8-flash"), Some(" Medium ")),
        ["--model", "gemini-3.8-flash", "--effort", "medium"]
    );
    assert_eq!(
        args(Some("claude-opus-5-5"), Some("high")),
        ["--model", "claude-opus-5-5", "--effort", "high"]
    );
    assert_eq!(
        args(Some("gpt-oss-120b"), Some("medium")),
        ["--model", "gpt-oss-120b", "--effort", "medium"]
    );
    // gemini-4-carbon has no xhigh/max; orchestrators take no --effort.
    assert_eq!(
        args(Some("gemini-4-carbon"), Some("xhigh")),
        ["--model", "gemini-4-carbon"]
    );
    assert_eq!(
        args(Some("gemini-pro-orchestrator"), Some("high")),
        ["--model", "gemini-pro-orchestrator"]
    );
    // Leveled model IDs carry their own effort; passing --effort conflicts on mismatch.
    for leveled in ["gemini-4-hydrogen-xhigh", "gpt-oss-120b-medium"] {
        assert_eq!(args(Some(leveled), Some("low")), ["--model", leveled]);
    }
    // Discovered or custom base models forward valid effort levels, while invalid levels drop.
    assert_eq!(
        args(Some("custom-model"), Some("xhigh")),
        ["--model", "custom-model", "--effort", "xhigh"]
    );
    assert_eq!(
        args(Some("custom-model"), Some("ultra")),
        ["--model", "custom-model"]
    );
    // Never --effort without --model: agy errors without a model that supports it.
    assert!(args(None, Some("high")).is_empty());
    assert!(args(Some("  "), Some("low")).is_empty());
    assert!(args(None, None).is_empty());

    for (model, efforts) in ANTIGRAVITY_MODELS {
        for effort in ANTIGRAVITY_EFFORTS {
            let emitted = args(Some(model), Some(effort));
            assert_eq!(
                emitted.iter().any(|arg| arg == "--effort"),
                efforts.contains(effort),
                "{model} {effort}"
            );
        }
    }
}

#[test]
fn antigravity_permissions_roundtrip_and_strip_every_go_spelling() {
    let runtime = Some(Runtime::Antigravity);
    assert!(permission_mode_args(
        runtime.map(Runtime::key).unwrap_or(""),
        PermissionMode::Default
    )
    .is_empty());
    assert!(permission_mode_args(
        runtime.map(Runtime::key).unwrap_or(""),
        PermissionMode::Auto
    )
    .is_empty());
    assert_eq!(
        permission_mode_args(
            runtime.map(Runtime::key).unwrap_or(""),
            PermissionMode::AcceptEdits
        ),
        ["--mode", "accept-edits"]
    );
    assert_eq!(
        permission_mode_args(
            runtime.map(Runtime::key).unwrap_or(""),
            PermissionMode::Bypass
        ),
        ["--dangerously-skip-permissions"]
    );
    for mode in [
        PermissionMode::Default,
        PermissionMode::AcceptEdits,
        PermissionMode::Bypass,
    ] {
        let args = apply_permission_mode(
            runtime.map(Runtime::key).unwrap_or(""),
            &["--debug".into()],
            mode,
        );
        assert_eq!(
            infer_permission_mode(runtime.map(Runtime::key).unwrap_or(""), &args),
            mode
        );
    }
    assert_eq!(
        infer_permission_mode(
            runtime.map(Runtime::key).unwrap_or(""),
            &apply_permission_mode(
                runtime.map(Runtime::key).unwrap_or(""),
                &[],
                PermissionMode::Auto
            )
        ),
        PermissionMode::Default
    );

    let noisy: Vec<String> = [
        "--keep",
        "-mode=plan",
        "--mode",
        "plan",
        "-mode",
        "accept-edits",
        "--mode=accept-edits",
        "-dangerously-skip-permissions",
        "--dangerously-skip-permissions",
        "-dangerously-skip-permissions=true",
        "--dangerously-skip-permissions=false",
        "--model",
        "gemini-3.8-flash",
        "--kept-too",
    ]
    .map(String::from)
    .to_vec();
    assert_eq!(
        strip_permission_flags(runtime.map(Runtime::key).unwrap_or(""), &noisy),
        ["--keep", "--model", "gemini-3.8-flash", "--kept-too"]
    );

    for (args, mode) in [
        (vec!["-mode", "accept-edits"], PermissionMode::AcceptEdits),
        (vec!["-mode=accept-edits"], PermissionMode::AcceptEdits),
        (vec!["--mode=accept-edits"], PermissionMode::AcceptEdits),
        (vec!["--mode", "plan"], PermissionMode::Default),
        (
            vec!["-dangerously-skip-permissions"],
            PermissionMode::Bypass,
        ),
        (
            vec!["--dangerously-skip-permissions=true"],
            PermissionMode::Bypass,
        ),
        (
            vec!["--dangerously-skip-permissions=false"],
            PermissionMode::Default,
        ),
        (
            vec!["--mode", "accept-edits", "--dangerously-skip-permissions"],
            PermissionMode::Bypass,
        ),
    ] {
        let args: Vec<String> = args.into_iter().map(String::from).collect();
        assert_eq!(
            infer_permission_mode(runtime.map(Runtime::key).unwrap_or(""), &args),
            mode,
            "{args:?}"
        );
    }

    // The equals form on a boolean flag is Go-only; other runtimes keep it.
    let claude = vec!["--dangerously-skip-permissions=true".to_string()];
    assert_eq!(
        strip_permission_flags(Runtime::ClaudeCode.key(), &claude),
        claude
    );
    assert!(mission_bus_sandbox_args(
        runtime.map(Runtime::key).unwrap_or(""),
        Some(Path::new("/tmp/mission"))
    )
    .is_empty());
    assert!(mission_bus_sandbox_args(runtime.map(Runtime::key).unwrap_or(""), None).is_empty());
}
