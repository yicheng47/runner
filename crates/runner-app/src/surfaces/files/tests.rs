use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::Arc;
use std::time::{Duration, Instant};

use gpui::{
    div, prelude::*, px, size, Context, Entity, Modifiers, TestAppContext, VisualTestContext,
    Window, WindowHandle,
};
use runner_core::protocol::files::{FilesRootSource, WorktreeKind};

use super::tree::PreparedStatus;
use super::{FilesEvent, FilesPanel, FilesSurface, FilesView};
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
    /// The app zoom actions that reached the window, as the app's own do.
    app_zooms: Vec<&'static str>,
    /// Whether the owner's layout shows the tree beside an open file.
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
                let has_file = self
                    .panel
                    .view(&context.subject)
                    .is_some_and(|view| view.read(cx).has_tabs());
                // As `FilesPanel::layout`: no room and no file, no widening.
                let widened = (self.tree_fits || has_file).then_some(Widened {
                    file: settings.file_viewer_width.max(288.),
                    tree: self.tree_fits,
                    tree_width: settings.file_tree_width,
                    standard: 288.,
                });
                let owner = cx.entity_id();
                Some(self.panel.show_with(context, widened, Some(owner), cx))
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
            .on_action(cx.listener(|host, _: &crate::ZoomIn, _, _| host.app_zooms.push("in")))
            .on_action(cx.listener(|host, _: &crate::ZoomOut, _, _| host.app_zooms.push("out")))
            .on_action(cx.listener(|host, _: &crate::ZoomReset, _, _| host.app_zooms.push("reset")))
            .children(
                self.elsewhere
                    .clone()
                    .map(|focus| div().h(px(4.)).key_context("Terminal").track_focus(&focus)),
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
        app_zooms: Vec::new(),
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
fn tree_rows_span_the_column_with_a_guide_for_each_ancestor() {
    let mut h = harness(Some(chat("in-project")));
    h.wait("the folder", |h| h.has("FILES_ROW src/deep"));
    h.click("FILES_ROW src/deep");
    h.wait("the nested folder", |h| h.has("FILES_ROW src/deep/x.rs"));

    let tree = h.visual.debug_bounds("FILES_TREE").unwrap();
    for selector in ["FILES_ROW a.txt", "FILES_ROW src/deep/x.rs"] {
        let row = h.visual.debug_bounds(selector).unwrap();
        assert_eq!(row.left(), tree.left(), "{selector}");
        assert_eq!(row.right(), tree.right(), "{selector}");
    }

    assert!(!h.has("FILES_GUIDE a.txt 0"));
    assert!(h.has("FILES_GUIDE src/deep 0"));
    assert!(!h.has("FILES_GUIDE src/deep 1"));
    let deep = h.visual.debug_bounds("FILES_GUIDE src/deep 0").unwrap();
    let outer = h
        .visual
        .debug_bounds("FILES_GUIDE src/deep/x.rs 0")
        .unwrap();
    let inner = h
        .visual
        .debug_bounds("FILES_GUIDE src/deep/x.rs 1")
        .unwrap();
    assert_eq!(
        outer.left(),
        deep.left(),
        "an ancestor's guide stays straight"
    );
    assert_eq!(outer.top(), deep.bottom(), "the rows' guides join");
    assert!(inner.left() > outer.left());
}

#[test]
fn a_long_line_scrolls_by_a_horizontal_scrollbar_that_shows_on_hover() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("src/long.rs"), format!("{}\n", "x".repeat(600))).unwrap();
    h.open("src/long.rs");
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.redraw();
    let code = h.code();
    let bounds = h.visual.debug_bounds("FILE_CODE").unwrap();
    assert!(
        !h.has("SCROLLBAR_THUMB_X"),
        "hidden until the pointer is over the file"
    );

    h.visual
        .simulate_mouse_move(bounds.center(), None, Modifiers::default());
    h.settle();
    let thumb = h.visual.debug_bounds("SCROLLBAR_THUMB_X").unwrap();
    assert!(thumb.bottom() <= bounds.bottom());
    assert!(thumb.size.width < bounds.size.width / 2.);

    let target = thumb.center() + gpui::point(px(80.), px(0.));
    h.visual.simulate_mouse_down(
        thumb.center(),
        gpui::MouseButton::Left,
        Modifiers::default(),
    );
    // The first move starts the drag; the next one moves it.
    for _ in 0..2 {
        h.visual
            .simulate_mouse_move(target, Some(gpui::MouseButton::Left), Modifiers::default());
    }
    h.visual
        .simulate_mouse_up(target, gpui::MouseButton::Left, Modifiers::default());
    h.settle();
    assert!(code.read_with(&h.visual, |code, _| code.scroll_offset().x) > px(0.));
    assert_eq!(
        code.read_with(&h.visual, |code, _| code.selected_text()),
        None,
        "dragging the thumb selects no text"
    );
}

#[test]
fn a_file_that_fits_shows_no_horizontal_scrollbar() {
    let mut h = viewer(chat("in-project"));
    h.open("a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.redraw();
    let bounds = h.visual.debug_bounds("FILE_CODE").unwrap();
    h.visual
        .simulate_mouse_move(bounds.center(), None, Modifiers::default());
    h.settle();
    assert!(!h.has("SCROLLBAR_THUMB_X"));
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
    h.wait("the worktrees", |h| h.has("FILES_PICKER_ITEM 2"));
    let items = h.read(|view| view.picker.as_ref().unwrap().items.clone());
    assert!(items[0].path.is_none(), "Choose folder… first");
    assert_eq!(items[1].title, "runner", "then the main checkout");
    assert!(items[1].subtitle.starts_with("main · "));
    assert_eq!(items[2].title, "feat-x");
    assert!(items[2].current);
    assert_eq!(
        h.read(|view| view.picker.as_ref().unwrap().highlighted),
        1,
        "Enter stays on the first folder as the worktrees arrive"
    );

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
    h.wait("the worktrees", |h| h.has("FILES_PICKER_ITEM 2"));
    h.click("FILES_PICKER_ITEM 1");
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

    h.visual.simulate_keystrokes("space");
    h.visual.simulate_keystrokes("enter");
    h.settle();
    assert_eq!(
        h.tabs(),
        [".gitignore", "old.txt"],
        "the click and Space or Enter each open a tab"
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
    assert_eq!(h.read(|view| view.picker.as_ref().unwrap().highlighted), 1);
    assert!(visible(&mut h, 0), "filtering starts at the top");
    assert!(visible(&mut h, 1));
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

    fn tabs(&mut self) -> Vec<String> {
        self.read(|view| view.open_tabs())
    }

    fn open(&mut self, path: &str) {
        let view = self.view();
        view.update(&mut self.visual, |view, cx| view.open_file(path.into(), cx));
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
fn every_click_opens_a_tab_that_stays_and_an_open_file_switches_to_it() {
    let mut h = viewer(chat("in-project"));
    h.click("FILES_ROW a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert_eq!(h.tabs(), ["a.txt"]);
    assert_eq!(h.line(0), "a");

    h.click("FILES_ROW old.txt");
    h.wait("the next file", |h| h.has("FILE_TAB old.txt"));
    assert_eq!(h.tabs(), ["a.txt", "old.txt"], "each click keeps its tab");

    // A double-click is two clicks: nothing more.
    h.double_click("FILES_ROW .gitignore");
    h.wait("a third tab", |h| h.has("FILE_TAB .gitignore"));
    assert_eq!(h.tabs(), ["a.txt", "old.txt", ".gitignore"]);
    h.double_click("FILE_TAB old.txt");
    assert_eq!(h.read(|view| view.active_tab()), Some("old.txt".into()));
    assert_eq!(h.tabs().len(), 3);

    // Opening a file already open switches to its tab.
    h.click("FILES_ROW a.txt");
    assert_eq!(h.read(|view| view.active_tab()), Some("a.txt".into()));
    assert_eq!(h.tabs().len(), 3);

    for name in ["a.txt", "old.txt", ".gitignore"] {
        h.click(format!("FILE_TAB_CLOSE {name}"));
    }
    h.wait("the empty view", |h| h.has("FILE_NO_FILE"));
    assert!(!h.has("FILE_CODE"));
    assert!(h.has("FILES_ROW a.txt"));
}

#[test]
fn switching_tabs_reveals_the_file_in_the_tree_and_marks_changed_files() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("a.txt"), "changed\n").unwrap();
    h.wait("the status", |h| h.has("FILES_LETTER a.txt"));
    h.click("FILES_ROW src");
    assert!(!h.has("FILES_ROW src/deep"));
    h.open("src/deep/x.rs");
    h.open("a.txt");
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
        h.open(&format!("src/file_{index}.rs"));
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
    h.open("a.txt");
    h.open("old.txt");
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
    assert_eq!(h.tabs(), ["a.txt"]);
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
    h.open("old.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.focus_code();
    h.visual.simulate_keystrokes(&primary("f"));
    h.settle();
    h.visual.simulate_keystrokes(&close);
    h.settle();
    assert_eq!(h.tabs(), ["a.txt"]);
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
    h.open("src/find.rs");
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
    h.open("src/live.rs");
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
    h.open("runnerd.sqlite");
    h.wait("the binary placeholder", |h| h.has("FILE_BINARY"));
    assert!(h.has("FILE_OPEN_DEFAULT"));
    assert_text(&mut h.visual, "FILE_STATE_TITLE", theme::text());
    h.open("events.ndjson");
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
    h.open("src/big.rs");
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
    h.open("src/select.rs");
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
    h.open("a.txt");
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
    h.open("a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert!(h.has("FILES_TREE_COLUMN"));
    assert_eq!(h.read(|view| view.widening()), Some(true));
    h.click("FILE_TREE_TOGGLE");
    assert!(!h.has("FILES_TREE_COLUMN"));
    assert_eq!(h.read(|view| view.widening()), Some(false));
    // Turned back on, the tree shows again.
    h.click("FILE_TREE_TOGGLE");
    assert!(h.has("FILES_TREE_COLUMN"));
    assert_eq!(h.read(|view| view.widening()), Some(true));
}

#[test]
fn an_open_file_widens_the_panel_into_the_room_the_mission_leaves() {
    let mut h = viewer(mission("on-worktree"));
    let subject = PanelSubject::Mission("on-worktree".into());
    let layout = |h: &mut Harness, surface: f32| {
        h.host
            .read_with(&h.visual, |host, cx| {
                host.panel.surface.set(surface);
                host.panel.layout(
                    Some(&subject),
                    FilesSurface::Shown,
                    288.,
                    440.,
                    240.,
                    6.,
                    cx,
                )
            })
            .unwrap()
    };
    let beside = |h: &mut Harness, files: FilesSurface| {
        h.host
            .read_with(&h.visual, |host, cx| {
                host.panel.surface.set(1200.);
                host.panel
                    .layout(Some(&subject), files, 288., 440., 240., 6., cx)
                    .0
            })
            .unwrap()
    };
    assert_eq!(
        beside(&mut h, FilesSurface::Open),
        288.,
        "no file and another surface showing, the standard width"
    );
    assert!(
        layout(&mut h, 1200.).0 > 288.,
        "Files showing a root is wide before a file opens"
    );
    h.open("a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    // 6 + 420 + 440 + 240: the tree fits beside the file.
    let (width, widened) = layout(&mut h, 1106.);
    assert_eq!(width, 680.);
    assert!(widened.unwrap().tree);
    // Narrower: the file gives way to the mission first, and the tree stays.
    let (width, widened) = layout(&mut h, 1105.);
    assert_eq!(width, 679.);
    assert!(widened.unwrap().tree);
    assert_eq!(layout(&mut h, 1000.).0, 574.);
    assert_eq!(
        layout(&mut h, 600.).0,
        528.,
        "the file never below the standard width, the tree still shown"
    );
    assert!(layout(&mut h, 600.).1.unwrap().tree);

    // A file left open keeps the width while another surface shows, and
    // closing Files gives it back, with the file tabs kept for reopening.
    assert!(beside(&mut h, FilesSurface::Open) > 288.);
    assert_eq!(beside(&mut h, FilesSurface::Closed), 288.);
    assert!(h.read(|view| view.has_tabs()));
    assert!(beside(&mut h, FilesSurface::Shown) > 288.);
}

#[test]
fn a_saved_tree_width_widens_the_panel_and_narrows_the_file_sooner() {
    let mut h = viewer(mission("on-worktree"));
    let subject = PanelSubject::Mission("on-worktree".into());
    let layout = |h: &mut Harness, surface: f32| {
        h.host
            .read_with(&h.visual, |host, cx| {
                host.panel.surface.set(surface);
                host.panel.layout(
                    Some(&subject),
                    FilesSurface::Shown,
                    288.,
                    440.,
                    320.,
                    6.,
                    cx,
                )
            })
            .unwrap()
    };
    h.open("a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    // 6 + 420 + 440 + 320: the wider tree fits beside the file.
    let (width, widened) = layout(&mut h, 1186.);
    assert_eq!(width, 760.);
    assert!(widened.unwrap().tree);
    // Narrower: the file gives way to the mission, and the tree stays.
    let (width, widened) = layout(&mut h, 1185.);
    assert_eq!(width, 759.);
    assert!(widened.unwrap().tree);
    assert_eq!(layout(&mut h, 1000.).0, 608.);
    assert_eq!(layout(&mut h, 600.).0, 608.);
}

#[test]
fn dragging_the_tree_divider_sizes_the_tree_against_the_file_and_saves_both() {
    let mut h = viewer(chat("in-project"));
    h.visual.simulate_resize(size(px(800.), px(900.)));
    h.open("a.txt");
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
    h.open("a.txt");
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
    h.open("src/long.rs");
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
    h.open("data.bin");
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
    assert_eq!(h.tabs(), ["a.txt"]);
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
    h.open("src/tabs.rs");
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
    h.open("src/wide.rs");
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
    h.open("src/minified.js");
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
        view.open_file("a.txt".into(), cx);
        view.read_seq("a.txt").unwrap()
    });
    h.visual
        .update(|window, cx| view.update(cx, |view, cx| view.close_tab(0, window, cx)));
    let second = view.update(&mut h.visual, |view, cx| {
        view.open_file("a.txt".into(), cx);
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
    h.open("src/many.rs");
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
    h.open("a.txt");
    h.open("old.txt");
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
        assert!(!h.tabs().contains(&active), "⌘W closed the tab");
        assert_eq!(closed(&mut h), 0, "the pane stays");
        if !with_find {
            h.open("old.txt");
            h.wait("the file", |h| h.has("FILE_CODE"));
        }
    }
}

// ── Highlighting and Markdown (m5) ─────────────────────────────────────

impl Harness {
    fn highlighted(&mut self) -> bool {
        let code = self.code();
        code.read_with(&self.visual, |code, _| code.highlights().is_some())
    }

    /// The runs drawn for `line` in the last frame: text and colour.
    fn painted(&mut self, line: usize) -> Vec<(String, gpui::Hsla)> {
        let code = self.code();
        code.read_with(&self.visual, |code, _| {
            code.painted
                .borrow()
                .get(&line)
                .cloned()
                .unwrap_or_default()
        })
    }

    fn set_terminal_themes(
        &mut self,
        dark: crate::app_settings::DarkTerminalTheme,
        light: crate::app_settings::LightTerminalTheme,
    ) {
        self.store.update(&mut self.visual, |store, cx| {
            store.update_settings(
                |settings| {
                    settings.dark_terminal_theme = dark;
                    settings.light_terminal_theme = light;
                    true
                },
                false,
                cx,
            );
        });
        self.redraw();
    }
}

fn color(hex: u32) -> gpui::Hsla {
    gpui::rgb(hex).into()
}

#[test]
fn code_is_highlighted_in_the_terminal_palette_of_each_theme() {
    use crate::app_settings::{DarkTerminalTheme, LightTerminalTheme};
    let _theme = ThemeGuard::new();
    theme::set_active_variant(ThemeVariant::Carbon);
    let mut h = viewer(chat("in-project"));
    std::fs::write(
        h.repo.join("src/client.rs"),
        "pub struct Client {\n    reader: u32,\n}\n",
    )
    .unwrap();
    h.open("src/client.rs");
    h.wait("the highlights", |h| h.has("FILE_CODE") && h.highlighted());
    for (variant, dark, light, keyword, ty) in [
        // Appearance's defaults: Runner in dark, Rosé Pine Dawn in light.
        (
            ThemeVariant::Carbon,
            DarkTerminalTheme::Runner,
            LightTerminalTheme::RosePineDawn,
            0xc792ea,
            0xffcb6b,
        ),
        (
            ThemeVariant::RunnerLight,
            DarkTerminalTheme::Runner,
            LightTerminalTheme::RosePineDawn,
            0x286983,
            0x56949f,
        ),
        (
            ThemeVariant::Carbon,
            DarkTerminalTheme::CatppuccinMocha,
            LightTerminalTheme::RosePineDawn,
            0xcba6f7,
            0xf9e2af,
        ),
    ] {
        theme::set_active_variant(variant);
        h.set_terminal_themes(dark, light);
        let line = h.painted(0);
        assert!(
            line.contains(&("pub".into(), color(keyword))),
            "{variant:?} {line:?}"
        );
        assert!(
            line.contains(&("struct".into(), color(keyword))),
            "{line:?}"
        );
        assert!(line.contains(&("Client".into(), color(ty))), "{line:?}");
        assert!(
            line.contains(&(" ".into(), theme::text())),
            "plain text keeps the text colour: {line:?}"
        );
        let field = h.painted(1);
        assert!(
            field.contains(&("u32".into(), color(ty))),
            "type.builtin falls back to type: {field:?}"
        );
    }
}

#[test]
fn a_file_in_no_bundled_language_stays_plain() {
    let _theme = ThemeGuard::new();
    theme::set_active_variant(ThemeVariant::Carbon);
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("notes.txt"), "fn not code\n").unwrap();
    h.open("notes.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert!(!h.highlighted());
    assert_eq!(h.painted(0), [("fn not code".to_owned(), theme::text())]);
}

#[test]
fn a_ten_megabyte_file_is_highlighted_off_the_ui_thread_and_styles_only_visible_lines() {
    let _theme = ThemeGuard::new();
    theme::set_active_variant(ThemeVariant::Carbon);
    let mut h = viewer(chat("in-project"));
    let line = "let value: u32 = compute(1, \"text\"); // a note\n";
    let limit = runner_core::protocol::files::MAX_READ_BYTES as usize;
    let body = line.repeat(limit / line.len());
    assert!(body.len() > limit - line.len() && body.len() <= limit);
    std::fs::write(h.repo.join("src/big.rs"), &body).unwrap();
    h.open("src/big.rs");
    h.wait("the highlights", |h| h.has("FILE_CODE") && h.highlighted());
    let code = h.code();
    let spans =
        |h: &mut Harness| code.read_with(&h.visual, |code, _| code.highlights().cloned().unwrap());
    let first = spans(&mut h);
    let lines = code.read_with(&h.visual, |code, _| code.document().line_count());
    assert_eq!(first.line_count(), lines);

    // A reply lands and draws with no highlighting in its path: until the
    // background executor brings the new spans, the last ones stay.
    let read = h.read(|view| view.read_seq("src/big.rs")).unwrap();
    let view = h.view();
    view.update(&mut h.visual, |view, cx| {
        view.reply_text("src/big.rs", read, &body, cx)
    });
    assert!(
        Arc::ptr_eq(&spans(&mut h), &first),
        "a new text is never highlighted on the UI thread"
    );
    h.wait("the new highlights", |h| !Arc::ptr_eq(&spans(h), &first));

    let counts = crate::render_counts::RenderCounts::default();
    h.visual.update(|_, cx| cx.set_global(counts.clone()));
    counts.0.borrow_mut().clear();
    h.redraw();
    let styled = counts
        .0
        .borrow()
        .get("file-code-styled-lines")
        .copied()
        .unwrap_or(0);
    assert!(
        styled > 0 && styled < 100,
        "styled {styled} of {lines} lines"
    );
    assert!(h.painted(0).contains(&("let".into(), color(0xc792ea))));
}

#[test]
fn a_reload_highlights_the_new_text() {
    let _theme = ThemeGuard::new();
    theme::set_active_variant(ThemeVariant::Carbon);
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("src/edit.rs"), "fn one() {}\n").unwrap();
    h.open("src/edit.rs");
    h.wait("the highlights", |h| h.has("FILE_CODE") && h.highlighted());
    // FSEvents can miss a write in the instant its stream starts, so write
    // again until the watcher reports it.
    let path = h.repo.join("src/edit.rs");
    let mut touched = Instant::now() - Duration::from_secs(1);
    h.wait("the reload's highlights", |h| {
        if touched.elapsed() >= Duration::from_secs(1) {
            std::fs::write(&path, "// now a comment\nfn two() {}\n").unwrap();
            touched = Instant::now();
        }
        h.line(0) == "// now a comment"
            && h.painted(0) == [("// now a comment".to_owned(), color(0x6b6e7a))]
    });
    assert!(h.painted(1).contains(&("two".into(), color(0x82aaff))));
}

#[test]
fn find_matches_over_highlighted_code() {
    let _theme = ThemeGuard::new();
    theme::set_active_variant(ThemeVariant::Carbon);
    let mut h = viewer(chat("in-project"));
    std::fs::write(
        h.repo.join("src/find_hl.rs"),
        "struct Client;\nimpl Client {}\nfn make() -> Client { Client }\n",
    )
    .unwrap();
    h.open("src/find_hl.rs");
    h.wait("the highlights", |h| h.has("FILE_CODE") && h.highlighted());
    h.focus_code();
    h.visual.simulate_keystrokes(&primary("f"));
    h.settle();
    h.visual.simulate_input("Client");
    h.wait("the matches", |h| {
        h.read(|view| view.find_state())
            .is_some_and(|(count, _)| count.as_deref() == Some("1 of 4"))
    });
    let code = h.code();
    assert_eq!(
        code.read_with(&h.visual, |code, _| code.find_highlights()),
        (4, Some(0))
    );
    assert!(
        h.painted(0).contains(&("Client".into(), color(0xffcb6b))),
        "still highlighted"
    );
}

const README: &str = "# Runner\n\
\n\
A local *cockpit* with `runner mission start` and [the guide](docs/guide.md).\n\
\n\
![Screenshot](assets/shot.png)\n\
\n\
| Role | Slots |\n\
| :--- | ---: |\n\
| coder | 1 |\n\
\n\
- [x] highlighting\n\
- [ ] settings\n\
\n\
```rust\n\
fn main() {}\n\
```\n\
\n\
<p align=\"center\">centred</p>\n\
\n\
See [Zed](https://zed.dev) and [outside](../../outside.md).\n";

/// A PNG's first bytes: it has NULs, so runnerd sees a binary file.
const PNG: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01\x08\x06\0\0\0\x1f\x15\xc4\x89\0\0\0\rIDATx\x9cc\xf8\x0f\0\0\x01\x01\0\x05\x18\xd8N\0\0\0\0IEND\xaeB`\x82";

#[test]
fn markdown_opens_rendered_in_both_themes_with_its_blocks_and_local_images() {
    let _theme = ThemeGuard::new();
    let mut h = viewer(chat("in-project"));
    std::fs::create_dir_all(h.repo.join("assets")).unwrap();
    std::fs::write(h.repo.join("assets/shot.png"), PNG).unwrap();
    std::fs::write(h.repo.join("README.md"), README).unwrap();
    h.open("README.md");
    h.wait("the page", |h| h.has("FILE_MARKDOWN"));
    assert!(!h.has("FILE_CODE"), "Markdown opens rendered");
    for selector in [
        "FILE_MD_HEADING",
        "FILE_MD_TABLE",
        "FILE_MD_TASK_DONE",
        "FILE_MD_TASK",
        "FILE_MD_CODE",
        "FILE_MD_HTML",
        "FILE_MD_IMAGE",
        "FILE_MARKDOWN_SOURCE",
    ] {
        assert!(h.has(selector), "{selector}");
    }
    assert!(
        !h.has("FILE_MD_IMAGE_ALT"),
        "the PNG came through files_read"
    );
    for variant in [ThemeVariant::Carbon, ThemeVariant::RunnerLight] {
        theme::set_active_variant(variant);
        h.redraw();
        assert_text(&mut h.visual, "FILE_MD_HEADING", theme::text());
        assert_text(&mut h.visual, "FILE_MD_PARAGRAPH", theme::muted());
    }
    let page = h.read(|view| view.rendered_view()).unwrap();
    let rows = page.read_with(&h.visual, |page, _| page.page().rows.len());
    assert_eq!(
        rows, 10,
        "heading, paragraph, image, 2 table rows, 2 items, a code line, HTML, the links"
    );
}

#[test]
fn the_source_toggle_switches_both_ways_and_find_opens_the_source() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("README.md"), README).unwrap();
    h.open("README.md");
    h.wait("the page", |h| h.has("FILE_MARKDOWN"));
    h.click("FILE_MARKDOWN_SOURCE");
    assert!(h.has("FILE_CODE") && !h.has("FILE_MARKDOWN"));
    assert_eq!(h.line(0), "# Runner");
    h.wait("the source's highlights", |h| h.highlighted());
    assert!(h.painted(0).iter().any(|(text, _)| text == "Runner"));
    h.click("FILE_MARKDOWN_SOURCE");
    assert!(h.has("FILE_MARKDOWN") && !h.has("FILE_CODE"));

    // ⌘F over the page searches its source.
    let page = h.read(|view| view.rendered_view()).unwrap();
    let focus = page.read_with(&h.visual, |page, _| page.focus_handle().clone());
    h.visual.update(|window, cx| focus.focus(window, cx));
    h.settle();
    h.visual.simulate_keystrokes(&primary("f"));
    h.settle();
    assert!(h.has("FILE_FIND_BAR") && h.has("FILE_CODE"));
    h.visual.simulate_input("guide");
    h.wait("the matches", |h| {
        h.read(|view| view.find_state())
            .is_some_and(|(count, _)| count.as_deref() == Some("1 of 2"))
    });
    // Back to the page, find closes with the source.
    h.click("FILE_MARKDOWN_SOURCE");
    assert!(!h.has("FILE_FIND_BAR"));
    assert_eq!(h.read(|view| view.find_state()), None);
}

#[test]
fn links_open_the_browser_or_a_project_file_in_a_new_tab_and_never_leave_the_root() {
    let mut h = viewer(chat("in-project"));
    std::fs::create_dir_all(h.repo.join("docs")).unwrap();
    std::fs::write(h.repo.join("docs/guide.md"), "# Guide\n").unwrap();
    std::fs::write(h.repo.join("README.md"), README).unwrap();
    h.open("README.md");
    h.wait("the page", |h| h.has("FILE_MARKDOWN"));
    let view = h.view();
    view.update(&mut h.visual, |view, cx| {
        view.follow_link("https://zed.dev", cx)
    });
    assert_eq!(h.read(|view| view.opened_urls()), ["https://zed.dev"]);
    view.update(&mut h.visual, |view, cx| {
        view.follow_link("../../outside.md", cx)
    });
    view.update(&mut h.visual, |view, cx| view.follow_link("#install", cx));
    h.settle();
    assert_eq!(h.tabs(), ["README.md"], "nothing outside the root");
    view.update(&mut h.visual, |view, cx| {
        view.follow_link("docs/guide.md", cx)
    });
    h.wait("the linked file", |h| {
        h.has("FILE_MARKDOWN") && h.read(|view| view.active_tab()) == Some("docs/guide.md".into())
    });
    assert_eq!(
        h.tabs(),
        ["README.md", "docs/guide.md"],
        "a tab of its own beside the page"
    );
    let page = h.read(|view| view.rendered_view()).unwrap();
    assert_eq!(
        page.read_with(&h.visual, |page, _| page.path().to_owned()),
        "docs/guide.md"
    );
}

#[test]
fn a_rendered_page_reloads_on_change_keeping_its_place() {
    let mut h = viewer(chat("in-project"));
    let path = h.repo.join("NOTES.md");
    let body = (0..200)
        .map(|block| format!("Paragraph {block}.\n\n"))
        .collect::<String>();
    std::fs::write(&path, &body).unwrap();
    h.open("NOTES.md");
    h.wait("the page", |h| h.has("FILE_MARKDOWN"));
    let page = h.read(|view| view.rendered_view()).unwrap();
    page.update(&mut h.visual, |page, _| page.scroll_to_row(120));
    h.redraw();
    let mut touched = Instant::now() - Duration::from_secs(1);
    h.wait("the reload", |h| {
        if touched.elapsed() >= Duration::from_secs(1) {
            std::fs::write(&path, format!("# Edited\n\n{body}")).unwrap();
            touched = Instant::now();
        }
        page.read_with(&h.visual, |page, _| page.page().rows.len()) == 201
    });
    assert_eq!(
        page.read_with(&h.visual, |page, _| page.scroll_top().item_ix),
        120,
        "the reload keeps the block in view"
    );
}

#[test]
fn a_long_code_block_list_and_table_draw_only_their_rows_in_view() {
    let mut h = viewer(chat("in-project"));
    let mut text = String::from("```rust\n");
    for line in 0..20_000 {
        text.push_str(&format!("let v{line} = {line};\n"));
    }
    text.push_str("```\n\n");
    for item in 0..5_000 {
        text.push_str(&format!("- item {item}\n"));
    }
    text.push_str("\n| a | b |\n| - | - |\n");
    for row in 0..5_000 {
        text.push_str(&format!("| {row} | x |\n"));
    }
    std::fs::write(h.repo.join("BIG.md"), &text).unwrap();
    h.open("BIG.md");
    h.wait("the page", |h| h.has("FILE_MARKDOWN"));
    let page = h.read(|view| view.rendered_view()).unwrap();
    let rows = page.read_with(&h.visual, |page, _| page.page().rows.len());
    assert_eq!(rows, 20_000 + 5_000 + 5_001);
    let counts = crate::render_counts::RenderCounts::default();
    h.visual.update(|_, cx| cx.set_global(counts.clone()));
    for row in [0, 10_000, 22_000, 28_000] {
        page.update(&mut h.visual, |page, _| page.scroll_to_row(row));
        counts.0.borrow_mut().clear();
        h.redraw();
        let drawn = counts
            .0
            .borrow()
            .get("file-markdown-rows")
            .copied()
            .unwrap_or(0);
        assert!(drawn > 0 && drawn < 100, "drew {drawn} rows at {row}");
    }
}

#[test]
fn linked_and_table_images_load_and_a_linked_image_opens_its_link() {
    let mut h = viewer(chat("in-project"));
    std::fs::create_dir_all(h.repo.join("assets")).unwrap();
    std::fs::create_dir_all(h.repo.join("docs")).unwrap();
    std::fs::write(h.repo.join("assets/shot.png"), PNG).unwrap();
    std::fs::write(h.repo.join("docs/guide.md"), "# Guide\n").unwrap();
    std::fs::write(
        h.repo.join("TABLE.md"),
        "| Logo |\n| --- |\n| ![Shot](assets/shot.png) |\n",
    )
    .unwrap();
    std::fs::write(
        h.repo.join("assets/logo.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40"/></svg>"#,
    )
    .unwrap();
    std::fs::write(
        h.repo.join("PICS.md"),
        "[![Logo](assets/logo.svg)](docs/guide.md)\n",
    )
    .unwrap();
    h.open("TABLE.md");
    h.wait("the table's image", |h| {
        h.has("FILE_MD_TABLE") && h.has("FILE_MD_IMAGE")
    });
    assert!(!h.has("FILE_MD_IMAGE_ALT"));
    h.open("PICS.md");
    h.wait("the linked image", |h| {
        h.read(|view| view.active_tab()) == Some("PICS.md".into()) && h.has("FILE_MD_IMAGE")
    });
    let page = h.read(|view| view.rendered_view()).unwrap();
    let images = page.read_with(&h.visual, |page, _| page.page().images());
    assert_eq!(images, ["assets/logo.svg"]);
    h.click("FILE_MD_IMAGE");
    h.wait("the linked file", |h| {
        h.read(|view| view.active_tab()) == Some("docs/guide.md".into())
    });
    assert_eq!(h.tabs().len(), 3);
}

#[test]
fn a_deleted_rendered_page_keeps_closing_as_its_only_action() {
    let mut h = viewer(chat("in-project"));
    std::fs::create_dir_all(h.repo.join("assets")).unwrap();
    std::fs::create_dir_all(h.repo.join("docs")).unwrap();
    std::fs::write(
        h.repo.join("assets/logo.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40"/></svg>"#,
    )
    .unwrap();
    std::fs::write(h.repo.join("docs/guide.md"), "# Guide\n").unwrap();
    let path = h.repo.join("GONE.md");
    std::fs::write(
        &path,
        "[![Logo](assets/logo.svg)](docs/guide.md)\n\nSee [Zed](https://zed.dev).\n",
    )
    .unwrap();
    h.open("GONE.md");
    h.wait("the page", |h| h.has("FILE_MD_IMAGE"));
    // FSEvents can miss a change in the instant its stream starts: write
    // until the watcher reports one, then delete.
    let page = h.read(|view| view.rendered_view()).unwrap();
    let mut touched = Instant::now() - Duration::from_secs(1);
    h.wait("the watcher", |h| {
        if touched.elapsed() >= Duration::from_secs(1) {
            std::fs::write(
                &path,
                "[![Logo](assets/logo.svg)](docs/guide.md)\n\nSee [Zed](https://zed.dev).\n\nMore.\n",
            )
            .unwrap();
            touched = Instant::now();
        }
        page.read_with(&h.visual, |page, _| page.page().rows.len()) == 3
    });
    std::fs::remove_file(&path).unwrap();
    h.wait("the deleted state", |h| h.has("FILE_DELETED"));
    assert!(h.has("FILE_MARKDOWN"), "the page stays, dimmed");
    h.click("FILE_MD_IMAGE");
    let view = h.view();
    view.update(&mut h.visual, |view, cx| {
        view.follow_link("https://zed.dev", cx)
    });
    h.settle();
    assert_eq!(h.tabs(), ["GONE.md"], "no link opens");
    assert!(h.read(|view| view.opened_urls()).is_empty());
    h.click("FILE_MARKDOWN_SOURCE");
    assert!(
        h.has("FILE_MARKDOWN") && !h.has("FILE_CODE"),
        "the toggle does nothing"
    );
    h.click("FILE_DELETED_CLOSE");
    assert!(h.tabs().is_empty() && !h.has("FILE_MARKDOWN"));
}

#[test]
fn a_drag_selects_rendered_text_across_rows_and_copies_it() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(
        h.repo.join("SELECT.md"),
        "First paragraph here.\n\n```rust\nfn main() {}\nlet x = 1;\n```\n",
    )
    .unwrap();
    h.open("SELECT.md");
    h.wait("the page", |h| h.has("FILE_MARKDOWN"));
    let first = h.visual.debug_bounds("FILE_MD_TEXT 0").unwrap();
    let middle = h.visual.debug_bounds("FILE_MD_TEXT 1").unwrap();
    let last = h.visual.debug_bounds("FILE_MD_TEXT 2").unwrap();
    let start = gpui::point(first.left() + px(1.), first.center().y);
    let end = gpui::point(last.right() - px(2.), last.center().y);
    h.visual
        .simulate_mouse_down(start, gpui::MouseButton::Left, Modifiers::default());
    for point in [first.center(), middle.center(), last.center(), end] {
        h.visual
            .simulate_mouse_move(point, Some(gpui::MouseButton::Left), Modifiers::default());
        h.settle();
    }
    h.visual
        .simulate_mouse_up(end, gpui::MouseButton::Left, Modifiers::default());
    h.settle();
    h.visual.simulate_keystrokes(&primary("c"));
    h.settle();
    let copied = h
        .visual
        .read_from_clipboard()
        .and_then(|item| item.text())
        .expect("copied text");
    assert_eq!(copied, "First paragraph here.\nfn main() {}\nlet x = 1;");

    let page = h.read(|view| view.rendered_view()).unwrap();
    page.update(&mut h.visual, |page, cx| {
        use super::markdown::Position;
        page.select(
            Position { row: 1, offset: 3 },
            Position { row: 1, offset: 7 },
            cx,
        )
    });
    assert_eq!(
        page.read_with(&h.visual, |page, _| page.selected_text()),
        Some("main".into())
    );
}

#[test]
fn a_single_long_code_line_shapes_only_the_rows_in_view() {
    let mut h = viewer(chat("in-project"));
    let line = "let x = 1; ".repeat(500_000);
    std::fs::write(h.repo.join("LINE.md"), format!("```\n{line}\n```\n")).unwrap();
    h.open("LINE.md");
    h.wait("the page", |h| h.has("FILE_MARKDOWN"));
    let page = h.read(|view| view.rendered_view()).unwrap();
    let rows = page.read_with(&h.visual, |page, _| page.page().rows.len());
    assert!(rows > 10_000, "{rows} rows");
    for row in [0, rows / 2, rows - 1] {
        page.update(&mut h.visual, |page, _| page.scroll_to_row(row));
        super::markdown::DRAWN_CODE_BYTES.set(0);
        h.redraw();
        let drawn = super::markdown::DRAWN_CODE_BYTES.get();
        assert!(
            drawn > 0 && drawn < 100 * 520,
            "shaped {drawn} bytes at {row}"
        );
    }
}

#[test]
fn list_markers_line_up_with_their_text_after_other_blocks() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(
        h.repo.join("LIST.md"),
        "A paragraph first.\n\n- one\n- two\n\nAnother.\n\n- [x] done\n- [ ] todo\n\n1. first\n",
    )
    .unwrap();
    h.open("LIST.md");
    h.wait("the page", |h| h.has("FILE_MARKDOWN"));
    for row in [1, 2, 4, 5, 6] {
        let marker = h
            .visual
            .debug_bounds(format!("FILE_MD_MARKER {row}").leak())
            .unwrap_or_else(|| panic!("marker {row}"));
        let text = h
            .visual
            .debug_bounds(format!("FILE_MD_TEXT {row}").leak())
            .unwrap_or_else(|| panic!("text {row}"));
        assert_eq!(marker.top(), text.top(), "row {row}");
    }
}

#[test]
fn each_linked_image_in_a_table_row_opens_its_own_link() {
    let mut h = viewer(chat("in-project"));
    std::fs::create_dir_all(h.repo.join("docs")).unwrap();
    for name in ["a", "b"] {
        std::fs::write(h.repo.join(format!("docs/{name}.md")), "# Doc\n").unwrap();
        std::fs::write(
            h.repo.join(format!("docs/{name}.svg")),
            r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40"/></svg>"#,
        )
        .unwrap();
    }
    std::fs::write(
        h.repo.join("ROW.md"),
        "| Links |\n| --- |\n| [![A](docs/a.svg)](docs/a.md) [![B](docs/b.svg)](docs/b.md) |\n",
    )
    .unwrap();
    h.open("ROW.md");
    h.wait("the images", |h| h.has("FILE_MD_IMAGE_LINK docs/b.md"));
    h.click("FILE_MD_IMAGE_LINK docs/a.md");
    h.wait("the first link", |h| {
        h.read(|view| view.active_tab()) == Some("docs/a.md".into())
    });
    h.open("ROW.md");
    h.wait("the page again", |h| h.has("FILE_MD_IMAGE_LINK docs/b.md"));
    h.click("FILE_MD_IMAGE_LINK docs/b.md");
    h.wait("the second link", |h| {
        h.read(|view| view.active_tab()) == Some("docs/b.md".into())
    });
}

#[test]
fn a_drag_over_a_link_selects_it_and_a_click_opens_it() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("LINK.md"), "[the Zed site](https://zed.dev)\n").unwrap();
    h.open("LINK.md");
    h.wait("the page", |h| h.has("FILE_MARKDOWN"));
    let text = h.visual.debug_bounds("FILE_MD_TEXT 0").unwrap();
    let start = gpui::point(text.left() + px(1.), text.center().y);
    let end = gpui::point(text.left() + px(60.), text.center().y);
    h.visual
        .simulate_mouse_down(start, gpui::MouseButton::Left, Modifiers::default());
    h.visual
        .simulate_mouse_move(end, Some(gpui::MouseButton::Left), Modifiers::default());
    h.settle();
    h.visual
        .simulate_mouse_up(end, gpui::MouseButton::Left, Modifiers::default());
    h.settle();
    assert!(
        h.read(|view| view.opened_urls()).is_empty(),
        "a drag selects, it does not open"
    );
    h.visual.simulate_keystrokes(&primary("c"));
    h.settle();
    let copied = h
        .visual
        .read_from_clipboard()
        .and_then(|item| item.text())
        .unwrap_or_default();
    assert!(
        !copied.is_empty() && "the Zed site".starts_with(&copied),
        "{copied:?}"
    );

    h.visual.simulate_click(
        gpui::point(text.left() + px(20.), text.center().y),
        Modifiers::default(),
    );
    h.settle();
    assert_eq!(h.read(|view| view.opened_urls()), ["https://zed.dev"]);
}

#[test]
fn a_drag_over_a_table_link_or_linked_image_selects_and_a_click_opens() {
    let mut h = viewer(chat("in-project"));
    std::fs::create_dir_all(h.repo.join("docs")).unwrap();
    std::fs::write(h.repo.join("docs/a.md"), "# A\n").unwrap();
    std::fs::write(
        h.repo.join("docs/a.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40"/></svg>"#,
    )
    .unwrap();
    std::fs::write(
        h.repo.join("TLINK.md"),
        "| Site | Logo |\n| --- | --- |\n| [the Zed site](https://zed.dev) | [![A](docs/a.svg)](docs/a.md) |\n",
    )
    .unwrap();
    h.open("TLINK.md");
    h.wait("the table", |h| h.has("FILE_MD_IMAGE_LINK docs/a.md"));
    let drag = |h: &mut Harness, from: gpui::Point<gpui::Pixels>, to: gpui::Point<gpui::Pixels>| {
        h.visual
            .simulate_mouse_down(from, gpui::MouseButton::Left, Modifiers::default());
        h.visual
            .simulate_mouse_move(to, Some(gpui::MouseButton::Left), Modifiers::default());
        h.settle();
        h.visual
            .simulate_mouse_up(to, gpui::MouseButton::Left, Modifiers::default());
        h.settle();
    };
    let copied = |h: &mut Harness| {
        h.visual.simulate_keystrokes(&primary("c"));
        h.settle();
        h.visual
            .read_from_clipboard()
            .and_then(|item| item.text())
            .unwrap_or_default()
    };

    // The text link: a drag selects the row and opens nothing.
    let cell = h.visual.debug_bounds("FILE_MD_CELL 1-0").unwrap();
    let link = gpui::point(cell.left() + px(14.), cell.center().y);
    drag(&mut h, link, link + gpui::point(px(30.), px(0.)));
    assert!(h.read(|view| view.opened_urls()).is_empty());
    assert_eq!(copied(&mut h), "the Zed site\tA");

    // The linked image: the same.
    let image = h
        .visual
        .debug_bounds("FILE_MD_IMAGE_LINK docs/a.md")
        .unwrap();
    drag(
        &mut h,
        image.center(),
        image.center() + gpui::point(px(10.), px(0.)),
    );
    assert_eq!(h.tabs(), ["TLINK.md"], "no file opens");
    assert_eq!(copied(&mut h), "the Zed site\tA");

    // Plain clicks open both.
    h.visual.simulate_click(link, Modifiers::default());
    h.settle();
    assert_eq!(h.read(|view| view.opened_urls()), ["https://zed.dev"]);
    h.click("FILE_MD_IMAGE_LINK docs/a.md");
    h.wait("the linked file", |h| {
        h.read(|view| view.active_tab()) == Some("docs/a.md".into())
    });
}

#[test]
fn a_selection_left_elsewhere_does_not_block_clicking_a_linked_image() {
    let mut h = viewer(chat("in-project"));
    std::fs::create_dir_all(h.repo.join("docs")).unwrap();
    std::fs::write(h.repo.join("docs/a.md"), "# A\n").unwrap();
    std::fs::write(
        h.repo.join("docs/a.svg"),
        r#"<svg xmlns="http://www.w3.org/2000/svg" width="80" height="40"><rect width="80" height="40"/></svg>"#,
    )
    .unwrap();
    std::fs::write(
        h.repo.join("SEL.md"),
        "Some text to select first.\n\n[![A](docs/a.svg)](docs/a.md)\n",
    )
    .unwrap();
    h.open("SEL.md");
    h.wait("the image", |h| h.has("FILE_MD_IMAGE_LINK docs/a.md"));
    let text = h.visual.debug_bounds("FILE_MD_TEXT 0").unwrap();
    let (from, to) = (
        gpui::point(text.left() + px(1.), text.center().y),
        gpui::point(text.left() + px(80.), text.center().y),
    );
    h.visual
        .simulate_mouse_down(from, gpui::MouseButton::Left, Modifiers::default());
    h.visual
        .simulate_mouse_move(to, Some(gpui::MouseButton::Left), Modifiers::default());
    h.settle();
    h.visual
        .simulate_mouse_up(to, gpui::MouseButton::Left, Modifiers::default());
    h.settle();
    let page = h.read(|view| view.rendered_view()).unwrap();
    assert!(page
        .read_with(&h.visual, |page, _| page.selected_text())
        .is_some_and(|text| !text.is_empty()));
    h.click("FILE_MD_IMAGE_LINK docs/a.md");
    h.wait("the linked file", |h| {
        h.read(|view| view.active_tab()) == Some("docs/a.md".into())
    });
}

// ── Settings → Side panel → Files ──────────────────────────────────────

impl Harness {
    fn set(&mut self, update: impl FnOnce(&mut crate::app_settings::AppSettings)) {
        self.store.update(&mut self.visual, |store, cx| {
            store.update_settings(
                |settings| {
                    update(settings);
                    true
                },
                false,
                cx,
            );
        });
        self.settle();
    }
}

#[test]
fn markdown_set_to_open_as_source_opens_the_highlighted_source() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("README.md"), README).unwrap();
    h.set(|settings| settings.markdown_open_as = crate::app_settings::MarkdownOpenAs::Source);
    h.open("README.md");
    h.wait("the source", |h| h.has("FILE_CODE"));
    assert!(!h.has("FILE_MARKDOWN"), "Markdown opens as its source");
    assert_eq!(h.line(0), "# Runner");
    // `</>` still switches the tab to the page.
    h.wait("the page beside it", |h| h.has("FILE_MARKDOWN_SOURCE"));
    h.click("FILE_MARKDOWN_SOURCE");
    h.wait("the page", |h| h.has("FILE_MARKDOWN"));
    assert!(!h.has("FILE_CODE"));
}

#[test]
fn code_is_highlighted_and_a_reload_highlights_the_new_text() {
    let _theme = ThemeGuard::new();
    theme::set_active_variant(ThemeVariant::Carbon);
    let mut h = viewer(chat("in-project"));
    std::fs::write(
        h.repo.join("src/client.rs"),
        "pub struct Client {\n    reader: u32,\n}\n",
    )
    .unwrap();
    h.open("src/client.rs");
    h.wait("the highlights", |h| h.highlighted());
    h.redraw();
    assert!(
        h.painted(0).contains(&("pub".into(), color(0xc792ea))),
        "{:?}",
        h.painted(0)
    );

    // FSEvents can miss a write in the instant its stream starts, so write
    // again until the watcher reports it.
    let path = h.repo.join("src/client.rs");
    let mut touched = Instant::now() - Duration::from_secs(1);
    h.wait("the reload", |h| {
        if touched.elapsed() >= Duration::from_secs(1) {
            std::fs::write(&path, "// note\nfn go() {}\n").unwrap();
            touched = Instant::now();
        }
        h.line(0) == "// note"
    });
    h.wait("the new highlights", |h| {
        h.redraw();
        h.painted(1).contains(&("fn".into(), color(0xc792ea)))
    });
}

#[test]
fn the_viewer_font_size_follows_the_terminal_until_set() {
    let mut h = viewer(chat("in-project"));
    h.open("a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    let line_height = |h: &mut Harness| {
        h.redraw();
        let code = h.code();
        code.read_with(&h.visual, |code, _| code.line_height())
            .map(f32::from)
    };
    let expected = |size: f32| {
        crate::terminal::element::row_height(
            size,
            crate::app_settings::TERMINAL_LINE_HEIGHT_DEFAULT,
        )
    };
    assert_eq!(line_height(&mut h), Some(expected(13.)));
    h.set(|settings| settings.terminal_font_size = 16);
    assert_eq!(
        line_height(&mut h),
        Some(expected(16.)),
        "follows the terminal"
    );
    h.set(|settings| settings.files_font_size = Some(19));
    assert_eq!(line_height(&mut h), Some(expected(19.)), "its own size");
    h.set(|settings| settings.terminal_font_size = 11);
    assert_eq!(
        line_height(&mut h),
        Some(expected(19.)),
        "no longer follows"
    );
    h.set(|settings| settings.files_font_size = None);
    assert_eq!(
        line_height(&mut h),
        Some(expected(11.)),
        "an unset size follows the terminal"
    );
}

#[test]
fn the_tree_draws_the_chosen_icon_set() {
    use crate::file_icons::IconSet;
    let mut h = viewer(chat("in-project"));
    assert_eq!(
        h.read(|view| view.look.as_ref().map(|look| look.icons)),
        Some(IconSet::Catppuccin)
    );
    for set in [IconSet::Material, IconSet::Minimal, IconSet::Catppuccin] {
        h.set(|settings| settings.file_icons = set);
        assert_eq!(
            h.read(|view| view.look.as_ref().map(|look| look.icons)),
            Some(set)
        );
        assert!(h.has("FILES_ROW a.txt"));
    }
}

#[test]
fn a_chat_widens_by_the_same_rule_as_a_mission() {
    let mut h = viewer(chat("in-project"));
    let subject = PanelSubject::Session("in-project".into());
    let layout = |h: &mut Harness, surface: f32| {
        h.host
            .read_with(&h.visual, |host, cx| {
                host.panel.surface.set(surface);
                host.panel.layout(
                    Some(&subject),
                    FilesSurface::Shown,
                    288.,
                    440.,
                    240.,
                    6.,
                    cx,
                )
            })
            .unwrap()
    };
    h.open("a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert_eq!(layout(&mut h, 1106.).0, 680.);
    // 6 + 420 + 334 + 240: the file narrows, the tree stays.
    let (width, widened) = layout(&mut h, 1000.);
    assert_eq!(width, 574.);
    assert!(
        widened.unwrap().tree,
        "the tree stays while the file narrows"
    );
    assert_eq!(layout(&mut h, 600.).0, 528.);
    h.click("FILE_TREE_TOGGLE");
    let (width, widened) = layout(&mut h, 1000.);
    assert_eq!(width, 440., "the tree's room goes to the file");
    assert!(!widened.unwrap().tree);
}

fn link(path: PathBuf, line: Option<u32>) -> crate::file_links::FileLinkTarget {
    crate::file_links::FileLinkTarget {
        path,
        line,
        column: None,
    }
}

fn launched() -> Vec<crate::file_links::FileLinkTarget> {
    crate::file_links::LAUNCHED.with(|launched| {
        launched
            .borrow()
            .iter()
            .map(|(_, target)| target.clone())
            .collect()
    })
}

impl Harness {
    /// ⌘-clicks `target` in a terminal beside `view`; the count rises each
    /// time the link asks the owner to show the panel.
    fn click_link(
        &mut self,
        view: &Entity<FilesView>,
        target: crate::file_links::FileLinkTarget,
    ) -> std::rc::Rc<std::cell::Cell<usize>> {
        let shown = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = shown.clone();
        view.update(&mut self.visual, |view, cx| {
            view.open_terminal_link(
                target,
                std::rc::Rc::new(move |_| count.set(count.get() + 1)),
                cx,
            )
        });
        shown
    }
}

#[test]
fn a_terminal_link_inside_the_root_opens_a_tab_at_its_line() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("lines.txt"), "one\ntwo\nthree\nfour\n").unwrap();
    crate::file_links::LAUNCHED.with(|launched| launched.borrow_mut().clear());
    let view = h.view();
    // The temporary folder as the terminal names it: through macOS's
    // `/var` symlink, with a `..` on the way.
    let path = h.temp.path().join("runner/src/../lines.txt");
    let shown = h.click_link(&view, link(path, Some(3)));
    let selected = |h: &mut Harness, text: &str| {
        h.read(|view| view.code_view()).is_some_and(|code| {
            code.read_with(&h.visual, |code, _| code.selected_text()) == Some(text.into())
        })
    };
    h.wait("the cited line", |h| selected(h, "three"));
    assert_eq!(h.read(|view| view.open_tabs()), ["lines.txt"]);
    assert_eq!(shown.get(), 1, "the owner shows the panel");
    assert!(launched().is_empty());
    assert!(h.has("FILES_TREE_COLUMN"));

    // A second link opens a tab of its own, and the first one stays.
    let shown = h.click_link(&view, link(h.repo.join("a.txt"), None));
    h.wait("the next file", |h| {
        h.read(|view| view.active_tab()) == Some("a.txt".into())
    });
    assert_eq!(h.read(|view| view.open_tabs()), ["lines.txt", "a.txt"]);
    assert_eq!(shown.get(), 1);

    // A link to a file already open switches to its tab, at the new line.
    h.click_link(&view, link(h.repo.join("lines.txt"), Some(1)));
    h.wait("the open file at line 1", |h| {
        h.read(|view| view.active_tab()) == Some("lines.txt".into()) && selected(h, "one")
    });
    assert_eq!(h.read(|view| view.open_tabs()), ["lines.txt", "a.txt"]);
}

#[test]
fn a_terminal_link_cited_at_a_line_in_markdown_shows_the_source_there() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("notes.md"), "# Notes\n\nfirst\nsecond\n").unwrap();
    let view = h.view();
    h.click_link(&view, link(h.repo.join("notes.md"), Some(4)));
    h.wait("the source", |h| {
        h.read(|view| view.code_view()).is_some_and(|code| {
            code.read_with(&h.visual, |code, _| code.selected_text()) == Some("second".into())
        })
    });
    assert!(h.read(|view| view.rendered_view()).is_none());
    // Without a line, it opens as the Markdown setting says: rendered.
    std::fs::write(h.repo.join("plain.md"), "# Plain\n").unwrap();
    h.click_link(&view, link(h.repo.join("plain.md"), None));
    h.wait("the page", |h| {
        h.read(|view| view.rendered_view()).is_some()
    });
    assert_eq!(h.read(|view| view.active_tab()), Some("plain.md".into()));
}

#[test]
fn a_terminal_link_outside_the_root_opens_the_editor() {
    let mut h = viewer(chat("in-project"));
    let outside = h.temp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("note.txt"), "note\n").unwrap();
    let view = h.view();
    // A symlink inside the root that leads out of it.
    #[cfg(unix)]
    std::os::unix::fs::symlink(&outside, h.repo.join("escape")).unwrap();
    let targets = [
        Some(link(outside.join("note.txt"), Some(1))),
        // `..` that leaves the root.
        Some(link(h.repo.join("src/../../outside/note.txt"), None)),
        cfg!(unix).then(|| link(h.repo.join("escape/note.txt"), Some(2))),
    ];
    for target in targets.into_iter().flatten() {
        crate::file_links::LAUNCHED.with(|launched| launched.borrow_mut().clear());
        let shown = h.click_link(&view, target.clone());
        h.wait("the editor", |_| !launched().is_empty());
        assert_eq!(launched(), [target]);
        assert_eq!(shown.get(), 0);
        assert!(h.read(|view| view.open_tabs()).is_empty());
    }
}

#[test]
fn a_terminal_link_waits_for_a_root_still_resolving() {
    let mut h = harness(None);
    std::fs::write(h.repo.join("lines.txt"), "one\ntwo\n").unwrap();
    let view = h
        .host
        .update(&mut h.visual, |host, _, cx| {
            host.panel.link_view(&chat("in-project"), cx)
        })
        .unwrap();
    assert_eq!(view.read_with(&h.visual, |view, _| view.state()), "loading");
    let shown = h.click_link(&view, link(h.repo.join("lines.txt"), Some(2)));
    h.wait("the preview", |h| {
        view.read_with(&h.visual, |view, _| view.active_tab()) == Some("lines.txt".into())
    });
    assert_eq!(shown.get(), 1);
    // Not shown, so not watching.
    assert_eq!(
        view.read_with(&h.visual, |view, _| view.watched().map(Path::to_path_buf)),
        None
    );
}

#[test]
fn a_remote_or_rootless_view_sends_links_to_the_editor() {
    let mut h = harness(None);
    std::fs::write(h.repo.join("lines.txt"), "one\n").unwrap();
    for context in [
        PanelContext {
            remote: true,
            ..chat("in-project")
        },
        chat("outside"),
    ] {
        crate::file_links::LAUNCHED.with(|launched| launched.borrow_mut().clear());
        let view = h
            .host
            .update(&mut h.visual, |host, _, cx| {
                host.panel.link_view(&context, cx)
            })
            .unwrap();
        let target = link(h.repo.join("lines.txt"), Some(1));
        let shown = h.click_link(&view, target.clone());
        h.wait("the editor", |_| !launched().is_empty());
        assert_eq!(launched(), [target], "{context:?}");
        assert_eq!(shown.get(), 0);
    }
}

#[test]
fn whether_a_link_previews_is_worked_out_off_the_ui_thread() {
    let mut h = viewer(chat("in-project"));
    let outside = h.temp.path().join("outside.txt");
    std::fs::write(&outside, "x\n").unwrap();
    let view = h.view();
    for (path, previews) in [(h.repo.join("a.txt"), true), (outside, false)] {
        let redraws = std::rc::Rc::new(std::cell::Cell::new(0));
        let count = redraws.clone();
        let redraw: crate::surfaces::files::ShowPreview =
            std::rc::Rc::new(move |_| count.set(count.get() + 1));
        let since = std::time::Instant::now();
        let first = view.update(&mut h.visual, |view, cx| {
            view.link_previews(&path, since, redraw.clone(), cx)
        });
        assert_eq!(first, None, "unknown until checked");
        h.wait("the answer", |_| redraws.get() == 1);
        let known = view.update(&mut h.visual, |view, cx| {
            view.link_previews(&path, since, redraw.clone(), cx)
        });
        assert_eq!(known, Some(previews), "{path:?}");
    }
}

#[test]
fn picking_from_open_in_saves_the_editor_links_fall_back_to() {
    use crate::app_settings::FileLinkEditor;
    let mut h = viewer(chat("in-project"));
    h.store.update(&mut h.visual, |store, cx| {
        store.update_settings(
            |settings| {
                settings.file_link_editor = FileLinkEditor::Zed;
                true
            },
            false,
            cx,
        )
    });
    h.open("a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.click("FILE_OPEN_IN_MENU");
    h.click("FILE_OPEN_IN_ITEM file-open-in-default 0");
    assert_eq!(
        h.store
            .read_with(&h.visual, |store, _| store.settings.file_link_editor),
        FileLinkEditor::DefaultApp
    );
    let launched = h.read(|view| view.launched());
    let [(editor, path, line)] = launched.as_slice() else {
        panic!("one launch: {launched:?}");
    };
    assert_eq!(
        (*editor, *line),
        (Some(FileLinkEditor::DefaultApp), Some(1))
    );
    // The root as git names it; on Windows another spelling of the folder.
    assert_same(Some(path), &h.repo.join("a.txt"));
    assert_eq!(
        *path,
        path.components().collect::<PathBuf>(),
        "a native path"
    );
}

#[test]
fn a_file_path_in_the_root_uses_this_computers_separators() {
    use super::view::absolute;
    if cfg!(windows) {
        assert_eq!(
            absolute(Path::new("C:/Users/runneradmin/runner"), "src//lib.rs"),
            PathBuf::from(r"C:\Users\runneradmin\runner\src\lib.rs")
        );
    } else {
        assert_eq!(
            absolute(Path::new("/home/runner"), "src//lib.rs"),
            PathBuf::from("/home/runner/src/lib.rs")
        );
    }
}

#[cfg(unix)]
#[test]
fn a_retargeted_symlink_is_checked_again_on_the_next_click_and_hover() {
    let mut h = harness(None);
    let outside = h.temp.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("far.rs"), "far\n").unwrap();
    std::fs::write(h.repo.join("b.txt"), "b\n").unwrap();
    let current = h.repo.join("current.rs");
    let point = |to: &Path| {
        let _ = std::fs::remove_file(&current);
        std::os::unix::fs::symlink(to, &current).unwrap();
    };
    // A hidden panel's view: not shown, so not watching.
    let view = h
        .host
        .update(&mut h.visual, |host, _, cx| {
            host.panel.link_view(&chat("in-project"), cx)
        })
        .unwrap();
    h.wait("the root", |h| {
        view.read_with(&h.visual, |view, _| view.state()) == "root"
    });
    let hover = |h: &mut Harness| {
        let since = std::time::Instant::now();
        loop {
            let answer = view.update(&mut h.visual, |view, cx| {
                view.link_previews(&current, since, std::rc::Rc::new(|_| {}), cx)
            });
            if let Some(answer) = answer {
                return answer;
            }
            h.settle();
            std::thread::sleep(Duration::from_millis(10));
        }
    };
    let click = |h: &mut Harness| {
        crate::file_links::LAUNCHED.with(|launched| launched.borrow_mut().clear());
        h.click_link(&view, link(current.clone(), None));
        h.wait("the click", |h| {
            !launched().is_empty()
                || view
                    .read_with(&h.visual, |view, _| view.active_tab())
                    .is_some()
        });
    };
    let active = |h: &mut Harness| view.read_with(&h.visual, |view, _| view.active_tab());

    point(&h.repo.join("a.txt"));
    assert!(hover(&mut h));
    click(&mut h);
    assert_eq!(active(&mut h), Some("a.txt".into()));

    // Inside to inside: the next click opens the new target.
    point(&h.repo.join("b.txt"));
    assert!(hover(&mut h));
    click(&mut h);
    h.wait("b.txt", |h| active(h) == Some("b.txt".into()));

    // Inside to outside: the editor.
    point(&outside.join("far.rs"));
    assert!(!hover(&mut h));
    click(&mut h);
    assert_eq!(launched(), [link(current.clone(), None)]);
    assert_eq!(active(&mut h), Some("b.txt".into()));

    // Outside to inside: the preview again.
    point(&h.repo.join("a.txt"));
    assert!(hover(&mut h));
    click(&mut h);
    h.wait("a.txt", |h| active(h) == Some("a.txt".into()));
}

#[test]
fn a_cited_line_stays_in_view_with_find_open() {
    let mut h = viewer(chat("in-project"));
    h.visual.simulate_resize(size(px(900.), px(400.)));
    let lines = |marker: &str| {
        (1..=400)
            .map(|line| {
                if line == 2 {
                    "needle near the top".to_owned()
                } else if line == 380 {
                    marker.to_owned()
                } else {
                    format!("line {line}")
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    };
    std::fs::write(h.repo.join("long.txt"), lines("cited one")).unwrap();
    std::fs::write(h.repo.join("other.txt"), lines("cited two")).unwrap();
    // Find open over an existing tab, with a match near the top.
    h.open("long.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.focus_code();
    h.visual.simulate_keystrokes(&primary("f"));
    h.settle();
    h.visual.simulate_input("needle");
    h.wait("the match", |h| {
        h.read(|view| view.find_state())
            .and_then(|(count, _)| count)
            .is_some()
    });
    let view = h.view();
    for (file, cited) in [("long.txt", "cited one"), ("other.txt", "cited two")] {
        h.click_link(&view, link(h.repo.join(file), Some(380)));
        h.wait("the cited line", |h| {
            h.read(|view| view.active_tab()) == Some(file.into())
                && h.read(|view| view.code_view()).is_some_and(|code| {
                    code.read_with(&h.visual, |code, _| code.selected_text()) == Some(cited.into())
                })
        });
        h.redraw();
        h.wait("the search", |h| {
            h.read(|view| view.find_state())
                .and_then(|(count, _)| count)
                .is_some()
        });
        h.redraw();
        let code = h.code();
        let (_, _, bounds) = code.read_with(&h.visual, |code, _| code.geometry());
        let y = code.read_with(&h.visual, |code, _| code.row_y(379));
        assert!(
            y > bounds.top() && y < bounds.bottom(),
            "{file}: line 380 in view, {y:?} in {bounds:?}"
        );
        assert_eq!(
            code.read_with(&h.visual, |code, _| code.selected_text()),
            Some(cited.into())
        );
    }
}

#[test]
fn zoom_with_focus_in_the_viewer_steps_only_the_files_font_size() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("README.md"), "# Notes\n\nText.\n").unwrap();
    h.visual.update(|_, cx| {
        let overrides = Default::default();
        for (id, binding) in [
            ("zoom-in", "in"),
            ("zoom-out", "out"),
            ("zoom-reset", "reset"),
        ] {
            let combo = crate::keymap::effective_binding(id, &overrides).unwrap();
            for keys in crate::keymap::binding_strings(&combo) {
                cx.bind_keys([match binding {
                    "in" => gpui::KeyBinding::new(&keys, crate::ZoomIn, None),
                    "out" => gpui::KeyBinding::new(&keys, crate::ZoomOut, None),
                    _ => gpui::KeyBinding::new(&keys, crate::ZoomReset, None),
                }]);
            }
        }
        // As the app installs them: after app zoom's.
        crate::keymap::install_file_zoom_bindings(cx, &overrides);
    });
    let terminal_size = h
        .store
        .read_with(&h.visual, |store, _| store.settings.terminal_font_size);
    let size = |h: &mut Harness| {
        h.store
            .read_with(&h.visual, |store, _| store.settings.files_font_size)
    };
    let app_zooms = |h: &mut Harness| {
        h.host
            .read_with(&h.visual, |host, _| host.app_zooms.clone())
            .unwrap()
    };
    let (zoom_in, zoom_out, reset) = (primary("="), primary("-"), primary("0"));

    h.open("a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.focus_code();
    h.visual.simulate_keystrokes(&zoom_in);
    h.settle();
    assert_eq!(
        size(&mut h),
        Some(terminal_size + 1),
        "from the terminal's size"
    );
    h.visual
        .simulate_keystrokes(&format!("{zoom_out} {zoom_out}"));
    h.settle();
    assert_eq!(size(&mut h), Some(terminal_size - 1));
    assert!(app_zooms(&mut h).is_empty(), "the app's zoom never moved");
    h.visual.simulate_keystrokes(&reset);
    h.settle();
    assert_eq!(
        size(&mut h),
        Some(terminal_size - 1),
        "reset leaves the viewer's text alone"
    );
    assert_eq!(app_zooms(&mut h), ["reset"], "and resets the app's zoom");

    // The range's ends hold.
    h.set(|settings| settings.files_font_size = Some(32));
    h.focus_code();
    h.visual.simulate_keystrokes(&zoom_in);
    h.settle();
    assert_eq!(size(&mut h), Some(32));
    h.set(|settings| settings.files_font_size = Some(8));
    h.focus_code();
    h.visual.simulate_keystrokes(&zoom_out);
    h.settle();
    assert_eq!(size(&mut h), Some(8));

    // The find field and a rendered page are the viewer too.
    h.set(|settings| settings.files_font_size = None);
    h.focus_code();
    h.visual.simulate_keystrokes(&primary("f"));
    h.settle();
    assert!(h.has("FILE_FIND_BAR"));
    h.visual.simulate_keystrokes(&zoom_in);
    h.settle();
    assert_eq!(size(&mut h), Some(terminal_size + 1), "from the find field");
    h.visual.simulate_keystrokes(&reset);
    h.settle();
    assert_eq!(
        size(&mut h),
        Some(terminal_size + 1),
        "reset from the find field"
    );
    assert_eq!(app_zooms(&mut h), ["reset", "reset"]);
    h.visual.simulate_keystrokes("escape");
    h.open("README.md");
    h.wait("the page", |h| h.has("FILE_MARKDOWN"));
    let page = h.read(|view| view.rendered_view()).unwrap();
    let focus = page.read_with(&h.visual, |page, _| page.focus_handle().clone());
    h.visual.update(|window, cx| focus.focus(window, cx));
    h.visual.simulate_keystrokes(&zoom_in);
    h.settle();
    assert_eq!(
        size(&mut h),
        Some(terminal_size + 2),
        "from a rendered page"
    );
    h.visual.simulate_keystrokes(&reset);
    h.settle();
    assert_eq!(
        size(&mut h),
        Some(terminal_size + 2),
        "reset from a rendered page"
    );
    assert_eq!(
        app_zooms(&mut h),
        ["reset", "reset", "reset"],
        "the app's zoom moved only for its reset"
    );

    // The tree and a terminal zoom the app.
    h.click("FILES_ROW a.txt");
    h.visual.simulate_keystrokes(&zoom_in);
    h.settle();
    let elsewhere = h
        .host
        .read_with(&h.visual, |host, _| host.elsewhere.clone())
        .unwrap()
        .unwrap();
    h.visual.update(|window, cx| elsewhere.focus(window, cx));
    h.visual.simulate_keystrokes(&format!("{zoom_out} {reset}"));
    h.settle();
    assert_eq!(app_zooms(&mut h)[3..], ["in", "out", "reset"]);
    assert_eq!(size(&mut h), Some(terminal_size + 2));
}

#[test]
fn the_tree_beside_an_open_file_sits_on_the_panel_colour_in_both_themes() {
    let _theme = ThemeGuard::new();
    let mut h = viewer(chat("in-project"));
    h.click("FILES_ROW a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    for variant in [ThemeVariant::Carbon, ThemeVariant::RunnerLight] {
        theme::set_active_variant(variant);
        h.redraw();
        assert_fill_color(&mut h.visual, "FILES_TREE_COLUMN", Some(theme::panel()));
        assert_ne!(
            theme::panel(),
            theme::bg(),
            "{variant:?}: lighter than the file"
        );
        // The selected row keeps its tint on the new surface.
        assert_fill_color(
            &mut h.visual,
            "FILES_ROW_SELECTED",
            Some(theme::sidebar_selected()),
        );
        // The hover tint, the selected one at 60%, still differs from it.
        assert_ne!(theme::sidebar_selected(), theme::panel());
        assert_ne!(
            theme::with_alpha(theme::sidebar_selected(), 0.6).to_rgb(),
            theme::panel().to_rgb()
        );
    }
    // Beside the empty view too, and alone it keeps the panel's background.
    h.click("FILE_TAB_CLOSE a.txt");
    for variant in [ThemeVariant::Carbon, ThemeVariant::RunnerLight] {
        theme::set_active_variant(variant);
        h.redraw();
        assert_fill_color(&mut h.visual, "FILES_TREE_COLUMN", Some(theme::panel()));
    }
    h.host
        .update(&mut h.visual, |host, _, cx| {
            host.tree_fits = false;
            cx.notify();
        })
        .unwrap();
    h.settle();
    assert_fill_color(&mut h.visual, "FILES_TREE_COLUMN", None);
}

#[test]
fn a_root_opens_on_the_empty_view_and_lists_what_changed() {
    let mut h = viewer(chat("in-project"));
    h.wait("the status", |h| h.has("FILE_NO_FILE"));
    assert!(h.has("FILES_TREE_COLUMN"), "the tree stays beside it");

    std::fs::write(h.repo.join("a.txt"), "changed\n").unwrap();
    std::fs::write(h.repo.join("src/deep/x.rs"), "fn y() {}\n").unwrap();
    h.wait("the changes", |h| h.has("FILE_CHANGED src/deep/x.rs"));
    assert!(h.has("FILE_CHANGED_LIST") && !h.has("FILE_NO_FILE"));
    h.click("FILE_CHANGED a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert_eq!(h.tabs(), ["a.txt"]);

    h.click("FILE_TAB_CLOSE a.txt");
    assert!(h.has("FILE_CHANGED_LIST"), "closing returns to the list");
}

#[test]
fn a_tree_that_does_not_fit_takes_the_panel_until_a_file_opens() {
    let mut h = viewer(chat("in-project"));
    h.host
        .update(&mut h.visual, |host, _, cx| {
            host.tree_fits = false;
            cx.notify();
        })
        .unwrap();
    h.settle();
    assert!(h.has("FILES_ROW a.txt") && !h.has("FILE_VIEWER"));
    h.click("FILES_ROW a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert!(!h.has("FILES_TREE_COLUMN"));
}

#[test]
fn the_layout_gives_a_tree_with_no_room_the_panel_until_a_file_opens() {
    let mut h = viewer(chat("in-project"));
    let subject = PanelSubject::Session("in-project".into());
    let layout = |h: &mut Harness, surface: f32| {
        h.host
            .read_with(&h.visual, |host, cx| {
                host.panel.surface.set(surface);
                host.panel.layout(
                    Some(&subject),
                    FilesSurface::Shown,
                    288.,
                    440.,
                    240.,
                    6.,
                    cx,
                )
            })
            .unwrap()
    };
    // 6 + 420 + 288 + 240: the narrowest that holds the file area and the tree.
    assert!(layout(&mut h, 954.).1.is_some());
    assert_eq!(layout(&mut h, 953.), (288., None));

    h.open("a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    let (width, widened) = layout(&mut h, 953.);
    assert_eq!(width, 528., "a file open keeps the tree beside it");
    assert!(widened.unwrap().tree);

    h.click("FILE_TAB_CLOSE a.txt");
    assert_eq!(layout(&mut h, 953.), (288., None));
    assert!(layout(&mut h, 954.).1.is_some());

    h.click("FILE_TREE_TOGGLE");
    assert!(
        layout(&mut h, 600.).1.is_some(),
        "with the tree toggled off there is no tree to give the panel"
    );
}

#[test]
fn the_finder_reads_the_new_root_when_the_root_changes() {
    let mut h = viewer(chat("in-project"));
    h.click("FILES_SEARCH");
    h.wait("the index", |h| h.has("FILES_FINDER_ITEM 0"));
    let other = h.temp.path().join("other");
    std::fs::create_dir_all(&other).unwrap();
    std::fs::write(other.join("only.txt"), "only\n").unwrap();
    let view = h.view();
    view.update(&mut h.visual, |view, cx| {
        view.set_root(Some(other.clone()), cx)
    });
    h.wait("the new root's files", |h| {
        h.read(|view| {
            view.finder
                .as_ref()
                .is_some_and(|finder| finder.path_at(0) == Some("only.txt"))
        })
    });
    assert_eq!(
        h.read(|view| view.finder.as_ref().unwrap().matches.len()),
        1
    );
    h.visual.simulate_keystrokes("enter");
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert_eq!(h.tabs(), ["only.txt"]);
}

#[test]
fn the_finder_opens_centred_and_finds_a_file_by_name() {
    let mut h = viewer(chat("in-project"));
    h.visual.simulate_resize(size(px(900.), px(900.)));
    h.settle();
    h.click("FILES_SEARCH");
    h.wait("the index", |h| h.has("FILES_FINDER_ITEM 0"));
    let view = h.visual.debug_bounds("FILES_FINDER").unwrap();
    let tree = h.visual.debug_bounds("FILES_TREE_COLUMN").unwrap();
    assert!(
        view.left() < tree.left() && view.right() > tree.left(),
        "over the whole view, not only the tree: {view:?} {tree:?}"
    );
    assert_eq!(view.size.width, px(520.));
    assert_eq!(
        view.center().x,
        h.read(|view| view.view_bounds.get().center().x)
    );

    h.visual.simulate_input("x.r");
    h.settle();
    assert_eq!(
        h.read(|view| view.finder.as_ref().unwrap().matches.len()),
        1
    );
    h.visual.simulate_keystrokes("enter");
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert_eq!(h.tabs(), ["src/deep/x.rs"]);
    assert!(!h.has("FILES_FINDER"));
}

#[test]
fn the_finder_skips_ignored_files_says_when_nothing_matches_and_esc_restores_focus() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("debug.log"), "x").unwrap();
    h.click("FILES_ROW a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    let before = h.visual.update(|window, cx| window.focused(cx));
    assert!(before.is_some());

    let view = h.view();
    h.visual
        .update(|window, cx| view.update(cx, |view, cx| view.toggle_finder(window, cx)));
    h.wait("the index", |h| h.has("FILES_FINDER_ITEM 0"));
    h.visual.simulate_input("debug.log");
    h.settle();
    assert!(h.has("FILES_FINDER_EMPTY"), "git ignores it");
    h.visual.simulate_keystrokes("enter");
    h.settle();
    assert!(h.has("FILES_FINDER"), "nothing to open");

    h.visual.simulate_keystrokes("escape");
    h.settle();
    assert!(!h.has("FILES_FINDER"));
    let after = h.visual.update(|window, cx| window.focused(cx));
    assert!(before == after, "focus returns to where it was");
}

#[test]
fn the_arrows_choose_the_match_enter_opens() {
    let mut h = viewer(chat("in-project"));
    h.click("FILES_SEARCH");
    h.wait("the index", |h| h.has("FILES_FINDER_ITEM 0"));
    h.visual.simulate_input(".txt");
    h.settle();
    h.visual.simulate_keystrokes("down");
    h.settle();
    h.visual.simulate_keystrokes("enter");
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert_eq!(h.tabs(), ["old.txt"], "a.txt ranks first, being shorter");
}

#[test]
fn files_stands_shown_open_or_closed_among_the_surfaces() {
    use crate::surfaces::side_panel::{OpenSurfaces, PanelViewId};
    let settings = crate::app_settings::AppSettings::default();
    let of = |surface, open: &OpenSurfaces| FilesSurface::of(surface, &settings, open);
    let mut open = OpenSurfaces::default();
    assert_eq!(of(PanelSurface::Chat, &open), FilesSurface::Closed);
    open.open(PanelViewId::Files);
    assert_eq!(of(PanelSurface::Chat, &open), FilesSurface::Shown);
    open.open(PanelViewId::Info);
    assert_eq!(of(PanelSurface::Chat, &open), FilesSurface::Open);
    open.close(PanelViewId::Files, false);
    assert_eq!(of(PanelSurface::Chat, &open), FilesSurface::Closed);

    // A terminal tab's only surface is Files, with no tab to close.
    assert_eq!(
        of(PanelSurface::Terminal, &OpenSurfaces::default()),
        FilesSurface::Shown
    );
    assert_eq!(
        of(PanelSurface::Mission, &OpenSurfaces::default()),
        FilesSurface::Closed,
        "a mission shows Sessions until Files opens"
    );
}

fn tick(h: &mut Harness) {
    h.visual
        .executor()
        .advance_clock(super::viewer::IGNORED_REFRESH);
    h.settle();
}

#[test]
fn an_open_ignored_file_refreshes_and_shows_as_deleted() {
    let mut h = viewer(chat("in-project"));
    let path = h.repo.join("z.log");
    std::fs::write(&path, "first\n").unwrap();
    h.wait("the file to be ignored", |h| {
        h.read(|view| {
            view.git_status()
                .is_some_and(|status| status.is_ignored("z.log"))
        })
    });
    h.open("z.log");
    h.wait("the file", |h| h.has("FILE_CODE"));
    assert_eq!(h.line(0), "first");

    // The watcher stays quiet about it, so only a read notices.
    std::fs::write(&path, "second, longer\n").unwrap();
    std::thread::sleep(Duration::from_millis(500));
    h.settle();
    assert_eq!(h.line(0), "first");
    h.wait("the refresh", |h| {
        tick(h);
        h.line(0) == "second, longer"
    });

    // A read that finds the file as it was leaves the tab alone.
    let code = h.code();
    let document = code.read_with(&h.visual, |code, _| code.document().clone());
    tick(&mut h);
    h.wait("the read", |h| h.read(|view| !view.reading("z.log")));
    let same = code.read_with(&h.visual, |code, _| Arc::ptr_eq(code.document(), &document));
    assert!(same, "nothing to redraw");

    // Behind another tab too, it goes on being read.
    h.open("a.txt");
    h.wait("the other tab", |h| h.line(0) == "a");
    std::fs::remove_file(&path).unwrap();
    h.wait("the deleted mark", |h| {
        tick(h);
        h.has("FILE_TAB_LETTER z.log D")
    });
    h.click("FILE_TAB z.log");
    assert!(h.has("FILE_DELETED"));
    assert_eq!(h.line(0), "second, longer", "the last version read stays");

    // And it comes back.
    std::fs::write(&path, "third\n").unwrap();
    h.wait("the file again", |h| {
        tick(h);
        !h.has("FILE_DELETED")
    });
    assert_eq!(h.line(0), "third");
}

#[test]
fn an_ignored_file_stays_refreshed_after_the_status_stops_listing_it() {
    let mut h = viewer(chat("in-project"));
    let path = h.repo.join("z.log");
    std::fs::write(&path, "first\n").unwrap();
    h.wait("the file to be ignored", |h| {
        h.read(|view| {
            view.git_status()
                .is_some_and(|status| status.is_ignored("z.log"))
        })
    });
    h.open("z.log");
    h.wait("the file", |h| h.has("FILE_CODE"));

    // Deleted, and a status for another change arrives before the next read.
    std::fs::remove_file(&path).unwrap();
    receive_status(&mut h, runner_core::protocol::files::GitStatus::default());
    h.wait("the deleted state", |h| {
        tick(h);
        h.has("FILE_DELETED")
    });
    assert!(h.has("FILE_TAB_LETTER z.log D"));

    // Still deleted through another status, and noticed when it returns.
    receive_status(&mut h, runner_core::protocol::files::GitStatus::default());
    std::fs::write(&path, "again\n").unwrap();
    h.wait("the file again", |h| {
        tick(h);
        !h.has("FILE_DELETED")
    });
    assert_eq!(h.line(0), "again");
}

#[test]
fn a_file_opened_before_the_status_arrives_is_noted_when_it_does() {
    use runner_core::protocol::files::GitStatus;
    let mut h = viewer(chat("in-project"));
    h.open("old.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.wait("the status", |h| h.read(|view| view.git_status().is_some()));
    let view = h.view();
    let reads = |h: &mut Harness, view: &Entity<FilesView>| {
        view.read_with(&h.visual, |view, _| view.read_seq("old.txt"))
    };
    let before = reads(&mut h, &view);
    tick(&mut h);
    assert_eq!(reads(&mut h, &view), before, "a file git does not ignore");
    receive_status(&mut h, GitStatus::new(vec![], vec!["old.txt".into()]));
    tick(&mut h);
    assert!(reads(&mut h, &view) > before, "read once git ignores it");
}

#[test]
fn switching_to_an_ignored_file_s_tab_reads_it_again() {
    let mut h = viewer(chat("in-project"));
    let path = h.repo.join("z.log");
    std::fs::write(&path, "first\n").unwrap();
    h.wait("the file to be ignored", |h| {
        h.read(|view| {
            view.git_status()
                .is_some_and(|status| status.is_ignored("z.log"))
        })
    });
    h.open("z.log");
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.open("a.txt");
    h.wait("the other tab", |h| h.line(0) == "a");
    std::fs::write(&path, "second, longer\n").unwrap();
    h.open("z.log");
    h.wait("the read", |h| h.line(0) == "second, longer");
}

#[test]
fn a_rescan_lists_every_loaded_folder_again() {
    // Marked shown without being watched: only the event below can refresh it.
    let mut h = harness(Some(chat("in-project")));
    h.show(None);
    h.host
        .update(&mut h.visual, |host, _, cx| {
            host.unshown = Some(cx.new(|cx| FilesView::new(chat("in-project"), cx)));
            cx.notify();
        })
        .unwrap();
    let view = h
        .host
        .read_with(&h.visual, |host, _| host.unshown.clone())
        .unwrap()
        .unwrap();
    view.update(&mut h.visual, |view, _| view.shown = true);
    h.wait("the working directory", |h| h.has("FILES_ROW src/deep"));
    h.click("FILES_ROW src/deep");
    h.wait("the nested folder", |h| h.has("FILES_ROW src/deep/x.rs"));
    let selected = view.read_with(&h.visual, |view, _| view.selected());
    let root = view.read_with(&h.visual, |view, _| view.root().unwrap().path.clone());
    std::fs::write(h.repo.join("src/deep/new.rs"), "new\n").unwrap();
    std::fs::remove_file(h.repo.join("src/deep/x.rs")).unwrap();

    // Naming only what changed lists only that.
    view.update(&mut h.visual, |view, cx| {
        view.receive(
            FilesEvent::Changed(runner_core::protocol::files::FilesChanged {
                root: root.clone(),
                folders: vec![String::new()],
                paths: vec!["a.txt".into()],
            }),
            cx,
        )
    });
    h.settle();
    assert!(h.has("FILES_ROW src/deep/x.rs"));

    view.update(&mut h.visual, |view, cx| {
        view.receive(
            FilesEvent::Changed(runner_core::protocol::files::FilesChanged {
                root,
                folders: vec![String::new()],
                paths: vec![],
            }),
            cx,
        )
    });
    h.wait("the new file", |h| h.has("FILES_ROW src/deep/new.rs"));
    assert!(!h.has("FILES_ROW src/deep/x.rs"), "the deleted file went");
    assert_eq!(
        view.read_with(&h.visual, |view, _| view.selected()),
        selected,
        "the selection stays"
    );
}

/// Delivers `status` as the daemon would. The watcher's first real status can
/// arrive after it and replace it, so it is delivered again until it holds.
fn receive_status(h: &mut Harness, status: runner_core::protocol::files::GitStatus) {
    let view = h.view();
    let root = h.read(|view| view.root().unwrap().path.clone());
    h.wait("the status to hold", |h| {
        let update = FilesEvent::Status {
            root: root.clone(),
            status: PreparedStatus::new(status.clone()),
        };
        view.update(&mut h.visual, |view, cx| view.receive(update, cx));
        h.settle();
        h.read(|view| view.git_status().as_ref() == Some(&status))
    });
}

#[test]
fn a_status_past_its_limits_says_so() {
    use runner_core::protocol::files::{FileStatus, GitStatus, StatusEntry};
    let mut h = viewer(chat("in-project"));
    h.wait("the status", |h| h.read(|view| view.git_status().is_some()));
    assert!(!h.has("FILES_STATUS_TRUNCATED"));
    receive_status(
        &mut h,
        GitStatus {
            changed: 300_000,
            truncated: true,
            ..GitStatus::new(
                vec![StatusEntry {
                    path: "a.txt".into(),
                    status: FileStatus::Modified,
                }],
                vec![],
            )
        },
    );
    assert!(h.has("FILES_STATUS_TRUNCATED"));
    assert!(h.has("FILE_CHANGED a.txt"), "what it holds still lists");
    receive_status(&mut h, GitStatus::default());
    assert!(!h.has("FILES_STATUS_TRUNCATED"));
}

#[test]
fn a_nested_repository_is_no_changed_file() {
    use runner_core::protocol::files::{FileStatus, GitStatus, StatusEntry};
    let mut h = viewer(chat("in-project"));
    h.wait("the status", |h| h.read(|view| view.git_status().is_some()));
    let entry = |path: &str, status| StatusEntry {
        path: path.into(),
        status,
    };
    receive_status(
        &mut h,
        GitStatus::new(
            vec![
                entry("nested/", FileStatus::Untracked),
                entry("vendor/sub/", FileStatus::Modified),
                entry("a.txt", FileStatus::Modified),
            ],
            vec![],
        ),
    );
    assert!(h.has("FILE_CHANGED a.txt"));
    assert!(!h.has("FILE_CHANGED nested/"));
    assert!(!h.has("FILE_CHANGED vendor/sub/"));

    receive_status(
        &mut h,
        GitStatus::new(vec![entry("nested/", FileStatus::Untracked)], vec![]),
    );
    assert!(h.has("FILE_NO_FILE"), "nothing to list");
}

#[test]
fn the_select_all_key_selects_the_file() {
    let mut h = viewer(chat("in-project"));
    h.open("a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.focus_code();
    h.visual.simulate_keystrokes(&primary("a"));
    h.visual.simulate_keystrokes(&primary("c"));
    h.settle();
    let copied = h.visual.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some("a\n"));
}

#[test]
fn select_all_copies_the_file_with_its_line_endings() {
    for text in [
        "one\ntwo\n",
        "one\r\ntwo\r\n",
        "one\ntwo",
        "one\n\n",
        "\n\n",
        "\n",
    ] {
        let mut h = viewer(chat("in-project"));
        std::fs::write(h.repo.join("whole.txt"), text).unwrap();
        h.open("whole.txt");
        h.wait("the file", |h| h.has("FILE_CODE"));
        h.focus_code();
        h.visual.simulate_keystrokes(&primary("a"));
        h.visual.simulate_keystrokes(&primary("c"));
        h.settle();
        let copied = h.visual.read_from_clipboard().and_then(|item| item.text());
        assert_eq!(copied.as_deref(), Some(text));
    }
}

#[test]
fn only_select_all_takes_the_final_line_ending() {
    let mut h = viewer(chat("in-project"));
    std::fs::write(h.repo.join("token.txt"), "token\n").unwrap();
    h.open("token.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.focus_code();
    h.settle();
    let code = h.code();
    let selected = |h: &mut Harness| code.read_with(&h.visual, |code, _| code.selected_text());
    let (advance, left, bounds) = code.read_with(&h.visual, |code, _| code.geometry());
    let y = code.read_with(&h.visual, |code, _| code.row_y(0));
    let word = gpui::point(bounds.left() + left + advance * 2., y);
    let double_click = |h: &mut Harness| {
        for click_count in [1, 2] {
            h.visual.simulate_event(gpui::MouseDownEvent {
                button: gpui::MouseButton::Left,
                position: word,
                modifiers: Modifiers::default(),
                click_count,
                first_mouse: false,
            });
            h.visual.simulate_event(gpui::MouseUpEvent {
                button: gpui::MouseButton::Left,
                position: word,
                modifiers: Modifiers::default(),
                click_count,
            });
        }
        h.settle();
    };

    double_click(&mut h);
    assert_eq!(selected(&mut h).as_deref(), Some("token"));

    h.visual.simulate_keystrokes(&primary("a"));
    assert_eq!(selected(&mut h).as_deref(), Some("token\n"));

    double_click(&mut h);
    assert_eq!(
        selected(&mut h).as_deref(),
        Some("token"),
        "a later selection is only what it covers"
    );
}

#[test]
fn select_all_ends_a_drag_in_progress() {
    let mut h = viewer(chat("in-project"));
    let text = "one\ntwo\nthree\n";
    std::fs::write(h.repo.join("drag.txt"), text).unwrap();
    h.open("drag.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.focus_code();
    h.settle();
    let code = h.code();
    let (advance, left, bounds) = code.read_with(&h.visual, |code, _| code.geometry());
    let at = |h: &mut Harness, line| {
        let y = code.read_with(&h.visual, |code, _| code.row_y(line));
        gpui::point(bounds.left() + left + advance, y)
    };

    let start = at(&mut h, 0);
    h.visual
        .simulate_mouse_down(start, gpui::MouseButton::Left, Modifiers::default());
    h.visual.simulate_keystrokes(&primary("a"));
    let inside = at(&mut h, 1);
    h.visual
        .simulate_mouse_move(inside, Some(gpui::MouseButton::Left), Modifiers::default());
    h.visual
        .simulate_mouse_up(inside, gpui::MouseButton::Left, Modifiers::default());
    h.settle();

    let (selection, end) = code.read_with(&h.visual, |code, _| {
        (code.selection(), code.document().end())
    });
    assert_eq!(
        selection,
        Some((super::document::Position::default(), end)),
        "what shows selected is the file"
    );
    h.visual.simulate_keystrokes(&primary("c"));
    h.settle();
    let copied = h.visual.read_from_clipboard().and_then(|item| item.text());
    assert_eq!(copied.as_deref(), Some(text));
}

#[test]
fn select_all_follows_the_file_when_it_is_read_again() {
    let mut h = viewer(chat("in-project"));
    h.open("a.txt");
    h.wait("the file", |h| h.has("FILE_CODE"));
    h.focus_code();
    h.visual.simulate_keystrokes(&primary("a"));
    let code = h.code();
    let reread = |h: &mut Harness, text: &str| {
        let document = Arc::new(super::document::Document::decode(text.as_bytes()));
        code.update(&mut h.visual, |code, cx| code.set_document(document, cx));
        code.read_with(&h.visual, |code, _| code.selected_text())
    };
    assert_eq!(
        reread(&mut h, "longer\nnow\n").as_deref(),
        Some("longer\nnow\n")
    );
    assert_eq!(reread(&mut h, ""), None, "an empty file copies nothing");
}

#[test]
fn moving_a_chat_into_a_project_finds_its_root() {
    let mut h = harness(Some(chat("outside")));
    h.wait("the choice", |h| h.read(|view| view.state() == "choose"));
    let src = h.repo.join("src");
    h.store.update(&mut h.visual, |store, _| {
        store
            .test_core
            .db
            .get()
            .unwrap()
            .execute_batch(&format!(
                "UPDATE sessions SET project_id = 'runner', cwd = '{}' WHERE id = 'outside'",
                sql(&src)
            ))
            .unwrap();
    });
    let mut moved = chat("outside");
    moved.project = Some("runner".into());
    h.show(Some(moved));
    h.wait("the root", |h| h.read(|view| view.state() == "root"));
}

#[test]
fn resolving_the_same_root_leaves_no_tab_loading() {
    let mut h = viewer(chat("in-project"));
    let view = h.view();
    view.update(&mut h.visual, |view, cx| {
        view.open_file("a.txt".into(), cx);
        // A change of project resolves the root again before the read's reply
        // arrives, and the reply is dropped.
        let mut moved = chat("in-project");
        moved.project = Some("other".into());
        view.set_context(moved, cx);
    });
    h.wait("the file", |h| h.has("FILE_CODE"));
}

/// A second chat on the same root as `in-project`.
fn same_root_chat(h: &mut Harness) -> PanelContext {
    h.store.update(&mut h.visual, |store, _| {
        store
            .test_core
            .db
            .get()
            .unwrap()
            .execute_batch(&format!(
                "INSERT INTO sessions (id, status, agent_runtime, agent_command, cwd, project_id)
                 VALUES ('same-root', 'stopped', 'shell', 'shell', '{}', 'runner');",
                sql(&h.repo),
            ))
            .unwrap();
    });
    PanelContext {
        project: Some("runner".into()),
        ..chat("same-root")
    }
}

#[test]
fn a_hidden_view_does_no_work_for_its_root_and_catches_up_when_shown() {
    use runner_core::protocol::files::{FilesChanged, GitStatus};
    let mut h = harness(Some(chat("in-project")));
    let other = same_root_chat(&mut h);
    h.wait("the first status", |h| {
        h.status_read() && h.has("FILES_ROW a.txt")
    });
    let hidden = h.view();
    let root = h.read(|view| view.root().unwrap().path.clone());

    h.show(Some(other.clone()));
    h.wait("the second status", |h| {
        h.status_read() && h.has("FILES_ROW a.txt")
    });
    assert_same(h.read(|view| view.watched().map(Path::to_path_buf)), &root);
    let snapshot = |h: &mut Harness| {
        hidden.read_with(&h.visual, |view, _| {
            let tree = view.tree_ref().unwrap();
            (tree.rebuilds(), tree.status().cloned())
        })
    };
    let before = snapshot(&mut h);

    // The shown view's watcher reports a real edit; only the shown view reacts.
    let fresh = h.repo.join("fresh.txt");
    let mut touched = Instant::now() - Duration::from_secs(1);
    h.wait("the watcher", |h| {
        if touched.elapsed() >= Duration::from_secs(1) {
            std::fs::write(&fresh, format!("{:?}\n", Instant::now())).unwrap();
            touched = Instant::now();
        }
        h.has("FILES_LETTER fresh.txt")
    });
    assert_eq!(snapshot(&mut h), before, "the hidden view did nothing");

    // Delivered to it directly, too, with no timing to depend on.
    hidden.update(&mut h.visual, |view, cx| {
        view.receive(
            FilesEvent::Changed(FilesChanged {
                root: root.clone(),
                folders: vec![String::new()],
                paths: Vec::new(),
            }),
            cx,
        );
        view.receive(
            FilesEvent::Status {
                root: root.clone(),
                status: PreparedStatus::new(GitStatus::new(Vec::new(), vec!["a.txt".into()])),
            },
            cx,
        );
    });
    h.settle();
    assert_eq!(
        snapshot(&mut h),
        before,
        "events to a hidden view are dropped"
    );

    h.show(Some(chat("in-project")));
    h.wait("the view to catch up", |h| h.has("FILES_LETTER fresh.txt"));
}

#[test]
fn a_status_that_arrives_clears_the_error_of_one_that_failed() {
    use runner_core::protocol::files::{FilesStatusChanged, GitStatus, FILES_STATUS_EVENT};
    let mut h = harness(None);
    let events = h
        .store
        .read_with(&h.visual, |store, _| store.test_core.events.clone());
    let mut sent = events.subscribe();
    h.show(Some(chat("in-project")));
    h.wait("the status", |h| h.status_read());
    let view = h.view();
    let root = h.read(|view| view.root().unwrap().path.clone());

    // The watcher reports its first status on its own, after the read above,
    // and would clear the error below whenever it landed. Wait until it is
    // sent, then until a status sent after it has reached the view.
    h.wait("the watcher's first status", |_| {
        std::iter::from_fn(|| sent.try_recv().ok()).any(|event| {
            event.name == FILES_STATUS_EVENT
                && serde_json::from_value::<FilesStatusChanged>(event.payload)
                    .is_ok_and(|update| update.root == root)
        })
    });
    let marker = GitStatus::new(Vec::new(), vec!["marker".into()]);
    events.emit(
        FILES_STATUS_EVENT,
        &FilesStatusChanged {
            root: root.clone(),
            status: marker.clone(),
        },
    );
    h.wait("the status sent after it", |h| {
        h.read(|view| view.git_status().as_ref() == Some(&marker))
    });

    let deadline = |view: &mut FilesView, cx: &mut Context<FilesView>| {
        view.status_error = Some("runnerd request deadline exceeded".into());
        cx.notify();
    };
    view.update(&mut h.visual, deadline);
    h.settle();
    assert!(h.has("FILES_ERROR"));

    // Another root's status says nothing about this one.
    view.update(&mut h.visual, |view, cx| {
        view.receive(
            FilesEvent::Status {
                root: root.join("src"),
                status: PreparedStatus::new(GitStatus::default()),
            },
            cx,
        )
    });
    h.settle();
    assert!(h.has("FILES_ERROR"));

    receive_status(&mut h, GitStatus::default());
    assert!(!h.has("FILES_ERROR"), "the status arrived after all");

    // An error about something else stays through a status.
    let picker_error = h.temp.path().join("nowhere").to_string_lossy().into_owned();
    h.click("FILES_ROOT_BUTTON");
    h.visual.simulate_input(&picker_error);
    h.visual.simulate_keystrokes("enter");
    h.wait("the error", |h| h.has("FILES_ERROR"));
    receive_status(&mut h, GitStatus::new(Vec::new(), vec!["other.log".into()]));
    assert!(h.has("FILES_ERROR"));
}
