use super::*;
use serde_json::json;

pub(super) fn model_cache(home: &Path) {
    std::fs::write(home.join("models_cache.json"), json!({"models":{"account-model":{"info":{"reasoning_efforts":[{"id":"low"},{"id":"high"}]}},"other-model":{"info":{"reasoning_efforts":[{"id":"xhigh"}]}}}}).to_string()).unwrap();
}
#[test]
fn fresh_resume_and_model_effort_keep_exact_prompt_and_native_defaults() {
    let home = tempfile::tempdir().unwrap();
    model_cache(home.path());
    let env =
        std::collections::BTreeMap::from([("GROK_HOME".into(), home.path().display().to_string())]);
    let body = "/literal @file\n角色 'quote' \"double\" $var `cmd`";
    let mut ctx = LaunchContext {
        env: &env,
        cwd: None,
        default_model: Some("account-model"),
        role_args: &[],
        app_data_dir: home.path(),
        session_id: "fixture",
        resuming: false,
        mission: false,
        model: None,
        effort: Some("high"),
        codex_speed: None,
        codex_tui_workspace: false,
        system_prompt: None,
        first_turn: Some(body),
    };
    for mission in [false, true] {
        ctx.mission = mission;
        assert_eq!(
            Grok.launch_args(&ctx),
            strings(&["--trust", "--reasoning-effort", "high", "--", body])
        );
        ctx.resuming = true;
        assert_eq!(
            Grok.launch_args(&ctx),
            strings(&["--trust", "--reasoning-effort", "high"])
        );
        ctx.resuming = false;
    }
    ctx.model = Some(" other-model ");
    assert!(!Grok
        .launch_args(&ctx)
        .iter()
        .any(|arg| arg == "--reasoning-effort"));
    ctx.effort = Some("xhigh");
    assert_eq!(
        &Grok.launch_args(&ctx)[..5],
        strings(&[
            "--trust",
            "-m",
            "other-model",
            "--reasoning-effort",
            "xhigh"
        ])
    );
    ctx.model = None;
    std::fs::write(
        home.path().join("config.toml"),
        "[models]\ndefault = 'other-model'\ndefault_reasoning_effort = 'xhigh'\n",
    )
    .unwrap();
    assert!(Grok
        .launch_args(&ctx)
        .contains(&"--reasoning-effort".into()));
    assert_eq!(
        Grok.native_defaults_with_env(home.path(), &env)
            .model
            .as_deref(),
        Some("other-model")
    );
    assert!(Grok.first_turn_argv(Some(" \n")).is_empty());
    let plan = Grok.resume_plan(None);
    assert_eq!(plan.args[0], "-s");
    assert!(is_uuid(plan.assigned_key.as_deref().unwrap()));
    let resumed = Grok.resume_plan(plan.assigned_key.as_deref());
    assert_eq!(
        resumed.args,
        strings(&["--resume", plan.assigned_key.as_deref().unwrap()])
    );
    assert!(resumed.resuming);
    assert_ne!(
        Grok.resume_plan(Some("invalid")).assigned_key,
        plan.assigned_key
    );
}
#[test]
fn permission_aliases_and_values_are_stripped_without_losing_other_args() {
    let args = strings(&[
        "--permission-mode",
        "auto",
        "--permission-mode=plan",
        "--always-approve",
        "--yolo",
        "--trust",
        "--sandbox",
        "safe",
        "--sandbox=other",
        "--no-subagents",
        "-m",
        "chosen",
    ]);
    assert_eq!(
        PERMISSIONS.strip(&args),
        strings(&["--no-subagents", "-m", "chosen"])
    );
    for alias in [
        "--yolo",
        "--always-approve",
        "--permission-mode=bypassPermissions",
    ] {
        assert_eq!(
            PERMISSIONS.infer(&strings(&[alias])),
            PermissionMode::Bypass
        );
    }
    assert_eq!(
        PERMISSIONS.mode_args(PermissionMode::Default),
        strings(&["--permission-mode", "default"])
    );
    assert_eq!(
        PERMISSIONS.apply_mission(&args),
        strings(&[
            "--no-subagents",
            "-m",
            "chosen",
            "--permission-mode",
            "bypassPermissions"
        ])
    );
}
#[test]
fn home_resolution_and_session_probe_match_cwd_without_directory_encoding() {
    let home = tempfile::tempdir().unwrap();
    let cwd = home.path().join("project 空格");
    std::fs::create_dir(&cwd).unwrap();
    let key = uuid::Uuid::new_v4().to_string();
    let env = HashMap::from([("GROK_HOME".into(), home.path().display().to_string())]);
    let ctx = ProbeContext {
        cwd: cwd.to_str(),
        role_env: &env,
    };
    assert_eq!(Grok.conversation_exists(&key, &ctx), Some(false));
    let path = home
        .path()
        .join("sessions/opaque-slug-hash")
        .join(&key)
        .join("summary.json");
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, json!({"info":{"id":key,"cwd":cwd}}).to_string()).unwrap();
    assert_eq!(Grok.conversation_exists(&key, &ctx), Some(true));
    let ctx = ProbeContext {
        cwd: home.path().to_str(),
        role_env: &env,
    };
    assert_eq!(Grok.conversation_exists(&key, &ctx), Some(false));
    assert_eq!(Grok.conversation_exists("../../foreign", &ctx), Some(false));
    crate::golden::with_config_env(Default::default(), || {
        assert_eq!(
            config_dir(&Default::default(), None, Some(home.path())),
            Some(home.path().join(".grok"))
        );
        assert_eq!(
            config_dir(
                &std::collections::BTreeMap::from([("GROK_HOME".into(), "relative".into())]),
                Some(&cwd),
                None
            ),
            Some(cwd.join("relative"))
        );
        assert_eq!(
            config_dir(
                &std::collections::BTreeMap::from([("GROK_HOME".into(), "relative".into())]),
                None,
                Some(home.path())
            ),
            Some(home.path().join("relative"))
        );
    });
}
#[test]
fn capabilities_and_platform_hooks_are_honest() {
    let catalog = Grok.catalog().unwrap();
    assert!(catalog.default_enabled);
    assert_eq!(catalog.models.len(), 1);
    assert_eq!(catalog.efforts.len(), 1);
    assert_eq!(catalog.update_args, &["update"]);
    assert!(catalog.npm_package.is_none());
    assert!(Grok.skills().toggle.is_none());
    assert!(Grok.usage().is_none());
    assert!(Grok.status_hooks().unwrap().supported(false));
    assert!(!Grok.status_hooks().unwrap().supported(true));
    assert!(catalog.native_fork);
    assert!(Grok.fork_plan("source", "label").is_none());
    let source = uuid::Uuid::new_v4().to_string();
    let Some(ForkPlan::Direct(plan)) = Grok.fork_plan(&source, "label") else {
        panic!("native fork")
    };
    assert!(plan.resuming);
    assert_eq!(
        &plan.args[..3],
        strings(&["--resume", &source, "--fork-session"])
    );
    assert!(is_uuid(plan.assigned_key.as_deref().unwrap()));
    assert_ne!(plan.assigned_key.as_deref(), Some(source.as_str()));
}

#[test]
fn stored_model_and_effort_options_cannot_bypass_the_effective_model_gate() {
    let home = tempfile::tempdir().unwrap();
    model_cache(home.path());
    let env =
        std::collections::BTreeMap::from([("GROK_HOME".into(), home.path().display().to_string())]);
    let role_args = strings(&[
        "--model=other-model",
        "-mother-model",
        "--effort=xhigh",
        "--reasoning-effort",
        "high",
        "--no-subagents",
    ]);
    assert_eq!(
        Grok.launch_role_args(&role_args),
        strings(&["--no-subagents"])
    );
    let mut ctx = LaunchContext {
        env: &env,
        cwd: None,
        default_model: None,
        role_args: &role_args,
        app_data_dir: home.path(),
        session_id: "fixture",
        resuming: false,
        mission: false,
        model: None,
        effort: None,
        codex_speed: None,
        codex_tui_workspace: false,
        system_prompt: None,
        first_turn: None,
    };
    assert_eq!(
        Grok.launch_args(&ctx),
        strings(&["--trust", "-m", "other-model"])
    );
    ctx.model = Some("account-model");
    assert_eq!(
        Grok.launch_args(&ctx),
        strings(&[
            "--trust",
            "-m",
            "account-model",
            "--reasoning-effort",
            "high"
        ])
    );
    ctx.effort = Some("xhigh");
    assert_eq!(
        Grok.launch_args(&ctx),
        strings(&["--trust", "-m", "account-model"])
    );
}

#[test]
fn environment_model_precedes_native_and_cached_defaults_but_not_explicit_model() {
    let home = tempfile::tempdir().unwrap();
    model_cache(home.path());
    std::fs::write(
        home.path().join("config.toml"),
        "[models]\ndefault = 'account-model'\n",
    )
    .unwrap();
    crate::golden::with_config_env(
        std::collections::BTreeMap::from([("GROK_DEFAULT_MODEL", "account-model".into())]),
        || {
            let env = std::collections::BTreeMap::from([
                ("GROK_HOME".into(), home.path().display().to_string()),
                ("GROK_DEFAULT_MODEL".into(), "other-model".into()),
            ]);
            let mut ctx = LaunchContext {
                env: &env,
                cwd: None,
                default_model: Some("account-model"),
                role_args: &[],
                app_data_dir: home.path(),
                session_id: "fixture",
                resuming: false,
                mission: false,
                model: None,
                effort: Some("high"),
                codex_speed: None,
                codex_tui_workspace: false,
                system_prompt: None,
                first_turn: None,
            };
            assert_eq!(Grok.launch_args(&ctx), strings(&["--trust"]));
            assert_eq!(
                Grok.native_defaults_with_env(home.path(), &env)
                    .model
                    .as_deref(),
                Some("other-model")
            );
            ctx.effort = Some("xhigh");
            assert_eq!(
                Grok.launch_args(&ctx),
                strings(&["--trust", "--reasoning-effort", "xhigh"])
            );
            ctx.model = Some("account-model");
            assert_eq!(
                Grok.launch_args(&ctx),
                strings(&["--trust", "-m", "account-model"])
            );
            ctx.effort = Some("high");
            assert_eq!(
                Grok.launch_args(&ctx),
                strings(&[
                    "--trust",
                    "-m",
                    "account-model",
                    "--reasoning-effort",
                    "high"
                ])
            );
            let unknown_env = std::collections::BTreeMap::from([
                ("GROK_HOME".into(), home.path().display().to_string()),
                ("GROK_DEFAULT_MODEL".into(), "unknown".into()),
            ]);
            ctx.env = &unknown_env;
            ctx.model = None;
            assert_eq!(Grok.launch_args(&ctx), strings(&["--trust"]));
            assert_eq!(
                models::environment_model(&Default::default()).as_deref(),
                Some("account-model")
            );
        },
    );
}
