use alacritty_terminal::grid::Dimensions as _;
use alacritty_terminal::index::{Column, Line, Point, Side};
use alacritty_terminal::selection::SelectionType;
use alacritty_terminal::term::cell::Flags;
use alacritty_terminal::term::TermMode;
use alacritty_terminal::vte::ansi::CursorShape;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use crate::session::manager::{
    ExitEvent, OutputEvent, SessionEvents, SessionSpawnedEvent, SessionUpdatedEvent,
};

use super::support::{TerminalBridge, TerminalSession, UserInputMode};
use crate::session::runtime::{
    OutputStream, RuntimeOutput, RuntimeResult, RuntimeSession, SessionRuntime, SessionStatus,
    SpawnSpec,
};
use crate::AppCore;
use runner_terminal::replay::visible_lines;
use runner_terminal::terminal::LinkTarget;

fn flush_terminal_events(terminal: &TerminalSession) {
    terminal.model.flush_events();
    terminal.mirror.refresh_metadata();
}

#[test]
fn terminal_titles_persist_topics_and_ignore_directory_status_and_reset() {
    use crate::repo::session::{self, SessionRowDb};
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let conn = core.db.get().unwrap();
    let mut row = SessionRowDb::new_running("replay-race".into());
    row.agent_runtime = Some("codex".into());
    row.agent_command = Some("codex".into());
    row.title = Some("Manual name".into());
    row.cwd = Some("/Users/jason/repos/yicheng47".into());
    row.live_title = Some("Previous topic".into());
    session::insert(&conn, &row).unwrap();
    conn.execute_batch(
        "CREATE TABLE title_writes (title TEXT);
            CREATE TRIGGER record_title AFTER UPDATE OF live_title ON sessions
            BEGIN INSERT INTO title_writes VALUES (NEW.live_title); END;",
    )
    .unwrap();
    let terminal =
        TerminalSession::attach(core.clone(), row.id.clone(), 80, 24, Arc::new(|| {})).unwrap();
    assert_eq!(terminal.title(), "Previous topic");
    for (seq, data, expected, writes) in [
        (1, "\x1b[22;0t\x1b]0;⠋ Cars\x07", "Cars", 1),
        (2, "\x1b]0;⠙ Cars\x1b\\\x1b]2;Cars\x07", "Cars", 1),
        (3, "\x1b]2;Electric", "Cars", 1),
        (4, " cars\x1b\\", "Electric cars", 2),
        (5, "\x1b]0;\x07", "Electric cars", 2),
        (6, "\x1b]2;New topic | yicheng47\x07", "New topic", 3),
        (7, "\x1b[23;0t", "New topic", 3),
        (8, "\x1b]0;yicheng47\x07", "New topic", 3),
        (
            9,
            "\x1b]0;[ ! ] Action Required | yicheng47\x07",
            "New topic",
            3,
        ),
        (10, "\x1b]0;◐ Airplane type\x07", "Airplane type", 4),
        (11, "\x1b]0;◑ Airplane type\x07", "Airplane type", 4),
        (12, "\x1b]0;◒ Airplane type\x07", "Airplane type", 4),
        (13, "\x1b]0;◓ Airplane type\x07", "Airplane type", 4),
        (14, "\x1b]0;✳ Airplane type\x07", "Airplane type", 4),
    ] {
        terminal.feed_output(&output(seq, data)).unwrap();
        flush_terminal_events(&terminal);
        assert_eq!(terminal.title(), expected);
        let stored = session::get_row(&conn, &row.id).unwrap().unwrap();
        assert_eq!(
            stored.live_title.as_deref(),
            (!expected.is_empty()).then_some(expected)
        );
        assert_eq!(stored.title.as_deref(), Some("Manual name"));
        let count: usize = conn
            .query_row("SELECT COUNT(*) FROM title_writes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, writes);
    }
    terminal
        .feed_output(&output(15, "\x1b]0;Last topic\x07"))
        .unwrap();
    flush_terminal_events(&terminal);
    let detail = crate::ops::session::session_get(&core, &row.id)
        .unwrap()
        .unwrap();
    assert_eq!(detail.live_title.as_deref(), Some("Last topic"));
    let reopened = TerminalSession::attach(core, row.id, 80, 24, Arc::new(|| {})).unwrap();
    assert_eq!(reopened.title(), "Last topic");
}

#[test]
fn shell_titles_keep_braille_without_persistence() {
    use crate::repo::session::{self, SessionRowDb};
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let conn = core.db.get().unwrap();
    let mut row = SessionRowDb::new_running("replay-race".into());
    row.agent_runtime = Some("shell".into());
    session::insert(&conn, &row).unwrap();
    let terminal =
        TerminalSession::attach(core.clone(), row.id.clone(), 80, 24, Arc::new(|| {})).unwrap();
    let wakes = Arc::new(AtomicUsize::new(0));
    let notify = Arc::clone(&wakes);
    let bridge = runner_terminal::terminal::TerminalBridge::new(
        crate::daemon::InProcessTransport::client(core.clone()),
        Arc::new(move || {
            notify.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();
    let mirror = bridge.attach(&row.id).unwrap();
    let before = wakes.load(Ordering::Relaxed);
    let mut events = core.events.subscribe();
    terminal
        .feed_output(&output(1, "\x1b]2;  ⠋ shell ⠹  \x07"))
        .unwrap();
    flush_terminal_events(&terminal);
    assert_eq!(terminal.title(), "⠋ shell ⠹");
    assert_eq!(mirror.title(), "⠋ shell ⠹");
    assert_eq!(wakes.load(Ordering::Relaxed), before + 1);
    assert!(matches!(
        events.try_recv(),
        Err(tokio::sync::broadcast::error::TryRecvError::Empty)
    ));
    assert_eq!(
        session::get_row(&conn, &row.id)
            .unwrap()
            .unwrap()
            .live_title,
        None
    );
    let reopened = TerminalSession::attach(core, row.id, 80, 24, Arc::new(|| {})).unwrap();
    assert_eq!(reopened.title(), "");
}

/// Minimal `AppCore` over a temp dir — the pieces `boot_core` wires
/// in runner-app, minus login-shell discovery and startup cleanup.
fn test_core(root: &std::path::Path) -> AppCore {
    test_core_with_runtime(
        root,
        Arc::new(crate::session::pty_runtime::PtyRuntime::new()),
    )
}

fn test_core_with_runtime(root: &std::path::Path, runtime: Arc<dyn SessionRuntime>) -> AppCore {
    let app_data_dir = root.join("app-data");
    std::fs::create_dir_all(&app_data_dir).unwrap();
    let pool = Arc::new(crate::db::open_pool(&app_data_dir.join("runner.db")).unwrap());
    let windows = Arc::new(crate::windows::WindowRegistry::new());
    windows.register("main");
    let runtime_shell_env = Arc::new(std::sync::RwLock::new(
        crate::shell_path::LoginShellEnv::default(),
    ));
    let runtime_discovery = Arc::new(std::sync::RwLock::new(
        crate::shell_path::DiscoveryState::startup(None, None),
    ));
    AppCore {
        db: pool,
        app_data_dir,
        sessions: crate::session::SessionManager::new(
            Arc::clone(&runtime_shell_env),
            Arc::clone(&runtime_discovery),
            runtime,
        ),
        runtime_shell_env,
        runtime_discovery,
        usage: Arc::new(crate::usage::UsageService::default()),
        buses: crate::event_bus::BusRegistry::new(),
        routers: crate::router::RouterRegistry::new(),
        mission_grid_hint: Arc::new(std::sync::Mutex::new(None)),
        mcp: Arc::new(crate::mcp::McpHandle::new()),
        windows,
        events: crate::events::EventChannel::new(),
        session_event_observer: Default::default(),
        app_version: "0.0.0-test".into(),
    }
}

fn output(seq: u64, text: &str) -> OutputEvent {
    OutputEvent {
        session_id: "replay-race".into(),
        mission_id: None,
        seq,
        bytes: text.as_bytes().to_vec(),
    }
}

#[test]
fn terminal_links_detect_plain_urls_and_osc_8_targets() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let terminal =
        TerminalSession::attach(core, "replay-race".into(), 80, 4, Arc::new(|| {})).unwrap();
    terminal
            .feed_output(&output(
                1,
                "plain https://example.com/path\r\n\x1b]8;;https://example.com/osc\x1b\\linked label\x1b]8;;\x1b\\",
            ))
            .unwrap();

    let plain = terminal
        .link_at(Point::new(Line(0), Column(10)))
        .expect("plain URL");
    assert_eq!(
        plain.target,
        LinkTarget::Url("https://example.com/path".into())
    );
    assert!(plain.contains(Point::new(Line(0), Column(6))));
    assert!(plain.contains(Point::new(Line(0), Column(29))));

    let osc = terminal
        .link_at(Point::new(Line(1), Column(3)))
        .expect("OSC 8 link");
    assert_eq!(
        osc.target,
        LinkTarget::Url("https://example.com/osc".into())
    );
    assert_eq!(osc.start, Point::new(Line(1), Column(0)));
    assert_eq!(osc.end, Point::new(Line(1), Column(11)));
}

#[test]
fn terminal_url_detection_follows_soft_wraps_but_not_hard_line_breaks() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let terminal =
        TerminalSession::attach(core, "replay-race".into(), 14, 4, Arc::new(|| {})).unwrap();
    terminal
        .feed_output(&output(1, "xxhttps://example.com/path"))
        .unwrap();

    let wrapped = terminal
        .link_at(Point::new(Line(1), Column(3)))
        .expect("wrapped URL");
    assert_eq!(
        wrapped.target,
        LinkTarget::Url("https://example.com/path".into())
    );
    assert_eq!(wrapped.start, Point::new(Line(0), Column(2)));
    assert_eq!(wrapped.end, Point::new(Line(1), Column(11)));

    {
        let mut term = terminal.term.lock();
        term.grid_mut()[Line(0)][Column(13)]
            .flags
            .remove(Flags::WRAPLINE);
    }
    assert_eq!(
        terminal
            .link_at(Point::new(Line(0), Column(3)))
            .expect("complete URL prefix")
            .target,
        LinkTarget::Url("https://exam".into())
    );
    assert!(terminal.link_at(Point::new(Line(1), Column(3))).is_none());
}

fn insert_session_row(
    core: &AppCore,
    id: &str,
    cwd: Option<&std::path::Path>,
    project_id: Option<String>,
) {
    let conn = core.db.get().unwrap();
    let mut row = crate::repo::session::SessionRowDb::new_running(id.into());
    row.cwd = cwd.map(|cwd| cwd.to_string_lossy().into_owned());
    row.project_id = project_id;
    crate::repo::session::insert(&conn, &row).unwrap();
}

fn file_target(path: std::path::PathBuf, line: Option<u32>, column: Option<u32>) -> LinkTarget {
    LinkTarget::File { path, line, column }
}

#[test]
fn terminal_file_links_resolve_existing_paths_against_the_session_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let cwd = temp.path().join("project");
    std::fs::create_dir_all(cwd.join("src")).unwrap();
    for file in ["src/lib.rs", "README.md", "Makefile"] {
        std::fs::write(cwd.join(file), "").unwrap();
    }
    insert_session_row(&core, "file-links", Some(&cwd), None);
    let terminal =
        TerminalSession::attach(core, "file-links".into(), 120, 6, Arc::new(|| {})).unwrap();
    terminal
            .feed_output(&output(
                1,
                "see src/lib.rs:12:5, then ./README.md and Makefile\r\nsrc/lib.rs(3,4) src/lib.rs#L9 src/nope.rs:1 a/b and/or 1.2.3 (src/lib.rs:2).",
            ))
            .unwrap();
    let link = |column: usize| terminal.link_at(Point::new(Line(0), Column(column)));
    let link_1 = |column: usize| terminal.link_at(Point::new(Line(1), Column(column)));

    let with_line = link(6).expect("path with line and column");
    assert_eq!(
        with_line.target,
        file_target(cwd.join("src/lib.rs"), Some(12), Some(5))
    );
    assert_eq!(with_line.start, Point::new(Line(0), Column(4)));
    assert_eq!(with_line.end, Point::new(Line(0), Column(18)));
    assert!(
        link(19).is_none(),
        "the trailing comma is not part of the link"
    );

    let dot_relative = link(30).expect("./ relative path");
    assert_eq!(
        dot_relative.target,
        file_target(cwd.join("README.md"), None, None)
    );
    assert_eq!(dot_relative.start, Point::new(Line(0), Column(26)));
    assert_eq!(dot_relative.end, Point::new(Line(0), Column(36)));

    let bare_name = link(45).expect("extension-less file that exists");
    assert_eq!(
        bare_name.target,
        file_target(cwd.join("Makefile"), None, None)
    );
    assert!(link(39).is_none(), "`and` is not a file");

    let parens = link_1(2).expect("(line,col) suffix");
    assert_eq!(
        parens.target,
        file_target(cwd.join("src/lib.rs"), Some(3), Some(4))
    );
    assert_eq!(parens.end, Point::new(Line(1), Column(14)));

    let hash_line = link_1(20).expect("#L suffix");
    assert_eq!(
        hash_line.target,
        file_target(cwd.join("src/lib.rs"), Some(9), None)
    );
    assert_eq!(hash_line.end, Point::new(Line(1), Column(28)));

    assert!(
        link_1(32).is_none(),
        "a path that does not exist never links"
    );
    assert!(link_1(45).is_none(), "a/b never links");
    assert!(link_1(50).is_none(), "and/or never links");
    assert!(link_1(57).is_none(), "version strings never link");

    let wrapped_in_parens = link_1(65).expect("path inside parentheses");
    assert_eq!(
        wrapped_in_parens.target,
        file_target(cwd.join("src/lib.rs"), Some(2), None)
    );
    assert_eq!(wrapped_in_parens.start, Point::new(Line(1), Column(62)));
    assert_eq!(wrapped_in_parens.end, Point::new(Line(1), Column(73)));
}

#[test]
fn terminal_file_links_fall_back_to_the_project_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let project_dir = temp.path().join("proj");
    std::fs::create_dir_all(&project_dir).unwrap();
    std::fs::write(project_dir.join("notes.md"), "").unwrap();
    let project = {
        let conn = core.db.get().unwrap();
        crate::repo::project::create(&conn, "Proj", project_dir.to_str().unwrap()).unwrap()
    };
    insert_session_row(&core, "project-links", None, Some(project.id));
    let terminal =
        TerminalSession::attach(core, "project-links".into(), 80, 4, Arc::new(|| {})).unwrap();
    terminal.feed_output(&output(1, "edited notes.md")).unwrap();

    let link = terminal
        .link_at(Point::new(Line(0), Column(9)))
        .expect("project-relative path");
    assert_eq!(
        link.target,
        file_target(project_dir.join("notes.md"), None, None)
    );
}

#[cfg(unix)]
#[test]
fn terminal_file_links_without_a_cwd_only_resolve_absolute_paths() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let file = temp.path().join("abs.rs");
    std::fs::write(&file, "").unwrap();
    let terminal = TerminalSession::attach(core, "no-row".into(), 200, 4, Arc::new(|| {})).unwrap();
    let absolute = file.display().to_string();
    terminal
        .feed_output(&output(1, &format!("{absolute} abs.rs")))
        .unwrap();

    let link = terminal
        .link_at(Point::new(Line(0), Column(1)))
        .expect("absolute path");
    assert_eq!(link.target, file_target(file.clone(), None, None));
    assert!(terminal
        .link_at(Point::new(Line(0), Column(absolute.len() + 2)))
        .is_none());
}

#[test]
fn terminal_osc_8_file_uris_map_to_file_targets() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let terminal =
        TerminalSession::attach(core, "osc-files".into(), 80, 4, Arc::new(|| {})).unwrap();
    terminal
            .feed_output(&output(
                1,
                "\x1b]8;;file:///tmp/x.rs:7\x1b\\x.rs\x1b]8;;\x1b\\ \x1b]8;;file://localhost/tmp/y%20z.rs#L12\x1b\\y\x1b]8;;\x1b\\",
            ))
            .unwrap();

    let with_suffix = terminal
        .link_at(Point::new(Line(0), Column(1)))
        .expect("file URI with :line");
    assert_eq!(
        with_suffix.target,
        file_target("/tmp/x.rs".into(), Some(7), None)
    );
    let with_fragment = terminal
        .link_at(Point::new(Line(0), Column(5)))
        .expect("file URI with #L fragment");
    assert_eq!(
        with_fragment.target,
        file_target("/tmp/y z.rs".into(), Some(12), None)
    );
}

fn insert_runtime_session_row(core: &AppCore, id: &str, runtime: &str, cwd: &std::path::Path) {
    let conn = core.db.get().unwrap();
    let mut row = crate::repo::session::SessionRowDb::new_running(id.into());
    row.cwd = Some(cwd.to_string_lossy().into_owned());
    row.agent_runtime = Some(runtime.into());
    crate::repo::session::insert(&conn, &row).unwrap();
}

fn cwd_report(path: &std::path::Path, terminator: &str) -> String {
    let path = path.to_string_lossy().replace('\\', "/");
    let path = if path.starts_with('/') {
        path
    } else {
        format!("/{path}")
    };
    format!(
        "\x1b]7;file://localhost{}{terminator}",
        path.replace(' ', "%20")
    )
}

#[test]
fn a_shell_session_keeps_its_last_valid_osc7_directory_while_it_exists() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let spawn = temp.path().join("spawn");
    let live = temp.path().join("live dir");
    std::fs::create_dir_all(&spawn).unwrap();
    std::fs::create_dir_all(&live).unwrap();
    insert_runtime_session_row(&core, "shell-cwd", "shell", &spawn);
    let terminal =
        TerminalSession::attach(core, "shell-cwd".into(), 80, 4, Arc::new(|| {})).unwrap();
    let feed = |seq: u64, text: &str| {
        terminal
            .feed_output(&OutputEvent {
                session_id: "shell-cwd".into(),
                mission_id: None,
                seq,
                bytes: text.as_bytes().to_vec(),
            })
            .unwrap()
    };
    assert_eq!(terminal.live_cwd(), None);

    let report = cwd_report(&live, "\x1b\\");
    let (head, tail) = report.split_at(report.len() / 2);
    feed(1, &format!("$ cd 'live dir'\r\n{head}"));
    assert_eq!(terminal.live_cwd(), None);
    feed(2, &format!("{tail}$ "));
    assert_eq!(terminal.live_cwd(), Some(live.clone()));

    feed(
            3,
            "\x1b]7;file://runner-575-elsewhere.invalid/srv\x07\x1b]7;garbage\x07\x1b]7;file://localhost\x07",
        );
    assert_eq!(
        terminal.live_cwd(),
        Some(live.clone()),
        "ignored reports keep the previous directory"
    );

    let missing = temp.path().join("reported but missing");
    feed(4, &cwd_report(&missing, "\x07"));
    assert_eq!(
        terminal.live_cwd(),
        None,
        "a missing directory is not offered"
    );
    feed(5, &cwd_report(&live, "\x07"));
    assert_eq!(terminal.live_cwd(), Some(live.clone()));
    std::fs::remove_dir(&live).unwrap();
    assert_eq!(
        terminal.live_cwd(),
        None,
        "a removed directory is not offered"
    );
}

#[test]
fn agent_sessions_never_take_a_live_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    insert_runtime_session_row(&core, "agent-cwd", "claude-code", temp.path());
    let terminal =
        TerminalSession::attach(core, "agent-cwd".into(), 80, 4, Arc::new(|| {})).unwrap();
    terminal
        .feed_output(&OutputEvent {
            session_id: "agent-cwd".into(),
            mission_id: None,
            seq: 1,
            bytes: cwd_report(temp.path(), "\x07").into_bytes(),
        })
        .unwrap();
    assert_eq!(terminal.live_cwd(), None);
    assert!(!terminal.model.is_shell());
}

#[test]
fn relative_file_links_try_the_live_cwd_then_the_spawn_cwd() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let spawn = temp.path().join("project");
    let live = spawn.join("crates/app");
    std::fs::create_dir_all(spawn.join("src")).unwrap();
    std::fs::create_dir_all(&live).unwrap();
    for file in ["src/lib.rs", "README.md"] {
        std::fs::write(spawn.join(file), "").unwrap();
    }
    for file in ["main.rs", "README.md"] {
        std::fs::write(live.join(file), "").unwrap();
    }
    insert_runtime_session_row(&core, "shell-links", "shell", &spawn);
    let terminal =
        TerminalSession::attach(core, "shell-links".into(), 120, 4, Arc::new(|| {})).unwrap();
    terminal
        .feed_output(&OutputEvent {
            session_id: "shell-links".into(),
            mission_id: None,
            seq: 1,
            bytes: format!(
                "{}main.rs:3 src/lib.rs README.md",
                cwd_report(&live, "\x07")
            )
            .into_bytes(),
        })
        .unwrap();
    let link = |column: usize| {
        terminal
            .link_at(Point::new(Line(0), Column(column)))
            .map(|link| link.target)
    };
    assert_eq!(
        link(1),
        Some(file_target(live.join("main.rs"), Some(3), None))
    );
    assert_eq!(
        link(12),
        Some(file_target(spawn.join("src/lib.rs"), None, None))
    );
    assert_eq!(
        link(24),
        Some(file_target(live.join("README.md"), None, None))
    );
}

#[test]
fn queued_user_input_reports_backend_errors_without_blocking_the_caller() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let mut events = core.events.subscribe();
    let terminal = TerminalSession::attach_with_input_mode(
        core,
        "missing-session".into(),
        80,
        24,
        Arc::new(|| {}),
        UserInputMode::Queued,
    )
    .unwrap();

    terminal.write_user_bytes(b"x").unwrap();

    let event = events.blocking_recv().unwrap();
    assert_eq!(event.name, "session/input-error");
    assert_eq!(event.payload["session_id"], "missing-session");
    assert!(event.payload["message"]
        .as_str()
        .unwrap()
        .contains("session not found"));
}

/// Stands in for the PTY: hands the manager a live output channel
/// and records every byte the manager writes back to stdin.
#[derive(Default)]
struct RecordingRuntime {
    output: std::sync::Mutex<Option<std::sync::mpsc::Sender<RuntimeOutput>>>,
    writes: std::sync::Mutex<Vec<Vec<u8>>>,
}

impl RecordingRuntime {
    fn push_output(&self, bytes: &[u8]) {
        let output = self.output.lock().unwrap();
        output
            .as_ref()
            .expect("spawned")
            .send(RuntimeOutput::Stream(bytes.to_vec()))
            .unwrap();
    }

    fn writes(&self) -> Vec<Vec<u8>> {
        self.writes.lock().unwrap().clone()
    }
}

impl SessionRuntime for RecordingRuntime {
    fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
        let (tx, rx) = std::sync::mpsc::channel();
        *self.output.lock().unwrap() = Some(tx);
        Ok((
            RuntimeSession {
                runtime: "recording".into(),
                session_id: spec.session_id,
            },
            OutputStream::new(rx, Arc::new(AtomicBool::new(false))),
        ))
    }

    fn stop(&self, _: &RuntimeSession) -> RuntimeResult<()> {
        self.output.lock().unwrap().take();
        Ok(())
    }

    fn send_bytes(&self, _: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()> {
        self.writes.lock().unwrap().push(bytes.to_vec());
        Ok(())
    }

    fn send_key(&self, _: &RuntimeSession, key: &str) -> RuntimeResult<()> {
        self.writes.lock().unwrap().push(key.as_bytes().to_vec());
        Ok(())
    }

    fn resize(&self, _: &RuntimeSession, _: u16, _: u16) -> RuntimeResult<()> {
        Ok(())
    }

    fn status(&self, _: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
        Ok(Some(SessionStatus {
            alive: true,
            ..Default::default()
        }))
    }
}

/// The terminal is the only thing answering a session's queries,
/// and it answers whether or not a pane shows the session (#213's
/// contract, #524's regression): one reply per query, in order.
#[test]
fn hidden_terminal_answers_each_query_once() {
    for (palette, background) in [
        (
            runner_terminal::palette::RUNNER,
            b"\x1b]11;rgb:1515/1616/1b1b\x1b\\".as_slice(),
        ),
        (
            runner_terminal::palette::ROSE_PINE_DAWN,
            b"\x1b]11;rgb:fafa/f4f4/eded\x1b\\".as_slice(),
        ),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Arc::new(RecordingRuntime::default());
        let core = test_core_with_runtime(temp.path(), Arc::clone(&runtime) as _);
        let bridge = TerminalBridge::new(core.clone(), Arc::new(|| {})).unwrap();
        bridge.set_palette(palette);
        let role = crate::ops::role::create(
            &core.db.get().unwrap(),
            crate::ops::role::CreateRoleInput {
                handle: "probe".into(),
                display_name: "Probe".into(),
                runtime: crate::model::Runtime::Trae,
                command: "probe".into(),
                args: Vec::new(),
                working_dir: None,
                system_prompt: None,
                env: Default::default(),
                model: None,
                effort: None,
                codex_speed: None,
                permission_mode: crate::router::runtime::PermissionMode::Auto,
            },
        )
        .unwrap();
        let spawned = core
            .sessions
            .spawn_direct(
                &role,
                None,
                None,
                None,
                None,
                Some(temp.path().to_str().unwrap()),
                Some(80),
                Some(24),
                &core.app_data_dir,
                Arc::clone(&core.db),
                Arc::new(core.session_events()),
                None,
            )
            .unwrap();
        assert!(bridge.session(&spawned.id).is_some());
        assert_eq!(bridge.session(&spawned.id).unwrap().viewer_count(), 0);

        runtime.push_output(b"\x1b]11;?\x1b\\\x1b[c");

        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while runtime.writes().len() < 2 && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::thread::sleep(std::time::Duration::from_millis(100));
        let writes = runtime.writes();
        assert_eq!(
            writes.len(),
            2,
            "expected one OSC 11 report and one DA1 reply, got {writes:?}"
        );
        assert_eq!(writes[0], background);
        assert_eq!(writes[1], b"\x1b[?6c");
        core.sessions.kill(&spawned.id).ok();
    }
}

/// #755: Runner answers the kitty keyboard query, follows the app's
/// push and pop, and keeps doing so after `configure`. While the app has
/// the protocol on, Shift+Enter and the chords whose legacy bytes a
/// strict decoder stops recognising (pi's Alt+Enter and Alt+D) go out
/// as `CSI u`; outside it they keep their legacy bytes. The answer to a
/// query names only the flag Runner implements, whatever the app pushed.
#[test]
fn keys_follow_the_apps_kitty_keyboard_mode() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Arc::new(RecordingRuntime::default());
    let core = test_core_with_runtime(temp.path(), Arc::clone(&runtime) as _);
    let bridge = TerminalBridge::new(core.clone(), Arc::new(|| {})).unwrap();
    let role = crate::ops::role::create(
        &core.db.get().unwrap(),
        crate::ops::role::CreateRoleInput {
            handle: "probe".into(),
            display_name: "Probe".into(),
            runtime: crate::model::Runtime::Trae,
            command: "probe".into(),
            args: Vec::new(),
            working_dir: None,
            system_prompt: None,
            env: Default::default(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: crate::router::runtime::PermissionMode::Auto,
        },
    )
    .unwrap();
    let spawned = core
        .sessions
        .spawn_direct(
            &role,
            None,
            None,
            None,
            None,
            Some(temp.path().to_str().unwrap()),
            Some(80),
            Some(24),
            &core.app_data_dir,
            Arc::clone(&core.db),
            Arc::new(core.session_events()),
            None,
        )
        .unwrap();
    let session = bridge.session(&spawned.id).unwrap();
    let wait_for = |count: usize| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while runtime.writes().len() < count && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        runtime.writes()
    };
    let shift_enter = || session.send_key("enter", false, false, true, None).unwrap();
    let alt_enter = || session.send_key("enter", false, true, false, None).unwrap();
    let alt_d = || {
        session
            .send_key("d", false, true, false, Some("∂"))
            .unwrap()
    };

    shift_enter();
    alt_enter();
    alt_d();
    assert_eq!(
        wait_for(3),
        [b"\x1b\r".to_vec(), b"\x1b\r".to_vec(), b"\x1bd".to_vec()],
        "legacy until the app opts in"
    );

    runtime.push_output(b"\x1b[>7u\x1b[?u");
    assert_eq!(
        wait_for(4)[3],
        b"\x1b[?1u",
        "only the disambiguate bit of the pushed 7 is acknowledged"
    );
    shift_enter();
    alt_enter();
    alt_d();
    session
        .send_key("escape", false, false, false, None)
        .unwrap();
    session.send_key("c", true, false, false, None).unwrap();
    assert_eq!(
        wait_for(9)[4..],
        [
            b"\x1b[13;2u".to_vec(),
            b"\x1b[13;3u".to_vec(),
            b"\x1b[100;3u".to_vec(),
            b"\x1b[27u".to_vec(),
            b"\x1b[99;5u".to_vec(),
        ]
    );

    session.configure(3, CursorShape::Beam);
    alt_d();
    assert_eq!(
        wait_for(10)[9],
        b"\x1b[100;3u",
        "configure keeps the protocol on"
    );

    runtime.push_output(b"\x1b[<1u\x1b[?u");
    assert_eq!(wait_for(11)[10], b"\x1b[?0u");
    alt_enter();
    alt_d();
    assert_eq!(
        wait_for(13)[11..],
        [b"\x1b\r".to_vec(), b"\x1bd".to_vec()],
        "legacy again once the app pops"
    );

    runtime.push_output(b"\x1b[>31u\x1b[?u");
    assert_eq!(
        wait_for(14)[13],
        b"\x1b[?1u",
        "event, alternate, all-keys and text flags are not acknowledged"
    );
    shift_enter();
    assert_eq!(wait_for(15)[14], b"\x1b[13;2u");

    runtime.push_output(b"\x1b[<1u\x1b[>8u\x1b[?u");
    assert_eq!(
        wait_for(16)[15],
        b"\x1b[?0u",
        "all-keys alone is not the disambiguate level"
    );
    shift_enter();
    assert_eq!(wait_for(17)[16], b"\x1b\r");

    // Batched in one chunk, each query is answered from the mode at its
    // own position, not from wherever the chunk ends.
    runtime.push_output(b"\x1b[<u\x1b[>1u\x1b[?u\x1b[<u\x1b[?u");
    assert_eq!(
        wait_for(19)[17..],
        [b"\x1b[?1u".to_vec(), b"\x1b[?0u".to_vec()],
        "push, query, pop, query"
    );
    runtime.push_output(b"\x1b[?u\x1b[>1u\x1b[?u\x1b[<u");
    assert_eq!(
        wait_for(21)[19..],
        [b"\x1b[?0u".to_vec(), b"\x1b[?1u".to_vec()],
        "query, push, query"
    );
    core.sessions.kill(&spawned.id).ok();
}

#[test]
fn a_subscribed_tui_learns_of_a_scheme_flip() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Arc::new(RecordingRuntime::default());
    let core = test_core_with_runtime(temp.path(), Arc::clone(&runtime) as _);
    let bridge = TerminalBridge::new(core.clone(), Arc::new(|| {})).unwrap();
    bridge.set_palette(runner_terminal::palette::RUNNER);
    let role = crate::ops::role::create(
        &core.db.get().unwrap(),
        crate::ops::role::CreateRoleInput {
            handle: "probe".into(),
            display_name: "Probe".into(),
            runtime: crate::model::Runtime::Trae,
            command: "probe".into(),
            args: Vec::new(),
            working_dir: None,
            system_prompt: None,
            env: Default::default(),
            model: None,
            effort: None,
            codex_speed: None,
            permission_mode: crate::router::runtime::PermissionMode::Auto,
        },
    )
    .unwrap();
    let spawned = core
        .sessions
        .spawn_direct(
            &role,
            None,
            None,
            None,
            None,
            Some(temp.path().to_str().unwrap()),
            Some(80),
            Some(24),
            &core.app_data_dir,
            Arc::clone(&core.db),
            Arc::new(core.session_events()),
            None,
        )
        .unwrap();
    assert!(bridge.session(&spawned.id).is_some());
    let wait_for = |count: usize| {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
        while runtime.writes().len() < count && std::time::Instant::now() < deadline {
            std::thread::sleep(std::time::Duration::from_millis(10));
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
        runtime.writes()
    };

    runtime.push_output(b"\x1b[?2031$p");
    assert_eq!(wait_for(1), vec![b"\x1b[?2031;2$y".to_vec()]);

    runtime.push_output(b"\x1b[?2031h\x1b[?996n\x1b[?2031$p");
    let writes = wait_for(3);
    assert_eq!(
        &writes[1..],
        [b"\x1b[?997;1n".to_vec(), b"\x1b[?2031;1$y".to_vec()]
    );

    bridge.set_palette(runner_terminal::palette::ROSE_PINE_DAWN);
    assert_eq!(wait_for(4)[3], b"\x1b[?997;2n".to_vec());
    bridge.set_palette(runner_terminal::palette::CATPPUCCIN_MOCHA);
    assert_eq!(wait_for(5)[4], b"\x1b[?997;1n".to_vec());
    bridge.set_palette(runner_terminal::palette::RUNNER);
    assert_eq!(wait_for(5).len(), 5, "dark to dark is not a scheme change");

    runtime.push_output(b"\x1b[?2031l");
    std::thread::sleep(std::time::Duration::from_millis(100));
    bridge.set_palette(runner_terminal::palette::ROSE_PINE_DAWN);
    assert_eq!(wait_for(5).len(), 5, "unsubscribed sessions get no report");
    core.sessions.kill(&spawned.id).ok();
}

#[test]
fn unseen_terminals_inherit_the_bridge_palette() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let bridge = TerminalBridge::new(core, Arc::new(|| {})).unwrap();
    bridge.set_palette(runner_terminal::palette::ROSE_PINE_DAWN);
    for (session_id, mission_id) in [("direct", None), ("slot", Some("mission".into()))] {
        bridge.output(&OutputEvent {
            session_id: session_id.into(),
            mission_id,
            seq: 1,
            bytes: b"ready".to_vec(),
        });
        assert_eq!(
            bridge.session(session_id).unwrap().palette(),
            runner_terminal::palette::ROSE_PINE_DAWN
        );
    }
    bridge.set_palette(runner_terminal::palette::RUNNER);
    assert_eq!(
        bridge.session("direct").unwrap().palette(),
        runner_terminal::palette::RUNNER
    );
    assert_eq!(
        bridge.session("slot").unwrap().palette(),
        runner_terminal::palette::RUNNER
    );
}

#[test]
fn registry_creates_on_first_output_and_survives_view_drop() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events_core = core.clone();
    let mut broadcast = core.events.subscribe();
    let bridge = TerminalBridge::new(core, Arc::new(|| {})).unwrap();
    let events = super::support::TestEvents::new(events_core.clone(), Arc::clone(&bridge));
    assert_eq!(bridge.live_session_count(), 0);

    events.output(&output(1, "first-marker"));
    assert!(broadcast.try_recv().is_err());
    let view = bridge.session("replay-race").unwrap();
    assert_eq!(bridge.live_session_count(), 1);
    drop(view);

    events.output(&output(2, "second-marker"));
    let reopened = bridge.session("replay-race").unwrap();
    let rendered = {
        let term = reopened.term.lock();
        visible_lines(&*term).join("\n")
    };
    assert!(rendered.contains("first-markersecond-marker"));
}

#[test]
fn hidden_terminal_output_does_not_wake_the_ui() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let wakes = Arc::new(AtomicUsize::new(0));
    let wake_count = Arc::clone(&wakes);
    let terminal = TerminalSession::attach(
        core,
        "replay-race".into(),
        80,
        24,
        Arc::new(move || {
            wake_count.fetch_add(1, Ordering::Relaxed);
        }),
    )
    .unwrap();

    terminal.feed_output(&output(1, "hidden")).unwrap();
    assert_eq!(wakes.load(Ordering::Relaxed), 0);

    let view = terminal.view();
    terminal.feed_output(&output(2, "visible")).unwrap();
    assert_eq!(wakes.load(Ordering::Relaxed), 1);

    drop(view);
    terminal.feed_output(&output(3, "hidden-again")).unwrap();
    assert_eq!(wakes.load(Ordering::Relaxed), 1);
}

#[test]
fn registry_removes_exit_and_archive_sessions() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events_core = core.clone();
    let bridge = TerminalBridge::new(core, Arc::new(|| {})).unwrap();
    let events = super::support::TestEvents::new(events_core.clone(), Arc::clone(&bridge));
    events.output(&output(1, "exit-marker"));
    let exited = Arc::downgrade(&bridge.session("replay-race").unwrap());
    events.exit(&ExitEvent {
        session_id: "replay-race".into(),
        mission_id: None,
        exit_code: Some(0),
        success: true,
    });
    assert!(bridge.session("replay-race").is_none());
    assert!(exited.upgrade().is_none());

    events.output(&output(2, "archive-marker"));
    let archived = Arc::downgrade(&bridge.session("replay-race").unwrap());
    events.archived(&SessionUpdatedEvent {
        session_id: "replay-race".into(),
        mission_id: None,
    });
    assert_eq!(bridge.live_session_count(), 0);
    assert!(archived.upgrade().is_none());
}

#[test]
fn registry_replaces_terminal_on_respawn_and_resets_first_paint() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events_core = core.clone();
    let bridge = TerminalBridge::new(core, Arc::new(|| {})).unwrap();
    let events = super::support::TestEvents::new(events_core.clone(), Arc::clone(&bridge));
    events.spawned(&SessionSpawnedEvent {
        session_id: "replay-race".into(),
        mission_id: None,
        cols: 80,
        rows: 24,
    });
    events.output(&output(1, "old-child"));
    let old = bridge.session("replay-race").unwrap();
    assert_eq!(old.output_activity().first_paint_seq, 1);

    events.spawned(&SessionSpawnedEvent {
        session_id: "replay-race".into(),
        mission_id: None,
        cols: 100,
        rows: 30,
    });
    let fresh = bridge.session("replay-race").unwrap();
    assert!(!Arc::ptr_eq(&old, &fresh));
    assert_eq!(fresh.size(), (100, 30));
    assert_eq!(fresh.output_activity().first_paint_seq, 0);
    events.output(&output(2, "\x1b[?2004hnew-child"));

    let activity = fresh.output_activity();
    assert_eq!(activity.last_seq, 2);
    assert_eq!(activity.tui_ready_seq, 2);
    assert_eq!(activity.first_paint_seq, 2);
    let rendered = visible_lines(&*fresh.term.lock()).join("\n");
    assert!(rendered.contains("new-child"));
    assert!(!rendered.contains("old-child"));
}

#[test]
fn registry_has_zero_live_terminals_after_twenty_spawn_exit_cycles() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events_core = core.clone();
    let bridge = TerminalBridge::new(core, Arc::new(|| {})).unwrap();
    let events = super::support::TestEvents::new(events_core.clone(), Arc::clone(&bridge));

    for seq in 1..=20 {
        events.spawned(&SessionSpawnedEvent {
            session_id: "replay-race".into(),
            mission_id: None,
            cols: 80,
            rows: 24,
        });
        events.output(&output(seq, "cycle"));
        assert_eq!(bridge.live_session_count(), 1);
        events.exit(&ExitEvent {
            session_id: "replay-race".into(),
            mission_id: None,
            exit_code: Some(0),
            success: true,
        });
        assert_eq!(bridge.live_session_count(), 0);
    }
}

#[test]
fn terminal_scroll_state_and_absolute_scroll_stay_in_sync() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let woke = Arc::new(AtomicBool::new(false));
    let wake_flag = Arc::clone(&woke);
    let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
        wake_flag.store(true, Ordering::Release);
    });
    let terminal = TerminalSession::attach(core, "replay-race".into(), 80, 24, waker).unwrap();
    terminal
        .feed_output(&output(1, &"scrollback line\r\n".repeat(80)))
        .unwrap();

    let bottom = terminal.scroll_state();
    assert_eq!(bottom.screen_lines, 24);
    assert!(bottom.history_lines > 0);
    assert_eq!(bottom.display_offset, 0);

    woke.store(false, Ordering::Release);
    terminal.scroll_to_display_offset(bottom.history_lines);
    assert_eq!(terminal.scroll_state().display_offset, bottom.history_lines);
    assert!(woke.load(Ordering::Acquire));

    terminal.scroll_to_display_offset(0);
    assert_eq!(terminal.scroll_state().display_offset, 0);
}

#[test]
fn terminal_configuration_applies_scrollback_and_cursor_shape() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    let terminal = TerminalSession::attach(core, "replay-race".into(), 80, 5, waker).unwrap();

    terminal.configure(3, CursorShape::Beam);
    terminal
        .feed_output(&output(1, &"configured line\r\n".repeat(20)))
        .unwrap();

    let term = terminal.term.lock();
    assert_eq!(term.history_size(), 3);
    assert_eq!(term.renderable_content().cursor.shape, CursorShape::Beam);
}

#[test]
fn unrepresentable_legacy_wheel_does_not_scroll_local_history() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let terminal =
        TerminalSession::attach(core, "replay-race".into(), 80, 5, Arc::new(|| {})).unwrap();
    terminal
        .feed_output(&output(1, &"scrollback line\r\n".repeat(20)))
        .unwrap();
    terminal.feed_output(&output(2, "\x1b[?1000h")).unwrap();
    assert!(terminal.scroll_state().history_lines > 0);

    terminal.scroll(2, false, 223, 4);
    assert_eq!(terminal.scroll_state().display_offset, 0);
    terminal.scroll(2, true, 223, 4);
    assert_eq!(terminal.scroll_state().display_offset, 2);
}

#[test]
fn terminal_selection_matches_xterm_words_wraps_and_line_copy() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let terminal = TerminalSession::attach_with_input_mode(
        core,
        "replay-race".into(),
        8,
        3,
        Arc::new(|| {}),
        UserInputMode::Queued,
    )
    .unwrap();
    terminal.configure(100, CursorShape::Block);
    assert!(terminal.cell_is_whitespace(Point::new(Line(100), Column(100))));
    {
        let mut term = terminal.term.lock();
        for (column, character) in "foo/bar,".chars().enumerate() {
            term.grid_mut()[Line(0)][Column(column)].c = character;
        }
        for (column, character) in "abcdefgh".chars().enumerate() {
            term.grid_mut()[Line(1)][Column(column)].c = character;
        }
        term.grid_mut()[Line(1)][Column(7)]
            .flags
            .insert(Flags::WRAPLINE);
        for (column, character) in "ijk".chars().enumerate() {
            term.grid_mut()[Line(2)][Column(column)].c = character;
        }
    }

    terminal.start_selection(
        SelectionType::Semantic,
        Point::new(Line(0), Column(2)),
        Side::Left,
    );
    assert_eq!(terminal.selection_text().as_deref(), Some("foo/bar"));
    terminal.feed_output(&output(1, "\x1b[31m")).unwrap();
    terminal.scroll_local(1);
    assert_eq!(terminal.selection_text().as_deref(), Some("foo/bar"));

    terminal.start_selection(
        SelectionType::Simple,
        Point::new(Line(1), Column(5)),
        Side::Left,
    );
    terminal.update_selection(Point::new(Line(2), Column(1)), Side::Right);
    assert_eq!(terminal.selection_text().as_deref(), Some("fghij"));

    terminal.start_selection(
        SelectionType::Lines,
        Point::new(Line(2), Column(1)),
        Side::Left,
    );
    assert_eq!(terminal.selection_text().as_deref(), Some("abcdefghijk\n"));
}

#[test]
fn user_input_vertical_resize_and_mouse_mode_clear_selection() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let terminal = TerminalSession::attach_with_input_mode(
        core,
        "replay-race".into(),
        8,
        3,
        Arc::new(|| {}),
        UserInputMode::Queued,
    )
    .unwrap();
    let select = || {
        terminal.start_selection(
            SelectionType::Lines,
            Point::new(Line(0), Column(0)),
            Side::Left,
        );
        assert!(terminal.selection_text().is_some());
    };

    select();
    terminal.write_user_bytes(b"x").unwrap();
    assert!(terminal.selection_text().is_none());

    select();
    terminal.resize(8, 4);
    assert!(terminal.selection_text().is_none());

    select();
    terminal
        .feed_output(&output(1, "\x1b[?1000h\x1b[?1006h"))
        .unwrap();
    assert!(terminal.selection_text().is_none());
}

#[test]
fn output_activity_tracks_sequence_idle_time_and_tui_ready_signals() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(|| {});
    let terminal = TerminalSession::attach(core, "replay-race".into(), 80, 24, waker).unwrap();

    assert_eq!(terminal.output_activity().last_seq, 0);
    terminal.feed_output(&output(1, "booting")).unwrap();
    let first = terminal.output_activity();
    assert_eq!(first.last_seq, 1);
    assert_eq!(first.tui_ready_seq, 0);
    assert_eq!(first.first_paint_seq, 1);
    assert!(first.last_output_at.is_some());

    terminal
        .feed_output(&output(2, "\x1b[?2004hready"))
        .unwrap();
    let ready = terminal.output_activity();
    assert_eq!(ready.last_seq, 2);
    assert_eq!(ready.tui_ready_seq, 2);
    assert_eq!(ready.first_paint_seq, 1);
    assert!(ready.last_output_at >= first.last_output_at);
}

#[test]
fn first_paint_requires_a_visible_non_whitespace_cell() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let terminal =
        TerminalSession::attach(core, "replay-race".into(), 8, 3, Arc::new(|| {})).unwrap();

    terminal
        .feed_output(&output(1, "\x1b[?1049h\x1b[2J\x1b[H"))
        .unwrap();
    let alternate_screen = terminal.output_activity();
    assert_eq!(alternate_screen.tui_ready_seq, 1);
    assert_eq!(alternate_screen.first_paint_seq, 0);

    terminal
        .feed_output(&output(2, "\x1b[31m   \x1b[0m\x1b[2;2H"))
        .unwrap();
    assert_eq!(terminal.output_activity().first_paint_seq, 0);

    terminal.feed_output(&output(3, "x")).unwrap();
    assert_eq!(terminal.output_activity().first_paint_seq, 3);

    terminal.feed_output(&output(4, "later")).unwrap();
    assert_eq!(terminal.output_activity().first_paint_seq, 3);
}

/// The shape of #647's 09-21 probe: a redraw that opens a synchronized
/// update, clears, hides the cursor while drawing and shows it again, and
/// never sends the end marker.
const HELD_REDRAW: &str = "\x1b[?2026h\x1b[2J\x1b[H\x1b[?25lredrawn\r\n> \x1b[?25h";

fn bytes_output(seq: u64, bytes: &[u8]) -> OutputEvent {
    OutputEvent {
        session_id: "replay-race".into(),
        mission_id: None,
        seq,
        bytes: bytes.to_vec(),
    }
}

fn screen(terminal: &TerminalSession) -> Vec<String> {
    visible_lines(&*terminal.term.lock())
}

fn sync_deadline(terminal: &TerminalSession) -> Option<Instant> {
    terminal.sync_state().0
}

/// Polls in 5 ms steps for at most `limit`. The flushes under test land at
/// vte's 150 ms deadline, so the limit only bounds a failing run.
fn wait_until(limit: Duration, mut done: impl FnMut() -> bool) -> bool {
    let start = Instant::now();
    loop {
        if done() {
            return true;
        }
        if start.elapsed() >= limit {
            return false;
        }
        std::thread::sleep(Duration::from_millis(5));
    }
}

fn populated_terminal(core: AppCore) -> Arc<TerminalSession> {
    let terminal =
        TerminalSession::attach(core, "replay-race".into(), 20, 4, Arc::new(|| {})).unwrap();
    terminal
        .feed_output(&output(1, "old prompt\r\nold output"))
        .unwrap();
    terminal
}

#[test]
fn a_synchronized_update_without_its_end_marker_flushes_at_its_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let terminal = populated_terminal(test_core(temp.path()));
    terminal.feed_output(&output(2, HELD_REDRAW)).unwrap();
    let deadline = sync_deadline(&terminal).expect("the redraw is held");
    let held = screen(&terminal);
    if Instant::now() < deadline {
        assert_eq!(held[..2], ["old prompt", "old output"]);
    }

    assert!(wait_until(Duration::from_secs(2), || screen(&terminal)[0] == "redrawn"));
    assert!(Instant::now() >= deadline);
    {
        let term = terminal.term.lock();
        assert_eq!(visible_lines(&*term)[..2], ["redrawn", ">"]);
        assert!(term.mode().contains(TermMode::SHOW_CURSOR));
        assert_eq!(term.grid().cursor.point, Point::new(Line(1), Column(2)));
    }
    assert_eq!(sync_deadline(&terminal), None);

    terminal.feed_output(&output(3, "\x1b[?2026l")).unwrap();
    assert_eq!(screen(&terminal)[..2], ["redrawn", ">"]);
}

#[test]
fn resizes_neither_release_a_held_update_early_nor_lose_it() {
    let temp = tempfile::tempdir().unwrap();
    let terminal = populated_terminal(test_core(temp.path()));
    terminal.feed_output(&output(2, HELD_REDRAW)).unwrap();
    let deadline = sync_deadline(&terminal).expect("the redraw is held");

    for (cols, rows) in [(30, 6), (12, 3), (24, 5)] {
        terminal.resize(cols, rows);
    }
    let resized = screen(&terminal);
    let held_bytes = terminal.sync_state().1;
    if Instant::now() < deadline {
        assert!(held_bytes > 0);
        assert!(!resized.iter().any(|line| line.contains("redrawn")));
    }

    assert!(wait_until(Duration::from_secs(2), || screen(&terminal)[0] == "redrawn"));
    assert_eq!(screen(&terminal), ["redrawn", ">", "", "", ""]);
}

#[test]
fn a_complete_update_split_anywhere_applies_once_without_a_timeout_flush() {
    const UPDATE: &[u8] = b"\x1b[?2026habc\r\n\x1b[?2026l";
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let wakes = Arc::new(AtomicUsize::new(0));
    let mut terminals = Vec::new();
    for split in 0..=UPDATE.len() {
        let wake_count = Arc::clone(&wakes);
        let terminal = TerminalSession::attach(
            core.clone(),
            format!("split-{split}"),
            20,
            4,
            Arc::new(move || {
                wake_count.fetch_add(1, Ordering::Relaxed);
            }),
        )
        .unwrap();
        let view = terminal.view();
        terminal.feed_output(&output(1, "top\r\n")).unwrap();
        terminal
            .feed_output(&bytes_output(2, &UPDATE[..split]))
            .unwrap();
        terminal
            .feed_output(&bytes_output(3, &UPDATE[split..]))
            .unwrap();
        assert_eq!(
            screen(&terminal),
            ["top", "abc", "", ""],
            "split at {split}"
        );
        terminals.push((split, terminal, view));
    }
    let fed_wakes = wakes.load(Ordering::Relaxed);

    // A flusher scheduled while the update was open wakes at its deadline
    // and stands down.
    assert!(wait_until(Duration::from_secs(2), || terminals
        .iter()
        .all(|(_, terminal, _)| !terminal.sync_state().2)));
    assert_eq!(wakes.load(Ordering::Relaxed), fed_wakes);
    for (split, terminal, _) in &terminals {
        assert_eq!(screen(terminal), ["top", "abc", "", ""], "split at {split}");
        assert_eq!(sync_deadline(terminal), None, "split at {split}");
    }
}

#[test]
fn a_flush_scheduled_for_a_dropped_session_does_nothing() {
    let temp = tempfile::tempdir().unwrap();
    let terminal = populated_terminal(test_core(temp.path()));
    terminal.feed_output(&output(2, HELD_REDRAW)).unwrap();
    let deadline = sync_deadline(&terminal).expect("the redraw is held");
    let term = Arc::clone(&terminal.term);
    let session = Arc::downgrade(&terminal);

    drop(terminal);
    assert!(wait_until(Duration::from_secs(2), || session
        .upgrade()
        .is_none()));
    std::thread::sleep(
        deadline.saturating_duration_since(Instant::now()) + Duration::from_millis(100),
    );
    assert_eq!(
        visible_lines(&*term.lock())[..2],
        ["old prompt", "old output"]
    );
}

#[test]
fn a_begin_marker_inside_a_held_update_extends_its_deadline() {
    let temp = tempfile::tempdir().unwrap();
    let terminal = populated_terminal(test_core(temp.path()));
    terminal
        .feed_output(&output(2, "\x1b[?2026h\x1b[2J\x1b[H\x1b[?25lredrawn"))
        .unwrap();
    let first = sync_deadline(&terminal).expect("the redraw is held");
    std::thread::sleep(Duration::from_millis(75));
    const EXTENSION: &str = "\x1b[?2026h\r\n> \x1b[?25h";
    terminal.feed_output(&output(3, EXTENSION)).unwrap();
    let extended = sync_deadline(&terminal).expect("the redraw is still held");
    assert!(extended > first);
    // A stalled runner can oversleep past `first`, so the first update
    // flushes on time and the extension opens a new one instead.
    let extended_in_time = terminal.sync_state().1 > EXTENSION.len();

    std::thread::sleep(first.saturating_duration_since(Instant::now()) + Duration::from_millis(20));
    let after_first = screen(&terminal);
    if extended_in_time && Instant::now() < extended {
        assert_eq!(after_first[..2], ["old prompt", "old output"]);
    }

    assert!(wait_until(Duration::from_secs(2), || screen(&terminal)
        [..2]
        == ["redrawn", ">"]));
    assert_eq!(screen(&terminal)[..2], ["redrawn", ">"]);
}

#[test]
fn da1_has_one_reply_with_zero_one_and_two_mirrors() {
    for mirror_count in 0..=2 {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Arc::new(RecordingRuntime::default());
        let core = test_core_with_runtime(temp.path(), runtime.clone());
        let role = crate::ops::role::create(
            &core.db.get().unwrap(),
            crate::ops::role::CreateRoleInput {
                handle: "one-reply".into(),
                display_name: "One reply".into(),
                runtime: crate::model::Runtime::Trae,
                command: "probe".into(),
                args: Vec::new(),
                working_dir: None,
                system_prompt: None,
                env: Default::default(),
                model: None,
                effort: None,
                codex_speed: None,
                permission_mode: crate::router::runtime::PermissionMode::Auto,
            },
        )
        .unwrap();
        let spawned = core
            .sessions
            .spawn_direct(
                &role,
                None,
                None,
                None,
                None,
                Some(temp.path().to_str().unwrap()),
                Some(80),
                Some(24),
                &core.app_data_dir,
                Arc::clone(&core.db),
                Arc::new(core.session_events()),
                None,
            )
            .unwrap();
        let client = crate::daemon::InProcessTransport::client(core.clone());
        let mirrors: Vec<_> = (0..mirror_count)
            .map(|_| {
                runner_terminal::terminal::TerminalMirror::attach(
                    client.clone(),
                    spawned.id.clone(),
                    Arc::new(|| {}),
                )
                .unwrap()
            })
            .collect();
        runtime.push_output(b"\x1b[c");
        assert!(wait_until(Duration::from_secs(5), || !runtime
            .writes()
            .is_empty()));
        core.sessions
            .terminal_model(&spawned.id)
            .unwrap()
            .flush_events();
        assert!(wait_until(Duration::from_secs(5), || mirrors
            .iter()
            .all(|mirror| mirror.output_activity().last_seq == 1)));
        assert_eq!(
            runtime
                .writes()
                .into_iter()
                .filter(|bytes| !bytes.is_empty())
                .collect::<Vec<_>>(),
            vec![b"\x1b[?6c".to_vec()],
            "{mirror_count} mirrors"
        );
        core.sessions.kill(&spawned.id).unwrap();
    }
}

#[test]
fn full_frame_queue_requires_a_fresh_snapshot() {
    use runner_core::protocol::terminal::TerminalFrame;
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal("frames", (80, 24), &core.db, &events)
        .unwrap();
    let client = crate::daemon::InProcessTransport::client(core.clone());
    let mut attachment = client.attach("frames").unwrap();
    for _ in 0..=super::FRAME_CAPACITY {
        core.sessions
            .ingest_output_chunk("frames", None, b"x", events.as_ref());
    }
    assert!(matches!(
        attachment.frames.recv().unwrap(),
        TerminalFrame::Resync
    ));
    assert!(attachment.frames.recv().is_err());
    let fresh = client.attach("frames").unwrap();
    assert_eq!(fresh.snapshot.seq, super::FRAME_CAPACITY as u64 + 1);
    let restored = runner_terminal::replay::replay_bytes(
        fresh.snapshot.cols,
        fresh.snapshot.rows,
        &fresh.snapshot.bytes,
    );
    assert!(visible_lines(&restored)
        .join("")
        .contains(&"x".repeat(super::FRAME_CAPACITY + 1)));
}

#[test]
fn attach_and_output_share_the_sequence_parse_lock() {
    use runner_core::protocol::terminal::TerminalFrame;
    for _ in 0..30 {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        core.sessions
            .prepare_unlisted_terminal("frames", (40, 24), &core.db, &events)
            .unwrap();
        let barrier = Arc::new(std::sync::Barrier::new(2));
        let writer_core = core.clone();
        let writer_events = Arc::clone(&events);
        let writer_barrier = Arc::clone(&barrier);
        let writer = std::thread::spawn(move || {
            writer_barrier.wait();
            for n in 0..32 {
                writer_core.sessions.ingest_output_chunk(
                    "frames",
                    None,
                    format!("{n}\r\n").as_bytes(),
                    writer_events.as_ref(),
                );
            }
        });
        barrier.wait();
        let mut attachment = crate::daemon::InProcessTransport::client(core.clone())
            .attach("frames")
            .unwrap();
        let mut restored = runner_terminal::replay::replay_bytes(
            attachment.snapshot.cols,
            attachment.snapshot.rows,
            &attachment.snapshot.bytes,
        );
        let mut parser: alacritty_terminal::vte::ansi::Processor =
            alacritty_terminal::vte::ansi::Processor::new();
        let mut seq = attachment.snapshot.seq;
        writer.join().unwrap();
        while seq < 32 {
            let TerminalFrame::Output { seq: next, bytes } = attachment.frames.recv().unwrap()
            else {
                panic!("unexpected frame");
            };
            assert_eq!(next, seq + 1);
            seq = next;
            parser.advance(&mut restored, &bytes);
        }
        let model = core.sessions.terminal_model("frames").unwrap();
        let original = model.term.lock();
        assert_eq!(restored.history_size(), original.history_size());
        assert_eq!(restored.grid().cursor, original.grid().cursor);
        for line in -(original.history_size() as i32)..original.screen_lines() as i32 {
            for column in 0..original.columns() {
                assert_eq!(
                    restored.grid()[Line(line)][Column(column)],
                    original.grid()[Line(line)][Column(column)]
                );
            }
        }
        assert_eq!(restored.mode(), original.mode());
    }
}

#[test]
fn resize_is_broadcast_to_every_subscriber_except_its_origin() {
    use runner_core::protocol::terminal::TerminalFrame;
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal("resize", (80, 24), &core.db, &events)
        .unwrap();
    let client = crate::daemon::InProcessTransport::client(core.clone());
    let mut origin = client.attach("resize").unwrap();
    let mut other = client.attach("resize").unwrap();
    client.resize("resize", origin.subscriber_id, 96, 32);
    assert!(matches!(
        other.frames.recv().unwrap(),
        TerminalFrame::Resized {
            seq: 1,
            cols: 96,
            rows: 32
        }
    ));
    core.sessions
        .ingest_output_chunk("resize", None, b"after", events.as_ref());
    assert!(matches!(
        origin.frames.recv().unwrap(),
        TerminalFrame::Output { seq: 2, .. }
    ));
    assert_eq!(client.attach("resize").unwrap().snapshot.cols, 96);
}

#[test]
fn mirror_attach_preserves_rep_after_cursor_and_charset_changes() {
    use runner_terminal::terminal::TerminalMirror;
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal("rep", (20, 5), &core.db, &events)
        .unwrap();
    core.sessions
        .ingest_output_chunk("rep", None, b"abc\x1b[H\x1b(0", events.as_ref());
    let client = crate::daemon::InProcessTransport::client(core.clone());
    let mirror = TerminalMirror::attach(client, "rep".into(), Arc::new(|| {})).unwrap();
    let model = core.sessions.terminal_model("rep").unwrap();
    assert_eq!(
        visible_lines(&mirror.term.lock()),
        visible_lines(&model.term.lock())
    );
    core.sessions
        .ingest_output_chunk("rep", None, b"\x1b[3b", events.as_ref());
    let deadline = Instant::now() + Duration::from_secs(5);
    loop {
        let original = model.term.lock();
        let restored = mirror.term.lock();
        if visible_lines(&restored) == visible_lines(&original) {
            assert_eq!(restored.grid().cursor, original.grid().cursor);
            break;
        }
        assert!(
            Instant::now() < deadline,
            "mirror lost REP continuation state"
        );
        drop(restored);
        drop(original);
        std::thread::yield_now();
    }
}
