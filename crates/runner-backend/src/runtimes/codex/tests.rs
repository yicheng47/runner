use super::*;
use crate::runtimes::test_support::*;

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
            !inject_codex_hooks(Some(Runtime::Codex), &args, false),
            "{args:?}"
        );
        assert!(
            codex_status_args(Some(Runtime::Codex), &args, Path::new("/unused"), "session")
                .is_empty()
        );
    }
    for args in [
        vec![],
        vec!["-c", "model=fixture-model"],
        vec!["--config=features.hooks=true"],
        vec!["-c", "sandbox_mode=read-only"],
    ] {
        let args = args.into_iter().map(String::from).collect::<Vec<_>>();
        assert!(
            inject_codex_hooks(Some(Runtime::Codex), &args, false),
            "{args:?}"
        );
    }
    assert!(inject_codex_hooks(Some(Runtime::Codex), &[], true));
    for runtime in [
        None,
        Some(Runtime::ClaudeCode),
        Some(Runtime::Trae),
        Some(Runtime::Copilot),
        Some(Runtime::Pi),
        Some(Runtime::Shell),
    ] {
        assert!(!inject_codex_hooks(runtime, &[], false));
        assert!(!inject_codex_hooks(runtime, &[], true));
    }
}

#[cfg(unix)]
#[test]
fn codex_injection_roundtrips_toml_and_shell_metacharacters() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir
        .path()
        .join("spaces \"double\" triple ''' dollar $ backtick `");
    let args = codex_status_args(Some(Runtime::Codex), &[], &root, "session");
    assert_eq!(
        &args[..3],
        &["--enable", "hooks", "--dangerously-bypass-hook-trust"]
    );
    for (pair, event) in args[3..]
        .chunks_exact(2)
        .zip(crate::session::codex_status::EVENTS)
    {
        assert_eq!(pair[0], "-c");
        let config = pair[1].parse::<toml_edit::DocumentMut>().unwrap();
        let groups = config["hooks"][event].as_array().unwrap();
        assert_eq!(groups.len(), 1);
        let handlers = groups.get(0).unwrap().as_inline_table().unwrap()["hooks"]
            .as_array()
            .unwrap();
        assert_eq!(handlers.len(), 1);
        let hook = handlers.get(0).unwrap().as_inline_table().unwrap();
        assert_eq!(hook["timeout"].as_integer(), Some(2));
        let command = hook["command"].as_str().unwrap();
        let path = crate::session::hook_feed::status_path(&root, "session");
        assert_eq!(
            command,
            crate::session::codex_status::hook_command(&path, event)
        );
        let result = std::process::Command::new("sh")
            .args(["-c", command])
            .output()
            .unwrap();
        assert!(result.status.success());
        assert_eq!(result.stdout, b"{}\n");
        assert!(result.stderr.is_empty());
    }
    assert!(!root.exists());
}

#[cfg(windows)]
#[test]
fn codex_injection_on_windows_calls_the_session_reporter_script() {
    use crate::session::codex_status::{self, EVENTS};
    use crate::session::hook_feed::{hook_path, powershell_script_path, status_path};
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("spaces triple ''' dollar $ backtick `");
    let args = codex_status_args(Some(Runtime::Codex), &[], &root, "session");
    assert_eq!(
        &args[..3],
        &["--enable", "hooks", "--dangerously-bypass-hook-trust"]
    );
    assert_eq!(args.len(), 3 + 2 * EVENTS.len());
    let path = status_path(&root, "session");
    let quote = |path: &std::path::Path| hook_path(path).replace('\'', "''");
    for (pair, event) in args[3..].chunks_exact(2).zip(EVENTS) {
        assert_eq!(pair[0], "-c");
        let command = codex_status::hook_command(&path, event);
        assert_eq!(
            command,
            format!(
                "try{{& ([ScriptBlock]::Create([IO.File]::ReadAllText('{}'))) '{}' '{event}'}}\
                     catch{{[Console]::OpenStandardInput().CopyTo([IO.Stream]::Null)}};'{{}}'",
                quote(&powershell_script_path(&path)),
                quote(&path),
            )
        );
        assert_eq!(
            pair[1],
            format!(
                "hooks.{event}=[{{hooks=[{{type=\"command\",command={},timeout=2}}]}}]",
                toml_edit::Value::from(command.clone())
            )
        );
        let config = pair[1].parse::<toml_edit::DocumentMut>().unwrap();
        let hook = config["hooks"][event]
            .as_array()
            .unwrap()
            .get(0)
            .unwrap()
            .as_inline_table()
            .unwrap()["hooks"]
            .as_array()
            .unwrap()
            .get(0)
            .unwrap()
            .as_inline_table()
            .unwrap();
        assert_eq!(hook["command"].as_str(), Some(command.as_str()));
        assert!(!command.contains('"') && !command.contains('\\'));
    }
    // Without its script (setup failed or the session ended) the hook drains stdin.
    let command = codex_status::hook_command(&path, "Stop");
    for shell in crate::session::hook_feed::POWERSHELLS {
        if let Some(output) =
            crate::session::hook_feed::run_powershell(shell, &command, &[], &vec![b'x'; 256 * 1024])
        {
            assert!(output.status.success(), "{shell}: {output:?}");
            assert_eq!(output.stdout, b"{}\r\n");
            assert!(output.stderr.is_empty(), "{shell}: {output:?}");
        }
    }
    assert!(!root.exists());

    let app_data = std::path::Path::new(
        r"C:\Users\Jason Wang (Runner Windows Smoke)\AppData\Roaming\com.wycstudios.runner-dev",
    );
    let args = codex_status_args(
        Some(Runtime::Codex),
        &[],
        app_data,
        "01M2NCJRAFFVFJQBA0NGDWMDXR",
    );
    let line = args.iter().map(|arg| arg.len() + 3).sum::<usize>();
    assert!(line < 8191, "Codex hook argv is {line} characters");
}

#[test]
fn codex_runtime_returns_no_argv_for_system_prompt() {
    // Codex has no dedicated system-prompt flag. Persona/brief
    // delivery is handled through first-turn plumbing, not
    // `system_prompt_args`.
    let args = system_prompt_args(Some(Runtime::Codex), Some("be helpful"));
    assert!(
        args.is_empty(),
        "codex has no native system_prompt argv flag: {args:?}",
    );
}

#[test]
fn codex_runtime_omits_argv_when_prompt_is_blank() {
    assert!(system_prompt_args(Some(Runtime::Codex), None).is_empty());
    assert!(system_prompt_args(Some(Runtime::Codex), Some("")).is_empty());
    assert!(system_prompt_args(Some(Runtime::Codex), Some("   ")).is_empty());
}

#[test]
fn codex_fresh_returns_empty_plan() {
    let plan = resume_plan(Some(Runtime::Codex), None);
    assert!(plan.args.is_empty());
    assert!(plan.assigned_key.is_none());
    assert!(!plan.resuming);
}

#[test]
fn codex_resume_uses_subcommand_prefix() {
    let prior = uuid::Uuid::new_v4().to_string();
    let plan = resume_plan(Some(Runtime::Codex), Some(&prior));
    assert!(plan.resuming);
    assert!(plan.prepend, "codex resume is a subcommand, must prepend");
    assert_eq!(plan.args, vec!["resume", &prior]);
}

#[test]
fn codex_fork_executes_headlessly_and_reads_thread_started() {
    let source = "019fa1b9-a133-7841-b4dd-730d376ab1d1";
    let plan = fork_plan(Some(Runtime::Codex), source, "Source").unwrap();
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
    let args = model_effort_args(Some(Runtime::Codex), Some("gpt-5-codex"), Some("high"));
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
    let args = model_effort_args(Some(Runtime::Codex), Some("gpt-5-codex"), None);
    assert_eq!(args, vec!["--model".to_string(), "gpt-5-codex".to_string()]);
}

#[test]
fn codex_lowercases_effort_for_case_sensitive_toml_enum() {
    // Codex's `model_reasoning_effort` is a case-sensitive TOML
    // enum and rejects "High" with `unknown variant 'High',
    // expected one of 'none', 'minimal', 'low', 'medium', 'high',
    // 'xhigh'`. Rows often store the level title-cased ("High"),
    // so the codex branch normalises before forwarding.
    let args = model_effort_args(Some(Runtime::Codex), Some("gpt-5-codex"), Some("High"));
    assert!(
        args.windows(2)
            .any(|w| w[0] == "-c" && w[1] == "model_reasoning_effort=high"),
        "expected lowercased effort override, got: {args:?}",
    );
}

#[test]
fn codex_lowercases_mixed_case_effort() {
    let args = model_effort_args(Some(Runtime::Codex), None, Some("XHIGH"));
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
        mission_bus_sandbox_args(Some(Runtime::Codex), Some(&dir)),
        vec!["--add-dir".to_string(), dir.to_string_lossy().to_string()],
    );
    assert!(mission_bus_sandbox_args(Some(Runtime::Codex), None).is_empty());
    assert!(mission_bus_sandbox_args(Some(Runtime::ClaudeCode), Some(&dir)).is_empty());
    assert!(mission_bus_sandbox_args(Some(Runtime::Shell), Some(&dir)).is_empty());
}

#[test]
fn codex_trailing_args_omit_positional_prompt() {
    // `system_prompt` is not a positional first turn. Model/effort
    // flags still ride along so the role row's pinned settings
    // reach the spawned CLI.
    for plan_resuming in [false, true] {
        let args = trailing_runtime_args(
            Some(Runtime::Codex),
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
    let out = apply_permission_mode(Some(Runtime::Codex), &user, PermissionMode::Auto);
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
    let out = apply_permission_mode(Some(Runtime::Codex), &user, PermissionMode::Bypass);
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
    let out = apply_permission_mode(Some(Runtime::Codex), &user, PermissionMode::AcceptEdits);
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
    let out = apply_permission_mode(Some(Runtime::Codex), &user, PermissionMode::Bypass);
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
    let out = apply_permission_mode(Some(Runtime::Codex), &user, PermissionMode::Default);
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
        apply_mission_permission_mode(Some(Runtime::Codex), &row, MissionPermissionMode::Bypass),
        vec![
            "--ask-for-approval".to_string(),
            "never".to_string(),
            "--sandbox".to_string(),
            "danger-full-access".to_string(),
        ],
    );
    assert_eq!(
        apply_mission_permission_mode(
            Some(Runtime::Codex),
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
        infer_permission_mode(Some(Runtime::Codex), &args),
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
        infer_permission_mode(Some(Runtime::Codex), &args),
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
        infer_permission_mode(Some(Runtime::Codex), &args),
        PermissionMode::Auto
    );
}

#[test]
fn infer_permission_mode_codex_partial_match_falls_back_to_default() {
    // --sandbox is present, --ask-for-approval is not. Neither
    // pair fully matches → default.
    let args = vec!["--sandbox=workspace-write".to_string()];
    assert_eq!(
        infer_permission_mode(Some(Runtime::Codex), &args),
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
        infer_permission_mode(Some(Runtime::Codex), &args),
        PermissionMode::Default,
    );
}
