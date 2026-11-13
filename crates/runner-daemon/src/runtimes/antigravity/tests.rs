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
    assert_eq!(resumed, ["--log-file", log.as_str()]);
    assert!(system_prompt_args(Runtime::Antigravity.key(), Some("persona")).is_empty());
    assert_eq!(
        first_turn_argv(Runtime::Antigravity.key(), Some("body")),
        ["-i", "body"]
    );
}

#[test]
fn antigravity_status_args_require_the_installed_hooks_folder() {
    let root = tempfile::tempdir().unwrap();
    assert!(antigravity_status_args_for(Runtime::Antigravity.key(), root.path()).is_empty());
    crate::runtimes::antigravity::agy_status::install_hooks(root.path()).unwrap();
    let args = antigravity_status_args_for(Runtime::Antigravity.key(), root.path());
    if cfg!(windows) {
        assert!(args.is_empty(), "agy hook status is macOS-only");
    } else {
        assert_eq!(
            args,
            [
                "--add-dir".to_owned(),
                crate::runtimes::antigravity::agy_status::hooks_dir(root.path())
                    .to_string_lossy()
                    .into_owned(),
            ]
        );
    }
    assert!(antigravity_status_args_for(Runtime::Copilot.key(), root.path()).is_empty());
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
    assert_eq!(
        mission_bus_sandbox_args(
            runtime.map(Runtime::key).unwrap_or(""),
            Some(Path::new("/tmp/mission"))
        ),
        ["--add-dir", "/tmp/mission"]
    );
    assert!(mission_bus_sandbox_args(runtime.map(Runtime::key).unwrap_or(""), None).is_empty());
}

fn antigravity_status_args_for(key: &str, data: &Path) -> Vec<String> {
    if key == Runtime::Antigravity.key() {
        antigravity_status_args(data)
    } else {
        Vec::new()
    }
}
