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
    DirectLaunch, ExitEvent, OutputEvent, SessionEvents, SessionSpawnedEvent, SessionUpdatedEvent,
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
    assert_eq!(
        session::get_row(&conn, &row.id)
            .unwrap()
            .unwrap()
            .title
            .as_deref(),
        Some("Manual name")
    );
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
        (15, "\x1b]0;| Airplane type\x07", "Airplane type", 4),
        (16, "\x1b]0;/ Airplane type\x07", "Airplane type", 4),
        (17, "\x1b]0;- Airplane type\x07", "Airplane type", 4),
        (18, "\x1b]0;\\ Airplane type\x07", "Airplane type", 4),
    ] {
        terminal.feed_output(&output(seq, data)).unwrap();
        flush_terminal_events(&terminal);
        assert_eq!(terminal.title(), expected);
        let stored = session::get_row(&conn, &row.id).unwrap().unwrap();
        assert_eq!(
            stored.live_title.as_deref(),
            (!expected.is_empty()).then_some(expected)
        );
        assert_eq!(stored.title, None);
        let count: usize = conn
            .query_row("SELECT COUNT(*) FROM title_writes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, writes);
    }
    terminal
        .feed_output(&output(19, "\x1b]0;Last topic\x07"))
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
fn replayed_or_progress_titles_keep_the_runner_name_until_a_new_title_arrives() {
    use crate::repo::session::{self, SessionRowDb};
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let conn = core.db.get().unwrap();
    for (id, runtime, cwd, steps) in [
        (
            "pi-fork",
            "pi",
            "/repos/runner",
            [
                ("\x1b]0;π - Cars - runner\x07", "Cars", true),
                ("\x1b]0;π - Cars - runner\x07", "Cars", true),
                ("\x1b]0;π - Trucks - runner\x07", "Trucks", false),
            ],
        ),
        (
            "codex-progress",
            "codex",
            "/Users/jason/repos/yicheng47",
            [
                ("\x1b]0;⠙ renaming... ⠙ | yicheng47\x07", "Cars", true),
                ("\x1b]0;Cars | yicheng47\x07", "Cars", true),
                ("\x1b]0;Trucks | yicheng47\x07", "Trucks", false),
            ],
        ),
    ] {
        let mut row = SessionRowDb::new_running(id.into());
        row.agent_runtime = Some(runtime.into());
        row.agent_command = Some(runtime.into());
        row.cwd = Some(cwd.into());
        row.title = Some("Cars (fork)".into());
        row.live_title = Some("Cars".into());
        session::insert(&conn, &row).unwrap();
        let terminal =
            TerminalSession::attach(core.clone(), row.id.clone(), 80, 24, Arc::new(|| {})).unwrap();
        for (seq, (data, live, named)) in steps.into_iter().enumerate() {
            terminal.feed_output(&output(seq as u64 + 1, data)).unwrap();
            flush_terminal_events(&terminal);
            assert_eq!(terminal.title(), live, "{id} {data:?}");
            let stored = session::get_row(&conn, &row.id).unwrap().unwrap();
            assert_eq!(stored.live_title.as_deref(), Some(live), "{id} {data:?}");
            assert_eq!(
                stored.title.as_deref(),
                named.then_some("Cars (fork)"),
                "{id} {data:?}"
            );
        }
    }
}

#[test]
fn pi_terminal_titles_ignore_subprocess_osc_and_persist_branded_topics() {
    use crate::repo::session::{self, SessionRowDb};
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let conn = core.db.get().unwrap();
    let mut row = SessionRowDb::new_running("replay-race".into());
    row.agent_runtime = Some("pi".into());
    row.agent_command = Some("pi".into());
    row.cwd = Some(r"C:\Users\jason\runner".into());
    session::insert(&conn, &row).unwrap();
    conn.execute_batch(
        "CREATE TABLE title_writes (title TEXT);
            CREATE TRIGGER record_title AFTER UPDATE OF live_title ON sessions
            BEGIN INSERT INTO title_writes VALUES (NEW.live_title); END;",
    )
    .unwrap();
    let terminal =
        TerminalSession::attach(core.clone(), row.id.clone(), 80, 24, Arc::new(|| {})).unwrap();
    assert_eq!(terminal.title(), "");

    for (seq, data, expected, writes) in [
        (1, "\x1b]0;npm view pi-hashline-edit-pro version\x07", "", 0),
        (
            2,
            "\x1b]2;C:\\Windows\\system32\\cmd.exe - npm view pi-hashline-edit-pro version\x07",
            "",
            0,
        ),
        (3, "\x1b]0;π - runner\x07", "", 0),
        (
            4,
            "\x1b]0;π - Fix login bug - runner\x07",
            "Fix login bug",
            1,
        ),
        (
            5,
            "\x1b]0;npm view pi-hashline-edit-pro version\x07",
            "Fix login bug",
            1,
        ),
        (6, "\x1b]0;π - runner\x07", "Fix login bug", 1),
        (
            7,
            "\x1b]2;pi - Renamed topic - runner\x1b\\",
            "Renamed topic",
            2,
        ),
    ] {
        terminal.feed_output(&output(seq, data)).unwrap();
        flush_terminal_events(&terminal);
        assert_eq!(terminal.title(), expected);
        let stored = session::get_row(&conn, &row.id).unwrap().unwrap();
        assert_eq!(
            stored.live_title.as_deref(),
            (!expected.is_empty()).then_some(expected)
        );
        let count: usize = conn
            .query_row("SELECT COUNT(*) FROM title_writes", [], |row| row.get(0))
            .unwrap();
        assert_eq!(count, writes);
    }

    let reopened = TerminalSession::attach(core, row.id, 80, 24, Arc::new(|| {})).unwrap();
    assert_eq!(reopened.title(), "Renamed topic");
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
        crate::test_support::isolated_shell_env(None),
    ));
    let runtime_discovery = Arc::new(std::sync::RwLock::new(
        crate::shell_path::DiscoveryState::startup(None, None),
    ));
    AppCore {
        remote: Default::default(),
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
        telemetry: Arc::new(crate::telemetry::TelemetryService::without_network()),
        claude_accounts: Arc::new(
            crate::runtimes::claude_code::accounts::AccountService::without_authentication(),
        ),
        buses: crate::event_bus::BusRegistry::new(),
        routers: crate::router::RouterRegistry::new(),
        mission_roster_locks: Default::default(),
        mission_grid_hint: Arc::new(std::sync::Mutex::new(None)),
        windows,
        events: crate::events::EventChannel::new(),
        files: Default::default(),
        session_event_observer: Default::default(),
        reports: Default::default(),
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

/// OSC 52 writes leave the daemon only as an in-memory `session/clipboard` event.
#[test]
fn osc52_writes_are_emitted_as_session_clipboard_events() {
    let temp = tempfile::tempdir().unwrap();
    let runtime = Arc::new(RecordingRuntime::default());
    let core = test_core_with_runtime(temp.path(), Arc::clone(&runtime) as _);
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
            DirectLaunch {
                cwd: Some(temp.path().to_str().unwrap().into()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
            &core.app_data_dir,
            Arc::clone(&core.db),
            Arc::new(core.session_events()),
        )
        .unwrap();
    let mut events = core.events.subscribe();
    runtime.push_output(b"\x1b]52;c;cnVubmVyLW9zYzUy\x07\x1b]52;p;cHJpbWFyeQ==\x1b\\");

    let mut received = Vec::new();
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    while received.len() < 2 && std::time::Instant::now() < deadline {
        match events.try_recv() {
            Ok(event) if event.name == "session/clipboard" => received.push(event.payload),
            Ok(_) => {}
            Err(_) => std::thread::sleep(std::time::Duration::from_millis(10)),
        }
    }
    assert_eq!(
        received,
        [
            serde_json::json!({"session_id": spawned.id, "target": "clipboard", "text": "runner-osc52"}),
            serde_json::json!({"session_id": spawned.id, "target": "selection", "text": "primary"}),
        ]
    );
    assert!(runtime.writes().is_empty(), "a write is not answered");
    core.sessions.kill(&spawned.id).ok();
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

    fn resize(&self, _: &RuntimeSession, _: u16, _: u16, _: u16, _: u16) -> RuntimeResult<()> {
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
            b"\x1b]11;rgb:f5f5/f1f1/ebeb\x1b\\".as_slice(),
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
                DirectLaunch {
                    cwd: Some(temp.path().to_str().unwrap().into()),
                    cols: Some(80),
                    rows: Some(24),
                    ..Default::default()
                },
                &core.app_data_dir,
                Arc::clone(&core.db),
                Arc::new(core.session_events()),
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

/// Sessions resumed at daemon start answer color queries before the app
/// reconnects, so the palette the app last sent outlives the daemon; the
/// app's first push still wins when it is the built-in default.
#[test]
fn terminal_palette_survives_a_daemon_restart() {
    use runner_terminal::palette::{ROSE_PINE_DAWN, RUNNER};
    let temp = tempfile::tempdir().unwrap();
    let paths = crate::daemon::boot::NativePaths::new(
        temp.path().join("app-data"),
        temp.path().join("logs"),
    );
    let before = crate::daemon::boot::boot(&paths, Vec::new(), false).unwrap();
    assert_eq!(*before.sessions.terminal_palette.lock().unwrap(), RUNNER);
    TerminalBridge::new(before.clone(), Arc::new(|| {}))
        .unwrap()
        .set_palette(ROSE_PINE_DAWN);
    drop(before);

    let after = crate::daemon::boot::boot(&paths, Vec::new(), false).unwrap();
    assert_eq!(
        *after.sessions.terminal_palette.lock().unwrap(),
        ROSE_PINE_DAWN
    );

    TerminalBridge::new(after.clone(), Arc::new(|| {}))
        .unwrap()
        .set_palette(RUNNER);
    assert_eq!(*after.sessions.terminal_palette.lock().unwrap(), RUNNER);
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
            DirectLaunch {
                cwd: Some(temp.path().to_str().unwrap().into()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
            &core.app_data_dir,
            Arc::clone(&core.db),
            Arc::new(core.session_events()),
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

    session.configure(3, CursorShape::Beam, true);
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
    session
        .send_key("5", false, true, false, Some("["))
        .unwrap();
    assert_eq!(
        wait_for(22)[21],
        if cfg!(target_os = "macos") {
            b"\x1b5"
        } else {
            b"\x1b["
        },
        "legacy Meta uses the key without Option on macOS"
    );
    runtime.push_output(b"\x1b[>1u\x1b[?u");
    assert_eq!(wait_for(23)[22], b"\x1b[?1u");
    session
        .send_key("5", false, true, false, Some("["))
        .unwrap();
    assert_eq!(wait_for(24)[23], b"\x1b[53;3u");
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
            DirectLaunch {
                cwd: Some(temp.path().to_str().unwrap().into()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
            &core.app_data_dir,
            Arc::clone(&core.db),
            Arc::new(core.session_events()),
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
        output_seq: 0,
    });
    events.output(&output(1, "old-child"));
    let old = bridge.session("replay-race").unwrap();
    assert_eq!(old.output_activity().first_paint_seq, 1);

    events.spawned(&SessionSpawnedEvent {
        session_id: "replay-race".into(),
        mission_id: None,
        cols: 100,
        rows: 30,
        output_seq: 0,
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
            output_seq: 0,
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

    terminal.configure(3, CursorShape::Beam, true);
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
    terminal.configure(100, CursorShape::Block, true);
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
    terminal.resize(8, 4, 0, 0);
    assert!(wait_until(Duration::from_secs(2), || terminal.size() == (8, 4)));
    assert!(terminal.selection_text().is_none());

    select();
    terminal
        .feed_output(&output(2, "\x1b[?1000h\x1b[?1006h"))
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
        terminal.resize(cols, rows, 0, 0);
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
                DirectLaunch {
                    cwd: Some(temp.path().to_str().unwrap().into()),
                    cols: Some(80),
                    rows: Some(24),
                    ..Default::default()
                },
                &core.app_data_dir,
                Arc::clone(&core.db),
                Arc::new(core.session_events()),
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
fn desktop_resize_keeps_the_mirror_at_the_browser_size() {
    use runner_terminal::terminal::TerminalMirror;
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal("mirror-size", (80, 24), &core.db, &events)
        .unwrap();
    let client = crate::daemon::InProcessTransport::client(core.clone());
    let mirror = TerminalMirror::attach(client, "mirror-size".into(), Arc::new(|| {})).unwrap();
    let model = core.sessions.terminal_model("mirror-size").unwrap();
    core.sessions
        .browser_resize(
            "mirror-size",
            1,
            "Chrome",
            37,
            29,
            0,
            0,
            true,
            false,
            &core.db,
        )
        .unwrap();
    assert!(wait_until(Duration::from_secs(2), || mirror.size() == (37, 29)));

    mirror.resize(96, 32, 0, 0);
    assert_eq!(mirror.size(), (37, 29));
    assert_eq!(model.size(), (37, 29));
    core.sessions
        .ingest_output_chunk("mirror-size", None, b"\x1b[29;35HabcDEF", events.as_ref());
    assert!(wait_until(Duration::from_secs(2), || mirror
        .output_activity()
        .last_seq
        == 2));
    assert_eq!(
        visible_lines(&mirror.term.lock()),
        visible_lines(&model.term.lock())
    );

    core.sessions.release_browser("mirror-size", 1);
    assert!(wait_until(Duration::from_secs(2), || mirror
        .output_activity()
        .last_seq
        == 3));
    mirror.resize(120, 35, 0, 0);
    assert_eq!(mirror.size(), (37, 29));
    mirror.reattach().unwrap();
    assert_eq!(mirror.size(), model.size());
}

#[test]
fn desktop_resize_updates_the_origin_mirror_in_frame_order() {
    use runner_terminal::terminal::TerminalMirror;
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal("desktop-size", (80, 24), &core.db, &events)
        .unwrap();
    let client = crate::daemon::InProcessTransport::client(core.clone());
    let mirror = TerminalMirror::attach(client, "desktop-size".into(), Arc::new(|| {})).unwrap();
    let model = core.sessions.terminal_model("desktop-size").unwrap();

    for (index, (cols, rows)) in [(96, 32), (40, 12), (120, 35)].into_iter().enumerate() {
        mirror.resize(cols, rows, 0, 0);
        core.sessions.ingest_output_chunk(
            "desktop-size",
            None,
            format!("\x1b[{rows};{}HabcDEF", cols - 2).as_bytes(),
            events.as_ref(),
        );
        assert!(wait_until(Duration::from_secs(2), || mirror
            .output_activity()
            .last_seq
            == (index as u64 + 1) * 2));
        assert_eq!(mirror.size(), (cols, rows));
        assert_eq!(
            visible_lines(&mirror.term.lock()),
            visible_lines(&model.term.lock())
        );
    }
}

#[test]
fn resize_is_broadcast_to_every_subscriber_including_its_origin() {
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
    client.resize("resize", origin.subscriber_id, 96, 32, 0, 0);
    for attachment in [&mut origin, &mut other] {
        assert!(matches!(
            attachment.frames.recv().unwrap(),
            TerminalFrame::Resized {
                seq: 1,
                cols: 96,
                rows: 32,
                ..
            }
        ));
    }
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

#[test]
fn browser_controller_follows_the_terminal_size_table() {
    use runner_core::protocol::terminal::TerminalController;
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal("control", (80, 24), &core.db, &events)
        .unwrap();
    let state = core.sessions.session_state("control").unwrap();
    assert!(matches!(
        state.lock().unwrap().controller,
        super::Controller::Desktop
    ));
    core.sessions.resize("control", 90, 30, &core.db).unwrap();
    assert_eq!(
        core.sessions.terminal_model("control").unwrap().size(),
        (90, 30)
    );
    core.sessions
        .browser_resize("control", 1, "First", 10, 2, 0, 0, true, false, &core.db)
        .unwrap();
    assert_eq!(
        core.sessions.terminal_model("control").unwrap().size(),
        (20, 8)
    );
    assert_eq!(
        core.sessions
            .attach_terminal("control")
            .unwrap()
            .snapshot
            .controller,
        TerminalController::Browser {
            channel: 1,
            name: "First".into()
        }
    );
    let seq = state.lock().unwrap().output_seq;
    core.sessions.resize("control", 100, 40, &core.db).unwrap();
    assert_eq!(state.lock().unwrap().output_seq, seq);
    assert_eq!(
        core.sessions.terminal_model("control").unwrap().size(),
        (20, 8)
    );
    core.sessions
        .browser_resize("control", 1, "First", 70, 20, 0, 0, false, true, &core.db)
        .unwrap();
    core.sessions
        .browser_resize("control", 2, "Second", 80, 25, 0, 0, false, true, &core.db)
        .unwrap();
    assert_eq!(
        core.sessions.terminal_model("control").unwrap().size(),
        (70, 20)
    );
    core.sessions
        .browser_resize("control", 2, "Second", 80, 25, 0, 0, true, false, &core.db)
        .unwrap();
    core.sessions.release_browser("control", 1);
    assert_eq!(
        core.sessions.browser_controller("control", 2),
        runner_core::companion::Controller::You
    );
    core.sessions
        .browser_resize("control", 1, "First", 50, 10, 0, 0, false, false, &core.db)
        .unwrap();
    assert_eq!(
        core.sessions.terminal_model("control").unwrap().size(),
        (80, 25)
    );
    core.sessions.release_browser_channel(2);
    assert_eq!(
        core.sessions.browser_controller("control", 1),
        runner_core::companion::Controller::None
    );
    assert_eq!(
        core.sessions.terminal_model("control").unwrap().size(),
        (80, 25)
    );
    core.sessions.resize("control", 120, 35, &core.db).unwrap();
    assert_eq!(
        core.sessions.terminal_model("control").unwrap().size(),
        (80, 25)
    );
    core.sessions
        .browser_resize("control", 1, "First", 50, 10, 0, 0, false, false, &core.db)
        .unwrap();
    assert_eq!(
        core.sessions.terminal_model("control").unwrap().size(),
        (50, 10)
    );
    core.sessions
        .take_back_terminal("control", 120, 35, 0, 0, &core.db)
        .unwrap();
    core.sessions
        .browser_resize("control", 2, "Second", 80, 25, 0, 0, false, false, &core.db)
        .unwrap();
    assert_eq!(
        core.sessions.terminal_model("control").unwrap().size(),
        (120, 35)
    );
    assert_eq!(
        core.sessions.browser_controller("control", 2),
        runner_core::companion::Controller::Desktop
    );
    core.sessions.release_browser_channel(1);
    core.sessions.resize("control", 125, 37, &core.db).unwrap();
    assert_eq!(
        core.sessions.terminal_model("control").unwrap().size(),
        (125, 37)
    );
}

#[test]
fn snapshot_history_limit_keeps_screen_modes_and_newest_history() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal("history", (30, 8), &core.db, &events)
        .unwrap();
    let model = core.sessions.terminal_model("history").unwrap();
    model.configure_named(100, "block");
    let bytes = (0..30)
        .map(|i| format!("line-{i:02}\r\n"))
        .collect::<String>();
    core.sessions
        .ingest_output_chunk("history", None, bytes.as_bytes(), events.as_ref());
    let full = core.sessions.attach_terminal("history").unwrap().snapshot;
    let limited = core
        .sessions
        .attach_terminal_with_history("history", 2)
        .unwrap()
        .snapshot;
    let screen = core
        .sessions
        .attach_terminal_with_history("history", 0)
        .unwrap()
        .snapshot;
    assert_eq!(limited.seq, full.seq);
    assert_eq!((limited.cols, limited.rows), (30, 8));
    assert!(full.bytes.windows(7).any(|bytes| bytes == b"line-00"));
    assert!(!limited.bytes.windows(7).any(|bytes| bytes == b"line-00"));
    assert!(limited.bytes.windows(7).any(|bytes| bytes == b"line-29"));
    assert!(screen.bytes.len() < limited.bytes.len());
    assert!(limited.bytes.len() < full.bytes.len());
    assert_eq!(model.snapshot(99).bytes, full.bytes);
}

#[test]
fn browser_viewports_clamp_to_the_approved_bounds_without_changing_desktop_sizes() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal("bounds", (80, 24), &core.db, &events)
        .unwrap();
    for (take, viewport) in [(true, false), (false, true)] {
        core.sessions
            .browser_resize(
                "bounds",
                1,
                "Browser",
                u16::MAX,
                u16::MAX,
                0,
                0,
                take,
                viewport,
                &core.db,
            )
            .unwrap();
        assert_eq!(
            core.sessions.terminal_model("bounds").unwrap().size(),
            (512, 256)
        );
        core.sessions
            .browser_resize("bounds", 1, "Browser", 1, 1, 0, 0, false, true, &core.db)
            .unwrap();
        assert_eq!(
            core.sessions.terminal_model("bounds").unwrap().size(),
            (20, 8)
        );
    }
    core.sessions
        .take_back_terminal("bounds", 10, 4, 0, 0, &core.db)
        .unwrap();
    assert_eq!(
        core.sessions.terminal_model("bounds").unwrap().size(),
        (10, 4)
    );
}

#[test]
fn terminal_close_reason_retains_an_intentional_stop_after_exit_consumes_the_flag() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal("stopped", (80, 24), &core.db, &events)
        .unwrap();
    core.sessions
        .session_state("stopped")
        .unwrap()
        .lock()
        .unwrap()
        .killed = true;
    assert!(core.sessions.take_killed("stopped"));
    assert!(core.sessions.terminal_was_stopped("stopped"));
    core.sessions
        .prepare_unlisted_terminal("stopped", (80, 24), &core.db, &events)
        .unwrap();
    assert!(!core.sessions.terminal_was_stopped("stopped"));
}

#[test]
fn browser_input_uses_the_terminal_queue_and_the_desktop_input_observation() {
    use crate::companion::core::{Core, DaemonCore};
    let temp = tempfile::tempdir().unwrap();
    let runtime = Arc::new(RecordingRuntime::default());
    let core = Arc::new(test_core_with_runtime(temp.path(), runtime as _));
    let role = {
        let conn = core.db.get().unwrap();
        crate::test_support::insert_test_role(&conn, "probe", "probe", "trae", "probe");
        crate::ops::role::get(&conn, "probe").unwrap()
    };
    let spawned = core
        .sessions
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(temp.path().to_str().unwrap().into()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
        .unwrap();
    let (input, queued) = std::sync::mpsc::channel();
    core.sessions
        .session_state(&spawned.id)
        .unwrap()
        .lock()
        .unwrap()
        .terminal_input = Some(super::TerminalInput::new(input));
    let model = core.sessions.terminal_model(&spawned.id).unwrap();
    let backend = DaemonCore(Arc::downgrade(&core));
    core.sessions
        .browser_resize(
            &spawned.id,
            1,
            "Client",
            80,
            24,
            0,
            0,
            true,
            false,
            &core.db,
        )
        .unwrap();
    model.publish_input(model.feed_output(1, "\x1b[2;1H❯ Try a task".as_bytes()));
    let before = model.input_reset_guard();
    let terminal = model.term.lock();
    let (start, started) = std::sync::mpsc::channel();
    let input_core = Arc::downgrade(&core);
    let session = spawned.id.clone();
    let input_thread = std::thread::spawn(move || {
        start.send(()).unwrap();
        DaemonCore(input_core).input(&session, 1, b"h").unwrap();
    });
    started.recv_timeout(Duration::from_secs(2)).unwrap();
    let premature = queued.recv_timeout(Duration::from_millis(250));
    drop(terminal);
    assert!(
        matches!(premature, Err(std::sync::mpsc::RecvTimeoutError::Timeout)),
        "input became available before its observation could read the pre-echo grid"
    );
    assert_eq!(queued.recv_timeout(Duration::from_secs(2)).unwrap(), b"h");
    assert!(model.input_reset_guard() > before);
    let observation = model
        .feed_output(2, "\r\x1b[K❯ h".as_bytes())
        .expect("the echo confirms the draft");
    assert_eq!(
        observation.state,
        runner_terminal::input_state::InputState::Drafting
    );
    core.sessions
        .session_state(&spawned.id)
        .unwrap()
        .lock()
        .unwrap()
        .model
        .apply(
            crate::session::state::SessionEvent::TestInputAt(Some(Instant::now())),
            crate::session::clock::state_now(),
        );
    model.publish_input(Some(observation));
    assert!(matches!(
        core.sessions.delivery_readiness(&spawned.id),
        crate::router::DeliveryReadiness::Drafting
    ));
    input_thread.join().unwrap();
    for bytes in [
        b"hello".as_slice(),
        b"\r",
        b"\x1b[200~line one\nline two\x1b[201~",
        b"\x1b[A",
    ] {
        let guard = model.input_reset_guard();
        backend.input(&spawned.id, 1, bytes).unwrap();
        assert_eq!(queued.recv_timeout(Duration::from_secs(2)).unwrap(), bytes);
        assert!(model.input_reset_guard() > guard);
    }
    core.sessions.kill(&spawned.id).unwrap();
}

#[test]
fn companion_snapshot_rep_continuation_fixtures_match_host() {
    use runner_core::companion::{HostMessage, Snapshot, TerminalBytes};
    for (name, unfinished, output) in [
        ("snapshot_rep", &b""[..], &b"\x1b[3b"[..]),
        ("snapshot_rep_unfinished", &b"\x1b[3"[..], &b"b"[..]),
    ] {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
        core.sessions
            .prepare_unlisted_terminal("rep", (20, 8), &core.db, &events)
            .unwrap();
        core.sessions
            .ingest_output_chunk("rep", None, b"abc", events.as_ref());
        if !unfinished.is_empty() {
            core.sessions
                .ingest_output_chunk("rep", None, unfinished, events.as_ref());
        }
        let snapshot = core
            .sessions
            .attach_terminal_with_history("rep", 0)
            .unwrap()
            .snapshot;
        assert_eq!(snapshot.preceding_char, Some('c'));
        assert_eq!(snapshot.unfinished_len, unfinished.len());
        let message = HostMessage::Snapshot(Snapshot {
            session: "rep".into(),
            seq: snapshot.seq,
            cols: snapshot.cols,
            rows: snapshot.rows,
            cell_width: snapshot.cell_width,
            cell_height: snapshot.cell_height,
            data: TerminalBytes(snapshot.bytes),
            more: false,
            preceding_char: snapshot.preceding_char,
            unfinished_len: snapshot.unfinished_len,
        });
        let fixture = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../runner-core/src/companion/fixtures")
            .join(format!("{name}.json"));
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(fixture).unwrap()).unwrap();
        assert_eq!(serde_json::to_value(message).unwrap(), expected);
        core.sessions
            .ingest_output_chunk("rep", None, output, events.as_ref());
        let model = core.sessions.terminal_model("rep").unwrap();
        assert_eq!(visible_lines(&model.term.lock())[0], "abcccc");
    }
}

async fn companion_next(
    adapter: &mut crate::companion::Adapter,
) -> runner_core::companion::HostMessage {
    tokio::time::timeout(Duration::from_secs(2), async {
        loop {
            if let Some(bytes) = adapter.next(true).await.unwrap() {
                return serde_json::from_slice(&bytes).unwrap();
            }
            if adapter.has_terminal_work() {
                adapter.changed().await.unwrap();
                continue;
            }
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap()
}

#[tokio::test]
async fn companion_backlog_sends_final_snapshot_after_exit_for_both_queue_limits() {
    use crate::companion::{
        core::{Core, DaemonCore},
        tests::{wait_for_buffered, with_output_held},
    };
    use runner_core::companion::*;
    for (subscriber_overflow, killed) in
        [(true, false), (false, false), (true, true), (false, true)]
    {
        let temp = tempfile::tempdir().unwrap();
        let runtime = Arc::new(RecordingRuntime::default());
        let core = Arc::new(test_core_with_runtime(temp.path(), runtime.clone()));
        let role = {
            let conn = core.db.get().unwrap();
            crate::test_support::insert_test_role(&conn, "probe", "probe", "trae", "probe");
            crate::ops::role::get(&conn, "probe").unwrap()
        };
        let spawned = core
            .sessions
            .spawn_direct(
                &role,
                DirectLaunch {
                    cwd: Some(temp.path().to_str().unwrap().into()),
                    cols: Some(80),
                    rows: Some(24),
                    ..Default::default()
                },
                &core.app_data_dir,
                core.db.clone(),
                Arc::new(core.session_events()),
            )
            .unwrap();
        let backend = Arc::new(DaemonCore(Arc::downgrade(&core)));
        let mut adapter =
            crate::companion::Adapter::new(backend.clone(), 77, "test".into()).unwrap();
        adapter
            .request(ClientMessage::Attach(AttachRequest {
                id: 1,
                session: spawned.id.clone(),
                cols: 80,
                rows: 24,
                cell_width: None,
                cell_height: None,
                control: ControlMode::Take,
                history: Some(0),
            }))
            .await
            .unwrap();
        assert!(matches!(
            companion_next(&mut adapter).await,
            HostMessage::Result(_)
        ));
        let HostMessage::Snapshot(initial) = companion_next(&mut adapter).await else {
            panic!()
        };
        assert!(matches!(
            companion_next(&mut adapter).await,
            HostMessage::Resized(_)
        ));
        core.sessions
            .ingest_output_chunk(&spawned.id, None, b"before", &core.session_events());
        let HostMessage::Output(output) = companion_next(&mut adapter).await else {
            panic!()
        };
        assert!(output.seq > initial.seq);
        let state = core.sessions.session_state(&spawned.id).unwrap();
        let source = state.lock().unwrap().companion_terminals[0].clone();
        let model = Arc::downgrade(&core.sessions.terminal_model(&spawned.id).unwrap());
        let burst = vec![b'x'; MAX_MESSAGE_SIZE + 1];
        let ingest = |chunk: &[u8]| {
            core.sessions
                .ingest_output_chunk(&spawned.id, None, chunk, &core.session_events())
        };
        if subscriber_overflow {
            with_output_held(&adapter, || {
                for chunk in burst.chunks(8 * 1024) {
                    ingest(chunk);
                }
            });
        } else {
            for (i, chunk) in burst.chunks(8 * 1024).enumerate() {
                ingest(chunk);
                wait_for_buffered(&adapter, i + 1).await;
            }
        }
        core.sessions
            .take_back_terminal(&spawned.id, 512, 100, 0, 0, &core.db)
            .unwrap();
        core.sessions.ingest_output_chunk(
            &spawned.id,
            None,
            b"\r\nFINAL-MARKER",
            &core.session_events(),
        );
        if killed {
            core.sessions.kill(&spawned.id).unwrap();
        } else {
            runtime
                .stop(&core.sessions.live_runtime_session(&spawned.id).unwrap())
                .unwrap();
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while core.sessions.session_live(&spawned.id) || (!killed && model.upgrade().is_some())
            {
                tokio::task::yield_now().await;
            }
        })
        .await
        .unwrap();
        assert!(backend.clone().attach(&spawned.id, 0, 1).is_err());
        assert!(state.lock().unwrap().companion_terminals.is_empty());
        wait_for_buffered(&adapter, usize::MAX).await;
        let mut final_bytes = Vec::new();
        let mut seq = None;
        loop {
            let HostMessage::Snapshot(snapshot) = companion_next(&mut adapter).await else {
                panic!("expected final snapshot")
            };
            assert_eq!((snapshot.cols, snapshot.rows), (512, 100));
            assert!(snapshot.seq > output.seq);
            assert_eq!(*seq.get_or_insert(snapshot.seq), snapshot.seq);
            final_bytes.extend_from_slice(&snapshot.data.0);
            if !snapshot.more {
                break;
            }
        }
        assert!(final_bytes.len() < MAX_MESSAGE_SIZE);
        assert!(final_bytes
            .windows(b"FINAL-MARKER".len())
            .any(|w| w == b"FINAL-MARKER"));
        let HostMessage::Resized(resized) = companion_next(&mut adapter).await else {
            panic!()
        };
        assert_eq!(resized.seq, seq.unwrap());
        assert_eq!((resized.cols, resized.rows), (512, 100));
        let HostMessage::Closed(closed) = companion_next(&mut adapter).await else {
            panic!()
        };
        assert_eq!(
            closed.reason,
            if killed {
                ClosedReason::Stopped
            } else {
                ClosedReason::Exited
            }
        );
        assert!(source.upgrade().is_none());
        assert!(adapter.next(true).await.unwrap().is_none());
    }
}

#[tokio::test]
async fn companion_final_snapshot_outlives_registry_without_retaining_the_terminal() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .prepare_unlisted_terminal("final", (20, 8), &core.db, &events)
        .unwrap();
    let (attachment, source) = core.sessions.attach_companion_terminal("final", 2).unwrap();
    core.sessions
        .detach_terminal("final", attachment.subscriber_id);
    drop(attachment);
    let model = Arc::downgrade(&core.sessions.terminal_model("final").unwrap());
    for _ in 0..30 {
        core.sessions
            .ingest_output_chunk("final", None, b"history\r\n", events.as_ref());
    }
    core.sessions
        .ingest_output_chunk("final", None, b"FINAL-MARKER", events.as_ref());
    let state = core.sessions.session_state("final").unwrap();
    state.lock().unwrap().finish_companion_terminals();
    core.sessions.discard_unlisted_terminal("final");
    drop(state);
    tokio::time::timeout(Duration::from_secs(2), async {
        while model.upgrade().is_some() {
            tokio::task::yield_now().await;
        }
    })
    .await
    .unwrap();
    let mut final_attachment = source.attach().unwrap();
    assert!(final_attachment.frames.recv().is_err());
    assert!(final_attachment
        .snapshot
        .bytes
        .windows(12)
        .any(|w| w == b"FINAL-MARKER"));
    assert!(source.attach().is_err(), "final state is consumed once");
    let mut term = runner_terminal::replay::new_term(20, 8);
    let mut parser: alacritty_terminal::vte::ansi::Processor =
        alacritty_terminal::vte::ansi::Processor::new();
    parser.advance(&mut term, &final_attachment.snapshot.bytes);
    assert!(term.grid().history_size() <= 2);
}

fn prepare_control_tab(
    core: &AppCore,
    ids: &[&str],
    drawer: &[&str],
) -> crate::repo::node::NodeRow {
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    for id in ids.iter().chain(drawer) {
        core.sessions
            .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
            .unwrap();
    }
    let layout = serde_json::json!({"slots": ids, "drawer": {"shells": drawer}}).to_string();
    crate::repo::node::create_tab(&core.db.get().unwrap(), None, "Split", 0, &layout).unwrap()
}

#[test]
fn split_tab_transfers_all_panes_and_drawer_without_sharing_viewport_sizes() {
    use runner_core::companion::Controller;
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    prepare_control_tab(&core, &["left", "right"], &["drawer"]);
    prepare_control_tab(&core, &["other"], &[]);
    let all = ["left", "right", "drawer"];
    core.sessions
        .browser_resize("left", 1, "First", 100, 30, 0, 0, true, false, &core.db)
        .unwrap();
    for id in all {
        assert_eq!(core.sessions.browser_controller(id, 1), Controller::You);
    }
    assert_eq!(
        core.sessions.browser_controller("other", 1),
        Controller::Desktop
    );
    assert_eq!(
        core.sessions.terminal_model("right").unwrap().size(),
        (80, 24)
    );
    core.sessions
        .browser_resize("right", 1, "First", 65, 40, 0, 0, false, true, &core.db)
        .unwrap();
    assert_eq!(
        core.sessions.terminal_model("left").unwrap().size(),
        (100, 30)
    );
    core.sessions
        .browser_resize("right", 2, "Second", 70, 35, 0, 0, true, false, &core.db)
        .unwrap();
    for id in all {
        assert_eq!(core.sessions.browser_controller(id, 2), Controller::You);
        assert!(core.sessions.browser_input(id, 1, b"stale").is_err());
        core.sessions
            .browser_resize(id, 1, "First", 200, 60, 0, 0, false, true, &core.db)
            .unwrap();
    }
    assert_eq!(
        core.sessions.terminal_model("left").unwrap().size(),
        (100, 30)
    );
    assert_eq!(
        core.sessions.terminal_model("right").unwrap().size(),
        (70, 35)
    );
    core.sessions
        .take_back_terminal("left", 110, 42, 0, 0, &core.db)
        .unwrap();
    for id in all {
        assert_eq!(core.sessions.browser_controller(id, 2), Controller::Desktop);
        assert!(core.sessions.browser_input(id, 2, b"stale").is_err());
    }
    assert_eq!(
        core.sessions.terminal_model("right").unwrap().size(),
        (70, 35)
    );
    core.sessions.resize("right", 55, 28, &core.db).unwrap();
    assert_eq!(
        core.sessions.terminal_model("right").unwrap().size(),
        (55, 28)
    );
    core.sessions.release_browser_channel(2);
    for id in all {
        assert_eq!(core.sessions.browser_controller(id, 2), Controller::Desktop);
    }
    core.sessions.drain_terminal_workers();
}

#[test]
fn split_tab_detach_tracks_group_streams_and_ignores_stale_owner_cleanup() {
    use runner_core::companion::Controller;
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let mut tab = prepare_control_tab(&core, &["left", "right"], &["drawer"]);
    prepare_control_tab(&core, &["other"], &[]);
    for (id, subscriber) in [("left", 1), ("right", 2), ("drawer", 3), ("other", 4)] {
        core.sessions
            .browser_attached(id, subscriber, 1, &core.db)
            .unwrap();
    }
    core.sessions
        .browser_resize("left", 1, "First", 100, 30, 0, 0, true, false, &core.db)
        .unwrap();
    core.sessions
        .browser_resize("other", 1, "First", 90, 25, 0, 0, true, false, &core.db)
        .unwrap();
    core.sessions
        .browser_attached("drawer", 5, 1, &core.db)
        .unwrap();
    core.sessions
        .browser_detached("drawer", 3, 1, true, &core.db)
        .unwrap();
    assert_eq!(core.sessions.browser_controller("left", 1), Controller::You);
    tab.layout =
        Some(serde_json::json!({"slots": ["right"], "drawer": {"shells": ["drawer"]}}).to_string());
    crate::repo::node::upsert(&core.db.get().unwrap(), &tab).unwrap();
    core.sessions
        .browser_detached("left", 1, 1, true, &core.db)
        .unwrap();
    assert_eq!(
        core.sessions.browser_controller("left", 1),
        Controller::None
    );
    assert_eq!(
        core.sessions.browser_controller("right", 1),
        Controller::You
    );
    core.sessions
        .session_state("right")
        .unwrap()
        .lock()
        .unwrap()
        .terminal_input
        .take();
    core.sessions
        .browser_detached("right", 2, 1, true, &core.db)
        .unwrap();
    assert_eq!(
        core.sessions.browser_controller("drawer", 1),
        Controller::You
    );
    core.sessions
        .browser_detached("drawer", 5, 1, true, &core.db)
        .unwrap();
    assert_eq!(
        core.sessions.browser_controller("drawer", 1),
        Controller::None
    );
    assert_eq!(
        core.sessions.browser_controller("other", 1),
        Controller::You
    );
    core.sessions
        .browser_resize("drawer", 2, "Second", 70, 35, 0, 0, true, false, &core.db)
        .unwrap();
    core.sessions
        .browser_attached("drawer", 6, 2, &core.db)
        .unwrap();
    core.sessions
        .browser_detached("drawer", 5, 1, true, &core.db)
        .unwrap();
    core.sessions.release_browser_channel(1);
    assert_eq!(
        core.sessions.browser_controller("drawer", 2),
        Controller::You
    );
    assert_eq!(
        core.sessions.browser_controller("other", 2),
        Controller::None
    );
    core.sessions
        .browser_detached("drawer", 6, 2, true, &core.db)
        .unwrap();
    assert_eq!(
        core.sessions.browser_controller("drawer", 2),
        Controller::None
    );
    core.sessions.drain_terminal_workers();
}

#[test]
fn restarted_and_new_split_panes_inherit_the_destination_tab_controller() {
    use runner_core::companion::Controller;
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let tab = prepare_control_tab(&core, &["left", "right"], &[]);
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    core.sessions
        .browser_resize("left", 1, "First", 100, 30, 0, 0, true, false, &core.db)
        .unwrap();
    let old = core.sessions.attach_terminal("right").unwrap();
    core.sessions
        .browser_attached("right", old.subscriber_id, 1, &core.db)
        .unwrap();
    core.sessions
        .prepare_unlisted_terminal("right", (60, 20), &core.db, &events)
        .unwrap();
    assert_eq!(
        core.sessions.browser_controller("right", 1),
        Controller::You
    );
    let replacement = core.sessions.attach_terminal("right").unwrap();
    assert_ne!(old.subscriber_id, replacement.subscriber_id);
    core.sessions
        .browser_attached("right", replacement.subscriber_id, 1, &core.db)
        .unwrap();
    core.sessions
        .browser_detached("right", old.subscriber_id, 1, true, &core.db)
        .unwrap();
    assert_eq!(core.sessions.browser_controller("left", 1), Controller::You);
    assert_eq!(
        core.sessions.browser_controller("right", 1),
        Controller::You
    );
    core.sessions
        .prepare_unlisted_terminal("new", (50, 15), &core.db, &events)
        .unwrap();
    let layout = serde_json::json!({"slots": ["left", "right", "new"]}).to_string();
    crate::ops::node::node_tab_upsert(
        &core,
        crate::ops::node::NodeTabUpsertInput {
            id: tab.id,
            parent_id: None,
            name: "Split".into(),
            layout,
        },
    )
    .unwrap();
    assert_eq!(core.sessions.browser_controller("new", 1), Controller::You);
    assert_eq!(
        core.sessions.terminal_model("new").unwrap().size(),
        (50, 15)
    );
    core.sessions
        .browser_resize("left", 2, "Second", 80, 24, 0, 0, true, false, &core.db)
        .unwrap();
    core.sessions
        .browser_attached("right", replacement.subscriber_id, 2, &core.db)
        .unwrap();
    core.sessions
        .browser_detached("right", replacement.subscriber_id, 1, true, &core.db)
        .unwrap();
    for id in ["left", "right", "new"] {
        assert_eq!(core.sessions.browser_controller(id, 2), Controller::You);
        assert!(core
            .sessions
            .browser_input(id, 1, b"old generation")
            .is_err());
    }
    core.sessions.release_browser_channel(2);
    core.sessions
        .prepare_unlisted_terminal("right", (65, 25), &core.db, &events)
        .unwrap();
    assert_eq!(
        core.sessions.browser_controller("right", 2),
        Controller::None
    );
    core.sessions
        .take_back_terminal("left", 110, 42, 0, 0, &core.db)
        .unwrap();
    core.sessions
        .prepare_unlisted_terminal("right", (65, 25), &core.db, &events)
        .unwrap();
    assert_eq!(
        core.sessions.browser_controller("right", 2),
        Controller::Desktop
    );
    core.sessions.drain_terminal_workers();
}

#[test]
fn moving_a_pane_adopts_its_destination_without_old_tab_teardown_releasing_it() {
    use runner_core::companion::Controller;
    for destination_channel in [1, 2] {
        let temp = tempfile::tempdir().unwrap();
        let core = test_core(temp.path());
        let source = prepare_control_tab(&core, &["left", "moving"], &[]);
        let destination = prepare_control_tab(&core, &["other"], &[]);
        core.sessions
            .browser_resize("left", 1, "Source", 100, 30, 0, 0, true, false, &core.db)
            .unwrap();
        core.sessions
            .browser_resize(
                "other",
                destination_channel,
                "Destination",
                70,
                25,
                0,
                0,
                true,
                false,
                &core.db,
            )
            .unwrap();
        for (id, subscriber, channel) in [
            ("left", 1, 1),
            ("moving", 2, 1),
            ("other", 3, destination_channel),
        ] {
            core.sessions
                .browser_attached(id, subscriber, channel, &core.db)
                .unwrap();
        }
        crate::ops::node::node_tab_upsert(
            &core,
            crate::ops::node::NodeTabUpsertInput {
                id: destination.id,
                parent_id: None,
                name: "Destination".into(),
                layout: serde_json::json!({"slots": ["other", "moving"]}).to_string(),
            },
        )
        .unwrap();
        assert_eq!(
            core.sessions
                .browser_controller("moving", destination_channel),
            Controller::You
        );
        assert_eq!(core.sessions.browser_controller("left", 1), Controller::You);
        assert_eq!(
            crate::repo::node::session_ids(
                &crate::repo::node::get(&core.db.get().unwrap(), &source.id)
                    .unwrap()
                    .unwrap()
            ),
            ["left"]
        );
        crate::ops::node::node_tab_upsert(
            &core,
            crate::ops::node::NodeTabUpsertInput {
                id: source.id.clone(),
                parent_id: None,
                name: "Empty source".into(),
                layout: serde_json::json!({"slots": []}).to_string(),
            },
        )
        .unwrap();
        crate::ops::node::node_tab_delete(&core, &source.id).unwrap();
        core.sessions
            .browser_detached("moving", 2, 1, true, &core.db)
            .unwrap();
        assert_eq!(
            core.sessions
                .browser_controller("moving", destination_channel),
            Controller::You
        );
        core.sessions
            .browser_detached("left", 1, 1, true, &core.db)
            .unwrap();
        assert_eq!(
            core.sessions.browser_controller("left", 1),
            Controller::None
        );
        assert_eq!(
            core.sessions
                .browser_controller("other", destination_channel),
            Controller::You
        );
        assert_eq!(
            core.sessions
                .browser_controller("moving", destination_channel),
            Controller::You
        );
        core.sessions
            .browser_detached("other", 3, destination_channel, true, &core.db)
            .unwrap();
        assert_eq!(
            core.sessions
                .browser_controller("moving", destination_channel),
            Controller::None
        );
        core.sessions.drain_terminal_workers();
    }
}

#[test]
fn split_tab_if_free_claims_and_resizes_free_or_same_channel_panes_without_stealing() {
    use runner_core::companion::Controller;
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    prepare_control_tab(&core, &["left", "right"], &[]);
    core.sessions
        .browser_resize("left", 1, "First", 90, 28, 0, 0, true, false, &core.db)
        .unwrap();
    core.sessions.release_browser_channel(1);
    assert_eq!(
        core.sessions.browser_controller("left", 2),
        Controller::None
    );
    assert_eq!(
        core.sessions.browser_controller("right", 2),
        Controller::None
    );

    // Reconnecting with IfFree claims the free split tab and resizes every pane.
    core.sessions
        .browser_resize(
            "left",
            2,
            "Reconnected",
            104,
            31,
            0,
            0,
            false,
            false,
            &core.db,
        )
        .unwrap();
    core.sessions
        .browser_resize(
            "right",
            2,
            "Reconnected",
            72,
            39,
            0,
            0,
            false,
            false,
            &core.db,
        )
        .unwrap();
    assert_eq!(core.sessions.browser_controller("left", 2), Controller::You);
    assert_eq!(
        core.sessions.browser_controller("right", 2),
        Controller::You
    );
    assert_eq!(
        core.sessions.terminal_model("left").unwrap().size(),
        (104, 31)
    );
    assert_eq!(
        core.sessions.terminal_model("right").unwrap().size(),
        (72, 39)
    );

    // If one pane is None while a sibling is already owned by this same channel, IfFree claims it
    // instead of leaving it uncontrolled, while another channel or Desktop is never stolen from.
    core.sessions.release_browser("right", 2);
    assert_eq!(
        core.sessions.browser_controller("right", 2),
        Controller::None
    );
    core.sessions
        .browser_resize("right", 3, "Other", 60, 20, 0, 0, false, false, &core.db)
        .unwrap();
    assert_eq!(
        core.sessions.browser_controller("right", 2),
        Controller::None
    );
    assert_eq!(core.sessions.browser_controller("left", 2), Controller::You);

    core.sessions
        .browser_resize(
            "right",
            2,
            "Reconnected",
            76,
            41,
            0,
            0,
            false,
            false,
            &core.db,
        )
        .unwrap();
    assert_eq!(
        core.sessions.browser_controller("right", 2),
        Controller::You
    );
    assert_eq!(
        core.sessions.terminal_model("right").unwrap().size(),
        (76, 41)
    );

    core.sessions
        .take_back_terminal("left", 110, 42, 0, 0, &core.db)
        .unwrap();
    core.sessions
        .browser_resize(
            "right",
            2,
            "Reconnected",
            60,
            20,
            0,
            0,
            false,
            false,
            &core.db,
        )
        .unwrap();
    assert_eq!(
        core.sessions.browser_controller("left", 2),
        Controller::Desktop
    );
    assert_eq!(
        core.sessions.browser_controller("right", 2),
        Controller::Desktop
    );

    // Passive split growth: when channel 2 owns the tab, adding a new pane inherits channel 2 and
    // IfFree resizes it; when Desktop owns the tab, adding a new pane inherits Desktop and IfFree
    // preserves Desktop ownership and size.
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let tab_id = crate::repo::node::find_for_session(&core.db.get().unwrap(), "left")
        .unwrap()
        .unwrap()
        .id;
    core.sessions
        .prepare_unlisted_terminal("split-client-owned", (80, 24), &core.db, &events)
        .unwrap();
    core.sessions
        .browser_resize(
            "left",
            2,
            "Reconnected",
            104,
            31,
            0,
            0,
            true,
            false,
            &core.db,
        )
        .unwrap();
    crate::ops::node::node_tab_upsert(
        &core,
        runner_core::protocol::node::NodeTabUpsertInput {
            id: tab_id.clone(),
            parent_id: None,
            name: "Split".into(),
            layout: serde_json::json!({ "slots": ["left", "right", "split-client-owned"] })
                .to_string(),
        },
    )
    .unwrap();
    core.sessions
        .browser_resize(
            "split-client-owned",
            2,
            "Reconnected",
            88,
            34,
            0,
            0,
            false,
            false,
            &core.db,
        )
        .unwrap();
    assert_eq!(
        core.sessions.browser_controller("split-client-owned", 2),
        Controller::You
    );
    assert_eq!(
        core.sessions
            .terminal_model("split-client-owned")
            .unwrap()
            .size(),
        (88, 34)
    );

    core.sessions
        .take_back_terminal("left", 112, 40, 0, 0, &core.db)
        .unwrap();
    core.sessions
        .prepare_unlisted_terminal("split-desktop-owned", (96, 28), &core.db, &events)
        .unwrap();
    crate::ops::node::node_tab_upsert(
        &core,
        runner_core::protocol::node::NodeTabUpsertInput {
            id: tab_id,
            parent_id: None,
            name: "Split".into(),
            layout: serde_json::json!({
                "slots": ["left", "right", "split-client-owned", "split-desktop-owned"]
            })
            .to_string(),
        },
    )
    .unwrap();
    core.sessions
        .browser_resize(
            "split-desktop-owned",
            2,
            "Reconnected",
            64,
            22,
            0,
            0,
            false,
            false,
            &core.db,
        )
        .unwrap();
    assert_eq!(
        core.sessions.browser_controller("left", 2),
        Controller::Desktop
    );
    assert_eq!(
        core.sessions.browser_controller("split-desktop-owned", 2),
        Controller::Desktop
    );
    assert_eq!(
        core.sessions
            .terminal_model("split-desktop-owned")
            .unwrap()
            .size(),
        (96, 28)
    );
    core.sessions.drain_terminal_workers();
}

#[test]
fn release_browser_channel_touches_only_sessions_owned_by_that_channel_1377() {
    let temp = tempfile::tempdir().unwrap();
    let core = test_core(temp.path());
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    for id in ["wedged", "ch1-owned", "ch2-owned"] {
        core.sessions
            .prepare_unlisted_terminal(id, (80, 24), &core.db, &events)
            .unwrap();
    }
    core.sessions
        .browser_resize("ch1-owned", 1, "Phone", 80, 24, 0, 0, true, false, &core.db)
        .unwrap();
    core.sessions
        .browser_resize(
            "ch2-owned",
            2,
            "Tablet",
            80,
            24,
            0,
            0,
            true,
            false,
            &core.db,
        )
        .unwrap();

    // Hold `wedged`'s SessionState lock on another thread for the entire channel release check.
    let wedged_state = core.sessions.session_state("wedged").unwrap();
    let (locked_tx, locked_rx) = std::sync::mpsc::channel();
    let (unlock_tx, unlock_rx) = std::sync::mpsc::channel();
    let holder = std::thread::spawn(move || {
        let _guard = wedged_state.lock().unwrap();
        locked_tx.send(()).unwrap();
        let _ = unlock_rx.recv();
    });
    locked_rx.recv().unwrap();

    let (done_tx, done_rx) = std::sync::mpsc::channel();
    let sessions = core.sessions.clone();
    std::thread::spawn(move || {
        sessions.release_browser_channel(99);
        sessions.release_browser_channel(1);
        let _ = done_tx.send(());
    });
    done_rx
        .recv_timeout(std::time::Duration::from_secs(5))
        .expect("release_browser_channel must not lock unrelated wedged SessionState");

    assert_eq!(
        core.sessions.browser_controller("ch1-owned", 1),
        runner_core::companion::Controller::None
    );
    assert_eq!(
        core.sessions.browser_controller("ch2-owned", 2),
        runner_core::companion::Controller::You
    );

    let _ = unlock_tx.send(());
    holder.join().unwrap();
    core.sessions.drain_terminal_workers();
}

#[test]
fn daemon_global_image_budget_enforces_limit_across_concurrent_sessions_without_cross_eviction() {
    use crate::session::SessionManager;
    use base64::Engine as _;
    use runner_terminal::kitty::{GlobalImageBudget, DEFAULT_GLOBAL_BUDGET};
    use std::collections::HashMap;
    use std::sync::{mpsc, Barrier, Condvar, Mutex};

    let default_temp = tempfile::tempdir().unwrap();
    let default_core = test_core(default_temp.path());
    assert_eq!(
        default_core.sessions.image_budget().limit(),
        DEFAULT_GLOBAL_BUDGET
    );
    assert_eq!(DEFAULT_GLOBAL_BUDGET, 1024 * 1024 * 1024);

    #[derive(Default)]
    struct MultiSessionRuntime {
        outputs: Mutex<HashMap<String, mpsc::Sender<RuntimeOutput>>>,
        writes: Mutex<HashMap<String, Vec<Vec<u8>>>>,
        writes_cv: Condvar,
    }
    impl MultiSessionRuntime {
        fn push_output(&self, session_id: &str, bytes: &[u8]) {
            self.outputs
                .lock()
                .unwrap()
                .get(session_id)
                .expect("spawned")
                .send(RuntimeOutput::Stream(bytes.to_vec()))
                .unwrap();
        }
        fn wait_writes_len(&self, session_id: &str, expected: usize) -> Vec<Vec<u8>> {
            let mut guard = self.writes.lock().unwrap();
            let deadline = Instant::now() + Duration::from_secs(10);
            while guard.get(session_id).map_or(0, Vec::len) < expected {
                let rem = deadline
                    .checked_duration_since(Instant::now())
                    .expect("timed out waiting for PTY replies");
                let (next, _) = self.writes_cv.wait_timeout(guard, rem).unwrap();
                guard = next;
            }
            guard.get(session_id).cloned().unwrap_or_default()
        }
        fn take_writes(&self, session_id: &str) -> Vec<Vec<u8>> {
            self.writes
                .lock()
                .unwrap()
                .remove(session_id)
                .unwrap_or_default()
        }
    }
    impl SessionRuntime for MultiSessionRuntime {
        fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
            let (tx, rx) = mpsc::channel();
            self.outputs
                .lock()
                .unwrap()
                .insert(spec.session_id.clone(), tx);
            Ok((
                RuntimeSession {
                    runtime: "multi-kitty".into(),
                    session_id: spec.session_id,
                },
                OutputStream::new(rx, Arc::new(AtomicBool::new(false))),
            ))
        }
        fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
            self.outputs.lock().unwrap().remove(&session.session_id);
            Ok(())
        }
        fn send_bytes(&self, session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()> {
            self.writes
                .lock()
                .unwrap()
                .entry(session.session_id.clone())
                .or_default()
                .push(bytes.to_vec());
            self.writes_cv.notify_all();
            Ok(())
        }
        fn send_key(&self, _: &RuntimeSession, _: &str) -> RuntimeResult<()> {
            Ok(())
        }
        fn resize(&self, _: &RuntimeSession, _: u16, _: u16, _: u16, _: u16) -> RuntimeResult<()> {
            Ok(())
        }
        fn status(&self, _: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
            Ok(Some(SessionStatus {
                alive: true,
                ..Default::default()
            }))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let runtime = Arc::new(MultiSessionRuntime::default());
    let limit = 4 * 1024 * 1024; // 4 MiB shared daemon cap for fast deterministic stress runs
    let budget = Arc::new(GlobalImageBudget::new(limit));
    let mut core = test_core_with_runtime(temp.path(), Arc::clone(&runtime) as _);
    core.sessions = SessionManager::new_with_image_budget(
        Arc::clone(&core.runtime_shell_env),
        Arc::clone(&core.runtime_discovery),
        Arc::clone(&runtime) as _,
        Arc::clone(&budget),
    );
    core.sessions.set_runtime_events(core.events.clone());

    let role = {
        let conn = core.db.get().unwrap();
        crate::test_support::insert_test_role(&conn, "budget", "budget", "trae", "budget");
        crate::ops::role::get(&conn, "budget").unwrap()
    };
    let events: Arc<dyn SessionEvents> = Arc::new(core.session_events());
    let spawn_one = || {
        core.sessions
            .spawn_direct(
                &role,
                DirectLaunch {
                    cwd: Some(temp.path().to_str().unwrap().into()),
                    cols: Some(80),
                    rows: Some(24),
                    ..Default::default()
                },
                &core.app_data_dir,
                core.db.clone(),
                Arc::clone(&events),
            )
            .unwrap()
            .id
    };
    let a = spawn_one();
    let b = spawn_one();
    let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);

    // 1. Place a 500 KiB image on A's alt screen, then return to primary.
    // Primary overflow must never evict A's inactive alt screen.
    let img_500k = b64(&vec![1u8; 400 * 425 * 3]); // 510,000 B
    runtime.push_output(
        &a,
        format!("\x1b[?1049h\x1b_Ga=t,i=900,f=24,s=400,v=425;{img_500k}\x1b\\\x1b[?1049l")
            .as_bytes(),
    );
    let _ = runtime.wait_writes_len(&a, 1);
    runtime.take_writes(&a);
    assert!(core
        .sessions
        .terminal_model(&a)
        .unwrap()
        .graphics()
        .lock()
        .unwrap()
        .alt
        .images()
        .contains_key(&900));

    // 2. Stream images concurrently on A and B past the shared 4 MiB limit.
    let mut a_sub = core.sessions.attach_terminal(&a).unwrap();
    let img_600k = b64(&vec![2u8; 500 * 400 * 3]); // 600,000 B
    let start = Arc::new(Barrier::new(2));
    let workers: Vec<_> = [(a.clone(), 1u32), (b.clone(), 100u32)]
        .into_iter()
        .map(|(sid, base_id)| {
            let rt = Arc::clone(&runtime);
            let bgt = Arc::clone(&budget);
            let payload = img_600k.clone();
            let bar = Arc::clone(&start);
            std::thread::spawn(move || {
                bar.wait();
                for idx in 0..8u32 {
                    let img_id = base_id + idx;
                    rt.push_output(
                        &sid,
                        format!("\x1b_Ga=t,i={img_id},f=24,s=500,v=400;{payload}\x1b\\").as_bytes(),
                    );
                    let _ = rt.wait_writes_len(&sid, (idx + 1) as usize);
                    assert!(
                        bgt.used() <= limit,
                        "global budget exceeded: {} > {}",
                        bgt.used(),
                        limit
                    );
                }
            })
        })
        .collect();
    for worker in workers {
        worker.join().unwrap();
    }
    runtime.take_writes(&a);
    runtime.take_writes(&b);
    assert!(budget.used() <= limit);
    assert!(
        core.sessions
            .terminal_model(&a)
            .unwrap()
            .graphics()
            .lock()
            .unwrap()
            .alt
            .images()
            .contains_key(&900),
        "overflow on primary must never evict inactive alt screen"
    );

    // 3. Clear both primary screens (ED 2 frees all unplaced images), fill A to 3.51 MiB, put one
    // 200 KiB image on B, then ask B for a 1.2 MiB image that its active screen cannot cover ->
    // ENOMEM & B keeps its image. Also verify eviction on A emits `a=d,d=I,i=<id>,q=2` to mirrors.
    runtime.push_output(&a, b"\x1b[2J\x1b_Ga=q,i=990,f=24,s=1,v=1;AAAA\x1b\\");
    runtime.push_output(&b, b"\x1b[2J\x1b_Ga=q,i=991,f=24,s=1,v=1;AAAA\x1b\\");
    let _ = runtime.wait_writes_len(&a, 1);
    let _ = runtime.wait_writes_len(&b, 1);
    runtime.take_writes(&a);
    runtime.take_writes(&b);
    assert_eq!(budget.used(), 510_000);

    for i in 1..=7u32 {
        runtime.push_output(
            &a,
            format!("\x1b_Ga=t,i={i},f=24,s=500,v=400;{img_600k}\x1b\\").as_bytes(),
        );
    }
    let a_fill = runtime.wait_writes_len(&a, 7);
    assert!(
        a_fill.iter().all(|reply| reply.ends_with(b";OK\x1b\\")),
        "expected all 7 A images to fit by evicting A's own oldest primary image: {a_fill:?}"
    );
    runtime.take_writes(&a);
    // Drain A's subscriber and confirm eviction reached it as `a=d,d=I,i=<id>,q=2`.
    runtime.push_output(&a, b"eviction-check-sentinel");
    let mut saw_eviction_delete = false;
    loop {
        match a_sub.frames.recv().unwrap() {
            runner_core::protocol::terminal::TerminalFrame::Output { bytes, .. } => {
                if bytes == b"eviction-check-sentinel" {
                    break;
                }
                if String::from_utf8_lossy(&bytes).contains("\x1b_Ga=d,d=I,i=") {
                    saw_eviction_delete = true;
                }
            }
            runner_core::protocol::terminal::TerminalFrame::Resync => {
                break;
            }
            _ => {}
        }
    }
    assert!(
        saw_eviction_delete,
        "evicting an image must emit a=d,d=I,i=<id>,q=2 to mirrors"
    );
    drop(a_sub);
    // A now holds 510 KiB (alt) + 6 * 600 KiB (i=2..=7) = 4,110,000 B.
    // Delete i=7 to leave 3,510,000 B (~684 KiB free).
    runtime.push_output(
        &a,
        b"\x1b_Ga=d,d=I,i=7,q=2\x1b\\\x1b_Ga=q,i=992,f=24,s=1,v=1;AAAA\x1b\\",
    );
    let _ = runtime.wait_writes_len(&a, 1);
    runtime.take_writes(&a);
    assert_eq!(budget.used(), 510_000 + 5 * 600_000);
    let img_200k = b64(&vec![3u8; 256 * 260 * 3]); // 199,680 B
    runtime.push_output(
        &b,
        format!("\x1b_Ga=t,i=50,f=24,s=256,v=260;{img_200k}\x1b\\").as_bytes(),
    );
    let b_ok = runtime.wait_writes_len(&b, 1);
    assert_eq!(b_ok, vec![b"\x1b_Gi=50;OK\x1b\\".to_vec()]);
    runtime.take_writes(&b);

    let img_1_2m = b64(&vec![4u8; 640 * 640 * 3]); // 1,228,800 B (> free + B's 199,680 B)
    runtime.push_output(
        &b,
        format!("\x1b_Ga=t,i=51,f=24,s=640,v=640;{img_1_2m}\x1b\\").as_bytes(),
    );
    let b_oom = runtime.wait_writes_len(&b, 1);
    assert_eq!(
        b_oom,
        vec![b"\x1b_Gi=51;ENOMEM: out of memory\x1b\\".to_vec()]
    );
    runtime.take_writes(&b);
    assert!(
        core.sessions
            .terminal_model(&b)
            .unwrap()
            .graphics()
            .lock()
            .unwrap()
            .primary
            .images()
            .contains_key(&50),
        "ENOMEM must preserve the session's existing images"
    );
    assert!(!core
        .sessions
        .terminal_model(&b)
        .unwrap()
        .graphics()
        .lock()
        .unwrap()
        .primary
        .images()
        .contains_key(&51));

    // 4. Ordinary session_stop frees A's primary + alt images immediately (without forgetting
    // session state) so B's 1.2 MiB image now succeeds while B stays live.
    crate::ops::session::session_stop(&core, &a).unwrap();
    assert_eq!(
        budget.used(),
        199_680,
        "ordinary session_stop must release both primary and alt images immediately"
    );

    runtime.push_output(
        &b,
        format!("\x1b_Ga=t,i=52,f=24,s=640,v=640;{img_1_2m}\x1b\\").as_bytes(),
    );
    let b_retry = runtime.wait_writes_len(&b, 1);
    assert_eq!(b_retry, vec![b"\x1b_Gi=52;OK\x1b\\".to_vec()]);

    crate::ops::session::session_stop(&core, &b).unwrap();
    assert_eq!(
        budget.used(),
        0,
        "stopping all sessions must free budget to 0 without forgetting session state"
    );
    core.sessions.drain_terminal_workers();
}

#[tokio::test]
async fn companion_filters_30mib_graphics_omits_snapshot_graphics_and_carries_cell_size() {
    use crate::companion::core::DaemonCore;
    use base64::Engine as _;
    use runner_core::companion::*;
    use runner_terminal::kitty::CellSize;

    let temp = tempfile::tempdir().unwrap();
    let runtime = Arc::new(RecordingRuntime::default());
    let core = Arc::new(test_core_with_runtime(
        temp.path(),
        Arc::clone(&runtime) as _,
    ));
    let role = {
        let conn = core.db.get().unwrap();
        crate::test_support::insert_test_role(
            &conn,
            "comp-kitty",
            "comp-kitty",
            "trae",
            "comp-kitty",
        );
        crate::ops::role::get(&conn, "comp-kitty").unwrap()
    };
    let spawned = core
        .sessions
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(temp.path().to_str().unwrap().into()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
        .unwrap();
    let id = spawned.id.clone();

    // Attach a local subscriber and a companion Adapter (carrying cellWidth/cellHeight).
    let mut local_sub = core.sessions.attach_terminal(&id).unwrap();
    let backend = Arc::new(DaemonCore(Arc::downgrade(&core)));
    let mut adapter = crate::companion::Adapter::new(backend, 88, "iPad".into()).unwrap();
    adapter
        .request(ClientMessage::Attach(AttachRequest {
            id: 1,
            session: id.clone(),
            cols: 90,
            rows: 30,
            cell_width: Some(16),
            cell_height: Some(32),
            control: ControlMode::Take,
            history: Some(100),
        }))
        .await
        .unwrap();
    assert!(matches!(
        companion_next(&mut adapter).await,
        HostMessage::Result(_)
    ));
    let HostMessage::Snapshot(initial_snap) = companion_next(&mut adapter).await else {
        panic!("expected initial companion snapshot");
    };
    assert_eq!(
        (initial_snap.cell_width, initial_snap.cell_height),
        (16, 32)
    );
    assert!(!initial_snap.data.0.windows(3).any(|w| w == b"\x1b_G"));
    let HostMessage::Resized(initial_resized) = companion_next(&mut adapter).await else {
        panic!("expected initial companion resized");
    };
    assert_eq!(
        (initial_resized.cell_width, initial_resized.cell_height),
        (16, 32)
    );
    assert_eq!(
        core.sessions.terminal_model(&id).unwrap().cell_size(),
        CellSize {
            width: 16,
            height: 32
        }
    );
    // Drain the Resized frame on local_sub caused by companion attach.
    assert!(matches!(
        local_sub.frames.recv().unwrap(),
        runner_core::protocol::terminal::TerminalFrame::Resized {
            cols: 90,
            rows: 30,
            cell_width: 16,
            cell_height: 32,
            ..
        }
    ));

    // Companion Viewport without cellWidth/cellHeight preserves (16, 32); with (20, 40) updates it.
    adapter
        .request(ClientMessage::Viewport(ViewportRequest {
            id: 2,
            session: id.clone(),
            cols: 92,
            rows: 30,
            cell_width: None,
            cell_height: None,
        }))
        .await
        .unwrap();
    let mut saw_preserved = false;
    while !saw_preserved {
        if let HostMessage::Resized(r) = companion_next(&mut adapter).await {
            assert_eq!(
                (r.cols, r.rows, r.cell_width, r.cell_height),
                (92, 30, 16, 32)
            );
            saw_preserved = true;
        }
    }
    let _ = local_sub.frames.recv().unwrap();

    adapter
        .request(ClientMessage::TakeControl(ViewportRequest {
            id: 3,
            session: id.clone(),
            cols: 92,
            rows: 30,
            cell_width: Some(20),
            cell_height: Some(40),
        }))
        .await
        .unwrap();
    let mut saw_updated = false;
    while !saw_updated {
        if let HostMessage::Resized(r) = companion_next(&mut adapter).await {
            assert_eq!(
                (r.cols, r.rows, r.cell_width, r.cell_height),
                (92, 30, 20, 40)
            );
            saw_updated = true;
        }
    }
    let _ = local_sub.frames.recv().unwrap();

    // Stream a 30 MiB image transmission (`t=f`) surrounded by text.
    let raw_30m = vec![0x42u8; 3200 * 3280 * 3]; // 31,488,000 bytes (~30.03 MiB)
    let file_30m = temp.path().join("companion-30mib.bin");
    std::fs::write(&file_30m, &raw_30m).unwrap();
    let path_b64 =
        base64::engine::general_purpose::STANDARD.encode(file_30m.to_string_lossy().as_bytes());
    let burst = format!("before-30m\x1b_Ga=T,i=1,t=f,f=24,s=3200,v=3280;{path_b64}\x1b\\after-30m");
    runtime.push_output(&burst.into_bytes());

    // Local subscriber receives leading text, the ~40 MiB normalized image frame, and trailing text.
    let mut local_saw_graphics = false;
    let mut local_frames = 0usize;
    while local_frames < 3 {
        let frame = local_sub.frames.recv().unwrap();
        if let runner_core::protocol::terminal::TerminalFrame::Output { bytes, .. } = frame {
            if bytes.windows(3).any(|w| w == b"\x1b_G") {
                local_saw_graphics = true;
            }
            local_frames += 1;
        }
    }
    assert!(
        local_saw_graphics,
        "local subscriber must receive the 30 MiB image"
    );

    // Companion client receives ONLY the surrounding text — no graphics, no empty output, no resync!
    let mut companion_text = Vec::new();
    let mut companion_seq = initial_resized.seq;
    while !companion_text.ends_with(b"after-30m") {
        match companion_next(&mut adapter).await {
            HostMessage::Output(out) => {
                assert!(
                    !out.data.0.is_empty(),
                    "companion must never receive an empty output message"
                );
                assert!(
                    !out.data.0.windows(3).any(|w| w == b"\x1b_G"),
                    "companion must never receive Kitty graphics commands"
                );
                assert!(out.seq > companion_seq);
                companion_seq = out.seq;
                companion_text.extend_from_slice(&out.data.0);
            }
            HostMessage::Snapshot(_) => {
                panic!("30 MiB image transmission must not trigger a companion resync");
            }
            HostMessage::Result(_) => {}
            other => panic!("unexpected companion message during 30 MiB burst: {other:?}"),
        }
    }
    assert_eq!(companion_text, b"before-30mafter-30m");

    // Local snapshot includes graphics; companion attach, re-attach, and final snapshots omit them.
    let local_snap = core.sessions.attach_terminal(&id).unwrap().snapshot;
    assert!(local_snap.bytes.windows(3).any(|w| w == b"\x1b_G"));

    let (comp_attach, comp_source) = core.sessions.attach_companion_terminal(&id, 100).unwrap();
    assert!(!comp_attach
        .snapshot
        .bytes
        .windows(3)
        .any(|w| w == b"\x1b_G"));
    assert_eq!(
        (
            comp_attach.snapshot.cell_width,
            comp_attach.snapshot.cell_height
        ),
        (20, 40)
    );

    let comp_reattach = comp_source.attach().unwrap();
    assert!(!comp_reattach
        .snapshot
        .bytes
        .windows(3)
        .any(|w| w == b"\x1b_G"));

    // Trigger final companion snapshot (session exit path) and verify it also omits graphics.
    core.sessions
        .session_state(&id)
        .unwrap()
        .lock()
        .unwrap()
        .finish_companion_terminals();
    let comp_final = comp_source.attach().unwrap();
    assert!(!comp_final.snapshot.bytes.windows(3).any(|w| w == b"\x1b_G"));
    assert_eq!(
        (
            comp_final.snapshot.cell_width,
            comp_final.snapshot.cell_height
        ),
        (20, 40)
    );

    drop(adapter);
    core.sessions.kill(&id).unwrap();
    core.sessions.drain_terminal_workers();
}

#[test]
fn daemon_frame_queue_resyncs_above_128_mib_below_frame_count_cap() {
    use runner_core::protocol::terminal::{TerminalFrame, TERMINAL_QUEUE_MAX_BYTES};

    let queue = super::FrameQueue::default();
    let chunk = vec![b'a'; 45 * 1024 * 1024]; // 3 * 45 MiB = 135 MiB > 128 MiB, 3 << 64
    assert!(3 * chunk.len() > TERMINAL_QUEUE_MAX_BYTES);
    assert!(queue.push(TerminalFrame::Output {
        seq: 1,
        bytes: chunk.clone(),
    }));
    assert!(queue.push(TerminalFrame::Output {
        seq: 2,
        bytes: chunk.clone(),
    }));
    assert!(!queue.push(TerminalFrame::Output {
        seq: 3,
        bytes: chunk,
    }));
    let drained = queue.drain();
    assert_eq!(drained.len(), 1);
    assert!(matches!(drained[0], TerminalFrame::Resync));
    assert!(queue.is_closed());
}

#[test]
fn forwarder_bounds_staged_media_and_normalized_memory_across_burst() {
    use crate::session::manager::SessionManager;
    use base64::Engine as _;
    use runner_core::protocol::terminal::TerminalFrame;
    use runner_terminal::kitty::GlobalImageBudget;

    let temp = tempfile::tempdir().unwrap();
    let runtime = Arc::new(RecordingRuntime::default());
    let budget = Arc::new(GlobalImageBudget::new(8 * 1024 * 1024));
    let mut core = test_core_with_runtime(temp.path(), Arc::clone(&runtime) as _);
    core.sessions = SessionManager::new_with_image_budget(
        Arc::clone(&core.runtime_shell_env),
        Arc::clone(&core.runtime_discovery),
        Arc::clone(&runtime) as _,
        budget,
    );
    core.sessions.set_runtime_events(core.events.clone());
    core.sessions.set_kitty_staging_budget(64 * 1024);

    let role = {
        let conn = core.db.get().unwrap();
        crate::test_support::insert_test_role(
            &conn,
            "bound-burst",
            "bound-burst",
            "trae",
            "bound-burst",
        );
        crate::ops::role::get(&conn, "bound-burst").unwrap()
    };
    let spawned = core
        .sessions
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(temp.path().to_str().unwrap().into()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
        .unwrap();
    let id = spawned.id.clone();
    let mut sub = core.sessions.attach_terminal(&id).unwrap();

    // Build ONE burst containing 10 interleaved text + 40 KiB `t=f` images (~407 KiB total raw,
    // ~542 KiB normalized) with staging budget set to 64 KiB.
    let img_raw = vec![0x55u8; 128 * 106 * 3]; // 40,704 B (< 64 KiB)
    let img_file = temp.path().join("staged-40k.bin");
    std::fs::write(&img_file, &img_raw).unwrap();
    let path_b64 =
        base64::engine::general_purpose::STANDARD.encode(img_file.to_string_lossy().as_bytes());
    let mut burst = Vec::new();
    let mut expected_replies = Vec::new();
    for i in 1..=10u32 {
        burst.extend_from_slice(format!("t{i}|").as_bytes());
        burst.extend_from_slice(
            format!("\x1b_Ga=t,i={i},t=f,f=24,s=128,v=106;{path_b64}\x1b\\").as_bytes(),
        );
        expected_replies.push(format!("\x1b_Gi={i};OK\x1b\\").into_bytes());
    }
    burst.extend_from_slice(b"end-burst");

    runtime.push_output(&burst);
    let replies = {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            let writes = runtime.writes.lock().unwrap().clone();
            if writes.len() >= 10 {
                break writes;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for 10 ordered Kitty replies (got {})",
                writes.len()
            );
            std::thread::yield_now();
        }
    };
    assert_eq!(
        replies, expected_replies,
        "all 10 replies must arrive in exact stream order"
    );

    let peak_staged = core.sessions.max_staged_kitty_bytes();
    assert!(
        peak_staged >= img_raw.len() && peak_staged <= 64 * 1024,
        "forwarder must bound staged media to a single batch/file (expected {}..=65536, got {peak_staged})",
        img_raw.len()
    );

    let mut plain_text = Vec::new();
    while !plain_text.ends_with(b"end-burst") {
        match sub.frames.recv().unwrap() {
            TerminalFrame::Output { bytes, .. } => {
                if let Some(stripped) = runner_terminal::kitty::strip_graphics_commands(&bytes) {
                    plain_text.extend_from_slice(&stripped);
                } else {
                    plain_text.extend_from_slice(&bytes);
                }
            }
            TerminalFrame::Resized { .. } | TerminalFrame::ClearGraphics { .. } => {}
            TerminalFrame::Resync => panic!("bounded 400 KiB burst must not Resync"),
        }
    }
    assert_eq!(
        String::from_utf8_lossy(&plain_text),
        "t1|t2|t3|t4|t5|t6|t7|t8|t9|t10|end-burst"
    );
    let model = core.sessions.terminal_model(&id).unwrap();
    let graphics = model.graphics().lock().unwrap();
    for i in 1..=10u32 {
        assert!(
            graphics.primary.images().contains_key(&i),
            "missing image i={i}"
        );
    }
    drop(graphics);
    drop(sub);
    core.sessions.kill(&id).unwrap();
    core.sessions.drain_terminal_workers();
}

#[test]
fn retained_terminal_exit_keeps_text_and_releases_both_screens_graphics_budget() {
    use crate::session::manager::SessionManager;
    use base64::Engine as _;
    use runner_core::protocol::{Runtime, RuntimeTaskKind, RuntimeUpdateState, RuntimeUpdateTask};
    use runner_terminal::kitty::GlobalImageBudget;
    use std::collections::HashMap;
    use std::sync::Mutex;

    #[derive(Default)]
    struct ClosableRuntime {
        outputs: Mutex<HashMap<String, std::sync::mpsc::Sender<RuntimeOutput>>>,
        writes: Mutex<Vec<Vec<u8>>>,
        writes_cv: std::sync::Condvar,
    }
    impl ClosableRuntime {
        fn push_output(&self, id: &str, bytes: &[u8]) {
            self.outputs
                .lock()
                .unwrap()
                .get(id)
                .unwrap()
                .send(RuntimeOutput::Stream(bytes.to_vec()))
                .unwrap();
        }
        fn close(&self, id: &str) {
            self.outputs.lock().unwrap().remove(id);
        }
        fn wait_writes_len(&self, expected: usize) -> Vec<Vec<u8>> {
            let mut guard = self.writes.lock().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while guard.len() < expected {
                let rem = deadline
                    .checked_duration_since(std::time::Instant::now())
                    .expect("timed out waiting for PTY replies");
                let (next, _) = self.writes_cv.wait_timeout(guard, rem).unwrap();
                guard = next;
            }
            guard.clone()
        }
    }
    impl SessionRuntime for ClosableRuntime {
        fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
            let (tx, rx) = std::sync::mpsc::channel();
            self.outputs
                .lock()
                .unwrap()
                .insert(spec.session_id.clone(), tx);
            Ok((
                RuntimeSession {
                    runtime: "closable-kitty".into(),
                    session_id: spec.session_id,
                },
                OutputStream::new(rx, Arc::new(AtomicBool::new(false))),
            ))
        }
        fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
            self.close(&session.session_id);
            Ok(())
        }
        fn send_bytes(&self, _: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()> {
            self.writes.lock().unwrap().push(bytes.to_vec());
            self.writes_cv.notify_all();
            Ok(())
        }
        fn send_key(&self, _: &RuntimeSession, _: &str) -> RuntimeResult<()> {
            Ok(())
        }
        fn resize(&self, _: &RuntimeSession, _: u16, _: u16, _: u16, _: u16) -> RuntimeResult<()> {
            Ok(())
        }
        fn status(&self, _: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
            Ok(Some(SessionStatus {
                alive: false,
                exit_code: Some(0),
                ..Default::default()
            }))
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let runtime = Arc::new(ClosableRuntime::default());
    let budget = Arc::new(GlobalImageBudget::new(4 * 1024 * 1024));
    let mut core = test_core_with_runtime(temp.path(), Arc::clone(&runtime) as _);
    core.sessions = SessionManager::new_with_image_budget(
        Arc::clone(&core.runtime_shell_env),
        Arc::clone(&core.runtime_discovery),
        Arc::clone(&runtime) as _,
        Arc::clone(&budget),
    );
    core.sessions.set_runtime_events(core.events.clone());

    let role = {
        let conn = core.db.get().unwrap();
        crate::test_support::insert_test_role(
            &conn,
            "retain-exit",
            "retain-exit",
            "trae",
            "retain-exit",
        );
        crate::ops::role::get(&conn, "retain-exit").unwrap()
    };
    let spawned = core
        .sessions
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(temp.path().to_str().unwrap().into()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
        .unwrap();
    let id = spawned.id.clone();

    // Mark session as a runtime update task so `forget_runtime_handle` takes the `retain_terminal`
    // path on natural exit.
    core.sessions
        .runtime_updates
        .lock()
        .unwrap()
        .tasks
        .push(RuntimeUpdateTask {
            id: id.clone(),
            runtime: Runtime::Codex,
            kind: RuntimeTaskKind::Update,
            from_version: None,
            to_version: None,
            state: RuntimeUpdateState::Running,
            step: "Updating".into(),
            message: None,
            exit_code: None,
            installed_version: None,
            running_sessions: 0,
        });

    let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    let img = b64(&vec![7u8; 200 * 200 * 3]); // 120,000 B each
    runtime.push_output(
        &id,
        format!(
            "retained-text\x1b_Ga=t,i=1,f=24,s=200,v=200;{img}\x1b\\\
             \x1b[?1049h\x1b_Ga=t,i=2,f=24,s=200,v=200;{img}\x1b\\\x1b[?1049l"
        )
        .as_bytes(),
    );
    let replies = runtime.wait_writes_len(2);
    assert_eq!(
        replies,
        vec![
            b"\x1b_Gi=1;OK\x1b\\".to_vec(),
            b"\x1b_Gi=2;OK\x1b\\".to_vec()
        ]
    );
    assert_eq!(budget.used(), 240_000);

    // Close PTY stream so forwarder exits through `forget_runtime_handle` (`retain_terminal=true`).
    runtime.close(&id);
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    while core.sessions.live_runtime_session(&id).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for retained-terminal session to exit"
        );
        std::thread::yield_now();
    }

    assert_eq!(
        budget.used(),
        0,
        "retained-terminal exit must free both primary and alt screen graphics budget"
    );
    let model = core
        .sessions
        .terminal_model(&id)
        .expect("retained terminal model must still exist after exit");
    {
        let graphics = model.graphics().lock().unwrap();
        assert!(graphics.primary.images().is_empty());
        assert!(graphics.alt.images().is_empty());
    }
    let snap = model.snapshot(0);
    assert!(
        String::from_utf8_lossy(&snap.bytes).contains("retained-text"),
        "retained terminal text must survive graphics release on exit"
    );

    core.sessions.drain_terminal_workers();
}

#[test]
fn exiting_forwarder_cannot_clear_resumed_process_images_or_handle() {
    use crate::session::manager::{
        ExitEvent, RoleActivityEvent, SessionManager, SessionSpawnedEvent,
    };
    use crate::session::runtime::RuntimeError;
    use base64::Engine as _;
    use runner_core::protocol::{
        ClientError, ClientEvent, DaemonClient, EventError, EventSubscription, Request, Response,
        Transport,
    };
    use runner_terminal::kitty::GlobalImageBudget;
    use runner_terminal::terminal::TerminalBridge;
    use std::sync::Mutex;

    struct ClosedEvents;
    impl EventSubscription for ClosedEvents {
        fn recv(
            &mut self,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<Output = std::result::Result<ClientEvent, EventError>>
                    + Send
                    + '_,
            >,
        > {
            Box::pin(async { Err(EventError::Closed) })
        }
    }
    struct LocalSessionsTransport(Arc<SessionManager>);
    impl Transport for LocalSessionsTransport {
        fn call(&self, _request: Request) -> std::result::Result<Response, ClientError> {
            Err(ClientError::msg("unused"))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            Box::new(ClosedEvents)
        }
        fn attach(
            &self,
            session_id: &str,
        ) -> std::result::Result<runner_core::protocol::terminal::TerminalAttachment, ClientError>
        {
            self.0
                .attach_terminal(session_id)
                .map_err(|e| ClientError::msg(e.to_string()))
        }
    }

    type StopGate = (
        std::sync::mpsc::Sender<()>,
        std::sync::mpsc::Receiver<RuntimeResult<()>>,
    );

    #[derive(Default)]
    struct MultiLaunchRuntime {
        /// Indexed by spawn ordinal (0 = launch 1, 1 = launch 2, ...), sharing the same session ID & runtime label.
        spawns: Mutex<Vec<Option<std::sync::mpsc::Sender<RuntimeOutput>>>>,
        writes: Mutex<Vec<Vec<u8>>>,
        writes_cv: std::sync::Condvar,
        stop_gate: Mutex<Option<StopGate>>,
    }
    impl MultiLaunchRuntime {
        fn push_output(&self, launch: usize, bytes: &[u8]) {
            self.spawns.lock().unwrap()[launch]
                .as_ref()
                .expect("launch stream open")
                .send(RuntimeOutput::Stream(bytes.to_vec()))
                .unwrap();
        }
        fn close_launch(&self, launch: usize) {
            self.spawns.lock().unwrap()[launch] = None;
        }
        fn arm_stop_gate(
            &self,
        ) -> (
            std::sync::mpsc::Receiver<()>,
            std::sync::mpsc::Sender<RuntimeResult<()>>,
        ) {
            let (entered_tx, entered_rx) = std::sync::mpsc::channel();
            let (release_tx, release_rx) = std::sync::mpsc::channel();
            *self.stop_gate.lock().unwrap() = Some((entered_tx, release_rx));
            (entered_rx, release_tx)
        }
        fn wait_writes_len(&self, expected: usize) -> Vec<Vec<u8>> {
            let mut guard = self.writes.lock().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while guard.len() < expected {
                let rem = deadline
                    .checked_duration_since(std::time::Instant::now())
                    .expect("timed out waiting for PTY replies");
                let (next, _) = self.writes_cv.wait_timeout(guard, rem).unwrap();
                guard = next;
            }
            guard.clone()
        }
        fn take_writes(&self) -> Vec<Vec<u8>> {
            std::mem::take(&mut *self.writes.lock().unwrap())
        }
    }
    impl SessionRuntime for MultiLaunchRuntime {
        fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
            let (tx, rx) = std::sync::mpsc::channel();
            self.spawns.lock().unwrap().push(Some(tx));
            Ok((
                RuntimeSession {
                    runtime: "shared-label".into(),
                    session_id: spec.session_id,
                },
                OutputStream::new(rx, Arc::new(AtomicBool::new(false))),
            ))
        }
        fn stop(&self, _: &RuntimeSession) -> RuntimeResult<()> {
            let gate = self.stop_gate.lock().unwrap().take();
            if let Some((entered_tx, release_rx)) = gate {
                let _ = entered_tx.send(());
                return release_rx
                    .recv_timeout(std::time::Duration::from_secs(10))
                    .expect("stop gate released");
            }
            Ok(())
        }
        fn send_bytes(&self, _: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()> {
            self.writes.lock().unwrap().push(bytes.to_vec());
            self.writes_cv.notify_all();
            Ok(())
        }
        fn send_key(&self, _: &RuntimeSession, _: &str) -> RuntimeResult<()> {
            Ok(())
        }
        fn resize(&self, _: &RuntimeSession, _: u16, _: u16, _: u16, _: u16) -> RuntimeResult<()> {
            Ok(())
        }
        fn status(&self, _: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
            Ok(Some(SessionStatus {
                alive: false,
                exit_code: Some(0),
                ..Default::default()
            }))
        }
    }

    struct PausingEvents {
        pause_tx: Mutex<Option<std::sync::mpsc::Sender<()>>>,
        release_rx: Mutex<Option<std::sync::mpsc::Receiver<()>>>,
        log: Arc<Mutex<Vec<&'static str>>>,
        bridge: Arc<TerminalBridge>,
    }
    impl SessionEvents for PausingEvents {
        fn spawned(&self, ev: &SessionSpawnedEvent) {
            self.log.lock().unwrap().push("session/spawned");
            self.bridge.handle_event(&ClientEvent {
                name: "session/spawned".into(),
                payload: serde_json::json!({ "session_id": ev.session_id }),
            });
        }
        fn role_activity(&self, ev: &RoleActivityEvent) {
            // Only pause the exiting forwarder's role_activity callback (after DB committed stopped),
            // not the initial spawn's active_sessions == 1 callback.
            if ev.active_sessions == 0 {
                if let Some(tx) = self.pause_tx.lock().unwrap().take() {
                    let _ = tx.send(());
                    if let Some(rx) = self.release_rx.lock().unwrap().take() {
                        let _ = rx.recv_timeout(std::time::Duration::from_secs(10));
                    }
                }
            }
        }
        fn exit(&self, ev: &ExitEvent) {
            self.log.lock().unwrap().push("session/exit");
            self.bridge.handle_event(&ClientEvent {
                name: "session/exit".into(),
                payload: serde_json::json!({ "session_id": ev.session_id }),
            });
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let runtime = Arc::new(MultiLaunchRuntime::default());
    let budget = Arc::new(GlobalImageBudget::new(4 * 1024 * 1024));
    let mut core = test_core_with_runtime(temp.path(), Arc::clone(&runtime) as _);
    core.sessions = SessionManager::new_with_image_budget(
        Arc::clone(&core.runtime_shell_env),
        Arc::clone(&core.runtime_discovery),
        Arc::clone(&runtime) as _,
        Arc::clone(&budget),
    );
    core.sessions.set_runtime_events(core.events.clone());

    let client = DaemonClient::new(Arc::new(LocalSessionsTransport(Arc::clone(&core.sessions))));
    let bridge = TerminalBridge::new(client, Arc::new(|| {})).unwrap();
    let events_log = Arc::new(Mutex::new(Vec::<&'static str>::new()));

    let role = {
        let conn = core.db.get().unwrap();
        crate::test_support::insert_test_role(&conn, "overlap", "overlap", "trae", "overlap");
        crate::ops::role::get(&conn, "overlap").unwrap()
    };
    let (pause_tx, pause_rx) = std::sync::mpsc::channel();
    let (release_tx, release_rx) = std::sync::mpsc::channel();
    let events: Arc<dyn SessionEvents> = Arc::new(PausingEvents {
        pause_tx: Mutex::new(Some(pause_tx)),
        release_rx: Mutex::new(Some(release_rx)),
        log: Arc::clone(&events_log),
        bridge: Arc::clone(&bridge),
    });
    let spawned = core
        .sessions
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(temp.path().to_str().unwrap().into()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
            &core.app_data_dir,
            core.db.clone(),
            Arc::clone(&events),
        )
        .unwrap();
    let id = spawned.id.clone();

    let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    let img = b64(&vec![5u8; 64 * 64 * 3]);
    runtime.push_output(
        0,
        format!("\x1b_Ga=t,i=10,f=24,s=64,v=64;{img}\x1b\\").as_bytes(),
    );
    assert_eq!(
        runtime.wait_writes_len(1),
        vec![b"\x1b_Gi=10;OK\x1b\\".to_vec()]
    );
    runtime.take_writes();
    assert_eq!(budget.used(), 64 * 64 * 3);

    // Part A: Close Launch 1's stream and pause its forwarder at `emit_role_activity` AFTER DB committed `stopped`.
    runtime.close_launch(0);
    pause_rx
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("launch 1 forwarder reached role_activity callback");
    assert_eq!(
        budget.used(),
        0,
        "launch 1 must already have released its graphics before committing stopped"
    );

    // Resume Launch 2 (same session ID and same runtime label) and store image i=75.
    core.sessions
        .resume(
            &id,
            Some(80),
            Some(24),
            &core.app_data_dir,
            core.db.clone(),
            Arc::clone(&events),
        )
        .unwrap();
    let mirror_launch2 = bridge
        .session(&id)
        .expect("bridge attached Launch 2 mirror on session/spawned");
    runtime.push_output(
        1,
        format!("\x1b_Ga=t,i=75,f=24,s=64,v=64;{img}\x1b\\").as_bytes(),
    );
    assert_eq!(
        runtime.wait_writes_len(1),
        vec![b"\x1b_Gi=75;OK\x1b\\".to_vec()]
    );
    runtime.take_writes();
    assert!(core
        .sessions
        .terminal_model(&id)
        .unwrap()
        .graphics()
        .lock()
        .unwrap()
        .primary
        .images()
        .contains_key(&75));

    // Release Launch 1's paused forwarder and fence on its completion hook (not session/exit, which must be suppressed).
    let launch1_done = core.sessions.watch_forwarder_exit(&id);
    let _ = release_tx.send(());
    launch1_done
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("launch 1 forwarder completed exit");
    assert_eq!(
        *events_log.lock().unwrap(),
        vec!["session/spawned", "session/spawned"],
        "stale launch 1 forwarder must not emit session/exit over resumed launch 2"
    );
    assert!(
        bridge
            .session(&id)
            .is_some_and(|m| Arc::ptr_eq(&m, &mirror_launch2)),
        "TerminalBridge must keep launch 2's mirror after stale launch 1 forwarder exits"
    );
    runtime.push_output(1, b"\x1b_Ga=q,i=76,f=24,s=1,v=1;AAAA\x1b\\");
    assert_eq!(
        runtime.wait_writes_len(1),
        vec![b"\x1b_Gi=76;OK\x1b\\".to_vec()]
    );
    runtime.take_writes();
    assert!(
        core.sessions.live_runtime_session(&id).is_ok(),
        "stale launch 1 forwarder must not forget launch 2's live handle"
    );
    assert!(
        core.sessions
            .terminal_model(&id)
            .unwrap()
            .graphics()
            .lock()
            .unwrap()
            .primary
            .images()
            .contains_key(&75),
        "stale launch 1 forwarder must not erase launch 2's image 75"
    );
    assert_eq!(budget.used(), 64 * 64 * 3);
    drop(mirror_launch2);
    drop(bridge);

    // Part B1: Delayed kill() paused inside runtime.stop() returning Ok(()) must not retire
    // or wipe images of Launch 3 installed after Launch 2 exited cleanly.
    let (stop1_entered, stop1_release) = runtime.arm_stop_gate();
    let sessions_k1 = Arc::clone(&core.sessions);
    let id_k1 = id.clone();
    let kill1 = std::thread::spawn(move || sessions_k1.kill(&id_k1));
    stop1_entered
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("kill1 entered runtime.stop");

    let launch2_done = core.sessions.watch_forwarder_exit(&id);
    runtime.close_launch(1);
    launch2_done
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("launch 2 exited cleanly");

    core.sessions
        .resume(
            &id,
            Some(80),
            Some(24),
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
        .unwrap();
    runtime.push_output(
        2,
        format!("\x1b_Ga=t,i=88,f=24,s=64,v=64;{img}\x1b\\").as_bytes(),
    );
    assert_eq!(
        runtime.wait_writes_len(1),
        vec![b"\x1b_Gi=88;OK\x1b\\".to_vec()]
    );
    runtime.take_writes();

    stop1_release.send(Ok(())).unwrap();
    kill1
        .join()
        .unwrap()
        .expect("delayed kill1 returns Ok without disturbing launch 3");
    assert!(
        core.sessions.live_runtime_session(&id).is_ok(),
        "delayed kill1 (Ok) must not remove launch 3's live handle"
    );
    assert!(
        core.sessions
            .terminal_model(&id)
            .unwrap()
            .graphics()
            .lock()
            .unwrap()
            .primary
            .images()
            .contains_key(&88),
        "delayed kill1 (Ok) must not clear launch 3's image 88"
    );
    runtime.push_output(2, b"\x1b_Ga=q,i=89,f=24,s=1,v=1;AAAA\x1b\\");
    assert_eq!(
        runtime.wait_writes_len(1),
        vec![b"\x1b_Gi=89;OK\x1b\\".to_vec()]
    );
    runtime.take_writes();

    // Part B2: Delayed kill() paused inside runtime.stop() returning Err must also leave
    // Launch 4's handle, state, and images intact.
    let (stop2_entered, stop2_release) = runtime.arm_stop_gate();
    let sessions_k2 = Arc::clone(&core.sessions);
    let id_k2 = id.clone();
    let kill2 = std::thread::spawn(move || sessions_k2.kill(&id_k2));
    stop2_entered
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("kill2 entered runtime.stop");

    let launch3_done = core.sessions.watch_forwarder_exit(&id);
    runtime.close_launch(2);
    launch3_done
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("launch 3 exited cleanly");

    core.sessions
        .resume(
            &id,
            Some(80),
            Some(24),
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
        .unwrap();
    runtime.push_output(
        3,
        format!("\x1b_Ga=t,i=99,f=24,s=64,v=64;{img}\x1b\\").as_bytes(),
    );
    assert_eq!(
        runtime.wait_writes_len(1),
        vec![b"\x1b_Gi=99;OK\x1b\\".to_vec()]
    );
    runtime.take_writes();

    stop2_release
        .send(Err(RuntimeError::Msg("stop failed".into())))
        .unwrap();
    assert!(
        kill2.join().unwrap().is_err(),
        "delayed kill2 must propagate its stop error"
    );
    assert!(
        core.sessions.live_runtime_session(&id).is_ok(),
        "delayed kill2 (Err) must not disturb launch 4's live handle"
    );
    assert!(
        core.sessions
            .terminal_model(&id)
            .unwrap()
            .graphics()
            .lock()
            .unwrap()
            .primary
            .images()
            .contains_key(&99),
        "delayed kill2 (Err) must not clear launch 4's image 99"
    );
    runtime.push_output(3, b"\x1b_Ga=q,i=100,f=24,s=1,v=1;AAAA\x1b\\");
    assert_eq!(
        runtime.wait_writes_len(1),
        vec![b"\x1b_Gi=100;OK\x1b\\".to_vec()]
    );
    runtime.take_writes();
    assert_eq!(budget.used(), 64 * 64 * 3);

    runtime.close_launch(3);
    core.sessions.kill(&id).unwrap();
    core.sessions.drain_terminal_workers();
    assert_eq!(budget.used(), 0);
}

#[test]
fn stopping_or_retaining_exit_resyncs_local_mirrors_on_both_screens_while_preserving_text() {
    use crate::session::manager::SessionManager;
    use alacritty_terminal::grid::Dimensions;
    use alacritty_terminal::index::{Column, Line, Point};
    use base64::Engine as _;
    use runner_core::protocol::terminal::TerminalFrame;
    use runner_core::protocol::{
        ClientError, DaemonClient, EventError, EventSubscription, Request, Response, Runtime,
        RuntimeTaskKind, RuntimeUpdateState, RuntimeUpdateTask, Transport,
    };
    use runner_terminal::kitty::GlobalImageBudget;
    use runner_terminal::terminal::TerminalMirror;
    use std::collections::HashMap;
    use std::sync::Mutex;

    struct ClosedEvents;
    impl EventSubscription for ClosedEvents {
        fn recv(
            &mut self,
        ) -> std::pin::Pin<
            Box<
                dyn std::future::Future<
                        Output = std::result::Result<
                            runner_core::protocol::ClientEvent,
                            EventError,
                        >,
                    > + Send
                    + '_,
            >,
        > {
            Box::pin(async { Err(EventError::Closed) })
        }
    }
    struct LocalSessionsTransport(Arc<SessionManager>);
    impl Transport for LocalSessionsTransport {
        fn call(&self, _request: Request) -> std::result::Result<Response, ClientError> {
            Err(ClientError::msg("unused"))
        }
        fn subscribe(&self) -> Box<dyn EventSubscription> {
            Box::new(ClosedEvents)
        }
        fn attach(
            &self,
            session_id: &str,
        ) -> std::result::Result<runner_core::protocol::terminal::TerminalAttachment, ClientError>
        {
            self.0
                .attach_terminal(session_id)
                .map_err(|e| ClientError::msg(e.to_string()))
        }
    }

    #[derive(Default)]
    struct MultiSessionCloseRuntime {
        outputs: Mutex<HashMap<String, Vec<Option<std::sync::mpsc::Sender<RuntimeOutput>>>>>,
        writes: Mutex<HashMap<String, Vec<Vec<u8>>>>,
        writes_cv: std::sync::Condvar,
    }
    impl MultiSessionCloseRuntime {
        fn push_latest(&self, id: &str, bytes: &[u8]) {
            self.outputs.lock().unwrap()[id]
                .last()
                .and_then(Option::as_ref)
                .expect("open output stream")
                .send(RuntimeOutput::Stream(bytes.to_vec()))
                .unwrap();
        }
        fn close_latest(&self, id: &str) {
            if let Some(slot) = self
                .outputs
                .lock()
                .unwrap()
                .get_mut(id)
                .and_then(|v| v.last_mut())
            {
                *slot = None;
            }
        }
        fn wait_writes_len(&self, id: &str, expected: usize) -> Vec<Vec<u8>> {
            let mut guard = self.writes.lock().unwrap();
            let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
            while guard.get(id).map_or(0, Vec::len) < expected {
                let rem = deadline
                    .checked_duration_since(std::time::Instant::now())
                    .expect("timed out waiting for PTY replies");
                let (next, _) = self.writes_cv.wait_timeout(guard, rem).unwrap();
                guard = next;
            }
            guard.get(id).cloned().unwrap_or_default()
        }
        fn take_writes(&self, id: &str) {
            if let Some(v) = self.writes.lock().unwrap().get_mut(id) {
                v.clear();
            }
        }
    }
    impl SessionRuntime for MultiSessionCloseRuntime {
        fn spawn(&self, spec: SpawnSpec) -> RuntimeResult<(RuntimeSession, OutputStream)> {
            let (tx, rx) = std::sync::mpsc::channel();
            self.outputs
                .lock()
                .unwrap()
                .entry(spec.session_id.clone())
                .or_default()
                .push(Some(tx));
            Ok((
                RuntimeSession {
                    runtime: "resync-kitty".into(),
                    session_id: spec.session_id,
                },
                OutputStream::new(rx, Arc::new(AtomicBool::new(false))),
            ))
        }
        fn stop(&self, session: &RuntimeSession) -> RuntimeResult<()> {
            self.close_latest(&session.session_id);
            Ok(())
        }
        fn send_bytes(&self, session: &RuntimeSession, bytes: &[u8]) -> RuntimeResult<()> {
            self.writes
                .lock()
                .unwrap()
                .entry(session.session_id.clone())
                .or_default()
                .push(bytes.to_vec());
            self.writes_cv.notify_all();
            Ok(())
        }
        fn send_key(&self, _: &RuntimeSession, _: &str) -> RuntimeResult<()> {
            Ok(())
        }
        fn resize(&self, _: &RuntimeSession, _: u16, _: u16, _: u16, _: u16) -> RuntimeResult<()> {
            Ok(())
        }
        fn status(&self, _: &RuntimeSession) -> RuntimeResult<Option<SessionStatus>> {
            Ok(Some(SessionStatus {
                alive: false,
                exit_code: Some(0),
                ..Default::default()
            }))
        }
    }

    fn read_row0(m: &TerminalMirror) -> String {
        let term = m.term.lock();
        (0..term.columns())
            .map(|c| term.grid()[Point::new(Line(0), Column(c))].c)
            .collect::<String>()
            .trim_end()
            .to_owned()
    }
    fn wait_until(pred: impl Fn() -> bool, msg: &str) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        while !pred() {
            assert!(std::time::Instant::now() < deadline, "{msg}");
            std::thread::yield_now();
        }
    }

    let temp = tempfile::tempdir().unwrap();
    let runtime = Arc::new(MultiSessionCloseRuntime::default());
    let budget = Arc::new(GlobalImageBudget::new(4 * 1024 * 1024));
    let mut core = test_core_with_runtime(temp.path(), Arc::clone(&runtime) as _);
    core.sessions = SessionManager::new_with_image_budget(
        Arc::clone(&core.runtime_shell_env),
        Arc::clone(&core.runtime_discovery),
        Arc::clone(&runtime) as _,
        Arc::clone(&budget),
    );
    core.sessions.set_runtime_events(core.events.clone());

    let role = {
        let conn = core.db.get().unwrap();
        crate::test_support::insert_test_role(&conn, "resync-m", "resync-m", "trae", "resync-m");
        crate::ops::role::get(&conn, "resync-m").unwrap()
    };
    let transport = DaemonClient::new(Arc::new(LocalSessionsTransport(Arc::clone(&core.sessions))));
    let b64 = |bytes: &[u8]| base64::engine::general_purpose::STANDARD.encode(bytes);
    let img = b64(&vec![3u8; 32 * 32 * 3]);

    // Part A: Stop/resume at unchanged geometry with both primary and alt screen images.
    let s1 = core
        .sessions
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(temp.path().to_str().unwrap().into()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
        .unwrap()
        .id;
    let mut raw_sub = core.sessions.attach_terminal(&s1).unwrap();
    let mirror1 = TerminalMirror::attach(transport.clone(), s1.clone(), Arc::new(|| {})).unwrap();

    runtime.push_latest(
        &s1,
        format!(
            "kept-before-stop\x1b_Ga=T,i=1,f=24,s=32,v=32;{img}\x1b\\\
             \x1b[?1049h\x1b_Ga=T,i=2,f=24,s=32,v=32;{img}\x1b\\\x1b[?1049l"
        )
        .as_bytes(),
    );
    assert_eq!(runtime.wait_writes_len(&s1, 2).len(), 2);
    runtime.take_writes(&s1);
    wait_until(
        || {
            let g = mirror1.graphics().lock().unwrap();
            g.primary.images().contains_key(&1) && g.alt.images().contains_key(&2)
        },
        "mirror1 must receive both primary and alt images",
    );
    // Drain raw_sub's initial image frame so we observe the stop frame cleanly.
    let _ = raw_sub.frames.recv().unwrap();

    crate::ops::session::session_stop(&core, &s1).unwrap();
    assert_eq!(budget.used(), 0);
    assert!(
        matches!(raw_sub.frames.recv().unwrap(), TerminalFrame::ClearGraphics { .. }),
        "stopping a session with graphics must deliver ordered ClearGraphics to retained local subscribers"
    );
    wait_until(
        || {
            let g = mirror1.graphics().lock().unwrap();
            g.primary.images().is_empty() && g.alt.images().is_empty()
        },
        "mirror1 must clear both primary and alt images after stop",
    );
    assert!(
        read_row0(&mirror1).contains("kept-before-stop"),
        "stop clear must preserve retained terminal text: {:?}",
        read_row0(&mirror1)
    );

    // Resume at unchanged geometry: both raw_sub (staying open!) and mirror1 receive the new
    // process's text right after ClearGraphics with zero stale images.
    core.sessions
        .resume(
            &s1,
            Some(80),
            Some(24),
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
        .unwrap();
    runtime.push_latest(&s1, b"\r\nnew-launch-text");
    assert!(matches!(
        raw_sub.frames.recv().unwrap(),
        TerminalFrame::Output { ref bytes, .. } if bytes == b"\r\nnew-launch-text"
    ));
    wait_until(
        || {
            let term = mirror1.term.lock();
            (0..term.columns())
                .map(|c| term.grid()[Point::new(Line(1), Column(c))].c)
                .collect::<String>()
                .contains("new-launch-text")
        },
        "mirror1 must receive resumed process text after ClearGraphics",
    );
    {
        let g = mirror1.graphics().lock().unwrap();
        assert!(g.primary.images().is_empty());
        assert!(g.alt.images().is_empty());
    }
    drop(mirror1);
    drop(raw_sub);
    crate::ops::session::session_stop(&core, &s1).unwrap();

    // Part B: Agent-update `retain_terminal` natural exit with a live TerminalMirror attached.
    let s2 = core
        .sessions
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(temp.path().to_str().unwrap().into()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
        .unwrap()
        .id;
    core.sessions
        .runtime_updates
        .lock()
        .unwrap()
        .tasks
        .push(RuntimeUpdateTask {
            id: s2.clone(),
            runtime: Runtime::Codex,
            kind: RuntimeTaskKind::Update,
            from_version: None,
            to_version: None,
            state: RuntimeUpdateState::Running,
            step: "Updating".into(),
            message: None,
            exit_code: None,
            installed_version: None,
            running_sessions: 0,
        });
    let mirror2 = TerminalMirror::attach(transport.clone(), s2.clone(), Arc::new(|| {})).unwrap();
    runtime.push_latest(
        &s2,
        format!(
            "update-output\x1b_Ga=T,i=10,f=24,s=32,v=32;{img}\x1b\\\
             \x1b[?1049h\x1b_Ga=T,i=11,f=24,s=32,v=32;{img}\x1b\\\x1b[?1049l"
        )
        .as_bytes(),
    );
    assert_eq!(runtime.wait_writes_len(&s2, 2).len(), 2);
    wait_until(
        || {
            let g = mirror2.graphics().lock().unwrap();
            g.primary.images().contains_key(&10) && g.alt.images().contains_key(&11)
        },
        "mirror2 must receive both primary and alt images",
    );

    runtime.close_latest(&s2);
    wait_until(
        || core.sessions.live_runtime_session(&s2).is_err(),
        "retain_terminal update session must finish exit",
    );
    assert_eq!(budget.used(), 0);
    wait_until(
        || {
            let g = mirror2.graphics().lock().unwrap();
            g.primary.images().is_empty() && g.alt.images().is_empty()
        },
        "agent-update mirror must clear both primary and alt images on retained exit",
    );
    assert!(
        read_row0(&mirror2).contains("update-output"),
        "agent-update mirror must preserve its retained text: {:?}",
        read_row0(&mirror2)
    );
    drop(mirror2);

    // Part C: Ordinary non-retained natural exit with an UNREAD local subscriber + live mirror.
    // The terminal is released on exit (`terminal_model` disappears), yet the unread subscriber
    // MUST still receive "last text" followed by ClearGraphics (never Resync -> terminal not found).
    let s3 = core
        .sessions
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(temp.path().to_str().unwrap().into()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
        .unwrap()
        .id;
    let mut unread_sub = core.sessions.attach_terminal(&s3).unwrap();
    let mirror3 = TerminalMirror::attach(transport, s3.clone(), Arc::new(|| {})).unwrap();
    let exit3_done = core.sessions.watch_forwarder_exit(&s3);
    runtime.push_latest(
        &s3,
        format!(
            "last text\x1b_Ga=T,i=20,f=24,s=32,v=32;{img}\x1b\\\
             \x1b[?1049h\x1b_Ga=T,i=21,f=24,s=32,v=32;{img}\x1b\\\x1b[?1049l"
        )
        .as_bytes(),
    );
    assert_eq!(runtime.wait_writes_len(&s3, 2).len(), 2);
    runtime.close_latest(&s3);
    exit3_done
        .recv_timeout(std::time::Duration::from_secs(10))
        .expect("ordinary natural exit forwarder finished");
    assert_eq!(budget.used(), 0);
    assert!(
        core.sessions.terminal_model(&s3).is_err(),
        "ordinary natural exit must release the daemon terminal model"
    );

    // Drain unread_sub AFTER the daemon terminal was released: final text and ClearGraphics arrive in order.
    let first_frame = unread_sub.frames.recv().unwrap();
    let second_frame = unread_sub.frames.recv().unwrap();
    assert!(
        matches!(first_frame, TerminalFrame::Output { ref bytes, .. } if String::from_utf8_lossy(bytes).contains("last text")),
        "unread subscriber must receive final text on ordinary exit, got {first_frame:?}"
    );
    assert!(
        matches!(second_frame, TerminalFrame::ClearGraphics { .. }),
        "unread subscriber must receive ordered ClearGraphics after final text, got {second_frame:?}"
    );
    assert!(
        unread_sub.frames.recv().is_err(),
        "stream closes cleanly after final text and ClearGraphics"
    );

    wait_until(
        || {
            let g = mirror3.graphics().lock().unwrap();
            read_row0(&mirror3).contains("last text")
                && g.primary.images().is_empty()
                && g.alt.images().is_empty()
        },
        "mirror3 must preserve 'last text' and clear both screens' graphics on ordinary natural exit",
    );

    drop(mirror3);
    drop(unread_sub);
    core.sessions.drain_terminal_workers();
}

#[test]
fn burst_of_65_one_pixel_file_images_coalesces_without_queue_resync() {
    use crate::session::manager::SessionManager;
    use base64::Engine as _;
    use runner_core::protocol::terminal::TerminalFrame;
    use runner_terminal::kitty::GlobalImageBudget;

    let temp = tempfile::tempdir().unwrap();
    let runtime = Arc::new(RecordingRuntime::default());
    let budget = Arc::new(GlobalImageBudget::new(8 * 1024 * 1024));
    let mut core = test_core_with_runtime(temp.path(), Arc::clone(&runtime) as _);
    core.sessions = SessionManager::new_with_image_budget(
        Arc::clone(&core.runtime_shell_env),
        Arc::clone(&core.runtime_discovery),
        Arc::clone(&runtime) as _,
        budget,
    );
    core.sessions.set_runtime_events(core.events.clone());

    let role = {
        let conn = core.db.get().unwrap();
        crate::test_support::insert_test_role(&conn, "tiny65", "tiny65", "trae", "tiny65");
        crate::ops::role::get(&conn, "tiny65").unwrap()
    };
    let id = core
        .sessions
        .spawn_direct(
            &role,
            DirectLaunch {
                cwd: Some(temp.path().to_str().unwrap().into()),
                cols: Some(80),
                rows: Some(24),
                ..Default::default()
            },
            &core.app_data_dir,
            core.db.clone(),
            Arc::new(core.session_events()),
        )
        .unwrap()
        .id;

    // Attach an unread local subscriber before sending 65 1-pixel `t=f` images + 1 query in one read.
    let mut sub = core.sessions.attach_terminal(&id).unwrap();
    let seq_before = core.sessions.terminal_output_sequence(&id);

    let px_file = temp.path().join("one-pixel.bin");
    std::fs::write(&px_file, [0x11u8, 0x22, 0x33]).unwrap();
    let path_b64 =
        base64::engine::general_purpose::STANDARD.encode(px_file.to_string_lossy().as_bytes());
    let mut burst = Vec::new();
    for i in 1..=65u32 {
        burst.extend_from_slice(
            format!("\x1b_Ga=t,i={i},t=f,f=24,s=1,v=1;{path_b64}\x1b\\").as_bytes(),
        );
    }
    burst.extend_from_slice(b"\x1b_Ga=q,i=999,f=24,s=1,v=1;AAAA\x1b\\");
    runtime.push_output(&burst);

    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
    loop {
        if runtime.writes.lock().unwrap().len() >= 66 {
            break;
        }
        assert!(
            std::time::Instant::now() < deadline,
            "timed out waiting for 66 replies"
        );
        std::thread::yield_now();
    }

    assert_eq!(
        core.sessions.terminal_output_sequence(&id),
        seq_before + 1,
        "65 one-pixel file images (195 raw bytes) + query must coalesce into 1 frame"
    );
    match sub.frames.recv().unwrap() {
        TerminalFrame::Output { seq, bytes } => {
            assert_eq!(seq, seq_before + 1);
            assert!(bytes.len() <= 1024 * 1024);
            let wire = String::from_utf8_lossy(&bytes);
            assert!(wire.contains("i=1,"));
            assert!(wire.contains("i=65,"));
            assert!(!wire.contains("a=q"));
        }
        other => panic!("expected coalesced Output frame without Resync, got {other:?}"),
    }
    let model = core.sessions.terminal_model(&id).unwrap();
    let g = model.graphics().lock().unwrap();
    for i in 1..=65u32 {
        assert!(g.primary.images().contains_key(&i), "missing image i={i}");
    }
    drop(g);
    drop(sub);
    core.sessions.kill(&id).unwrap();
    core.sessions.drain_terminal_workers();
}
