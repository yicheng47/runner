use std::path::{Path, PathBuf};

use runner_backend::agent_skill::{self, SkillRootState};
use runner_backend::model::Runtime;
use runner_backend::ops::runtime::runtime_catalog;

use super::AppStore;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct RunnerSkillStatus {
    pub(crate) detected: bool,
    pub(crate) foreign: Option<PathBuf>,
}

impl AppStore {
    pub(crate) fn initialize_skill_defaults(&mut self) {
        let Some(home) = self.home_dir.clone() else {
            self.runner_skill_status = RunnerSkillStatus::default();
            return;
        };
        let debug = cfg!(debug_assertions);
        if !self.settings.runner_skill_enabled {
            if let Err(error) = agent_skill::remove(&home, debug) {
                eprintln!("Runner skill removal failed: {error}");
            }
        }
        let catalog = match runtime_catalog(&self.core) {
            Ok(catalog) => catalog,
            Err(error) => {
                eprintln!("Runner skill defaults failed: {error}");
                self.runner_skill_status = RunnerSkillStatus::default();
                return;
            }
        };
        let content =
            agent_skill::render(debug, &agent_skill::sidecar_path(&self.core.app_data_dir));
        let available = catalog
            .iter()
            .filter(|entry| entry.available)
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        if self.settings.runner_skill_enabled {
            reconcile_skill_roots(&home, debug, &content, &available);
        }
        self.runner_skill_status = skill_status(&home, debug, &content, &available);
    }

    pub(crate) fn refresh_runner_skill_status(&mut self) {
        let Some(home) = &self.home_dir else {
            self.runner_skill_status = RunnerSkillStatus::default();
            return;
        };
        let Ok(catalog) = runtime_catalog(&self.core) else {
            self.runner_skill_status = RunnerSkillStatus::default();
            return;
        };
        let debug = cfg!(debug_assertions);
        let expected =
            agent_skill::render(debug, &agent_skill::sidecar_path(&self.core.app_data_dir));
        let available = catalog
            .iter()
            .filter(|entry| entry.available)
            .map(|entry| entry.name)
            .collect::<Vec<_>>();
        self.runner_skill_status = skill_status(home, debug, &expected, &available);
    }

    pub(crate) fn runner_skill_status(&self) -> &RunnerSkillStatus {
        &self.runner_skill_status
    }
}

fn skill_status(
    home: &Path,
    debug: bool,
    expected: &str,
    available: &[Runtime],
) -> RunnerSkillStatus {
    let mut status = RunnerSkillStatus::default();
    for relative in agent_skill::SKILL_ROOTS {
        let detected = root_is_detected(relative, available);
        status.detected |= detected;
        let root_status = agent_skill::status_root(&home.join(relative), debug, expected);
        if root_status.state == SkillRootState::Foreign && status.foreign.is_none() {
            status.foreign = Some(root_status.folder);
        }
    }
    status
}

fn reconcile_skill_roots(home: &Path, debug: bool, content: &str, available: &[Runtime]) {
    for relative in agent_skill::SKILL_ROOTS {
        if root_is_detected(relative, available) {
            if let Err(error) = agent_skill::install_root(&home.join(relative), debug, content) {
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

fn root_runtimes(relative: &str) -> &'static [Runtime] {
    match relative {
        ".claude/skills" => &[Runtime::ClaudeCode],
        ".agents/skills" => &[Runtime::Codex, Runtime::Copilot, Runtime::Pi],
        ".trae/skills" => &[Runtime::Trae],
        _ => &[],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gpui::{AppContext as _, TestAppContext};
    use runner_backend::{
        db, event_bus, events, mcp, router, session, shell_path, windows, AppCore,
    };
    use std::fs;
    use std::sync::{Arc, Mutex, RwLock};

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
        AppCore {
            db: Arc::new(db::open_pool(&temp.join("runner.db")).unwrap()),
            app_data_dir: app_data,
            sessions: session::SessionManager::new(
                Arc::clone(&runtime_shell_env),
                Arc::clone(&runtime_discovery),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
            buses: event_bus::BusRegistry::new(),
            routers: router::RouterRegistry::new(),
            mission_grid_hint: Arc::new(Mutex::new(None)),
            mcp: Arc::new(mcp::McpHandle::new()),
            windows: Arc::new(windows::WindowRegistry::new()),
            events: events::EventChannel::new(),
            session_event_observer: Default::default(),
            app_version: "0.0.0-test".into(),
        }
    }

    #[test]
    fn root_runtime_ownership_matches_the_three_shared_locations() {
        assert_eq!(root_runtimes(".claude/skills"), &[Runtime::ClaudeCode]);
        assert_eq!(
            root_runtimes(".agents/skills"),
            &[Runtime::Codex, Runtime::Copilot, Runtime::Pi]
        );
        assert_eq!(root_runtimes(".trae/skills"), &[Runtime::Trae]);
    }

    #[test]
    fn detected_roots_install_restore_and_pick_up_later_detection() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let content = "managed skill";

        reconcile_skill_roots(&home, true, content, &[Runtime::Codex]);
        assert_eq!(
            fs::read_to_string(home.join(".agents/skills/runner-dev/SKILL.md")).unwrap(),
            content
        );
        assert!(!home.join(".claude/skills/runner-dev").exists());
        assert!(!home.join(".trae/skills/runner-dev").exists());

        fs::remove_dir_all(home.join(".agents/skills/runner-dev")).unwrap();
        reconcile_skill_roots(&home, true, content, &[Runtime::Codex]);
        assert_eq!(
            fs::read_to_string(home.join(".agents/skills/runner-dev/SKILL.md")).unwrap(),
            content
        );

        reconcile_skill_roots(&home, true, content, &[Runtime::Codex, Runtime::ClaudeCode]);
        assert_eq!(
            fs::read_to_string(home.join(".claude/skills/runner-dev/SKILL.md")).unwrap(),
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
        agent_skill::install(&home, &app_data, true).unwrap();
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
