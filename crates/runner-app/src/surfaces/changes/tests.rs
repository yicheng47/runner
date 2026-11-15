use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use gpui::{
    div, prelude::*, px, size, Context, Entity, FocusHandle, Modifiers, TestAppContext,
    VisualTestContext, Window, WindowHandle,
};
use runner_core::protocol::changes::{
    ChangedFile, ChangesCommits, ChangesTarget, CommitSummary, DiffCounts, ModeChange, MAX_COMMITS,
};
use runner_core::protocol::files::FileStatus;

use super::diff::DiffPos;
use super::model::{
    abbreviate_folder, branch_header_label, build_commit_rows, build_flat_rows, build_tree_rows,
    choose_initial_target, comparison_label, is_crlf_only_change, older_commits_hidden,
    reconcile_file_selection, CommitRow, FocusPlace, ListRow, SelectedTarget,
};
use super::ChangesView;
use crate::i18n::{self, Locale};
use crate::surfaces::files::FilesPanel;
use crate::surfaces::side_panel::{PanelContext, PanelSubject, PanelSurface, Widened};

struct Host {
    panel: FilesPanel,
    context: Option<PanelContext>,
    outside_focus: FocusHandle,
    closed_panes: usize,
    app_zooms: Vec<&'static str>,
}

impl Render for Host {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.context {
            Some(context) => {
                let settings = &crate::app_store::global_app_store(cx).read(cx).settings;
                let widened = Some(Widened {
                    file: settings.file_viewer_width.max(288.),
                    tree: true,
                    tree_width: settings.file_tree_width,
                    standard: 288.,
                });
                let owner = cx.entity_id();
                Some(
                    self.panel
                        .show_changes_with(context, widened, Some(owner), cx),
                )
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
            .child(div().id("outside-focus").track_focus(&self.outside_focus))
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
            "-c",
            "commit.gpgsign=false",
        ])
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

fn real(path: &Path) -> PathBuf {
    if cfg!(windows) {
        path.to_path_buf()
    } else {
        path.canonicalize().unwrap()
    }
}

fn sql(path: &Path) -> String {
    path.to_string_lossy().replace('\'', "''")
}

struct Harness {
    _temp: tempfile::TempDir,
    repo_path: PathBuf,
    worktree: PathBuf,
    plain: PathBuf,
    _cx: TestAppContext,
    store: Entity<crate::app_store::AppStore>,
    host: WindowHandle<Host>,
    visual: VisualTestContext,
}

fn setup_repo(dir: &Path) {
    std::fs::create_dir_all(dir.join("src/sub")).unwrap();
    git(dir, &["init", "-q", "-b", "main"]);
    git(dir, &["config", "core.autocrlf", "false"]);
    std::fs::write(dir.join(".gitignore"), ".worktrees/\n").unwrap();
    std::fs::write(
        dir.join("src/lib.rs"),
        "fn alpha() {\n    let a = 1;\n}\n\nfn beta() {\n    let b = 2;\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/sub/util.rs"), "pub fn util() -> i32 { 1 }\n").unwrap();
    std::fs::write(dir.join("README.md"), "# Runner\n").unwrap();
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", "initial commit"]);
}

fn harness(context: Option<PanelContext>) -> Harness {
    let temp = tempfile::tempdir().unwrap();
    let repo_path = temp.path().join("runner");
    setup_repo(&repo_path);
    let worktree = repo_path.join(".worktrees/feat-x");
    git(
        &repo_path,
        &["worktree", "add", "-q", "-b", "feat/x", ".worktrees/feat-x"],
    );
    // Put two commits on feat/x ahead of local main.
    std::fs::write(
        worktree.join("src/lib.rs"),
        "/*\nfn alpha() {\n    let a = 1;\n}\n*/\nfn alpha_v2() {\n    let a = 10;\n}\n\nfn beta() {\n    let b = 2;\n}\n",
    )
    .unwrap();
    git(&worktree, &["add", "-A"]);
    git(&worktree, &["commit", "-q", "-m", "feat: rewrite alpha"]);

    std::fs::write(
        worktree.join("src/sub/added.rs"),
        "pub fn added() -> bool {\n    true\n}\n",
    )
    .unwrap();
    git(&worktree, &["add", "-A"]);
    git(&worktree, &["commit", "-q", "-m", "feat: add helper"]);

    let plain = temp.path().join("plain-dir");
    std::fs::create_dir_all(&plain).unwrap();
    std::fs::write(plain.join("notes.txt"), "hello\n").unwrap();

    let repo_path = real(&repo_path);
    let worktree = real(&worktree);
    let plain = real(&plain);
    let missing = repo_path.join(".worktrees/gone");

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
                 VALUES ('runner', 'runner', '{repo}', 0, '2026-11-15T00:00:00Z');
                 INSERT INTO sessions (id, status, agent_runtime, agent_command, cwd, project_id)
                 VALUES ('main-chat', 'stopped', 'shell', 'shell', '{repo}', 'runner'),
                        ('plain-chat', 'stopped', 'shell', 'shell', '{plain}', NULL),
                        ('outside', 'stopped', 'shell', 'shell', '{home}', NULL);
                 INSERT INTO missions (id, title, status, started_at, cwd, project_id)
                 VALUES ('on-worktree', 'M', 'running', '2026-11-15T00:00:00Z', '{worktree}', 'runner'),
                        ('gone', 'G', 'running', '2026-11-15T00:00:00Z', '{gone}', 'runner');",
                repo = sql(&repo_path),
                plain = sql(&plain),
                home = sql(temp.path()),
                worktree = sql(&worktree),
                gone = sql(&missing),
            ))
            .unwrap();
        // Save `plain` explicitly for `plain-chat` so it opens directly on that non-git folder.
        let _ = store.client.files_set_root(
            &runner_core::protocol::files::FilesSubject::Session("plain-chat".into()),
            Some(&plain),
        );
        store.refresh(crate::app_store::StoreRefreshKind::All, cx);
    });

    let host = cx.add_window(|_, cx| Host {
        panel: FilesPanel::default(),
        context,
        outside_focus: cx.focus_handle(),
        closed_panes: 0,
        app_zooms: Vec::new(),
    });
    let visual = VisualTestContext::from_window(host.into(), &cx);
    visual.simulate_resize(size(px(760.), px(900.)));
    let mut h = Harness {
        _temp: temp,
        repo_path,
        worktree,
        plain,
        _cx: cx,
        store,
        host,
        visual,
    };
    h.settle();
    h
}

fn mission_ctx(id: &str) -> PanelContext {
    PanelContext {
        subject: PanelSubject::Mission(id.into()),
        surface: PanelSurface::Mission,
        cwd: None,
        project: Some("runner".into()),
        remote: false,
    }
}

fn chat_ctx(id: &str, project: Option<&str>, remote: bool) -> PanelContext {
    PanelContext {
        subject: PanelSubject::Session(id.into()),
        surface: PanelSurface::Chat,
        cwd: None,
        project: project.map(str::to_owned),
        remote,
    }
}

impl Harness {
    fn settle(&mut self) {
        self.visual.run_until_parked();
    }

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
            .unwrap_or_else(|| panic!("missing selector {selector}"));
        self.visual
            .simulate_click(bounds.center(), Modifiers::default());
        self.settle();
    }

    fn press(&mut self, key: &str) {
        self.visual.simulate_keystrokes(key);
        self.settle();
    }

    fn view(&mut self) -> Entity<ChangesView> {
        self.host
            .read_with(&self.visual, |host, _| {
                let subject = &host.context.as_ref().unwrap().subject;
                host.panel.changes_view(subject)
            })
            .unwrap()
            .expect("a shown ChangesView")
    }

    fn read<T>(&mut self, f: impl FnOnce(&ChangesView) -> T) -> T {
        let view = self.view();
        view.read_with(&self.visual, |v, _| f(v))
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
}

fn sample_file(path: &str) -> ChangedFile {
    ChangedFile {
        path: path.into(),
        status: FileStatus::Modified,
        counts: DiffCounts::Lines {
            added: 3,
            removed: 1,
        },
        old_blob_id: Some("111".into()),
        new_blob_id: Some("222".into()),
        viewed: false,
        changed_since_viewed: false,
    }
}

#[test]
fn flat_and_tree_grouping_sorts_by_path_and_collapses_folders() {
    let files = vec![
        sample_file("Cargo.toml"),
        sample_file("crates/runner-app/src/a.rs"),
        sample_file("crates/runner-app/src/b.rs"),
        sample_file("docs/spec.md"),
    ];
    let flat = build_flat_rows(&files);
    assert_eq!(flat.len(), 4);
    assert_eq!(
        flat[1],
        ListRow::File {
            file_index: 1,
            path: "crates/runner-app/src/a.rs".into(),
            name: "a.rs".into(),
            folder_hint: Some("crates/…/src".into()),
            depth: 0,
        }
    );

    let mut collapsed = BTreeSet::new();
    let tree = build_tree_rows(&files, &collapsed);
    let paths: Vec<&str> = tree.iter().map(ListRow::path).collect();
    assert_eq!(
        paths,
        vec![
            "crates",
            "crates/runner-app",
            "crates/runner-app/src",
            "crates/runner-app/src/a.rs",
            "crates/runner-app/src/b.rs",
            "docs",
            "docs/spec.md",
            "Cargo.toml",
        ]
    );

    collapsed.insert("crates/runner-app".into());
    let collapsed_tree = build_tree_rows(&files, &collapsed);
    let collapsed_paths: Vec<&str> = collapsed_tree.iter().map(ListRow::path).collect();
    assert_eq!(
        collapsed_paths,
        vec![
            "crates",
            "crates/runner-app",
            "docs",
            "docs/spec.md",
            "Cargo.toml"
        ]
    );
}

#[test]
fn opening_selection_and_comparison_labels_cover_every_target_and_older_commits_cap() {
    let now = chrono::Utc::now();
    let commits = ChangesCommits {
        checkout_top: PathBuf::from("/tmp/repo"),
        branch: Some("feat/1141".into()),
        head_commit: Some("a1f3c9e".into()),
        base: Some("origin/main".into()),
        commits: vec![
            CommitSummary {
                id: "a1f3c9e000".into(),
                short_id: "a1f3c9e".into(),
                subject: "merge main".into(),
                author: "Jason".into(),
                date: now,
                parent_count: 2,
            },
            CommitSummary {
                id: "7be20d4000".into(),
                short_id: "7be20d4".into(),
                subject: "normal commit".into(),
                author: "Jason".into(),
                date: now,
                parent_count: 1,
            },
            CommitSummary {
                id: "0c2ad18000".into(),
                short_id: "0c2ad18".into(),
                subject: "root commit".into(),
                author: "Jason".into(),
                date: now,
                parent_count: 0,
            },
        ],
        total: MAX_COMMITS + 5,
    };

    let detached = i18n::testing::with_locale(Locale::English, || {
        assert_eq!(
            choose_initial_target(&commits, 2),
            SelectedTarget::Uncommitted
        );
        assert_eq!(choose_initial_target(&commits, 0), SelectedTarget::Branch);
        let mut capped = commits.clone();
        capped.commits = (0..MAX_COMMITS)
            .map(|i| CommitSummary {
                id: format!("sha-{i:04}"),
                short_id: format!("s{i:04}"),
                subject: format!("commit {i}"),
                author: "Jason".into(),
                date: now,
                parent_count: 1,
            })
            .collect();
        assert_eq!(older_commits_hidden(&capped), 5);
        assert_eq!(branch_header_label(&commits), "feat/1141");

        assert_eq!(
            comparison_label(&SelectedTarget::Branch, &commits),
            "feat/1141 vs origin/main"
        );
        assert_eq!(
            comparison_label(&SelectedTarget::Uncommitted, &commits),
            "Working tree vs HEAD"
        );
        assert_eq!(
            comparison_label(&SelectedTarget::Commit("a1f3c9e000".into()), &commits),
            "a1f3c9e vs its first parent"
        );
        assert_eq!(
            comparison_label(&SelectedTarget::Commit("7be20d4000".into()), &commits),
            "7be20d4 vs its parent"
        );
        assert_eq!(
            comparison_label(&SelectedTarget::Commit("0c2ad18000".into()), &commits),
            "0c2ad18, the first commit"
        );

        let unborn = ChangesCommits {
            checkout_top: PathBuf::from("/tmp/repo"),
            branch: Some("main".into()),
            head_commit: None,
            base: None,
            commits: Vec::new(),
            total: 0,
        };
        assert_eq!(
            choose_initial_target(&unborn, 0),
            SelectedTarget::Uncommitted
        );
        assert_eq!(
            build_commit_rows(&unborn, 0, 1),
            vec![CommitRow::Uncommitted { count: 1 }]
        );
        assert_eq!(
            comparison_label(&SelectedTarget::Uncommitted, &unborn),
            "Working tree, no commits yet"
        );

        let detached = ChangesCommits {
            branch: None,
            head_commit: Some("deadbee".into()),
            ..commits
        };
        assert_eq!(branch_header_label(&detached), "detached at deadbee");
        detached
    });

    i18n::testing::with_locale(Locale::SimplifiedChinese, || {
        assert_eq!(branch_header_label(&detached), "游离于 deadbee");
        assert_eq!(
            comparison_label(&SelectedTarget::Branch, &detached),
            "deadbee 对比 origin/main"
        );
    });
}

#[test]
fn crlf_only_detection_and_selection_reconciliation() {
    assert!(is_crlf_only_change(
        b"one\ntwo\n",
        b"one\r\ntwo\r\n",
        None,
        false
    ));
    assert!(is_crlf_only_change(
        b"one\r\ntwo\r\n",
        b"one\ntwo\n",
        None,
        false
    ));
    assert!(!is_crlf_only_change(b"one\n", b"one\n", None, false));
    assert!(!is_crlf_only_change(b"one\n", b"two\r\n", None, false));
    assert!(!is_crlf_only_change(
        b"one\n",
        b"one\r\n",
        Some(&ModeChange {
            old: "100644".into(),
            new: "100755".into(),
        }),
        false
    ));
    assert!(!is_crlf_only_change(b"one\n", b"one\r\n", None, true));

    assert_eq!(abbreviate_folder("README.md"), None);
    assert_eq!(abbreviate_folder("src/lib.rs"), Some("src".into()));
    assert_eq!(
        abbreviate_folder("crates/runner-app/src/surfaces/mod.rs"),
        Some("crates/…/surfaces".into())
    );

    let files = vec![sample_file("a.rs"), sample_file("c.rs")];
    let flat_rows = build_flat_rows(&files);
    assert_eq!(
        reconcile_file_selection(Some("a.rs"), &files, &flat_rows, false),
        Some("a.rs".into())
    );
    assert_eq!(
        reconcile_file_selection(Some("b.rs"), &files, &flat_rows, false),
        Some("c.rs".into())
    );
    assert_eq!(
        reconcile_file_selection(Some("z.rs"), &files, &flat_rows, false),
        Some("c.rs".into())
    );

    // Removing both an earlier row (`binary.bin`) and the selected row (`long.txt`) in one refresh
    // must advance to `mode.sh`, NOT skip it to `rename-old.txt`.
    let surviving = vec![
        sample_file("conflict.txt"),
        sample_file("delete-me.txt"),
        sample_file("large.txt"),
        sample_file("line-endings.txt"),
        sample_file("mode.sh"),
        sample_file("rename-old.txt"),
    ];
    let surviving_flat = build_flat_rows(&surviving);
    assert_eq!(
        reconcile_file_selection(Some("long.txt"), &surviving, &surviving_flat, false),
        Some("mode.sh".into())
    );

    // Tree mode orders folders before root files (`crates/a.rs`, `docs/spec.md`, `Cargo.toml`, `README.md`),
    // so removing `docs/spec.md` advances to `Cargo.toml` even though `"Cargo.toml" < "docs/spec.md"` in ASCII.
    let tree_files = vec![
        sample_file("Cargo.toml"),
        sample_file("README.md"),
        sample_file("crates/a.rs"),
    ];
    let tree_rows = build_tree_rows(&tree_files, &BTreeSet::new());
    assert_eq!(
        reconcile_file_selection(Some("docs/spec.md"), &tree_files, &tree_rows, true),
        Some("Cargo.toml".into())
    );
}

#[test]
fn worktree_opens_on_all_changes_or_uncommitted_and_switches_commits() {
    let mut h = harness(Some(mission_ctx("on-worktree")));
    h.wait("All changes diff", |h| h.has("CHANGES_DIFF_CODE"));
    assert_eq!(
        h.read(|v| v.selected_target().cloned()),
        Some(SelectedTarget::Branch)
    );
    assert!(h.has("CHANGES_COMMIT_ALL"));
    assert!(h.has("CHANGES_COMMIT_UNCOMMITTED"));
    assert!(h.has("CHANGES_ROW src/lib.rs"));
    assert!(h.has("CHANGES_ROW src/sub/added.rs"));

    // Add an uncommitted edit: live or freshly opened view excludes uncommitted from All changes.
    std::fs::write(h.worktree.join("dirty.txt"), "uncommitted\n").unwrap();
    h.show(None);
    // Reset to test opening selection on a dirty worktree via main-chat vs worktree:
    let mut h2 = harness(None);
    std::fs::write(h2.worktree.join("dirty.txt"), "uncommitted\n").unwrap();
    h2.show(Some(mission_ctx("on-worktree")));
    h2.wait("Uncommitted diff", |h| h.has("CHANGES_ROW dirty.txt"));
    assert_eq!(
        h2.read(|v| v.selected_target().cloned()),
        Some(SelectedTarget::Uncommitted)
    );

    // Clicking All changes excludes dirty.txt and shows only the committed branch files.
    h2.click("CHANGES_COMMIT_ALL");
    h2.wait("All changes files", |h| {
        h.has("CHANGES_ROW src/lib.rs") && !h.has("CHANGES_ROW dirty.txt")
    });
    assert!(h2.has("CHANGES_ROW src/sub/added.rs"));
}

#[test]
fn flat_and_tree_switch_persists_in_settings_and_toggles_folders() {
    let mut h = harness(Some(mission_ctx("on-worktree")));
    h.wait("All changes loaded", |h| {
        h.has("CHANGES_ROW src/sub/added.rs")
    });
    assert!(!h.has("CHANGES_FOLDER src"));

    h.click("CHANGES_MODE_TREE");
    h.wait("Tree folder rows", |h| h.has("CHANGES_FOLDER src"));
    assert!(h
        .store
        .read_with(&h.visual, |store, _| store.settings.changes_tree_view));
    assert!(h.has("CHANGES_FOLDER src/sub"));
    assert!(h.has("CHANGES_ROW src/sub/added.rs"));

    // Clicking a folder collapses its subtree.
    h.click("CHANGES_FOLDER src/sub");
    assert!(!h.has("CHANGES_ROW src/sub/added.rs"));
    h.click("CHANGES_FOLDER src/sub");
    assert!(h.has("CHANGES_ROW src/sub/added.rs"));

    h.click("CHANGES_MODE_FLAT");
    assert!(!h.has("CHANGES_FOLDER src"));
    assert!(!h
        .store
        .read_with(&h.visual, |store, _| store.settings.changes_tree_view));
}

#[test]
fn keyboard_moves_focus_between_commits_files_and_diff_and_jumps_hunks() {
    let mut h = harness(None);
    // Create a file with two separated hunks on feat/x so `]` and `[` jump between them.
    let mut lines: Vec<String> = (1..=30).map(|i| format!("let line_{i} = {i};")).collect();
    std::fs::write(h.worktree.join("two_hunks.rs"), lines.join("\n") + "\n").unwrap();
    git(&h.worktree, &["add", "two_hunks.rs"]);
    git(&h.worktree, &["commit", "-q", "-m", "add two_hunks"]);
    lines[1] = format!("let line_2 = {};", "x".repeat(200));
    lines[27] = "let line_28 = 2800;".into();
    std::fs::write(h.worktree.join("two_hunks.rs"), lines.join("\n") + "\n").unwrap();

    h.show(Some(mission_ctx("on-worktree")));
    h.wait("two_hunks diff", |h| h.has("CHANGES_DIFF_CODE"));
    assert_eq!(
        h.read(|v| v.selected_path().map(str::to_owned)),
        Some("two_hunks.rs".into())
    );

    // Focus list by clicking the row, then Space marks viewed, Tab switches to Commits, Tab back to Files.
    h.click("CHANGES_ROW two_hunks.rs");
    assert_eq!(h.read(|v| v.focus_place()), FocusPlace::Files);

    h.press("space");
    assert!(h.read(|v| {
        match &v.body {
            super::Body::Root(s) => s.files[0].viewed,
            _ => false,
        }
    }));

    // Tab moves to Commits, where Space does NOT toggle viewed.
    h.press("tab");
    assert_eq!(h.read(|v| v.focus_place()), FocusPlace::Commits);
    h.press("space");
    assert!(h.read(|v| {
        match &v.body {
            super::Body::Root(s) => s.files[0].viewed,
            _ => false,
        }
    }));

    // Shift-Tab returns to Files.
    h.press("shift-tab");
    assert_eq!(h.read(|v| v.focus_place()), FocusPlace::Files);

    // Enter enters the diff; `j` moves cursor line, `]` jumps to hunk 2, `[` jumps back to hunk 1, Esc returns to list.
    h.press("enter");
    assert_eq!(h.read(|v| v.focus_place()), FocusPlace::Diff);
    let diff = h.read(|v| v.unified_diff_view()).unwrap();
    let initial_row = diff.read_with(&h.visual, |d, _| d.cursor_row());
    h.press("j");
    let stepped_row = diff.read_with(&h.visual, |d, _| d.cursor_row());
    assert_eq!(stepped_row, initial_row + 1);

    h.press("]");
    let second_hunk_row = diff.read_with(&h.visual, |d, _| d.cursor_row());
    assert!(
        second_hunk_row > stepped_row,
        "{second_hunk_row} vs {stepped_row}"
    );

    h.press("[");
    let back_row = diff.read_with(&h.visual, |d, _| d.cursor_row());
    assert!(back_row < second_hunk_row);

    h.press("right");
    let scrolled_x = diff.read_with(&h.visual, |d, _| d.scroll_offset().x);
    assert!(
        scrolled_x > px(0.),
        "right arrow scrolls sideways: {scrolled_x:?}"
    );
    h.press("left");
    let scrolled_back_x = diff.read_with(&h.visual, |d, _| d.scroll_offset().x);
    assert!(
        scrolled_back_x < scrolled_x,
        "{scrolled_back_x:?} vs {scrolled_x:?}"
    );

    h.press("escape");
    assert_eq!(h.read(|v| v.focus_place()), FocusPlace::Files);
}

#[test]
fn unified_diff_highlights_both_versions_and_copies_without_gutter() {
    let mut h = harness(Some(mission_ctx("on-worktree")));
    h.wait("src/lib.rs diff", |h| h.has("CHANGES_DIFF_CODE"));
    assert_eq!(
        h.read(|v| v.selected_path().map(str::to_owned)),
        Some("src/lib.rs".into())
    );

    let diff = h.read(|v| v.unified_diff_view()).unwrap();
    h.wait("two-version syntax highlights", |h| {
        let (old_hl, new_hl) = diff.read_with(&h.visual, |d, _| d.has_highlights());
        old_hl && new_hl
    });

    // Select across row 0 (@@ hunk header) through row 4 and verify selected_text omits @@ and gutter.
    diff.update(&mut h.visual, |d, cx| {
        d.set_selection_for_test(
            DiffPos { row: 0, offset: 0 },
            DiffPos {
                row: 3,
                offset: 100,
            },
            cx,
        );
    });
    let copied = diff
        .read_with(&h.visual, |d, _| d.selected_text())
        .expect("selected text");
    assert!(
        !copied.contains("@@"),
        "copied text must exclude hunk header: {copied:?}"
    );
    assert!(
        copied.contains("fn alpha()"),
        "copied text has code: {copied:?}"
    );
}

#[test]
fn viewed_marks_toggle_at_end_of_diff_sync_across_clients_and_turn_amber_on_edit() {
    let mut h = harness(None);
    std::fs::write(h.worktree.join("tracked.rs"), "fn v1() {}\n").unwrap();
    h.show(Some(mission_ctx("on-worktree")));
    h.wait("tracked.rs diff", |h| h.has("CHANGES_DIFF_VIEWED"));

    // Click end-of-diff viewed checkbox.
    h.click("CHANGES_DIFF_VIEWED");
    h.wait("daemon recorded viewed mark", |h| {
        let files = h.store.read_with(&h.visual, |store, _| {
            store
                .client
                .changes_files(&h.worktree, &ChangesTarget::Uncommitted)
                .unwrap()
        });
        files.totals.viewed == 1
    });

    // Edit tracked.rs -> mark clears and turns changed_since_viewed!
    std::fs::write(h.worktree.join("tracked.rs"), "fn v2() {}\n").unwrap();
    let view = h.view();
    let root = h.worktree.clone();
    view.update(&mut h.visual, |v, cx| {
        v.receive(
            crate::surfaces::files::FilesEvent::Changed(
                runner_core::protocol::files::FilesChanged {
                    root,
                    paths: vec!["tracked.rs".into()],
                    folders: vec!["".into()],
                },
            ),
            cx,
        );
    });
    h.wait("changed_since_viewed after edit", |h| {
        h.read(|v| match &v.body {
            super::Body::Root(s) => s
                .files
                .iter()
                .find(|f| f.path == "tracked.rs")
                .is_some_and(|f| !f.viewed && f.changed_since_viewed),
            _ => false,
        })
    });
}

#[test]
fn edge_placeholders_and_root_states_render_expected_screens() {
    let mut h = harness(None);
    // CRLF-only modification on README.md, binary file, and mode-only file.
    std::fs::write(h.worktree.join("README.md"), b"# Runner\r\n").unwrap();
    std::fs::write(h.worktree.join("icon.bin"), b"\x00\x01\x02\x03").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(
            h.worktree.join("src/sub/util.rs"),
            std::fs::Permissions::from_mode(0o755),
        )
        .unwrap();
    }

    h.show(Some(mission_ctx("on-worktree")));
    h.wait("README.md CRLF placeholder", |h| {
        h.has("CHANGES_PLACEHOLDER_CRLF")
    });

    h.click("CHANGES_ROW icon.bin");
    h.wait("binary placeholder", |h| {
        h.has("CHANGES_PLACEHOLDER_BINARY")
    });
    assert!(h.has("CHANGES_OPEN_DEFAULT_APP"));

    #[cfg(unix)]
    {
        h.click("CHANGES_ROW src/sub/util.rs");
        h.wait("mode-only placeholder", |h| {
            h.has("CHANGES_PLACEHOLDER_MODE")
        });
    }

    // Remote, Choose, Missing, and NotInGit states.
    h.show(Some(chat_ctx("main-chat", Some("runner"), true)));
    h.wait("remote screen", |h| h.has("CHANGES_REMOTE"));

    h.show(Some(chat_ctx("outside", None, false)));
    h.wait("choose screen", |h| h.has("CHANGES_CHOOSE"));

    h.show(Some(mission_ctx("gone")));
    h.wait("missing screen", |h| h.has("CHANGES_MISSING"));

    let plain = h.plain.clone();
    let _ = plain;
    h.show(Some(chat_ctx("plain-chat", None, false)));
    h.wait("not-in-git screen", |h| h.has("CHANGES_NOT_GIT"));
}

#[test]
fn diff_footer_viewed_mark_binds_to_displayed_blob_version_across_races() {
    let mut h = harness(None);
    std::fs::write(h.worktree.join("tracked.rs"), "fn v1() {}\n").unwrap();
    h.show(Some(mission_ctx("on-worktree")));
    h.wait("tracked.rs v1 diff", |h| h.has("CHANGES_DIFF_VIEWED"));

    let v1_blob = h
        .read(|v| {
            v.loaded_diff_version()
                .and_then(|ver| ver.new_blob_id.clone())
        })
        .expect("v1 new_blob_id");

    // Hold incoming diff loads so `state.files` advances to v2 while `LoadedDiff` still shows v1.
    let view = h.view();
    view.update(&mut h.visual, |v, _| v.set_hold_diffs(true));

    std::fs::write(h.worktree.join("tracked.rs"), "fn v2() {}\n").unwrap();
    let root = h.worktree.clone();
    view.update(&mut h.visual, |v, cx| {
        v.receive(
            crate::surfaces::files::FilesEvent::Changed(
                runner_core::protocol::files::FilesChanged {
                    root,
                    paths: vec!["tracked.rs".into()],
                    folders: vec!["".into()],
                },
            ),
            cx,
        );
    });

    h.wait("list updated to v2 while diff still holds v1", |h| {
        h.read(|v| match &v.body {
            super::Body::Root(s) => s
                .files
                .iter()
                .find(|f| f.path == "tracked.rs")
                .and_then(|f| f.new_blob_id.as_ref())
                .is_some_and(|blob| blob != &v1_blob),
            _ => false,
        })
    });
    assert_eq!(
        h.read(|v| v
            .loaded_diff_version()
            .and_then(|ver| ver.new_blob_id.clone())),
        Some(v1_blob.clone()),
        "displayed diff must still be v1 while v2 load is in flight"
    );

    // Clicking the diff footer now marks v1 (what the user actually saw), NOT unseen v2!
    h.click("CHANGES_DIFF_VIEWED");
    h.wait("v2 list row shows unviewed + changed_since_viewed", |h| {
        h.read(|v| match &v.body {
            super::Body::Root(s) => s
                .files
                .iter()
                .find(|f| f.path == "tracked.rs")
                .is_some_and(|f| !f.viewed && f.changed_since_viewed),
            _ => false,
        })
    });

    // Wait for initial v1 highlights, then release v2 diff and verify stale highlights clear first
    // and v2 diff arrives unviewed with changed_since_viewed.
    let diff = h.read(|v| v.unified_diff_view()).unwrap();
    h.wait("v1 highlights ready", |h| {
        diff.read_with(&h.visual, |d, _| d.has_highlights().1)
    });
    view.update(&mut h.visual, |v, _| {
        v.set_hold_diffs(false);
        assert!(v.release_held_diffs() >= 1);
    });
    h.wait("v2 diff applied", |h| {
        h.read(|v| {
            v.loaded_diff_version()
                .and_then(|ver| ver.new_blob_id.as_ref())
                .is_some_and(|blob| blob != &v1_blob)
        })
    });
    assert!(h.read(|v| match &v.body {
        super::Body::Root(s) => match &s.diff_state {
            super::ActiveDiff::Loaded(loaded) => !loaded.viewed && loaded.changed_since_viewed,
            _ => false,
        },
        _ => false,
    }));

    // Reverse race: `changes_diff` returns v3 BEFORE `state.files` refreshes from v2.
    let v2_blob = h
        .read(|v| {
            v.loaded_diff_version()
                .and_then(|ver| ver.new_blob_id.clone())
        })
        .expect("v2 new_blob_id");
    view.update(&mut h.visual, |v, _| v.set_hold_snapshots(true));
    std::fs::write(h.worktree.join("tracked.rs"), "fn v3() {}\n").unwrap();
    view.update(&mut h.visual, |v, cx| {
        v.reload_diff_for_test("tracked.rs", cx)
    });

    h.wait("diff updated to v3 while list still holds v2", |h| {
        h.read(|v| {
            v.loaded_diff_version()
                .and_then(|ver| ver.new_blob_id.as_ref())
                .is_some_and(|blob| blob != &v2_blob)
        })
    });
    assert_eq!(
        h.read(|v| match &v.body {
            super::Body::Root(s) => s
                .files
                .iter()
                .find(|f| f.path == "tracked.rs")
                .and_then(|f| f.new_blob_id.clone()),
            _ => None,
        }),
        Some(v2_blob),
        "list still holds v2 before snapshot release"
    );

    // Clicking diff footer now marks v3 (the displayed diff), NOT v2 in `state.files`.
    h.click("CHANGES_DIFF_VIEWED");
    view.update(&mut h.visual, |v, _| {
        v.set_hold_snapshots(false);
        assert!(v.release_held_snapshots() >= 1);
    });
    h.wait("v3 marked viewed in daemon and list", |h| {
        h.read(|v| match &v.body {
            super::Body::Root(s) => s
                .files
                .iter()
                .find(|f| f.path == "tracked.rs")
                .is_some_and(|f| f.viewed && !f.changed_since_viewed),
            _ => false,
        })
    });
}

#[test]
fn enter_while_diff_is_loading_focuses_diff_on_arrival_without_stealing_if_focus_moved() {
    let mut h = harness(None);
    let mut lines: Vec<String> = (1..=30).map(|i| format!("let line_{i} = {i};")).collect();
    std::fs::write(h.worktree.join("two_hunks.rs"), lines.join("\n") + "\n").unwrap();
    std::fs::write(h.worktree.join("alpha.rs"), "pub fn alpha() {}\n").unwrap();
    git(&h.worktree, &["add", "two_hunks.rs", "alpha.rs"]);
    git(&h.worktree, &["commit", "-q", "-m", "add test files"]);
    std::fs::write(h.worktree.join("alpha.rs"), "pub fn alpha_next() {}\n").unwrap();
    lines[1] = "let line_2 = 200;".into();
    lines[27] = "let line_28 = 2800;".into();
    std::fs::write(h.worktree.join("two_hunks.rs"), lines.join("\n") + "\n").unwrap();

    h.show(Some(mission_ctx("on-worktree")));
    h.wait("alpha.rs diff", |h| h.has("CHANGES_DIFF_CODE"));
    h.click("CHANGES_ROW alpha.rs");

    // 1. Hold diff loads, select `two_hunks.rs`, press Enter while Loading -> focus transfers on arrival.
    let view = h.view();
    view.update(&mut h.visual, |v, _| v.set_hold_diffs(true));
    h.press("down");
    assert_eq!(
        h.read(|v| v.selected_path().map(str::to_owned)),
        Some("two_hunks.rs".into())
    );
    assert!(h.read(|v| v.is_diff_loading()));

    h.press("enter");
    assert_eq!(h.read(|v| v.focus_place()), FocusPlace::Diff);

    view.update(&mut h.visual, |v, _| {
        v.set_hold_diffs(false);
        assert_eq!(v.release_held_diffs(), 1);
    });
    h.wait("two_hunks.rs diff arrived", |h| h.has("CHANGES_DIFF_CODE"));

    let diff = h.read(|v| v.unified_diff_view()).unwrap();
    let initial_row = diff.read_with(&h.visual, |d, _| d.cursor_row());
    h.press("j");
    let stepped_row = diff.read_with(&h.visual, |d, _| d.cursor_row());
    assert_eq!(stepped_row, initial_row + 1);
    h.press("]");
    assert!(diff.read_with(&h.visual, |d, _| d.cursor_row()) > stepped_row);
    h.press("escape");
    assert_eq!(h.read(|v| v.focus_place()), FocusPlace::Files);

    // 2. Press Enter while Loading, then press Escape BEFORE diff arrives -> keeps focus in list.
    view.update(&mut h.visual, |v, _| v.set_hold_diffs(true));
    h.press("up");
    assert!(h.read(|v| v.is_diff_loading()));
    h.press("enter");
    assert_eq!(h.read(|v| v.focus_place()), FocusPlace::Diff);
    h.press("escape");
    assert_eq!(h.read(|v| v.focus_place()), FocusPlace::Files);

    view.update(&mut h.visual, |v, _| {
        v.set_hold_diffs(false);
        assert_eq!(v.release_held_diffs(), 1);
    });
    h.wait("alpha.rs diff arrived", |h| h.has("CHANGES_DIFF_CODE"));
    h.press("tab");
    assert_eq!(
        h.read(|v| v.focus_place()),
        FocusPlace::Commits,
        "list kept focus after Escape during loading"
    );
    h.press("tab");
    assert_eq!(h.read(|v| v.focus_place()), FocusPlace::Files);

    // 3. Press Enter while Loading, then focus an outside view BEFORE diff arrives -> does not steal focus.
    view.update(&mut h.visual, |v, _| v.set_hold_diffs(true));
    h.press("down");
    assert!(h.read(|v| v.is_diff_loading()));
    h.press("enter");
    h.host
        .update(&mut h.visual, |host, window, cx| {
            host.outside_focus.focus(window, cx);
        })
        .unwrap();
    h.settle();

    view.update(&mut h.visual, |v, _| {
        v.set_hold_diffs(false);
        assert_eq!(v.release_held_diffs(), 1);
    });
    h.wait("two_hunks.rs diff arrived without stealing focus", |h| {
        h.has("CHANGES_DIFF_CODE")
    });
    let outside_still_focused = h
        .host
        .update(&mut h.visual, |host, window, _| {
            host.outside_focus.is_focused(window)
        })
        .unwrap();
    assert!(
        outside_still_focused,
        "late diff completion must not steal focus from outside view"
    );
}

#[test]
fn tree_keyboard_expand_and_collapse_preserves_highlighted_folder_with_other_file_selected() {
    let mut h = harness(None);
    std::fs::create_dir_all(h.worktree.join("a_dir")).unwrap();
    std::fs::create_dir_all(h.worktree.join("b_dir")).unwrap();
    std::fs::write(h.worktree.join("a_dir/one.rs"), "pub fn one() {}\n").unwrap();
    std::fs::write(h.worktree.join("b_dir/two.rs"), "pub fn two() {}\n").unwrap();

    h.show(Some(mission_ctx("on-worktree")));
    h.wait("a_dir/one.rs diff", |h| h.has("CHANGES_ROW a_dir/one.rs"));
    h.click("CHANGES_MODE_TREE");
    h.wait("tree folders", |h| {
        h.has("CHANGES_FOLDER a_dir") && h.has("CHANGES_FOLDER b_dir")
    });

    // Select `a_dir/one.rs` (outside `b_dir`), then arrow down once onto folder `b_dir`.
    h.click("CHANGES_ROW a_dir/one.rs");
    assert_eq!(
        h.read(|v| v.selected_path().map(str::to_owned)),
        Some("a_dir/one.rs".into())
    );
    h.press("down");
    assert_eq!(
        h.read(|v| v.highlighted_row().cloned()),
        Some(ListRow::Folder {
            path: "b_dir".into(),
            name: "b_dir".into(),
            depth: 0,
            expanded: true,
        })
    );
    assert_eq!(
        h.read(|v| v.selected_path().map(str::to_owned)),
        Some("a_dir/one.rs".into())
    );

    // Left collapses `b_dir` and keeps highlight on `b_dir` (does NOT jump back to `a_dir/one.rs`).
    h.press("left");
    assert!(!h.has("CHANGES_ROW b_dir/two.rs"));
    assert_eq!(
        h.read(|v| v.highlighted_row().cloned()),
        Some(ListRow::Folder {
            path: "b_dir".into(),
            name: "b_dir".into(),
            depth: 0,
            expanded: false,
        })
    );
    assert_eq!(
        h.read(|v| v.selected_path().map(str::to_owned)),
        Some("a_dir/one.rs".into())
    );

    // Right expands `b_dir` and still keeps highlight on `b_dir`, so Down steps into `b_dir/two.rs`.
    h.press("right");
    assert!(h.has("CHANGES_ROW b_dir/two.rs"));
    assert_eq!(
        h.read(|v| v.highlighted_row().cloned()),
        Some(ListRow::Folder {
            path: "b_dir".into(),
            name: "b_dir".into(),
            depth: 0,
            expanded: true,
        })
    );
    assert_eq!(
        h.read(|v| v.selected_path().map(str::to_owned)),
        Some("a_dir/one.rs".into())
    );

    h.press("down");
    assert_eq!(
        h.read(|v| v.selected_path().map(str::to_owned)),
        Some("b_dir/two.rs".into())
    );

    // Left on a child file highlights its parent folder `b_dir`, and Left again collapses `b_dir`
    // (hiding `selected_path`) while keeping highlight on `b_dir`.
    h.press("left");
    assert_eq!(
        h.read(|v| v.highlighted_row().map(|r| r.path().to_owned())),
        Some("b_dir".into())
    );
    h.press("left");
    assert!(!h.has("CHANGES_ROW b_dir/two.rs"));
    assert_eq!(
        h.read(|v| v.highlighted_row().cloned()),
        Some(ListRow::Folder {
            path: "b_dir".into(),
            name: "b_dir".into(),
            depth: 0,
            expanded: false,
        })
    );
}

#[test]
fn removing_root_after_load_transitions_to_missing_when_shown_or_reopened() {
    let mut h = harness(None);
    let wt_live = h.repo_path.join(".worktrees/wt-live");
    let wt_target = h.repo_path.join(".worktrees/wt-target");
    let wt_hidden = h.repo_path.join(".worktrees/wt-hidden");
    for (branch, rel, dir) in [
        ("feat/live", ".worktrees/wt-live", &wt_live),
        ("feat/target", ".worktrees/wt-target", &wt_target),
        ("feat/hidden", ".worktrees/wt-hidden", &wt_hidden),
    ] {
        git(&h.repo_path, &["worktree", "add", "-q", "-b", branch, rel]);
        std::fs::write(dir.join("src/lib.rs"), "fn changed() {}\n").unwrap();
    }
    let wt_live = real(&wt_live);
    let wt_target = real(&wt_target);
    let wt_hidden = real(&wt_hidden);

    h.store.update(&mut h._cx, |store, cx| {
        store
            .test_core
            .db
            .get()
            .unwrap()
            .execute_batch(&format!(
                "INSERT INTO missions (id, title, status, started_at, cwd, project_id)
                 VALUES ('m-live', 'L', 'running', '2026-11-15T00:00:00Z', '{live}', 'runner'),
                        ('m-target', 'T', 'running', '2026-11-15T00:00:00Z', '{target}', 'runner'),
                        ('m-hidden', 'H', 'running', '2026-11-15T00:00:00Z', '{hidden}', 'runner');",
                live = sql(&wt_live),
                target = sql(&wt_target),
                hidden = sql(&wt_hidden),
            ))
            .unwrap();
        store.refresh(crate::app_store::StoreRefreshKind::All, cx);
    });

    // 1. Removed while shown -> watcher refresh resolves to Missing and clears stale list/diff.
    h.show(Some(mission_ctx("m-live")));
    h.wait("m-live diff", |h| {
        h.has("CHANGES_DIFF_CODE") && h.has("CHANGES_ROW src/lib.rs")
    });
    std::fs::remove_dir_all(&wt_live).unwrap();
    let view = h.view();
    view.update(&mut h.visual, |v, cx| {
        v.receive(
            crate::surfaces::files::FilesEvent::Changed(
                runner_core::protocol::files::FilesChanged {
                    root: wt_live.clone(),
                    paths: Vec::new(),
                    folders: vec!["".into()],
                },
            ),
            cx,
        );
    });
    h.wait("m-live missing screen", |h| h.has("CHANGES_MISSING"));
    assert!(!h.has("CHANGES_DIFF_CODE"));
    assert!(!h.has("CHANGES_ROW src/lib.rs"));

    // 2. Removed while shown -> target switch (`select_target`) resolves to Missing and clears stale UI.
    h.show(Some(mission_ctx("m-target")));
    h.wait("m-target diff", |h| h.has("CHANGES_DIFF_CODE"));
    std::fs::remove_dir_all(&wt_target).unwrap();
    let target_view = h.view();
    target_view.update(&mut h.visual, |v, cx| {
        let head = match &v.body {
            super::Body::Root(s) => s.commits.as_ref().unwrap().head_commit.clone().unwrap(),
            _ => unreachable!(),
        };
        v.select_target(SelectedTarget::Commit(head), cx);
    });
    h.wait("m-target missing screen", |h| h.has("CHANGES_MISSING"));
    assert!(!h.has("CHANGES_DIFF_CODE"));
    assert!(!h.has("CHANGES_ROW src/lib.rs"));

    // 3. Removed while hidden -> reopening resolves to Missing instead of keeping stale RootChanges.
    h.show(Some(mission_ctx("m-hidden")));
    h.wait("m-hidden diff", |h| h.has("CHANGES_DIFF_CODE"));
    h.show(None);
    std::fs::remove_dir_all(&wt_hidden).unwrap();
    h.show(Some(mission_ctx("m-hidden")));
    h.wait("m-hidden missing screen on reopen", |h| {
        h.has("CHANGES_MISSING")
    });
    assert!(!h.has("CHANGES_DIFF_CODE"));
    assert!(!h.has("CHANGES_ROW src/lib.rs"));
}

#[test]
fn removing_multiple_changed_files_advances_selection_to_surviving_neighbor() {
    let mut h = harness(None);
    let names = [
        "binary.bin",
        "conflict.txt",
        "delete-me.txt",
        "large.txt",
        "line-endings.txt",
        "long.txt",
        "mode.sh",
        "rename-old.txt",
    ];
    for name in names {
        std::fs::write(h.worktree.join(name), format!("base {name}\n")).unwrap();
    }
    git(&h.worktree, &["add", "-A"]);
    git(&h.worktree, &["commit", "-q", "-m", "seed tracked files"]);
    for name in names {
        std::fs::write(h.worktree.join(name), format!("dirty {name}\n")).unwrap();
    }

    h.show(Some(mission_ctx("on-worktree")));
    h.wait("dirty rows loaded", |h| h.has("CHANGES_ROW long.txt"));
    h.click("CHANGES_ROW long.txt");
    h.wait("long.txt selected", |h| {
        h.read(|v| v.selected_path() == Some("long.txt"))
    });

    // Restore both an earlier row (`binary.bin`) and the selected row (`long.txt`) in one update.
    git(&h.worktree, &["checkout", "--", "binary.bin", "long.txt"]);
    let view = h.view();
    let root = h.worktree.clone();
    view.update(&mut h.visual, |v, cx| {
        v.receive(
            crate::surfaces::files::FilesEvent::Changed(
                runner_core::protocol::files::FilesChanged {
                    root,
                    paths: vec!["binary.bin".into(), "long.txt".into()],
                    folders: vec!["".into()],
                },
            ),
            cx,
        );
    });

    h.wait("selection advanced after multi-file removal", |h| {
        !h.has("CHANGES_ROW long.txt") && !h.has("CHANGES_ROW binary.bin")
    });
    assert_eq!(
        h.read(|v| v.selected_path().map(str::to_owned)),
        Some("mode.sh".into()),
        "selection must advance to immediate surviving neighbor mode.sh, not skip to rename-old.txt"
    );
    assert_eq!(
        h.read(|v| v.highlighted_row().map(|r| r.path().to_owned())),
        Some("mode.sh".into())
    );
}
