use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use gpui::{AppContext as _, Context};
use runner_core::protocol::agent_skill::{self, SkillRootState};
use runner_core::protocol::model::Runtime;
use runner_core::protocol::DaemonClient;

use super::AppStore;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RunnerSkillStatus {
    pub(crate) detected: bool,
    pub(crate) foreign: Option<PathBuf>,
}

pub(super) struct SkillReconcileOutcome {
    pub(super) seq: u64,
    pub(super) enabled: bool,
    pub(super) status: RunnerSkillStatus,
}

impl AppStore {
    pub(crate) fn refresh_runner_skill_status(&mut self) {
        let Some(home) = &self.home_dir else {
            self.runner_skill_status = RunnerSkillStatus::default();
            return;
        };
        let Ok(catalog) = self.client.runtime_catalog() else {
            self.runner_skill_status = RunnerSkillStatus::default();
            return;
        };
        let debug = cfg!(debug_assertions);
        let expected = agent_skill::render(debug, &agent_skill::sidecar_path(&self.app_data_dir));
        let available = catalog
            .iter()
            .filter(|entry| entry.available)
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        self.runner_skill_status = skill_status(&self.client, home, debug, &expected, &available);
    }
    pub(crate) fn initialize_skill_defaults(&mut self) {
        self.desired_skill_enabled
            .store(self.settings.runner_skill_enabled, Ordering::Release);
        let outcome = reconcile_skill_defaults_serialized(
            &self.client,
            self.home_dir.as_deref(),
            &self.app_data_dir,
            &self.desired_skill_enabled,
            &self.skill_status_seq,
            &self.skill_reconcile_lock,
        );
        self.apply_skill_reconcile_outcome(outcome);
    }
    pub(super) fn apply_skill_reconcile_outcome(&mut self, outcome: SkillReconcileOutcome) -> bool {
        if outcome.seq >= self.applied_skill_status_seq
            && self.settings.runner_skill_enabled == outcome.enabled
        {
            self.applied_skill_status_seq = outcome.seq;
            self.runner_skill_status = outcome.status;
            return true;
        }
        false
    }
    pub(crate) fn schedule_skill_defaults_reconcile(&mut self, cx: &mut Context<Self>) {
        self.desired_skill_enabled
            .store(self.settings.runner_skill_enabled, Ordering::Release);
        let client = self.client.clone();
        let home = self.home_dir.clone();
        let app_data_dir = self.app_data_dir.clone();
        let desired = Arc::clone(&self.desired_skill_enabled);
        let seq = Arc::clone(&self.skill_status_seq);
        let lock = Arc::clone(&self.skill_reconcile_lock);
        let bg = cx.background_spawn(async move {
            reconcile_skill_defaults_serialized(
                &client,
                home.as_deref(),
                &app_data_dir,
                &desired,
                &seq,
                &lock,
            )
        });
        self.skill_reconcile_task = Some(cx.spawn(async move |weak, cx| {
            let outcome = bg.await;
            let _ = weak.update(cx, |store, cx| {
                if store.apply_skill_reconcile_outcome(outcome) {
                    cx.notify();
                }
            });
        }));
    }
}

pub(super) fn reconcile_skill_defaults_serialized(
    client: &DaemonClient,
    home: Option<&Path>,
    app_data_dir: &Path,
    desired_enabled: &AtomicBool,
    seq_counter: &AtomicU64,
    lock: &Mutex<()>,
) -> SkillReconcileOutcome {
    let Some(home) = home else {
        let _guard = lock.lock().unwrap();
        let seq = seq_counter.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
        return SkillReconcileOutcome {
            seq,
            enabled: desired_enabled.load(Ordering::Acquire),
            status: RunnerSkillStatus::default(),
        };
    };
    let debug = cfg!(debug_assertions);
    if !desired_enabled.load(Ordering::Acquire) {
        let _guard = lock.lock().unwrap();
        if !desired_enabled.load(Ordering::Acquire) {
            if let Err(error) = client.agent_skill_remove(home, debug) {
                eprintln!("Runner skill removal failed: {error}");
            }
        }
    }
    let catalog = match client.runtime_catalog() {
        Ok(catalog) => catalog,
        Err(error) => {
            eprintln!("Runner skill defaults failed: {error}");
            let _guard = lock.lock().unwrap();
            let seq = seq_counter.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
            return SkillReconcileOutcome {
                seq,
                enabled: desired_enabled.load(Ordering::Acquire),
                status: RunnerSkillStatus::default(),
            };
        }
    };
    let content = agent_skill::render(debug, &agent_skill::sidecar_path(app_data_dir));
    let available = catalog
        .iter()
        .filter(|entry| entry.available)
        .map(|entry| entry.name)
        .collect::<Vec<_>>();
    let _guard = lock.lock().unwrap();
    loop {
        let enabled = desired_enabled.load(Ordering::Acquire);
        if enabled {
            reconcile_skill_roots(client, home, debug, &content, &available);
        } else if let Err(error) = client.agent_skill_remove(home, debug) {
            eprintln!("Runner skill removal failed: {error}");
        }
        let status = skill_status(client, home, debug, &content, &available);
        if desired_enabled.load(Ordering::Acquire) == enabled {
            let seq = seq_counter.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
            return SkillReconcileOutcome {
                seq,
                enabled,
                status,
            };
        }
    }
}

fn skill_status(
    client: &DaemonClient,
    home: &Path,
    debug: bool,
    expected: &str,
    available: &[Runtime],
) -> RunnerSkillStatus {
    let mut status = RunnerSkillStatus::default();
    for relative in agent_skill::SKILL_ROOTS {
        let detected = root_is_detected(relative, available);
        status.detected |= detected;
        let root_status = client
            .agent_skill_status_root(&home.join(relative), debug, expected)
            .unwrap_or(agent_skill::SkillRootStatus {
                root: home.join(relative),
                folder: home.join(relative).join(agent_skill::skill_name(debug)),
                state: SkillRootState::Missing,
            });
        if root_status.state == SkillRootState::Foreign && status.foreign.is_none() {
            status.foreign = Some(root_status.folder);
        }
    }
    status
}

fn reconcile_skill_roots(
    client: &DaemonClient,
    home: &Path,
    debug: bool,
    content: &str,
    available: &[Runtime],
) {
    for relative in agent_skill::SKILL_ROOTS {
        if root_is_detected(relative, available) {
            if let Err(error) =
                client.agent_skill_install_root(&home.join(relative), debug, content)
            {
                eprintln!("Runner skill default for {relative} failed: {error}");
            }
        }
    }
}

fn root_is_detected(relative: &str, available: &[Runtime]) -> bool {
    root_runtimes(relative)
        .iter()
        .any(|runtime| available.contains(runtime))
}

fn root_runtimes(relative: &str) -> Vec<Runtime> {
    let mut runtimes: Vec<_> = Runtime::ALL
        .into_iter()
        .filter(|runtime| runtime.managed_skill_root() == Some(relative))
        .collect();
    runtimes.sort_by_key(|runtime| runtime.key());
    runtimes
}

impl super::AppStore {
    pub(crate) fn runner_skill_status(&self) -> &RunnerSkillStatus {
        &self.runner_skill_status
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::app_store::AppStore;
    use gpui::TestAppContext;
    use runner_daemon::{db, session, shell_path, AppCore};
    use std::fs;
    use std::sync::{Arc, RwLock};

    fn test_core(temp: &Path, app_data: PathBuf) -> AppCore {
        let isolated_path = temp.join("empty-bin").display().to_string();
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv {
            path: Some(isolated_path.clone()),
            ..Default::default()
        }));
        let mut discovery = shell_path::DiscoveryState::startup(None, None);
        discovery.checking = false;
        discovery.result = Some(shell_path::DiscoveryResult {
            shell: None,
            outcome: shell_path::DiscoveryOutcome::Ok,
            duration_ms: 0,
            env: shell_path::LoginShellEnv {
                path: Some(isolated_path),
                ..Default::default()
            },
        });
        let runtime_discovery = Arc::new(RwLock::new(discovery));
        crate::test_support::core(
            Arc::new(db::open_fixture_pool(&temp.join("runner.db")).unwrap()),
            app_data,
            session::SessionManager::new(
                Arc::clone(&runtime_shell_env),
                Arc::clone(&runtime_discovery),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
        )
    }

    #[test]
    fn root_runtime_ownership_matches_agent_locations() {
        assert_eq!(root_runtimes(".claude/skills"), &[Runtime::ClaudeCode]);
        assert_eq!(
            root_runtimes(".agents/skills"),
            &[
                Runtime::Codex,
                Runtime::Copilot,
                Runtime::Cursor,
                Runtime::Grok,
                Runtime::OpenCode,
                Runtime::Pi
            ]
        );
        assert_eq!(root_runtimes(".trae/skills"), &[Runtime::Trae]);
        assert_eq!(
            root_runtimes(".gemini/antigravity-cli/skills"),
            &[Runtime::Antigravity]
        );
    }

    #[test]
    fn detected_roots_install_restore_and_pick_up_later_detection() {
        let temp = tempfile::tempdir().unwrap();
        let client = crate::test_support::isolated_client(temp.path());
        let home = temp.path().join("home");
        let content = "managed skill";

        reconcile_skill_roots(&client, &home, true, content, &[Runtime::Codex]);
        assert_eq!(
            fs::read_to_string(home.join(".agents/skills/runner-dev/SKILL.md")).unwrap(),
            content
        );
        assert!(!home.join(".claude/skills/runner-dev").exists());
        assert!(!home.join(".trae/skills/runner-dev").exists());

        fs::remove_dir_all(home.join(".agents/skills/runner-dev")).unwrap();
        reconcile_skill_roots(&client, &home, true, content, &[Runtime::Codex]);
        assert_eq!(
            fs::read_to_string(home.join(".agents/skills/runner-dev/SKILL.md")).unwrap(),
            content
        );

        reconcile_skill_roots(
            &client,
            &home,
            true,
            content,
            &[Runtime::Codex, Runtime::ClaudeCode, Runtime::Antigravity],
        );
        assert_eq!(
            fs::read_to_string(home.join(".claude/skills/runner-dev/SKILL.md")).unwrap(),
            content
        );
        assert_eq!(
            fs::read_to_string(home.join(".gemini/antigravity-cli/skills/runner-dev/SKILL.md"))
                .unwrap(),
            content
        );
    }

    #[test]
    fn startup_uses_only_the_injected_home_and_detection_ignores_agent_switches() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("test-home");
        let untouched_home = temp.path().join("not-the-test-home");
        let app_data = temp.path().join("app-data");
        let executable = temp
            .path()
            .join(format!("codex{}", std::env::consts::EXE_SUFFIX));
        fs::write(&executable, "test").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            fs::set_permissions(&executable, fs::Permissions::from_mode(0o755)).unwrap();
        }
        for target_home in [&home, &untouched_home] {
            let folder = target_home.join(".agents/skills/runner-dev");
            fs::create_dir_all(&folder).unwrap();
            fs::write(folder.join(agent_skill::SKILL_MARKER), "managed").unwrap();
            fs::write(folder.join("SKILL.md"), "stale canary").unwrap();
        }
        let config_canaries: Vec<_> = runner_daemon::ops::mcp::McpClientId::all()
            .into_iter()
            .map(|client| client.config_path(&home))
            .collect();
        for path in &config_canaries {
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, "config canary").unwrap();
        }

        let core = test_core(temp.path(), app_data.clone());
        db::set_runtime_override(&core.db, Runtime::Codex.key(), executable.to_str()).unwrap();
        let mut settings = crate::app_settings::AppSettings::default();
        settings.disabled_agents.insert("codex".into());
        let mut cx = TestAppContext::single();
        let store = cx.new(|cx| {
            AppStore::new(
                core,
                Some(home.clone()),
                None,
                temp.path().join("settings.json"),
                settings,
                None,
                cx,
            )
        });

        let expected = agent_skill::render(true, &agent_skill::sidecar_path(&app_data));
        assert_eq!(
            fs::read_to_string(home.join(".agents/skills/runner-dev/SKILL.md")).unwrap(),
            expected
        );
        assert_eq!(
            fs::read_to_string(untouched_home.join(".agents/skills/runner-dev/SKILL.md")).unwrap(),
            "stale canary"
        );
        for path in &config_canaries {
            assert_eq!(fs::read_to_string(path).unwrap(), "config canary");
        }
        assert!(store.read_with(&cx, |store, _| store.settings.mcp_registrations_removed));
        assert!(store.read_with(&cx, |store, _| store.runner_skill_status().detected));

        fs::remove_dir_all(home.join(".agents/skills/runner-dev")).unwrap();
        store.update(&mut cx, |store, _| store.initialize_skill_defaults());
        assert_eq!(
            fs::read_to_string(home.join(".agents/skills/runner-dev/SKILL.md")).unwrap(),
            expected
        );
    }

    #[test]
    fn disabled_switch_removes_all_owned_roots_and_stays_off() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let app_data = temp.path().join("app");
        runner_daemon::agent_skill::install(&home, &app_data, true).unwrap();
        let foreign = home.join(".trae/skills/runner-dev");
        fs::remove_file(foreign.join(agent_skill::SKILL_MARKER)).unwrap();
        let settings = crate::app_settings::AppSettings {
            runner_skill_enabled: false,
            ..Default::default()
        };
        let core = test_core(temp.path(), app_data);
        let mut cx = TestAppContext::single();
        let store = cx.new(|cx| {
            AppStore::new(
                core,
                Some(home.clone()),
                None,
                temp.path().join("settings.json"),
                settings,
                None,
                cx,
            )
        });
        for relative in agent_skill::SKILL_ROOTS {
            let folder = home.join(relative).join("runner-dev");
            if *relative == ".trae/skills" {
                assert!(folder.exists());
            } else {
                assert!(!folder.exists());
            }
        }
        assert_eq!(
            store.read_with(&cx, |store, _| store.runner_skill_status().foreign.clone()),
            Some(foreign)
        );
    }
}
