use std::collections::BTreeSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::{Duration, Instant};

use gpui::{
    div, prelude::*, px, size, Context, Entity, FocusHandle, Modifiers, TestAppContext,
    VisualTestContext, Window, WindowHandle,
};
use runner_core::protocol::changes::{
    ChangedFile, ChangesBranches, ChangesCommits, ChangesTarget, CommitSummary, DiffCounts,
    ModeChange, RemoteBranches, MAX_COMMITS,
};
use runner_core::protocol::files::{FileStatus, WorktreeEntry, WorktreeKind};

use super::diff::DiffPos;
use super::model::{
    abbreviate_folder, base_chip_label, branch_header_label, build_base_picker_items,
    build_commit_rows, build_flat_rows, build_tree_rows, choose_initial_target,
    default_base_highlight, filter_base_picker_items, is_crlf_only_change, older_commits_hidden,
    reconcile_file_selection, BasePickerSection, CommitRow, FocusPlace, ListRow, SelectedTarget,
};
use super::ChangesView;
use crate::i18n::{self, Locale};
use crate::surfaces::files::picker::{
    checkout_session_labels, CheckoutSessionCandidate, CheckoutSessionKind,
};
use crate::surfaces::files::FilesPanel;
use crate::surfaces::side_panel::{PanelContext, PanelSubject, PanelSurface, Widened};

struct Host {
    panel: FilesPanel,
    context: Option<PanelContext>,
    outside_focus: FocusHandle,
    closed_panes: usize,
    app_zooms: Vec<&'static str>,
    tree_fits: bool,
    maximized: bool,
    maximize_requests: usize,
}

impl Render for Host {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = match &self.context {
            Some(context) => {
                let settings = &crate::app_store::global_app_store(cx).read(cx).settings;
                let widened = Some(Widened {
                    file: if self.maximized {
                        960.
                    } else {
                        settings.changes_viewer_width.max(288.)
                    },
                    tree: self.tree_fits,
                    tree_width: settings.file_tree_width,
                    standard: 288.,
                });
                let owner = cx.entity_id();
                let weak = cx.weak_entity();
                let on_request_maximize: crate::surfaces::side_panel::PanelMaximizeCallback =
                    std::rc::Rc::new(move |maximize, _, cx| {
                        let _ = weak.update(cx, |host, cx| {
                            host.maximized = maximize;
                            if maximize {
                                host.maximize_requests += 1;
                            }
                            cx.notify();
                        });
                    });
                if !self.maximized {
                    let _ = self
                        .panel
                        .show_changes_with(context, widened, Some(owner), cx);
                }
                Some(self.panel.show_changes_with_maximize(
                    context,
                    widened,
                    Some(owner),
                    self.maximized,
                    Some(on_request_maximize),
                    cx,
                ))
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
        tree_fits: true,
        maximized: false,
        maximize_requests: 0,
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
        following: None,
    }
}

fn chat_ctx(id: &str, project: Option<&str>, remote: bool) -> PanelContext {
    PanelContext {
        subject: PanelSubject::Session(id.into()),
        surface: PanelSurface::Chat,
        cwd: None,
        project: project.map(str::to_owned),
        remote,
        following: None,
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

    fn focused(&mut self, f: impl FnOnce(&ChangesView) -> &FocusHandle) -> bool {
        let view = self.view();
        self.host
            .update(&mut self.visual, |_, window, cx| {
                f(view.read(cx)).is_focused(window)
            })
            .unwrap()
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
fn opening_selection_branch_labels_and_base_chip_labels_cover_every_state() {
    let now = chrono::Utc::now();
    let commits = ChangesCommits {
        checkout_top: PathBuf::from("/tmp/repo"),
        branch: Some("feat/1141".into()),
        head_commit: Some("a1f3c9e".into()),
        base: Some("origin/main".into()),
        auto_base: Some("origin/main".into()),
        base_missing: false,
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
        worktrees: Vec::new(),
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
            base_chip_label(None, Some(&commits)),
            ("Auto · origin/main".into(), false)
        );
        assert_eq!(
            base_chip_label(Some("feat/645-runnerd"), Some(&commits)),
            ("feat/645-runnerd".into(), true)
        );

        let unborn = ChangesCommits {
            checkout_top: PathBuf::from("/tmp/repo"),
            branch: Some("main".into()),
            head_commit: None,
            base: None,
            auto_base: None,
            base_missing: false,
            commits: Vec::new(),
            total: 0,
            worktrees: Vec::new(),
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
            base_chip_label(None, Some(&unborn)),
            ("Auto · none".into(), false)
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
            base_chip_label(None, Some(&detached)),
            ("自动 · origin/main".into(), false)
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
    h.wait("All changes list", |h| h.has("CHANGES_ROW src/lib.rs"));
    // C1: Changes starts compact with no diff open until a file is clicked.
    assert!(!h.has("CHANGES_DIFF_CODE"));
    assert_eq!(h.read(|v| v.widening()), Some(false));
    h.click("CHANGES_ROW src/lib.rs");
    h.wait("All changes diff", |h| h.has("CHANGES_DIFF_CODE"));
    assert_eq!(h.read(|v| v.widening()), Some(true));
    assert_eq!(
        h.read(|v| v.selected_target().cloned()),
        Some(SelectedTarget::Branch)
    );
    assert!(h.has("CHANGES_COMMIT_ALL"));
    assert!(h.has("CHANGES_COMMIT_UNCOMMITTED"));
    assert!(h.has("CHANGES_ROW src/lib.rs"));
    assert!(h.has("CHANGES_ROW src/sub/added.rs"));
    // Clicking `×` closes the diff and shrinks back to compact list-only.
    h.click("CHANGES_CLOSE_DIFF");
    assert!(!h.has("CHANGES_DIFF_CODE"));
    assert_eq!(h.read(|v| v.widening()), Some(false));

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
    h.wait("two_hunks row", |h| h.has("CHANGES_ROW two_hunks.rs"));
    h.click("CHANGES_ROW two_hunks.rs");
    h.wait("two_hunks diff", |h| h.has("CHANGES_DIFF_CODE"));
    assert_eq!(
        h.read(|v| v.selected_path().map(str::to_owned)),
        Some("two_hunks.rs".into())
    );
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
    h.wait("src/lib.rs row", |h| h.has("CHANGES_ROW src/lib.rs"));
    h.click("CHANGES_ROW src/lib.rs");
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
    h.wait("tracked.rs row", |h| h.has("CHANGES_ROW tracked.rs"));
    h.click("CHANGES_ROW tracked.rs");
    h.wait("tracked.rs diff", |h| h.has("CHANGES_DIFF_VIEWED"));

    // Click end-of-diff viewed checkbox.
    h.click("CHANGES_DIFF_VIEWED");
    h.wait("daemon recorded viewed mark", |h| {
        let files = h.store.read_with(&h.visual, |store, _| {
            store
                .client
                .changes_files(&h.worktree, &ChangesTarget::Uncommitted, None)
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
    h.wait("README.md row", |h| h.has("CHANGES_ROW README.md"));
    h.click("CHANGES_ROW README.md");
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
    use runner_core::protocol::files::{
        FilesChanged, FilesStatusChanged, FILES_CHANGED_EVENT, FILES_STATUS_EVENT,
    };

    for control in ["CHANGES_DIFF_VIEWED", "CHANGES_HEADER_VIEWED"] {
        let mut h = harness(None);
        std::fs::write(h.worktree.join("tracked.rs"), "fn v1() {}\n").unwrap();
        let worktree_path = h.worktree.clone();
        let events = h
            .store
            .read_with(&h.visual, |store, _| store.test_core.events.clone());
        let mut sent = events.subscribe();

        h.show(Some(mission_ctx("on-worktree")));
        // Require observed startup FILES_STATUS_EVENT before probing live watcher delivery.
        let mut saw_startup = false;
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline && !saw_startup {
            h.visual.run_until_parked();
            loop {
                match sent.try_recv() {
                    Ok(event) if event.name == FILES_STATUS_EVENT => {
                        if serde_json::from_value::<FilesStatusChanged>(event.payload).is_ok_and(
                            |update| {
                                runner_core::app_paths::same_checkout_path(
                                    &update.root,
                                    &worktree_path,
                                )
                            },
                        ) {
                            saw_startup = true;
                            break;
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
            if !saw_startup {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        assert!(
            saw_startup,
            "{control}: timed out waiting for startup FILES_STATUS_EVENT"
        );

        // Prove the OS watcher is delivering live edits via an independent sentinel before writing
        // the target file once.
        let sentinel = worktree_path.join(".watch-ready");
        std::fs::write(&sentinel, "ready\n").unwrap();
        let mut saw_changed = false;
        let mut saw_status = false;
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline && !(saw_changed && saw_status) {
            h.visual.run_until_parked();
            loop {
                match sent.try_recv() {
                    Ok(event) if event.name == FILES_CHANGED_EVENT => {
                        if serde_json::from_value::<FilesChanged>(event.payload).is_ok_and(
                            |changed| {
                                runner_core::app_paths::same_checkout_path(
                                    &changed.root,
                                    &worktree_path,
                                ) && (changed.paths.iter().any(|p| p == ".watch-ready")
                                    || (changed.paths.is_empty() && changed.folders == [""]))
                            },
                        ) {
                            saw_changed = true;
                        }
                    }
                    Ok(event) if event.name == FILES_STATUS_EVENT => {
                        if serde_json::from_value::<FilesStatusChanged>(event.payload).is_ok_and(
                            |update| {
                                runner_core::app_paths::same_checkout_path(
                                    &update.root,
                                    &worktree_path,
                                ) && update
                                    .status
                                    .entries
                                    .iter()
                                    .any(|e| e.path == ".watch-ready")
                            },
                        ) {
                            saw_status = true;
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
            if !(saw_changed && saw_status) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        assert!(
            saw_changed && saw_status,
            "{control}: timed out waiting for sentinel readiness (saw_changed={saw_changed}, saw_status={saw_status})"
        );
        std::fs::remove_file(&sentinel).unwrap();
        h.wait("sentinel removed from Changes list", |h| {
            h.has("CHANGES_ROW tracked.rs") && !h.has("CHANGES_ROW .watch-ready")
        });

        h.click("CHANGES_ROW tracked.rs");
        h.wait("tracked.rs v1 diff", |h| h.has(control));

        let v1_blob = h
            .read(|v| {
                v.loaded_diff_version()
                    .and_then(|ver| ver.new_blob_id.clone())
            })
            .expect("v1 new_blob_id");

        // Hold incoming diff loads so `state.files` advances to v2 via the real watcher while
        // `LoadedDiff` still displays v1 (single write, no retries or synthetic event injection).
        let view = h.view();
        view.update(&mut h.visual, |v, _| v.set_hold_diffs(true));

        let tracked = h.worktree.join("tracked.rs");
        std::fs::write(&tracked, "fn v2() {}\n").unwrap();
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
            "{control}: displayed diff must still be v1 while v2 load is in flight"
        );

        // Clicking the Viewed control now marks v1 (what the user actually saw), NOT unseen v2!
        h.click(control);
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

        // Wait for initial v1 highlights AND the held v2 diff reply before releasing.
        let diff = h.read(|v| v.unified_diff_view()).unwrap();
        h.wait("v1 highlights and held v2 diff ready", |h| {
            diff.read_with(&h.visual, |d, _| d.has_highlights().1)
                && h.read(|v| !v.held_diffs.is_empty())
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
        std::fs::write(&tracked, "fn v3() {}\n").unwrap();
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
            "{control}: list still holds v2 before snapshot release"
        );

        // Clicking the Viewed control now marks v3 (the displayed diff), NOT v2 in `state.files`.
        h.click(control);
        // Wait for the watcher-driven snapshot to be held before releasing it.
        h.wait("v3 snapshot held", |h| {
            h.read(|v| !v.held_snapshots.is_empty())
        });
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
}

#[test]
fn changes_narrow_sheet_hides_list_column_when_widened_tree_is_false_and_restores_on_close() {
    let mut h = harness(None);
    h.show(Some(mission_ctx("on-worktree")));
    h.wait("src/lib.rs row", |h| h.has("CHANGES_ROW src/lib.rs"));
    h.click("CHANGES_ROW src/lib.rs");
    h.wait("src/lib.rs diff", |h| h.has("CHANGES_DIFF_CODE"));
    assert!(h.has("CHANGES_TREE_GUTTER"));

    // Simulate a 500 px narrow sheet where compute_panel_layout drops the list column (`tree: false`).
    h.host
        .update(&mut h.visual, |host, _, cx| {
            host.tree_fits = false;
            cx.notify();
        })
        .unwrap();
    h.visual.simulate_resize(size(px(420.), px(800.)));
    h.settle();
    assert!(h.has("CHANGES_DIFF_COLUMN"));
    assert!(
        !h.has("CHANGES_TREE_GUTTER"),
        "list column and resize gutter must hide when widened.tree is false"
    );
    assert!(
        !h.has("CHANGES_ROW src/lib.rs"),
        "list column must not shrink the narrow sheet diff"
    );

    // Clicking × closes the diff and restores the full-width Changes list rail.
    h.click("CHANGES_CLOSE_DIFF");
    h.wait("list restores on close", |h| {
        h.has("CHANGES_ROW src/lib.rs")
    });
    assert!(!h.has("CHANGES_DIFF_COLUMN"));
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
    h.wait("alpha.rs row", |h| h.has("CHANGES_ROW alpha.rs"));
    h.click("CHANGES_ROW alpha.rs");
    h.wait("alpha.rs diff", |h| h.has("CHANGES_DIFF_CODE"));

    // A live watcher refresh can queue another reply while a diff is held.
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
        assert!(v.release_held_diffs() >= 1);
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
        assert!(v.release_held_diffs() >= 1);
    });
    h.wait("alpha.rs diff arrived", |h| h.has("CHANGES_DIFF_CODE"));
    h.press("tab");
    assert_eq!(
        h.read(|v| v.focus_place()),
        FocusPlace::Commits,
        "list kept focus after Escape during loading"
    );
    h.press("shift-tab");
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
        assert!(v.release_held_diffs() >= 1);
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

    let events = h
        .store
        .read_with(&h.visual, |store, _| store.test_core.events.clone());
    let wait_for_event =
        |h: &mut Harness,
         sent: &mut tokio::sync::broadcast::Receiver<runner_daemon::events::AppEvent>,
         root: &Path,
         desc: &str,
         pred: &dyn Fn(&runner_daemon::events::AppEvent) -> bool| {
            let deadline = Instant::now() + Duration::from_secs(15);
            while Instant::now() < deadline {
                h.visual.run_until_parked();
                loop {
                    match sent.try_recv() {
                        Ok(event) if pred(&event) => return,
                        Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                        Err(_) => break,
                    }
                }
                std::thread::sleep(Duration::from_millis(10));
            }
            panic!("timed out waiting for {desc} on {}", root.display());
        };
    let open_and_await_watcher = |h: &mut Harness, mission_id: &str, root: &Path, label: &str| {
        use runner_core::protocol::files::{
            FilesChanged, FilesStatusChanged, FILES_CHANGED_EVENT, FILES_STATUS_EVENT,
        };
        let mut sent = events.subscribe();
        h.show(Some(mission_ctx(mission_id)));
        wait_for_event(h, &mut sent, root, "initial FILES_STATUS_EVENT", &|event| {
            event.name == FILES_STATUS_EVENT
                && serde_json::from_value::<FilesStatusChanged>(event.payload.clone()).is_ok_and(
                    |update| runner_core::app_paths::same_checkout_path(&update.root, root),
                )
        });
        h.wait(label, |h| h.has("CHANGES_ROW src/lib.rs"));
        h.click("CHANGES_ROW src/lib.rs");
        h.wait(label, |h| h.has("CHANGES_DIFF_CODE"));

        // Write a sentinel file and wait for both the OS watcher change event and
        // a status snapshot listing `.watch-ready` (in either order, since polling
        // emits Status before Changed).
        std::fs::write(root.join(".watch-ready"), "ready\n").unwrap();
        let mut saw_changed = false;
        let mut saw_status = false;
        let deadline = Instant::now() + Duration::from_secs(15);
        while Instant::now() < deadline && !(saw_changed && saw_status) {
            h.visual.run_until_parked();
            loop {
                match sent.try_recv() {
                    Ok(event) if event.name == FILES_CHANGED_EVENT => {
                        if serde_json::from_value::<FilesChanged>(event.payload).is_ok_and(
                            |changed| {
                                runner_core::app_paths::same_checkout_path(&changed.root, root)
                                    && (changed.paths.iter().any(|path| path == ".watch-ready")
                                        || (changed.paths.is_empty() && changed.folders == [""]))
                            },
                        ) {
                            saw_changed = true;
                        }
                    }
                    Ok(event) if event.name == FILES_STATUS_EVENT => {
                        if serde_json::from_value::<FilesStatusChanged>(event.payload).is_ok_and(
                            |update| {
                                runner_core::app_paths::same_checkout_path(&update.root, root)
                                    && update
                                        .status
                                        .entries
                                        .iter()
                                        .any(|entry| entry.path == ".watch-ready")
                            },
                        ) {
                            saw_status = true;
                        }
                    }
                    Ok(_) | Err(tokio::sync::broadcast::error::TryRecvError::Lagged(_)) => {}
                    Err(_) => break,
                }
            }
            if !(saw_changed && saw_status) {
                std::thread::sleep(Duration::from_millis(10));
            }
        }
        assert!(
            saw_changed && saw_status,
            "timed out waiting for sentinel readiness on {} (saw_changed={saw_changed}, saw_status={saw_status})",
            root.display()
        );
        h.settle();
        h.wait(label, |h| h.has("CHANGES_ROW src/lib.rs"));
        h.click("CHANGES_ROW src/lib.rs");
        h.wait(label, |h| h.has("CHANGES_DIFF_CODE"));
        assert!(
            h.store
                .read_with(&h.visual, |store, _| store.test_core.files.watched_roots())
                .iter()
                .any(|watched| runner_core::app_paths::same_checkout_path(watched, root)),
            "watcher must stay subscribed to {}",
            root.display()
        );
        while !matches!(
            sent.try_recv(),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty
                | tokio::sync::broadcast::error::TryRecvError::Closed)
        ) {}
        sent
    };
    let remove_checkout = |h: &mut Harness, root: &Path| {
        h.wait(
            &format!("removal of {}", root.display()),
            |_| match std::fs::remove_dir_all(root) {
                Ok(()) => true,
                Err(error) if cfg!(windows) && error.raw_os_error() == Some(32) => false,
                Err(error) => panic!("removing {}: {error}", root.display()),
            },
        );
    };

    // 1. Removed while shown -> production watcher detects removal, emits FILES_CHANGED_EVENT,
    // unwatches the gone root, and resolves Changes to Missing while clearing stale list/diff.
    let mut sent_live = open_and_await_watcher(&mut h, "m-live", &wt_live, "m-live diff");
    remove_checkout(&mut h, &wt_live);
    h.wait("m-live missing screen", |h| h.has("CHANGES_MISSING"));
    assert!(!h.has("CHANGES_DIFF_CODE"));
    assert!(!h.has("CHANGES_ROW src/lib.rs"));
    wait_for_event(
        &mut h,
        &mut sent_live,
        &wt_live,
        "removal FILES_CHANGED_EVENT",
        &|event| {
            event.name == runner_core::protocol::files::FILES_CHANGED_EVENT
                && serde_json::from_value::<runner_core::protocol::files::FilesChanged>(
                    event.payload.clone(),
                )
                .is_ok_and(|changed| {
                    runner_core::app_paths::same_checkout_path(&changed.root, &wt_live)
                        && changed.folders == [""]
                        && changed.paths.is_empty()
                })
        },
    );
    h.wait("m-live unwatches removed root", |h| {
        let watched = h
            .store
            .read_with(&h.visual, |store, _| store.test_core.files.watched_roots());
        !watched
            .iter()
            .any(|root| runner_core::app_paths::same_checkout_path(root, &wt_live))
    });

    // 2. Removed while shown -> target switch (`select_target`) resolves to Missing and clears stale UI.
    let _sent_target = open_and_await_watcher(&mut h, "m-target", &wt_target, "m-target diff");
    remove_checkout(&mut h, &wt_target);
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
    let _sent_hidden = open_and_await_watcher(&mut h, "m-hidden", &wt_hidden, "m-hidden diff");
    h.show(None);
    remove_checkout(&mut h, &wt_hidden);
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

#[test]
fn split_diff_pairs_changed_blocks_strips_crlf_and_copies_active_side_only() {
    use super::diff::{build_split_rows, DiffSide, SplitCell, SplitRow};
    use crate::app_settings::DiffViewMode;
    use crate::surfaces::files::document::Document;
    use runner_core::protocol::changes::{DiffHunk, DiffOp};

    let old_doc = Document::decode(b"ctx0\r\nrem1\r\nrem2\r\nrem3\r\nctx1\r\ndel_only\r\n");
    let new_doc = Document::decode(b"ctx0\nadd1\nctx1\nins1\nins2\n");
    let hunks = vec![DiffHunk {
        old_start: 1,
        old_lines: 6,
        new_start: 1,
        new_lines: 5,
        header: "@@ -1,6 +1,5 @@ fn demo()\r".into(),
        ops: vec![
            DiffOp::Context { count: 1 },
            DiffOp::Removed { count: 3 },
            DiffOp::Added { count: 1 },
            DiffOp::Context { count: 1 },
            DiffOp::Removed { count: 1 },
            DiffOp::Added { count: 2 },
        ],
    }];
    let (rows, hunk_rows, _) = build_split_rows(&old_doc, &new_doc, &hunks);
    assert_eq!(hunk_rows, vec![0]);
    assert_eq!(
        rows,
        vec![
            SplitRow::Hunk {
                header: "@@ -1,6 +1,5 @@ fn demo()".into(),
            },
            SplitRow::Pair {
                left: SplitCell::Context { line: 0 },
                right: SplitCell::Context { line: 0 },
            },
            SplitRow::Pair {
                left: SplitCell::Removed { line: 1 },
                right: SplitCell::Added { line: 1 },
            },
            SplitRow::Pair {
                left: SplitCell::Removed { line: 2 },
                right: SplitCell::Empty,
            },
            SplitRow::Pair {
                left: SplitCell::Removed { line: 3 },
                right: SplitCell::Empty,
            },
            SplitRow::Pair {
                left: SplitCell::Context { line: 4 },
                right: SplitCell::Context { line: 2 },
            },
            SplitRow::Pair {
                left: SplitCell::Removed { line: 5 },
                right: SplitCell::Added { line: 3 },
            },
            SplitRow::Pair {
                left: SplitCell::Empty,
                right: SplitCell::Added { line: 4 },
            },
        ]
    );

    let mut h = harness(None);
    let long_tail = " + 1".repeat(80);
    std::fs::write(
        h.worktree.join("pair.rs"),
        format!(
            "fn one() {{ let x = 1{long_tail}; }}\nfn two() {{\n    old_a();\n    old_b();\n}}\n{}\nfn tail() {{ old_tail(); }}\n",
            (1..=12)
                .map(|i| format!("let keep_{i} = {i};"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .unwrap();
    git(&h.worktree, &["add", "pair.rs"]);
    git(&h.worktree, &["commit", "-q", "-m", "add pair.rs"]);
    std::fs::write(
        h.worktree.join("pair.rs"),
        format!(
            "fn one() {{ let x = 1{long_tail}; }}\nfn two() {{\n    new_only();\n}}\n{}\nfn tail() {{\n    new_tail_1();\n    new_tail_2();\n}}\n",
            (1..=12)
                .map(|i| format!("let keep_{i} = {i};"))
                .collect::<Vec<_>>()
                .join("\n")
        ),
    )
    .unwrap();

    h.store.update(&mut h.visual, |store, cx| {
        store.update_settings(
            |settings| {
                settings.changes_viewer_width = 800.;
                true
            },
            false,
            cx,
        );
    });
    h.visual.simulate_resize(size(px(1100.), px(900.)));
    h.show(Some(mission_ctx("on-worktree")));
    h.wait("pair.rs row", |h| h.has("CHANGES_ROW pair.rs"));
    h.click("CHANGES_ROW pair.rs");
    h.wait("pair.rs diff", |h| h.has("CHANGES_DIFF_CODE"));

    h.click("CHANGES_DIFF_MODE_SPLIT");
    let diff = h.read(|v| v.unified_diff_view()).unwrap();
    assert_eq!(
        diff.read_with(&h.visual, |d, _| d.mode()),
        DiffViewMode::Split
    );
    let split_len = diff.read_with(&h.visual, |d, _| d.split_rows().len());
    assert!(split_len > 6, "split rows built: {split_len}");

    // Left-side selection copies only old/context lines without @@, gutter, or blank filler.
    diff.update(&mut h.visual, |d, cx| {
        d.set_split_selection_for_test(
            DiffSide::Left,
            DiffPos { row: 0, offset: 0 },
            DiffPos {
                row: 5,
                offset: 200,
            },
            cx,
        );
    });
    let left_copied = diff
        .read_with(&h.visual, |d, _| d.selected_text())
        .expect("left selected text");
    assert!(!left_copied.contains("@@"), "{left_copied:?}");
    assert!(left_copied.contains("old_a();"), "{left_copied:?}");
    assert!(left_copied.contains("old_b();"), "{left_copied:?}");
    assert!(!left_copied.contains("new_only();"), "{left_copied:?}");

    // Right-side selection copies only new/context lines without @@, gutter, or blank filler.
    diff.update(&mut h.visual, |d, cx| {
        d.set_split_selection_for_test(
            DiffSide::Right,
            DiffPos { row: 0, offset: 0 },
            DiffPos {
                row: 5,
                offset: 200,
            },
            cx,
        );
    });
    let right_copied = diff
        .read_with(&h.visual, |d, _| d.selected_text())
        .expect("right selected text");
    assert!(!right_copied.contains("@@"), "{right_copied:?}");
    assert!(right_copied.contains("new_only();"), "{right_copied:?}");
    assert!(!right_copied.contains("old_a();"), "{right_copied:?}");
    assert!(!right_copied.contains("old_b();"), "{right_copied:?}");

    // Keyboard navigation in Split: Enter focuses diff, j/k moves row, ]/[ jumps hunks,
    // right/left scrolls horizontally in sync, Escape returns to list.
    h.click("CHANGES_ROW pair.rs");
    h.press("enter");
    assert_eq!(h.read(|v| v.focus_place()), FocusPlace::Diff);
    let row0 = diff.read_with(&h.visual, |d, _| d.cursor_row());
    h.press("j");
    let row1 = diff.read_with(&h.visual, |d, _| d.cursor_row());
    assert_eq!(row1, row0 + 1);
    h.press("]");
    let hunk2 = diff.read_with(&h.visual, |d, _| d.cursor_row());
    assert!(hunk2 > row1, "{hunk2} vs {row1}");
    h.press("[");
    assert!(diff.read_with(&h.visual, |d, _| d.cursor_row()) < hunk2);
    h.press("right");
    let sx = diff.read_with(&h.visual, |d, _| d.scroll_offset().x);
    assert!(sx > px(0.), "split horizontal scroll: {sx:?}");
    h.press("escape");
    assert_eq!(h.read(|v| v.focus_place()), FocusPlace::Files);
}

#[test]
fn split_360px_rule_auto_maximizes_narrow_panel_and_falls_back_to_unified_on_restore() {
    use crate::app_settings::DiffViewMode;

    let mut h = harness(None);
    h.store.update(&mut h.visual, |store, cx| {
        store.update_settings(
            |settings| {
                settings.changes_viewer_width = 520.;
                settings.changes_diff_mode = DiffViewMode::Unified;
                true
            },
            false,
            cx,
        );
    });
    h.show(Some(mission_ctx("on-worktree")));
    h.wait("src/lib.rs row", |h| h.has("CHANGES_ROW src/lib.rs"));
    h.click("CHANGES_ROW src/lib.rs");
    h.wait("src/lib.rs diff", |h| h.has("CHANGES_DIFF_CODE"));

    let diff = h.read(|v| v.unified_diff_view()).unwrap();
    assert_eq!(
        diff.read_with(&h.visual, |d, _| d.mode()),
        DiffViewMode::Unified
    );

    // At 520 px (< 720 px, so < 360 px per side), picking Split saves the preference
    // and requests side-panel maximize so Split can render.
    h.click("CHANGES_DIFF_MODE_SPLIT");
    assert_eq!(
        h.store
            .read_with(&h.visual, |store, _| store.settings.changes_diff_mode),
        DiffViewMode::Split
    );
    assert_eq!(
        h.host
            .read_with(&h.visual, |host, _| (
                host.maximized,
                host.maximize_requests
            ))
            .unwrap(),
        (true, 1)
    );
    assert_eq!(
        diff.read_with(&h.visual, |d, _| d.mode()),
        DiffViewMode::Split
    );

    // Restore chat while panel width is still 520 px: preference stays Split,
    // rendered mode falls back to Unified.
    h.host
        .update(&mut h.visual, |host, _, cx| {
            host.maximized = false;
            cx.notify();
        })
        .unwrap();
    h.settle();
    assert_eq!(
        h.store
            .read_with(&h.visual, |store, _| store.settings.changes_diff_mode),
        DiffViewMode::Split,
        "remembered preference stays Split"
    );
    assert_eq!(
        diff.read_with(&h.visual, |d, _| d.mode()),
        DiffViewMode::Unified,
        "narrow unmaximized panel falls back to Unified"
    );

    // Widening the diff past 720 px (360 px per side) renders Split without maximizing.
    h.store.update(&mut h.visual, |store, cx| {
        store.update_settings(
            |settings| {
                settings.changes_viewer_width = 760.;
                true
            },
            false,
            cx,
        );
    });
    h.settle();
    assert_eq!(
        diff.read_with(&h.visual, |d, _| d.mode()),
        DiffViewMode::Split
    );
    assert!(!h
        .host
        .read_with(&h.visual, |host, _| host.maximized)
        .unwrap());
}

#[test]
fn narrow_diff_header_keeps_unified_split_switch_in_bounds_and_clickable_for_long_paths() {
    use crate::app_settings::DiffViewMode;

    let mut h = harness(None);
    let rel = "src/deep/nested/very_long_component_filename_that_exceeds_narrow_panel_header.rs";
    let full = h.worktree.join(rel);
    std::fs::create_dir_all(full.parent().unwrap()).unwrap();
    std::fs::write(&full, "fn before() {}\n").unwrap();
    git(&h.worktree, &["add", rel]);
    git(&h.worktree, &["commit", "-q", "-m", "add long path"]);
    std::fs::write(&full, "fn after() {\n    let x = 1;\n}\n").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(&full, std::fs::Permissions::from_mode(0o755)).unwrap();
    }

    h.store.update(&mut h.visual, |store, cx| {
        store.update_settings(
            |settings| {
                settings.changes_viewer_width = 288.;
                settings.changes_diff_mode = DiffViewMode::Unified;
                true
            },
            false,
            cx,
        );
    });
    h.show(Some(mission_ctx("on-worktree")));
    h.visual.simulate_resize(size(px(288. + 240.), px(900.)));
    h.settle();
    let selector = format!("CHANGES_ROW {rel}");
    h.wait("long-path row", |h| h.has(&selector));
    h.click(&selector);
    h.wait("long-path diff code", |h| h.has("CHANGES_DIFF_CODE"));

    let header = h.visual.debug_bounds("CHANGES_DIFF_HEADER").unwrap();
    let mode_box = h.visual.debug_bounds("CHANGES_DIFF_MODE").unwrap();
    let unified_btn = h
        .visual
        .debug_bounds("CHANGES_DIFF_MODE_UNIFIED")
        .or_else(|| h.visual.debug_bounds("CHANGES_DIFF_MODE_UNIFIED active"))
        .unwrap();
    let split_btn = h
        .visual
        .debug_bounds("CHANGES_DIFF_MODE_SPLIT")
        .or_else(|| h.visual.debug_bounds("CHANGES_DIFF_MODE_SPLIT active"))
        .unwrap();

    assert_eq!(header.size.width, px(288.));
    assert!(
        mode_box.left() >= header.left() && mode_box.right() <= header.right(),
        "mode switch {mode_box:?} must stay inside header {header:?}"
    );
    assert!(
        unified_btn.left() >= header.left() && split_btn.right() <= header.right(),
        "Unified {unified_btn:?} and Split {split_btn:?} must stay inside header {header:?}"
    );

    // Clicking Split inside the 288 px header hits the control and triggers auto-maximize.
    h.click("CHANGES_DIFF_MODE_SPLIT");
    assert_eq!(
        h.store
            .read_with(&h.visual, |store, _| store.settings.changes_diff_mode),
        DiffViewMode::Split
    );
    assert!(h
        .host
        .read_with(&h.visual, |host, _| host.maximized)
        .unwrap());
    let diff = h.read(|v| v.unified_diff_view()).unwrap();
    assert_eq!(
        diff.read_with(&h.visual, |d, _| d.mode()),
        DiffViewMode::Split
    );
    assert!(
        !h.has("CHANGES_DIFF_COMPARE"),
        "Changes diff header omits the comparison label"
    );
}

#[test]
fn base_picker_filtering_omits_own_branch_and_upstream_and_hides_auto_unless_matching() {
    let branches = ChangesBranches {
        local: vec![
            "feat/child".into(),
            "feat/umbrella".into(),
            "main".into(),
            "origin/main".into(),
        ],
        remotes: vec![
            RemoteBranches {
                remote: "origin".into(),
                branches: vec![
                    "origin/feat/child".into(),
                    "origin/feat/umbrella".into(),
                    "origin/main".into(),
                ],
            },
            RemoteBranches {
                remote: "upstream".into(),
                branches: vec!["upstream/main".into()],
            },
        ],
        upstream: Some("refs/remotes/origin/feat/child".into()),
    };
    let items = build_base_picker_items(
        Some("refs/heads/origin/main"),
        Some("origin/main"),
        Some("feat/child"),
        &branches,
        "Auto",
        "none",
    );
    let labels: Vec<&str> = items.iter().map(|i| i.label.as_str()).collect();
    assert_eq!(
        labels,
        vec![
            "Auto",
            "feat/umbrella",
            "main",
            "origin/main",
            "origin/feat/umbrella",
            "origin/main",
            "upstream/main",
        ],
        "own branch and its remote upstream are omitted while colliding short names remain in separate sections"
    );
    // Colliding short names keep distinct canonical refs and check only the selected LOCAL row.
    let checked: Vec<(&BasePickerSection, Option<&str>)> = items
        .iter()
        .filter(|i| i.current)
        .map(|i| (&i.section, i.base_ref.as_deref()))
        .collect();
    assert_eq!(
        checked,
        vec![(&BasePickerSection::Local, Some("refs/heads/origin/main"))]
    );
    assert_eq!(
        base_chip_label(Some("refs/heads/origin/main"), None),
        ("origin/main".into(), true)
    );

    // Selecting the remote `refs/remotes/origin/main` checks only the ORIGIN row.
    let remote_items = build_base_picker_items(
        Some("refs/remotes/origin/main"),
        Some("origin/main"),
        Some("feat/child"),
        &branches,
        "Auto",
        "none",
    );
    let remote_checked: Vec<(&BasePickerSection, Option<&str>)> = remote_items
        .iter()
        .filter(|i| i.current)
        .map(|i| (&i.section, i.base_ref.as_deref()))
        .collect();
    assert_eq!(
        remote_checked,
        vec![(
            &BasePickerSection::Remote("origin".into()),
            Some("refs/remotes/origin/main")
        )]
    );

    // Local upstream (`branch.<name>.remote = .`) omits the LOCAL upstream branch while keeping remote `origin/feat/umbrella`.
    let local_upstream_branches = ChangesBranches {
        upstream: Some("refs/heads/feat/umbrella".into()),
        ..branches.clone()
    };
    let local_up_items = build_base_picker_items(
        None,
        Some("origin/main"),
        Some("feat/child"),
        &local_upstream_branches,
        "Auto",
        "none",
    );
    assert!(
        !local_up_items
            .iter()
            .any(|i| i.base_ref.as_deref() == Some("refs/heads/feat/umbrella")),
        "local upstream must be omitted"
    );
    assert!(
        local_up_items
            .iter()
            .any(|i| i.base_ref.as_deref() == Some("refs/remotes/origin/feat/umbrella")),
        "remote branch sharing the short name stays available"
    );

    let visible_all = filter_base_picker_items(&items, "");
    assert_eq!(default_base_highlight(&items, &visible_all, ""), 3);

    let visible_umb = filter_base_picker_items(&items, "umbrella");
    let umb_labels: Vec<&str> = visible_umb
        .iter()
        .map(|&idx| items[idx].label.as_str())
        .collect();
    assert_eq!(
        umb_labels,
        vec!["feat/umbrella", "origin/feat/umbrella"],
        "Auto is hidden when query does not match it"
    );
    assert_eq!(default_base_highlight(&items, &visible_umb, "umbrella"), 0);

    let visible_auto = filter_base_picker_items(&items, "auto");
    assert_eq!(visible_auto, vec![0]);
}

#[test]
fn checkout_session_labels_cover_own_session_chat_terminal_mission_slot_and_several_sessions() {
    i18n::testing::with_locale(Locale::English, || {
        let wt = |path: &str, branch: &str, kind: WorktreeKind| WorktreeEntry {
            path: PathBuf::from(path),
            branch: Some(branch.into()),
            commit: Some("1234567".into()),
            kind,
            locked: false,
            prunable: false,
            uncommitted: Some(0),
        };
        let worktrees = vec![
            wt("/repo", "main", WorktreeKind::Main),
            wt(
                "/repo/.worktrees/wt-chat",
                "feat/chat",
                WorktreeKind::Linked,
            ),
            wt(
                "/repo/.worktrees/wt-term",
                "feat/term",
                WorktreeKind::Linked,
            ),
            wt(
                "/repo/.worktrees/wt-slot",
                "feat/slot",
                WorktreeKind::Linked,
            ),
            wt(
                "/repo/.worktrees/wt-many",
                "feat/many",
                WorktreeKind::Linked,
            ),
        ];
        let candidates = vec![
            CheckoutSessionCandidate {
                cwd: PathBuf::from("/repo/.worktrees/wt-chat/src"),
                kind: CheckoutSessionKind::Chat,
            },
            CheckoutSessionCandidate {
                cwd: PathBuf::from("/repo/.worktrees/wt-term"),
                kind: CheckoutSessionKind::Terminal,
            },
            CheckoutSessionCandidate {
                cwd: PathBuf::from("/repo/.worktrees/wt-slot"),
                kind: CheckoutSessionKind::MissionSlot("coder".into()),
            },
            CheckoutSessionCandidate {
                cwd: PathBuf::from("/repo/.worktrees/wt-many"),
                kind: CheckoutSessionKind::Chat,
            },
            CheckoutSessionCandidate {
                cwd: PathBuf::from("/repo/.worktrees/wt-many/sub"),
                kind: CheckoutSessionKind::MissionSlot("qa".into()),
            },
        ];

        let labels = checkout_session_labels(
            PanelSurface::Chat,
            Some(Path::new("/repo")),
            &worktrees,
            &candidates,
        );
        assert_eq!(labels[Path::new("/repo")].text, "this chat");
        assert!(labels[Path::new("/repo")].is_own);
        assert_eq!(labels[Path::new("/repo/.worktrees/wt-chat")].text, "chat");
        assert!(!labels[Path::new("/repo/.worktrees/wt-chat")].is_own);
        assert_eq!(
            labels[Path::new("/repo/.worktrees/wt-term")].text,
            "terminal"
        );
        assert_eq!(
            labels[Path::new("/repo/.worktrees/wt-slot")].text,
            "mission · coder"
        );
        assert_eq!(
            labels[Path::new("/repo/.worktrees/wt-many")].text,
            "2 sessions"
        );

        let mission_labels = checkout_session_labels(
            PanelSurface::Mission,
            Some(Path::new("/repo/.worktrees/wt-slot")),
            &worktrees,
            &candidates,
        );
        assert_eq!(
            mission_labels[Path::new("/repo/.worktrees/wt-slot")].text,
            "this mission"
        );
        assert!(
            !mission_labels.contains_key(Path::new("/repo")),
            "nested .worktrees sessions must not match the main checkout"
        );
    });
}

fn wait_watcher_ready(h: &mut Harness, root: &Path) {
    use runner_core::protocol::files::{FilesStatusChanged, FILES_STATUS_EVENT};
    let events = h
        .store
        .read_with(&h.visual, |store, _| store.test_core.events.clone());
    let mut sent = events.subscribe();
    let deadline = Instant::now() + Duration::from_secs(5);
    while Instant::now() < deadline {
        h.visual.run_until_parked();
        if std::iter::from_fn(|| sent.try_recv().ok()).any(|event| {
            event.name == FILES_STATUS_EVENT
                && serde_json::from_value::<FilesStatusChanged>(event.payload).is_ok_and(|update| {
                    runner_core::app_paths::same_checkout_path(&update.root, root)
                })
        }) {
            break;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    std::thread::sleep(Duration::from_millis(250));
    h.settle();
}

#[test]
fn base_row_picker_keyboard_persistence_and_missing_base_notice() {
    let mut h = harness(None);
    let first_feat_commit = {
        let out = Command::new("git")
            .current_dir(&h.worktree)
            .args(["rev-parse", "HEAD~1"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    git(
        &h.repo_path,
        &["branch", "feat/umbrella", &first_feat_commit],
    );
    git(
        &h.repo_path,
        &[
            "update-ref",
            "refs/remotes/upstream/umbrella",
            &first_feat_commit,
        ],
    );
    std::fs::write(h.worktree.join("src/extra.rs"), "pub fn extra() {}\n").unwrap();
    git(&h.worktree, &["add", "-A"]);
    git(&h.worktree, &["commit", "-q", "-m", "feat: extra commit"]);

    let worktree_path = h.worktree.clone();
    h.show(Some(mission_ctx("on-worktree")));
    wait_watcher_ready(&mut h, &worktree_path);
    h.wait("Auto · main base chip", |h| {
        h.has("CHANGES_BASE_CHIP Auto · main")
    });
    assert!(h.has("CHANGES_BRANCH_COUNT 3 commits"));

    // Focus Changes via `focus_default` (starts on `Files`) and traverse via real Tab keystrokes:
    // Files -> Commits -> Base chip -> Checkout button -> Files (and Shift-Tab backwards).
    let view = h.view();
    h.host
        .update(&mut h.visual, |_, window, cx| {
            view.update(cx, |v, cx| v.focus_default(window, cx));
        })
        .unwrap();
    h.settle();
    assert!(h.focused(|v| &v.list_focus) && h.read(|v| v.focus_place() == FocusPlace::Files));

    h.press("tab");
    assert!(h.focused(|v| &v.list_focus) && h.read(|v| v.focus_place() == FocusPlace::Commits));
    h.press("tab");
    assert!(
        h.focused(|v| &v.base_chip_focus),
        "second Tab reaches the Base chip trigger"
    );
    h.press("tab");
    assert!(
        h.focused(|v| &v.checkout_button_focus),
        "third Tab reaches the Checkout picker trigger"
    );
    h.press("shift-tab");
    assert!(
        h.focused(|v| &v.base_chip_focus),
        "Shift-Tab returns from Checkout button to Base chip"
    );

    // Open BasePicker with Enter, filter and pick `feat/umbrella` with Enter; verify focus returns to Base chip.
    h.press("enter");
    h.wait("base picker item for umbrella", |h| {
        h.has("CHANGES_BASE_PICKER_ITEM 1 feat/umbrella")
    });
    h.visual.simulate_input("feat/umbrella");
    h.settle();
    assert!(
        h.has("CHANGES_BASE_PICKER_ITEM 0 feat/umbrella"),
        "typing hides Auto and leaves feat/umbrella at index 0"
    );
    h.press("enter");

    h.wait("chosen base chip feat/umbrella", |h| {
        h.has("CHANGES_BASE_CHIP feat/umbrella") && h.has("CHANGES_BRANCH_COUNT 2 commits")
    });
    assert!(!h.has("CHANGES_BASE_PICKER"));
    assert!(
        h.focused(|v| &v.base_chip_focus),
        "picking a base returns focus to the Base chip"
    );
    assert_eq!(
        h.store.read_with(&h.visual, |store, _| {
            store
                .settings
                .changes_base(&h.worktree, Some("feat/x"))
                .map(str::to_owned)
        }),
        Some("refs/heads/feat/umbrella".to_owned()),
        "chosen base is saved as canonical full ref in settings keyed by checkout and branch"
    );

    // Picking Auto removes the saved entry.
    h.click("CHANGES_BASE_CHIP feat/umbrella");
    h.wait("base picker Auto row", |h| {
        h.has("CHANGES_BASE_PICKER_ITEM 0 Auto")
    });
    h.click("CHANGES_BASE_PICKER_ITEM 0 Auto");
    h.wait("Auto · main restored", |h| {
        h.has("CHANGES_BASE_CHIP Auto · main") && h.has("CHANGES_BRANCH_COUNT 3 commits")
    });
    assert_eq!(
        h.store.read_with(&h.visual, |store, _| {
            store
                .settings
                .changes_base(&h.worktree, Some("feat/x"))
                .map(str::to_owned)
        }),
        None,
        "picking Auto removes the entry"
    );

    // Pick local `feat/umbrella`, then delete the branch on disk -> real OS watcher event (no manual load_snapshot!)
    // refreshes Changes, falls back to Auto with amber notice, and removes saved entry.
    h.click("CHANGES_BASE_CHIP Auto · main");
    h.wait("umbrella row", |h| {
        h.has("CHANGES_BASE_PICKER_ITEM 1 feat/umbrella")
    });
    h.click("CHANGES_BASE_PICKER_ITEM 1 feat/umbrella");
    h.wait("chosen umbrella", |h| {
        h.has("CHANGES_BASE_CHIP feat/umbrella")
    });

    git(&h.repo_path, &["branch", "-D", "feat/umbrella"]);
    h.wait("watcher-driven missing local base notice", |h| {
        h.has("CHANGES_MISSING_BASE_NOTICE") && h.has("CHANGES_BASE_CHIP Auto · main")
    });
    assert_eq!(
        h.store.read_with(&h.visual, |store, _| {
            store
                .settings
                .changes_base(&h.worktree, Some("feat/x"))
                .map(str::to_owned)
        }),
        None,
        "deleted base removes saved entry"
    );

    // Opening the picker clears the missing-base notice; pick remote `upstream/umbrella`, then delete remote ref ->
    // real OS watcher event refreshes Changes for remote-tracking refs too.
    h.click("CHANGES_BASE_CHIP Auto · main");
    h.wait("base picker open", |h| {
        h.has("CHANGES_BASE_PICKER_ITEM 2 upstream/umbrella")
    });
    assert!(
        !h.has("CHANGES_MISSING_BASE_NOTICE"),
        "opening the picker clears the missing-base notice"
    );
    h.click("CHANGES_BASE_PICKER_ITEM 2 upstream/umbrella");
    h.wait("chosen remote base upstream/umbrella", |h| {
        h.has("CHANGES_BASE_CHIP upstream/umbrella") && h.has("CHANGES_BRANCH_COUNT 2 commits")
    });

    git(
        &h.repo_path,
        &["update-ref", "-d", "refs/remotes/upstream/umbrella"],
    );
    h.wait("watcher-driven missing remote base notice", |h| {
        h.has("CHANGES_MISSING_BASE_NOTICE") && h.has("CHANGES_BASE_CHIP Auto · main")
    });
}

#[test]
fn selecting_target_while_base_snapshot_is_pending_preserves_new_base_commits_and_target() {
    let mut h = harness(None);
    let first_feat_commit = {
        let out = Command::new("git")
            .current_dir(&h.worktree)
            .args(["rev-parse", "HEAD~1"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    let tip_short = {
        let out = Command::new("git")
            .current_dir(&h.worktree)
            .args(["rev-parse", "--short", "HEAD"])
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    };
    git(
        &h.repo_path,
        &["branch", "feat/umbrella", &first_feat_commit],
    );
    std::fs::write(h.worktree.join("dirty.txt"), "uncommitted\n").unwrap();

    let worktree_path = h.worktree.clone();
    h.show(Some(mission_ctx("on-worktree")));
    wait_watcher_ready(&mut h, &worktree_path);
    h.wait("initial 2 commits since main", |h| {
        h.has("CHANGES_BRANCH_COUNT 2 commits") && h.has("CHANGES_ROW dirty.txt")
    });

    let view = h.view();
    view.update(&mut h.visual, |v, _| {
        v.hold_snapshots = true;
    });

    // Pick `refs/heads/feat/umbrella` while snapshots are held.
    h.host
        .update(&mut h.visual, |_, window, cx| {
            view.update(cx, |v, cx| {
                v.pick_base(Some("refs/heads/feat/umbrella".into()), window, cx);
            });
        })
        .unwrap();
    h.settle();
    h.wait("held snapshot queued", |h| {
        h.read(|v| !v.held_snapshots.is_empty())
    });

    // Before the snapshot releases, select the tip commit target (`src/sub/added.rs`).
    h.click(format!("CHANGES_COMMIT {tip_short}"));
    h.wait(
        "tip commit file loaded while base snapshot still held",
        |h| h.has("CHANGES_ROW src/sub/added.rs"),
    );
    assert!(
        h.has("CHANGES_BRANCH_COUNT 2 commits"),
        "snapshot metadata has not landed yet while held"
    );

    // Release the held base snapshot -> commit count & list update to 1 commit (ahead of feat/umbrella)
    // AND the selected tip commit's files (`src/sub/added.rs`) remain intact.
    view.update(&mut h.visual, |v, _| {
        v.hold_snapshots = false;
        for tx in v.held_snapshots.drain(..) {
            let _ = tx.send(());
        }
    });
    h.wait(
        "base metadata applied without losing target selection",
        |h| {
            h.has("CHANGES_BASE_CHIP feat/umbrella")
                && h.has("CHANGES_BRANCH_COUNT 1 commit")
                && h.has("CHANGES_ROW src/sub/added.rs")
        },
    );
}

#[test]
fn missing_base_notice_allows_shared_settings_sync_and_clears_on_branch_switch() {
    let mut h = harness(None);
    git(&h.repo_path, &["branch", "feat/gone-base"]);
    git(&h.repo_path, &["branch", "feat/shared-base"]);
    h.store.update(&mut h.visual, |store, cx| {
        store.update_settings(
            |s| {
                let a = s.set_changes_base(
                    &h.worktree,
                    Some("feat/x"),
                    Some("refs/heads/feat/gone-base"),
                );
                let b = s.set_changes_base(&h.worktree, Some("feat/y"), Some("refs/heads/main"));
                a || b
            },
            false,
            cx,
        );
    });

    let worktree_path = h.worktree.clone();
    h.show(Some(mission_ctx("on-worktree")));
    wait_watcher_ready(&mut h, &worktree_path);
    h.wait("chosen gone-base chip", |h| {
        h.has("CHANGES_BASE_CHIP feat/gone-base")
    });

    // 1. Delete `feat/gone-base` -> missing-base notice appears.
    git(&h.repo_path, &["branch", "-D", "feat/gone-base"]);
    h.wait("missing base notice for feat/gone-base", |h| {
        h.has("CHANGES_MISSING_BASE_NOTICE") && h.has("CHANGES_BASE_CHIP Auto · main")
    });

    // Another window/session saves `refs/heads/feat/shared-base` for `(worktree, feat/x)` ->
    // panel synchronizes to the new shared base and clears the missing-base notice.
    h.store.update(&mut h.visual, |store, cx| {
        store.update_settings(
            |s| {
                s.set_changes_base(
                    &h.worktree,
                    Some("feat/x"),
                    Some("refs/heads/feat/shared-base"),
                )
            },
            false,
            cx,
        );
    });
    h.wait("shared base applied and notice cleared", |h| {
        h.has("CHANGES_BASE_CHIP feat/shared-base") && !h.has("CHANGES_MISSING_BASE_NOTICE")
    });

    // 2. Delete `feat/shared-base` so notice appears again, then switch checkout to `feat/y` (which has valid saved base `main`) ->
    // notice clears and `main` becomes the active base.
    git(&h.repo_path, &["branch", "-D", "feat/shared-base"]);
    h.wait("missing base notice for feat/shared-base", |h| {
        h.has("CHANGES_MISSING_BASE_NOTICE")
    });

    git(&h.worktree, &["checkout", "-q", "-b", "feat/y"]);
    h.wait(
        "switched to feat/y with saved base main and notice cleared",
        |h| h.has("CHANGES_BASE_CHIP main") && !h.has("CHANGES_MISSING_BASE_NOTICE"),
    );
}

#[test]
fn single_checkout_is_plain_and_multiple_checkouts_share_files_root_and_handle_removal() {
    let mut h = harness(None);
    // Single-checkout standalone git repo -> plain branch row, which becomes a picker automatically
    // when a second worktree is created on disk (via watcher events, without manual load_snapshot).
    let single_dir = h._temp.path().join("single-repo");
    setup_repo(&single_dir);
    let single_dir = real(&single_dir);
    h.store.update(&mut h.visual, |store, cx| {
        store
            .test_core
            .db
            .get()
            .unwrap()
            .execute_batch(&format!(
                "INSERT INTO projects (id, name, cwd, position, created_at)
                 VALUES ('single', 'single', '{single}', 1, '2026-11-15T00:00:00Z');
                 INSERT INTO sessions (id, status, agent_runtime, agent_command, cwd, project_id)
                 VALUES ('single-chat', 'running', 'codex', 'codex', '{single}', 'single');",
                single = sql(&single_dir),
            ))
            .unwrap();
        store.refresh(crate::app_store::StoreRefreshKind::All, cx);
    });
    h.show(Some(chat_ctx("single-chat", Some("single"), false)));
    wait_watcher_ready(&mut h, &single_dir);
    h.wait("single-repo base row", |h| h.has("CHANGES_BASE_ROW"));
    assert!(h.has("CHANGES_BRANCH_HEADER"));
    assert!(
        !h.has("CHANGES_CHECKOUT_BUTTON"),
        "one checkout keeps the plain branch row"
    );

    git(
        &single_dir,
        &["worktree", "add", "-q", "--detach", ".worktrees/second"],
    );
    h.wait("watcher turns plain branch row into checkout picker", |h| {
        h.has("CHANGES_CHECKOUT_BUTTON")
    });
    git(
        &single_dir,
        &["worktree", "remove", "--force", ".worktrees/second"],
    );
    h.wait(
        "watcher returns branch row to plain when second worktree is removed",
        |h| !h.has("CHANGES_CHECKOUT_BUTTON"),
    );

    // Multi-checkout repo (`main-chat` on `repo_path`, with linked worktree `feat/x`).
    std::fs::write(h.worktree.join("untracked_wt.txt"), "dirty\n").unwrap();
    let repo_path = h.repo_path.clone();
    let worktree_path = h.worktree.clone();
    h.show(Some(chat_ctx("main-chat", Some("runner"), false)));
    wait_watcher_ready(&mut h, &repo_path);
    h.wait("multi-checkout main button", |h| {
        h.has("CHANGES_CHECKOUT_KIND_MAIN") && h.has("CHANGES_CHECKOUT_BUTTON")
    });

    // Traverse via keyboard (`Shift-Tab` from default `Files` focus lands directly on `Checkout` trigger),
    // open CheckoutPicker with Enter, filter and pick `feat/x` with Enter, and verify focus returns to Checkout trigger.
    let view = h.view();
    h.host
        .update(&mut h.visual, |_, window, cx| {
            view.update(cx, |v, cx| v.focus_default(window, cx));
        })
        .unwrap();
    h.settle();
    h.press("shift-tab");
    assert!(
        h.focused(|v| &v.checkout_button_focus),
        "Shift-Tab from Files reaches Checkout button"
    );
    h.press("enter");
    h.wait("checkout picker rows", |h| {
        h.has("CHANGES_CHECKOUT_PICKER_ITEM 0 main")
            && h.has("CHANGES_CHECKOUT_PICKER_ITEM 1 feat/x")
    });
    h.visual.simulate_input("feat-x");
    h.settle();
    assert!(h.has("CHANGES_CHECKOUT_PICKER_ITEM 0 feat/x"));
    h.press("enter");

    h.wait(
        "viewing feat/x with tint, shortened count and return button",
        |h| {
            h.has("CHANGES_BRANCH_HEADER other")
                && h.has("CHANGES_CHECKOUT_KIND_LINKED")
                && h.has("CHANGES_CHECKOUT_RETURN")
                && h.has("CHANGES_BRANCH_COUNT 2")
                && h.has("CHANGES_ROW untracked_wt.txt")
        },
    );
    assert!(
        h.focused(|v| &v.checkout_button_focus),
        "picking a checkout preserves return focus on the Checkout trigger"
    );

    // Verify Files' root for `main-chat` was updated to `h.worktree` too (`files_set_root`).
    let resolved_for_files = h.store.read_with(&h.visual, |store, _| {
        store
            .client
            .files_root(&runner_core::protocol::files::FilesSubject::Session(
                "main-chat".into(),
            ))
            .unwrap()
    });
    match resolved_for_files {
        runner_core::protocol::files::FilesRootOutcome::Root(r) => {
            assert_eq!(
                runner_core::app_paths::normalize_checkout_path(&r.path),
                runner_core::app_paths::normalize_checkout_path(&h.worktree),
            );
        }
        other => panic!("expected saved worktree root, got {other:?}"),
    }

    // Tab from Checkout trigger reaches the Return button, and Enter switches back to own checkout (`None`).
    h.press("tab");
    assert!(h.focused(|v| &v.checkout_return_focus));
    h.press("enter");
    h.wait("returned to main checkout", |h| {
        h.has("CHANGES_CHECKOUT_KIND_MAIN") && !h.has("CHANGES_CHECKOUT_RETURN")
    });

    // Switch back to `feat/x`, wait for watcher on `h.worktree`, then remove `h.worktree` ->
    // real OS watcher event falls back to own checkout with amber notice (no manual load_snapshot!).
    h.click("CHANGES_CHECKOUT_BUTTON");
    h.wait("checkout picker", |h| {
        h.has("CHANGES_CHECKOUT_PICKER_ITEM 1 feat/x")
    });
    h.click("CHANGES_CHECKOUT_PICKER_ITEM 1 feat/x");
    h.wait("viewing feat/x again", |h| h.has("CHANGES_CHECKOUT_RETURN"));
    wait_watcher_ready(&mut h, &worktree_path);

    git(
        &h.repo_path,
        &["worktree", "remove", "--force", ".worktrees/feat-x"],
    );
    h.wait("removed worktree amber notice on own checkout", |h| {
        h.has("CHANGES_REMOVED_CHECKOUT_NOTICE") && !h.has("CHANGES_CHECKOUT_RETURN")
    });
    assert!(
        !h.has("CHANGES_CHECKOUT_BUTTON"),
        "with feat/x removed, only one checkout remains so branch row returns to plain"
    );
}

#[test]
fn changes_diff_survives_eviction_past_kept_views_without_active_watches() {
    let mut h = harness(Some(mission_ctx("on-worktree")));
    h.wait("worktree changes list loaded", |h| {
        h.has("CHANGES_ROW src/lib.rs")
    });
    h.click("CHANGES_ROW src/lib.rs");
    h.wait("src/lib.rs diff opened", |h| h.has("CHANGES_DIFF_CODE"));

    let origin = PanelSubject::Mission("on-worktree".into());
    for i in 0..14 {
        h.show(Some(chat_ctx(&format!("evict-other-{i}"), None, false)));
    }
    let evicted_and_widening = h
        .host
        .read_with(&h.visual, |host, cx| {
            (
                host.panel.changes_view(&origin).is_none(),
                host.panel.changes_widening(&origin, cx),
            )
        })
        .unwrap();
    assert_eq!(
        evicted_and_widening,
        (true, Some(true)),
        "evicted Changes subject must drop its live view while preserving open diff metadata"
    );
    h.wait("evicted Changes watches released", |h| {
        h.store.read_with(&h.visual, |store, _| {
            store.test_core.files.watched_roots().is_empty()
        })
    });

    // Re-evict before root resolution finishes: pending saved state must survive rather than drop.
    h.host
        .update(&mut h.visual, |host, _, cx| {
            host.context = Some(mission_ctx("on-worktree"));
            cx.notify();
        })
        .unwrap();
    h.visual.update(|window, _| window.refresh());
    assert!(
        h.host
            .read_with(&h.visual, |host, _| host
                .panel
                .changes_view(&origin)
                .is_some())
            .unwrap(),
        "revisiting evicted Changes subject must instantiate a fresh view before re-eviction"
    );
    for i in 14..28 {
        h.show(Some(chat_ctx(&format!("evict-other-{i}"), None, false)));
    }
    assert_eq!(
        h.host
            .read_with(&h.visual, |host, cx| (
                host.panel.changes_view(&origin).is_none(),
                host.panel.changes_widening(&origin, cx),
            ))
            .unwrap(),
        (true, Some(true)),
        "pending saved Changes state must survive re-eviction before root resolution completes"
    );
    h.wait("re-evicted Changes watches released", |h| {
        h.store.read_with(&h.visual, |store, _| {
            store.test_core.files.watched_roots().is_empty()
        })
    });

    // Revisit on the SAME checkout -> src/lib.rs diff restores automatically.
    h.show(Some(mission_ctx("on-worktree")));
    h.wait("evicted open diff restores automatically on revisit", |h| {
        h.has("CHANGES_DIFF_CODE") && h.has("CHANGES_DIFF_HEADER")
    });

    // Evict again, modify the same relative path (`src/lib.rs`) in the main checkout, and switch
    // `on-worktree`'s root to the main checkout: revisiting must NOT silently open `main`'s diff.
    std::fs::write(h.repo_path.join("src/lib.rs"), "pub fn main_only() {}\n").unwrap();
    for i in 28..42 {
        h.show(Some(chat_ctx(&format!("evict-other-{i}"), None, false)));
    }
    h.wait("second eviction Changes watches released", |h| {
        h.store.read_with(&h.visual, |store, _| {
            store.test_core.files.watched_roots().is_empty()
        })
    });
    let main_repo = h.repo_path.clone();
    h.store.read_with(&h.visual, |store, _| {
        store
            .client
            .files_set_root(
                &runner_core::protocol::files::FilesSubject::Mission("on-worktree".into()),
                Some(&main_repo),
            )
            .unwrap();
    });
    h.show(Some(mission_ctx("on-worktree")));
    h.wait(
        "switched checkout loads Changes list without restoring old-root diff",
        |h| {
            h.has("CHANGES_ROW src/lib.rs")
                && h.read(|v| match &v.body {
                    super::Body::Root(s) => {
                        s.commits.is_some()
                            && runner_core::app_paths::same_checkout_path(&s.root.path, &main_repo)
                    }
                    _ => false,
                })
        },
    );
    assert!(
        !h.has("CHANGES_DIFF_CODE") && !h.has("CHANGES_DIFF_COLUMN"),
        "saved Changes diff must not reopen under a different checkout root"
    );
}
