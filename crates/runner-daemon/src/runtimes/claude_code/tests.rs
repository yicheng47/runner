use super::*;
use crate::runtimes::test_support::*;

#[cfg(windows)]
#[test]
fn claude_settings_on_windows_quote_the_native_reporter_for_bash() {
    let root = Path::new(r"C:\Users\Jason Wang\it's runner app");
    let args = claude_settings_args_for(Runtime::ClaudeCode.key(), &[], root, "session");
    let settings: serde_json::Value = serde_json::from_str(&args[1]).unwrap();
    for event in [
        "SessionStart",
        "PermissionRequest",
        "PermissionDenied",
        "PostToolUseFailure",
        "Elicitation",
        "ElicitationResult",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PreCompact",
        "PostCompact",
        "Notification",
        "Stop",
        "StopFailure",
    ] {
        let entries = settings["hooks"][event].as_array().unwrap();
        let status = entries.last().unwrap()["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert_eq!(
            status,
            format!(
                "'C:/Users/Jason Wang/it'\\''s runner app/bin/runner.exe' hook report --runtime claude-code --event {event} 2>/dev/null || :"
            ),
        );
        assert_eq!(
            status,
            hook_report_command(root, Runtime::ClaudeCode, event, false)
        );
    }
}

#[test]
fn claude_transcript_lookup_preserves_resume_identity() {
    let home = tempfile::tempdir().unwrap();
    let prior = uuid::Uuid::new_v4().to_string();
    #[cfg(windows)]
    let cases = [
        ("C--Users-ROG", [r"C:\Users\ROG", "C:/Users/ROG"]),
        (
            "C--Work-my-project-v1-0",
            [r"C:\Work\my_project v1.0", "C:/Work/my_project v1.0"],
        ),
    ];
    #[cfg(not(windows))]
    let cases = [(
        "-Users-tester-app-test",
        ["/Users/tester/app.test", "/Users/tester/app-test"],
    )];

    for (encoded, cwds) in cases {
        let project_dir = home.path().join(".claude/projects").join(encoded);
        std::fs::create_dir_all(&project_dir).unwrap();
        std::fs::write(project_dir.join(format!("{prior}.jsonl")), "{}\n").unwrap();
        for cwd in cwds {
            let exists = conversation_file_exists_at(
                home.path(),
                ".claude",
                cwd,
                &prior,
                claude_code_project_dir,
            );
            assert!(exists, "saved Claude transcript must be found for {cwd}");
            let plan = resume_plan(Runtime::ClaudeCode.key(), exists.then_some(prior.as_str()));
            assert!(plan.resuming);
            assert_eq!(plan.args, ["--resume", prior.as_str()]);
        }
    }
    assert!(!conversation_file_exists_at(
        home.path(),
        ".claude",
        cases[0].1[0],
        &uuid::Uuid::new_v4().to_string(),
        claude_code_project_dir,
    ));
}

#[test]
fn claude_alias_transcripts_resume_from_canonical_or_literal_project_paths() {
    let home = tempfile::tempdir().unwrap();
    #[cfg(windows)]
    let root = {
        let canonical = home.path().canonicalize().unwrap();
        let canonical = canonical.to_str().unwrap();
        Path::new(canonical.strip_prefix(r"\\?\").unwrap()).to_path_buf()
    };
    #[cfg(unix)]
    let root = home.path().to_path_buf();
    let project = root.join("project");
    std::fs::create_dir_all(&project).unwrap();
    #[cfg(unix)]
    let alias = {
        let alias = root.join("project-alias");
        std::os::unix::fs::symlink(&project, &alias).unwrap();
        alias
    };
    #[cfg(windows)]
    let alias = {
        std::fs::create_dir_all(root.join("alias")).unwrap();
        root.join("alias/../project")
    };
    #[cfg(unix)]
    let canonical = project.canonicalize().unwrap();
    #[cfg(windows)]
    let canonical = project;
    let prior = uuid::Uuid::new_v4().to_string();
    with_conversation_home(home.path(), || {
        for cwd in [&canonical, &alias] {
            let transcript = home
                .path()
                .join(".claude/projects")
                .join(claude_code_project_dir(&cwd.to_string_lossy()))
                .join(format!("{prior}.jsonl"));
            std::fs::create_dir_all(transcript.parent().unwrap()).unwrap();
            std::fs::write(&transcript, "{}\n").unwrap();
            for probe in [&alias, cwd] {
                let found = claude_code_conversation_exists(Some(&probe.to_string_lossy()), &prior);
                assert!(
                    found,
                    "stored cwd={cwd:?}; probe={probe:?}; canonical={:?}; transcript={transcript:?}",
                    probe.canonicalize()
                );
                let plan = resume_plan(Runtime::ClaudeCode.key(), found.then_some(prior.as_str()));
                assert!(plan.resuming);
                assert_eq!(plan.args, ["--resume", prior.as_str()]);
            }
            std::fs::remove_file(transcript).unwrap();
        }
        assert!(!claude_code_conversation_exists(
            Some(&alias.to_string_lossy()),
            &prior
        ));
        let plan = resume_plan(Runtime::ClaudeCode.key(), None);
        assert!(!plan.resuming);
        assert_ne!(plan.assigned_key.as_deref(), Some(prior.as_str()));
    });
}

#[cfg(windows)]
#[test]
fn claude_canonical_windows_paths_use_ordinary_disk_and_unc_spellings() {
    for (verbatim, ordinary) in [
        (r"\\?\C:\Work\project", r"C:\Work\project"),
        (r"\\?\UNC\server\share\project", r"\\server\share\project"),
        (r"C:\Work\project", r"C:\Work\project"),
        (r"\\server\share\project", r"\\server\share\project"),
    ] {
        assert_eq!(
            ordinary_windows_path(Path::new(verbatim)),
            Path::new(ordinary)
        );
    }
}

#[test]
fn claude_code_returns_no_argv_for_system_prompt() {
    // claude-code's --append-system-prompt is SDK-only; the
    // interactive TUI ignores it. The argv path returns empty,
    // and call sites fold the prompt into first-turn delivery
    // instead.
    let args = system_prompt_args(Runtime::ClaudeCode.key(), Some("be helpful"));
    assert!(args.is_empty());
}

#[test]
fn claude_settings_injects_fullscreen_and_per_spawn_session_start_hook() {
    let app_data_dir = Path::new("/tmp/runner app-data");
    let args = claude_settings_args_for(
        Runtime::ClaudeCode.key(),
        &[],
        app_data_dir,
        "runner-session-one",
    );
    assert_eq!(args.len(), 2);
    assert_eq!(args[0], "--settings");
    assert_eq!(args.iter().filter(|arg| *arg == "--settings").count(), 1);
    assert!(!args[1].contains(['\n', '\t']));
    let settings = serde_json::from_str::<serde_json::Value>(&args[1]).unwrap();
    assert_eq!(settings["tui"], "fullscreen");
    assert_eq!(settings["theme"], "auto");
    let command = settings["hooks"]["SessionStart"][0]["hooks"][0]["command"]
        .as_str()
        .unwrap();
    assert_eq!(
        command,
        hook_report_command(app_data_dir, Runtime::ClaudeCode, "SessionStart", false)
    );
    assert_eq!(
        settings["hooks"]["SessionStart"].as_array().unwrap().len(),
        1
    );
    assert!(!command.contains("session-keys"));
    assert_eq!(
        settings["hooks"]["SessionStart"][0]["hooks"][0]["type"],
        "command"
    );

    let other = claude_settings_args_for(
        Runtime::ClaudeCode.key(),
        &[],
        app_data_dir,
        "runner-session-two",
    );
    assert_eq!(args[1], other[1]);
}

#[test]
fn claude_settings_acknowledge_bypass_only_for_bypass_spawns() {
    let app_data_dir = Path::new("/tmp/runner-app-data");
    let settings_for = |role_args: &[String]| {
        let args = claude_settings_args_for(
            Runtime::ClaudeCode.key(),
            role_args,
            app_data_dir,
            "runner-session",
        );
        serde_json::from_str::<serde_json::Value>(&args[1]).unwrap()
    };

    for bypass_args in [
        vec!["--permission-mode".to_string(), "bypassPermissions".into()],
        vec!["--dangerously-skip-permissions".to_string()],
        apply_mission_permission_mode(
            Runtime::ClaudeCode.key(),
            &["--verbose".to_string()],
            MissionPermissionMode::Bypass,
        ),
    ] {
        let settings = settings_for(&bypass_args);
        assert_eq!(
            settings["skipDangerousModePermissionPrompt"],
            serde_json::Value::Bool(true),
            "bypass args must acknowledge the consent dialog: {bypass_args:?}"
        );
        assert_eq!(settings["tui"], "fullscreen");
        assert!(settings["hooks"]["SessionStart"].is_array());
    }

    for other_args in [
        vec![],
        vec!["--permission-mode".to_string(), "auto".into()],
        vec!["--permission-mode".to_string(), "acceptEdits".into()],
        apply_mission_permission_mode(
            Runtime::ClaudeCode.key(),
            &["--permission-mode".to_string(), "bypassPermissions".into()],
            MissionPermissionMode::Auto,
        ),
    ] {
        let settings = settings_for(&other_args);
        assert!(
            settings.get("skipDangerousModePermissionPrompt").is_none(),
            "non-bypass args must not acknowledge the dialog: {other_args:?}"
        );
    }
}

#[test]
fn claude_status_hooks_have_short_timeouts_and_match_verified_notifications() {
    let args = claude_settings_args_for(
        Runtime::ClaudeCode.key(),
        &[],
        Path::new("/tmp/runner app"),
        "session",
    );
    let settings: serde_json::Value = serde_json::from_str(&args[1]).unwrap();
    for event in [
        "SessionStart",
        "UserPromptSubmit",
        "PreToolUse",
        "PostToolUse",
        "PreCompact",
        "PostCompact",
        "Notification",
        "Stop",
        "StopFailure",
    ] {
        let hook = &settings["hooks"][event][0]["hooks"][0];
        assert_eq!(hook["type"], "command", "{event}");
        assert_eq!(hook["timeout"], 2, "{event}");
        assert!(hook["command"].as_str().unwrap().contains(&format!(
            "hook report --runtime claude-code --event {event}"
        )));
    }
    assert_eq!(
        settings["hooks"]["Notification"][0]["matcher"],
        "^(idle_prompt|permission_prompt|elicitation_dialog)$"
    );
    assert!(settings["hooks"]["StopFailure"][0].get("matcher").is_none());
    assert!(settings["hooks"].get("SubagentStop").is_none());
}

#[test]
fn claude_settings_respects_runner_settings_flags() {
    for role_args in [
        vec!["--settings".into(), "custom.json".into()],
        vec!["--settings=custom.json".into()],
    ] {
        assert!(claude_settings_args_for(
            Runtime::ClaudeCode.key(),
            &role_args,
            Path::new("/tmp/runner-app-data"),
            "runner-session"
        )
        .is_empty());
    }
}

#[test]
fn claude_settings_do_not_leak_to_other_runtimes() {
    for runtime in ["codex", "pi", "antigravity", "shell"] {
        assert!(claude_settings_args_for(
            runtime,
            &[],
            Path::new("/tmp/runner-app-data"),
            "runner-session"
        )
        .is_empty());
    }
}

#[test]
fn claude_code_fresh_self_assigns_session_id() {
    let plan = resume_plan(Runtime::ClaudeCode.key(), None);
    assert!(!plan.resuming);
    assert!(!plan.prepend);
    assert_eq!(plan.args.len(), 2);
    assert_eq!(plan.args[0], "--session-id");
    let assigned = plan.assigned_key.as_deref().unwrap();
    assert_eq!(plan.args[1], assigned);
    assert!(is_uuid(assigned), "assigned key must be a UUID");
}

#[test]
fn claude_code_resumes_with_prior_uuid() {
    // `--resume <uuid>` is the right flag. `--session-id` would
    // be rejected as "already in use" because claude-code treats
    // it as fresh-only. Fresh-spawn first-turn delivery ensures
    // the conversation file exists before any resume attempt.
    let prior = uuid::Uuid::new_v4().to_string();
    let plan = resume_plan(Runtime::ClaudeCode.key(), Some(&prior));
    assert!(plan.resuming);
    assert!(!plan.prepend);
    assert_eq!(plan.args, vec!["--resume", &prior]);
    assert_eq!(plan.assigned_key.as_deref(), Some(prior.as_str()));
}

#[test]
fn claude_code_falls_back_to_fresh_on_invalid_prior_key() {
    // A non-UUID prior key would crash claude-code's --resume parser.
    // Treat it as missing and start fresh.
    let plan = resume_plan(Runtime::ClaudeCode.key(), Some("not-a-uuid"));
    assert!(!plan.resuming);
    assert_eq!(plan.args[0], "--session-id");
}

#[test]
fn claude_code_fork_assigns_a_new_session_key() {
    let source = "019fa1b9-a133-7841-b4dd-730d376ab1d1";
    let plan = fork_plan(Runtime::ClaudeCode.key(), source, "Source").unwrap();
    let ForkPlan::Direct(plan) = plan else {
        panic!("claude-code must spawn its fork directly")
    };
    let assigned = plan.assigned_key.as_deref().unwrap();
    assert_ne!(assigned, source);
    assert_eq!(
        plan.args,
        vec![
            "--resume",
            source,
            "--fork-session",
            "--session-id",
            assigned,
        ]
    );
    assert!(!plan.prepend);
    assert!(plan.resuming);
}

#[test]
fn claude_code_emits_model_and_effort_flags() {
    let args = model_effort_args(
        Runtime::ClaudeCode.key(),
        Some("claude-opus-4-7"),
        Some("xhigh"),
    );
    assert_eq!(
        args,
        vec![
            "--model".to_string(),
            "claude-opus-4-7".to_string(),
            "--effort".to_string(),
            "xhigh".to_string(),
        ]
    );
}

#[test]
fn claude_code_forwards_effort_verbatim() {
    // Asymmetric on purpose: claude-code's `--effort` is case-
    // insensitive (accepts `High`), so we forward the row's
    // value verbatim rather than risk regressing already-shipped
    // behavior. Only the codex branch normalises.
    let args = model_effort_args(Runtime::ClaudeCode.key(), None, Some("High"));
    assert!(
        args.windows(2)
            .any(|w| w[0] == "--effort" && w[1] == "High"),
        "expected verbatim effort for claude-code, got: {args:?}",
    );
}

#[test]
fn apply_permission_mode_claude_code_each_mode() {
    for (mode, expected_extra) in [
        (
            PermissionMode::AcceptEdits,
            vec!["--permission-mode".to_string(), "acceptEdits".to_string()],
        ),
        (
            PermissionMode::Auto,
            vec!["--permission-mode".to_string(), "auto".to_string()],
        ),
        (
            PermissionMode::Bypass,
            vec![
                "--permission-mode".to_string(),
                "bypassPermissions".to_string(),
            ],
        ),
    ] {
        let user = vec!["--mcp-debug".to_string()];
        let out = apply_permission_mode(Runtime::ClaudeCode.key(), &user, mode);
        let mut want = vec!["--mcp-debug".to_string()];
        want.extend(expected_extra);
        assert_eq!(out, want, "mode={mode:?}");
    }
}

#[test]
fn apply_permission_mode_claude_code_cycles_cleanly() {
    // AcceptEdits → Auto → Bypass → Default with a custom flag
    // in the middle. Each transition must end with the canonical
    // args for the chosen mode, never an accumulation.
    let mut args = vec!["--mcp-debug".to_string()];
    args = apply_permission_mode(
        Runtime::ClaudeCode.key(),
        &args,
        PermissionMode::AcceptEdits,
    );
    assert_eq!(
        args,
        vec![
            "--mcp-debug".to_string(),
            "--permission-mode".to_string(),
            "acceptEdits".to_string(),
        ],
    );
    args = apply_permission_mode(Runtime::ClaudeCode.key(), &args, PermissionMode::Auto);
    assert_eq!(
        args,
        vec![
            "--mcp-debug".to_string(),
            "--permission-mode".to_string(),
            "auto".to_string(),
        ],
        "cycling to Auto must replace the prior --permission-mode value, not stack",
    );
    args = apply_permission_mode(Runtime::ClaudeCode.key(), &args, PermissionMode::Bypass);
    assert_eq!(
        args,
        vec![
            "--mcp-debug".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
        ],
    );
    args = apply_permission_mode(Runtime::ClaudeCode.key(), &args, PermissionMode::Default);
    assert_eq!(args, vec!["--mcp-debug".to_string()]);
}

#[test]
fn apply_permission_mode_claude_code_strips_legacy_dangerous_flag() {
    // Pre-rename rows carried `--dangerously-skip-permissions`
    // for the Bypass state. The strip helper must drop it on
    // any mode change so the row converges to the new
    // `--permission-mode <value>` shape rather than carrying
    // both side-by-side.
    let user = vec![
        "--mcp-debug".to_string(),
        "--dangerously-skip-permissions".to_string(),
    ];
    let out = apply_permission_mode(Runtime::ClaudeCode.key(), &user, PermissionMode::Bypass);
    assert_eq!(
        out,
        vec![
            "--mcp-debug".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
        ],
        "legacy flag stripped; canonical --permission-mode bypassPermissions added",
    );
}

#[test]
fn apply_mission_permission_mode_keeps_unrelated_claude_args() {
    let row = vec![
        "--permission-mode".to_string(),
        "plan".to_string(),
        "--model".to_string(),
        "opus".to_string(),
    ];
    assert_eq!(
        apply_mission_permission_mode(
            Runtime::ClaudeCode.key(),
            &row,
            MissionPermissionMode::Bypass
        ),
        vec![
            "--model".to_string(),
            "opus".to_string(),
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
        ],
    );
    assert_eq!(
        apply_mission_permission_mode(Runtime::ClaudeCode.key(), &row, MissionPermissionMode::Auto),
        vec![
            "--model".to_string(),
            "opus".to_string(),
            "--permission-mode".to_string(),
            "auto".to_string(),
        ],
    );
    assert_eq!(
        apply_mission_permission_mode(
            Runtime::ClaudeCode.key(),
            &row,
            MissionPermissionMode::RoleDefault
        ),
        row,
    );
}

#[test]
fn infer_permission_mode_claude_code_each_state() {
    assert_eq!(
        infer_permission_mode(Runtime::ClaudeCode.key(), &["--mcp-debug".into()]),
        PermissionMode::Default,
    );
    assert_eq!(
        infer_permission_mode(
            Runtime::ClaudeCode.key(),
            &["--permission-mode".into(), "acceptEdits".into()],
        ),
        PermissionMode::AcceptEdits,
    );
    assert_eq!(
        infer_permission_mode(
            Runtime::ClaudeCode.key(),
            &["--permission-mode=acceptEdits".into()]
        ),
        PermissionMode::AcceptEdits,
    );
    assert_eq!(
        infer_permission_mode(
            Runtime::ClaudeCode.key(),
            &["--permission-mode".into(), "auto".into()],
        ),
        PermissionMode::Auto,
    );
    assert_eq!(
        infer_permission_mode(
            Runtime::ClaudeCode.key(),
            &["--permission-mode".into(), "bypassPermissions".into()],
        ),
        PermissionMode::Bypass,
    );
}

#[test]
fn infer_permission_mode_claude_code_legacy_dangerous_flag_reads_as_bypass() {
    // Pre-rename rows used `--dangerously-skip-permissions` for
    // Bypass. The dropdown must still load Bypass for those rows
    // so a save converges them to `--permission-mode
    // bypassPermissions`.
    let args = vec!["--dangerously-skip-permissions".to_string()];
    assert_eq!(
        infer_permission_mode(Runtime::ClaudeCode.key(), &args),
        PermissionMode::Bypass,
    );
}

#[test]
fn infer_permission_mode_claude_code_bypass_wins_over_accept_edits() {
    // A row carrying both `--permission-mode acceptEdits` AND
    // the legacy `--dangerously-skip-permissions` flag resolves
    // to Bypass — bypass is strictly more aggressive, and the
    // strip-and-replace round-trip on save converges the row to
    // a single canonical pair.
    let args = vec![
        "--permission-mode".to_string(),
        "acceptEdits".to_string(),
        "--dangerously-skip-permissions".to_string(),
    ];
    assert_eq!(
        infer_permission_mode(Runtime::ClaudeCode.key(), &args),
        PermissionMode::Bypass,
    );
}

#[test]
fn strip_permission_flags_drops_claude_code_permission_mode() {
    // Cycling through modes shouldn't leave orphan
    // `--permission-mode acceptEdits` when the user picks Bypass
    // or Default afterward.
    let user = vec![
        "--permission-mode".to_string(),
        "acceptEdits".to_string(),
        "--debug".to_string(),
        "--dangerously-skip-permissions".to_string(),
    ];
    let out = strip_permission_flags(Runtime::ClaudeCode.key(), &user);
    assert_eq!(out, vec!["--debug".to_string()]);
}

#[test]
fn claude_code_trailing_args_unaffected_by_resume_flag_when_first_turn_absent() {
    // claude-code's `system_prompt_args` is empty (the persona
    // stub rides via stdin). With `first_turn = None`, the
    // `plan_resuming` flag has no effect — the trailing args
    // are just the model/effort pair.
    let fresh = trailing_runtime_args(
        Runtime::ClaudeCode.key(),
        &[],
        Path::new("/tmp/runner-app-data"),
        "runner-session",
        false,
        Some("claude-opus-4-7"),
        Some("xhigh"),
        None,
        Some("be helpful"),
        None,
    );
    let resuming = trailing_runtime_args(
        Runtime::ClaudeCode.key(),
        &[],
        Path::new("/tmp/runner-app-data"),
        "runner-session",
        true,
        Some("claude-opus-4-7"),
        Some("xhigh"),
        None,
        Some("be helpful"),
        None,
    );
    assert_eq!(fresh, resuming);
    assert_eq!(
        &fresh[..5],
        [
            "--model",
            "claude-opus-4-7",
            "--effort",
            "xhigh",
            "--settings"
        ]
    );
    let settings: serde_json::Value = serde_json::from_str(&fresh[5]).unwrap();
    assert_eq!(settings["tui"], "fullscreen");
    assert!(settings["hooks"]["SessionStart"].is_array());
}

fn claude_settings_args_for(key: &str, args: &[String], data: &Path, session: &str) -> Vec<String> {
    if key == Runtime::ClaudeCode.key() {
        claude_settings_args(args, data, session)
    } else {
        Vec::new()
    }
}
