use runner_backend::agent_skill::{self, InstallOutcome};
use runner_backend::model::Runtime;
use runner_backend::ops::runtime::runtime_catalog;

use super::AppStore;
use crate::app_settings::AppSettings;

impl AppStore {
    pub(crate) fn initialize_skill_defaults(&mut self) {
        let Some(home) = self.skill_home.clone() else {
            return;
        };
        let debug = cfg!(debug_assertions);
        if let Err(error) = agent_skill::refresh(&home, &self.core.app_data_dir, debug) {
            eprintln!("Runner skill refresh failed: {error}");
        }
        let catalog = match runtime_catalog(&self.core) {
            Ok(catalog) => catalog,
            Err(error) => {
                eprintln!("Runner skill defaults failed: {error}");
                return;
            }
        };
        let content =
            agent_skill::render(debug, &agent_skill::sidecar_path(&self.core.app_data_dir));
        let mut changed = false;
        for relative in agent_skill::SKILL_ROOTS {
            let eligible = root_runtimes(relative).iter().any(|runtime| {
                catalog.iter().any(|entry| {
                    entry.name == *runtime
                        && entry.available
                        && self
                            .settings
                            .is_agent_enabled(entry.name, entry.default_enabled)
                })
            });
            match initialize_root(&mut self.settings, relative, eligible, || {
                agent_skill::install_root(&home.join(relative), debug, &content)
            }) {
                Ok(initialized) => changed |= initialized,
                Err(error) => eprintln!("Runner skill default for {relative} failed: {error}"),
            }
        }
        if changed {
            self.save_settings();
        }
    }
}

fn root_runtimes(relative: &str) -> &'static [Runtime] {
    match relative {
        ".claude/skills" => &[Runtime::ClaudeCode],
        ".agents/skills" => &[Runtime::Codex, Runtime::Copilot, Runtime::Pi],
        ".trae/skills" => &[Runtime::Trae],
        _ => &[],
    }
}

fn initialize_root(
    settings: &mut AppSettings,
    relative: &str,
    eligible: bool,
    install: impl FnOnce() -> runner_backend::error::Result<InstallOutcome>,
) -> runner_backend::error::Result<bool> {
    if settings.initialized_skill_roots.contains(relative) || !eligible {
        return Ok(false);
    }
    install()?;
    Ok(settings.initialized_skill_roots.insert(relative.into()))
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
    fn initializes_each_eligible_root_once_and_respects_removal() {
        let mut settings = AppSettings::default();
        let mut installs = 0;
        assert!(initialize_root(&mut settings, ".agents/skills", true, || {
            installs += 1;
            Ok(InstallOutcome::Installed)
        })
        .unwrap());
        assert_eq!(installs, 1);
        assert!(!initialize_root(&mut settings, ".agents/skills", true, || {
            panic!("a removed skill must not be reinstalled")
        })
        .unwrap());

        let reloaded: AppSettings =
            serde_json::from_str(&serde_json::to_string(&settings).unwrap()).unwrap();
        assert!(reloaded.initialized_skill_roots.contains(".agents/skills"));
    }

    #[test]
    fn unavailable_root_is_not_recorded_and_can_install_later() {
        let mut settings = AppSettings::default();
        assert!(!initialize_root(&mut settings, ".trae/skills", false, || {
            panic!("an unavailable root must not be installed")
        })
        .unwrap());
        assert!(settings.initialized_skill_roots.is_empty());
        assert!(initialize_root(&mut settings, ".trae/skills", true, || {
            Ok(InstallOutcome::Installed)
        })
        .unwrap());
    }

    #[test]
    fn foreign_root_is_attempted_once_without_becoming_runner_owned() {
        let mut settings = AppSettings::default();
        assert!(initialize_root(&mut settings, ".claude/skills", true, || {
            Ok(InstallOutcome::Foreign)
        })
        .unwrap());
        assert!(settings.initialized_skill_roots.contains(".claude/skills"));
    }

    #[test]
    fn app_store_startup_uses_only_the_injected_skill_home() {
        let temp = tempfile::tempdir().unwrap();
        let skill_home = temp.path().join("test-home");
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
        for home in [&skill_home, &untouched_home] {
            for relative in [".claude/skills", ".trae/skills"] {
                let folder = home.join(relative).join("runner-dev");
                fs::create_dir_all(&folder).unwrap();
                fs::write(folder.join(agent_skill::SKILL_MARKER), "managed").unwrap();
                fs::write(folder.join("SKILL.md"), "stale canary").unwrap();
            }
        }

        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = AppCore {
            db: Arc::new(db::open_pool(&temp.path().join("runner.db")).unwrap()),
            app_data_dir: app_data.clone(),
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
        };
        db::set_runtime_override(&core.db, Runtime::Codex.key(), executable.to_str()).unwrap();
        let mut cx = TestAppContext::single();
        let store = cx.new(|cx| {
            AppStore::new(
                core,
                Some(skill_home.clone()),
                temp.path().join("settings.json"),
                AppSettings::default(),
                None,
                cx,
            )
        });

        let expected = agent_skill::render(true, &agent_skill::sidecar_path(&app_data));
        for relative in agent_skill::SKILL_ROOTS {
            assert_eq!(
                fs::read_to_string(skill_home.join(relative).join("runner-dev/SKILL.md")).unwrap(),
                expected
            );
        }
        assert!(store.read_with(&cx, |store, _| store
            .settings
            .initialized_skill_roots
            .contains(".agents/skills")));
        for relative in [".claude/skills", ".trae/skills"] {
            assert_eq!(
                fs::read_to_string(untouched_home.join(relative).join("runner-dev/SKILL.md"))
                    .unwrap(),
                "stale canary"
            );
        }
        assert!(!untouched_home.join(".agents/skills/runner-dev").exists());
    }
}
