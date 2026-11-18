use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use super::AppStore;
use crate::app_settings::AppSettings;
use runner_core::protocol::agent_skill;
use runner_core::protocol::command::{
    CommandActionOutcome, CommandInstallInputs, CommandPlatform, RunnerCommandStatus,
};
#[cfg(test)]
use runner_daemon::cli_install::{CommandEscalation, UserPathRegistry};

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
    pub(crate) fn isolated(system_bin: PathBuf) -> Self {
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

pub(crate) struct CommandDefaultSnapshot {
    pub(super) seq: u64,
    pub(super) newly_initialized: bool,
    pub(super) status: Option<(RunnerCommandStatus, bool, PathBuf)>,
}

pub(super) fn compute_command_install_inputs(
    client: &runner_core::protocol::DaemonClient,
    home: Option<&std::path::Path>,
    app_data_dir: &std::path::Path,
    support: Option<&CommandInstallSupport>,
    require_finished_discovery: bool,
) -> Option<CommandInstallInputs> {
    let support = support?;
    let home = home?.to_path_buf();
    let debug = cfg!(debug_assertions);
    #[cfg(windows)]
    let platform = CommandPlatform::Windows;
    #[cfg(not(windows))]
    let platform = CommandPlatform::Unix;

    let login_path = if platform == CommandPlatform::Unix {
        let discovery = client.discovery_snapshot().ok()?;
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
            .or_else(|| discovery.shell_env.path.clone())
            .unwrap_or_default()
    } else {
        std::env::var("PATH").unwrap_or_default()
    };
    let sidecar = agent_skill::sidecar_path(app_data_dir);
    let local_bin = home.join(".local/bin");
    let system_bin = support.system_bin.clone();
    Some(CommandInstallInputs {
        home,
        login_path,
        system_path: std::env::var("PATH").unwrap_or_default(),
        sidecar,
        local_bin,
        system_bin: system_bin.clone(),
        system_bin_writable: client
            .command_directory_writable(&system_bin)
            .unwrap_or(false),
        debug,
        platform,
    })
}

pub(super) fn reconcile_command_default_serialized(
    client: &runner_core::protocol::DaemonClient,
    home: Option<&std::path::Path>,
    app_data_dir: &std::path::Path,
    support: Option<&CommandInstallSupport>,
    initialized_flag: &AtomicBool,
    seq_counter: &AtomicU64,
    lock: &Mutex<()>,
) -> CommandDefaultSnapshot {
    let default_inputs = (!initialized_flag.load(Ordering::Acquire))
        .then(|| compute_command_install_inputs(client, home, app_data_dir, support, true))
        .flatten();
    let status_inputs = compute_command_install_inputs(client, home, app_data_dir, support, false);

    let _guard = lock.lock().unwrap();
    let mut newly_initialized = false;
    if !initialized_flag.load(Ordering::Acquire) {
        if let Some(inputs) = default_inputs {
            let integration = support.expect("command inputs require support").integration;
            match client.command_install_default(&inputs, integration.system) {
                Ok(
                    CommandActionOutcome::Installed(_)
                    | CommandActionOutcome::AlreadyInstalled(_)
                    | CommandActionOutcome::Foreign(_)
                    | CommandActionOutcome::Unsupported,
                ) => {
                    initialized_flag.store(true, Ordering::Release);
                    newly_initialized = true;
                }
                Ok(_) => {}
                Err(error) => eprintln!("Runner command default failed: {error}"),
            }
        }
    }
    let status = status_inputs.and_then(|inputs| {
        let integration = support.expect("command inputs require support").integration;
        match client.command_status(&inputs, integration.system) {
            Ok(status) => Some(status),
            Err(error) => {
                eprintln!("Runner command status failed: {error}");
                None
            }
        }
    });
    let seq = seq_counter.fetch_add(1, Ordering::AcqRel).wrapping_add(1);
    CommandDefaultSnapshot {
        seq,
        newly_initialized,
        status,
    }
}

pub(crate) struct CommandActionContext {
    pub(crate) client: runner_core::protocol::DaemonClient,
    pub(crate) home_dir: Option<PathBuf>,
    pub(crate) app_data_dir: PathBuf,
    pub(crate) support: Option<CommandInstallSupport>,
    pub(crate) seq_counter: Arc<AtomicU64>,
    pub(crate) lock: Arc<Mutex<()>>,
}

pub(crate) fn run_user_command_action_serialized(
    ctx: &CommandActionContext,
    action: UserCommandAction,
) -> Option<(
    Result<CommandActionOutcome, runner_core::protocol::ClientError>,
    CommandDefaultSnapshot,
)> {
    let inputs = compute_command_install_inputs(
        &ctx.client,
        ctx.home_dir.as_deref(),
        &ctx.app_data_dir,
        ctx.support.as_ref(),
        false,
    )?;
    let integration = ctx.support.as_ref()?.integration;
    let _guard = ctx.lock.lock().unwrap();
    let result = run_user_command_action(&ctx.client, &inputs, action, integration);
    let status = match ctx.client.command_status(&inputs, integration.system) {
        Ok(status) => Some(status),
        Err(error) => {
            eprintln!("Runner command status failed: {error}");
            None
        }
    };
    let seq = ctx
        .seq_counter
        .fetch_add(1, Ordering::AcqRel)
        .wrapping_add(1);
    Some((
        result,
        CommandDefaultSnapshot {
            seq,
            newly_initialized: false,
            status,
        },
    ))
}

impl AppStore {
    pub(crate) fn apply_command_default(&mut self, snapshot: CommandDefaultSnapshot) -> bool {
        if snapshot.newly_initialized && !self.settings.initialized_command_install {
            self.settings.initialized_command_install = true;
            self.command_install_initialized
                .store(true, Ordering::Release);
            self.save_settings();
        }
        if snapshot.seq < self.applied_command_status_seq {
            return false;
        }
        self.applied_command_status_seq = snapshot.seq;
        match snapshot.status {
            Some((status, requires_escalation, target)) => {
                self.runner_command_status = Some(status);
                self.command_install_requires_escalation = requires_escalation;
                self.command_action_target = Some(target);
            }
            None => {
                self.runner_command_status = None;
                self.command_install_requires_escalation = false;
                self.command_action_target = None;
            }
        }
        true
    }

    pub(crate) fn initialize_command_default(&mut self) {
        self.command_install_initialized
            .store(self.settings.initialized_command_install, Ordering::Release);
        let snapshot = reconcile_command_default_serialized(
            &self.client,
            self.home_dir.as_deref(),
            &self.app_data_dir,
            self.command_install_support.as_ref(),
            &self.command_install_initialized,
            &self.command_status_seq,
            &self.command_reconcile_lock,
        );
        self.apply_command_default(snapshot);
    }

    pub(crate) fn command_action_context(&self) -> CommandActionContext {
        CommandActionContext {
            client: self.client.clone(),
            home_dir: self.home_dir.clone(),
            app_data_dir: self.app_data_dir.clone(),
            support: self.command_install_support.clone(),
            seq_counter: Arc::clone(&self.command_status_seq),
            lock: Arc::clone(&self.command_reconcile_lock),
        }
    }

    pub(crate) fn command_install_inputs(
        &self,
        require_finished_discovery: bool,
    ) -> Option<CommandInstallInputs> {
        compute_command_install_inputs(
            &self.client,
            self.home_dir.as_deref(),
            &self.app_data_dir,
            self.command_install_support.as_ref(),
            require_finished_discovery,
        )
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

        let result = self.client.command_status(&inputs, integration.system);
        match result {
            Ok((status, requires_escalation, target)) => {
                self.runner_command_status = Some(status);
                self.command_install_requires_escalation = requires_escalation;
                self.command_action_target = Some(target);
            }
            Err(error) => eprintln!("Runner command status failed: {error}"),
        }
    }
}

impl super::AppStore {
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
    client: &runner_core::protocol::DaemonClient,
    inputs: &CommandInstallInputs,
    action: UserCommandAction,
    integration: CommandInstallIntegration,
) -> Result<CommandActionOutcome, runner_core::protocol::ClientError> {
    client.command_action(
        inputs,
        action == UserCommandAction::Install,
        integration.system,
    )
}

#[cfg(test)]
#[cfg(test)]
#[cfg(test)]
fn initialize_command_default_with(
    settings: &mut AppSettings,
    inputs: Option<&CommandInstallInputs>,
    registry: &mut dyn UserPathRegistry,
    escalation: &mut dyn CommandEscalation,
) -> runner_daemon::error::Result<bool> {
    if settings.initialized_command_install {
        return Ok(false);
    }
    let Some(inputs) = inputs else {
        return Ok(false);
    };
    let outcome =
        runner_daemon::cli_install::install_command_default(inputs, registry, escalation)?;
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
    use crate::app_store::AppStore;
    #[cfg(unix)]
    use gpui::{AppContext as _, TestAppContext};
    #[cfg(unix)]
    use runner_core::protocol::discovery::DiscoveryOutcome;
    use runner_daemon::cli_install::{EscalationOutcome, RegistryPathValue, RegistryValueKind};
    use runner_daemon::{db, session, shell_path, AppCore};
    use std::path::Path;
    use std::sync::{Arc, RwLock};

    #[cfg_attr(not(unix), allow(dead_code))]
    fn test_core(temp: &Path, app_data_dir: PathBuf) -> AppCore {
        let runtime_shell_env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let runtime_discovery =
            Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        crate::test_support::core(
            Arc::new(db::open_fixture_pool(&temp.join("runner.db")).unwrap()),
            app_data_dir,
            session::SessionManager::new(
                Arc::clone(&runtime_shell_env),
                Arc::clone(&runtime_discovery),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            runtime_shell_env,
            runtime_discovery,
        )
    }

    #[derive(Default)]
    struct FakeRegistry {
        value: Option<RegistryPathValue>,
        writes: usize,
    }

    impl UserPathRegistry for FakeRegistry {
        fn read_path(&self) -> runner_daemon::error::Result<Option<RegistryPathValue>> {
            Ok(self.value.clone())
        }

        fn read_machine_path(&self) -> runner_daemon::error::Result<Option<RegistryPathValue>> {
            Ok(None)
        }

        fn write_path(&mut self, value: &RegistryPathValue) -> runner_daemon::error::Result<()> {
            self.value = Some(value.clone());
            self.writes += 1;
            Ok(())
        }

        fn broadcast_environment_change(&mut self) -> runner_daemon::error::Result<()> {
            Ok(())
        }
    }

    #[derive(Default)]
    struct FakeEscalation {
        calls: usize,
    }

    impl CommandEscalation for FakeEscalation {
        fn run(&mut self, _apple_script: &str) -> runner_daemon::error::Result<EscalationOutcome> {
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
            Some(runner_core::command_install::RunnerCommandState::Installed)
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
            runner_daemon::cli_install::uninstall_command(&inputs, &mut registry, &mut escalation)
                .unwrap(),
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
