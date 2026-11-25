use super::*;
use crate::runtimes::test_support::*;

#[test]
fn opencode_optional_extensions_keep_existing_agents_on_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let env = std::collections::BTreeMap::from([
        ("XDG_DATA_HOME".into(), "alternate-data".into()),
        ("XDG_CONFIG_HOME".into(), "alternate-config".into()),
        ("OPENCODE_CONFIG_CONTENT".into(), "{}".into()),
    ]);
    for runtime in Runtime::ALL
        .into_iter()
        .filter(|runtime| *runtime != Runtime::OpenCode)
    {
        let adapter = adapter(runtime);
        assert!(adapter.mcp_json_parser().is_none(), "{runtime:?}");
        assert!(
            adapter.read_mcp_entries(directory.path()).is_none(),
            "{runtime:?}"
        );
        assert!(
            adapter
                .write_mcp_entry(directory.path(), "server", None, false)
                .is_none(),
            "{runtime:?}"
        );
        assert!(
            adapter
                .skill_roots_with_env(directory.path(), None, &env)
                .is_none(),
            "{runtime:?}"
        );
        for resuming in [false, true] {
            assert!(
                adapter
                    .launch_context_env(&LaunchContext {
                        env: &env,
                        cwd: Some(directory.path()),
                        default_model: Some("provider/default"),
                        role_args: &[],
                        app_data_dir: directory.path(),
                        session_id: "fixture",
                        resuming,
                        mission: !resuming,
                        model: Some("provider/override"),
                        effort: Some("high"),
                        codex_speed: None,
                        codex_tui_workspace: false,
                        system_prompt: None,
                        first_turn: Some("fixture prompt"),
                    })
                    .is_empty(),
                "{runtime:?} resuming={resuming}"
            );
        }
    }
}

#[test]
fn shell_runtime_omits_flag() {
    assert!(system_prompt_args(Runtime::Shell.key(), Some("ignored")).is_empty());
}

#[test]
fn missing_or_blank_prompt_omits_flag() {
    assert!(system_prompt_args(Runtime::ClaudeCode.key(), None).is_empty());
    assert!(system_prompt_args(Runtime::ClaudeCode.key(), Some("")).is_empty());
    assert!(system_prompt_args(Runtime::ClaudeCode.key(), Some("   ")).is_empty());
}

#[test]
fn unknown_runtime_degrades_to_no_flag() {
    assert!(system_prompt_args("aider-future", Some("hi")).is_empty());
}

#[test]
fn unknown_runtime_returns_empty_resume_plan() {
    let plan = resume_plan("aider-future", Some("anything"));
    assert!(plan.args.is_empty());
    assert!(plan.assigned_key.is_none());
    assert!(!plan.resuming);
}

#[test]
fn native_fork_capability_comes_from_runtime_definition() {
    assert!(supports_native_fork(Runtime::ClaudeCode.key()));
    assert!(supports_native_fork(Runtime::Codex.key()));
    assert!(supports_native_fork(Runtime::Pi.key()));
    assert!(!supports_native_fork(Runtime::Trae.key()));
    assert!(!supports_native_fork(Runtime::Copilot.key()));
    assert!(!supports_native_fork(Runtime::Antigravity.key()));
    assert!(!supports_native_fork("aider-future"));
}

#[test]
fn native_fork_capability_matches_available_fork_plans() {
    let source = "019fa1b9-a133-7841-b4dd-730d376ab1d1";
    for definition in runtime_definitions() {
        assert_eq!(
            definition.native_fork,
            fork_plan(definition.name.key(), source, "Source").is_some(),
            "runtime {}",
            definition.name,
        );
    }
}

#[test]
fn unsupported_or_invalid_fork_returns_none() {
    let source = "019fa1b9-a133-7841-b4dd-730d376ab1d1";
    assert!(fork_plan(Runtime::Trae.key(), source, "note").is_none());
    assert!(fork_plan("aider-future", source, "note").is_none());
    assert!(fork_plan(Runtime::Codex.key(), "not-a-uuid", "note").is_none());
}

#[test]
fn permission_mode_args_per_runtime() {
    // Default → no flags for any runtime / any mode.
    assert!(permission_mode_args(Runtime::ClaudeCode.key(), PermissionMode::Default).is_empty());
    assert!(permission_mode_args(Runtime::Codex.key(), PermissionMode::Default).is_empty());
    assert!(permission_mode_args(Runtime::Trae.key(), PermissionMode::Default).is_empty());
    assert!(permission_mode_args(Runtime::Pi.key(), PermissionMode::Default).is_empty());
    // claude-code: AcceptEdits / Auto / Bypass each emit
    // `--permission-mode <value>` with a runtime-specific value.
    assert_eq!(
        permission_mode_args(Runtime::ClaudeCode.key(), PermissionMode::AcceptEdits),
        vec!["--permission-mode".to_string(), "acceptEdits".to_string()],
    );
    assert_eq!(
        permission_mode_args(Runtime::ClaudeCode.key(), PermissionMode::Auto),
        vec!["--permission-mode".to_string(), "auto".to_string()],
    );
    assert_eq!(
        permission_mode_args(Runtime::ClaudeCode.key(), PermissionMode::Bypass),
        vec![
            "--permission-mode".to_string(),
            "bypassPermissions".to_string(),
        ],
    );
    // codex: AcceptEdits has no equivalent (returns empty);
    // Auto uses on-request (on-failure is deprecated per
    // `codex --help`); Bypass uses never.
    assert!(permission_mode_args(Runtime::Codex.key(), PermissionMode::AcceptEdits).is_empty());
    assert_eq!(
        permission_mode_args(Runtime::Codex.key(), PermissionMode::Auto),
        vec![
            "--ask-for-approval".to_string(),
            "on-request".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ],
    );
    assert_eq!(
        permission_mode_args(Runtime::Codex.key(), PermissionMode::Bypass),
        vec![
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ],
    );
    assert!(permission_mode_args(Runtime::Trae.key(), PermissionMode::AcceptEdits).is_empty());
    assert!(permission_mode_args(Runtime::Trae.key(), PermissionMode::Auto).is_empty());
    assert_eq!(
        permission_mode_args(Runtime::Trae.key(), PermissionMode::Bypass),
        vec![
            "--permission-mode".to_string(),
            "bypass_permissions".to_string(),
        ],
    );
    for mode in [
        PermissionMode::Default,
        PermissionMode::AcceptEdits,
        PermissionMode::Auto,
        PermissionMode::Bypass,
    ] {
        assert!(permission_mode_args(Runtime::Pi.key(), mode).is_empty());
        assert_eq!(
            infer_permission_mode(Runtime::Pi.key(), &["--whatever".into()]),
            PermissionMode::Default
        );
    }
    // Unknown runtime → empty for every mode.
    for mode in [
        PermissionMode::Default,
        PermissionMode::AcceptEdits,
        PermissionMode::Auto,
        PermissionMode::Bypass,
    ] {
        assert!(permission_mode_args(Runtime::Shell.key(), mode).is_empty());
        assert!(permission_mode_args("aider-future", mode).is_empty());
    }
}

#[test]
fn apply_permission_mode_no_op_for_unsupported_runtime() {
    let user = vec!["--whatever".to_string()];
    for mode in [
        PermissionMode::Default,
        PermissionMode::AcceptEdits,
        PermissionMode::Auto,
        PermissionMode::Bypass,
    ] {
        assert_eq!(
            apply_permission_mode(Runtime::Shell.key(), &user, mode),
            user,
            "shell must be a no-op (mode={mode:?})",
        );
    }
}

#[test]
fn mission_permission_args_per_runtime() {
    assert_eq!(
        mission_permission_args(Runtime::ClaudeCode.key()),
        vec!["--permission-mode", "bypassPermissions"],
    );
    // Codex cannot ask to escalate with `never`, so missions need full access.
    assert_eq!(
        mission_permission_args(Runtime::Codex.key()),
        vec![
            "--ask-for-approval",
            "never",
            "--sandbox",
            "danger-full-access"
        ],
    );
    assert_eq!(
        mission_permission_args(Runtime::Trae.key()),
        vec!["--permission-mode", "bypass_permissions"],
    );
    assert_eq!(
        mission_permission_args(Runtime::Antigravity.key()),
        vec!["--dangerously-skip-permissions"],
    );
    assert!(mission_permission_args(Runtime::Shell.key()).is_empty());
    assert!(mission_permission_args("unknown").is_empty());
    // The role-level Codex Bypass mapping is untouched.
    assert_eq!(
        permission_mode_args(Runtime::Codex.key(), PermissionMode::Bypass),
        vec![
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "--sandbox".to_string(),
            "workspace-write".to_string(),
        ],
    );
}

#[test]
fn apply_mission_permissions_leaves_shell_and_unknown_alone() {
    let row = vec!["--whatever".to_string()];
    for runtime in ["pi", "shell", "aider-future"] {
        assert_eq!(apply_mission_permissions(runtime, &row), row, "{runtime}");
    }
}

#[test]
fn infer_permission_mode_unsupported_runtime_default() {
    let args = vec!["--whatever".to_string()];
    assert_eq!(
        infer_permission_mode(Runtime::Shell.key(), &args),
        PermissionMode::Default,
    );
    assert_eq!(
        infer_permission_mode("aider-future", &args),
        PermissionMode::Default,
    );
}

#[test]
fn strip_permission_flags_handles_dangling_value() {
    // If `--ask-for-approval` is the last token (no value follows
    // — the user mid-typed), strip just the flag and don't panic
    // on the missing pair.
    let user = vec!["--debug".to_string(), "--ask-for-approval".to_string()];
    let out = strip_permission_flags(Runtime::Codex.key(), &user);
    assert_eq!(out, vec!["--debug".to_string()]);
}

#[test]
fn first_turn_rides_trailing_argv_on_fresh_spawn_for_supported_runtimes() {
    for runtime in [
        "claude-code",
        "codex",
        "trae",
        "copilot",
        "pi",
        "antigravity",
    ] {
        let body = "You are the architect. Goal: ship 0007.";
        let args = trailing_runtime_args(
            runtime,
            &[],
            Path::new("/tmp/runner-app-data"),
            "runner-session",
            false,
            Some("model-x"),
            Some("high"),
            None,
            Some("persona"),
            Some(body),
        );
        // Body lands as the trailing positional.
        assert_eq!(args.last().map(String::as_str), Some(body));
        assert!(args
            .windows(2)
            .any(|w| w[0] == "--model" && w[1] == "model-x"));
    }
}

#[test]
fn first_turn_suppressed_on_resume_for_supported_runtimes() {
    for runtime in [
        "claude-code",
        "codex",
        "trae",
        "copilot",
        "pi",
        "antigravity",
    ] {
        let body = "You are the architect. Goal: ship 0007.";
        let args = trailing_runtime_args(
            runtime,
            &[],
            Path::new("/tmp/runner-app-data"),
            "runner-session",
            true,
            Some("model-x"),
            Some("high"),
            None,
            Some("persona"),
            Some(body),
        );
        assert!(
            !args.iter().any(|a| a == body),
            "resume must not replay the first-turn body as positional argv ({runtime}): {args:?}"
        );
    }
}

#[test]
fn first_turn_argv_empty_for_blank_or_unsupported_runtime() {
    assert!(first_turn_argv(Runtime::ClaudeCode.key(), Some("   \n  ")).is_empty());
    assert!(first_turn_argv(Runtime::ClaudeCode.key(), None).is_empty());
    assert!(first_turn_argv(Runtime::Shell.key(), Some("body")).is_empty());
    assert!(first_turn_argv("unknown", Some("body")).is_empty());
}

#[test]
fn accounts_hook_defaults_to_none_for_other_runtimes() {
    for runtime in Runtime::ALL {
        let supported = adapter(runtime).accounts().is_some();
        assert_eq!(
            supported,
            runtime == Runtime::ClaudeCode,
            "unexpected accounts hook for {runtime}"
        );
    }
    assert!(for_key("unknown").accounts().is_none());
}
