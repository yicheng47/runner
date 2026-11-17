use std::path::Path;

use gpui::{div, prelude::*, px, rems, svg, AnyElement, FontWeight, SharedString, Window};
use runner_app::ui::{Button, ButtonSize, ButtonVariant};
use runner_core::app_paths::is_existing_dir;
use runner_core::protocol::worktree::{MissingWorktreeBranchState, WorktreeCheckout};
use runner_core::protocol::DaemonClient;

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RestoreFallbackPrompt {
    pub(crate) message: crate::i18n::Text,
}

fn clean_restore_reason(error: &str) -> String {
    let stripped = error
        .split_once("cannot recreate missing worktree ")
        .and_then(|(_, rest)| rest.split_once(": "))
        .map(|(_, detail)| detail)
        .unwrap_or(error);
    stripped
        .split("; restore in main checkout ")
        .next()
        .unwrap_or(stripped)
        .trim()
        .to_owned()
}

fn classify_error_string(error: &str) -> Option<RestoreFallbackPrompt> {
    if !error.contains("cannot recreate missing worktree") {
        return None;
    }
    if error.contains("branch no longer exists") {
        return Some(RestoreFallbackPrompt {
            message: crate::i18n::message("restore.branch_gone"),
        });
    }
    Some(RestoreFallbackPrompt {
        message: crate::i18n::messagef(
            "restore.failed_fallback_hint",
            &[("reason", clean_restore_reason(error))],
        ),
    })
}

/// Classifies whether a failed restore or resume is caused by an unrecoverable or
/// missing linked worktree, returning copy that explains why and warns about
/// Claude history loss when falling back to the main checkout.
pub(crate) fn classify_restore_failure(
    daemon: &DaemonClient,
    cwd: Option<&str>,
    project_id: Option<&str>,
    error: &str,
) -> Option<RestoreFallbackPrompt> {
    if let Some(cwd) = cwd.map(str::trim).filter(|c| !c.is_empty())
        && !is_existing_dir(Path::new(cwd))
        && let Ok(WorktreeCheckout::Git {
            missing: true,
            missing_branch,
            candidate_branches,
            ..
        }) = daemon.worktree_checkout(Path::new(cwd), project_id)
    {
        let message = match missing_branch {
            Some(MissingWorktreeBranchState::Gone) => crate::i18n::message("restore.branch_gone"),
            Some(MissingWorktreeBranchState::Ambiguous) => crate::i18n::messagef(
                "restore.branch_ambiguous",
                &[("branches", candidate_branches.join(", "))],
            ),
            Some(MissingWorktreeBranchState::Surviving) | None => crate::i18n::messagef(
                "restore.failed_fallback_hint",
                &[("reason", clean_restore_reason(error))],
            ),
        };
        return Some(RestoreFallbackPrompt { message });
    }
    classify_error_string(error)
}

pub(crate) fn render_restore_fallback_banner(
    id_prefix: &'static str,
    prompt: &RestoreFallbackPrompt,
    busy: bool,
    on_restore_in_main: impl Fn(&mut Window, &mut gpui::App) + 'static,
    on_dismiss: impl Fn(&mut Window, &mut gpui::App) + 'static,
) -> AnyElement {
    let color = crate::theme::danger();
    div()
        .id(SharedString::from(format!("{id_prefix}-restore-fallback")))
        .debug_selector(|| "RESTORE_FALLBACK_BANNER".into())
        .w_full()
        .flex_none()
        .flex()
        .flex_wrap()
        .items_center()
        .justify_between()
        .gap(rems(10. / 16.))
        .px(rems(14. / 16.))
        .py(rems(10. / 16.))
        .bg(gpui::rgb(0x22161a))
        .border_b_1()
        .border_color(gpui::rgb(0x4a2226))
        .child(
            div()
                .flex_1()
                .min_w(px(220.))
                .flex()
                .items_start()
                .gap(rems(10. / 16.))
                .child(
                    svg()
                        .mt(rems(2. / 16.))
                        .path("triangle-alert.svg")
                        .size(rems(14. / 16.))
                        .flex_none()
                        .text_color(color),
                )
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .whitespace_normal()
                        .text_size(crate::theme::text_ui())
                        .line_height(rems(18. / 16.))
                        .font_weight(FontWeight::MEDIUM)
                        .text_color(gpui::rgb(0xd8d8dc))
                        .child(prompt.message.resolve()),
                ),
        )
        .child(
            div()
                .flex_none()
                .flex()
                .items_center()
                .gap(rems(6. / 16.))
                .child(
                    Button::new(
                        SharedString::from(format!("{id_prefix}-restore-in-main")),
                        crate::i18n::text("restore.in_main_checkout"),
                    )
                    .debug_selector("RESTORE_IN_MAIN_BUTTON")
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Secondary)
                    .disabled(busy)
                    .on_press(on_restore_in_main),
                )
                .child(
                    Button::new(
                        SharedString::from(format!("{id_prefix}-dismiss-restore-fallback")),
                        crate::i18n::text("ui.dismiss"),
                    )
                    .size(ButtonSize::Sm)
                    .variant(ButtonVariant::Ghost)
                    .disabled(busy)
                    .on_press(on_dismiss),
                ),
        )
        .into_any_element()
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use crate::surfaces::start_chat::{tests::modal_harness, ChatMode};
    use crate::{AppRoute, PaneLayout};

    use super::*;

    fn git_cmd(dir: &Path, args: &[&str]) {
        let out = std::process::Command::new("git")
            .args(args)
            .current_dir(dir)
            .output()
            .expect("git command");
        assert!(
            out.status.success(),
            "git {:?} failed: {}",
            args,
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn wait_until(visual: &gpui::VisualTestContext, label: &str, mut pred: impl FnMut() -> bool) {
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            visual.run_until_parked();
            if pred() {
                return;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for {label}"
            );
            std::thread::yield_now();
        }
    }

    #[test]
    fn non_worktree_failure_does_not_offer_restore_fallback() {
        let temp = tempfile::Builder::new()
            .prefix("1213-m2-nonwt-restore-")
            .tempdir()
            .unwrap();
        let db_pool =
            Arc::new(runner_daemon::db::open_fixture_pool(&temp.path().join("r.db")).unwrap());
        let env = Arc::new(std::sync::RwLock::new(
            runner_daemon::shell_path::LoginShellEnv::default(),
        ));
        let discovery = Arc::new(std::sync::RwLock::new(
            runner_daemon::shell_path::DiscoveryState::startup(None, None),
        ));
        let core = crate::test_support::core(
            db_pool,
            temp.path().to_owned(),
            runner_daemon::session::SessionManager::new(
                env.clone(),
                discovery.clone(),
                Arc::new(runner_daemon::session::pty_runtime::PtyRuntime::new()),
            ),
            env,
            discovery,
        );
        let client = DaemonClient::new(Arc::new(runner_daemon::daemon::InProcessTransport(core)));
        let dir = temp.path().join("existing");
        std::fs::create_dir_all(&dir).unwrap();
        assert!(classify_restore_failure(
            &client,
            Some(dir.to_str().unwrap()),
            None,
            "binary codex not found"
        )
        .is_none());
    }

    #[test]
    fn chat_and_mission_resume_restore_fallback_offer_main_checkout() {
        let temp = tempfile::Builder::new()
            .prefix("1213-m2-resume-restore-")
            .tempdir()
            .unwrap();
        let repo = temp.path().join("repo");
        std::fs::create_dir_all(repo.join("apps/web")).unwrap();
        git_cmd(&repo, &["init", "-b", "main"]);
        git_cmd(&repo, &["config", "user.name", "Runner Test"]);
        git_cmd(&repo, &["config", "user.email", "test@runner.local"]);
        std::fs::write(repo.join("apps/web/index.ts"), "export {};\n").unwrap();
        git_cmd(&repo, &["add", "."]);
        git_cmd(&repo, &["commit", "-m", "initial"]);

        let chat_wt = repo.join(".worktrees/chat-gone");
        let mission_wt = repo.join(".worktrees/mission-gone");
        let slot_wt = repo.join(".worktrees/slot-gone");
        git_cmd(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "feat/chat-gone",
                chat_wt.to_str().unwrap(),
            ],
        );
        git_cmd(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "feat/mission-gone",
                mission_wt.to_str().unwrap(),
            ],
        );
        git_cmd(
            &repo,
            &[
                "worktree",
                "add",
                "-b",
                "feat/slot-gone",
                slot_wt.to_str().unwrap(),
            ],
        );
        std::fs::remove_dir_all(&chat_wt).unwrap();
        std::fs::remove_dir_all(&mission_wt).unwrap();
        std::fs::remove_dir_all(&slot_wt).unwrap();
        git_cmd(&repo, &["worktree", "prune"]);
        git_cmd(&repo, &["branch", "-D", "feat/chat-gone"]);
        git_cmd(&repo, &["branch", "-D", "feat/mission-gone"]);
        git_cmd(&repo, &["branch", "-D", "feat/slot-gone"]);

        let mut harness = modal_harness(1400., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, _, cx| {
            root.start_chat_modal = None;
            let core = root.app_store.read(cx).test_core.clone();
            let conn = core.db.get().unwrap();
            let project = runner_daemon::repo::project::create(
                &conn,
                "ResumeRepo",
                &repo.to_string_lossy(),
            )
            .unwrap();
            conn.execute_batch(
                "INSERT INTO roles (id, handle, display_name, runtime, command, system_prompt, created_at, updated_at)
                 VALUES ('r-coder', 'coder', 'Coder', 'codex', '/bin/sh', 'Coder', '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');
                 INSERT INTO crews (id, name, created_at, updated_at)
                 VALUES ('crew', 'Crew', '2026-11-17T00:00:00Z', '2026-11-17T00:00:00Z');
                 INSERT INTO slots (id, crew_id, role_id, slot_handle, position, lead, added_at)
                 VALUES ('slot-1', 'crew', 'r-coder', 'coder', 0, 1, '2026-11-17T00:00:00Z');",
            )
            .unwrap();
            let chat_sub = chat_wt.join("apps/web");
            conn.execute(
                "INSERT INTO sessions(id,title,status,agent_runtime,agent_command,project_id,cwd)
                 VALUES ('stopped-chat','Stopped Chat','stopped','codex','/bin/sh',?1,?2)",
                [project.id.as_str(), chat_sub.to_string_lossy().as_ref()],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO missions(id,crew_id,project_id,title,status,cwd,started_at)
                 VALUES ('stopped-mission','crew',?1,'Gone mission','running',?2,'2026-11-17T00:00:00Z'),
                        ('slot-mission','crew',?1,'Slot gone mission','running',?3,'2026-11-17T00:00:00Z')",
                [
                    project.id.as_str(),
                    mission_wt.to_string_lossy().as_ref(),
                    slot_wt.to_string_lossy().as_ref(),
                ],
            )
            .unwrap();
            conn.execute(
                "INSERT INTO sessions(id,mission_id,slot_id,role_id,status,agent_runtime,agent_command,project_id,cwd)
                 VALUES ('m-slot-1','stopped-mission','slot-1','r-coder','stopped','codex','/bin/sh',?1,?2),
                        ('m-slot-2','slot-mission','slot-1','r-coder','stopped','codex','/bin/sh',?1,?3)",
                [
                    project.id.as_str(),
                    mission_wt.to_string_lossy().as_ref(),
                    slot_wt.to_string_lossy().as_ref(),
                ],
            )
            .unwrap();
            runner_daemon::repo::node::ensure_mission_node(
                &conn,
                "stopped-mission",
                Some(&project.id),
            )
            .unwrap();
            runner_daemon::repo::node::ensure_mission_node(
                &conn,
                "slot-mission",
                Some(&project.id),
            )
            .unwrap();
            drop(conn);
            let mut chat_tab = PaneLayout::single(Some("stopped-chat"), &[]);
            chat_tab.id = "01ARZ3NDEKTSV4RRFFQ69G5FAV".into();
            root.core(cx)
                .node_tab_upsert(chat_tab.upsert_input().unwrap())
                .unwrap();
            root.refresh_sessions(cx);
            root.reload_tabs(cx).unwrap();
            root.app_store.update(cx, |store, cx| {
                store.refresh(crate::app_store::StoreRefreshKind::All, cx);
            });
        });

        let mut visual = harness.visual.clone();
        let deadline = std::time::Instant::now() + std::time::Duration::from_secs(10);
        loop {
            visual.run_until_parked();
            let (sessions, missions) = {
                let mut s = Vec::new();
                let mut m = Vec::new();
                harness.act(|root, _, cx| {
                    let store = root.app_store.read(cx);
                    s = store
                        .sessions
                        .iter()
                        .map(|r| r.session_id.clone())
                        .collect();
                    m = store
                        .missions
                        .iter()
                        .map(|r| r.mission.id.clone())
                        .collect();
                });
                (s, m)
            };
            if sessions.iter().any(|s| s == "stopped-chat")
                && missions.iter().any(|m| m == "stopped-mission")
                && missions.iter().any(|m| m == "slot-mission")
            {
                break;
            }
            assert!(
                std::time::Instant::now() < deadline,
                "timed out waiting for store loaded chat and mission: sessions={sessions:?}, missions={missions:?}"
            );
            std::thread::yield_now();
        }

        // 1. Chat resume on deleted worktree + deleted branch -> offers Restore in main checkout.
        harness.act(|root, window, cx| {
            root.tabs.activate("01ARZ3NDEKTSV4RRFFQ69G5FAV");
            root.route = AppRoute::Chat;
            root.resume_chat("p1", "stopped-chat", window, cx);
        });
        wait_until(&visual, "chat restore fallback offered", || {
            let mut offered = false;
            harness.act(|root, _, _| {
                offered = root.chat_restore_fallback.is_some();
            });
            offered
        });
        let chat_msg = {
            let mut msg = String::new();
            harness.act(|root, _, _| {
                msg = root
                    .chat_restore_fallback
                    .as_ref()
                    .unwrap()
                    .prompt
                    .message
                    .resolve()
                    .to_string();
            });
            msg
        };
        assert!(chat_msg.contains("no longer exists"), "{chat_msg}");
        assert!(
            chat_msg.to_lowercase().contains("without its history"),
            "{chat_msg}"
        );
        visual.run_until_parked();
        assert!(visual.debug_bounds("RESTORE_FALLBACK_BANNER").is_some());
        assert!(visual.debug_bounds("RESTORE_IN_MAIN_BUTTON").is_some());

        harness.act(|root, window, cx| root.confirm_chat_restore_in_main(window, cx));
        wait_until(&visual, "stopped-chat cwd moved to main/apps/web", || {
            let mut moved = false;
            harness.act(|root, _, cx| {
                let core = root.app_store.read(cx).test_core.clone();
                let row =
                    runner_daemon::repo::session::get_row(&core.db.get().unwrap(), "stopped-chat")
                        .unwrap()
                        .unwrap();
                moved =
                    runner_core::app_paths::normalize_checkout_path(Path::new(
                        row.cwd.as_deref().unwrap(),
                    )) == runner_core::app_paths::normalize_checkout_path(&repo.join("apps/web"));
            });
            moved
        });

        // 2. Mission resume on deleted worktree + deleted branch -> offers Restore in main checkout.
        harness.act(|root, window, cx| {
            root.open_mission("stopped-mission".into(), window, cx);
        });
        wait_until(&visual, "mission opened", || {
            let mut ready = false;
            harness.act(|root, _, cx| {
                let ws = root.mission_workspace.read(cx);
                ready = ws.mission_id.as_deref() == Some("stopped-mission") && !ws.loading;
            });
            ready
        });
        harness.act(|root, window, cx| {
            root.mission_workspace.update(cx, |ws, cx| {
                ws.resume_open_mission(window, cx);
            });
        });
        wait_until(&visual, "mission restore fallback offered", || {
            let mut offered = false;
            harness.act(|root, _, cx| {
                offered = root.mission_workspace.read(cx).restore_fallback.is_some();
            });
            offered
        });
        let mission_msg = {
            let mut msg = String::new();
            harness.act(|root, _, cx| {
                msg = root
                    .mission_workspace
                    .read(cx)
                    .restore_fallback
                    .as_ref()
                    .unwrap()
                    .prompt
                    .message
                    .resolve()
                    .to_string();
            });
            msg
        };
        assert!(mission_msg.contains("no longer exists"), "{mission_msg}");
        assert!(
            mission_msg.to_lowercase().contains("without its history"),
            "{mission_msg}"
        );
        harness.act(|root, window, cx| {
            root.mission_workspace.update(cx, |ws, cx| {
                ws.confirm_mission_restore_in_main(None, window, cx);
            });
        });
        wait_until(&visual, "stopped-mission cwd moved to main root", || {
            let mut moved = false;
            harness.act(|root, _, cx| {
                let core = root.app_store.read(cx).test_core.clone();
                let m =
                    runner_daemon::repo::mission::get(&core.db.get().unwrap(), "stopped-mission")
                        .unwrap()
                        .unwrap();
                let s = runner_daemon::repo::session::get_row(&core.db.get().unwrap(), "m-slot-1")
                    .unwrap()
                    .unwrap();
                let norm_repo = runner_core::app_paths::normalize_checkout_path(&repo);
                moved = runner_core::app_paths::normalize_checkout_path(Path::new(
                    m.cwd.as_deref().unwrap(),
                )) == norm_repo
                    && runner_core::app_paths::normalize_checkout_path(Path::new(
                        s.cwd.as_deref().unwrap(),
                    )) == norm_repo;
            });
            moved
        });

        // 3. Single mission slot resume on deleted worktree + deleted branch -> offers Restore in main checkout.
        harness.act(|root, window, cx| {
            root.open_mission("slot-mission".into(), window, cx);
        });
        wait_until(&visual, "slot-mission opened", || {
            let mut ready = false;
            harness.act(|root, _, cx| {
                let ws = root.mission_workspace.read(cx);
                ready = ws.mission_id.as_deref() == Some("slot-mission") && !ws.loading;
            });
            ready
        });
        harness.act(|root, window, cx| {
            root.mission_workspace.update(cx, |ws, cx| {
                ws.act_on_slot(
                    "m-slot-2",
                    runner_app::ui::SessionControlKind::Resume,
                    window,
                    cx,
                );
            });
        });
        wait_until(&visual, "slot restore fallback offered", || {
            let mut offered = false;
            harness.act(|root, _, cx| {
                offered = root
                    .mission_workspace
                    .read(cx)
                    .restore_fallback
                    .as_ref()
                    .and_then(|fb| fb.slot_session_id.as_deref())
                    == Some("m-slot-2");
            });
            offered
        });
        harness.act(|root, window, cx| {
            root.mission_workspace.update(cx, |ws, cx| {
                ws.confirm_mission_restore_in_main(Some("m-slot-2".into()), window, cx);
            });
        });
        wait_until(&visual, "slot-mission cwd moved to main root", || {
            let mut moved = false;
            harness.act(|root, _, cx| {
                let core = root.app_store.read(cx).test_core.clone();
                let m = runner_daemon::repo::mission::get(&core.db.get().unwrap(), "slot-mission")
                    .unwrap()
                    .unwrap();
                let s = runner_daemon::repo::session::get_row(&core.db.get().unwrap(), "m-slot-2")
                    .unwrap()
                    .unwrap();
                let norm_repo = runner_core::app_paths::normalize_checkout_path(&repo);
                moved = runner_core::app_paths::normalize_checkout_path(Path::new(
                    m.cwd.as_deref().unwrap(),
                )) == norm_repo
                    && runner_core::app_paths::normalize_checkout_path(Path::new(
                        s.cwd.as_deref().unwrap(),
                    )) == norm_repo;
            });
            moved
        });
    }
}
