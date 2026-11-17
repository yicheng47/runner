use super::fixture::git;
use super::FilesService;
use crate::db;
use crate::model::SessionStatus;
use crate::ops;
use crate::repo;
use crate::test_support::{insert_test_role, insert_test_slot, test_session_row};
use runner_core::app_paths::{
    normalize_checkout_path, same_checkout_path, same_normalized_checkout_path,
};
use runner_core::protocol::files::WorktreeKind;
use runner_core::protocol::worktree::{
    MissingWorktreeBranchState, WorktreeCheckout, WorktreeNameCheck,
};
use runner_core::protocol::StartMissionInput;
use std::path::Path;
use std::sync::{Arc, Barrier};

fn init_clean_repo() -> tempfile::TempDir {
    let dir = tempfile::Builder::new()
        .prefix("1213-m1-")
        .tempdir()
        .unwrap();
    let path = dir.path();
    git(path, &["init", "-q", "-b", "main"]);
    git(path, &["config", "core.autocrlf", "false"]);
    std::fs::write(path.join("README.md"), "hello\n").unwrap();
    git(path, &["add", "-A"]);
    git(path, &["commit", "-q", "-m", "init"]);
    dir
}

fn head_commit(dir: &Path, rev: &str) -> String {
    git(dir, &["rev-parse", rev]).trim().to_owned()
}

fn short_commit(dir: &Path, rev: &str) -> String {
    git(dir, &["rev-parse", "--short", rev]).trim().to_owned()
}

#[test]
fn worktree_check_and_create_use_origin_head_then_main_head_with_no_fetch() {
    let origin_dir = tempfile::Builder::new()
        .prefix("1213-m1-origin-")
        .tempdir()
        .unwrap();
    git(origin_dir.path(), &["init", "-q", "--bare", "-b", "trunk"]);

    let seed = init_clean_repo();
    git(seed.path(), &["branch", "-M", "trunk"]);
    let trunk_rev = head_commit(seed.path(), "trunk");
    let origin_url = origin_dir.path().to_string_lossy().into_owned();
    git(seed.path(), &["remote", "add", "origin", &origin_url]);
    git(seed.path(), &["push", "-q", "origin", "trunk"]);

    let local_dir = tempfile::Builder::new()
        .prefix("1213-m1-clone-")
        .tempdir()
        .unwrap();
    git(local_dir.path(), &["clone", "-q", &origin_url, "repo"]);
    let repo = local_dir.path().join("repo");
    git(&repo, &["config", "core.autocrlf", "false"]);
    // Move local trunk to a different commit while leaving origin/HEAD on the cloned commit
    // and never fetching from origin.
    git(&repo, &["checkout", "-q", "-b", "local-work"]);
    std::fs::write(repo.join("local.txt"), "local\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "local move"]);
    let local_rev = head_commit(&repo, "HEAD");
    assert_ne!(local_rev, trunk_rev);
    // Point origin's HEAD somewhere else WITHOUT fetching in local repo to verify no fetch happens.
    git(seed.path(), &["checkout", "-q", "--detach"]);
    std::fs::write(seed.path().join("ahead.txt"), "ahead\n").unwrap();
    git(seed.path(), &["add", "-A"]);
    git(seed.path(), &["commit", "-q", "-m", "ahead"]);
    git(
        seed.path(),
        &["push", "-q", "origin", "HEAD:refs/heads/trunk"],
    );

    let files = FilesService::default();
    let default_base = files.worktree_default_base(&repo).unwrap().unwrap();
    assert_eq!(default_base.branch, "origin/trunk");

    let check = files
        .worktree_check(&repo, "feat/from-origin", None)
        .unwrap();
    assert!(matches!(
        check,
        WorktreeNameCheck::NewBranch { ref base, ref path }
            if base == "origin/trunk" && path == &normalize_checkout_path(&repo).join(".worktrees").join("feat-from-origin")
    ));

    let created = files
        .worktree_create(&repo, "feat/from-origin", None)
        .unwrap();
    assert!(created.created);
    assert_eq!(head_commit(&created.path, "HEAD"), trunk_rev);

    // Without origin, fallback is main checkout's HEAD (even when called from a linked worktree).
    let no_remote = init_clean_repo();
    let main_rev = head_commit(no_remote.path(), "HEAD");
    git(no_remote.path(), &["branch", "side", "HEAD"]);
    let side_wt = files
        .worktree_create(no_remote.path(), "side", None)
        .unwrap();
    std::fs::write(side_wt.path.join("side.txt"), "side\n").unwrap();
    git(&side_wt.path, &["add", "-A"]);
    git(&side_wt.path, &["commit", "-q", "-m", "side commit"]);
    let side_rev = head_commit(&side_wt.path, "HEAD");
    assert_ne!(side_rev, main_rev);

    let default_no_origin = files.worktree_default_base(&side_wt.path).unwrap().unwrap();
    assert_eq!(default_no_origin.branch, "main");
    let from_main = files
        .worktree_create(&side_wt.path, "feat/from-main", None)
        .unwrap();
    assert_eq!(head_commit(&from_main.path, "HEAD"), main_rev);

    // Detached main HEAD also resolves cleanly even when triggered from a linked worktree.
    git(no_remote.path(), &["checkout", "-q", "--detach", "HEAD"]);
    let detached_default = files.worktree_default_base(&side_wt.path).unwrap().unwrap();
    assert_eq!(
        detached_default.branch,
        short_commit(no_remote.path(), "HEAD")
    );
    let from_detached = files
        .worktree_create(&side_wt.path, "feat/from-detached", None)
        .unwrap();
    assert_eq!(head_commit(&from_detached.path, "HEAD"), main_rev);
}

#[test]
fn worktree_create_resolution_order_existing_checked_out_and_prunable() {
    let repo = init_clean_repo();
    let files = FilesService::default();

    // 1. Existing checked out branch (`main`): check & create return main checkout, idempotently.
    assert!(matches!(
        files.worktree_check(repo.path(), "main", None).unwrap(),
        WorktreeNameCheck::AlreadyCheckedOut { ref path }
            if same_checkout_path(path, repo.path())
    ));
    let reused_main = files.worktree_create(repo.path(), "main", None).unwrap();
    assert!(!reused_main.created);
    assert!(same_checkout_path(&reused_main.path, repo.path()));

    // 2. Existing local branch not checked out anywhere.
    git(repo.path(), &["branch", "feat/existing", "HEAD"]);
    let expected_commit = short_commit(repo.path(), "feat/existing");
    assert!(matches!(
        files.worktree_check(repo.path(), "feat/existing", None).unwrap(),
        WorktreeNameCheck::ExistingBranch { ref commit, ref path }
            if commit == &expected_commit
                && path == &normalize_checkout_path(repo.path()).join(".worktrees").join("feat-existing")
    ));
    let checked_out = files
        .worktree_create(repo.path(), "feat/existing", None)
        .unwrap();
    assert!(checked_out.created);
    assert_eq!(
        git(&checked_out.path, &["branch", "--show-current"]).trim(),
        "feat/existing"
    );

    // Retry while checked out returns same path without error (`created: false`).
    let retry = files
        .worktree_create(repo.path(), "feat/existing", None)
        .unwrap();
    assert!(!retry.created);
    assert!(same_normalized_checkout_path(
        &retry.path,
        &checked_out.path
    ));

    // 3. Stale prunable registration (folder deleted manually) is cleared before re-adding.
    std::fs::remove_dir_all(&checked_out.path).unwrap();
    assert!(files
        .worktree_list(repo.path())
        .unwrap()
        .worktrees
        .iter()
        .any(|w| w.prunable && w.branch.as_deref() == Some("feat/existing")));
    let recreated = files
        .worktree_create(repo.path(), "feat/existing", None)
        .unwrap();
    assert!(recreated.created);
    assert!(recreated.path.is_dir());
    assert_eq!(
        git(&recreated.path, &["branch", "--show-current"]).trim(),
        "feat/existing"
    );
}

#[test]
fn worktree_check_and_create_reject_invalid_names_and_path_collisions() {
    let repo = init_clean_repo();
    let files = FilesService::default();

    for bad in [
        "", "foo..bar", "-foo", "@{-1}", "HEAD", "foo.lock", "bad name",
    ] {
        let checked = files.worktree_check(repo.path(), bad, None).unwrap();
        assert!(
            matches!(checked, WorktreeNameCheck::Invalid { .. }),
            "expected invalid for {bad:?}, got {checked:?}"
        );
        assert!(
            files.worktree_create(repo.path(), bad, None).is_err(),
            "expected create failure for {bad:?}"
        );
    }

    // Explicit invalid base is rejected in check and create.
    assert!(matches!(
        files
            .worktree_check(repo.path(), "feat/new", Some("missing-ref"))
            .unwrap(),
        WorktreeNameCheck::Invalid { .. }
    ));
    let bad_base_err = files
        .worktree_create(repo.path(), "feat/new", Some("missing-ref"))
        .unwrap_err()
        .to_string();
    assert!(!bad_base_err.is_empty());

    // Unrelated folder already sitting at .worktrees/feat-collision is never overwritten.
    let collision_dir = repo.path().join(".worktrees").join("feat-collision");
    std::fs::create_dir_all(&collision_dir).unwrap();
    std::fs::write(collision_dir.join("keep.txt"), "keep\n").unwrap();
    assert!(matches!(
        files
            .worktree_check(repo.path(), "feat/collision", None)
            .unwrap(),
        WorktreeNameCheck::Invalid { .. }
    ));
    assert!(files
        .worktree_create(repo.path(), "feat/collision", None)
        .is_err());
    assert_eq!(
        std::fs::read_to_string(collision_dir.join("keep.txt")).unwrap(),
        "keep\n"
    );
}

#[test]
fn worktree_create_serializes_concurrent_calls_per_repository() {
    let repo = init_clean_repo();
    let files = Arc::new(FilesService::default());
    let barrier = Arc::new(Barrier::new(2));

    let results = std::thread::scope(|scope| {
        let handles: Vec<_> = (0..2)
            .map(|_| {
                let files = Arc::clone(&files);
                let barrier = Arc::clone(&barrier);
                let path = repo.path().to_path_buf();
                scope.spawn(move || {
                    barrier.wait();
                    files
                        .worktree_create(&path, "feat/concurrent", None)
                        .unwrap()
                })
            })
            .collect();
        handles
            .into_iter()
            .map(|h| h.join().unwrap())
            .collect::<Vec<_>>()
    });

    assert!(same_normalized_checkout_path(
        &results[0].path,
        &results[1].path
    ));
    assert_eq!(
        results.iter().filter(|r| r.created).count(),
        1,
        "exactly one concurrent create should add the worktree"
    );
}

#[test]
fn worktree_create_writes_info_exclude_once_and_respects_existing_ignore() {
    let repo = init_clean_repo();
    let files = FilesService::default();
    let exclude = repo.path().join(".git").join("info").join("exclude");

    let first = files
        .worktree_create(repo.path(), "feat/one", None)
        .unwrap();
    let content = std::fs::read_to_string(&exclude).unwrap();
    assert_eq!(content.matches("/.worktrees/").count(), 1);
    assert!(!repo.path().join(".gitignore").exists());

    // Creating a second worktree from inside the first linked worktree writes to common info/exclude
    // and never duplicates the exclude line.
    let _second = files
        .worktree_create(&first.path, "feat/two", None)
        .unwrap();
    let content_after = std::fs::read_to_string(&exclude).unwrap();
    assert_eq!(content_after.matches("/.worktrees/").count(), 1);

    // Repository whose .gitignore already ignores .worktrees/ leaves info/exclude untouched.
    let ignored_repo = super::fixture::repo();
    let gitignore_before = std::fs::read_to_string(ignored_repo.path().join(".gitignore")).unwrap();
    let ignored_exclude = ignored_repo
        .path()
        .join(".git")
        .join("info")
        .join("exclude");
    let exclude_before = std::fs::read_to_string(&ignored_exclude).unwrap();
    let _wt = files
        .worktree_create(ignored_repo.path(), "feat/ignored", None)
        .unwrap();
    assert_eq!(
        std::fs::read_to_string(&ignored_exclude).unwrap(),
        exclude_before
    );
    assert_eq!(
        std::fs::read_to_string(ignored_repo.path().join(".gitignore")).unwrap(),
        gitignore_before
    );
}

#[test]
fn worktree_list_and_live_branch_tracking_after_git_switch() {
    let repo = init_clean_repo();
    let files = FilesService::default();
    let wt = files
        .worktree_create(repo.path(), "feat/live", None)
        .unwrap();

    // Also add an external linked worktree outside .worktrees/.
    let external_parent = tempfile::Builder::new()
        .prefix("1213-m1-ext-")
        .tempdir()
        .unwrap();
    let external_path = external_parent.path().join("outside-wt");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feat/outside",
            &external_path.to_string_lossy(),
            "main",
        ],
    );

    let listed = files.worktree_list(repo.path()).unwrap();
    let main_item = listed
        .worktrees
        .iter()
        .find(|item| item.kind == WorktreeKind::Main)
        .unwrap();
    assert!(!main_item.managed);
    let managed_item = listed
        .worktrees
        .iter()
        .find(|item| same_normalized_checkout_path(&item.path, &wt.path))
        .unwrap();
    assert_eq!(managed_item.kind, WorktreeKind::Linked);
    assert!(managed_item.managed);
    assert_eq!(managed_item.branch.as_deref(), Some("feat/live"));
    let outside_item = listed
        .worktrees
        .iter()
        .find(|item| same_checkout_path(&item.path, &external_path))
        .unwrap();
    assert_eq!(outside_item.kind, WorktreeKind::Linked);
    assert!(!outside_item.managed);

    // Live `git switch` inside the worktree updates both checkout_state and worktree_list immediately.
    git(&wt.path, &["switch", "-q", "-c", "live/switched"]);
    let state = files.worktree_checkout_state(&wt.path, None).unwrap();
    assert!(matches!(
        state,
        WorktreeCheckout::Git {
            kind: WorktreeKind::Linked,
            managed: true,
            branch: Some(ref branch),
            missing: false,
            ..
        } if branch == "live/switched"
    ));

    git(&wt.path, &["switch", "-q", "--detach", "HEAD"]);
    let detached = files.worktree_checkout_state(&wt.path, None).unwrap();
    assert!(matches!(
        detached,
        WorktreeCheckout::Git {
            branch: None,
            missing: false,
            commit: Some(_),
            ..
        }
    ));
}

#[test]
fn worktree_checkout_state_and_restore_cover_removed_prunable_ambiguous_and_deleted_branches() {
    let repo = init_clean_repo();
    let files = FilesService::default();

    // 1. Deleted manually (`rm -rf`) -> prunable entry resolves exact branch even if another branch shares flattened name.
    let wt = files
        .worktree_create(repo.path(), "feat/restore", None)
        .unwrap();
    let sub = wt.path.join("nested");
    std::fs::create_dir_all(&sub).unwrap();
    git(repo.path(), &["branch", "feat-restore", "main"]);
    std::fs::remove_dir_all(&wt.path).unwrap();

    let state = files.worktree_checkout_state(&sub, None).unwrap();
    assert!(matches!(
        state,
        WorktreeCheckout::Git {
            kind: WorktreeKind::Linked,
            managed: true,
            branch: Some(ref b),
            missing: true,
            missing_branch: Some(MissingWorktreeBranchState::Surviving),
            ..
        } if b == "feat/restore"
    ));
    let restored = files.restore_if_missing(&wt.path, None).unwrap();
    assert!(same_normalized_checkout_path(&restored.unwrap(), &wt.path));
    assert_eq!(
        git(&wt.path, &["branch", "--show-current"]).trim(),
        "feat/restore"
    );

    // 2. Removed with `git worktree remove` when branch name match is unique.
    git(repo.path(), &["branch", "-D", "feat-restore"]);
    let wt_str = wt.path.to_string_lossy().into_owned();
    git(repo.path(), &["worktree", "remove", &wt_str]);
    assert!(!wt.path.exists());

    let removed_state = files.worktree_checkout_state(&wt.path, None).unwrap();
    assert!(matches!(
        removed_state,
        WorktreeCheckout::Git {
            kind: WorktreeKind::Linked,
            managed: true,
            branch: Some(ref b),
            missing: true,
            missing_branch: Some(MissingWorktreeBranchState::Surviving),
            ..
        } if b == "feat/restore"
    ));
    let recreated = files.restore_if_missing(&wt.path, None).unwrap();
    assert!(recreated.is_some());
    assert!(wt.path.is_dir());

    // 3. Ambiguous flattened branches after `git worktree remove` report fallback error.
    git(repo.path(), &["branch", "feat-restore", "main"]);
    git(repo.path(), &["worktree", "remove", &wt_str]);
    let ambiguous_state = files.worktree_checkout_state(&wt.path, None).unwrap();
    assert!(matches!(
        ambiguous_state,
        WorktreeCheckout::Git {
            branch: None,
            missing: true,
            missing_branch: Some(MissingWorktreeBranchState::Ambiguous),
            ref candidate_branches,
            ..
        } if candidate_branches.len() == 2
    ));
    let ambig_err = files
        .restore_if_missing(&wt.path, None)
        .unwrap_err()
        .to_string();
    assert!(
        ambig_err.contains("restore in main checkout"),
        "unexpected error: {ambig_err}"
    );

    // 4. Deleted branch after `git worktree remove` reports Gone fallback error.
    git(repo.path(), &["branch", "-D", "feat/restore"]);
    git(repo.path(), &["branch", "-D", "feat-restore"]);
    let gone_state = files.worktree_checkout_state(&wt.path, None).unwrap();
    assert!(matches!(
        gone_state,
        WorktreeCheckout::Git {
            branch: None,
            missing: true,
            missing_branch: Some(MissingWorktreeBranchState::Gone),
            ..
        }
    ));
    let gone_err = files
        .restore_if_missing(&wt.path, None)
        .unwrap_err()
        .to_string();
    assert!(
        gone_err.contains("restore in main checkout"),
        "unexpected error: {gone_err}"
    );
}

#[test]
fn worktree_remove_enforces_location_unarchived_sessions_and_missions_and_git_refusals() {
    let repo = init_clean_repo();
    let files = FilesService::default();
    let pool = db::open_in_memory().unwrap();
    let app_data = tempfile::Builder::new()
        .prefix("1213-m1-app-")
        .tempdir()
        .unwrap();

    // Main checkout & external worktrees are refused.
    assert!(files
        .worktree_remove_checked(&pool.get().unwrap(), repo.path(), "main")
        .is_err());

    let ext_dir = tempfile::Builder::new()
        .prefix("1213-m1-ext-")
        .tempdir()
        .unwrap();
    let ext_wt = ext_dir.path().join("outside");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feat/outside",
            &ext_wt.to_string_lossy(),
            "main",
        ],
    );
    assert!(files
        .worktree_remove_checked(&pool.get().unwrap(), repo.path(), "feat/outside")
        .is_err());

    // Manually created linked worktree directly under `.worktrees/` counts as Runner's.
    let manual_wt = normalize_checkout_path(repo.path())
        .join(".worktrees")
        .join("manual-hand");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feat/manual",
            &manual_wt.to_string_lossy(),
            "main",
        ],
    );

    // Dirty tree is refused cleanly with git's reason (never --force) and keeps folder.
    std::fs::write(manual_wt.join("dirty.txt"), "untracked\n").unwrap();
    let dirty_err = files
        .worktree_remove_checked(&pool.get().unwrap(), repo.path(), "feat/manual")
        .unwrap_err()
        .to_string();
    assert!(!dirty_err.is_empty());
    assert!(manual_wt.is_dir());
    std::fs::remove_file(manual_wt.join("dirty.txt")).unwrap();

    // Unarchived direct session blocks removal; archiving allows it.
    let wt_str = manual_wt.to_string_lossy().into_owned();
    {
        let conn = pool.get().unwrap();
        let mut s = test_session_row("sess-1", SessionStatus::Stopped);
        s.cwd = Some(wt_str.clone());
        repo::session::insert(&conn, &s).unwrap();
        assert!(files
            .worktree_remove_checked(&conn, repo.path(), &wt_str)
            .is_err());
        assert_eq!(
            repo::session::archive(&conn, "sess-1", chrono::Utc::now()).unwrap(),
            1
        );
    }

    // Unarchived mission blocks removal EVEN if we stop/archive sessions check excludes missions,
    // and succeeds once mission itself is archived (`missions.archived_at` set while `sessions.archived_at` stays NULL).
    {
        let mut conn = pool.get().unwrap();
        let crew = ops::crew::create(
            &conn,
            ops::crew::CreateCrewInput {
                name: "Crew".into(),
                ..Default::default()
            },
        )
        .unwrap()
        .id;
        insert_test_role(&conn, "r1", "worker", "codex", "codex");
        insert_test_slot(&conn, "sl1", &crew, "r1", "worker", 0, true);
        let started = ops::mission::start_with_files(
            &mut conn,
            app_data.path(),
            Some(&files),
            StartMissionInput {
                crew_id: Some(crew),
                project_id: None,
                title: "Active worktree mission".into(),
                goal_override: None,
                cwd: Some(wt_str.clone()),
                worktree: None,
                started_by_session_id: None,
                no_report: false,
            },
        )
        .unwrap();
        let mut slot_session = test_session_row("sess-mission", SessionStatus::Stopped);
        slot_session.mission_id = Some(started.mission.id.clone());
        slot_session.slot_id = Some("sl1".into());
        slot_session.cwd = Some(wt_str.clone());
        repo::session::insert(&conn, &slot_session).unwrap();

        assert!(files
            .worktree_remove_checked(&conn, repo.path(), "feat/manual")
            .is_err());

        // Archive the mission via repo::mission::complete_and_archive_if_running
        // (notice sessions.archived_at stays NULL while missions.archived_at is set).
        assert_eq!(
            repo::mission::complete_and_archive_if_running(
                &conn,
                &started.mission.id,
                chrono::Utc::now()
            )
            .unwrap(),
            1
        );

        // Unarchived empty (0-slot) mission also blocks removal while unarchived.
        let empty = ops::mission::start_with_files(
            &mut conn,
            app_data.path(),
            Some(&files),
            StartMissionInput {
                crew_id: None,
                project_id: None,
                title: "Empty worktree mission".into(),
                goal_override: None,
                cwd: Some(wt_str.clone()),
                worktree: None,
                started_by_session_id: None,
                no_report: false,
            },
        )
        .unwrap();
        assert!(files
            .worktree_remove_checked(&conn, repo.path(), "feat/manual")
            .is_err());
        assert_eq!(
            repo::mission::complete_and_archive_if_running(
                &conn,
                &empty.mission.id,
                chrono::Utc::now()
            )
            .unwrap(),
            1
        );
    }

    let sub = repo.path().join("sub");
    std::fs::create_dir_all(&sub).unwrap();
    let removed = files
        .worktree_remove_checked(&pool.get().unwrap(), &sub, ".worktrees/manual-hand")
        .unwrap();
    assert!(same_normalized_checkout_path(&removed.path, &manual_wt));
    assert!(!manual_wt.exists());
    // Removal never deletes the branch.
    assert_eq!(
        git(
            repo.path(),
            &["rev-parse", "--verify", "refs/heads/feat/manual"]
        )
        .trim()
        .len(),
        40
    );
}

#[test]
fn worktree_check_and_create_refuse_locked_missing_registration_without_unlocking() {
    let repo = init_clean_repo();
    let files = FilesService::default();

    // Case 1: external locked worktree deleted on disk (`!prunable && locked && !is_dir`).
    let ext_root = tempfile::Builder::new()
        .prefix("1213-m1-locked-")
        .tempdir()
        .unwrap();
    let ext_wt = ext_root.path().join("feat-locked-ext");
    git(
        repo.path(),
        &[
            "worktree",
            "add",
            "-b",
            "feat/locked-ext",
            &ext_wt.to_string_lossy(),
            "main",
        ],
    );
    git(
        repo.path(),
        &["worktree", "lock", &ext_wt.to_string_lossy()],
    );
    std::fs::remove_dir_all(&ext_wt).unwrap();

    let ext_check = files
        .worktree_check(repo.path(), "feat/locked-ext", None)
        .unwrap();
    assert!(
        matches!(
            ext_check,
            WorktreeNameCheck::Invalid { ref reason } if reason.contains("locked")
        ),
        "unexpected check: {ext_check:?}"
    );
    let ext_create_err = files
        .worktree_create(repo.path(), "feat/locked-ext", None)
        .unwrap_err()
        .to_string();
    assert!(
        ext_create_err.contains("locked"),
        "unexpected create error: {ext_create_err}"
    );

    // Case 2: managed `.worktrees/` locked worktree deleted on disk.
    let managed = files
        .worktree_create(repo.path(), "feat/locked-here", None)
        .unwrap();
    git(
        repo.path(),
        &["worktree", "lock", &managed.path.to_string_lossy()],
    );
    std::fs::remove_dir_all(&managed.path).unwrap();

    let managed_check = files
        .worktree_check(repo.path(), "feat/locked-here", None)
        .unwrap();
    assert!(
        matches!(
            managed_check,
            WorktreeNameCheck::Invalid { ref reason } if reason.contains("locked")
        ),
        "unexpected check: {managed_check:?}"
    );
    let managed_create_err = files
        .worktree_create(repo.path(), "feat/locked-here", None)
        .unwrap_err()
        .to_string();
    assert!(
        managed_create_err.contains("locked"),
        "unexpected create error: {managed_create_err}"
    );

    // Locked registrations remain intact (`locked == true && !prunable`).
    let listed = files.worktree_list(repo.path()).unwrap();
    for branch in ["feat/locked-ext", "feat/locked-here"] {
        let item = listed
            .worktrees
            .iter()
            .find(|item| item.branch.as_deref() == Some(branch))
            .expect("locked worktree remains registered");
        assert!(item.locked);
        assert!(!item.prunable);
    }
}

#[test]
fn worktree_checkout_and_restore_cover_removed_external_worktrees_without_prunable_entry() {
    let parent = tempfile::Builder::new()
        .prefix("1213-m1-ext-restore-")
        .tempdir()
        .unwrap();
    let repo = parent.path().join("repo");
    std::fs::create_dir_all(&repo).unwrap();
    git(&repo, &["init", "-q", "-b", "main"]);
    git(&repo, &["config", "core.autocrlf", "false"]);
    std::fs::write(repo.join("README.md"), "hello\n").unwrap();
    git(&repo, &["add", "-A"]);
    git(&repo, &["commit", "-q", "-m", "init"]);

    let outside_wt = parent.path().join("feat-outside");
    git(
        &repo,
        &[
            "worktree",
            "add",
            "-b",
            "feat/outside",
            &outside_wt.to_string_lossy(),
            "main",
        ],
    );
    git(
        &repo,
        &["worktree", "remove", &outside_wt.to_string_lossy()],
    );
    assert!(!outside_wt.exists());

    let files = FilesService::default();
    let surviving = files
        .worktree_checkout_state(&outside_wt, Some(&repo))
        .unwrap();
    assert!(
        matches!(
            surviving,
            WorktreeCheckout::Git {
                kind: WorktreeKind::Linked,
                managed: false,
                branch: Some(ref b),
                missing: true,
                missing_branch: Some(MissingWorktreeBranchState::Surviving),
                ..
            } if b == "feat/outside"
        ),
        "unexpected state: {surviving:?}"
    );
    // Subfolder inside an external worktree (`../feat-outside/src`) with no "src" branch restores `../feat-outside`.
    let restored = files.restore_if_missing(&outside_wt, Some(&repo)).unwrap();
    assert!(same_checkout_path(&restored.unwrap(), &outside_wt));
    let tracked_sub = outside_wt.join("src");
    std::fs::create_dir_all(&tracked_sub).unwrap();
    std::fs::write(tracked_sub.join("lib.rs"), "pub fn ok() {}\n").unwrap();
    git(&outside_wt, &["add", "-A"]);
    git(&outside_wt, &["commit", "-q", "-m", "add src"]);
    git(
        &repo,
        &["worktree", "remove", &outside_wt.to_string_lossy()],
    );
    assert!(!tracked_sub.exists());

    let sub_surviving = files
        .worktree_checkout_state(&tracked_sub, Some(&repo))
        .unwrap();
    assert!(
        matches!(
            sub_surviving,
            WorktreeCheckout::Git {
                ref top,
                managed: false,
                branch: Some(ref b),
                missing: true,
                missing_branch: Some(MissingWorktreeBranchState::Surviving),
                ..
            } if same_checkout_path(top, &outside_wt) && b == "feat/outside"
        ),
        "unexpected state: {sub_surviving:?}"
    );
    let sub_restored = files
        .restore_if_missing(&tracked_sub, Some(&repo))
        .unwrap()
        .unwrap();
    assert!(same_checkout_path(&sub_restored, &outside_wt));
    assert!(tracked_sub.is_dir());

    // Competing descendant branch `"src"` alongside `"feat/outside"` makes root ambiguous and is refused.
    git(&repo, &["branch", "src", "main"]);
    git(
        &repo,
        &["worktree", "remove", &outside_wt.to_string_lossy()],
    );
    let sub_ambig = files
        .worktree_checkout_state(&tracked_sub, Some(&repo))
        .unwrap();
    assert!(
        matches!(
            sub_ambig,
            WorktreeCheckout::Git {
                managed: false,
                branch: None,
                missing: true,
                missing_branch: Some(MissingWorktreeBranchState::Ambiguous),
                ref candidate_branches,
                ..
            } if candidate_branches == &vec!["feat/outside".to_string(), "src".to_string()]
        ),
        "unexpected state: {sub_ambig:?}"
    );
    let sub_ambig_err = files
        .restore_if_missing(&tracked_sub, Some(&repo))
        .unwrap_err()
        .to_string();
    assert!(
        sub_ambig_err.contains("restore in main checkout"),
        "{sub_ambig_err}"
    );
    assert!(!tracked_sub.exists());
    assert!(!outside_wt.exists());
    git(&repo, &["branch", "-D", "src"]);
    let restored = files.restore_if_missing(&outside_wt, Some(&repo)).unwrap();
    assert!(same_checkout_path(&restored.unwrap(), &outside_wt));
    assert_eq!(
        git(&outside_wt, &["branch", "--show-current"]).trim(),
        "feat/outside"
    );

    // Ambiguous flattened branch match preserves fallback behavior.
    git(&repo, &["branch", "feat-outside", "main"]);
    git(
        &repo,
        &["worktree", "remove", &outside_wt.to_string_lossy()],
    );
    let ambiguous = files
        .worktree_checkout_state(&outside_wt, Some(&repo))
        .unwrap();
    assert!(matches!(
        ambiguous,
        WorktreeCheckout::Git {
            managed: false,
            branch: None,
            missing: true,
            missing_branch: Some(MissingWorktreeBranchState::Ambiguous),
            ref candidate_branches,
            ..
        } if candidate_branches.len() == 2
    ));
    let ambig_err = files
        .restore_if_missing(&outside_wt, Some(&repo))
        .unwrap_err()
        .to_string();
    assert!(
        ambig_err.contains("restore in main checkout"),
        "{ambig_err}"
    );

    // With a prunable registration left (rm -rf), deleting the branch preserves Gone fallback;
    // with neither registration nor matching branch, an arbitrary external path stays Plain.
    git(&repo, &["branch", "-D", "feat-outside"]);
    let _ = files.restore_if_missing(&outside_wt, Some(&repo)).unwrap();
    std::fs::remove_dir_all(&outside_wt).unwrap();
    git(&repo, &["update-ref", "-d", "refs/heads/feat/outside"]);
    let gone_prunable = files
        .worktree_checkout_state(&outside_wt, Some(&repo))
        .unwrap();
    assert!(matches!(
        gone_prunable,
        WorktreeCheckout::Git {
            managed: false,
            branch: None,
            missing: true,
            missing_branch: Some(MissingWorktreeBranchState::Gone),
            ..
        }
    ));
    let gone_err = files
        .restore_if_missing(&outside_wt, Some(&repo))
        .unwrap_err()
        .to_string();
    assert!(gone_err.contains("restore in main checkout"), "{gone_err}");

    git(&repo, &["worktree", "prune"]);
    let plain_unregistered = files
        .worktree_checkout_state(&outside_wt, Some(&repo))
        .unwrap();
    assert_eq!(
        plain_unregistered,
        WorktreeCheckout::Plain { missing: true }
    );
    assert!(files
        .restore_if_missing(&outside_wt, Some(&repo))
        .unwrap()
        .is_none());

    let plain_notes = parent.path().join("plain-external-notes");
    assert_eq!(
        files
            .worktree_checkout_state(&plain_notes, Some(&repo))
            .unwrap(),
        WorktreeCheckout::Plain { missing: true }
    );

    // Deleted plain subfolder inside main_checkout remains Plain.
    let missing_sub = repo.join("deleted-subdir");
    let plain_sub = files
        .worktree_checkout_state(&missing_sub, Some(&repo))
        .unwrap();
    assert_eq!(plain_sub, WorktreeCheckout::Plain { missing: true });
}
