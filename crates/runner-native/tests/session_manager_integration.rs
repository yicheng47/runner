use std::collections::HashMap;
use std::sync::Arc;
use std::thread;
use std::time::{Duration, Instant};

use runner_app::ops::runner::CreateRunnerInput;
use runner_app::router::runtime::PermissionMode;
use runner_native::bootstrap::{boot_core, NativePaths};
use runner_native::replay::visible_lines;
use runner_native::terminal::{TerminalBridge, TerminalSession};

#[test]
fn direct_chat_flows_from_app_core_session_manager_into_terminal_grid() {
    let temp = tempfile::tempdir().unwrap();
    let paths = NativePaths::new(temp.path().join("app-data"), temp.path().join("logs"));
    let core = boot_core(&paths).unwrap();
    let runner = runner_app::ops::runner::runner_create(
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
    let spawned = runner_app::ops::session::session_start_direct(
        &core,
        runner.id,
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

    terminal.submit_text("manager-owned-pty").unwrap();
    let deadline = Instant::now() + Duration::from_secs(3);
    let mut rendered = false;
    while Instant::now() < deadline {
        rendered = {
            let term = terminal.term.lock();
            visible_lines(&*term)
                .join("\n")
                .contains("manager-owned-pty")
        };
        if rendered {
            break;
        }
        thread::sleep(Duration::from_millis(20));
    }

    runner_app::ops::session::session_kill(&core, &spawned.id).unwrap();
    assert!(
        rendered,
        "manager output never reached the alacritty terminal grid"
    );
}
