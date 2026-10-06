use super::*;
use crate::runtimes::test_support::*;

#[test]
fn pi_fork_assigns_a_new_session_key_and_prepends_native_args() {
    let source = "019fa1b9-a133-7841-b4dd-730d376ab1d1";
    let plan = fork_plan(Runtime::Pi.key(), source, "Source").unwrap();
    let ForkPlan::Direct(plan) = plan else {
        panic!("pi must spawn its fork directly")
    };
    let assigned = plan.assigned_key.as_deref().unwrap();
    assert_ne!(assigned, source);
    assert_eq!(plan.args, ["--fork", source, "--session-id", assigned]);
    assert!(plan.prepend);
    assert!(plan.resuming);
}

#[test]
fn pi_status_extension_args_require_the_installed_extension() {
    let root = tempfile::tempdir().unwrap();
    assert!(pi_status_args_for(Runtime::Pi.key(), root.path()).is_empty());
    crate::runtimes::pi::pi_status::install_extension(root.path()).unwrap();
    assert_eq!(
        pi_status_args_for(Runtime::Pi.key(), root.path()),
        [
            "-e".to_owned(),
            crate::runtimes::pi::pi_status::extension_path(root.path())
                .to_string_lossy()
                .into_owned(),
        ]
    );
    assert!(pi_status_args_for(Runtime::Copilot.key(), root.path()).is_empty());
}

#[test]
fn pi_assigns_and_resumes_the_same_id_with_prompt_file_args() {
    let fresh = resume_plan(Runtime::Pi.key(), None);
    let key = fresh.assigned_key.as_deref().unwrap();
    assert!(uuid::Uuid::parse_str(key).is_ok());
    assert_eq!(fresh.args, ["--session-id", key]);
    assert!(fresh.prepend);
    assert!(!fresh.resuming);

    let resumed = resume_plan(Runtime::Pi.key(), Some(key));
    assert_eq!(resumed.args, fresh.args);
    assert_eq!(resumed.assigned_key, fresh.assigned_key);
    assert!(resumed.prepend);
    assert!(resumed.resuming);
    assert_eq!(
        system_prompt_args(Runtime::Pi.key(), Some("/tmp/persona.md")),
        ["--append-system-prompt", "/tmp/persona.md"]
    );
    assert_eq!(
        model_effort_args(
            Runtime::Pi.key(),
            Some("deepseek/deepseek-v4-pro"),
            Some("High")
        ),
        ["--model", "deepseek/deepseek-v4-pro", "--thinking", "high"]
    );
    assert_eq!(
        first_turn_argv(Runtime::Pi.key(), Some("-goal")),
        ["--", "-goal"]
    );
}

#[test]
fn pi_status_extension_precedes_the_system_prompt_and_goal_on_spawn_and_resume() {
    let root = tempfile::tempdir().unwrap();
    crate::runtimes::pi::pi_status::install_extension(root.path()).unwrap();
    let extension = crate::runtimes::pi::pi_status::extension_path(root.path())
        .to_string_lossy()
        .into_owned();
    let fresh = trailing_runtime_args(
        Runtime::Pi.key(),
        &[],
        root.path(),
        "runner-session",
        false,
        None,
        None,
        None,
        Some("/tmp/prompt.md"),
        Some("== Mission ==\nGoal"),
    );
    assert_eq!(
        fresh,
        [
            "-e",
            extension.as_str(),
            "--append-system-prompt",
            "/tmp/prompt.md",
            "--",
            "== Mission ==\nGoal",
        ]
    );
    let resumed = trailing_runtime_args(
        Runtime::Pi.key(),
        &[],
        root.path(),
        "runner-session",
        true,
        None,
        None,
        None,
        Some("/tmp/prompt.md"),
        Some("not replayed"),
    );
    assert_eq!(
        resumed,
        [
            "-e",
            extension.as_str(),
            "--append-system-prompt",
            "/tmp/prompt.md",
        ]
    );
}

#[test]
fn pi_conversation_probe_uses_agent_dir_and_the_encoded_project_slug() {
    assert_eq!(pi_project_slug("/Users/jason"), "--Users-jason--");
    assert_eq!(pi_project_slug(r"C:\Users\x"), "--C--Users-x--");

    let home = tempfile::tempdir().unwrap();
    let cwd = home.path().join("project");
    let cwd = cwd.to_string_lossy();
    let key = uuid::Uuid::new_v4().to_string();
    let agent_dir = home.path().join("custom-agent");
    let sessions = agent_dir.join("sessions").join(pi_project_slug(&cwd));
    std::fs::create_dir_all(&sessions).unwrap();
    assert!(!pi_conversation_exists_at(
        Some(home.path()),
        cwd.as_ref(),
        &key,
        None,
        Some(agent_dir.as_os_str()),
    ));
    std::fs::write(
        sessions.join(format!("2026-09-18T00-00-00_{key}.jsonl")),
        "",
    )
    .unwrap();
    assert!(pi_conversation_exists_at(
        Some(home.path()),
        cwd.as_ref(),
        &key,
        None,
        Some(agent_dir.as_os_str()),
    ));
}

#[test]
fn pi_conversation_probe_session_dir_wins_and_holds_files_flat() {
    let home = tempfile::tempdir().unwrap();
    let cwd = home.path().join("project");
    let cwd = cwd.to_string_lossy();
    let key = uuid::Uuid::new_v4().to_string();
    let agent_dir = home.path().join("agent");
    let nested = agent_dir.join("sessions").join(pi_project_slug(&cwd));
    std::fs::create_dir_all(&nested).unwrap();
    std::fs::write(nested.join(format!("agent_{key}.jsonl")), "").unwrap();
    let session_dir = home.path().join("flat-sessions");
    std::fs::create_dir_all(&session_dir).unwrap();

    assert!(!pi_conversation_exists_at(
        Some(home.path()),
        cwd.as_ref(),
        &key,
        Some(session_dir.as_os_str()),
        Some(agent_dir.as_os_str()),
    ));
    std::fs::write(session_dir.join(format!("flat_{key}.jsonl")), "").unwrap();
    assert!(pi_conversation_exists_at(
        Some(home.path()),
        cwd.as_ref(),
        &key,
        Some(session_dir.as_os_str()),
        Some(agent_dir.as_os_str()),
    ));
}

#[test]
fn pi_conversation_probe_without_env_uses_the_default_directory() {
    let home = tempfile::tempdir().unwrap();
    let cwd = home.path().join("project");
    let cwd = cwd.to_string_lossy();
    let key = uuid::Uuid::new_v4().to_string();
    let sessions = home
        .path()
        .join(".pi/agent/sessions")
        .join(pi_project_slug(&cwd));
    std::fs::create_dir_all(&sessions).unwrap();
    std::fs::write(sessions.join(format!("default_{key}.jsonl")), "").unwrap();

    assert!(pi_conversation_exists_at(
        Some(home.path()),
        cwd.as_ref(),
        &key,
        None,
        None,
    ));
}

#[test]
fn pi_conversation_probe_does_not_fall_back_from_an_empty_configured_directory() {
    let home = tempfile::tempdir().unwrap();
    let cwd = home.path().join("project");
    let cwd = cwd.to_string_lossy();
    let key = uuid::Uuid::new_v4().to_string();
    let default_sessions = home
        .path()
        .join(".pi/agent/sessions")
        .join(pi_project_slug(&cwd));
    std::fs::create_dir_all(&default_sessions).unwrap();
    std::fs::write(default_sessions.join(format!("default_{key}.jsonl")), "").unwrap();
    let agent_dir = home.path().join("empty-agent");
    std::fs::create_dir_all(&agent_dir).unwrap();

    assert!(!pi_conversation_exists_at(
        Some(home.path()),
        cwd.as_ref(),
        &key,
        None,
        Some(agent_dir.as_os_str()),
    ));
}

#[test]
fn pi_session_directory_expands_tilde_and_resolves_relative_overrides_from_cwd() {
    let home = tempfile::tempdir().unwrap();
    let cwd = home.path().join("project");
    assert_eq!(
        pi_session_directory(
            Some(home.path()),
            cwd.to_str().unwrap(),
            Some(OsStr::new("~/flat-sessions")),
            None,
        ),
        Some(home.path().join("flat-sessions"))
    );
    assert_eq!(
        pi_session_directory(
            Some(home.path()),
            cwd.to_str().unwrap(),
            Some(OsStr::new("relative-sessions")),
            None,
        ),
        Some(cwd.join("relative-sessions"))
    );
}

fn pi_status_args_for(key: &str, data: &Path) -> Vec<String> {
    if key == Runtime::Pi.key() {
        pi_status_args(data)
    } else {
        Vec::new()
    }
}
