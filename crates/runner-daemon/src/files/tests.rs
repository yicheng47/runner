use std::collections::BTreeSet;
use std::path::Path;
use std::sync::{Arc, RwLock};
use std::time::{Duration, Instant};

use runner_core::protocol::files::{
    EntryKind, FileContent, FileStatus, FilesRootOutcome, FilesRootSource, FilesSubject, GitStatus,
    StatusEntry, WorktreeKind, FILES_CHANGED_EVENT, FILES_STATUS_EVENT,
};
use runner_core::protocol::DaemonClient;
use tokio::sync::broadcast;

use super::fixture::{git, repo};
use super::FilesService;
use crate::events::{AppEvent, EventChannel};
use crate::shell_path::LoginShellEnv;

fn service() -> (FilesService, broadcast::Receiver<AppEvent>) {
    let events = EventChannel::new();
    let rx = events.subscribe();
    (
        FilesService::new(Arc::new(RwLock::new(LoginShellEnv::default())), events),
        rx,
    )
}

fn entry(path: &str, status: FileStatus) -> StatusEntry {
    StatusEntry {
        path: path.into(),
        status,
    }
}

fn sorted(status: &GitStatus) -> Vec<StatusEntry> {
    let mut entries = status.entries.clone();
    entries.sort_by(|a, b| a.path.cmp(&b.path));
    entries
}

/// Paths from `git status --porcelain=v1 -z`, an independent format.
fn porcelain_v1_paths(dir: &Path) -> BTreeSet<String> {
    let output = git(
        dir,
        &["status", "--porcelain=v1", "-z", "--untracked-files=all"],
    );
    let mut paths = BTreeSet::new();
    let mut fields = output.split('\0');
    while let Some(field) = fields.next() {
        if field.len() < 4 {
            continue;
        }
        paths.insert(field[3..].to_owned());
        if field.starts_with('R') || field.starts_with('C') {
            fields.next();
        }
    }
    paths
}

#[test]
fn a_fixture_repository_reports_what_git_status_reports() {
    let fixture = repo();
    let path = fixture.path();
    std::fs::write(path.join("a.txt"), "a\nchanged\n").unwrap();
    git(path, &["mv", "old.txt", "new.txt"]);
    std::fs::write(path.join("added.txt"), "new\n").unwrap();
    git(path, &["add", "added.txt"]);
    std::fs::write(path.join("src/untracked file.rs"), "u\n").unwrap();
    std::fs::write(path.join("bin.dat"), [0u8, 9, 9]).unwrap();
    std::fs::create_dir_all(path.join("target/debug")).unwrap();
    std::fs::write(path.join("target/debug/out"), "o").unwrap();
    std::fs::write(path.join("z.log"), "l").unwrap();

    let (files, _rx) = service();
    let status = files.status(path).unwrap();
    assert_eq!(
        sorted(&status),
        vec![
            entry("a.txt", FileStatus::Modified),
            entry("added.txt", FileStatus::Added),
            entry("bin.dat", FileStatus::Modified),
            entry(
                "new.txt",
                FileStatus::Renamed {
                    from: "old.txt".into()
                }
            ),
            entry("src/untracked file.rs", FileStatus::Untracked),
        ]
    );
    assert_eq!(
        status
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<BTreeSet<_>>(),
        porcelain_v1_paths(path)
    );
    assert_eq!(status.ignored, ["target/", "z.log"]);
    assert_eq!(status.changed, 5);
    assert_eq!(status.rollup().into_iter().collect::<Vec<_>>(), ["src"]);

    let entries = files.list(path, "").unwrap();
    let ignored = |name: &str| {
        entries
            .iter()
            .find(|entry| entry.name == name)
            .unwrap_or_else(|| panic!("{name} not listed"))
            .ignored
    };
    assert!(ignored("target"));
    assert!(ignored("z.log"));
    assert!(!ignored("src"));
    assert!(!entries.iter().any(|entry| entry.name == ".git"));
    assert!(matches!(
        super::read(path, "bin.dat", false).unwrap(),
        FileContent::Binary {
            size: 3,
            bytes: None
        }
    ));
    assert!(files.list(path, "../").is_err());
}

#[test]
fn a_worktree_under_dot_worktrees_keeps_its_own_status() {
    let fixture = repo();
    let main = fixture.path();
    git(
        main,
        &["worktree", "add", "-q", ".worktrees/feat-x", "-b", "feat/x"],
    );
    let worktree = main.join(".worktrees/feat-x");
    std::fs::write(worktree.join("a.txt"), "a\nin the worktree\n").unwrap();
    std::fs::write(worktree.join("wt-only.txt"), "w\n").unwrap();
    std::fs::write(main.join("src/main-only.rs"), "m\n").unwrap();

    let (files, _rx) = service();
    let resolved = super::root::resolve(files.git().as_ref(), None, Some(&worktree));
    let FilesRootOutcome::Root(root) = resolved.outcome else {
        panic!("expected a root");
    };
    assert_eq!(root.git.unwrap().worktree, WorktreeKind::Linked);
    let worktree_status = files.status(&root.path).unwrap();
    assert_eq!(
        sorted(&worktree_status),
        vec![
            entry("a.txt", FileStatus::Modified),
            entry("wt-only.txt", FileStatus::Untracked),
        ]
    );
    assert_eq!(
        worktree_status
            .entries
            .iter()
            .map(|entry| entry.path.clone())
            .collect::<BTreeSet<_>>(),
        porcelain_v1_paths(&worktree)
    );

    let worktrees = files.worktrees(main).unwrap();
    let summary = worktrees
        .iter()
        .map(|worktree| (worktree.kind, worktree.branch.as_deref()))
        .collect::<Vec<_>>();
    assert_eq!(
        summary,
        [
            (WorktreeKind::Main, Some("main")),
            (WorktreeKind::Linked, Some("feat/x"))
        ]
    );
    assert_eq!(
        worktrees[1].path.canonicalize().unwrap(),
        worktree.canonicalize().unwrap()
    );

    let main_status = files.status(main).unwrap();
    assert_eq!(
        sorted(&main_status),
        vec![entry("src/main-only.rs", FileStatus::Untracked)]
    );
    assert!(main_status.ignored.contains(&".worktrees/".to_owned()));
    let listing = files.list(main, "").unwrap();
    let dot_worktrees = listing
        .iter()
        .find(|entry| entry.name == ".worktrees")
        .unwrap();
    assert_eq!(dot_worktrees.kind, EntryKind::Folder);
    assert!(dot_worktrees.ignored);
    assert!(files.list(main, ".worktrees/feat-x").unwrap()[0].ignored);
}

#[test]
fn a_plain_folder_has_an_empty_status_and_no_worktrees() {
    let plain = tempfile::tempdir().unwrap();
    std::fs::write(plain.path().join("notes.txt"), "n").unwrap();
    let (files, _rx) = service();
    assert_eq!(files.status(plain.path()).unwrap(), GitStatus::default());
    assert!(files.worktrees(plain.path()).unwrap().is_empty());
    assert!(!files.list(plain.path(), "").unwrap()[0].ignored);
}

fn insert_project(conn: &rusqlite::Connection, id: &str, cwd: &Path) {
    conn.execute(
        "INSERT INTO projects (id, name, cwd, position, created_at)
         VALUES (?1, 'Project', ?2, 0, '2026-11-04T00:00:00Z')",
        rusqlite::params![id, cwd.to_str().unwrap()],
    )
    .unwrap();
}

fn insert_session(conn: &rusqlite::Connection, id: &str, project: Option<&str>, cwd: &Path) {
    conn.execute(
        "INSERT INTO sessions (id, status, project_id, cwd) VALUES (?1, 'stopped', ?2, ?3)",
        rusqlite::params![id, project, cwd.to_str().unwrap()],
    )
    .unwrap();
}

fn insert_mission(conn: &rusqlite::Connection, id: &str, cwd: &Path) {
    conn.execute(
        "INSERT INTO missions (id, title, status, started_at, cwd)
         VALUES (?1, 'M', 'running', '2026-11-04T00:00:00Z', ?2)",
        rusqlite::params![id, cwd.to_str().unwrap()],
    )
    .unwrap();
}

fn own_or_saved(outcome: FilesRootOutcome) -> (FilesRootSource, std::path::PathBuf) {
    match outcome {
        FilesRootOutcome::Root(root) => (root.source, root.path),
        other => panic!("expected a root, got {other:?}"),
    }
}

fn same(a: &Path, b: &Path) -> bool {
    a.canonicalize().unwrap() == b.canonicalize().unwrap()
}

#[test]
fn a_saved_root_round_trips_per_session_and_mission() {
    let fixture = repo();
    let core = crate::test_support::test_core();
    {
        let conn = core.db.get().unwrap();
        insert_project(&conn, "p", fixture.path());
        insert_session(&conn, "chat", Some("p"), &fixture.path().join("src"));
        insert_session(&conn, "loose", None, fixture.path());
        insert_mission(&conn, "m", fixture.path());
    }
    let client: DaemonClient = crate::daemon::InProcessTransport::client(core.clone());
    let chat = FilesSubject::Session("chat".into());
    let mission = FilesSubject::Mission("m".into());

    let (source, path) = own_or_saved(client.files_root(&chat).unwrap());
    assert_eq!(source, FilesRootSource::Own);
    assert!(same(&path, fixture.path()));
    assert_eq!(
        client
            .files_root(&FilesSubject::Session("loose".into()))
            .unwrap(),
        FilesRootOutcome::ChooseFolder
    );
    assert!(client
        .files_root(&FilesSubject::Session("nope".into()))
        .is_err());

    let chosen = fixture.path().join("src").join("deep");
    let (source, path) = own_or_saved(client.files_set_root(&chat, Some(&chosen)).unwrap());
    assert_eq!(
        (source, path.as_path()),
        (FilesRootSource::Saved, chosen.as_path())
    );
    assert_eq!(
        own_or_saved(client.files_root(&chat).unwrap()),
        (FilesRootSource::Saved, chosen.clone())
    );
    assert_eq!(
        own_or_saved(client.files_root(&mission).unwrap()).0,
        FilesRootSource::Own,
        "the chat's root is its own"
    );

    let other = tempfile::tempdir().unwrap();
    client.files_set_root(&mission, Some(other.path())).unwrap();
    assert_eq!(
        client.files_recent_folders().unwrap(),
        [other.path().to_path_buf(), chosen.clone()]
    );
    assert!(client
        .files_set_root(&mission, Some(&fixture.path().join("missing")))
        .is_err());
    assert!(client
        .files_set_root(&mission, Some(Path::new("relative")))
        .is_err());

    let (source, _) = own_or_saved(client.files_set_root(&chat, None).unwrap());
    assert_eq!(source, FilesRootSource::Own);

    drop(other);
    let (source, path) = own_or_saved(client.files_root(&mission).unwrap());
    assert_eq!(source, FilesRootSource::Own);
    assert!(same(&path, fixture.path()));
    assert_eq!(
        crate::repo::mission::files_root(&core.db.get().unwrap(), "m").unwrap(),
        None,
        "a saved root that is gone is forgotten"
    );
    assert_eq!(client.files_recent_folders().unwrap(), [chosen]);
}

#[test]
fn recent_folders_keep_the_last_ten_newest_first() {
    let core = crate::test_support::test_core();
    let dirs = (0..12)
        .map(|_| tempfile::tempdir().unwrap())
        .collect::<Vec<_>>();
    for dir in &dirs {
        crate::db::push_files_recent_folder(&core.db, dir.path()).unwrap();
    }
    crate::db::push_files_recent_folder(&core.db, dirs[5].path()).unwrap();
    let recent = super::recent_folders(&core).unwrap();
    let expected = [5, 11, 10, 9, 8, 7, 6, 4, 3, 2]
        .iter()
        .map(|index| dirs[*index].path().to_path_buf())
        .collect::<Vec<_>>();
    assert_eq!(recent, expected);
}

#[test]
fn watches_are_shared_and_released_with_their_last_subscriber() {
    let plain = tempfile::tempdir().unwrap();
    let root = plain.path().to_path_buf();
    let (files, _rx) = service();
    files.watch(1, root.clone()).unwrap();
    files.watch(1, root.clone()).unwrap();
    files.watch(2, root.clone()).unwrap();
    assert_eq!(files.watched_roots(), std::slice::from_ref(&root));
    files.unwatch(1, &root);
    files.client_gone(1);
    assert_eq!(files.watched_roots(), std::slice::from_ref(&root));
    files.unwatch(2, &root);
    assert!(files.watched_roots().is_empty());

    files.watch(3, root.clone()).unwrap();
    files.client_gone(3);
    assert!(files.watched_roots().is_empty());
    assert!(files.watch(3, root.join("missing")).is_err());
}

fn is_for(event: &AppEvent, root: &Path) -> bool {
    event.payload["root"] == serde_json::json!(root)
}

fn next_event(rx: &mut broadcast::Receiver<AppEvent>, root: &Path, name: &str) -> AppEvent {
    let deadline = Instant::now() + Duration::from_secs(15);
    while Instant::now() < deadline {
        match rx.try_recv() {
            Ok(event) if event.name == name && is_for(&event, root) => return event,
            Ok(_) => {}
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    panic!("no {name} for {}", root.display());
}

fn events_during(
    rx: &mut broadcast::Receiver<AppEvent>,
    root: &Path,
    window: Duration,
) -> Vec<AppEvent> {
    let deadline = Instant::now() + window;
    let mut events = Vec::new();
    while Instant::now() < deadline {
        match rx.try_recv() {
            Ok(event) if is_for(&event, root) => events.push(event),
            Ok(_) => {}
            Err(_) => std::thread::sleep(Duration::from_millis(20)),
        }
    }
    events
}

fn status_of(event: &AppEvent) -> GitStatus {
    serde_json::from_value(event.payload["status"].clone()).unwrap()
}

/// The watcher's first status, then a pause for the platform's watch to
/// settle and anything it reports from before it started.
fn watch_ready(
    files: &FilesService,
    rx: &mut broadcast::Receiver<AppEvent>,
    root: &Path,
) -> GitStatus {
    files.watch(1, root.to_path_buf()).unwrap();
    let status = status_of(&next_event(rx, root, FILES_STATUS_EVENT));
    events_during(rx, root, Duration::from_millis(700));
    status
}

#[test]
fn an_edit_gives_one_change_and_one_status() {
    let fixture = repo();
    let root = fixture.path();
    let (files, mut rx) = service();
    assert_eq!(watch_ready(&files, &mut rx, root), GitStatus::default());

    std::fs::write(root.join("a.txt"), "a\nedited\n").unwrap();
    let events = events_during(
        &mut rx,
        root,
        super::watch::MAX_WAIT + Duration::from_secs(1),
    );
    let names = events.iter().map(|event| event.name).collect::<Vec<_>>();
    assert_eq!(
        names,
        [FILES_CHANGED_EVENT, FILES_STATUS_EVENT],
        "{events:?}"
    );
    assert_eq!(events[0].payload["paths"], serde_json::json!(["a.txt"]));
    assert_eq!(events[0].payload["folders"], serde_json::json!([""]));
    assert_eq!(
        status_of(&events[1]).entries,
        [entry("a.txt", FileStatus::Modified)]
    );
}

#[test]
fn a_build_into_an_ignored_folder_runs_no_status() {
    let fixture = repo();
    let root = fixture.path();
    std::fs::create_dir_all(root.join("target/debug")).unwrap();
    std::fs::write(root.join("target/debug/keep"), "k").unwrap();
    let (files, mut rx) = service();
    let status = watch_ready(&files, &mut rx, root);
    assert_eq!(status.ignored, ["target/"]);
    let runs = files.status_runs(root);

    for index in 0..10_000 {
        let folder = root.join(format!("target/debug/deps/{}", index % 100));
        if index < 100 {
            std::fs::create_dir_all(&folder).unwrap();
        }
        std::fs::write(folder.join(format!("{index}.o")), "object").unwrap();
    }
    let events = events_during(
        &mut rx,
        root,
        super::watch::MAX_WAIT + Duration::from_secs(1),
    );
    assert!(events.is_empty(), "{events:?}");
    // Windows also runs its unchanging safety-net status on a timer.
    #[cfg(not(windows))]
    assert_eq!(files.status_runs(root), runs);
    #[cfg(windows)]
    let _ = runs;
}

#[test]
fn a_commit_in_a_linked_worktree_refreshes_its_status() {
    let fixture = repo();
    git(
        fixture.path(),
        &["worktree", "add", "-q", ".worktrees/feat-x", "-b", "feat/x"],
    );
    let worktree = fixture.path().join(".worktrees/feat-x");
    std::fs::write(worktree.join("a.txt"), "a\nstaged\n").unwrap();
    git(&worktree, &["add", "a.txt"]);
    let (files, mut rx) = service();
    assert_eq!(
        watch_ready(&files, &mut rx, &worktree).entries,
        [entry("a.txt", FileStatus::Modified)]
    );

    git(&worktree, &["commit", "-q", "-m", "commit in the worktree"]);
    assert_eq!(
        status_of(&next_event(&mut rx, &worktree, FILES_STATUS_EVENT)),
        GitStatus::default()
    );
}

/// A `git` that logs each run to `runs`; while `delay` exists, holds the
/// first `status` output for two seconds after git has read the tree,
/// marking that moment with `inspected`; and while `old` exists, refuses
/// `worktree list -z` as a git before 2.36 does.
#[cfg(unix)]
fn wrapped_git() -> (
    tempfile::TempDir,
    FilesService,
    broadcast::Receiver<AppEvent>,
) {
    use std::os::unix::fs::PermissionsExt;
    let real = std::process::Command::new("sh")
        .args(["-c", "command -v git"])
        .output()
        .unwrap();
    let real = String::from_utf8(real.stdout).unwrap().trim().to_owned();
    let dir = tempfile::tempdir().unwrap();
    let d = dir.path().display();
    let script = format!(
        "#!/bin/sh\n\
         echo \"$*\" >> \"{d}/runs\"\n\
         if [ \"$1\" = worktree ] && [ \"$4\" = -z ] && [ -e \"{d}/old\" ]; then\n\
         echo \"error: unknown switch z\" >&2; exit 129\n\
         fi\n\
         if [ \"$2\" = status ] && [ -e \"{d}/delay\" ] && mkdir \"{d}/delayed\" 2>/dev/null; then\n\
         \"{real}\" \"$@\" > \"{d}/out\"; code=$?\n\
         touch \"{d}/inspected\"; sleep 2; cat \"{d}/out\"; exit $code\n\
         fi\n\
         exec \"{real}\" \"$@\"\n"
    );
    let git = dir.path().join("git");
    std::fs::write(&git, script).unwrap();
    std::fs::set_permissions(&git, std::fs::Permissions::from_mode(0o755)).unwrap();
    let events = EventChannel::new();
    let rx = events.subscribe();
    let env = LoginShellEnv {
        path: Some(dir.path().display().to_string()),
        ..Default::default()
    };
    let files = FilesService::new(Arc::new(RwLock::new(env)), events);
    (dir, files, rx)
}

#[cfg(unix)]
fn runs(wrapper: &Path) -> usize {
    std::fs::read_to_string(wrapper.join("runs"))
        .map(|runs| runs.lines().count())
        .unwrap_or(0)
}

/// Makes `change` while the watcher's first status is held after git has
/// read the tree, and waits for a status showing `expected`.
#[cfg(unix)]
fn change_during_first_status(
    prepare: impl Fn(&Path),
    change: impl Fn(&Path),
    expected: Vec<StatusEntry>,
) {
    let fixture = repo();
    let root = fixture.path();
    prepare(root);
    let (wrapper, files, mut rx) = wrapped_git();
    std::fs::write(wrapper.path().join("delay"), "").unwrap();
    files.watch(1, root.to_path_buf()).unwrap();
    let deadline = Instant::now() + Duration::from_secs(15);
    while !wrapper.path().join("inspected").exists() {
        assert!(Instant::now() < deadline, "the first status never ran");
        std::thread::sleep(Duration::from_millis(10));
    }
    change(root);
    let deadline = Instant::now() + Duration::from_secs(15);
    loop {
        assert!(Instant::now() < deadline, "the change was never reported");
        let status = status_of(&next_event(&mut rx, root, FILES_STATUS_EVENT));
        if sorted(&status) == expected {
            break;
        }
    }
}

#[cfg(unix)]
#[test]
fn an_edit_while_the_first_status_runs_is_not_missed() {
    change_during_first_status(
        |_| {},
        |root| std::fs::write(root.join("a.txt"), "a\nedited while git ran\n").unwrap(),
        vec![entry("a.txt", FileStatus::Modified)],
    );
}

#[cfg(unix)]
#[test]
fn an_index_change_while_the_first_status_runs_is_not_missed() {
    change_during_first_status(
        |root| std::fs::write(root.join("new.txt"), "n\n").unwrap(),
        |root| drop(git(root, &["add", "new.txt"])),
        vec![entry("new.txt", FileStatus::Added)],
    );
}

#[cfg(unix)]
#[test]
fn each_request_runs_git_at_most_once() {
    let fixture = repo();
    let root = fixture.path();
    git(
        root,
        &["worktree", "add", "-q", ".worktrees/feat-x", "-b", "feat/x"],
    );
    let worktree = root.join(".worktrees/feat-x");
    let plain = tempfile::tempdir().unwrap();
    let (wrapper, files, _rx) = wrapped_git();
    // On a git before 2.36 the service's first worktree list costs one
    // approved retry, which an_old_git_lists_worktrees_with_one_retry_then_one_run
    // asserts; warm it so every count below holds on any git.
    files.worktrees(root).unwrap();
    let git = files.git();
    let counted = |request: &dyn Fn()| {
        let before = runs(wrapper.path());
        request();
        runs(wrapper.path()) - before
    };

    let resolve = |dir: &Path| {
        let FilesRootOutcome::Root(root) =
            super::root::resolve(git.as_ref(), None, Some(dir)).outcome
        else {
            panic!("expected a root");
        };
        root
    };
    assert_eq!(counted(&|| assert!(resolve(&worktree).git.is_some())), 1);
    assert_eq!(
        counted(&|| assert!(resolve(&root.join("src")).git.is_some())),
        1
    );
    assert_eq!(counted(&|| assert!(resolve(plain.path()).git.is_none())), 0);
    assert_eq!(counted(&|| drop(files.status(&worktree).unwrap())), 1);
    assert_eq!(
        counted(&|| drop(files.status(&root.join("src")).unwrap())),
        1
    );
    assert_eq!(counted(&|| drop(files.status(plain.path()).unwrap())), 0);
    assert_eq!(
        counted(&|| assert_eq!(files.worktrees(root).unwrap().len(), 2)),
        1
    );
    assert_eq!(
        counted(&|| assert!(files.worktrees(plain.path()).unwrap().is_empty())),
        0
    );
    assert_eq!(counted(&|| drop(files.list(&worktree, "").unwrap())), 0);
    assert_eq!(
        counted(&|| drop(super::read(root, "a.txt", false).unwrap())),
        0
    );
}

#[test]
fn listing_runs_no_status_and_survives_a_broken_index() {
    let fixture = repo();
    let root = fixture.path();
    std::fs::write(root.join(".git/index"), "not an index").unwrap();
    std::fs::create_dir_all(root.join("target")).unwrap();
    let (files, _rx) = service();
    let entries = files.list(root, "").unwrap();
    let names = entries
        .iter()
        .map(|entry| (entry.name.as_str(), entry.ignored))
        .collect::<Vec<_>>();
    assert_eq!(
        names,
        [
            ("src", false),
            ("target", false),
            (".gitignore", false),
            ("a.txt", false),
            ("bin.dat", false),
            ("old.txt", false),
        ]
    );
    assert!(files.status(root).is_err());
    assert!(files.list(root, "src").is_ok());
}

#[test]
fn an_ignored_root_marks_everything_beneath_it() {
    let fixture = repo();
    let path = fixture.path();
    std::fs::create_dir_all(path.join("target/debug/deps")).unwrap();
    std::fs::write(path.join("target/a.txt"), "a").unwrap();
    std::fs::write(path.join("target/debug/b"), "b").unwrap();
    let (files, _rx) = service();
    for root in [path.join("target"), path.join("target/debug")] {
        let status = files.status(&root).unwrap();
        assert!(status.root_ignored, "{}", root.display());
        assert!(status.ignored.is_empty());
        let entries = files.list(&root, "").unwrap();
        assert!(!entries.is_empty());
        assert!(entries.iter().all(|entry| entry.ignored), "{entries:?}");
    }
    assert!(!files.status(path).unwrap().root_ignored);
}

#[test]
fn an_ignored_root_s_churn_is_dropped() {
    let fixture = repo();
    let root = fixture.path().join("target");
    std::fs::create_dir_all(root.join("debug")).unwrap();
    std::fs::write(root.join("debug/keep"), "k").unwrap();
    let (files, mut rx) = service();
    assert!(watch_ready(&files, &mut rx, &root).root_ignored);
    let runs = files.status_runs(&root);
    for index in 0..500 {
        std::fs::write(root.join(format!("debug/{index}.o")), "object").unwrap();
    }
    let events = events_during(
        &mut rx,
        &root,
        super::watch::MAX_WAIT + Duration::from_secs(1),
    );
    assert!(events.is_empty(), "{events:?}");
    #[cfg(not(windows))]
    assert_eq!(files.status_runs(&root), runs);
    #[cfg(windows)]
    let _ = runs;
}

#[cfg(unix)]
#[test]
fn symlinked_roots_see_the_status_git_reports_through_them() {
    let fixture = repo();
    let path = fixture.path();
    std::fs::write(path.join("src/deep/x.rs"), "fn x() { changed }\n").unwrap();
    std::fs::write(path.join("src/new.rs"), "n\n").unwrap();
    std::os::unix::fs::symlink("src", path.join("internal-link")).unwrap();
    let outside = tempfile::tempdir().unwrap();
    let external = outside.path().join("external-link");
    std::os::unix::fs::symlink(path.join("src"), &external).unwrap();
    let expected = vec![
        entry("deep/x.rs", FileStatus::Modified),
        entry("new.rs", FileStatus::Untracked),
    ];
    let (files, mut rx) = service();
    let git = files.git();
    for root in [path.join("internal-link"), external.clone()] {
        assert_eq!(
            sorted(&files.status(&root).unwrap()),
            expected,
            "{}",
            root.display()
        );
        assert_eq!(files.worktrees(&root).unwrap().len(), 1);

        let saved = super::root::resolve(git.as_ref(), Some(&root), None).outcome;
        let FilesRootOutcome::Root(saved) = saved else {
            panic!("expected a root for {}", root.display());
        };
        assert_eq!(saved.path, root);
        assert!(same(&saved.git.unwrap().top, path));

        let own = super::root::resolve(git.as_ref(), None, Some(&root)).outcome;
        let FilesRootOutcome::Root(own) = own else {
            panic!("expected a root for {}", root.display());
        };
        assert!(same(&own.path, path));
        assert_eq!(own.cwd.as_deref(), Some("src"));
    }
    let watched = path.join("internal-link");
    assert_eq!(sorted(&watch_ready(&files, &mut rx, &watched)), expected);
}

#[cfg(unix)]
#[test]
fn an_old_git_lists_worktrees_with_one_retry_then_one_run() {
    let fixture = repo();
    let root = fixture.path();
    git(
        root,
        &["worktree", "add", "-q", ".worktrees/feat x", "-b", "feat/x"],
    );
    let (wrapper, files, _rx) = wrapped_git();
    std::fs::write(wrapper.path().join("old"), "").unwrap();
    let counted = |expected_runs: usize| {
        let before = runs(wrapper.path());
        let worktrees = files.worktrees(root).unwrap();
        assert_eq!(runs(wrapper.path()) - before, expected_runs);
        worktrees
            .into_iter()
            .map(|worktree| (worktree.kind, worktree.branch))
            .collect::<Vec<_>>()
    };
    let expected = vec![
        (WorktreeKind::Main, Some("main".to_owned())),
        (WorktreeKind::Linked, Some("feat/x".to_owned())),
    ];
    assert_eq!(
        counted(2),
        expected,
        "the first list is refused and retried"
    );
    assert_eq!(counted(1), expected);
    let FilesRootOutcome::Root(root) = super::root::resolve(
        files.git().as_ref(),
        None,
        Some(&root.join(".worktrees/feat x")),
    )
    .outcome
    else {
        panic!("expected a root");
    };
    assert_eq!(root.git.unwrap().branch.as_deref(), Some("feat/x"));
}
