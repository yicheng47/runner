use super::*;

#[cfg(windows)]
#[test]
fn windows_resume_defers_history_validation_to_cursor() {
    assert_eq!(
        CursorAgent.conversation_exists(
            "578dbdf3-2776-4aa0-ab95-ec4a2c3f58e4",
            &ProbeContext {
                cwd: Some(r"C:\workspace"),
                role_env: &Default::default(),
            },
        ),
        None
    );
}

#[test]
fn fresh_prompt_and_explicit_model_preserve_the_body_without_effort() {
    let body = "角色: coder\nquote ' \" — 修复 tests";
    for (mission, resuming) in [(false, false), (false, true), (true, false), (true, true)] {
        let args = CursorAgent.launch_args(&LaunchContext {
            role_args: &[],
            app_data_dir: Path::new("unused"),
            session_id: "slot",
            resuming,
            mission,
            model: Some(" account-model "),
            effort: Some("high"),
            codex_speed: None,
            system_prompt: None,
            first_turn: Some(body),
        });
        let mut expected = if resuming {
            strings(&["--model", "account-model"])
        } else {
            strings(&["--model", "account-model", body])
        };
        if !mission {
            expected.insert(0, "--trust".into());
        }
        assert_eq!(args, expected);
    }
    assert!(CursorAgent.first_turn_argv(Some(" \n")).is_empty());
    for model in [None, Some(" ")] {
        assert!(CursorAgent
            .model_effort_args(model, Some("high"))
            .is_empty());
    }
    assert!(CursorAgent.prompt_channels().resend_persona_on_fresh);
}

#[test]
fn new_conversations_have_independent_ids_and_resume_exact_uuid() {
    let first = CursorAgent.resume_plan(None);
    let second = CursorAgent.resume_plan(Some("invalid"));
    assert!(!first.resuming);
    assert_eq!(first.args[0], "--new-session-id");
    assert!(is_uuid(first.assigned_key.as_deref().unwrap()));
    assert_ne!(first.assigned_key, second.assigned_key);
    let resumed = CursorAgent.resume_plan(first.assigned_key.as_deref());
    assert!(resumed.resuming);
    assert_eq!(
        resumed.args,
        strings(&["--resume", first.assigned_key.as_deref().unwrap()])
    );
    assert!(matches!(CursorAgent.key_capture(), KeyCapture::Hook));
}

#[test]
fn permissions_strip_cursor_modes_and_keep_other_args() {
    let args = strings(&[
        "--force",
        "-f",
        "--yolo",
        "--sandbox",
        "enabled",
        "--sandbox=enabled",
        "--approve-mcps",
        "--trust",
        "--mode",
        "ask",
        "--mode=plan",
        "--plan",
        "--model",
        "chosen",
        "--header",
        "X: value",
    ]);
    assert_eq!(
        PERMISSIONS.strip(&args),
        strings(&["--model", "chosen", "--header", "X: value"])
    );
    assert_eq!(
        PERMISSIONS.apply_mission(&args, crate::router::runtime::MissionPermissionMode::Bypass),
        strings(&[
            "--model",
            "chosen",
            "--header",
            "X: value",
            "--force",
            "--sandbox",
            "disabled",
            "--approve-mcps",
            "--trust"
        ])
    );
}

#[test]
fn experimental_catalog_and_capability_boundaries_are_honest() {
    let entry = CursorAgent.catalog().unwrap();
    assert_eq!(entry.name.key(), "cursor");
    assert_eq!(entry.command, "cursor-agent");
    assert!(!entry.default_enabled);
    assert_eq!(entry.description, "Cursor Agent CLI");
    assert_eq!(entry.models.len(), 1);
    assert!(entry.models[0].value.is_empty());
    assert_eq!(entry.efforts.len(), 1);
    assert!(entry.efforts[0].value.is_empty());
    assert!(!entry.native_fork);
    assert_eq!(entry.capabilities, RuntimeCapabilities::default());
    assert!(entry.update_args.is_empty());
    assert!(CursorAgent.permissions().offered.is_empty());
    assert!(CursorAgent.fork_plan("chat", "label").is_none());
    assert!(CursorAgent.status_hooks().unwrap().supported(false));
    assert!(!CursorAgent.status_hooks().unwrap().supported(true));
    assert_eq!(CursorAgent.model_discovery().unwrap().method, "models");
    assert!(CursorAgent.usage().is_none());
    assert!(CursorAgent.mcp().is_none());
    assert!(CursorAgent.skills().toggle.is_none());
    assert_eq!(
        CursorAgent.skills().roots,
        [".cursor/skills", ".agents/skills"]
    );
}
