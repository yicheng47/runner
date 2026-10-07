use super::*;
use crate::runtimes::test_support::*;

#[test]
fn rollout_capture_resolves_spawn_home_then_runner_home_then_default() {
    let root = tempfile::tempdir().unwrap();
    let cwd = root.path().join("project");
    let runner_home = root.path().join("runner-codex");
    let role_home = root.path().join("role-codex");
    let mut spec = SpawnSpec {
        cwd: Some(cwd.clone()),
        ..Default::default()
    };
    with_conversation_home(root.path(), || {
        crate::golden::with_config_env(
            std::collections::BTreeMap::from([("CODEX_HOME", runner_home.as_os_str().to_owned())]),
            || {
                let resolved = |spec: &SpawnSpec| {
                    let KeyCapture::RolloutScan { sessions_root } =
                        Codex.key_capture_for_spawn(spec)
                    else {
                        panic!("expected rollout scan")
                    };
                    sessions_root
                };
                assert_eq!(resolved(&spec), Some(runner_home.join("sessions")));
                spec.env.insert(
                    "CODEX_HOME".into(),
                    role_home.to_string_lossy().into_owned(),
                );
                assert_eq!(resolved(&spec), Some(role_home.join("sessions")));
                spec.env.insert("CODEX_HOME".into(), "relative-home".into());
                assert_eq!(resolved(&spec), Some(cwd.join("relative-home/sessions")));
                spec.env.insert("CODEX_HOME".into(), "   ".into());
                assert_eq!(resolved(&spec), Some(runner_home.join("sessions")));
            },
        );
        spec.env.clear();
        crate::golden::with_config_env(Default::default(), || {
            let KeyCapture::RolloutScan { sessions_root } = Codex.key_capture_for_spawn(&spec)
            else {
                panic!("expected rollout scan")
            };
            assert_eq!(sessions_root, Some(root.path().join(".codex/sessions")));
        });
        crate::golden::with_config_env(
            std::collections::BTreeMap::from([(
                "CODEX_HOME",
                std::ffi::OsString::from("runner-relative"),
            )]),
            || {
                let KeyCapture::RolloutScan { sessions_root } = Codex.key_capture_for_spawn(&spec)
                else {
                    panic!("expected rollout scan")
                };
                assert_eq!(sessions_root, Some(cwd.join("runner-relative/sessions")));
                spec.cwd = None;
                let KeyCapture::RolloutScan { sessions_root } = Codex.key_capture_for_spawn(&spec)
                else {
                    panic!("expected rollout scan")
                };
                assert_eq!(
                    sessions_root,
                    Some(
                        std::env::current_dir()
                            .unwrap()
                            .join("runner-relative/sessions")
                    )
                );
            },
        );
    });
}

#[test]
fn codex_hook_overrides_and_opt_out_preserve_the_invocation() {
    for args in [
        vec!["--disable", "hooks"],
        vec!["--disable=hooks"],
        vec!["-c", "features.hooks=false"],
        vec!["--config=features={hooks=false}"],
        vec!["-c'features'.\"hooks\" = false"],
        vec!["-c=features.hooks=false"],
        vec!["--config", "hooks.Stop=[]"],
        vec!["-chooks={}"],
        vec!["-c", "'hooks'.Stop=[]"],
    ] {
        let args = args.into_iter().map(String::from).collect::<Vec<_>>();
        assert!(
            !inject_codex_hooks_for(Runtime::Codex.key(), &args, false),
            "{args:?}"
        );
        assert!(codex_status_args_for(
            Runtime::Codex.key(),
            &args,
            Path::new("/unused"),
            "session"
        )
        .is_empty());
    }
    for args in [
        vec![],
        vec!["-c", "model=fixture-model"],
        vec!["--config=features.hooks=true"],
        vec!["-c", "sandbox_mode=read-only"],
    ] {
        let args = args.into_iter().map(String::from).collect::<Vec<_>>();
        assert!(
            inject_codex_hooks_for(Runtime::Codex.key(), &args, false),
            "{args:?}"
        );
    }
    assert!(inject_codex_hooks_for(Runtime::Codex.key(), &[], true));
    for runtime in [
        None,
        Some(Runtime::ClaudeCode),
        Some(Runtime::Trae),
        Some(Runtime::Copilot),
        Some(Runtime::Pi),
        Some(Runtime::Shell),
    ] {
        assert!(!inject_codex_hooks_for(
            runtime.map(Runtime::key).unwrap_or(""),
            &[],
            false
        ));
        assert!(!inject_codex_hooks_for(
            runtime.map(Runtime::key).unwrap_or(""),
            &[],
            true
        ));
    }
}

#[test]
fn codex_injection_uses_structured_stdio_and_schema_guaranteed_projections() {
    use runner_core::protocol::hook;
    let root = Path::new("C:/Users/空 格 triple ''' dollar $ backtick `/runner");
    let args = codex_status_args_for(Runtime::Codex.key(), &[], root, "session");
    assert_eq!(
        &args[..3],
        &["--enable", "hooks", "--dangerously-bypass-hook-trust"]
    );
    assert_eq!(args.len(), 5 + 2 * codex_status::EVENTS.len());
    let config = args[4].parse::<toml_edit::DocumentMut>().unwrap();
    let server = &config["mcp_servers"]["runner_hooks"];
    assert_eq!(
        server["command"].as_str(),
        Some(hook_executable(root).to_string_lossy().as_ref())
    );
    assert_eq!(server["required"].as_bool(), Some(false));
    assert!(server.get("startup_timeout_sec").is_none());
    assert_eq!(
        server["args"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>(),
        ["hook", "serve"]
    );
    assert_eq!(
        server["env_vars"]
            .as_array()
            .unwrap()
            .iter()
            .map(|value| value.as_str().unwrap())
            .collect::<Vec<_>>(),
        [hook::ENDPOINT_ENV, hook::SESSION_ENV, hook::GENERATION_ENV]
    );
    for (pair, event) in args[5..].chunks_exact(2).zip(codex_status::EVENTS) {
        assert_eq!(pair[0], "-c");
        let config = pair[1].parse::<toml_edit::DocumentMut>().unwrap();
        let groups = config["hooks"][event].as_array().unwrap();
        let handler = groups.get(0).unwrap().as_inline_table().unwrap()["hooks"]
            .as_array()
            .unwrap()
            .get(0)
            .unwrap()
            .as_inline_table()
            .unwrap();
        if *event == "SessionEnd" {
            assert_eq!(handler["type"].as_str(), Some("command"));
            assert_eq!(handler["timeout"].as_integer(), Some(1));
            assert_eq!(
                handler["command"].as_str(),
                Some(hook_report_command(root, Runtime::Codex, event, cfg!(windows)).as_str())
            );
            assert!(handler.get("server").is_none());
            assert!(handler.get("input").is_none());
            continue;
        }
        assert_eq!(handler["type"].as_str(), Some("mcp_tool"));
        assert_eq!(handler["server"].as_str(), Some("runner_hooks"));
        assert_eq!(handler["tool"].as_str(), Some("report"));
        assert!(handler.get("command").is_none());
        let input = handler["input"].as_inline_table().unwrap();
        let mut fields = vec!["hook_event_name", "session_id", "transcript_path"];
        if *event == "SessionStart" {
            fields.push("source");
        } else {
            fields.push("turn_id");
        }
        if matches!(*event, "PreCompact" | "PostCompact") {
            fields.push("trigger");
        }
        assert_eq!(input.len(), fields.len());
        for field in fields {
            assert_eq!(
                input[field].as_str(),
                Some(format!("${{{field}}}").as_str())
            );
        }
    }
    let line = args.iter().map(|arg| arg.len() + 3).sum::<usize>();
    assert!(line < 8191, "Codex hook argv is {line} characters");
}

#[test]
fn codex_runtime_returns_no_argv_for_system_prompt() {
    // Codex has no dedicated system-prompt flag. Persona/brief
    // delivery is handled through first-turn plumbing, not
    // `system_prompt_args`.
    let args = system_prompt_args(Runtime::Codex.key(), Some("be helpful"));
    assert!(
        args.is_empty(),
        "codex has no native system_prompt argv flag: {args:?}",
    );
}

#[test]
fn codex_runtime_omits_argv_when_prompt_is_blank() {
    assert!(system_prompt_args(Runtime::Codex.key(), None).is_empty());
    assert!(system_prompt_args(Runtime::Codex.key(), Some("")).is_empty());
    assert!(system_prompt_args(Runtime::Codex.key(), Some("   ")).is_empty());
}

#[test]
fn codex_fresh_returns_empty_plan() {
    let plan = resume_plan(Runtime::Codex.key(), None);
    assert!(plan.args.is_empty());
    assert!(plan.assigned_key.is_none());
    assert!(!plan.resuming);
}

#[test]
fn codex_resume_uses_subcommand_prefix() {
    let prior = uuid::Uuid::new_v4().to_string();
    let plan = resume_plan(Runtime::Codex.key(), Some(&prior));
    assert!(plan.resuming);
    assert!(plan.prepend, "codex resume is a subcommand, must prepend");
    assert_eq!(plan.args, vec!["resume", &prior]);
}

#[test]
fn codex_fork_executes_headlessly_and_reads_thread_started() {
    let source = "019fa1b9-a133-7841-b4dd-730d376ab1d1";
    let plan = fork_plan(Runtime::Codex.key(), source, "Source").unwrap();
    let ForkPlan::Headless { args, source_key } = plan else {
        panic!("codex must materialize its fork headlessly")
    };
    assert_eq!(
        args,
        vec![
            "exec",
            "fork",
            source,
            "--json",
            "--skip-git-repo-check",
            "This chat was forked from 'Source'.",
        ]
    );
    assert_eq!(source_key, source);
}

#[test]
fn codex_emits_model_and_reasoning_effort_override() {
    // Issue #41: codex was silently dropping `effort`. Codex has no
    // dedicated reasoning-effort flag; the canonical wiring is via
    // its `-c key=value` config-override flag using the same
    // `model_reasoning_effort` key as `~/.codex/config.toml`.
    let args = model_effort_args(Runtime::Codex.key(), Some("gpt-5-codex"), Some("high"));
    assert!(
        args.windows(2)
            .any(|w| w[0] == "--model" && w[1] == "gpt-5-codex"),
        "expected --model flag, got: {args:?}",
    );
    assert!(
        args.windows(2)
            .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=high"),
        "expected `-c model_reasoning_effort=high`, got: {args:?}",
    );
}

#[test]
fn codex_emits_only_model_when_effort_unset() {
    let args = model_effort_args(Runtime::Codex.key(), Some("gpt-5-codex"), None);
    assert_eq!(args, vec!["--model".to_string(), "gpt-5-codex".to_string()]);
}

#[test]
fn codex_lowercases_effort_for_case_sensitive_toml_enum() {
    // Codex's `model_reasoning_effort` is a case-sensitive TOML
    // enum and rejects "High" with `unknown variant 'High',
    // expected one of 'none', 'minimal', 'low', 'medium', 'high',
    // 'xhigh'`. Rows often store the level title-cased ("High"),
    // so the codex branch normalises before forwarding.
    let args = model_effort_args(Runtime::Codex.key(), Some("gpt-5-codex"), Some("High"));
    assert!(
        args.windows(2)
            .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=high"),
        "expected lowercased effort override, got: {args:?}",
    );
}

#[test]
fn codex_lowercases_mixed_case_effort() {
    let args = model_effort_args(Runtime::Codex.key(), None, Some("XHIGH"));
    assert!(
        args.windows(2)
            .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=xhigh"),
        "expected lowercased effort override, got: {args:?}",
    );
}

#[test]
fn codex_mission_bus_sandbox_args_grants_only_mission_dir() {
    let dir = std::path::PathBuf::from("/tmp/runner/crews/c/missions/m");
    assert_eq!(
        mission_bus_sandbox_args(Runtime::Codex.key(), Some(&dir)),
        vec!["--add-dir".to_string(), dir.to_string_lossy().to_string()],
    );
    assert!(mission_bus_sandbox_args(Runtime::Codex.key(), None).is_empty());
    assert!(mission_bus_sandbox_args(Runtime::ClaudeCode.key(), Some(&dir)).is_empty());
    assert!(mission_bus_sandbox_args(Runtime::Shell.key(), Some(&dir)).is_empty());
}

#[test]
fn codex_trailing_args_omit_positional_prompt() {
    // `system_prompt` is not a positional first turn. Model/effort
    // flags still ride along so the role row's pinned settings
    // reach the spawned CLI.
    for plan_resuming in [false, true] {
        let args = trailing_runtime_args(
            Runtime::Codex.key(),
            &[],
            Path::new("/tmp/runner-app-data"),
            "runner-session",
            plan_resuming,
            Some("gpt-5-codex"),
            Some("high"),
            None,
            Some("be helpful"),
            None,
        );
        assert!(
            !args.iter().any(|a| a == "be helpful"),
            "codex trailing args must not contain the brief as positional argv \
                 (plan_resuming={plan_resuming}): {args:?}",
        );
        assert!(
            args.windows(2)
                .any(|w| w[0] == "--model" && w[1] == "gpt-5-codex"),
            "expected --model flag to survive (plan_resuming={plan_resuming}): {args:?}",
        );
        assert!(
            args.windows(2)
                .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=high"),
            "expected reasoning-effort override to survive \
                 (plan_resuming={plan_resuming}): {args:?}",
        );
    }
}

#[test]
fn apply_permission_mode_codex_appends_auto_pair() {
    let user = vec!["--debug".to_string(), "-v".to_string()];
    let out = apply_permission_mode(Runtime::Codex.key(), &user, PermissionMode::Auto);
    assert_eq!(
        out,
        vec![
            "--debug".to_string(),
            "-v".to_string(),
            "--ask-for-approval".to_string(),
            "on-request".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ],
        "user-provided args come first; canonical auto pair appended",
    );
}

#[test]
fn apply_permission_mode_codex_appends_bypass_pair() {
    let user = vec!["--debug".to_string()];
    let out = apply_permission_mode(Runtime::Codex.key(), &user, PermissionMode::Bypass);
    assert_eq!(
        out,
        vec![
            "--debug".to_string(),
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ],
    );
}

#[test]
fn apply_permission_mode_codex_accept_edits_is_no_op() {
    // Codex has no edits-only middle; AcceptEdits maps to the
    // empty arg list, so applying it strips any pre-existing
    // permission flags but doesn't write codex's pair.
    let user = vec![
        "--debug".to_string(),
        "--ask-for-approval".to_string(),
        "on-request".to_string(),
        "--sandbox".to_string(),
        "workspace-write".to_string(),
    ];
    let out = apply_permission_mode(Runtime::Codex.key(), &user, PermissionMode::AcceptEdits);
    assert_eq!(
        out,
        vec!["--debug".to_string()],
        "AcceptEdits on codex strips permission flags and adds none",
    );
}

#[test]
fn apply_permission_mode_codex_dedupes_existing_flags() {
    // Cycling between modes with stale/conflicting flags already
    // in args must replace them with the canonical pair, not
    // stack duplicates. Covers both `--flag value` and
    // `--flag=value` shapes plus an unrelated user arg in the
    // middle.
    let user = vec![
        "--ask-for-approval".to_string(),
        "untrusted".to_string(),
        "--debug".to_string(),
        "--sandbox=read-only".to_string(),
    ];
    let out = apply_permission_mode(Runtime::Codex.key(), &user, PermissionMode::Bypass);
    assert_eq!(
        out,
        vec![
            "--debug".to_string(),
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ],
    );
}

#[test]
fn apply_permission_mode_codex_default_strips_all_flags() {
    let user = vec![
        "--debug".to_string(),
        "--ask-for-approval".to_string(),
        "never".to_string(),
        "--sandbox".to_string(),
        "workspace-write".to_string(),
    ];
    let out = apply_permission_mode(Runtime::Codex.key(), &user, PermissionMode::Default);
    assert_eq!(out, vec!["--debug".to_string()]);
}

#[test]
fn apply_mission_permission_mode_converges_a_codex_row() {
    let row = vec![
        "--ask-for-approval".to_string(),
        "on-request".to_string(),
        "--sandbox".to_string(),
        "workspace-write".to_string(),
    ];
    assert_eq!(
        apply_mission_permission_mode(Runtime::Codex.key(), &row, MissionPermissionMode::Bypass),
        vec![
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "--sandbox".to_string(),
            "danger-full-access".to_string(),
        ],
    );
    assert_eq!(
        apply_mission_permission_mode(
            Runtime::Codex.key(),
            &row,
            MissionPermissionMode::RoleDefault
        ),
        row,
    );
}

#[test]
fn infer_permission_mode_codex_separated_form() {
    let args = vec![
        "--ask-for-approval".to_string(),
        "never".to_string(),
        "--sandbox".to_string(),
        "workspace-write".to_string(),
    ];
    assert_eq!(
        infer_permission_mode(Runtime::Codex.key(), &args),
        PermissionMode::Bypass,
    );
}

#[test]
fn infer_permission_mode_codex_equals_form() {
    // Equals-form must match too — same bug coverage as the
    // pre-rewrite test that locked in the frontend hand-port.
    let args = vec![
        "--ask-for-approval=never".to_string(),
        "--sandbox=workspace-write".to_string(),
    ];
    assert_eq!(
        infer_permission_mode(Runtime::Codex.key(), &args),
        PermissionMode::Bypass,
    );
}

#[test]
fn infer_permission_mode_codex_auto_pair() {
    let args = vec![
        "--ask-for-approval".to_string(),
        "on-request".to_string(),
        "--sandbox=workspace-write".to_string(),
    ];
    assert_eq!(
        infer_permission_mode(Runtime::Codex.key(), &args),
        PermissionMode::Auto
    );
}

#[test]
fn infer_permission_mode_codex_partial_match_falls_back_to_default() {
    // --sandbox is present, --ask-for-approval is not. Neither
    // pair fully matches → default.
    let args = vec!["--sandbox=workspace-write".to_string()];
    assert_eq!(
        infer_permission_mode(Runtime::Codex.key(), &args),
        PermissionMode::Default,
    );
}

#[test]
fn infer_permission_mode_codex_deprecated_value_falls_back_to_default() {
    // `on-failure` is deprecated per `codex --help` and not
    // exposed in the dropdown. A row carrying it (e.g. created
    // by an older Runner build) reads as Default — neither
    // Auto nor Bypass match — so the dropdown lands on Default
    // and a save converges the row to one of the new canonical
    // shapes.
    let args = vec![
        "--ask-for-approval=on-failure".to_string(),
        "--sandbox=workspace-write".to_string(),
    ];
    assert_eq!(
        infer_permission_mode(Runtime::Codex.key(), &args),
        PermissionMode::Default,
    );
}

fn inject_codex_hooks_for(key: &str, args: &[String], windows: bool) -> bool {
    for_key(key)
        .status_hooks()
        .is_some_and(|hooks| hooks.supported(windows))
        && key == Runtime::Codex.key()
        && inject_codex_hooks(args, windows)
}
fn codex_status_args_for(key: &str, args: &[String], data: &Path, session: &str) -> Vec<String> {
    if key == Runtime::Codex.key() {
        codex_status_args(args, data, session)
    } else {
        Vec::new()
    }
}
