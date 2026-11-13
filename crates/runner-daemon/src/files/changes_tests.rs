use super::*;
use crate::files::fixture::{git, repo};
use crate::test_support::test_core;
use runner_core::protocol::changes::{
    ChangedFile, ChangesDiff, ChangesTarget, ChangesViewed, DiffBody, DiffCounts, DiffOp,
    CHANGES_VIEWED_EVENT,
};
use runner_core::protocol::files::FileStatus;
use std::path::Path;

fn commit_all(dir: &Path, message: &str) -> String {
    git(dir, &["add", "-A"]);
    git(dir, &["commit", "-q", "-m", message]);
    git(dir, &["rev-parse", "HEAD"]).trim().to_owned()
}

fn hash_object(dir: &Path, path: &Path) -> String {
    git(dir, &["hash-object", path.to_str().unwrap()])
        .trim()
        .to_owned()
}

#[test]
fn parses_numstat_z_with_renames_copies_binary_and_newlines() {
    let raw = b"12\t3\tsrc/main.rs\x00-\t-\tassets/logo.bin\x004\t2\t\x00old dir/a b.txt\x00new dir/c d.txt\x001\t0\t\x00tmpl.txt\x00copied\nwith space.txt\x00";
    let parsed = parse_numstat_z(raw).unwrap();
    assert_eq!(
        parsed,
        vec![
            NumstatEntry {
                path: "src/main.rs".into(),
                old_path: None,
                counts: DiffCounts::Lines {
                    added: 12,
                    removed: 3
                },
            },
            NumstatEntry {
                path: "assets/logo.bin".into(),
                old_path: None,
                counts: DiffCounts::Binary,
            },
            NumstatEntry {
                path: "new dir/c d.txt".into(),
                old_path: Some("old dir/a b.txt".into()),
                counts: DiffCounts::Lines {
                    added: 4,
                    removed: 2
                },
            },
            NumstatEntry {
                path: "copied\nwith space.txt".into(),
                old_path: Some("tmpl.txt".into()),
                counts: DiffCounts::Lines {
                    added: 1,
                    removed: 0
                },
            },
        ]
    );
}

#[test]
fn parses_raw_and_numstat_z_including_copies_and_mode_changes() {
    let raw = b":100644 100644 1111111111111111111111111111111111111111 2222222222222222222222222222222222222222 C090\x00src/a.rs\x00src/a copy.rs\x00:100644 100755 3333333333333333333333333333333333333333 3333333333333333333333333333333333333333 M\x00script.sh\x001\t0\t\x00src/a.rs\x00src/a copy.rs\x000\t0\tscript.sh\x00";
    let entries = parse_raw_and_numstat_z(raw).unwrap();
    assert_eq!(entries.len(), 2);
    assert_eq!(entries[0].status, "C090");
    assert_eq!(entries[0].old_path.as_deref(), Some("src/a.rs"));
    assert_eq!(entries[0].path, "src/a copy.rs");
    assert_eq!(
        entries[0].counts,
        DiffCounts::Lines {
            added: 1,
            removed: 0
        }
    );
    assert_eq!(entries[1].old_mode, "100644");
    assert_eq!(entries[1].new_mode, "100755");
    assert_eq!(
        entries[1].counts,
        DiffCounts::Lines {
            added: 0,
            removed: 0
        }
    );
}

#[test]
fn parses_hunks_with_no_newline_at_end_of_file() {
    let diff = b"\
diff --git a/f.txt b/f.txt
index 1234567..89abcde 100644
--- a/f.txt
+++ b/f.txt
@@ -1,2 +1,3 @@ fn top()
 first
-last
\\ No newline at end of file
+last
+added
\\ No newline at end of file
@@ -10 +11,2 @@
-old
\\ No newline at end of file
+new1
+new2
";
    let hunks = parse_hunks(diff).unwrap();
    assert_eq!(hunks.len(), 2);
    assert_eq!(hunks[0].header, "@@ -1,2 +1,3 @@ fn top()");
    assert_eq!(
        (
            hunks[0].old_start,
            hunks[0].old_lines,
            hunks[0].new_start,
            hunks[0].new_lines
        ),
        (1, 2, 1, 3)
    );
    assert_eq!(
        hunks[0].ops,
        vec![
            DiffOp::Context { count: 1 },
            DiffOp::Removed { count: 1 },
            DiffOp::Added { count: 2 },
        ]
    );
    assert_eq!(hunks[1].header, "@@ -10 +11,2 @@");
    assert_eq!(
        (
            hunks[1].old_start,
            hunks[1].old_lines,
            hunks[1].new_start,
            hunks[1].new_lines
        ),
        (10, 1, 11, 2)
    );
    assert_eq!(
        hunks[1].ops,
        vec![DiffOp::Removed { count: 1 }, DiffOp::Added { count: 2 }]
    );
}

#[test]
fn parses_hunks_omitted_counts_and_trailing_no_newline() {
    let diff = b"\
@@ -1 +1,2 @@
-old
\\ No newline at end of file
+new1
+new2
@@ -9,2 +10 @@ fn end()
-b
-c
+d
";
    let hunks = parse_hunks(diff).unwrap();
    assert_eq!(hunks.len(), 2);
    assert_eq!(
        hunks[0].ops,
        vec![DiffOp::Removed { count: 1 }, DiffOp::Added { count: 2 }]
    );
    assert_eq!(
        hunks[1].ops,
        vec![DiffOp::Removed { count: 2 }, DiffOp::Added { count: 1 }]
    );
}

#[test]
fn parses_commits_z_with_root_normal_and_merge_parent_counts() {
    let payload = b"aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa\x1faaaaaaa\x1fmerge base\x1fAlice\x1f2026-11-13T10:00:00+08:00\x1f1111 2222\x00\
                    bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb\x1fbbbbbbb\x1fsingle\x1fBob\x1f2026-11-12T09:00:00Z\x1f0000\x00\
                    cccccccccccccccccccccccccccccccccccccccc\x1fccccccc\x1froot commit\x1fCarol\x1f2026-11-11T08:00:00Z\x1f";
    let commits = parse_commits_z(payload).unwrap();
    assert_eq!(commits.len(), 3);
    assert_eq!(commits[0].parent_count, 2);
    assert_eq!(commits[0].short_id, "aaaaaaa");
    assert_eq!(commits[1].parent_count, 1);
    assert_eq!(commits[1].author, "Bob");
    assert_eq!(commits[2].parent_count, 0);
    assert_eq!(commits[2].subject, "root commit");
}

#[test]
fn sha1_matches_git_hash_object() {
    let fixture = repo();
    let p = fixture.path().join("hello.txt");
    std::fs::write(&p, "hello\r\nworld\n").unwrap();
    assert_eq!(
        git_blob_hash(b"hello\r\nworld\n"),
        hash_object(fixture.path(), &p)
    );
    assert_eq!(
        git_blob_hash(b""),
        "e69de29bb2d1d6434b8b29ae775ad8c2e48c5391"
    );
}

#[test]
fn base_resolution_covers_origin_head_fallbacks_merged_detached_unborn_and_limit() {
    let core = test_core();

    // 1. Repository with no commits yet (unborn HEAD).
    let unborn_dir = tempfile::tempdir().unwrap();
    let unborn = unborn_dir.path();
    git(unborn, &["init", "-q", "-b", "main"]);
    git(unborn, &["config", "core.autocrlf", "false"]);
    std::fs::write(unborn.join("first.txt"), "one\ntwo\n").unwrap();
    let commits = changes_commits(&core, unborn).unwrap();
    assert_eq!(commits.branch.as_deref(), Some("main"));
    assert_eq!(commits.head_commit, None);
    assert_eq!(commits.base, None);
    assert!(commits.commits.is_empty());
    assert_eq!(commits.total, 0);
    let unborn_files = changes_files(&core, unborn, &ChangesTarget::Uncommitted).unwrap();
    assert_eq!(unborn_files.files.len(), 1);
    assert_eq!(unborn_files.files[0].status, FileStatus::Added);
    assert_eq!(
        unborn_files.files[0].counts,
        DiffCounts::Lines {
            added: 2,
            removed: 0
        }
    );

    // 2. Priority order: local main/master when no origin -> level with base lists 0, ahead lists 2.
    let fixture = repo();
    let path = fixture.path();
    let level = changes_commits(&core, path).unwrap();
    assert_eq!(level.branch.as_deref(), Some("main"));
    assert!(level.head_commit.is_some());
    assert_eq!(level.base.as_deref(), Some("main"));
    assert!(level.commits.is_empty());
    assert_eq!(level.total, 0);

    git(path, &["checkout", "-q", "-b", "feat"]);
    std::fs::write(path.join("a.txt"), "c1\n").unwrap();
    commit_all(path, "first feat");
    std::fs::write(path.join("a.txt"), "c2\n").unwrap();
    commit_all(path, "second feat");
    let local_base = changes_commits(&core, path).unwrap();
    assert_eq!(local_base.base.as_deref(), Some("main"));
    assert_eq!(local_base.commits.len(), 2);
    assert_eq!(local_base.commits[0].subject, "second feat");
    assert_eq!(local_base.commits[1].subject, "first feat");

    // 3. With origin remote whose refs are absent (`origin/trunk` only), do NOT fall back to local main.
    git(
        path,
        &[
            "remote",
            "add",
            "origin",
            "https://example.invalid/repo.git",
        ],
    );
    let trunk_sha = git(path, &["rev-parse", "main"]).trim().to_owned();
    git(
        path,
        &["update-ref", "refs/remotes/origin/trunk", &trunk_sha],
    );
    let no_match = changes_commits(&core, path).unwrap();
    assert_eq!(no_match.base, None);
    assert!(no_match.commits.is_empty());

    // 4. origin/master -> origin/main -> origin/HEAD priority order + merge of base into branch.
    git(
        path,
        &["update-ref", "refs/remotes/origin/master", &trunk_sha],
    );
    assert_eq!(
        changes_commits(&core, path).unwrap().base.as_deref(),
        Some("origin/master")
    );

    // Advance main, point origin/main to advanced main, merge origin/main into feat.
    git(path, &["checkout", "-q", "main"]);
    std::fs::write(path.join("old.txt"), "base moved\n").unwrap();
    let main2 = commit_all(path, "upstream main commit");
    git(path, &["update-ref", "refs/remotes/origin/main", &main2]);
    git(path, &["checkout", "-q", "feat"]);
    git(path, &["merge", "-q", "--no-edit", "origin/main"]);

    let after_merge = changes_commits(&core, path).unwrap();
    assert_eq!(after_merge.base.as_deref(), Some("origin/main"));
    assert_eq!(after_merge.commits.len(), 3);
    assert_eq!(after_merge.commits[0].parent_count, 2);
    assert!(
        after_merge
            .commits
            .iter()
            .all(|c| c.subject != "upstream main commit"),
        "base's own commits stay excluded"
    );

    // origin/HEAD symbolic ref takes top precedence.
    git(
        path,
        &[
            "symbolic-ref",
            "refs/remotes/origin/HEAD",
            "refs/remotes/origin/trunk",
        ],
    );
    let sym = changes_commits(&core, path).unwrap();
    assert_eq!(sym.base.as_deref(), Some("origin/trunk"));

    // 5. Detached HEAD keeps commits reachable since base.
    git(path, &["checkout", "-q", "--detach", "HEAD"]);
    let detached = changes_commits(&core, path).unwrap();
    assert_eq!(detached.branch, None);
    assert!(detached.head_commit.is_some());
    assert!(!detached.commits.is_empty());

    // 6. No base at all on an isolated topic branch; falling back to local master when added.
    let no_base_dir = tempfile::tempdir().unwrap();
    let nb = no_base_dir.path();
    git(nb, &["init", "-q", "-b", "topic"]);
    git(nb, &["config", "core.autocrlf", "false"]);
    std::fs::write(nb.join("f.txt"), "1\n").unwrap();
    commit_all(nb, "only commit");
    let res = changes_commits(&core, nb).unwrap();
    assert_eq!(res.branch.as_deref(), Some("topic"));
    assert_eq!(res.base, None);
    assert!(res.commits.is_empty());
    assert_eq!(res.total, 0);

    git(nb, &["branch", "master", "HEAD"]);
    std::fs::write(nb.join("f.txt"), "2\n").unwrap();
    commit_all(nb, "topic ahead");
    let master_base = changes_commits(&core, nb).unwrap();
    assert_eq!(master_base.base.as_deref(), Some("master"));
    assert_eq!(master_base.commits.len(), 1);

    // 7. 205 commits caps list at 200 while reporting total = 205.
    let big_dir = tempfile::tempdir().unwrap();
    let big = big_dir.path();
    git(big, &["init", "-q", "-b", "main"]);
    git(big, &["config", "core.autocrlf", "false"]);
    std::fs::write(big.join("n.txt"), "0\n").unwrap();
    commit_all(big, "base");
    git(big, &["checkout", "-q", "-b", "many"]);
    let mut fast_import = String::new();
    for i in 1..=205 {
        let content = format!("{i}\n");
        let msg = format!("c{i}\n");
        let parent = if i == 1 {
            "from refs/heads/many^0\n"
        } else {
            ""
        };
        fast_import.push_str(&format!(
            "commit refs/heads/many\ncommitter Runner <runner@example.com> {} +0000\ndata {}\n{msg}{parent}M 100644 inline n.txt\ndata {}\n{content}\n",
            1_700_000_000 + i,
            msg.len(),
            content.len()
        ));
    }
    let mut child = std::process::Command::new("git")
        .current_dir(big)
        .args(["fast-import", "--quiet"])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .stdin(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    use std::io::Write;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(fast_import.as_bytes())
        .unwrap();
    assert!(child.wait().unwrap().success());
    git(big, &["reset", "-q", "--hard", "many"]);
    let capped = changes_commits(&core, big).unwrap();
    assert_eq!(capped.commits.len(), 200);
    assert_eq!(capped.total, 205);
    assert_eq!(capped.commits[0].subject, "c205");
}

#[cfg(unix)]
#[test]
fn file_kinds_and_edge_cases_match_git() {
    use runner_core::protocol::files::MAX_READ_BYTES;
    use std::os::unix::fs::{symlink, PermissionsExt};

    let core = test_core();
    let sub_src_dir = tempfile::tempdir().unwrap();
    let sub_src = sub_src_dir.path();
    git(sub_src, &["init", "-q", "-b", "main"]);
    std::fs::write(sub_src.join("s.txt"), "s1\n").unwrap();
    let sub1 = commit_all(sub_src, "s1");
    std::fs::write(sub_src.join("s.txt"), "s2\n").unwrap();
    let sub2 = commit_all(sub_src, "s2");

    let fixture = repo();
    let root = fixture.path();

    // Seed initial committed files for each special case.
    std::fs::write(root.join("lf_crlf.txt"), "one\ntwo\n").unwrap();
    std::fs::write(root.join("mode_only.sh"), "#!/bin/sh\necho hi\n").unwrap();
    std::fs::write(root.join("mode_and_edit.sh"), "#!/bin/sh\necho a\n").unwrap();
    std::fs::write(root.join("to_delete.txt"), "bye1\nbye2\n").unwrap();
    std::fs::write(root.join("to_rename.txt"), "rename body\nline 2\n").unwrap();
    std::fs::write(root.join("conflict.txt"), "base\n").unwrap();
    symlink("old_target", root.join("link")).unwrap();
    git(
        root,
        &[
            "-c",
            "protocol.file.allow=always",
            "submodule",
            "add",
            "-q",
            sub_src.to_str().unwrap(),
            "mysub",
        ],
    );
    git(root, &["-C", "mysub", "checkout", "-q", &sub1]);
    git(root, &["add", "mysub"]);
    let base_commit = commit_all(root, "seed cases");

    // Branch + conflict setup.
    git(root, &["checkout", "-q", "-b", "side"]);
    std::fs::write(root.join("conflict.txt"), "theirs\n").unwrap();
    let side_commit = commit_all(root, "theirs");
    git(root, &["checkout", "-q", "main"]);
    std::fs::write(root.join("conflict.txt"), "ours\n").unwrap();
    let ours_commit = commit_all(root, "ours");
    let merge = std::process::Command::new("git")
        .current_dir(root)
        .args([
            "-c",
            "user.name=Runner",
            "-c",
            "user.email=runner@example.com",
            "-c",
            "commit.gpgsign=false",
            "merge",
            "--no-edit",
            &side_commit,
        ])
        .env_remove("GIT_DIR")
        .env_remove("GIT_WORK_TREE")
        .env_remove("GIT_INDEX_FILE")
        .output()
        .unwrap();
    let merge_stdout = String::from_utf8_lossy(&merge.stdout);
    let merge_stderr = String::from_utf8_lossy(&merge.stderr);
    assert!(
        !merge.status.success()
            && (merge_stdout.contains("CONFLICT") || merge_stderr.contains("CONFLICT")),
        "expected merge conflict, got stdout={merge_stdout:?} stderr={merge_stderr:?}"
    );

    // Untracked nested non-submodule git repo (must NOT be listed).
    let nested = root.join("nested_repo");
    std::fs::create_dir_all(&nested).unwrap();
    git(&nested, &["init", "-q", "-b", "main"]);
    std::fs::write(nested.join("inner.txt"), "x\n").unwrap();
    commit_all(&nested, "nested");

    // Uncommitted edits covering every edge case:
    std::fs::write(root.join("a.txt"), "a edited\nadded\n").unwrap();
    std::fs::write(root.join("added.txt"), "new tracked\n").unwrap();
    git(root, &["add", "added.txt"]);
    git(root, &["rm", "-q", "to_delete.txt"]);
    git(root, &["mv", "to_rename.txt", "src/deep/renamed.txt"]);
    std::fs::write(root.join("untracked.txt"), "u1\nu2\nu3").unwrap();
    std::fs::write(root.join("bin_untracked.dat"), [0u8, 159, 146, 150]).unwrap();
    std::fs::write(root.join("bin.dat"), [9u8, 0, 8, 7, 6]).unwrap();
    std::fs::write(root.join("lf_crlf.txt"), "one\r\ntwo\r\n").unwrap();
    std::fs::set_permissions(
        root.join("mode_only.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::set_permissions(
        root.join("mode_and_edit.sh"),
        std::fs::Permissions::from_mode(0o755),
    )
    .unwrap();
    std::fs::write(root.join("mode_and_edit.sh"), "#!/bin/sh\necho b\n").unwrap();
    std::fs::remove_file(root.join("link")).unwrap();
    symlink("../outside_target", root.join("link")).unwrap();
    git(root, &["-C", "mysub", "checkout", "-q", &sub2]);

    // Too large (> 10 MB) untracked text file.
    let large = vec![b'a'; (MAX_READ_BYTES as usize) + 128];
    std::fs::write(root.join("huge.txt"), &large).unwrap();

    let files = changes_files(&core, root, &ChangesTarget::Uncommitted).unwrap();
    let by_path: std::collections::HashMap<&str, &ChangedFile> =
        files.files.iter().map(|f| (f.path.as_str(), f)).collect();

    assert!(!by_path.contains_key("nested_repo"));
    assert!(!by_path.contains_key("nested_repo/"));

    assert_eq!(by_path["a.txt"].status, FileStatus::Modified);
    assert_eq!(by_path["added.txt"].status, FileStatus::Added);
    assert_eq!(by_path["to_delete.txt"].status, FileStatus::Deleted);
    assert_eq!(by_path["to_delete.txt"].new_blob_id, None);
    assert_eq!(
        by_path["to_delete.txt"].counts,
        DiffCounts::Lines {
            added: 0,
            removed: 2
        }
    );
    assert_eq!(
        by_path["src/deep/renamed.txt"].status,
        FileStatus::Renamed {
            from: "to_rename.txt".into()
        }
    );
    assert_eq!(by_path["untracked.txt"].status, FileStatus::Untracked);
    assert_eq!(
        by_path["untracked.txt"].new_blob_id.as_deref(),
        Some(hash_object(root, &root.join("untracked.txt")).as_str())
    );
    assert_eq!(
        by_path["untracked.txt"].counts,
        DiffCounts::Lines {
            added: 3,
            removed: 0
        }
    );
    assert_eq!(by_path["bin_untracked.dat"].counts, DiffCounts::Binary);
    assert_eq!(by_path["bin.dat"].counts, DiffCounts::Binary);
    assert_eq!(by_path["conflict.txt"].status, FileStatus::Unmerged);

    // Check diffs for each kind:
    let bin_diff = changes_diff(&core, root, &ChangesTarget::Uncommitted, "bin.dat").unwrap();
    assert_eq!(
        bin_diff.body,
        DiffBody::Binary {
            old_size: Some(4),
            new_size: Some(5)
        }
    );

    let huge_diff = changes_diff(&core, root, &ChangesTarget::Uncommitted, "huge.txt").unwrap();
    assert_eq!(
        huge_diff.body,
        DiffBody::TooLarge {
            old_size: None,
            new_size: Some(MAX_READ_BYTES + 128)
        }
    );

    let crlf_diff = changes_diff(&core, root, &ChangesTarget::Uncommitted, "lf_crlf.txt").unwrap();
    let DiffBody::Text { old, new, hunks } = crlf_diff.body else {
        panic!("expected Text for crlf");
    };
    assert_eq!(old.0, b"one\ntwo\n");
    assert_eq!(new.0, b"one\r\ntwo\r\n");
    assert_eq!(hunks.len(), 1);

    let mode_only = changes_diff(&core, root, &ChangesTarget::Uncommitted, "mode_only.sh").unwrap();
    assert_eq!(
        mode_only.body,
        DiffBody::ModeOnly {
            old_mode: "100644".into(),
            new_mode: "100755".into()
        }
    );

    let mode_edit =
        changes_diff(&core, root, &ChangesTarget::Uncommitted, "mode_and_edit.sh").unwrap();
    assert_eq!(
        mode_edit.mode_change,
        Some(ModeChange {
            old: "100644".into(),
            new: "100755".into()
        })
    );
    assert!(matches!(mode_edit.body, DiffBody::Text { .. }));

    let conflict_diff =
        changes_diff(&core, root, &ChangesTarget::Uncommitted, "conflict.txt").unwrap();
    assert!(conflict_diff.conflicted);
    let DiffBody::Text { old, new, hunks } = conflict_diff.body else {
        panic!("expected Text for conflict");
    };
    assert_eq!(old.0, b"ours\n");
    assert!(String::from_utf8_lossy(&new.0).contains("<<<<<<<"));
    assert!(!hunks.is_empty());

    let symlink_diff = changes_diff(&core, root, &ChangesTarget::Uncommitted, "link").unwrap();
    let DiffBody::Text { old, new, .. } = symlink_diff.body else {
        panic!("expected Text for symlink");
    };
    assert_eq!(old.0, b"old_target");
    assert_eq!(new.0, b"../outside_target");

    let sub_diff = changes_diff(&core, root, &ChangesTarget::Uncommitted, "mysub").unwrap();
    assert_eq!(
        sub_diff.body,
        DiffBody::Submodule {
            old_commit: Some(sub1),
            new_commit: Some(sub2)
        }
    );

    // Commit diff (ours_commit vs parent, and root commit vs empty tree).
    let ours_files =
        changes_files(&core, root, &ChangesTarget::Commit(ours_commit.clone())).unwrap();
    assert_eq!(ours_files.files.len(), 1);
    assert_eq!(ours_files.files[0].path, "conflict.txt");

    let root_commit_sha = git(root, &["rev-list", "--max-parents=0", "HEAD"])
        .trim()
        .to_owned();
    let root_files =
        changes_files(&core, root, &ChangesTarget::Commit(root_commit_sha.clone())).unwrap();
    assert!(root_files
        .files
        .iter()
        .all(|f| f.status == FileStatus::Added));
    let _ = base_commit;

    // Subfolder root limits paths cleanly and refuses escapes.
    let sub_root = root.join("src");
    std::fs::write(sub_root.join("deep/x.rs"), "fn x() { 1 }\n").unwrap();
    let sub_files = changes_files(&core, &sub_root, &ChangesTarget::Uncommitted).unwrap();
    let sub_paths: Vec<&str> = sub_files.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(sub_paths, vec!["deep/renamed.txt", "deep/x.rs"]);
    assert!(changes_diff(&core, &sub_root, &ChangesTarget::Uncommitted, "../a.txt").is_err());
    let outside = tempfile::tempdir().unwrap();
    symlink(outside.path(), sub_root.join("escape")).unwrap();
    assert!(changes_diff(
        &core,
        &sub_root,
        &ChangesTarget::Uncommitted,
        "escape/secret.txt"
    )
    .is_err());
    assert!(changes_commits(&core, outside.path()).is_err());
    assert!(changes_diff(&core, root, &ChangesTarget::Commit("-bad".into()), "a.txt").is_err());

    // Old side > 10 MB committed then deleted reports TooLarge without reading full old blob.
    let big_commit_dir = tempfile::tempdir().unwrap();
    let bc = big_commit_dir.path();
    git(bc, &["init", "-q", "-b", "main"]);
    git(bc, &["config", "core.autocrlf", "false"]);
    std::fs::write(
        bc.join("giant.txt"),
        vec![b'z'; (MAX_READ_BYTES as usize) + 64],
    )
    .unwrap();
    commit_all(bc, "giant");
    git(bc, &["rm", "-q", "giant.txt"]);
    let giant_diff = changes_diff(&core, bc, &ChangesTarget::Uncommitted, "giant.txt").unwrap();
    assert_eq!(
        giant_diff.body,
        DiffBody::TooLarge {
            old_size: Some(MAX_READ_BYTES + 64),
            new_size: None
        }
    );
}

#[test]
fn viewed_marks_carry_over_detect_edits_and_emit_events() {
    let core = test_core();
    let fixture = repo();
    let root = fixture.path();
    let mut rx_client1 = core.events.subscribe();
    let mut rx_client2 = core.events.subscribe();

    std::fs::write(root.join("a.txt"), "edited v1\n").unwrap();
    let before = changes_files(&core, root, &ChangesTarget::Uncommitted).unwrap();
    assert_eq!(before.totals.viewed, 0);
    let a = before.files.iter().find(|f| f.path == "a.txt").unwrap();
    assert!(!a.viewed);
    assert!(!a.changed_since_viewed);

    // Mark viewed in Uncommitted via a subfolder root to verify shared checkout-relative key.
    changes_set_viewed(
        &core,
        &root.join("src"),
        &ChangesTarget::Uncommitted,
        "../not-allowed",
        None,
        None,
        true,
    )
    .unwrap_err();
    changes_set_viewed(
        &core,
        root,
        &ChangesTarget::Uncommitted,
        "a.txt",
        a.old_blob_id.as_deref(),
        a.new_blob_id.as_deref(),
        true,
    )
    .unwrap();

    // Both subscribers receive `changes/viewed`.
    for rx in [&mut rx_client1, &mut rx_client2] {
        let ev = rx.try_recv().unwrap();
        assert_eq!(ev.name, CHANGES_VIEWED_EVENT);
        let decoded: ChangesViewed = serde_json::from_value(ev.payload).unwrap();
        assert_eq!(decoded.path, "a.txt");
        assert!(decoded.viewed);
        assert_eq!(decoded.old_blob_id, a.old_blob_id);
        assert_eq!(decoded.new_blob_id, a.new_blob_id);
    }

    let marked = changes_files(&core, root, &ChangesTarget::Uncommitted).unwrap();
    assert_eq!(marked.totals.viewed, 1);
    let a_marked = marked.files.iter().find(|f| f.path == "a.txt").unwrap();
    assert!(a_marked.viewed);
    assert!(!a_marked.changed_since_viewed);

    // Edit the file further in Uncommitted: clears `viewed` and marks `changed_since_viewed`.
    std::fs::write(root.join("a.txt"), "edited v2\n").unwrap();
    let changed = changes_files(&core, root, &ChangesTarget::Uncommitted).unwrap();
    assert_eq!(changed.totals.viewed, 0);
    let a_changed = changed.files.iter().find(|f| f.path == "a.txt").unwrap();
    assert!(!a_changed.viewed);
    assert!(a_changed.changed_since_viewed);

    // Mark v2 viewed and commit unchanged: viewed mark carries over to the commit.
    changes_set_viewed(
        &core,
        root,
        &ChangesTarget::Uncommitted,
        "a.txt",
        a_changed.old_blob_id.as_deref(),
        a_changed.new_blob_id.as_deref(),
        true,
    )
    .unwrap();
    let commit_sha = commit_all(root, "commit a v2");
    let committed = changes_files(&core, root, &ChangesTarget::Commit(commit_sha.clone())).unwrap();
    assert_eq!(committed.files.len(), 1);
    assert!(committed.files[0].viewed);
    assert!(!committed.files[0].changed_since_viewed);

    // Clearing the mark removes it and broadcasts `viewed: false`.
    let _ = rx_client2.try_recv();
    changes_set_viewed(
        &core,
        root,
        &ChangesTarget::Commit(commit_sha.clone()),
        "a.txt",
        a_changed.old_blob_id.as_deref(),
        a_changed.new_blob_id.as_deref(),
        false,
    )
    .unwrap();
    let cleared_ev: ChangesViewed =
        serde_json::from_value(rx_client2.try_recv().unwrap().payload).unwrap();
    assert!(!cleared_ev.viewed);

    let after_clear = changes_files(&core, root, &ChangesTarget::Commit(commit_sha)).unwrap();
    assert!(!after_clear.files[0].viewed);
    assert!(!after_clear.files[0].changed_since_viewed);
}

#[test]
fn protocol_dispatch_serves_all_four_changes_requests() {
    use runner_core::protocol::{Request, Response};

    let core = test_core();
    let fixture = repo();
    let root = fixture.path().to_path_buf();
    std::fs::write(root.join("a.txt"), "edited via dispatch\n").unwrap();

    let Response::changes_commits(Ok(commits)) =
        crate::daemon::dispatch(&core, Request::changes_commits { root: root.clone() })
    else {
        panic!("expected changes_commits Ok");
    };
    assert_eq!(commits.branch.as_deref(), Some("main"));

    let Response::changes_files(Ok(files)) = crate::daemon::dispatch(
        &core,
        Request::changes_files {
            root: root.clone(),
            target: ChangesTarget::Uncommitted,
        },
    ) else {
        panic!("expected changes_files Ok");
    };
    let a = files.files.iter().find(|f| f.path == "a.txt").unwrap();

    let Response::changes_diff(Ok(diff)) = crate::daemon::dispatch(
        &core,
        Request::changes_diff {
            root: root.clone(),
            target: ChangesTarget::Uncommitted,
            path: "a.txt".into(),
        },
    ) else {
        panic!("expected changes_diff Ok");
    };
    assert!(matches!(diff.body, DiffBody::Text { .. }));

    let Response::changes_set_viewed(Ok(())) = crate::daemon::dispatch(
        &core,
        Request::changes_set_viewed {
            root: root.clone(),
            target: ChangesTarget::Uncommitted,
            path: "a.txt".into(),
            old_blob_id: a.old_blob_id.clone(),
            new_blob_id: a.new_blob_id.clone(),
            viewed: true,
        },
    ) else {
        panic!("expected changes_set_viewed Ok");
    };

    let Response::changes_files(Ok(after)) = crate::daemon::dispatch(
        &core,
        Request::changes_files {
            root,
            target: ChangesTarget::Uncommitted,
        },
    ) else {
        panic!("expected changes_files Ok after mark");
    };
    assert_eq!(after.totals.viewed, 1);
}

#[test]
fn autocrlf_and_clean_filter_carry_viewed_marks_while_preserving_working_bytes() {
    let core = test_core();

    // Case 1: core.autocrlf = true (tracked edits + untracked file)
    let crlf_repo = tempfile::tempdir().unwrap();
    let root = crlf_repo.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "core.autocrlf", "false"]);
    std::fs::write(root.join("crlf.txt"), b"one\ntwo\n").unwrap();
    std::fs::write(root.join("edited.txt"), b"l1\nl2\nl3\nl4\nl5\n").unwrap();
    commit_all(root, "initial lf");

    git(root, &["config", "core.autocrlf", "true"]);
    std::fs::write(root.join("crlf.txt"), b"one\r\ntwo\r\nthree\r\n").unwrap();
    std::fs::write(
        root.join("edited.txt"),
        b"l1\r\nl2\r\nEDITED\r\nl4\r\nl5\r\n",
    )
    .unwrap();
    std::fs::write(root.join("new_crlf.txt"), b"u1\r\nu2\r\n").unwrap();

    let uncommitted = changes_files(&core, root, &ChangesTarget::Uncommitted).unwrap();
    let crlf_entry = uncommitted
        .files
        .iter()
        .find(|f| f.path == "crlf.txt")
        .unwrap();
    let edited_entry = uncommitted
        .files
        .iter()
        .find(|f| f.path == "edited.txt")
        .unwrap();
    let new_entry = uncommitted
        .files
        .iter()
        .find(|f| f.path == "new_crlf.txt")
        .unwrap();

    // Counts under core.autocrlf=true must match `git diff HEAD --numstat` (ignoring CR at EOL),
    // not report every unchanged CRLF line as replaced.
    for (path, entry) in [("crlf.txt", crlf_entry), ("edited.txt", edited_entry)] {
        let numstat = git(root, &["diff", "HEAD", "--numstat", "--", path]);
        let mut cols = numstat.split_whitespace();
        let expected_added: u64 = cols.next().unwrap().parse().unwrap();
        let expected_removed: u64 = cols.next().unwrap().parse().unwrap();
        assert_eq!(
            entry.counts,
            DiffCounts::Lines {
                added: expected_added,
                removed: expected_removed,
            },
            "counts for {path} diverged from git diff HEAD --numstat"
        );
    }
    assert_eq!(
        edited_entry.counts,
        DiffCounts::Lines {
            added: 1,
            removed: 1,
        }
    );

    // Working bytes sent to the app must still keep CRLF intact, and the hunk covers only the edit.
    let diff = changes_diff(&core, root, &ChangesTarget::Uncommitted, "crlf.txt").unwrap();
    assert_diff_consistent_with_eol(Some(crlf_entry), &diff, true);
    let DiffBody::Text { new, .. } = diff.body else {
        panic!("expected Text for crlf.txt");
    };
    assert_eq!(new.0, b"one\r\ntwo\r\nthree\r\n");

    let edited_diff = changes_diff(&core, root, &ChangesTarget::Uncommitted, "edited.txt").unwrap();
    assert_diff_consistent_with_eol(Some(edited_entry), &edited_diff, true);
    let DiffBody::Text {
        new: edited_new,
        hunks: edited_hunks,
        ..
    } = edited_diff.body
    else {
        panic!("expected Text for edited.txt");
    };
    assert_eq!(edited_new.0, b"l1\r\nl2\r\nEDITED\r\nl4\r\nl5\r\n");
    assert_eq!(edited_hunks.len(), 1);
    assert_eq!(
        edited_hunks[0].ops,
        vec![
            DiffOp::Context { count: 2 },
            DiffOp::Removed { count: 1 },
            DiffOp::Added { count: 1 },
            DiffOp::Context { count: 2 },
        ]
    );

    for entry in [crlf_entry, edited_entry, new_entry] {
        changes_set_viewed(
            &core,
            root,
            &ChangesTarget::Uncommitted,
            &entry.path,
            entry.old_blob_id.as_deref(),
            entry.new_blob_id.as_deref(),
            true,
        )
        .unwrap();
    }

    let commit_sha = commit_all(root, "commit crlf files");
    let committed = changes_files(&core, root, &ChangesTarget::Commit(commit_sha)).unwrap();
    assert_eq!(committed.files.len(), 3);
    assert!(committed.files.iter().all(|f| f.viewed));

    // Case 2: .gitattributes clean filter normalizes blobs at commit time.
    let filter_repo = tempfile::tempdir().unwrap();
    let froot = filter_repo.path();
    git(froot, &["init", "-q", "-b", "main"]);
    git(froot, &["config", "filter.mask.clean", "tr X Y"]);
    std::fs::write(froot.join(".gitattributes"), "*.filtered filter=mask\n").unwrap();
    std::fs::write(froot.join("secret.filtered"), b"keep\n").unwrap();
    commit_all(froot, "base");

    std::fs::write(froot.join("secret.filtered"), b"keep\nX\n").unwrap();
    let before_commit = changes_files(&core, froot, &ChangesTarget::Uncommitted).unwrap();
    let filtered_file = before_commit
        .files
        .iter()
        .find(|f| f.path == "secret.filtered")
        .unwrap();
    let filtered_diff =
        changes_diff(&core, froot, &ChangesTarget::Uncommitted, "secret.filtered").unwrap();
    assert_diff_consistent(Some(filtered_file), &filtered_diff);
    changes_set_viewed(
        &core,
        froot,
        &ChangesTarget::Uncommitted,
        "secret.filtered",
        filtered_file.old_blob_id.as_deref(),
        filtered_file.new_blob_id.as_deref(),
        true,
    )
    .unwrap();
    let sha = commit_all(froot, "commit cleaned blob");
    let after_commit = changes_files(&core, froot, &ChangesTarget::Commit(sha)).unwrap();
    assert_eq!(after_commit.files.len(), 1);
    assert!(after_commit.files[0].viewed);
}

#[test]
fn literal_pathspecs_textconv_disabled_and_diff_relative_ignored() {
    let core = test_core();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "core.autocrlf", "false"]);
    git(root, &["config", "diff.relative", "true"]);
    git(root, &["config", "diff.masktail.textconv", "sed -n 2p"]);

    std::fs::create_dir_all(root.join("src/deep")).unwrap();
    std::fs::write(root.join(".gitattributes"), "*.conv diff=masktail\n").unwrap();
    std::fs::write(root.join("a.txt"), "a1\na2\n").unwrap();
    std::fs::write(root.join("[a].txt"), "bracket-old\n").unwrap();
    #[cfg(unix)]
    std::fs::write(root.join(":(exclude)a.txt"), "magic-old\n").unwrap();
    std::fs::write(root.join("note.conv"), "header-v1\nbody\n").unwrap();
    std::fs::write(root.join("src/deep/rel.txt"), "rel-old\n").unwrap();
    commit_all(root, "initial");

    std::fs::write(root.join("a.txt"), "A1\nA2\nA3\n").unwrap();
    std::fs::write(root.join("[a].txt"), "bracket-new\n").unwrap();
    #[cfg(unix)]
    std::fs::write(root.join(":(exclude)a.txt"), "magic-new\n").unwrap();
    std::fs::write(root.join("note.conv"), "header-v2\nbody\n").unwrap();
    std::fs::write(root.join("src/deep/rel.txt"), "rel-new\n").unwrap();

    // Subfolder works even when `diff.relative = true` is configured.
    let sub_root = root.join("src");
    let sub_uncommitted = changes_files(&core, &sub_root, &ChangesTarget::Uncommitted).unwrap();
    assert_eq!(sub_uncommitted.files.len(), 1);
    assert_eq!(sub_uncommitted.files[0].path, "deep/rel.txt");
    let sub_diff = changes_diff(
        &core,
        &sub_root,
        &ChangesTarget::Uncommitted,
        "deep/rel.txt",
    )
    .unwrap();
    let DiffBody::Text { old, new, hunks } = sub_diff.body else {
        panic!("expected Text for src/deep/rel.txt");
    };
    assert_eq!(old.0, b"rel-old\n");
    assert_eq!(new.0, b"rel-new\n");
    assert_eq!(hunks.len(), 1);

    // Literal pathspec prevents `[a].txt` from matching `a.txt`.
    let bracket_diff = changes_diff(&core, root, &ChangesTarget::Uncommitted, "[a].txt").unwrap();
    let DiffBody::Text { old, new, hunks } = bracket_diff.body else {
        panic!("expected Text for [a].txt");
    };
    assert_eq!(old.0, b"bracket-old\n");
    assert_eq!(new.0, b"bracket-new\n");
    assert_eq!(hunks.len(), 1);

    #[cfg(unix)]
    {
        let magic_diff =
            changes_diff(&core, root, &ChangesTarget::Uncommitted, ":(exclude)a.txt").unwrap();
        let DiffBody::Text { old, new, hunks } = magic_diff.body else {
            panic!("expected Text for :(exclude)a.txt");
        };
        assert_eq!(old.0, b"magic-old\n");
        assert_eq!(new.0, b"magic-new\n");
        assert_eq!(hunks.len(), 1);
    }

    // `--no-textconv` preserves actual diff counts and hunks even when textconv matches identical lines.
    let uncommitted = changes_files(&core, root, &ChangesTarget::Uncommitted).unwrap();
    let conv_file = uncommitted
        .files
        .iter()
        .find(|f| f.path == "note.conv")
        .unwrap();
    assert_eq!(
        conv_file.counts,
        DiffCounts::Lines {
            added: 1,
            removed: 1
        }
    );
    let conv_diff = changes_diff(&core, root, &ChangesTarget::Uncommitted, "note.conv").unwrap();
    let DiffBody::Text { old, new, hunks } = conv_diff.body else {
        panic!("expected Text for note.conv");
    };
    assert_eq!(old.0, b"header-v1\nbody\n");
    assert_eq!(new.0, b"header-v2\nbody\n");
    assert_eq!(hunks.len(), 1);

    // Same guarantees hold for Commit targets.
    let commit_sha = commit_all(root, "committed edits");
    let bracket_commit = changes_diff(
        &core,
        root,
        &ChangesTarget::Commit(commit_sha.clone()),
        "[a].txt",
    )
    .unwrap();
    let DiffBody::Text { old, new, .. } = bracket_commit.body else {
        panic!("expected Text for committed [a].txt");
    };
    assert_eq!(old.0, b"bracket-old\n");
    assert_eq!(new.0, b"bracket-new\n");

    let sub_commit = changes_files(&core, &sub_root, &ChangesTarget::Commit(commit_sha)).unwrap();
    assert_eq!(sub_commit.files.len(), 1);
    assert_eq!(sub_commit.files[0].path, "deep/rel.txt");
}

#[test]
fn staged_deletion_recreated_on_disk_reconciles_against_head() {
    let core = test_core();
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "core.autocrlf", "false"]);
    std::fs::write(root.join("gone.txt"), b"old1\nold2\n").unwrap();
    std::fs::write(root.join("cached.txt"), b"same\n").unwrap();
    commit_all(root, "initial");

    // Case 1: `git rm` then recreate modified file on disk.
    git(root, &["rm", "-q", "gone.txt"]);
    std::fs::write(root.join("gone.txt"), b"new1\nnew2\nnew3\n").unwrap();
    // Case 2: `git rm --cached` leaving identical bytes on disk matches HEAD (0 changes).
    git(root, &["rm", "--cached", "-q", "cached.txt"]);

    let files = changes_files(&core, root, &ChangesTarget::Uncommitted).unwrap();
    let paths: Vec<&str> = files.files.iter().map(|f| f.path.as_str()).collect();
    assert_eq!(paths, vec!["gone.txt"]);
    let gone = &files.files[0];
    assert_eq!(gone.status, FileStatus::Modified);
    assert!(gone.old_blob_id.is_some());
    assert!(gone.new_blob_id.is_some());
    assert_ne!(gone.old_blob_id, gone.new_blob_id);
    assert_eq!(
        gone.counts,
        DiffCounts::Lines {
            added: 3,
            removed: 2
        }
    );

    let diff = changes_diff(&core, root, &ChangesTarget::Uncommitted, "gone.txt").unwrap();
    assert_diff_consistent(Some(gone), &diff);
    let DiffBody::Text { old, new, hunks } = diff.body else {
        panic!("expected Text for recreated gone.txt");
    };
    assert_eq!(old.0, b"old1\nold2\n");
    assert_eq!(new.0, b"new1\nnew2\nnew3\n");
    assert_eq!(hunks.len(), 1);
    assert_eq!(hunks[0].old_lines, 2);
    assert_eq!(hunks[0].new_lines, 3);

    // Modifying the `--cached`-removed file on disk surfaces it as Modified against HEAD.
    std::fs::write(root.join("cached.txt"), b"same\nextra\n").unwrap();
    let files2 = changes_files(&core, root, &ChangesTarget::Uncommitted).unwrap();
    let cached = files2
        .files
        .iter()
        .find(|f| f.path == "cached.txt")
        .unwrap();
    assert_eq!(cached.status, FileStatus::Modified);
    assert_eq!(
        cached.counts,
        DiffCounts::Lines {
            added: 1,
            removed: 0
        }
    );
    let cached_diff = changes_diff(&core, root, &ChangesTarget::Uncommitted, "cached.txt").unwrap();
    assert_diff_consistent(Some(cached), &cached_diff);
    let DiffBody::Text { old, new, hunks } = cached_diff.body else {
        panic!("expected Text for cached.txt");
    };
    assert_eq!(old.0, b"same\n");
    assert_eq!(new.0, b"same\nextra\n");
    assert_eq!(hunks.len(), 1);
}

fn split_diff_lines(bytes: &[u8]) -> Vec<&[u8]> {
    if bytes.is_empty() {
        return Vec::new();
    }
    let mut lines = Vec::new();
    let mut start = 0;
    for (idx, byte) in bytes.iter().enumerate() {
        if *byte == b'\n' {
            lines.push(&bytes[start..=idx]);
            start = idx + 1;
        }
    }
    if start < bytes.len() {
        lines.push(&bytes[start..]);
    }
    lines
}

fn hunk_span_bounds(
    start: u32,
    count: u32,
    total_lines: usize,
    prev_end: usize,
    side: &str,
) -> (usize, usize) {
    if count == 0 {
        let anchor = start as usize;
        assert!(
            anchor >= prev_end && anchor <= total_lines,
            "{side} zero-count hunk anchor {anchor} out of bounds [{prev_end}..={total_lines}]"
        );
        return (anchor, anchor);
    }
    assert!(
        start >= 1,
        "{side} non-empty hunk must have 1-based start, got {start}"
    );
    let begin = start as usize - 1;
    let end = begin + count as usize;
    assert!(
        begin >= prev_end && end <= total_lines,
        "{side} hunk range [{begin}..{end}) outside [{prev_end}..={total_lines}]"
    );
    (begin, end)
}

fn split_eol(line: &[u8]) -> (&[u8], bool) {
    if let Some(body) = line.strip_suffix(b"\r\n") {
        (body, true)
    } else if let Some(body) = line.strip_suffix(b"\n") {
        (body, true)
    } else {
        (line, false)
    }
}

fn assert_diff_consistent(file_entry: Option<&ChangedFile>, diff: &ChangesDiff) {
    assert_diff_consistent_with_eol(file_entry, diff, false);
}

fn assert_diff_consistent_with_eol(
    file_entry: Option<&ChangedFile>,
    diff: &ChangesDiff,
    ignore_cr_at_eol: bool,
) {
    if let Some(entry) = file_entry {
        assert_eq!(
            diff.counts, entry.counts,
            "diff counts disagree with changes_files for {}",
            diff.path
        );
    }
    let DiffBody::Text { old, new, hunks } = &diff.body else {
        return;
    };
    let old_lines = split_diff_lines(&old.0);
    let new_lines = split_diff_lines(&new.0);
    let mut prev_old_end = 0usize;
    let mut prev_new_end = 0usize;
    let mut total_removed = 0u64;
    let mut total_added = 0u64;

    for hunk in hunks {
        let (mut old_cursor, old_end) = hunk_span_bounds(
            hunk.old_start,
            hunk.old_lines,
            old_lines.len(),
            prev_old_end,
            "old",
        );
        let (mut new_cursor, new_end) = hunk_span_bounds(
            hunk.new_start,
            hunk.new_lines,
            new_lines.len(),
            prev_new_end,
            "new",
        );
        let mut hunk_old_seen = 0u32;
        let mut hunk_new_seen = 0u32;

        for op in &hunk.ops {
            match *op {
                DiffOp::Context { count } => {
                    let c = count as usize;
                    assert!(
                        old_cursor + c <= old_end && new_cursor + c <= new_end,
                        "context op ({count}) exceeds hunk bounds for {} ({})",
                        diff.path,
                        hunk.header
                    );
                    for offset in 0..c {
                        let old_line = old_lines[old_cursor + offset];
                        let new_line = new_lines[new_cursor + offset];
                        let matches = if ignore_cr_at_eol {
                            split_eol(old_line) == split_eol(new_line)
                        } else {
                            old_line == new_line
                        };
                        assert!(
                            matches,
                            "context line mismatch in {} hunk {}: old={:?} new={:?}",
                            diff.path,
                            hunk.header,
                            String::from_utf8_lossy(old_line),
                            String::from_utf8_lossy(new_line)
                        );
                    }
                    old_cursor += c;
                    new_cursor += c;
                    hunk_old_seen += count;
                    hunk_new_seen += count;
                }
                DiffOp::Removed { count } => {
                    old_cursor += count as usize;
                    hunk_old_seen += count;
                    total_removed += u64::from(count);
                }
                DiffOp::Added { count } => {
                    new_cursor += count as usize;
                    hunk_new_seen += count;
                    total_added += u64::from(count);
                }
            }
        }

        assert_eq!(
            hunk_old_seen, hunk.old_lines,
            "hunk old count mismatch in {} ({})",
            diff.path, hunk.header
        );
        assert_eq!(
            hunk_new_seen, hunk.new_lines,
            "hunk new count mismatch in {} ({})",
            diff.path, hunk.header
        );
        assert_eq!(old_cursor, old_end);
        assert_eq!(new_cursor, new_end);
        prev_old_end = old_end;
        prev_new_end = new_end;
    }

    assert_eq!(
        diff.counts,
        DiffCounts::Lines {
            added: total_added,
            removed: total_removed,
        },
        "sum of hunk Added/Removed disagrees with counts for {}",
        diff.path
    );
}

#[test]
fn clean_filter_and_eol_attributes_keep_diff_consistent_across_targets() {
    let core = test_core();

    // Case 1: line-inserting clean filter (`filter.prefix.clean`) across unstaged, staged, and `git rm --cached`.
    let filter_repo = tempfile::tempdir().unwrap();
    let froot = filter_repo.path();
    git(froot, &["init", "-q", "-b", "main"]);
    git(froot, &["config", "core.autocrlf", "false"]);
    git(
        froot,
        &["config", "filter.prefix.clean", "printf 'header\\n'; cat"],
    );
    std::fs::write(froot.join(".gitattributes"), "*.txt filter=prefix\n").unwrap();
    std::fs::write(
        froot.join("context.txt"),
        b"line1\nline2\nline3\nline4\nold\n",
    )
    .unwrap();
    std::fs::write(froot.join("short.txt"), b"old\n").unwrap();
    commit_all(froot, "initial filtered commit");

    std::fs::write(
        froot.join("context.txt"),
        b"header\nline1\nline2\nline3\nline4\nnew\n",
    )
    .unwrap();
    std::fs::write(froot.join("short.txt"), b"new\n").unwrap();

    for stage in ["unstaged", "staged", "cached_rm"] {
        match stage {
            "unstaged" => {}
            "staged" => {
                git(froot, &["add", "context.txt", "short.txt"]);
            }
            "cached_rm" => {
                git(froot, &["rm", "--cached", "-q", "context.txt", "short.txt"]);
            }
            _ => unreachable!(),
        }

        let files = changes_files(&core, froot, &ChangesTarget::Uncommitted).unwrap();
        let ctx_entry = files
            .files
            .iter()
            .find(|f| f.path == "context.txt")
            .unwrap();
        let short_entry = files.files.iter().find(|f| f.path == "short.txt").unwrap();
        assert_eq!(ctx_entry.status, FileStatus::Modified);
        assert_eq!(short_entry.status, FileStatus::Modified);
        assert_eq!(
            ctx_entry.counts,
            DiffCounts::Lines {
                added: 1,
                removed: 1,
            }
        );
        assert_eq!(
            short_entry.counts,
            DiffCounts::Lines {
                added: 1,
                removed: 2,
            }
        );

        let ctx_diff =
            changes_diff(&core, froot, &ChangesTarget::Uncommitted, "context.txt").unwrap();
        assert_diff_consistent(Some(ctx_entry), &ctx_diff);
        let DiffBody::Text {
            old: ctx_old,
            new: ctx_new,
            hunks: ctx_hunks,
        } = ctx_diff.body
        else {
            panic!("expected Text for context.txt ({stage})");
        };
        assert_eq!(ctx_old.0, b"header\nline1\nline2\nline3\nline4\nold\n");
        assert_eq!(ctx_new.0, b"header\nline1\nline2\nline3\nline4\nnew\n");
        assert_eq!(ctx_hunks.len(), 1);
        assert_eq!(
            ctx_hunks[0].ops,
            vec![
                DiffOp::Context { count: 3 },
                DiffOp::Removed { count: 1 },
                DiffOp::Added { count: 1 },
            ]
        );

        let short_diff =
            changes_diff(&core, froot, &ChangesTarget::Uncommitted, "short.txt").unwrap();
        assert_diff_consistent(Some(short_entry), &short_diff);
        let DiffBody::Text {
            old: short_old,
            new: short_new,
            hunks: short_hunks,
        } = short_diff.body
        else {
            panic!("expected Text for short.txt ({stage})");
        };
        assert_eq!(short_old.0, b"header\nold\n");
        assert_eq!(short_new.0, b"new\n");
        assert_eq!(short_hunks.len(), 1);
        assert_eq!(
            short_hunks[0].ops,
            vec![DiffOp::Removed { count: 2 }, DiffOp::Added { count: 1 }]
        );
    }

    // Viewed mark still uses Git's normalized blob id and carries over once committed.
    let files_before_commit = changes_files(&core, froot, &ChangesTarget::Uncommitted).unwrap();
    let short_entry = files_before_commit
        .files
        .iter()
        .find(|f| f.path == "short.txt")
        .unwrap();
    changes_set_viewed(
        &core,
        froot,
        &ChangesTarget::Uncommitted,
        "short.txt",
        short_entry.old_blob_id.as_deref(),
        short_entry.new_blob_id.as_deref(),
        true,
    )
    .unwrap();
    let sha = commit_all(froot, "commit cleaned edits");
    let committed = changes_files(&core, froot, &ChangesTarget::Commit(sha)).unwrap();
    let committed_short = committed
        .files
        .iter()
        .find(|f| f.path == "short.txt")
        .unwrap();
    assert!(committed_short.viewed);

    // Case 2: `.gitattributes` `*.txt text eol=crlf` for both normal edits and `git rm` + recreate.
    let eol_repo = tempfile::tempdir().unwrap();
    let eroot = eol_repo.path();
    git(eroot, &["init", "-q", "-b", "main"]);
    git(eroot, &["config", "core.autocrlf", "false"]);
    std::fs::write(eroot.join(".gitattributes"), "*.txt text eol=crlf\n").unwrap();
    std::fs::write(eroot.join("eol_edit.txt"), b"one\ntwo\n").unwrap();
    std::fs::write(eroot.join("eol_rm.txt"), b"one\ntwo\n").unwrap();
    commit_all(eroot, "initial eol commit");

    std::fs::write(eroot.join("eol_edit.txt"), b"one\r\ntwo\r\nthree\r\n").unwrap();
    git(eroot, &["rm", "-q", "eol_rm.txt"]);
    std::fs::write(eroot.join("eol_rm.txt"), b"one\r\ntwo\r\nthree\r\n").unwrap();

    let eol_files = changes_files(&core, eroot, &ChangesTarget::Uncommitted).unwrap();
    for path in ["eol_edit.txt", "eol_rm.txt"] {
        let entry = eol_files.files.iter().find(|f| f.path == path).unwrap();
        assert_eq!(entry.status, FileStatus::Modified);
        assert_eq!(
            entry.counts,
            DiffCounts::Lines {
                added: 1,
                removed: 0,
            }
        );
        let diff = changes_diff(&core, eroot, &ChangesTarget::Uncommitted, path).unwrap();
        assert_diff_consistent_with_eol(Some(entry), &diff, true);
        let DiffBody::Text { old, new, hunks } = diff.body else {
            panic!("expected Text for {path}");
        };
        assert_eq!(old.0, b"one\ntwo\n");
        assert_eq!(new.0, b"one\r\ntwo\r\nthree\r\n");
        assert_eq!(hunks.len(), 1);
        assert_eq!(
            hunks[0].ops,
            vec![DiffOp::Context { count: 2 }, DiffOp::Added { count: 1 }]
        );

        changes_set_viewed(
            &core,
            eroot,
            &ChangesTarget::Uncommitted,
            path,
            entry.old_blob_id.as_deref(),
            entry.new_blob_id.as_deref(),
            true,
        )
        .unwrap();
    }

    let eol_sha = commit_all(eroot, "commit eol edits");
    let eol_committed = changes_files(&core, eroot, &ChangesTarget::Commit(eol_sha)).unwrap();
    assert_eq!(eol_committed.files.len(), 2);
    assert!(eol_committed.files.iter().all(|f| f.viewed));
}

#[test]
fn single_file_hash_fallback_handles_raw_and_special_filenames() {
    let core = test_core();
    let git_bin = core.files.git().unwrap();

    // Part 1 (all platforms, including Windows): exercise both hash_single_working_path branches directly.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    git(root, &["init", "-q", "-b", "main"]);
    git(root, &["config", "core.autocrlf", "false"]);
    git(
        root,
        &["config", "filter.prefix.clean", "printf 'header\\n'; cat"],
    );
    std::fs::write(root.join(".gitattributes"), "*.txt filter=prefix\n").unwrap();
    std::fs::write(root.join("plain.txt"), b"body\n").unwrap();

    let normalized = hash_single_working_path(&git_bin, root, "plain.txt", false)
        .unwrap()
        .expect("normalized hash");
    let raw = hash_single_working_path(&git_bin, root, "plain.txt", true)
        .unwrap()
        .expect("raw hash");
    assert_eq!(normalized, git_blob_hash(b"header\nbody\n"));
    assert_eq!(raw, git_blob_hash(b"body\n"));

    // Part 2 (Unix): real filenames routed through the single-file fallback (`\n`, `"`, `\`).
    #[cfg(unix)]
    {
        let udir = tempfile::tempdir().unwrap();
        let uroot = udir.path();
        git(uroot, &["init", "-q", "-b", "main"]);
        git(uroot, &["config", "core.autocrlf", "false"]);
        git(
            uroot,
            &["config", "filter.prefix.clean", "printf 'header\\n'; cat"],
        );
        std::fs::write(uroot.join(".gitattributes"), "*.txt filter=prefix\n").unwrap();

        let special_names = ["with\nnewline.txt", "\"quoted\".txt", "back\\slash.txt"];
        for name in special_names {
            std::fs::write(uroot.join(name), b"old\n").unwrap();
        }
        commit_all(uroot, "initial special filenames");

        for name in special_names {
            std::fs::write(uroot.join(name), b"new\n").unwrap();
        }

        let files = changes_files(&core, uroot, &ChangesTarget::Uncommitted).unwrap();
        for name in special_names {
            let entry = files
                .files
                .iter()
                .find(|f| f.path == name)
                .unwrap_or_else(|| panic!("missing special file {name:?}"));
            assert_eq!(entry.status, FileStatus::Modified);
            assert_eq!(
                entry.counts,
                DiffCounts::Lines {
                    added: 1,
                    removed: 2,
                }
            );
            assert_eq!(
                entry.new_blob_id.as_deref(),
                Some(git_blob_hash(b"header\nnew\n").as_str())
            );

            let diff = changes_diff(&core, uroot, &ChangesTarget::Uncommitted, name).unwrap();
            assert_diff_consistent(Some(entry), &diff);
            let DiffBody::Text { old, new, hunks } = diff.body else {
                panic!("expected Text for {name:?}");
            };
            assert_eq!(old.0, b"header\nold\n");
            assert_eq!(new.0, b"new\n");
            assert_eq!(
                hunks[0].ops,
                vec![DiffOp::Removed { count: 2 }, DiffOp::Added { count: 1 }]
            );

            changes_set_viewed(
                &core,
                uroot,
                &ChangesTarget::Uncommitted,
                name,
                entry.old_blob_id.as_deref(),
                entry.new_blob_id.as_deref(),
                true,
            )
            .unwrap();
        }

        let sha = commit_all(uroot, "commit special filenames");
        let committed = changes_files(&core, uroot, &ChangesTarget::Commit(sha)).unwrap();
        for name in special_names {
            let entry = committed.files.iter().find(|f| f.path == name).unwrap();
            assert!(entry.viewed, "viewed mark did not carry over for {name:?}");
        }
    }
}

#[test]
fn in_memory_viewed_marks_cover_set_clear_sticky_uncommitted_carry_over_and_changed_since_viewed() {
    let marks = ViewedMarks::default();
    let repo_a = Path::new("/work/repo-a");
    let repo_b = Path::new("/work/repo-b");

    // Empty store reports all false.
    assert_eq!(
        marks.flags_for(repo_a, [("a.rs", Some("old1"), Some("new1"))], true),
        vec![(false, false)]
    );

    // Setting a mark on Uncommitted matches both Uncommitted and Commit carry-over for identical blobs.
    marks.set(repo_a, "a.rs", Some("old1"), Some("new1"), true);
    assert_eq!(
        marks.flags_for(repo_a, [("a.rs", Some("old1"), Some("new1"))], true),
        vec![(true, false)]
    );
    assert_eq!(
        marks.flags_for(repo_a, [("a.rs", Some("old1"), Some("new1"))], false),
        vec![(true, false)]
    );

    // Checkouts are isolated from each other.
    assert_eq!(
        marks.flags_for(repo_b, [("a.rs", Some("old1"), Some("new1"))], true),
        vec![(false, false)]
    );

    // Sticky `uncommitted` flag: re-marking the same `(path, old, new)` on a commit keeps `uncommitted = true`,
    // so newer working-tree edits on that path still report `changed_since_viewed`.
    marks.set(repo_a, "a.rs", Some("old1"), Some("new1"), false);
    assert_eq!(
        marks.flags_for(repo_a, [("a.rs", Some("old1"), Some("new2"))], true),
        vec![(false, true)]
    );
    // In commit view (`is_uncommitted = false`), different blobs do not show `changed_since_viewed`.
    assert_eq!(
        marks.flags_for(repo_a, [("a.rs", Some("old1"), Some("new2"))], false),
        vec![(false, false)]
    );

    // `None` blob slots unify with `Some("")`.
    marks.set(repo_a, "added.rs", None, Some("b1"), true);
    assert_eq!(
        marks.flags_for(repo_a, [("added.rs", Some(""), Some("b1"))], true),
        vec![(true, false)]
    );
    assert_eq!(
        marks.flags_for(repo_a, [("added.rs", None, Some("b2"))], true),
        vec![(false, true)]
    );

    // Commit-only marks do not flag uncommitted edits as `changed_since_viewed` until set on uncommitted.
    marks.set(repo_a, "commit_only.rs", Some("c0"), Some("c1"), false);
    assert_eq!(
        marks.flags_for(repo_a, [("commit_only.rs", Some("c0"), Some("c2"))], true),
        vec![(false, false)]
    );
    marks.set(repo_a, "commit_only.rs", Some("c0"), Some("c1"), true);
    assert_eq!(
        marks.flags_for(repo_a, [("commit_only.rs", Some("c0"), Some("c2"))], true),
        vec![(false, true)]
    );

    // Clearing removes exact and changed_since_viewed state.
    marks.clear(repo_a, "a.rs", Some("old1"), Some("new1"));
    assert_eq!(
        marks.flags_for(
            repo_a,
            [
                ("a.rs", Some("old1"), Some("new1")),
                ("a.rs", Some("old1"), Some("new2")),
            ],
            true,
        ),
        vec![(false, false), (false, false)]
    );
    marks.clear(repo_a, "added.rs", Some(""), Some("b1"));
    marks.clear(repo_a, "commit_only.rs", Some("c0"), Some("c1"));
    assert!(marks.by_checkout.lock().unwrap().is_empty());
}
