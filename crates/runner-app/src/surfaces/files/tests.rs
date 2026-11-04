use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use gpui::{
    div, prelude::*, px, size, Context, Entity, Modifiers, TestAppContext, VisualTestContext,
    Window, WindowHandle,
};
use runner_core::protocol::files::{FilesRootSource, WorktreeKind};

use super::{FileOpen, FilesPanel, FilesView};
use crate::surfaces::side_panel::{PanelContext, PanelSubject, PanelSurface};
use crate::theme::{self, ThemeVariant};
use crate::theme_snapshot::{assert_border, assert_fill_color, assert_text, ThemeGuard};

struct Host {
    panel: FilesPanel,
    context: Option<PanelContext>,
    /// A view drawn without the panel, so never shown and never watching.
    unshown: Option<Entity<FilesView>>,
}

impl Render for Host {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(view) = self.unshown.clone() {
            return div().size_full().flex().flex_col().child(view);
        }
        let body = match &self.context {
            Some(context) => Some(self.panel.show(context, cx)),
            None => {
                self.panel.hide(cx);
                None
            }
        };
        div().size_full().flex().flex_col().children(body)
    }
}

fn git(dir: &Path, args: &[&str]) {
    let output = Command::new("git")
        .current_dir(dir)
        .args([
            "-c",
            "user.name=Runner",
            "-c",
            "user.email=runner@example.com",
        ])
        .args(["-c", "commit.gpgsign=false"])
        .args(args)
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .expect("run git");
    assert!(
        output.status.success(),
        "git {args:?}: {}",
        String::from_utf8_lossy(&output.stderr)
    );
}

/// A committed repository with `src/deep/x.rs`, `a.txt`, `old.txt` and a
/// `.gitignore` of `target/`, `.worktrees/` and `*.log`.
fn repo(dir: &Path) {
    std::fs::create_dir_all(dir.join("src/deep")).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "core.autocrlf", "false"]);
    std::fs::write(dir.join(".gitignore"), "target/\n.worktrees/\n*.log\n").unwrap();
    std::fs::write(dir.join("src/deep/x.rs"), "fn x() {}\n").unwrap();
    std::fs::write(dir.join("a.txt"), "a\n").unwrap();
    std::fs::write(dir.join("old.txt"), "old\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "init"]);
}

/// `path` as the tests write it to the database: the real folder on Unix,
/// where macOS's temporary folder is a symlink, and as is on Windows, where
/// a canonical path is a `\\?\` path git does not report.
fn real(path: &Path) -> PathBuf {
    if cfg!(windows) {
        path.to_path_buf()
    } else {
        path.canonicalize().unwrap()
    }
}

/// Paths from git and paths from the tests name the same folder.
#[track_caller]
fn assert_same(actual: Option<impl AsRef<Path>>, expected: &Path) {
    let actual = actual.expect("a path");
    assert_eq!(
        actual.as_ref().canonicalize().unwrap(),
        expected.canonicalize().unwrap(),
        "{:?}",
        actual.as_ref()
    );
}

struct Harness {
    temp: tempfile::TempDir,
    repo: PathBuf,
    _cx: TestAppContext,
    store: Entity<crate::app_store::AppStore>,
    host: WindowHandle<Host>,
    visual: VisualTestContext,
}

fn sql(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

/// A store whose database holds the project `runner` on a fixture repository,
/// a chat `in-project` working in its `src`, a chat `outside` with no project
/// and a mission `on-worktree` working in a linked worktree of it.
fn harness(context: Option<PanelContext>) -> Harness {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = temp.path().join("runner");
    repo(&repo_path);
    let worktree = repo_path.join(".worktrees/feat-x");
    git(
        &repo_path,
        &["worktree", "add", "-q", "-b", "feat/x", ".worktrees/feat-x"],
    );
    // Canonical, as git reports it and as the daemon resolves roots.
    let repo_path = real(&repo_path);
    let worktree = real(&worktree);
    let mut cx = TestAppContext::single();
    let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
    store.update(&mut cx, |store, cx| {
        store
            .test_core
            .db
            .get()
            .unwrap()
            .execute_batch(&format!(
                "INSERT INTO projects (id, name, cwd, position, created_at)
                 VALUES ('runner', 'runner', '{repo}', 0, '2026-11-04T00:00:00Z');
                 INSERT INTO sessions (id, status, agent_runtime, agent_command, cwd, project_id)
                 VALUES ('in-project', 'stopped', 'shell', 'shell', '{src}', 'runner'),
                        ('outside', 'stopped', 'shell', 'shell', '{home}', NULL);
                 INSERT INTO missions (id, title, status, started_at, cwd, project_id)
                 VALUES ('on-worktree', 'M', 'running', '2026-11-04T00:00:00Z', '{worktree}', 'runner'),
                        ('gone', 'G', 'running', '2026-11-04T00:00:00Z', '{gone}', 'runner');",
                repo = sql(&repo_path),
                src = sql(&repo_path.join("src")),
                home = sql(temp.path()),
                worktree = sql(&worktree),
                gone = sql(&repo_path.join(".worktrees/merged")),
            ))
            .unwrap();
        store.refresh(crate::app_store::StoreRefreshKind::All, cx);
    });
    let host = cx.add_window(|_, _| Host {
        panel: FilesPanel::default(),
        context,
        unshown: None,
    });
    let visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(340.), px(900.)));
    let mut harness = Harness {
        temp,
        repo: repo_path,
        _cx: cx,
        store,
        host,
        visual,
    };
    harness.settle();
    harness
}

fn chat(id: &str) -> PanelContext {
    PanelContext {
        subject: PanelSubject::Session(id.into()),
        surface: PanelSurface::Chat,
        cwd: None,
        project: (id == "in-project").then(|| "runner".into()),
        remote: false,
    }
}

fn mission(id: &str) -> PanelContext {
    PanelContext {
        subject: PanelSubject::Mission(id.into()),
        surface: PanelSurface::Mission,
        cwd: None,
        project: Some("runner".into()),
        remote: false,
    }
}

impl Harness {
    fn settle(&mut self) {
        self.visual.run_until_parked();
    }

    /// Runs the app until `ready`, for replies that come from git or the
    /// watcher's own threads.
    fn wait(&mut self, what: &str, mut ready: impl FnMut(&mut Self) -> bool) {
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            self.visual.run_until_parked();
            if ready(self) {
                return;
            }
            assert!(Instant::now() < deadline, "timed out waiting for {what}");
            std::thread::sleep(Duration::from_millis(20));
        }
    }

    fn has(&mut self, selector: impl Into<String>) -> bool {
        let selector: String = selector.into();
        self.visual.debug_bounds(selector.leak()).is_some()
    }

    fn click(&mut self, selector: impl Into<String>) {
        let selector: String = selector.into();
        let bounds = self
            .visual
            .debug_bounds(selector.clone().leak())
            .unwrap_or_else(|| panic!("missing {selector}"));
        self.visual
            .simulate_click(bounds.center(), Modifiers::default());
        self.settle();
    }

    fn view(&mut self) -> Entity<FilesView> {
        self.host
            .read_with(&self.visual, |host, _| {
                let subject = &host.context.as_ref().unwrap().subject;
                host.panel.view(subject)
            })
            .unwrap()
            .expect("a shown view")
    }

    fn read<T>(&mut self, read: impl FnOnce(&FilesView) -> T) -> T {
        let view = self.view();
        view.read_with(&self.visual, |view, _| read(view))
    }

    fn show(&mut self, context: Option<PanelContext>) {
        self.host
            .update(&mut self.visual, |host, _, cx| {
                host.context = context;
                cx.notify();
            })
            .unwrap();
        self.settle();
    }

    fn redraw(&mut self) {
        self.visual.update(|window, _| window.refresh());
        self.settle();
    }

    /// The roots the daemon's file service is watching.
    fn daemon_watches(&mut self) -> Vec<PathBuf> {
        self.store.read_with(&self.visual, |store, _| {
            store.test_core.files.watched_roots()
        })
    }

    fn status_read(&mut self) -> bool {
        !self.has("FILES_STATUS_READING") && self.has("FILES_ROOT_HEADER")
    }
}

#[test]
fn a_chat_in_a_project_opens_its_checkout_with_git_status() {
    let mut h = harness(Some(chat("in-project")));
    std::fs::write(h.repo.join("a.txt"), "changed\n").unwrap();
    std::fs::write(h.repo.join("src/new.rs"), "new\n").unwrap();
    std::fs::create_dir_all(h.repo.join("target/debug")).unwrap();
    h.show(None);
    h.show(Some(chat("in-project")));
    h.wait("the status", |h| h.has("FILES_LETTER a.txt"));

    assert_eq!(h.read(|view| view.state()), "root");
    let root = h.read(|view| view.root().cloned()).unwrap();
    assert_eq!(root.name, "runner");
    assert_eq!(root.git.as_ref().unwrap().worktree, WorktreeKind::Main);
    assert_eq!(root.cwd.as_deref(), Some("src"));
    for row in [
        ".worktrees",
        "src",
        "src/deep",
        "src/new.rs",
        "target",
        "a.txt",
    ] {
        assert!(h.has(format!("FILES_ROW {row}")), "{row}");
    }
    assert!(!h.has("FILES_ROW .git"));
    assert!(h.has("FILES_CWD_TAG"), "the working directory is tagged");
    assert!(h.has("FILES_LETTER src/new.rs"));
    assert!(h.has("FILES_DOT src"));
    assert!(!h.has("FILES_DOT target"));
    assert!(h.has("FILES_CHANGED 2"));
}

#[test]
fn status_letters_selection_and_dimmed_entries_follow_both_themes() {
    let _theme = ThemeGuard::new();
    let mut h = harness(Some(chat("in-project")));
    std::fs::write(h.repo.join("a.txt"), "changed\n").unwrap();
    std::fs::write(h.repo.join("src/new.rs"), "new\n").unwrap();
    std::fs::create_dir_all(h.repo.join("target")).unwrap();
    h.show(None);
    h.show(Some(chat("in-project")));
    h.wait("the status", |h| h.has("FILES_LETTER a.txt"));
    h.click("FILES_ROW a.txt");
    for variant in [ThemeVariant::Carbon, ThemeVariant::RunnerLight] {
        theme::set_active_variant(variant);
        h.redraw();
        assert_text(&mut h.visual, "FILES_LETTER M", theme::warning());
        assert_text(&mut h.visual, "FILES_LETTER U", theme::accent());
        assert_text(&mut h.visual, "FILES_ROW_IGNORED", theme::faint());
        assert_fill_color(
            &mut h.visual,
            "FILES_ROW_SELECTED",
            Some(theme::sidebar_selected()),
        );
        assert_border(&mut h.visual, "FILES_ROW_SELECTED", theme::faint());
    }
}

#[test]
fn reading_status_shows_until_the_first_status_arrives() {
    let mut h = harness(None);
    // Status arrives with the watch, which only a shown view holds.
    h.host
        .update(&mut h.visual, |host, _, cx| {
            host.unshown = Some(cx.new(|cx| FilesView::new(chat("in-project"), cx)));
            cx.notify();
        })
        .unwrap();
    h.wait("the root", |h| h.has("FILES_ROW a.txt"));
    assert!(h.has("FILES_STATUS_READING"));
    let view = h
        .host
        .read_with(&h.visual, |host, _| host.unshown.clone())
        .unwrap()
        .unwrap();
    view.update(&mut h.visual, |view, cx| view.set_shown(true, cx));
    h.wait("the status", |h| !h.has("FILES_STATUS_READING"));
    assert!(!h.has("FILES_CHANGED 0"), "a clean checkout shows no count");
}

#[test]
fn a_chat_outside_a_project_chooses_a_folder_and_keeps_it_across_restarts() {
    let mut h = harness(Some(chat("outside")));
    assert_eq!(h.read(|view| view.state()), "choose");
    assert!(h.has("FILES_CHOOSE"));
    assert!(h.has("FILES_CHOOSE files-choose-project 0"));
    h.click("FILES_CHOOSE files-choose-project 0");
    h.wait("the chosen root", |h| h.has("FILES_ROW a.txt"));
    let root = h.read(|view| view.root().cloned()).unwrap();
    assert_same(Some(&root.path), &h.repo);
    assert_eq!(root.source, FilesRootSource::Saved);

    // Switching away and back, then a fresh view as after a restart.
    h.show(Some(chat("in-project")));
    h.show(Some(chat("outside")));
    assert_same(
        h.read(|view| view.root().map(|root| root.path.clone())),
        &h.repo,
    );
    h.host
        .update(&mut h.visual, |host, _, _| {
            host.panel = FilesPanel::default()
        })
        .unwrap();
    h.redraw();
    h.wait("the saved root", |h| h.has("FILES_ROW a.txt"));
    assert_same(
        h.read(|view| view.root().map(|root| root.path.clone())),
        &h.repo,
    );
}

#[test]
fn a_typed_path_roots_the_tree_and_a_bare_name_asks_for_a_full_path() {
    let mut h = harness(Some(chat("outside")));
    let field = h.read(|view| view.choose_field.clone());
    let focus = field.read_with(&h.visual, |field, _| field.focus_handle());
    h.visual.update(|window, cx| focus.focus(window, cx));
    h.visual.simulate_input("runner");
    h.visual.simulate_keystrokes("enter");
    h.settle();
    assert!(h.has("FILES_ERROR"));
    let path = h.repo.join("src").to_string_lossy().into_owned();
    field.update(&mut h.visual, |field, cx| field.reset(path, cx));
    h.visual.update(|window, cx| focus.focus(window, cx));
    h.visual.simulate_keystrokes("enter");
    h.wait("the typed root", |h| h.has("FILES_ROW deep"));
    assert_same(
        h.read(|view| view.root().map(|root| root.path.clone())),
        &h.repo.join("src"),
    );
    assert!(!h.has("FILES_ERROR"));
}

#[test]
fn a_mission_opens_its_worktree_and_the_picker_roots_it_at_the_main_checkout() {
    let mut h = harness(Some(mission("on-worktree")));
    h.wait("the worktree", |h| h.has("FILES_ROW a.txt"));
    let root = h.read(|view| view.root().cloned()).unwrap();
    assert_eq!(root.name, "feat-x");
    assert_eq!(root.git.as_ref().unwrap().worktree, WorktreeKind::Linked);
    assert!(!h.has("FILES_ROW .worktrees"));

    h.click("FILES_ROOT_BUTTON");
    assert!(h.has("FILES_PICKER"));
    h.wait("the worktrees", |h| h.has("FILES_PICKER_ITEM 1"));
    let items = h.read(|view| view.picker.as_ref().unwrap().items.clone());
    assert_eq!(items[0].title, "runner", "the main checkout first");
    assert!(items[0].subtitle.starts_with("main · "));
    assert_eq!(items[1].title, "feat-x");
    assert!(items[1].current);
    assert!(items.last().unwrap().path.is_none(), "Choose folder… last");

    // Typing filters; the projects only list for a chat outside a project.
    h.visual.simulate_input("feat");
    h.settle();
    assert!(h.has("FILES_PICKER_ITEM 0"));
    assert!(h.has("FILES_PICKER_ITEM 1"), "feat-x and Choose folder…");
    assert!(!h.has("FILES_PICKER_ITEM 2"));
    h.visual.simulate_keystrokes("escape");
    h.settle();
    assert!(!h.has("FILES_PICKER"));

    h.click("FILES_ROOT_BUTTON");
    h.wait("the worktrees", |h| h.has("FILES_PICKER_ITEM 1"));
    h.click("FILES_PICKER_ITEM 0");
    h.wait("the main checkout", |h| h.has("FILES_ROW .worktrees"));
    assert_same(
        h.read(|view| view.root().map(|root| root.path.clone())),
        &h.repo,
    );
    h.wait("the status", |h| h.status_read());
    h.click("FILES_ROW .worktrees");
    let ignored = h.read(|view| {
        view.tree_ref()
            .unwrap()
            .rows()
            .iter()
            .filter(|row| row.path.starts_with(".worktrees"))
            .all(|row| row.ignored && row.letter.is_none())
    });
    assert!(ignored, ".worktrees is dimmed and carries no letters");
}

#[test]
fn enter_in_the_picker_applies_a_typed_path() {
    let mut h = harness(Some(mission("on-worktree")));
    h.wait("the worktree", |h| h.has("FILES_ROW a.txt"));
    h.click("FILES_ROOT_BUTTON");
    let path = h.repo.join("src").to_string_lossy().into_owned();
    h.visual.simulate_input(&path);
    h.visual.simulate_keystrokes("enter");
    h.wait("the typed root", |h| h.has("FILES_ROW deep"));
    assert!(!h.has("FILES_PICKER"));
}

#[test]
fn a_missing_checkout_offers_the_main_checkout() {
    let mut h = harness(Some(mission("gone")));
    assert_eq!(h.read(|view| view.state()), "missing");
    assert!(h.has("FILES_MISSING"));
    assert!(h.has("FILES_ROOT_BUTTON"), "the picker stays reachable");
    h.click("FILES_SHOW_MAIN");
    h.wait("the main checkout", |h| h.has("FILES_ROW a.txt"));
    assert_same(
        h.read(|view| view.root().map(|root| root.path.clone())),
        &h.repo,
    );
}

#[test]
fn a_remote_subject_never_asks_the_daemon() {
    let mut h = harness(Some(PanelContext {
        remote: true,
        ..chat("not-on-this-computer")
    }));
    assert!(h.has("FILES_REMOTE"));
    assert_eq!(h.read(|view| view.state()), "remote");
    assert_eq!(h.read(|view| view.watched().map(Path::to_path_buf)), None);
}

#[test]
fn a_plain_folder_has_no_status() {
    let mut h = harness(Some(chat("outside")));
    let plain = h.temp.path().join("notes");
    std::fs::create_dir_all(plain.join("drafts")).unwrap();
    std::fs::write(plain.join("todo.md"), "x").unwrap();
    let plain = real(&plain);
    let view = h.view();
    view.update(&mut h.visual, |view, cx| view.set_root(Some(plain), cx));
    h.wait("the plain root", |h| h.has("FILES_ROW todo.md"));
    assert!(h.read(|view| view.root().unwrap().git.is_none()));
    assert!(!h.has("FILES_STATUS_READING"));
    assert!(!h.has("FILES_CHANGED 0"));
}

#[test]
fn the_watch_follows_the_shown_view_and_its_root() {
    let mut h = harness(Some(chat("in-project")));
    h.wait("the root", |h| h.has("FILES_ROW a.txt"));
    let repo = h.repo.clone();
    assert_same(h.read(|view| view.watched().map(Path::to_path_buf)), &repo);

    let view = h.view();
    h.show(None);
    assert_eq!(
        view.read_with(&h.visual, |view, _| view.watched().map(Path::to_path_buf)),
        None
    );

    h.show(Some(chat("in-project")));
    assert_same(h.read(|view| view.watched().map(Path::to_path_buf)), &repo);
    let src = repo.join("src");
    view.update(&mut h.visual, |view, cx| {
        view.set_root(Some(src.clone()), cx)
    });
    h.wait("the new root", |h| h.has("FILES_ROW deep"));
    assert_same(h.read(|view| view.watched().map(Path::to_path_buf)), &src);

    // Another subject's view takes the watch over.
    h.show(Some(mission("on-worktree")));
    assert_eq!(
        view.read_with(&h.visual, |view, _| view.watched().map(Path::to_path_buf)),
        None
    );
}

#[test]
fn an_edit_on_disk_updates_the_tree_in_place() {
    let mut h = harness(Some(chat("in-project")));
    h.wait("the status", |h| {
        h.status_read() && h.has("FILES_ROW src/deep")
    });
    h.click("FILES_ROW src/deep");
    assert!(h.has("FILES_ROW src/deep/x.rs"));
    h.click("FILES_ROW src/deep/x.rs");
    let fresh = h.repo.join("src/deep/fresh.rs");
    let mut touched = Instant::now() - Duration::from_secs(1);
    // FSEvents can miss a write in the instant its stream starts, so write
    // again until the watcher reports it.
    h.wait("the watcher", |h| {
        if touched.elapsed() >= Duration::from_secs(1) {
            std::fs::write(&fresh, format!("{:?}\n", Instant::now())).unwrap();
            touched = Instant::now();
        }
        h.has("FILES_LETTER src/deep/fresh.rs")
    });
    let rows = h.read(|view| {
        view.tree_ref()
            .unwrap()
            .rows()
            .iter()
            .map(|row| row.path.clone())
            .collect::<Vec<_>>()
    });
    assert_eq!(
        rows[..5],
        [
            ".worktrees",
            "src",
            "src/deep",
            "src/deep/fresh.rs",
            "src/deep/x.rs"
        ],
        "expansion kept"
    );
    assert_eq!(h.read(|view| view.selected()), Some("src/deep/x.rs".into()));
    assert!(h.has("FILES_ROW_SELECTED"));
    assert!(h.has("FILES_CHANGED 1"));
}

#[test]
fn a_large_folder_draws_only_the_visible_rows() {
    let mut h = harness(Some(chat("outside")));
    let big = h.temp.path().join("big");
    std::fs::create_dir_all(&big).unwrap();
    for index in 0..3000 {
        std::fs::write(big.join(format!("f{index:04}.txt")), "").unwrap();
    }
    let big = real(&big);
    let view = h.view();
    view.update(&mut h.visual, |view, cx| view.set_root(Some(big), cx));
    h.wait("the listing", |h| h.has("FILES_ROW f0000.txt"));
    assert_eq!(h.read(|view| view.tree_ref().unwrap().rows().len()), 3000);
    assert!(!h.has("FILES_ROW f2999.txt"));
    assert!(!h.has("FILES_ROW f0100.txt"));
}

#[test]
fn the_keyboard_moves_opens_and_types_ahead() {
    let mut h = harness(Some(chat("in-project")));
    h.wait("the root", |h| h.has("FILES_ROW a.txt"));
    h.click("FILES_ROW .gitignore");
    assert_eq!(h.read(|view| view.selected()), Some(".gitignore".into()));
    h.visual.simulate_keystrokes("down");
    h.settle();
    assert_eq!(h.read(|view| view.selected()), Some("a.txt".into()));
    h.visual.simulate_keystrokes("o");
    h.settle();
    assert_eq!(h.read(|view| view.selected()), Some("old.txt".into()));

    let opened = std::rc::Rc::new(std::cell::RefCell::new(Vec::new()));
    let view = h.view();
    let seen = opened.clone();
    let _subscription = h.visual.update(|_, cx| {
        cx.subscribe(&view, move |_, event: &FileOpen, _| {
            seen.borrow_mut().push((event.path.clone(), event.keep))
        })
    });
    h.visual.simulate_keystrokes("space");
    h.visual.simulate_keystrokes("enter");
    h.settle();
    assert_eq!(
        *opened.borrow(),
        [("old.txt".to_owned(), false), ("old.txt".to_owned(), true)]
    );

    assert!(
        h.has("FILES_ROW src/deep"),
        "the working directory starts open"
    );
    h.click("FILES_ROW src");
    assert!(
        !h.has("FILES_ROW src/deep"),
        "a click closes an open folder"
    );
    h.visual.simulate_keystrokes("right");
    h.settle();
    assert!(h.has("FILES_ROW src/deep"), "→ opens it");
    h.visual.simulate_keystrokes("left");
    h.settle();
    assert!(!h.has("FILES_ROW src/deep"), "← closes it");
}

#[test]
fn each_root_state_draws_in_both_themes() {
    let _theme = ThemeGuard::new();
    for (context, selector) in [
        (chat("outside"), "FILES_CHOOSE"),
        (mission("gone"), "FILES_MISSING"),
        (
            PanelContext {
                remote: true,
                ..chat("elsewhere")
            },
            "FILES_REMOTE",
        ),
    ] {
        let mut h = harness(Some(context));
        for variant in [ThemeVariant::Carbon, ThemeVariant::RunnerLight] {
            theme::set_active_variant(variant);
            h.redraw();
            assert!(h.has(selector), "{selector}");
            assert_text(&mut h.visual, "FILES_TITLE", theme::text());
        }
    }
    let mut h = harness(Some(mission("gone")));
    for variant in [ThemeVariant::Carbon, ThemeVariant::RunnerLight] {
        theme::set_active_variant(variant);
        h.redraw();
        assert_text(&mut h.visual, "FILES_BODY", theme::muted());
    }
}

#[test]
fn right_clicking_a_folder_shows_files_from_there() {
    let mut h = harness(Some(chat("in-project")));
    h.wait("the root", |h| h.has("FILES_ROW a.txt"));
    let file = h.visual.debug_bounds("FILES_ROW a.txt").unwrap();
    h.visual.simulate_mouse_down(
        file.center(),
        gpui::MouseButton::Right,
        Modifiers::default(),
    );
    h.settle();
    assert!(h.has("CONTEXT_MENU_ITEM_1"), "Copy path and the reveal row");
    assert!(
        !h.has("CONTEXT_MENU_ITEM_2"),
        "no Show files from here on a file"
    );
    h.click("CONTEXT_MENU_ITEM_0");
    assert_same(
        h.visual.read_from_clipboard().and_then(|item| item.text()),
        &h.repo.join("a.txt"),
    );

    let folder = h.visual.debug_bounds("FILES_ROW src/deep").unwrap();
    h.visual.simulate_mouse_down(
        folder.center(),
        gpui::MouseButton::Right,
        Modifiers::default(),
    );
    h.settle();
    assert!(h.has("CONTEXT_MENU_ITEM_2"));
    h.click("CONTEXT_MENU_ITEM_0");
    h.wait("the new root", |h| h.has("FILES_ROW x.rs"));
    let root = h.read(|view| view.root().cloned()).unwrap();
    assert_same(Some(&root.path), &h.repo.join("src/deep"));
    assert_eq!(root.source, FilesRootSource::Saved);
}

#[test]
fn a_dropped_panel_releases_its_views_and_their_watch() {
    let mut h = harness(Some(chat("in-project")));
    h.wait("the root", |h| h.has("FILES_ROW a.txt"));
    let repo = h.repo.clone();
    h.wait("the daemon's watch", |h| h.daemon_watches().len() == 1);
    assert_same(h.daemon_watches().first(), &repo);

    // Open the picker and a context menu, whose callbacks hold the view.
    h.click("FILES_ROOT_BUTTON");
    h.visual.simulate_keystrokes("escape");
    h.settle();
    let row = h.visual.debug_bounds("FILES_ROW a.txt").unwrap();
    h.visual
        .simulate_mouse_down(row.center(), gpui::MouseButton::Right, Modifiers::default());
    h.settle();
    h.visual.simulate_keystrokes("escape");
    h.settle();

    let weak = h.view().downgrade();
    h.host
        .update(&mut h.visual, |host, _, cx| {
            host.panel.release(cx);
            host.context = None;
        })
        .unwrap();
    h.redraw();
    h.wait("the view's release", |h| {
        h.visual.update(|_, _| weak.upgrade().is_none())
    });
    h.wait("the daemon's unwatch", |h| h.daemon_watches().is_empty());
}

#[test]
fn an_evicted_view_is_released() {
    let mut h = harness(Some(chat("outside")));
    let first = h.view().downgrade();
    for index in 0..super::KEPT_VIEWS {
        h.show(Some(PanelContext {
            remote: true,
            ..chat(&format!("elsewhere-{index}"))
        }));
    }
    h.wait("the evicted view's release", |h| {
        h.visual.update(|_, _| first.upgrade().is_none())
    });
}

#[test]
fn a_root_that_fails_keeps_the_view_usable() {
    let mut h = harness(Some(chat("outside")));
    let missing = h.temp.path().join("nowhere").to_string_lossy().into_owned();
    let field = h.read(|view| view.choose_field.clone());
    let focus = field.read_with(&h.visual, |field, _| field.focus_handle());
    field.update(&mut h.visual, |field, cx| field.reset(missing.clone(), cx));
    h.visual.update(|window, cx| focus.focus(window, cx));
    h.visual.simulate_keystrokes("enter");
    h.wait("the error", |h| h.has("FILES_ERROR"));
    assert!(h.has("FILES_CHOOSE"), "Choose a folder stays");
    let path = h.repo.to_string_lossy().into_owned();
    field.update(&mut h.visual, |field, cx| field.reset(path, cx));
    h.visual.update(|window, cx| focus.focus(window, cx));
    h.visual.simulate_keystrokes("enter");
    h.wait("the corrected root", |h| h.has("FILES_ROW a.txt"));
    assert!(!h.has("FILES_ERROR"));

    // From the picker of a rooted tree, the tree stays.
    h.click("FILES_ROOT_BUTTON");
    h.visual.simulate_input(&missing);
    h.visual.simulate_keystrokes("enter");
    h.wait("the error", |h| h.has("FILES_ERROR"));
    assert!(h.has("FILES_ROW a.txt"), "the tree stays");
    assert_same(
        h.read(|view| view.root().map(|root| root.path.clone())),
        &h.repo,
    );
}

#[test]
fn the_picker_keeps_its_highlight_in_view() {
    let mut h = harness(Some(chat("outside")));
    h.store.update(&mut h.visual, |store, cx| {
        let mut sql_text = String::new();
        for index in 0..30 {
            let folder = h.temp.path().join(format!("project-{index:02}"));
            std::fs::create_dir_all(&folder).unwrap();
            sql_text.push_str(&format!(
                "INSERT INTO projects (id, name, cwd, position, created_at)
                 VALUES ('project-{index:02}', 'project-{index:02}', '{cwd}', {position}, '2026-11-04T00:00:00Z');",
                cwd = sql(&real(&folder)),
                position = index + 1,
            ));
        }
        store
            .test_core
            .db
            .get()
            .unwrap()
            .execute_batch(&sql_text)
            .unwrap();
        store.refresh(crate::app_store::StoreRefreshKind::All, cx);
    });
    h.settle();
    h.click("FILES_CHOOSE files-choose-project 0");
    h.wait("the root", |h| h.has("FILES_ROW a.txt"));
    h.click("FILES_ROOT_BUTTON");
    h.wait("the projects", |h| h.has("FILES_PICKER_ITEM 1"));
    let count = h.read(|view| view.picker.as_ref().unwrap().items.len());
    assert!(count > 25, "{count} items");
    let list = h.visual.debug_bounds("FILES_PICKER_LIST").unwrap();
    let visible = |h: &mut Harness, index: usize| {
        h.visual
            .debug_bounds(format!("FILES_PICKER_ITEM {index}").leak())
            .is_some_and(|item| item.top() >= list.top() && item.bottom() <= list.bottom())
    };
    for _ in 0..count - 1 {
        h.visual.simulate_keystrokes("down");
    }
    h.settle();
    let last = h.read(|view| view.picker.as_ref().unwrap().highlighted);
    assert_eq!(last, count - 1);
    assert!(visible(&mut h, last), "the highlight scrolls into view");
    assert!(!visible(&mut h, 0));

    h.visual.simulate_input("project-2");
    h.settle();
    assert_eq!(h.read(|view| view.picker.as_ref().unwrap().highlighted), 0);
    assert!(visible(&mut h, 0), "filtering starts at the top");
}

#[test]
fn an_ignored_folder_is_read_again_when_reopened() {
    let mut h = harness(None);
    std::fs::create_dir_all(h.repo.join("target/debug")).unwrap();
    std::fs::write(h.repo.join("target/debug/old.o"), "").unwrap();
    h.show(Some(chat("in-project")));
    h.wait("the status", |h| {
        h.status_read() && h.has("FILES_ROW target")
    });
    h.click("FILES_ROW target");
    h.wait("target", |h| h.has("FILES_ROW target/debug"));
    h.click("FILES_ROW target/debug");
    h.wait("target/debug", |h| h.has("FILES_ROW target/debug/old.o"));

    h.click("FILES_ROW target");
    assert!(!h.has("FILES_ROW target/debug"));
    std::fs::remove_file(h.repo.join("target/debug/old.o")).unwrap();
    std::fs::write(h.repo.join("target/debug/new.o"), "").unwrap();
    std::fs::write(h.repo.join("target/fresh.log"), "").unwrap();
    // The selected row draws as FILES_ROW_SELECTED.
    h.click("FILES_ROW_SELECTED");
    h.wait("the new listings", |h| {
        h.has("FILES_ROW target/debug/new.o") && h.has("FILES_ROW target/fresh.log")
    });
    assert!(!h.has("FILES_ROW target/debug/old.o"));
}

#[test]
fn overlapping_root_choices_save_the_last_one() {
    use runner_core::protocol::files::{FilesRootOutcome, FilesSubject};

    let mut h = harness(Some(chat("outside")));
    let first = real(&h.repo.join("src"));
    let last = h.repo.clone();
    let view = h.view();
    let (release, gate) = futures::channel::oneshot::channel();
    view.update(&mut h.visual, |view, cx| {
        view.root_gate = Some(gate);
        view.set_root(Some(first), cx);
        view.set_root(Some(last.clone()), cx);
    });
    h.settle();
    assert_eq!(
        h.read(|view| view.state()),
        "choose",
        "the second choice waits for the first to be saved"
    );
    release.send(()).unwrap();
    h.wait("the last root", |h| h.has("FILES_ROW a.txt"));
    assert_same(
        h.read(|view| view.root().map(|root| root.path.clone())),
        &last,
    );
    let saved = h.store.read_with(&h.visual, |store, _| {
        runner_daemon::files::root(&store.test_core, &FilesSubject::Session("outside".into()))
            .unwrap()
    });
    let FilesRootOutcome::Root(saved) = saved else {
        panic!("{saved:?}")
    };
    assert_same(Some(&saved.path), &last);
}
