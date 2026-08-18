use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use runner_app::bootstrap::{boot_core, NativePaths};
use runner_app::terminal_ime::TerminalInput;
use runner_backend::ops::runner::CreateRunnerInput;
use runner_backend::router::runtime::PermissionMode;
use runner_terminal::replay::visible_lines;
use runner_terminal::terminal::{TerminalBridge, TerminalSession};

fn wait_for_text(terminal: &TerminalSession, expected: &str) -> bool {
    let deadline = Instant::now() + Duration::from_secs(3);
    while Instant::now() < deadline {
        let rendered = {
            let term = terminal.term.lock();
            visible_lines(&*term).join("\n").contains(expected)
        };
        if rendered {
            return true;
        }
        thread::sleep(Duration::from_millis(20));
    }
    false
}

#[test]
fn direct_chat_flows_from_app_core_session_manager_into_terminal_grid() {
    let temp = tempfile::tempdir().unwrap();
    let paths = NativePaths::new(temp.path().join("app-data"), temp.path().join("logs"));
    let core = boot_core(&paths).unwrap();
    let runner = runner_backend::ops::runner::runner_create(
        &core,
        CreateRunnerInput {
            handle: "phase3-seam".into(),
            display_name: "Phase 3 seam".into(),
            runtime: "shell".into(),
            command: "/bin/cat".into(),
            args: Vec::new(),
            working_dir: Some(temp.path().to_string_lossy().into_owned()),
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            permission_mode: PermissionMode::Auto,
        },
    )
    .unwrap();
    let spawned = runner_backend::ops::session::session_start_direct(
        &core,
        runner.id,
        None,
        None,
        None,
        None,
        None,
        Some(80),
        Some(24),
    )
    .unwrap();
    let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    let bridge = TerminalBridge::new(core.clone(), Arc::clone(&waker)).unwrap();
    let terminal =
        TerminalSession::attach(core.clone(), spawned.id.clone(), 80, 24, waker).unwrap();
    bridge.attach(Arc::clone(&terminal)).unwrap();
    assert_eq!(terminal.size(), (80, 24));
    terminal.resize(96, 32);
    assert_eq!(terminal.size(), (96, 32));

    terminal.submit_text("manager-owned-pty").unwrap();
    let rendered = wait_for_text(&terminal, "manager-owned-pty");

    runner_backend::ops::session::session_kill(&core, &spawned.id).unwrap();
    assert!(
        rendered,
        "manager output never reached the alacritty terminal grid"
    );
}

#[test]
fn terminal_ime_commit_forwards_utf8_through_session_manager() {
    let temp = tempfile::tempdir().unwrap();
    let paths = NativePaths::new(temp.path().join("app-data"), temp.path().join("logs"));
    let core = boot_core(&paths).unwrap();
    let runner = runner_backend::ops::runner::runner_create(
        &core,
        CreateRunnerInput {
            handle: "terminal-ime".into(),
            display_name: "Terminal IME".into(),
            runtime: "shell".into(),
            command: "/bin/cat".into(),
            args: Vec::new(),
            working_dir: Some(temp.path().to_string_lossy().into_owned()),
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            permission_mode: PermissionMode::Auto,
        },
    )
    .unwrap();
    let spawned = runner_backend::ops::session::session_start_direct(
        &core,
        runner.id,
        None,
        None,
        None,
        None,
        None,
        Some(80),
        Some(24),
    )
    .unwrap();
    let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    let bridge = TerminalBridge::new(core.clone(), Arc::clone(&waker)).unwrap();
    let terminal =
        TerminalSession::attach(core.clone(), spawned.id.clone(), 80, 24, waker).unwrap();
    bridge.attach(Arc::clone(&terminal)).unwrap();

    let mut input = TerminalInput::new(Arc::clone(&terminal));
    input.replace_and_mark_text(None, "pinyin", Some(6..6));
    input.commit_text("拼音").unwrap();
    assert_eq!(input.marked_text(), None);
    let rendered = wait_for_text(&terminal, "拼音");
    let visible = {
        let term = terminal.term.lock();
        visible_lines(&*term).join("\n")
    };

    runner_backend::ops::session::session_kill(&core, &spawned.id).unwrap();
    assert!(rendered, "committed UTF-8 did not reach the terminal grid");
    assert!(!visible.contains("pinyin"), "marked text reached the PTY");
}

#[test]
fn bridge_keeps_multiple_tab_sessions_attached_with_independent_geometry() {
    let temp = tempfile::tempdir().unwrap();
    let paths = NativePaths::new(temp.path().join("app-data"), temp.path().join("logs"));
    let core = boot_core(&paths).unwrap();
    let runner = runner_backend::ops::runner::runner_create(
        &core,
        CreateRunnerInput {
            handle: "phase4-tabs".into(),
            display_name: "Phase 4 tabs".into(),
            runtime: "shell".into(),
            command: "/bin/cat".into(),
            args: Vec::new(),
            working_dir: Some(temp.path().to_string_lossy().into_owned()),
            system_prompt: None,
            env: HashMap::new(),
            model: None,
            effort: None,
            permission_mode: PermissionMode::Auto,
        },
    )
    .unwrap();
    let first = runner_backend::ops::session::session_start_direct(
        &core,
        runner.id.clone(),
        None,
        None,
        None,
        None,
        None,
        Some(80),
        Some(24),
    )
    .unwrap();
    let second = runner_backend::ops::session::session_start_direct(
        &core,
        runner.id,
        None,
        None,
        None,
        None,
        None,
        Some(120),
        Some(40),
    )
    .unwrap();
    let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    let bridge = TerminalBridge::new(core.clone(), Arc::clone(&waker)).unwrap();
    let first_terminal =
        TerminalSession::attach(core.clone(), first.id.clone(), 80, 24, Arc::clone(&waker))
            .unwrap();
    let second_terminal =
        TerminalSession::attach(core.clone(), second.id.clone(), 120, 40, waker).unwrap();
    bridge.attach(Arc::clone(&first_terminal)).unwrap();
    bridge.attach(Arc::clone(&second_terminal)).unwrap();

    first_terminal.submit_text("first-tab-stays-live").unwrap();
    second_terminal
        .submit_text("second-tab-stays-live")
        .unwrap();
    assert!(wait_for_text(&first_terminal, "first-tab-stays-live"));
    assert!(wait_for_text(&second_terminal, "second-tab-stays-live"));
    assert_eq!(first_terminal.size(), (80, 24));
    assert_eq!(second_terminal.size(), (120, 40));

    runner_backend::ops::session::session_kill(&core, &first.id).unwrap();
    runner_backend::ops::session::session_kill(&core, &second.id).unwrap();
}
