use super::*;
use crate::session::hook_queue::TestHookRoute;
use crate::session::state::agent::AgentModel;
use crate::session::status::Activity;

#[test]
fn installer_preserves_foreign_hooks_and_never_creates_the_home() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join(".grok");
    install(&home).unwrap();
    assert!(!home.exists());
    std::fs::create_dir_all(home.join("hooks")).unwrap();
    let foreign = home.join("hooks/orca-status.json");
    std::fs::write(&foreign, "foreign").unwrap();
    install(&home).unwrap();
    let path = home.join("hooks/runner-status.json");
    let bytes = std::fs::read(&path).unwrap();
    install(&home).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    assert_eq!(std::fs::read_to_string(foreign).unwrap(), "foreign");
    let document: Value = serde_json::from_slice(&bytes).unwrap();
    assert!(document["hooks"].get("PreToolUse").is_none());
    for event in EVENTS {
        assert_eq!(document["hooks"][event][0]["hooks"][0]["timeout"], 2);
    }
}
#[cfg(unix)]
#[test]
fn reporter_command_is_silent_with_unset_env_and_a_path_with_spaces() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let executable = root.path().join("Application Support/reporter");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::write(&executable, "#!/bin/sh\nprintf noise\nexit 1\n").unwrap();
    std::fs::set_permissions(&executable, std::fs::Permissions::from_mode(0o755)).unwrap();
    for reporter in [None, Some(executable.as_os_str())] {
        let mut command = std::process::Command::new("/bin/sh");
        command.args([
            "-c",
            hooks()["hooks"]["Stop"][0]["hooks"][0]["command"]
                .as_str()
                .unwrap(),
        ]);
        command.env_remove("RUNNER_HOOK_EXECUTABLE");
        if let Some(reporter) = reporter {
            command.env("RUNNER_HOOK_EXECUTABLE", reporter);
        }
        let output = command.output().unwrap();
        assert!(output.status.success());
        assert!(output.stdout.is_empty());
        assert!(output.stderr.is_empty());
    }
}
#[test]
fn hook_mapping_ignores_child_keys_stale_generations_and_session_end_stops() {
    let (route, receiver) = TestHookRoute::new(Runtime::Grok, "generation".into());
    let key = uuid::Uuid::new_v4().to_string();
    let next = uuid::Uuid::new_v4().to_string();
    let mut watcher = Watcher::new(
        receiver,
        &SpawnSpec {
            args: strings(&["-s", &key]),
            ..Default::default()
        },
    );
    let mut model = AgentModel::default();
    let mut keys = Vec::new();
    let mut drain = |watcher: &mut Watcher| {
        watcher
            .drain_events(
                0,
                &mut |event| {
                    model.reduce(event, 100);
                    Default::default()
                },
                &mut |key| keys.push(key),
            )
            .unwrap();
        model.value.clone()
    };
    let report = |event: &str, id: &str| json!({"hook_event_name":event,"sessionId":id});
    drop(route.admit(report("UserPromptSubmit", &key)).unwrap());
    assert_eq!(drain(&mut watcher).activity, Activity::Working);
    drop(
        route
            .admit(json!({"hook_event_name":"SessionStart","sessionId":key}))
            .unwrap(),
    );
    assert_eq!(drain(&mut watcher).activity, Activity::Working);
    drop(route.admit(json!({"hook_event_name":"Notification","sessionId":key,"notificationType":"permission_prompt","promptId":"turn"})).unwrap());
    assert!(drain(&mut watcher).needs_you());
    drop(route.admit(report("PostToolUse", &key)).unwrap());
    assert!(!drain(&mut watcher).needs_you());
    drop(
        route
            .admit(json!({"hook_event_name":"Stop","sessionId":key,"reason":"shutdown"}))
            .unwrap(),
    );
    assert_eq!(drain(&mut watcher).activity, Activity::Working);
    drop(
        route
            .admit(json!({"hook_event_name":"Stop","sessionId":key,"reason":"end_turn"}))
            .unwrap(),
    );
    assert_eq!(drain(&mut watcher).outcome, Some(TurnOutcome::Completed));
    drop(route.admit(report("UserPromptSubmit", &key)).unwrap());
    drop(route.admit(report("StopCancelled", &key)).unwrap());
    assert_eq!(drain(&mut watcher).outcome, Some(TurnOutcome::Interrupted));
    drop(route.admit(report("UserPromptSubmit", &key)).unwrap());
    drop(route.admit(report("StopFailure", &key)).unwrap());
    assert_eq!(drain(&mut watcher).outcome, Some(TurnOutcome::Failed));
    for field in ["subagentType", "agentId"] {
        for event in ["SessionStart", "UserPromptSubmit"] {
            let mut payload = report(event, &next);
            payload[field] = json!("child");
            drop(route.admit(payload).unwrap());
            drain(&mut watcher);
        }
    }
    let mut stale = report("SessionStart", &next);
    stale["generation"] = json!("old");
    assert!(route.admit(stale).is_err());
    drop(route.admit(report("SessionStart", &next)).unwrap());
    drain(&mut watcher);
    let dashboard = uuid::Uuid::new_v4().to_string();
    let mut stale = report("UserPromptSubmit", &dashboard);
    stale["generation"] = json!("old");
    assert!(route.admit(stale).is_err());
    drop(route.admit(report("UserPromptSubmit", &dashboard)).unwrap());
    assert_eq!(drain(&mut watcher).activity, Activity::Working);
    drop(
        route
            .admit(json!({"hook_event_name":"Stop","sessionId":next,"reason":"end_turn"}))
            .unwrap(),
    );
    assert_eq!(drain(&mut watcher).activity, Activity::Working);
    drop(
        route
            .admit(json!({"hook_event_name":"Stop","sessionId":dashboard,"reason":"end_turn"}))
            .unwrap(),
    );
    assert_eq!(drain(&mut watcher).outcome, Some(TurnOutcome::Completed));
    assert_eq!(keys, vec![next, dashboard]);
}

#[test]
fn delayed_turn_reports_preserve_the_current_turn_and_approval() {
    let (route, receiver) = TestHookRoute::new(Runtime::Grok, "generation".into());
    let key = uuid::Uuid::new_v4().to_string();
    let next = uuid::Uuid::new_v4().to_string();
    let mut watcher = Watcher::new(
        receiver,
        &SpawnSpec {
            args: strings(&["-s", &key]),
            ..Default::default()
        },
    );
    let mut model = AgentModel::default();
    let mut keys = Vec::new();
    let mut apply = |payload: Value| {
        drop(route.admit(payload).unwrap());
        watcher
            .drain_events(
                0,
                &mut |event| {
                    model.reduce(event, 100);
                    Default::default()
                },
                &mut |key| keys.push(key),
            )
            .unwrap();
        model.value.clone()
    };
    let report = |event: &str, id: &str, prompt: &str| json!({"hook_event_name":event,"sessionId":id,"promptId":prompt,"reason":"end_turn","notificationType":"permission_prompt"});
    apply(report("UserPromptSubmit", &key, "A"));
    apply(report("UserPromptSubmit", &key, "B"));
    assert!(apply(report("Notification", &key, "B")).needs_you());
    for event in [
        "Stop",
        "StopFailure",
        "StopCancelled",
        "PostToolUse",
        "PostToolUseFailure",
        "Notification",
    ] {
        let current = apply(report(event, &key, "A"));
        assert_eq!(current.activity, Activity::Working, "{event}");
        assert_eq!(current.outcome, None, "{event}");
        assert!(current.needs_you(), "{event}");
    }
    let ended = apply(report("Stop", &key, "B"));
    assert_eq!(ended.outcome, Some(TurnOutcome::Completed));
    assert!(!ended.needs_you());

    apply(report("UserPromptSubmit", &key, "C"));
    assert!(apply(report("Notification", &key, "C")).needs_you());
    let idle = apply(
        json!({"hook_event_name":"Notification","sessionId":key,"notificationType":"idle_prompt"}),
    );
    assert_eq!(idle.activity, Activity::Ready);
    assert!(!idle.needs_you());
    apply(report("UserPromptSubmit", &key, "D"));
    assert_eq!(
        apply(report("StopCancelled", &key, "unseen-bash-turn")).outcome,
        Some(TurnOutcome::Interrupted)
    );

    apply(json!({"hook_event_name":"SessionStart","sessionId":next}));
    apply(report("UserPromptSubmit", &next, "E"));
    assert_eq!(
        apply(report("StopCancelled", &key, "E")).activity,
        Activity::Working
    );
    assert_eq!(
        apply(report("StopCancelled", &next, "A")).outcome,
        Some(TurnOutcome::Interrupted)
    );
    assert_eq!(keys, vec![next]);
}

#[cfg(unix)]
#[test]
fn hooks_off_only_runs_identity_reporters() {
    use std::os::unix::fs::PermissionsExt;
    let root = tempfile::tempdir().unwrap();
    let reporter = root.path().join("reporter");
    let log = root.path().join("events");
    std::fs::write(
        &reporter,
        "#!/bin/sh\nprintf '%s\\n' \"$*\" >> \"$HOOK_TEST_LOG\"\n",
    )
    .unwrap();
    std::fs::set_permissions(&reporter, std::fs::Permissions::from_mode(0o755)).unwrap();
    let config = hooks();
    for event in ["SessionStart", "UserPromptSubmit", "Stop", "Notification"] {
        let command = config["hooks"][event][0]["hooks"][0]["command"]
            .as_str()
            .unwrap();
        assert!(std::process::Command::new("/bin/sh")
            .args(["-c", command])
            .env("RUNNER_HOOK_EXECUTABLE", &reporter)
            .env("RUNNER_STATUS_HOOKS", "false")
            .env("HOOK_TEST_LOG", &log)
            .status()
            .unwrap()
            .success());
    }
    let reports = std::fs::read_to_string(log).unwrap();
    assert!(reports.contains("--event SessionStart"));
    assert!(reports.contains("--event UserPromptSubmit"));
    assert!(!reports.contains("--event Stop"));
    assert!(!reports.contains("--event Notification"));
}
