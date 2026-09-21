use std::path::PathBuf;

use runner_backend::agent_skill;
use runner_backend::cli_install::NoUserPathRegistry;
#[cfg(target_os = "macos")]
use runner_backend::cli_install::OsascriptEscalation;
use runner_backend::cli_install::{
    self, CommandActionOutcome, CommandEscalation, CommandInstallInputs, CommandPlatform,
    NoEscalation, RunnerCommandStatus, UserPathRegistry,
};

use super::AppStore;
use crate::app_settings::AppSettings;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum UserCommandAction {
    Install,
    Uninstall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct CommandInstallIntegration {
    system: bool,
}

#[derive(Clone, Debug)]
pub(crate) struct CommandInstallSupport {
    system_bin: PathBuf,
    integration: CommandInstallIntegration,
}

impl CommandInstallSupport {
    #[cfg(not(test))]
    pub(crate) fn system(system_bin: PathBuf) -> Self {
        Self {
            system_bin,
            integration: CommandInstallIntegration { system: true },
        }
    }

    #[cfg(test)]
    #[cfg_attr(not(unix), allow(dead_code))]
    fn isolated(system_bin: PathBuf) -> Self {
        Self {
            system_bin,
            integration: CommandInstallIntegration { system: false },
        }
    }
}

pub(crate) fn mark_command_install_initialized(settings: &mut AppSettings) -> bool {
    if settings.initialized_command_install {
        return false;
    }
    settings.initialized_command_install = true;
    true
}

impl AppStore {
    pub(crate) fn initialize_command_default(&mut self) {
        if !self.settings.initialized_command_install {
            if let Some(inputs) = self.command_install_inputs(true) {
                let integration = self
                    .command_install_support
                    .as_ref()
                    .expect("command inputs require support")
                    .integration;
                let result = with_command_registry(integration, |registry| {
                    let mut escalation = NoEscalation;
                    initialize_command_default_with(
                        &mut self.settings,
                        Some(&inputs),
                        registry,
                        &mut escalation,
                    )
                });
                match result {
                    Ok(true) => self.save_settings(),
                    Ok(false) => {}
                    Err(error) => eprintln!("Runner command default failed: {error}"),
                }
            }
        }
        self.refresh_runner_command_status();
    }

    pub(crate) fn command_install_inputs(
        &self,
        require_finished_discovery: bool,
    ) -> Option<CommandInstallInputs> {
        let support = self.command_install_support.as_ref()?;
        let home = self.home_dir.clone()?;
        let debug = cfg!(debug_assertions);
        #[cfg(windows)]
        let platform = CommandPlatform::Windows;
        #[cfg(not(windows))]
        let platform = CommandPlatform::Unix;

        let login_path = if platform == CommandPlatform::Unix {
            let discovery = self.core.runtime_discovery.read().ok()?;
            if require_finished_discovery
                && !matches!(
                    discovery.result.as_ref(),
                    Some(result)
                        if result.outcome.is_success() && result.env.path.is_some()
                )
            {
                return None;
            }
            discovery
                .result
                .as_ref()
                .and_then(|result| result.env.path.clone())
                .or_else(|| self.core.runtime_shell_env.read().ok()?.path.clone())
                .unwrap_or_default()
        } else {
            std::env::var("PATH").unwrap_or_default()
        };
        let sidecar = agent_skill::sidecar_path(&self.core.app_data_dir);
        let local_bin = home.join(".local/bin");
        let system_bin = support.system_bin.clone();
        Some(CommandInstallInputs {
            home,
            login_path,
            system_path: std::env::var("PATH").unwrap_or_default(),
            sidecar,
            local_bin,
            system_bin: system_bin.clone(),
            system_bin_writable: cli_install::directory_writable(&system_bin),
            debug,
            platform,
        })
    }

    pub(crate) fn refresh_runner_command_status(&mut self) {
        self.runner_command_status = None;
        self.command_install_requires_escalation = false;
        self.command_action_target = None;
        let Some(inputs) = self.command_install_inputs(false) else {
            return;
        };
        let integration = self
            .command_install_support
            .as_ref()
            .expect("command inputs require support")
            .integration;
        let force_escalated = cli_install::force_escalated_command_install();
        let result = with_command_registry(integration, |registry| {
            let status = cli_install::command_status(&inputs, registry)?;
            let requires_escalation = inputs.platform == CommandPlatform::Unix
                && (force_escalated
                    || cli_install::default_command_target(&inputs, registry)?.is_none());
            let target =
                cli_install::explicit_command_target_path(&inputs, registry, force_escalated)?;
            Ok::<_, runner_backend::error::Error>((status, requires_escalation, target))
        });
        match result {
            Ok((status, requires_escalation, target)) => {
                self.runner_command_status = Some(status);
                self.command_install_requires_escalation = requires_escalation;
                self.command_action_target = Some(target);
            }
            Err(error) => eprintln!("Runner command status failed: {error}"),
        }
    }

    pub(crate) fn runner_command_status(&self) -> Option<&RunnerCommandStatus> {
        self.runner_command_status.as_ref()
    }

    pub(crate) fn command_install_requires_escalation(&self) -> bool {
        self.command_install_requires_escalation
    }

    pub(crate) fn command_action_target(&self) -> Option<&PathBuf> {
        self.command_action_target.as_ref()
    }

    pub(crate) fn command_install_integration(&self) -> Option<CommandInstallIntegration> {
        self.command_install_support
            .as_ref()
            .map(|support| support.integration)
    }
}

pub(crate) fn run_user_command_action(
    inputs: &CommandInstallInputs,
    action: UserCommandAction,
    integration: CommandInstallIntegration,
) -> runner_backend::error::Result<CommandActionOutcome> {
    with_command_registry(integration, |registry| {
        with_command_escalation(integration, |escalation| match action {
            UserCommandAction::Install => cli_install::install_command_explicit(
                inputs,
                registry,
                escalation,
                cli_install::force_escalated_command_install(),
            ),
            UserCommandAction::Uninstall => {
                cli_install::uninstall_command(inputs, registry, escalation)
            }
        })
    })
}

fn with_command_registry<T>(
    integration: CommandInstallIntegration,
    action: impl FnOnce(&mut dyn UserPathRegistry) -> T,
) -> T {
    if integration.system {
        #[cfg(windows)]
        {
            let mut registry = cli_install::SystemUserPathRegistry;
            action(&mut registry)
        }
        #[cfg(not(windows))]
        {
            let mut registry = NoUserPathRegistry;
            action(&mut registry)
        }
    } else {
        let mut registry = NoUserPathRegistry;
        action(&mut registry)
    }
}

fn with_command_escalation<T>(
    integration: CommandInstallIntegration,
    action: impl FnOnce(&mut dyn CommandEscalation) -> T,
) -> T {
    if integration.system {
        #[cfg(target_os = "macos")]
        {
            let mut escalation = OsascriptEscalation;
            action(&mut escalation)
        }
        #[cfg(not(target_os = "macos"))]
        {
            let mut escalation = NoEscalation;
            action(&mut escalation)
        }
    } else {
        let mut escalation = NoEscalation;
        action(&mut escalation)
    }
}

fn initialize_command_default_with(
    settings: &mut AppSettings,
    inputs: Option<&CommandInstallInputs>,
    registry: &mut dyn UserPathRegistry,
    escalation: &mut dyn CommandEscalation,
) -> runner_backend::error::Result<bool> {
    if settings.initialized_command_install {
        return Ok(false);
    }
    let Some(inputs) = inputs else {
        return Ok(false);
    };
    let outcome = cli_install::install_command_default(inputs, registry, escalation)?;
    if matches!(
        outcome,
        CommandActionOutcome::Installed(_)
            | CommandActionOutcome::AlreadyInstalled(_)
            | CommandActionOutcome::Foreign(_)
            | CommandActionOutcome::Unsupported
    ) {
        settings.initialized_command_install = true;
        return Ok(true);
    }
    Ok(false)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(unix)]
    use gpui::{AppContext as _, TestAppContext};
    use runner_backend::cli_install::{EscalationOutcome, RegistryPathValue, RegistryValueKind};
    use runner_backend::shell_path::DiscoveryOutcome;
    use runner_backend::{
        db, event_bus, events, mcp, router, session, shell_path, windows, AppCore,
    };
    use std::path::Path;
    use std::sync::{Arc, Mutex, RwLock};

    #[cfg_attr(not(unix), allow(dead_code))]
    fn test_core(temp: &Path, app_data_dir: PathBuf) -> AppCore {
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        AppCore {
            db: Arc::new(db::open_pool(&temp.join("runner.db")).unwrap()),
            app_data_dir,
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

    #[derive(Default)]
    struct FakeRegistry {
        value: Option<RegistryPathValue>,
        writes: usize,
    }

    impl UserPathRegistry for FakeRegistry {
        fn read_path(&self) -> runner_backend::error::Result<Option<RegistryPathValue>> {
            Ok(self.value.clone())
        }

        fn read_machine_path(&self) -> runner_backend::error::Result<Option<RegistryPathValue>> {
            Ok(None)
        }

        fn write_path(&mut self, value: &RegistryPathValue) -> runner_backend::error::Result<()> {
            self.value = Some(value.clone());
            self.writes += 1;
            Ok(())
        }

        fn broadcast_environment_change(&mut self) -> runner_backend::error::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeEscalation {
        calls: usize,
    }

    impl CommandEscalation for FakeEscalation {
        fn run(&mut self, _apple_script: &str) -> runner_backend::error::Result<EscalationOutcome> {
            self.calls += 1;
            Ok(EscalationOutcome::Completed)
        }
    }

    fn inputs(temp: &Path, path: String) -> CommandInstallInputs {
        CommandInstallInputs {
            home: temp.join("home"),
            login_path: path,
            system_path: String::new(),
            sidecar: temp.join("app/bin/runner"),
            local_bin: temp.join("home/.local/bin"),
            system_bin: temp.join("usr/local/bin"),
            system_bin_writable: false,
            debug: false,
            platform: CommandPlatform::Unix,
        }
    }

    #[cfg(unix)]
    #[test]
    fn default_waits_records_success_and_never_escalates() {
        let temp = tempfile::tempdir().unwrap();
        let local = temp.path().join("home/.local/bin");
        let inputs = inputs(temp.path(), local.display().to_string());
        std::fs::create_dir_all(inputs.sidecar.parent().unwrap()).unwrap();
        std::fs::write(&inputs.sidecar, "sidecar").unwrap();
        let mut settings = AppSettings::default();
        let mut registry = FakeRegistry::default();
        let mut escalation = FakeEscalation::default();

        assert!(!initialize_command_default_with(
            &mut settings,
            None,
            &mut registry,
            &mut escalation
        )
        .unwrap());
        assert!(!settings.initialized_command_install);
        assert!(initialize_command_default_with(
            &mut settings,
            Some(&inputs),
            &mut registry,
            &mut escalation
        )
        .unwrap());
        assert!(settings.initialized_command_install);
        assert_eq!(escalation.calls, 0);
        std::fs::remove_file(local.join("runner")).unwrap();
        assert!(!initialize_command_default_with(
            &mut settings,
            Some(&inputs),
            &mut registry,
            &mut escalation
        )
        .unwrap());
        assert!(!local.join("runner").exists());
    }

    #[cfg(unix)]
    #[test]
    fn store_default_waits_for_successful_discovery_with_a_path() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("home");
        let app_data = temp.path().join("app-data");
        let local_bin = home.join(".local/bin");
        let sidecar = agent_skill::sidecar_path(&app_data);
        std::fs::create_dir_all(sidecar.parent().unwrap()).unwrap();
        std::fs::write(&sidecar, "sidecar").unwrap();
        let core = test_core(temp.path(), app_data);
        let discovery = Arc::clone(&core.runtime_discovery);
        let mut cx = TestAppContext::single();
        let store = cx.new(|cx| {
            AppStore::new(
                core,
                Some(home.clone()),
                Some(CommandInstallSupport::isolated(
                    temp.path().join("system-bin"),
                )),
                temp.path().join("settings.json"),
                AppSettings::default(),
                None,
                cx,
            )
        });

        assert!(!store.read_with(&cx, |store, _| store.settings.initialized_command_install));
        assert!(!local_bin.join("runner-dev").exists());

        {
            let mut state = discovery.write().unwrap();
            state.checking = false;
            state.result = Some(shell_path::DiscoveryResult {
                shell: None,
                outcome: DiscoveryOutcome::Timeout,
                duration_ms: 0,
                env: shell_path::LoginShellEnv {
                    path: Some(local_bin.display().to_string()),
                    ..Default::default()
                },
            });
        }
        store.update(&mut cx, |store, _| store.initialize_command_default());
        assert!(!store.read_with(&cx, |store, _| store.settings.initialized_command_install));
        assert!(!local_bin.join("runner-dev").exists());

        {
            let mut state = discovery.write().unwrap();
            state.result = Some(shell_path::DiscoveryResult {
                shell: None,
                outcome: DiscoveryOutcome::Ok,
                duration_ms: 0,
                env: shell_path::LoginShellEnv::default(),
            });
        }
        store.update(&mut cx, |store, _| store.initialize_command_default());
        assert!(!store.read_with(&cx, |store, _| store.settings.initialized_command_install));
        assert!(!local_bin.join("runner-dev").exists());

        {
            let mut state = discovery.write().unwrap();
            state.result = Some(shell_path::DiscoveryResult {
                shell: None,
                outcome: DiscoveryOutcome::Ok,
                duration_ms: 0,
                env: shell_path::LoginShellEnv {
                    path: Some(local_bin.display().to_string()),
                    ..Default::default()
                },
            });
        }
        store.update(&mut cx, |store, _| store.initialize_command_default());

        assert!(store.read_with(&cx, |store, _| store.settings.initialized_command_install));
        assert_eq!(
            store.read_with(&cx, |store, _| store
                .runner_command_status()
                .map(|status| status.state.clone())),
            Some(runner_backend::cli_install::RunnerCommandState::Installed)
        );
        assert_eq!(
            std::fs::read_link(local_bin.join("runner-dev")).unwrap(),
            sidecar
        );
    }

    #[test]
    fn no_target_is_not_recorded_and_windows_preserves_registry_kind() {
        let temp = tempfile::tempdir().unwrap();
        let mut settings = AppSettings::default();
        let mut registry = FakeRegistry::default();
        let mut escalation = FakeEscalation::default();
        let no_target = inputs(temp.path(), "/usr/bin".into());
        assert!(!initialize_command_default_with(
            &mut settings,
            Some(&no_target),
            &mut registry,
            &mut escalation
        )
        .unwrap());
        assert!(!settings.initialized_command_install);

        // A Unix PATH is split on ':' and the install needs a symlink, so this half is Unix-only.
        #[cfg(unix)]
        {
            let mut later = no_target.clone();
            later.login_path = later.local_bin.display().to_string();
            std::fs::create_dir_all(later.sidecar.parent().unwrap()).unwrap();
            std::fs::write(&later.sidecar, "sidecar").unwrap();
            assert!(initialize_command_default_with(
                &mut settings,
                Some(&later),
                &mut registry,
                &mut escalation
            )
            .unwrap());
            assert!(settings.initialized_command_install);
        }

        let mut settings = AppSettings::default();
        let mut windows = inputs(temp.path(), String::new());
        windows.platform = CommandPlatform::Windows;
        windows.sidecar = PathBuf::from(r"C:\Runner\bin\runner.exe");
        registry.value = Some(RegistryPathValue {
            value: r"C:\Tools".into(),
            kind: RegistryValueKind::ExpandString,
        });
        assert!(initialize_command_default_with(
            &mut settings,
            Some(&windows),
            &mut registry,
            &mut escalation
        )
        .unwrap());
        assert_eq!(registry.writes, 1);
        assert_eq!(
            registry.value.unwrap().kind,
            RegistryValueKind::ExpandString
        );
    }

    #[cfg(unix)]
    #[test]
    fn explicit_uninstall_record_prevents_default_resurrection() {
        let temp = tempfile::tempdir().unwrap();
        let local = temp.path().join("home/.local/bin");
        let inputs = inputs(temp.path(), local.display().to_string());
        std::fs::create_dir_all(inputs.sidecar.parent().unwrap()).unwrap();
        std::fs::write(&inputs.sidecar, "sidecar").unwrap();
        std::fs::create_dir_all(&local).unwrap();
        std::os::unix::fs::symlink(&inputs.sidecar, local.join("runner")).unwrap();
        let mut settings = AppSettings::default();
        assert!(mark_command_install_initialized(&mut settings));
        let mut registry = FakeRegistry::default();
        let mut escalation = FakeEscalation::default();

        assert!(matches!(
            cli_install::uninstall_command(&inputs, &mut registry, &mut escalation).unwrap(),
            CommandActionOutcome::Removed(_)
        ));
        assert!(!initialize_command_default_with(
            &mut settings,
            Some(&inputs),
            &mut registry,
            &mut escalation
        )
        .unwrap());
        assert!(!local.join("runner").exists());
    }
}
