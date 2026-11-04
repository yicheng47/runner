use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use gpui::{
    div, prelude::*, px, size, Context, Entity, Modifiers, TestAppContext, VisualTestContext,
    Window, WindowHandle,
};
use runner_core::protocol::files::{FilesRootSource, WorktreeKind};

use super::{FileOpen, FilesPanel, FilesView};
use crate::surfaces::side_panel::{PanelContext, PanelSubject, PanelSurface, Widened};
use crate::theme::{self, ThemeVariant};
use crate::theme_snapshot::{assert_border, assert_fill_color, assert_text, ThemeGuard};

struct Host {
    panel: FilesPanel,
    context: Option<PanelContext>,
    /// A view drawn without the panel, so never shown and never watching.
    unshown: Option<Entity<FilesView>>,
    /// Focus outside the Files view, and how often the pane's ⌘W reached it.
    elsewhere: Option<gpui::FocusHandle>,
    closed_panes: usize,
    /// Whether the owner has room for the tree beside an open file.
    tree_fits: bool,
}

impl Render for Host {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if let Some(view) = self.unshown.clone() {
            return div().size_full().flex().flex_col().child(view);
        }
        let body = match &self.context {
            Some(context) => {
                let settings = &crate::app_store::global_app_store(cx).read(cx).settings;
                let widened = Widened {
                    file: settings.file_viewer_width.max(288.),
                    tree: self.tree_fits,
                    tree_width: settings.file_tree_width,
                    standard: 288.,
                };
                Some(self.panel.show_with(context, Some(widened), None, cx))
            }
            None => {
                self.panel.hide(cx);
                None
            }
        };
        div()
            .size_full()
            .flex()
            .flex_col()
            .on_action(cx.listener(|host, _: &crate::CloseTab, _, _| host.closed_panes += 1))
            .children(
                self.elsewhere
                    .clone()
                    .map(|focus| div().h(px(4.)).track_focus(&focus)),
            )
            .children(body)
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
    let host = cx.add_window(|_, cx| Host {
        panel: FilesPanel::default(),
        context,
        unshown: None,
        elsewhere: Some(cx.focus_handle()),
        closed_panes: 0,
        tree_fits: true,
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
    // The click also opens the file, read in the background: let it land
    // before the tree is drawn for each theme.
    h.wait("the file", |h| h.has("FILE_CODE"));
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

// ── The file viewer (m4) ───────────────────────────────────────────────

impl Harness {
    fn double_click(&mut self, selector: impl Into<String>) {
        let selector: String = selector.into();
        let bounds = self
            .visual
            .debug_bounds(selector.clone().leak())
            .unwrap_or_else(|| panic!("missing {selector}"));
        for click_count in [1, 2] {
            self.visual.simulate_event(gpui::MouseDownEvent {
                button: gpui::MouseButton::Left,
                position: bounds.center(),
                modifiers: Modifiers::default(),
                click_count,
                first_mouse: false,
            });
            self.visual.simulate_event(gpui::MouseUpEvent {
                button: gpui::MouseButton::Left,
                position: bounds.center(),
                modifiers: Modifiers::default(),
                click_count,
            });
        }
        self.settle();
    }

    fn tabs(&mut self) -> Vec<(String, bool)> {
        self.read(|view| view.open_tabs())
    }

    fn open(&mut self, path: &str, keep: bool) {
        let view = self.view();
        view.update(&mut self.visual, |view, cx| {
            view.open_file(path.into(), keep, cx)
        });
        self.settle();
    }

    fn code(&mut self) -> Entity<super::code::CodeView> {
        self.read(|view| view.code_view())
            .expect("a file shown as code")
    }

    fn line(&mut self, line: usize) -> String {
        let code = self.code();
        code.read_with(&self.visual, |code, _| {
            code.document().line(line).to_owned()
        })
    }

    fn focus_code(&mut self) {
        let code = self.code();
        let focus = code.read_with(&self.visual, |code, _| code.focus_handle().clone());
        self.visual.update(|window, cx| focus.focus(window, cx));
        self.settle();
    }
}

fn viewer(context: PanelContext) -> Harness {
    let mut h = harness(Some(context));
    h.visual.simulate_resize(size(px(900.), px(700.)));
    h.visual.update(|_, cx| {
        cx.bind_keys([gpui::KeyBinding::new(
            &crate::keymap::platform_default("cmd-w"),
            crate::CloseTab,
            None,
        )]);
        crate::keymap::install_file_viewer_bindings(cx);
    });
    h.wait("the root", |h| h.has("FILES_ROW a.txt"));
    h
}

fn primary(key: &str) -> String {
    if cfg!(target_os = "macos") {
        format!("cmd-{key}")
    } else {
        format!("ctrl-{key}")
    }
}

#[test]
fn a_click_previews_a_file_and_a_double_click_on_it_or_its_tab_keeps_it() {
    let mut h = viewer(chat("in-project"));
    h.click("FILES_ROW a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert_eq!(h.tabs(), [("a.txt".to_owned(), true)]);
    assert!(h.has("FILE_TAB_PREVIEW"), "the preview tab is italic");
    assert_eq!(h.line(0), "a");

    h.click("FILES_ROW old.txt");
    h.wait("the next file", |h| h.has("FILE_TAB old.txt"));
    assert_eq!(
        h.tabs(),
        [("old.txt".to_owned(), true)],
        "the preview is replaced"
    );

    // The selected row answers to its selected-row snapshot id.
    h.double_click("FILES_ROW_SELECTED");
    assert_eq!(h.tabs(), [("old.txt".to_owned(), false)]);
    assert!(!h.has("FILE_TAB_PREVIEW"));

    h.click("FILES_ROW .gitignore");
    h.wait("a preview beside it", |h| h.has("FILE_TAB .gitignore"));
    assert_eq!(
        h.tabs(),
        [
            ("old.txt".to_owned(), false),
            (".gitignore".to_owned(), true)
        ]
    );
    h.double_click("FILE_TAB .gitignore");
    assert_eq!(
        h.tabs(),
        [
            ("old.txt".to_owned(), false),
            (".gitignore".to_owned(), false)
        ]
    );

    // Opening a file already open switches to its tab.
    h.click("FILES_ROW old.txt");
    assert_eq!(h.read(|view| view.active_tab()), Some("old.txt".into()));
    assert_eq!(h.tabs().len(), 2);

    h.click("FILE_TAB_CLOSE old.txt");
    assert_eq!(h.read(|view| view.active_tab()), Some(".gitignore".into()));
    h.click("FILE_TAB_CLOSE .gitignore");
    assert!(
        !h.has("FILE_VIEWER"),
        "closing the last tab returns to the tree"
    );
    assert!(h.has("FILES_ROW a.txt"));
}

#[test]
fn switching_tabs_reveals_the_file_in_the_tree_and_marks_changed_files() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("a.txt"), "changed\n").unwrap();
    h.wait("the status", |h| h.has("FILES_LETTER a.txt"));
    h.click("FILES_ROW src");
    assert!(!h.has("FILES_ROW src/deep"));
    h.open("src/deep/x.rs", true);
    h.open("a.txt", true);
    h.wait("the letter", |h| h.has("FILE_TAB_LETTER a.txt M"));
    h.click("FILE_TAB src/deep/x.rs");
    h.wait("the revealed row", |h| {
        h.read(|view| view.selected()).as_deref() == Some("src/deep/x.rs")
    });
    assert!(h.has("FILES_ROW src/deep"), "its folders opened");
}

#[test]
fn the_tab_strip_overflows_into_a_list_of_every_open_file() {
    let mut h = viewer(chat("in-project"));
    for index in 0..9 {
        std::fs::write(h.repo.join(format!("src/file_{index}.rs")), "x\n").unwrap();
    }
    for index in 0..9 {
        h.open(&format!("src/file_{index}.rs"), true);
    }
    h.redraw();
    h.redraw();
    assert!(
        h.has("FILE_TAB_LIST"),
        "the list button shows while tabs are cut off"
    );
    h.click("FILE_TAB_LIST");
    assert!(h.has("FILE_TAB_LIST_POPUP"));
    for index in 0..9 {
        assert!(h.has(format!("FILE_LIST_ITEM src/file_{index}.rs")));
    }
    h.click("FILE_LIST_ITEM src/file_2.rs");
    assert!(!h.has("FILE_TAB_LIST_POPUP"));
    assert_eq!(
        h.read(|view| view.active_tab()),
        Some("src/file_2.rs".into())
    );
}

#[test]
fn cmd_w_closes_the_file_tab_only_while_the_files_view_has_focus() {
    let mut h = viewer(chat("in-project"));
    h.open("a.txt", true);
    h.open("old.txt", true);
    h.wait("the file", |h| h.has("FILE_CODE"));

    h.focus_code();
    h.visual.simulate_keystrokes("ctrl-tab");
    h.settle();
    assert_eq!(h.read(|view| view.active_tab()), Some("a.txt".into()));
    h.visual.simulate_keystrokes("ctrl-shift-tab");
    h.settle();
    assert_eq!(h.read(|view| view.active_tab()), Some("old.txt".into()));

    let close = crate::keymap::platform_default("cmd-w");
    h.visual.simulate_keystrokes(&close);
    h.settle();
    assert_eq!(h.tabs(), [("a.txt".to_owned(), false)]);
    let closed = |h: &mut Harness| {
        h.host
            .read_with(&h.visual, |host, _| host.closed_panes)
            .unwrap()
    };
    assert_eq!(closed(&mut h), 0, "the pane stays");

    // With focus outside the panel ⌘W keeps closing the pane.
    let elsewhere = h
        .host
        .read_with(&h.visual, |host, _| host.elsewhere.clone())
        .unwrap()
        .unwrap();
    h.visual.update(|window, cx| elsewhere.focus(window, cx));
    h.visual.simulate_keystrokes(&close);
    h.settle();
    assert_eq!(closed(&mut h), 1);
    assert_eq!(h.tabs().len(), 1);

    // In find's field ⌘W still closes the file tab.
    h.open("old.txt", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.focus_code();
    h.visual.simulate_keystrokes(&primary("f"));
    h.settle();
    h.visual.simulate_keystrokes(&close);
    h.settle();
    assert_eq!(h.tabs(), [("a.txt".to_owned(), false)]);
    assert_eq!(closed(&mut h), 1);

    // In the tree with the last tab closed, ⌘W falls through to the pane.
    h.click("FILES_ROW src");
    h.visual.simulate_keystrokes(&close);
    h.settle();
    assert!(h.tabs().is_empty());
    assert_eq!(closed(&mut h), 1);
    h.visual.simulate_keystrokes(&close);
    h.settle();
    assert_eq!(closed(&mut h), 2);
}

#[test]
fn find_over_a_file_counts_steps_and_closes_back_to_the_code() {
    let mut h = viewer(chat("in-project"));
    let mut text = String::new();
    for line in 0..300 {
        text.push_str(if line % 100 == 7 {
            "let needle = 1;\n"
        } else {
            "filler\n"
        });
    }
    std::fs::write(h.repo.join("src/find.rs"), text).unwrap();
    h.open("src/find.rs", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.focus_code();
    h.visual.simulate_keystrokes(&primary("f"));
    h.settle();
    assert!(h.has("FILE_FIND_BAR"));
    h.visual.simulate_input("needle");
    h.wait("the matches", |h| {
        h.read(|view| view.find_state())
            .is_some_and(|(count, _)| count.as_deref() == Some("1 of 3"))
    });
    h.visual.simulate_keystrokes("enter");
    h.settle();
    assert_eq!(
        h.read(|view| view.find_state()),
        Some((Some("2 of 3".into()), Some(1)))
    );
    h.visual.simulate_keystrokes("shift-enter shift-enter");
    h.settle();
    assert_eq!(
        h.read(|view| view.find_state()).unwrap().1,
        Some(2),
        "wraps back"
    );
    h.visual.simulate_keystrokes("escape");
    h.settle();
    assert!(!h.has("FILE_FIND_BAR"));
    let code = h.code();
    let focused = h
        .visual
        .update(|window, cx| code.read(cx).focus_handle().is_focused(window));
    assert!(focused, "Esc returns focus to the file");
}

#[test]
fn an_open_file_reloads_on_change_keeping_its_scroll_and_dims_when_deleted() {
    let _theme = ThemeGuard::new();
    theme::set_active_variant(ThemeVariant::Carbon);
    let mut h = viewer(chat("in-project"));
    let path = h.repo.join("src/live.rs");
    let body = (0..400)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    std::fs::write(&path, &body).unwrap();
    h.open("src/live.rs", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    let code = h.code();
    code.update(&mut h.visual, |code, cx| {
        code.scroll_to(gpui::point(px(0.), px(600.)), cx)
    });
    h.redraw();
    let scrolled = code.read_with(&h.visual, |code, _| code.scroll_offset());
    assert_eq!(scrolled.y, px(600.));

    // FSEvents can miss a write in the instant its stream starts, so write
    // again until the watcher reports it.
    let mut touched = Instant::now() - Duration::from_secs(1);
    h.wait("the reload", |h| {
        if touched.elapsed() >= Duration::from_secs(1) {
            std::fs::write(&path, format!("edited\n{body}")).unwrap();
            touched = Instant::now();
        }
        h.line(0) == "edited"
    });
    assert_eq!(
        code.read_with(&h.visual, |code, _| code.scroll_offset()),
        scrolled,
        "the reload keeps the scroll position"
    );

    // The Open in menu was open when the file went.
    h.click("FILE_OPEN_IN_MENU");
    assert!(h.has("FILE_OPEN_IN_POPUP"));
    std::fs::remove_file(&path).unwrap();
    h.wait("the deleted state", |h| h.has("FILE_DELETED"));
    assert!(h.has("FILE_TAB_LETTER src/live.rs D"));
    assert!(h.has("FILE_CODE"), "the last version read stays");
    assert_text(&mut h.visual, "FILE_DELETED_TEXT", theme::text());
    assert!(
        !h.has("FILE_OPEN_IN_POPUP"),
        "the menu closes with the file"
    );

    // Nothing opens the deleted path: not Open in, its menu, nor ⌘-click.
    h.click("FILE_OPEN_IN");
    h.click("FILE_OPEN_IN_MENU");
    assert!(!h.has("FILE_OPEN_IN_POPUP"));
    let bounds = h.visual.debug_bounds("FILE_CODE").unwrap();
    let open = if cfg!(target_os = "macos") {
        Modifiers::command()
    } else {
        Modifiers::control()
    };
    h.visual.simulate_click(bounds.center(), open);
    h.settle();
    assert!(h.read(|view| view.launched()).is_empty());

    // The retained version still reads and copies.
    h.focus_code();
    h.visual.dispatch_action(crate::SelectAll);
    h.visual.simulate_keystrokes(&primary("c"));
    h.settle();
    let copied = h.visual.read_from_clipboard().and_then(|item| item.text());
    assert!(copied.is_some_and(|text| text.starts_with("edited\nline 0")));

    h.click("FILE_DELETED_CLOSE");
    assert!(h.tabs().is_empty());
}

#[test]
fn binary_and_large_files_show_their_placeholders() {
    let _theme = ThemeGuard::new();
    theme::set_active_variant(ThemeVariant::Carbon);
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("runnerd.sqlite"), b"SQLite\0\x01\x02").unwrap();
    let large = std::fs::File::create(h.repo.join("events.ndjson")).unwrap();
    large
        .set_len(runner_core::protocol::files::MAX_READ_BYTES + 1)
        .unwrap();
    h.open("runnerd.sqlite", true);
    h.wait("the binary placeholder", |h| h.has("FILE_BINARY"));
    assert!(h.has("FILE_OPEN_DEFAULT"));
    assert_text(&mut h.visual, "FILE_STATE_TITLE", theme::text());
    h.open("events.ndjson", true);
    h.wait("the large placeholder", |h| h.has("FILE_TOO_LARGE"));
    assert!(h.has("FILE_OPEN_LARGE"));
    assert_text(&mut h.visual, "FILE_STATE_TITLE", theme::text());
    assert!(!h.has("FILE_CODE"));
}

#[test]
fn a_large_file_draws_only_the_visible_lines() {
    let mut h = viewer(chat("in-project"));
    let body = (0..10_000)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    std::fs::write(h.repo.join("src/big.rs"), body).unwrap();
    let counts = crate::render_counts::RenderCounts::default();
    h.visual.update(|_, cx| cx.set_global(counts.clone()));
    h.open("src/big.rs", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    counts.0.borrow_mut().clear();
    h.redraw();
    let shaped = counts
        .0
        .borrow()
        .get("file-code-lines")
        .copied()
        .unwrap_or(0);
    assert!(shaped > 0 && shaped < 100, "shaped {shaped} lines");
    assert_eq!(
        h.code()
            .read_with(&h.visual, |code, _| code.document().line_count()),
        10_000
    );
}

#[test]
fn a_drag_selects_across_lines_that_scroll_out_of_view_and_copies_them() {
    let mut h = viewer(chat("in-project"));
    let body = (0..300)
        .map(|line| format!("line {line}\n"))
        .collect::<String>();
    std::fs::write(h.repo.join("src/select.rs"), body).unwrap();
    h.open("src/select.rs", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    let bounds = h.visual.debug_bounds("FILE_CODE").unwrap();
    let start = gpui::point(bounds.left() + px(60.), bounds.top() + px(12.));
    h.visual
        .simulate_mouse_down(start, gpui::MouseButton::Left, Modifiers::default());
    let below = gpui::point(bounds.left() + px(200.), bounds.bottom() + px(400.));
    for _ in 0..6 {
        h.visual
            .simulate_mouse_move(below, Some(gpui::MouseButton::Left), Modifiers::default());
        h.settle();
    }
    h.visual
        .simulate_mouse_up(below, gpui::MouseButton::Left, Modifiers::default());
    h.settle();
    let code = h.code();
    assert!(code.read_with(&h.visual, |code, _| code.scroll_offset().y) > px(0.));
    h.visual.simulate_keystrokes(&primary("c"));
    h.settle();
    let copied = h
        .visual
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("copied text");
    let first = copied.lines().next().unwrap();
    assert!(!first.is_empty() && "line 0".ends_with(first), "{copied:?}");
    let lines = copied.lines().count();
    assert!(
        lines > 60,
        "the selection reaches past the first screen: {lines}"
    );
}

#[test]
fn open_in_lists_the_default_app_and_the_reveal_row() {
    let mut h = viewer(chat("in-project"));
    h.open("a.txt", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert!(h.has("FILE_OPEN_IN"));
    h.click("FILE_OPEN_IN_MENU");
    assert!(h.has("FILE_OPEN_IN_POPUP"));
    assert!(h.has("FILE_OPEN_IN_ITEM file-open-in-default 0"));
    assert!(h.has("FILE_OPEN_IN_ITEM file-open-in-reveal 0"));
}

#[test]
fn the_tree_toggle_hides_the_tree_beside_an_open_file() {
    let mut h = viewer(chat("in-project"));
    h.open("a.txt", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert!(h.has("FILES_TREE_COLUMN"));
    assert_eq!(h.read(|view| view.widening()), Some(true));
    h.click("FILE_TREE_TOGGLE");
    assert!(!h.has("FILES_TREE_COLUMN"));
    assert_eq!(h.read(|view| view.widening()), Some(false));
    // A panel with no room for the tree hides it with the toggle still on.
    h.click("FILE_TREE_TOGGLE");
    h.host
        .update(&mut h.visual, |host, _, cx| {
            host.tree_fits = false;
            cx.notify();
        })
        .unwrap();
    h.settle();
    assert!(!h.has("FILES_TREE_COLUMN"));
    assert!(h.has("FILE_TREE_TOGGLE"));
}

#[test]
fn an_open_file_widens_the_panel_into_the_room_the_mission_leaves() {
    let mut h = viewer(mission("on-worktree"));
    let subject = PanelSubject::Mission("on-worktree".into());
    let layout = |h: &mut Harness, surface: f32| {
        h.host
            .read_with(&h.visual, |host, cx| {
                host.panel.surface.set(surface);
                host.panel.layout(Some(&subject), 288., 440., 240., 6., cx)
            })
            .unwrap()
    };
    assert_eq!(
        layout(&mut h, 1200.),
        (288., None),
        "no file, the standard width"
    );
    h.open("a.txt", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    // 6 + 420 + 440 + 240: the tree fits beside the file.
    let (width, widened) = layout(&mut h, 1106.);
    assert_eq!(width, 680.);
    assert!(widened.unwrap().tree);
    // Narrower: the tree hides first, then the file gives way to the mission.
    assert_eq!(layout(&mut h, 1105.).0, 440.);
    assert!(!layout(&mut h, 1105.).1.unwrap().tree);
    assert_eq!(layout(&mut h, 800.).0, 374.);
    assert_eq!(
        layout(&mut h, 600.).0,
        288.,
        "never below the standard width"
    );
}

#[test]
fn a_saved_tree_width_widens_the_panel_and_still_hides_before_the_file_narrows() {
    let mut h = viewer(mission("on-worktree"));
    let subject = PanelSubject::Mission("on-worktree".into());
    let layout = |h: &mut Harness, surface: f32| {
        h.host
            .read_with(&h.visual, |host, cx| {
                host.panel.surface.set(surface);
                host.panel.layout(Some(&subject), 288., 440., 320., 6., cx)
            })
            .unwrap()
    };
    h.open("a.txt", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    // 6 + 420 + 440 + 320: the wider tree fits beside the file.
    let (width, widened) = layout(&mut h, 1186.);
    assert_eq!(width, 760.);
    assert!(widened.unwrap().tree);
    // Narrower: the tree hides first, then the file gives way to the mission.
    let (width, widened) = layout(&mut h, 1185.);
    assert_eq!(width, 440.);
    assert!(!widened.unwrap().tree);
    assert_eq!(layout(&mut h, 800.).0, 374.);
    assert_eq!(layout(&mut h, 600.).0, 288.);
}

#[test]
fn dragging_the_tree_divider_sizes_the_tree_against_the_file_and_saves_both() {
    let mut h = viewer(chat("in-project"));
    h.visual.simulate_resize(size(px(800.), px(900.)));
    h.open("a.txt", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    let settings = |h: &mut Harness| {
        h.store.read_with(&h.visual, |store, _| {
            (
                store.settings.file_tree_width,
                store.settings.file_viewer_width,
            )
        })
    };
    assert_eq!(settings(&mut h), (240., 440.));
    let gutter = h.visual.debug_bounds("FILE_TREE_GUTTER").unwrap();
    let column = h.visual.debug_bounds("FILES_TREE_COLUMN").unwrap();
    assert_eq!(gutter.center().x, column.left());
    let start = gutter.center();
    let to = |h: &mut Harness, dx: f32| {
        for _ in 0..3 {
            h.visual.simulate_mouse_move(
                start + gpui::point(px(dx), px(0.)),
                Some(gpui::MouseButton::Left),
                Modifiers::default(),
            );
            h.settle();
        }
    };
    h.visual
        .simulate_mouse_down(start, gpui::MouseButton::Left, Modifiers::default());
    // Toward the file: the tree widens by what the file gives up.
    to(&mut h, -60.);
    assert_eq!(settings(&mut h), (300., 380.));
    let column = h.visual.debug_bounds("FILES_TREE_COLUMN").unwrap();
    assert_eq!(column.size.width, px(300.));
    // The file stops at the panel's standard width.
    to(&mut h, -400.);
    assert_eq!(settings(&mut h), (392., 288.));
    // Toward the edge: the tree stops at its minimum.
    to(&mut h, 200.);
    assert_eq!(settings(&mut h), (160., 520.));
    to(&mut h, -60.);
    h.visual.simulate_mouse_up(
        start + gpui::point(px(-60.), px(0.)),
        gpui::MouseButton::Left,
        Modifiers::default(),
    );
    h.settle();
    let saved =
        crate::app_settings::AppSettings::load(&h.temp.path().join("settings.json")).unwrap();
    assert_eq!(
        (saved.file_tree_width, saved.file_viewer_width),
        (300., 380.)
    );
}

#[test]
fn dragging_the_tree_divider_beside_the_widest_file_keeps_the_panel_width() {
    let mut h = viewer(chat("in-project"));
    h.visual.simulate_resize(size(px(800.), px(900.)));
    h.store.update(&mut h.visual, |store, cx| {
        store.update_settings(
            |settings| {
                settings.file_viewer_width = 1100.;
                settings.file_tree_width = 300.;
                true
            },
            false,
            cx,
        );
    });
    h.open("a.txt", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    let start = h.visual.debug_bounds("FILE_TREE_GUTTER").unwrap().center();
    let end = start + gpui::point(px(250.), px(0.));
    h.visual
        .simulate_mouse_down(start, gpui::MouseButton::Left, Modifiers::default());
    for _ in 0..3 {
        h.visual
            .simulate_mouse_move(end, Some(gpui::MouseButton::Left), Modifiers::default());
        h.settle();
    }
    h.visual
        .simulate_mouse_up(end, gpui::MouseButton::Left, Modifiers::default());
    h.settle();
    // The file reaches its maximum, so the tree keeps what the panel holds.
    let saved =
        crate::app_settings::AppSettings::load(&h.temp.path().join("settings.json")).unwrap();
    assert_eq!(
        (saved.file_viewer_width, saved.file_tree_width),
        (1200., 200.)
    );
}

#[test]
fn a_reload_drops_find_results_for_the_old_text_before_they_draw_or_step() {
    let mut h = viewer(chat("in-project"));
    let ascii = format!("{}needle and more\n", "x".repeat(601));
    std::fs::write(h.repo.join("src/long.rs"), &ascii).unwrap();
    h.open("src/long.rs", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.focus_code();
    h.visual.simulate_keystrokes(&primary("f"));
    h.visual.simulate_input("needle");
    h.wait("the match", |h| {
        h.read(|view| view.find_state())
            .is_some_and(|(count, _)| count.as_deref() == Some("1 of 1"))
    });
    let code = h.code();
    h.redraw();
    assert!(
        code.read_with(&h.visual, |code, _| code.scroll_offset().x) > px(0.),
        "scrolled to the match at byte 601"
    );
    // The file comes back as wide text whose byte 601 is inside a
    // character, and find steps before its new search answers.
    let view = h.view();
    let read = h.read(|view| view.read_seq("src/long.rs")).unwrap();
    view.update(&mut h.visual, |view, cx| {
        view.hold_search(true, cx);
        view.reply_text("src/long.rs", read, &format!("{}\n", "汉".repeat(700)), cx);
        view.step(true, cx);
    });
    h.redraw();
    assert_eq!(
        code.read_with(&h.visual, |code, _| code.find_highlights()),
        (0, None)
    );
    assert_eq!(h.read(|view| view.find_state()), Some((None, None)));
    view.update(&mut h.visual, |view, cx| view.hold_search(false, cx));
    h.settle();
    assert_eq!(
        h.read(|view| view.find_state()).unwrap().0.as_deref(),
        Some("No results")
    );
}

#[test]
fn opening_from_a_focused_tree_that_then_hides_keeps_the_viewer_keys() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("data.bin"), b"\0\x01").unwrap();
    h.host
        .update(&mut h.visual, |host, _, cx| {
            host.tree_fits = false;
            cx.notify();
        })
        .unwrap();
    h.settle();
    // The tree has focus, opens the file and leaves the panel.
    h.click("FILES_ROW a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert!(!h.has("FILES_TREE_COLUMN"));
    h.visual.simulate_keystrokes(&primary("f"));
    h.settle();
    assert!(h.has("FILE_FIND_BAR"), "find reaches the file");
    h.visual.simulate_keystrokes("escape");
    h.settle();
    // A placeholder, with nothing to focus inside it, keeps the keys too.
    h.double_click("FILE_TAB a.txt");
    h.open("data.bin", true);
    h.wait("the placeholder", |h| h.has("FILE_BINARY"));
    h.visual.simulate_keystrokes("ctrl-tab");
    h.settle();
    assert_eq!(h.read(|view| view.active_tab()), Some("a.txt".into()));
    h.visual.simulate_keystrokes("ctrl-tab");
    h.settle();
    assert_eq!(h.read(|view| view.active_tab()), Some("data.bin".into()));
    let close = crate::keymap::platform_default("cmd-w");
    h.visual.simulate_keystrokes(&close);
    h.settle();
    assert_eq!(h.tabs(), [("a.txt".to_owned(), false)]);
    h.visual.simulate_keystrokes(&close);
    h.settle();
    assert!(h.tabs().is_empty());
    let closed = h
        .host
        .read_with(&h.visual, |host, _| host.closed_panes)
        .unwrap();
    assert_eq!(closed, 0, "the pane stays");
}

#[test]
fn find_after_a_long_tab_prefix_scrolls_to_the_match() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(
        h.repo.join("src/tabs.rs"),
        format!("{}needle\n", "\t".repeat(200)),
    )
    .unwrap();
    h.open("src/tabs.rs", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.focus_code();
    h.visual.simulate_keystrokes(&primary("f"));
    h.visual.simulate_input("needle");
    h.wait("the match", |h| {
        h.read(|view| view.find_state())
            .is_some_and(|(count, _)| count.as_deref() == Some("1 of 1"))
    });
    h.redraw();
    let code = h.code();
    let (char_width, text_left, bounds) = code.read_with(&h.visual, |code, _| code.geometry());
    let scroll = code.read_with(&h.visual, |code, _| code.scroll_offset()).x;
    let at = char_width * 800.;
    assert!(
        at >= scroll && at + char_width * 6. <= scroll + bounds.size.width - text_left,
        "column 800 in view: scrolled {scroll:?}"
    );
}

#[test]
fn a_long_line_of_wide_and_combining_text_selects_what_is_drawn() {
    let mut h = viewer(chat("in-project"));
    let wide = "汉".repeat(2000);
    let combining = "e\u{301}".repeat(2000);
    std::fs::write(h.repo.join("src/wide.rs"), format!("{wide}\n{combining}\n")).unwrap();
    h.open("src/wide.rs", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    let code = h.code();
    code.update(&mut h.visual, |code, cx| {
        code.scroll_to(gpui::point(px(0.), px(0.)), cx)
    });
    h.redraw();
    let (char_width, text_left, bounds) = code.read_with(&h.visual, |code, _| code.geometry());
    // Scrolled to column 1000, character 500 of the wide line.
    code.update(&mut h.visual, |code, cx| {
        code.scroll_to(gpui::point(char_width * 1000., px(0.)), cx)
    });
    h.redraw();
    let x = |column: f32| bounds.left() + text_left + char_width * column;
    let first_row = code.read_with(&h.visual, |code, _| code.row_y(0));
    let second_row = code.read_with(&h.visual, |code, _| code.row_y(1));
    // Columns 10 to 30 in view: characters 505 to 515.
    h.visual.simulate_mouse_down(
        gpui::point(x(10.), first_row),
        gpui::MouseButton::Left,
        Modifiers::default(),
    );
    h.visual.simulate_mouse_up(
        gpui::point(x(10.), first_row),
        gpui::MouseButton::Left,
        Modifiers::default(),
    );
    h.visual
        .simulate_click(gpui::point(x(30.), first_row), Modifiers::shift());
    h.settle();
    h.focus_code();
    h.visual.simulate_keystrokes(&primary("c"));
    h.settle();
    let copied = h
        .visual
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap();
    assert_eq!(copied, "汉".repeat(10));
    // The combining line: one cell per letter, its accent never split off.
    h.visual.simulate_mouse_down(
        gpui::point(x(4.), second_row),
        gpui::MouseButton::Left,
        Modifiers::default(),
    );
    h.visual.simulate_mouse_up(
        gpui::point(x(4.), second_row),
        gpui::MouseButton::Left,
        Modifiers::default(),
    );
    h.visual
        .simulate_click(gpui::point(x(7.), second_row), Modifiers::shift());
    h.settle();
    h.focus_code();
    h.visual.simulate_keystrokes(&primary("c"));
    h.settle();
    let copied = h
        .visual
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap();
    assert_eq!(copied, "e\u{301}".repeat(3));
}

#[test]
fn a_very_long_single_line_shapes_only_what_is_in_view() {
    let mut h = viewer(chat("in-project"));
    let line = "\tlet x = 1;".repeat(400_000);
    std::fs::write(h.repo.join("src/minified.js"), format!("{line}\n")).unwrap();
    h.open("src/minified.js", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.redraw();
    let code = h.code();
    let shaped = code.read_with(&h.visual, |code, _| code.shaped_bytes.get());
    assert!(
        shaped > 0 && shaped < 2048,
        "shaped {shaped} bytes at the start"
    );
    let (char_width, _, _) = code.read_with(&h.visual, |code, _| code.geometry());
    code.update(&mut h.visual, |code, cx| {
        code.scroll_to(gpui::point(char_width * 3_000_000., px(0.)), cx)
    });
    h.redraw();
    let shaped = code.read_with(&h.visual, |code, _| code.shaped_bytes.get());
    assert!(
        shaped > 0 && shaped < 2048,
        "shaped {shaped} bytes far right"
    );
}

#[test]
fn a_reply_for_a_closed_tab_never_passes_for_the_file_reopened() {
    let mut h = viewer(chat("in-project"));
    let view = h.view();
    let first = view.update(&mut h.visual, |view, cx| {
        view.open_file("a.txt".into(), true, cx);
        view.read_seq("a.txt").unwrap()
    });
    h.visual
        .update(|window, cx| view.update(cx, |view, cx| view.close_tab(0, window, cx)));
    let second = view.update(&mut h.visual, |view, cx| {
        view.open_file("a.txt".into(), true, cx);
        view.read_seq("a.txt").unwrap()
    });
    assert_ne!(first, second);
    h.settle();
    assert_eq!(h.line(0), "a");
    // The first read's reply arrives late, with what the file held then.
    view.update(&mut h.visual, |view, cx| {
        view.reply_text("a.txt", first, "stale\n", cx)
    });
    h.settle();
    assert_eq!(h.line(0), "a");
}

#[test]
fn a_late_reply_from_a_closed_find_never_passes_for_the_reopened_one() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("src/many.rs"), "aaaa aaaa\n").unwrap();
    h.open("src/many.rs", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.focus_code();
    let view = h.view();
    view.update(&mut h.visual, |view, cx| view.hold_search(true, cx));
    h.visual.simulate_keystrokes(&primary("f"));
    h.visual.simulate_input("aaaa");
    h.settle();
    let first = h.read(|view| view.last_search()).unwrap();
    // The bar closes and reopens on the same file with another query.
    h.visual.simulate_keystrokes("escape");
    h.settle();
    h.visual.simulate_keystrokes(&primary("f"));
    h.visual.simulate_input("a a");
    h.settle();
    let second = h.read(|view| view.last_search()).unwrap();
    assert_ne!(first, second);
    view.update(&mut h.visual, |view, cx| view.reply_search(second, 1, cx));
    // The first query's reply arrives after the second's.
    view.update(&mut h.visual, |view, cx| view.reply_search(first, 7, cx));
    h.settle();
    assert_eq!(
        h.read(|view| view.find_state()).unwrap().0.as_deref(),
        Some("1 of 1")
    );
}

#[test]
fn code_that_becomes_a_placeholder_hands_focus_back_to_the_viewer() {
    let mut h = viewer(chat("in-project"));
    h.host
        .update(&mut h.visual, |host, _, cx| {
            host.tree_fits = false;
            cx.notify();
        })
        .unwrap();
    h.settle();
    h.open("a.txt", true);
    h.open("old.txt", true);
    h.wait("the file", |h| h.has("FILE_CODE"));
    let close = crate::keymap::platform_default("cmd-w");
    let closed = |h: &mut Harness| {
        h.host
            .read_with(&h.visual, |host, _| host.closed_panes)
            .unwrap()
    };
    for with_find in [false, true] {
        h.focus_code();
        if with_find {
            h.visual.simulate_keystrokes(&primary("f"));
            h.settle();
            assert!(h.has("FILE_FIND_BAR"));
        }
        // The focused file is read again and is now binary.
        let active = h.read(|view| view.active_tab()).unwrap();
        let read = h.read(|view| view.read_seq(&active)).unwrap();
        let view = h.view();
        view.update(&mut h.visual, |view, cx| {
            view.reply_binary(&active, read, 9, cx)
        });
        h.settle();
        assert!(h.has("FILE_BINARY"));
        h.visual.simulate_keystrokes("ctrl-tab");
        h.settle();
        assert_ne!(
            h.read(|view| view.active_tab()),
            Some(active.clone()),
            "⌃Tab still acts"
        );
        h.visual.simulate_keystrokes("ctrl-tab");
        h.settle();
        h.visual.simulate_keystrokes(&close);
        h.settle();
        assert!(
            !h.tabs().iter().any(|(path, _)| *path == active),
            "⌘W closed the tab"
        );
        assert_eq!(closed(&mut h), 0, "the pane stays");
        if !with_find {
            h.open("old.txt", true);
            h.wait("the file", |h| h.has("FILE_CODE"));
        }
    }
}
