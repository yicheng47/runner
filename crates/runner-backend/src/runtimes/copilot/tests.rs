use super::*;
use crate::runtimes::test_support::*;

#[test]
fn copilot_permissions_roundtrip_and_strip_every_elevation_flag() {
    for (mode, expected) in [
        (PermissionMode::Default, vec![]),
        (PermissionMode::AcceptEdits, vec!["--allow-tool=write"]),
        (PermissionMode::Bypass, vec!["--yolo"]),
        (PermissionMode::Auto, vec![]),
    ] {
        let args = permission_mode_args(Some(Runtime::Copilot), mode);
        assert_eq!(args, expected);
        assert_eq!(
            infer_permission_mode(Some(Runtime::Copilot), &args),
            if mode == PermissionMode::Auto {
                PermissionMode::Default
            } else {
                mode
            }
        );
        assert_eq!(
            apply_permission_mode(
                Some(Runtime::Copilot),
                &["--yolo".into(), "--debug".into()],
                mode
            ),
            std::iter::once("--debug".to_string())
                .chain(args)
                .collect::<Vec<_>>()
        );
    }
    for allow_tool in [vec!["--allow-tool=write"], vec!["--allow-tool", "write"]] {
        let mut args = vec![
            "--model",
            "gpt-5.4",
            "--yolo",
            "--allow-all",
            "--allow-all-tools",
            "--allow-all-paths",
            "--allow-all-urls",
        ];
        args.extend(allow_tool.iter().copied());
        let args = args.into_iter().map(String::from).collect::<Vec<_>>();
        assert_eq!(
            strip_permission_flags(Some(Runtime::Copilot), &args),
            ["--model", "gpt-5.4"]
        );
        assert_eq!(
            infer_permission_mode(
                Some(Runtime::Copilot),
                &allow_tool.into_iter().map(String::from).collect::<Vec<_>>()
            ),
            PermissionMode::AcceptEdits
        );
    }
    assert_eq!(
        strip_permission_flags(
            Some(Runtime::Copilot),
            &[
                "--allow-tool".into(),
                "read".into(),
                "write".into(),
                "--model".into(),
                "gpt-5.4".into(),
            ],
        ),
        ["--model", "gpt-5.4"]
    );
    assert_eq!(
        strip_permission_flags(
            Some(Runtime::Copilot),
            &["--allow-tool".into(), "--model".into(), "gpt-5.4".into(),],
        ),
        ["--model", "gpt-5.4"]
    );
    assert_eq!(
        mission_permission_mode_args(Some(Runtime::Copilot), MissionPermissionMode::Bypass),
        Some(vec!["--yolo".into()])
    );
    assert_eq!(
        mission_permission_mode_args(Some(Runtime::Copilot), MissionPermissionMode::Auto),
        Some(vec![])
    );
    assert_eq!(
        mission_bus_sandbox_args(Some(Runtime::Copilot), Some(Path::new("/mission"))),
        ["--add-dir", "/mission"]
    );
    assert!(mission_bus_sandbox_args(Some(Runtime::Copilot), None).is_empty());
}

#[test]
fn copilot_status_plugin_args_require_a_complete_installed_plugin() {
    let root = tempfile::tempdir().unwrap();
    assert!(copilot_status_args(Some(Runtime::Copilot), root.path()).is_empty());
    crate::runtimes::copilot::copilot_status::install_plugin(root.path()).unwrap();
    let args = copilot_status_args(Some(Runtime::Copilot), root.path());
    assert_eq!(
        args,
        [
            "--plugin-dir".to_owned(),
            crate::runtimes::copilot::copilot_status::plugin_dir(root.path())
                .to_string_lossy()
                .into_owned(),
        ]
    );
}

#[test]
fn copilot_assigns_and_resumes_the_same_id_without_a_capture_thread() {
    let fresh = resume_plan(Some(Runtime::Copilot), None);
    let key = fresh.assigned_key.as_deref().unwrap();
    assert!(uuid::Uuid::parse_str(key).is_ok());
    assert_eq!(fresh.args, ["--session-id", key]);
    assert!(!fresh.resuming);
    assert!(!fresh.prepend);
    let resumed = resume_plan(Some(Runtime::Copilot), Some(key));
    assert_eq!(resumed.args, fresh.args);
    assert_eq!(resumed.assigned_key, fresh.assigned_key);
    assert!(resumed.resuming);
    assert!(!resumed.prepend);
    assert!(!resume_plan(Some(Runtime::Copilot), Some("not-a-uuid")).resuming);
    assert!(system_prompt_args(Some(Runtime::Copilot), Some("persona")).is_empty());
    assert_eq!(
        model_effort_args(Some(Runtime::Copilot), Some("gpt-5.4"), Some("high")),
        ["--model", "gpt-5.4", "--effort", "high"]
    );
    for effort in [
        "none", "minimal", "low", "medium", "high", "xhigh", "max", "High",
    ] {
        assert_eq!(
            model_effort_args(Some(Runtime::Copilot), None, Some(effort)),
            ["--effort", effort]
        );
    }
    assert_eq!(
        first_turn_argv(Some(Runtime::Copilot), Some("body")),
        ["-i", "body"]
    );
}

#[test]
fn copilot_conversation_probe_uses_events_file_in_its_home() {
    let home = tempfile::tempdir().unwrap();
    let key = uuid::Uuid::new_v4().to_string();
    assert!(!copilot_conversation_exists_at(home.path(), &key));
    let session = home.path().join("session-state").join(&key);
    std::fs::create_dir_all(&session).unwrap();
    assert!(!copilot_conversation_exists_at(home.path(), &key));
    std::fs::write(session.join("events.jsonl"), "").unwrap();
    assert!(copilot_conversation_exists_with_home(
        &key,
        home.path().to_str()
    ));
}
