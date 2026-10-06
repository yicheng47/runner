use std::collections::BTreeMap;
use std::hash::{Hash as _, Hasher as _};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use futures::{FutureExt as _, StreamExt as _};
use gpui::{App, AppContext as _, Context, Entity, Global};
use runner_core::protocol::crew::CrewListItem;
use runner_core::protocol::mission::MissionSummary;
use runner_core::protocol::model::Role;
use runner_core::protocol::node::NodeRow;
use runner_core::protocol::project::ProjectRow;
use runner_core::protocol::session::DirectSessionEntry;
use runner_core::protocol::session::SessionActivityState;
use runner_core::protocol::ClientEvent as AppEvent;
use runner_core::protocol::DaemonClient;
use runner_terminal::terminal::TerminalBridge;

use crate::app_settings::{
    AppSettings, DarkTerminalTheme, LightTerminalTheme, TerminalCursorStyle, TerminalFontFamily,
};

mod command_default;
mod mcp_removal;
mod skill_defaults;
pub(crate) use command_default::{
    mark_command_install_initialized, run_user_command_action, CommandInstallSupport,
    UserCommandAction,
};
pub(crate) use skill_defaults::RunnerSkillStatus;

#[derive(Clone)]
pub(crate) struct GlobalAppStore(pub(crate) Entity<AppStore>);

impl Global for GlobalAppStore {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoreRefreshKind {
    Activity,
    Nodes,
    Missions,
    Runtimes,
    All,
}

impl StoreRefreshKind {
    pub(crate) fn for_event(event: &AppEvent) -> Option<Self> {
        match event.name.as_str() {
            "runtime/changed" => Some(Self::Runtimes),
            "usage/updated" | "window_focus_map" | "session/status" => Some(Self::Activity),
            "chat/tab-attention-changed" | "chat/layout-changed" => Some(Self::Nodes),
            "event/appended"
                if event
                    .payload
                    .get("event")
                    .and_then(|event| event.get("type"))
                    .and_then(serde_json::Value::as_str)
                    .is_some_and(|signal| {
                        matches!(
                            signal,
                            "mission_start"
                                | "mission_stopped"
                                | "ask_human"
                                | "human_question"
                                | "human_response"
                                | "session_status"
                                | "runner_status"
                        )
                    }) =>
            {
                Some(Self::Missions)
            }
            "session/exit"
            | "session/spawned"
            | "session/fork-started"
            | "session/archived"
            | "session/updated"
            | "role/activity"
            | "role/changed"
            | "crew/changed"
            | "slot/changed"
            | "mission/changed"
            | "project/changed" => Some(Self::All),
            _ => None,
        }
    }

    pub(crate) fn merge(self, other: Self) -> Self {
        if self == other {
            return self;
        }
        Self::All
    }
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct StoreRevisions {
    pub(crate) terminal_wake: u64,
    pub(crate) sessions: u64,
    pub(crate) roles: u64,
    pub(crate) role_surfaces: u64,
    pub(crate) crews: u64,
    pub(crate) nodes: u64,
    pub(crate) tab_rows: u64,
    pub(crate) projects: u64,
    pub(crate) missions: u64,
    pub(crate) activity: u64,
    pub(crate) settings: u64,
    pub(crate) terminal_settings: u64,
    pub(crate) mission_settings: u64,
    pub(crate) shell_settings: u64,
    pub(crate) full_refresh: u64,
    pub(crate) error: u64,
    pub(crate) restart: u64,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct StoreReactions {
    pub(crate) sync_error: bool,
    pub(crate) reload_tabs: bool,
    pub(crate) prune_sidebar: bool,
    pub(crate) prune_window_state: bool,
    pub(crate) reload_role_surfaces: bool,
    pub(crate) reload_crew_surfaces: bool,
    pub(crate) apply_terminal_settings: bool,
    pub(crate) mission_settings: bool,
    pub(crate) shell_settings: bool,
    pub(crate) notify: bool,
    pub(crate) notify_without_settings: bool,
    pub(crate) terminal_wake: bool,
}

impl StoreReactions {
    pub(crate) fn notify_shell(self) -> bool {
        self.notify_without_settings || self.shell_settings
    }
}

impl StoreRevisions {
    pub(crate) fn reactions_since(self, previous: Self) -> StoreReactions {
        let mut data_revisions = self;
        data_revisions.terminal_wake = previous.terminal_wake;
        let mut non_settings_revisions = data_revisions;
        non_settings_revisions.settings = previous.settings;
        non_settings_revisions.mission_settings = previous.mission_settings;
        non_settings_revisions.shell_settings = previous.shell_settings;
        StoreReactions {
            sync_error: self.error != previous.error,
            reload_tabs: self.tab_rows != previous.tab_rows,
            prune_sidebar: self.projects != previous.projects,
            prune_window_state: self.full_refresh != previous.full_refresh,
            reload_role_surfaces: self.role_surfaces != previous.role_surfaces,
            reload_crew_surfaces: self.crews != previous.crews,
            apply_terminal_settings: self.terminal_settings != previous.terminal_settings,
            mission_settings: self.mission_settings != previous.mission_settings,
            shell_settings: self.shell_settings != previous.shell_settings,
            notify: data_revisions != previous,
            notify_without_settings: non_settings_revisions != previous,
            terminal_wake: self.terminal_wake != previous.terminal_wake,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TerminalSettingsSnapshot {
    light_theme: LightTerminalTheme,
    dark_theme: DarkTerminalTheme,
    is_light: bool,
    font_family: TerminalFontFamily,
    font_size: u16,
    cursor_style: TerminalCursorStyle,
}

impl From<&AppSettings> for TerminalSettingsSnapshot {
    fn from(settings: &AppSettings) -> Self {
        Self::for_variant(settings, crate::theme::active_variant())
    }
}

impl TerminalSettingsSnapshot {
    fn for_variant(settings: &AppSettings, variant: crate::theme::ThemeVariant) -> Self {
        Self {
            light_theme: settings.light_terminal_theme,
            dark_theme: settings.dark_terminal_theme,
            is_light: variant.is_light(),
            font_family: settings.terminal_font_family,
            font_size: settings.terminal_font_size,
            cursor_style: settings.terminal_cursor_style,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MissionSettingsSnapshot {
    rail_open: bool,
    rail_width: u32,
    fingerprint: u64,
}

impl From<&AppSettings> for MissionSettingsSnapshot {
    fn from(settings: &AppSettings) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        settings.last_mission_terminal_ids.hash(&mut hasher);
        Self {
            rail_open: settings.mission_rail_open,
            rail_width: settings.mission_rail_width.to_bits(),
            fingerprint: hasher.finish(),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ShellSettingsSnapshot(u64);

impl From<&AppSettings> for ShellSettingsSnapshot {
    fn from(settings: &AppSettings) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        std::mem::discriminant(&settings.app_theme).hash(&mut hasher);
        std::mem::discriminant(&settings.light_app_theme).hash(&mut hasher);
        std::mem::discriminant(&settings.dark_app_theme).hash(&mut hasher);
        settings.app_zoom.to_bits().hash(&mut hasher);
        std::mem::discriminant(&settings.light_terminal_theme).hash(&mut hasher);
        std::mem::discriminant(&settings.dark_terminal_theme).hash(&mut hasher);
        std::mem::discriminant(&settings.terminal_font_family).hash(&mut hasher);
        settings.terminal_font_size.hash(&mut hasher);
        std::mem::discriminant(&settings.terminal_cursor_style).hash(&mut hasher);
        settings.sidebar_width.to_bits().hash(&mut hasher);
        settings.sidebar_projects_open.hash(&mut hasher);
        settings.sidebar_chats_open.hash(&mut hasher);
        settings.sidebar_collapsed_projects.hash(&mut hasher);
        settings.chat_panel_open.hash(&mut hasher);
        settings.chat_panel_width.to_bits().hash(&mut hasher);
        settings.default_crew_id.hash(&mut hasher);
        settings.default_working_dir.hash(&mut hasher);
        settings.resume_on_launch.hash(&mut hasher);
        settings.default_runtime.hash(&mut hasher);
        settings.disabled_agents.hash(&mut hasher);
        settings.enabled_agents.hash(&mut hasher);
        settings.runner_skill_enabled.hash(&mut hasher);
        settings.keymap_overrides.hash(&mut hasher);
        Self(hasher.finish())
    }
}

pub(crate) struct AppStore {
    pub(crate) client: DaemonClient,
    pub(crate) app_data_dir: PathBuf,
    pub(crate) window_entries: Vec<runner_core::protocol::WindowEntry>,
    pub(crate) bridge: Arc<TerminalBridge>,
    #[cfg(test)]
    pub(crate) test_core: runner_daemon::AppCore,
    pub(crate) sessions: Vec<DirectSessionEntry>,
    pub(crate) session_details: BTreeMap<String, DirectSessionEntry>,
    pub(crate) roles: Vec<Role>,
    pub(crate) crews: Vec<CrewListItem>,
    pub(crate) nodes: Vec<NodeRow>,
    pub(crate) projects: Vec<ProjectRow>,
    pub(crate) missions: Vec<MissionSummary>,
    pub(crate) session_statuses: BTreeMap<String, runner_core::protocol::status::AgentStatus>,
    pub(crate) session_activity: BTreeMap<String, SessionActivityState>,
    #[cfg(any(windows, test))]
    mission_agent_ids: std::collections::BTreeSet<String>,
    pub(crate) live_session_count: usize,
    pub(crate) settings: AppSettings,
    settings_path: PathBuf,
    pub(crate) home_dir: Option<PathBuf>,
    pub(crate) usage: runner_core::protocol::UsageSnapshot,
    pub(crate) file_link_environment: (String, std::collections::HashMap<String, bool>),
    command_install_support: Option<CommandInstallSupport>,
    runner_command_status: Option<runner_core::command_install::RunnerCommandStatus>,
    command_install_requires_escalation: bool,
    command_action_target: Option<PathBuf>,
    runner_skill_status: RunnerSkillStatus,
    pub(crate) revisions: StoreRevisions,
    pub(crate) error: Option<String>,
    collecting_startup_errors: bool,
    pub(crate) daemon_disconnected: bool,
    pub(crate) daemon_notice: Option<runner_app::lifecycle::DaemonNotice>,
    pub(crate) stopped_session_count: usize,
    pub(crate) restarted_sessions: Option<usize>,
}

impl AppStore {
    pub(crate) fn new(
        #[cfg(not(test))] host: runner_app::bootstrap::ClientHost,
        #[cfg(test)] core: runner_daemon::AppCore,
        home_dir: Option<PathBuf>,
        command_install_support: Option<CommandInstallSupport>,
        settings_path: PathBuf,
        settings: AppSettings,
        settings_error: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        #[cfg(test)]
        let host = runner_app::bootstrap::ClientHost {
            client: crate::test_support::client(&core),
            app_data_dir: core.app_data_dir.clone(),
        };
        let (wake_tx, mut wake_rx) = futures::channel::mpsc::unbounded::<()>();
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = wake_tx.unbounded_send(());
        });
        let bridge = TerminalBridge::new(host.client.clone(), Arc::clone(&waker))
            .expect("terminal event bridge installation is infallible");

        cx.spawn(async move |weak, cx| {
            while wake_rx.next().await.is_some() {
                let delay = cx
                    .background_executor()
                    .timer(Duration::from_millis(4))
                    .fuse();
                futures::pin_mut!(delay);
                loop {
                    futures::select_biased! {
                        _ = delay => break,
                        wake = wake_rx.next().fuse() => {
                            if wake.is_none() {
                                break;
                            }
                        }
                    }
                }
                if weak
                    .update(cx, |this, cx| {
                        this.revisions.terminal_wake = this.revisions.terminal_wake.wrapping_add(1);
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        let (event_tx, mut event_rx) = futures::channel::mpsc::unbounded::<(
            StoreRefreshKind,
            EntityRefreshKind,
            Option<DaemonEvent>,
        )>();
        #[cfg(not(test))]
        let client = host.client.clone();
        #[cfg(test)]
        let client = crate::test_support::client(&core);
        let mut events = client.subscribe();
        cx.background_spawn(async move {
            loop {
                let refresh = match events.recv().await {
                    Ok(event) if event.name == "daemon/disconnected" => Some((
                        StoreRefreshKind::All,
                        EntityRefreshKind::All,
                        Some(DaemonEvent::Disconnected(
                            if event.payload["restart_limit"].as_bool().unwrap_or(false) {
                                runner_app::lifecycle::DaemonNotice::Repeated
                            } else {
                                runner_app::lifecycle::DaemonNotice::Stopped
                            },
                        )),
                    )),
                    Ok(event) if event.name == "daemon/reconnected" => Some((
                        StoreRefreshKind::All,
                        EntityRefreshKind::All,
                        Some(DaemonEvent::Reconnected),
                    )),
                    Ok(event) if event.name == "daemon/restarted" => Some((
                        StoreRefreshKind::All,
                        EntityRefreshKind::All,
                        Some(DaemonEvent::Restarted(
                            event.payload["count"].as_u64().unwrap_or(0) as usize,
                        )),
                    )),
                    Ok(event) => StoreRefreshKind::for_event(&event)
                        .map(|store| (store, EntityRefreshKind::for_event(&event), None)),
                    Err(runner_core::protocol::EventError::Lagged(_)) => {
                        Some((StoreRefreshKind::All, EntityRefreshKind::All, None))
                    }
                    Err(runner_core::protocol::EventError::Closed) => break,
                };
                if refresh.is_some_and(|refresh| event_tx.unbounded_send(refresh).is_err()) {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |weak, cx| {
            while let Some((mut refresh, mut entity_refresh, event)) = event_rx.next().await {
                let mut daemon_events = event.into_iter().collect::<Vec<_>>();
                while let Ok(next) = event_rx.try_recv() {
                    refresh = refresh.merge(next.0);
                    entity_refresh = entity_refresh.merge(next.1);
                    daemon_events.extend(next.2);
                }
                if weak
                    .update(cx, |this, cx| {
                        for event in daemon_events {
                            match event {
                                DaemonEvent::Disconnected(notice) => {
                                    this.daemon_disconnected = true;
                                    this.daemon_notice = Some(notice);
                                    this.error = None;
                                    this.revisions.error = this.revisions.error.wrapping_add(1);
                                }
                                DaemonEvent::Reconnected => {
                                    this.daemon_disconnected = false;
                                    if this.daemon_notice
                                        == Some(runner_app::lifecycle::DaemonNotice::Repeated)
                                    {
                                        this.daemon_notice = None;
                                        this.error = None;
                                    } else if this.daemon_notice
                                        == Some(runner_app::lifecycle::DaemonNotice::Stopped)
                                    {
                                        this.error =
                                            Some(runner_app::lifecycle::crash_recovery_message(
                                                this.stopped_session_count,
                                            ));
                                    }
                                    this.revisions.error = this.revisions.error.wrapping_add(1);
                                }
                                DaemonEvent::Restarted(count) => {
                                    this.restarted_sessions = Some(count);
                                    this.revisions.restart = this.revisions.restart.wrapping_add(1);
                                }
                            }
                        }
                        this.refresh(refresh, cx);
                        if entity_refresh.roles() {
                            this.revisions.role_surfaces =
                                this.revisions.role_surfaces.wrapping_add(1);
                        }
                        if entity_refresh.crews() {
                            this.refresh_crews_inner();
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        })
        .detach();

        if let Err(error) = bridge.attach_live_sessions() {
            eprintln!("attach live terminals: {error}");
        }
        let mut store = Self {
            app_data_dir: host.app_data_dir.clone(),
            client,
            window_entries: Vec::new(),
            bridge,
            #[cfg(test)]
            test_core: core.clone(),
            sessions: Vec::new(),
            session_details: BTreeMap::new(),
            roles: Vec::new(),
            crews: Vec::new(),
            nodes: Vec::new(),
            projects: Vec::new(),
            missions: Vec::new(),
            session_statuses: BTreeMap::new(),
            session_activity: BTreeMap::new(),
            #[cfg(any(windows, test))]
            mission_agent_ids: Default::default(),
            live_session_count: 0,
            settings,
            settings_path,
            home_dir,
            usage: Default::default(),
            file_link_environment: Default::default(),
            command_install_support,
            runner_command_status: None,
            command_install_requires_escalation: false,
            command_action_target: None,
            runner_skill_status: RunnerSkillStatus::default(),
            revisions: StoreRevisions::default(),
            error: None,
            collecting_startup_errors: true,
            daemon_disconnected: false,
            daemon_notice: None,
            stopped_session_count: 0,
            restarted_sessions: None,
        };
        if let Some(error) = settings_error {
            store.record_error(error);
        } else {
            store.initialize_mcp_removal();
            store.initialize_skill_defaults();
            store.initialize_command_default();
        }
        store.refresh_sessions_inner();
        store.refresh_roles_inner();
        store.refresh_crews_inner();
        store.refresh_nodes_inner();
        store.refresh_projects_inner();
        store.refresh_missions_blocking_inner();
        store.refresh_activity_inner();
        store.refresh_render_snapshots();
        store.collecting_startup_errors = false;
        store
    }

    pub(crate) fn refresh(&mut self, refresh: StoreRefreshKind, cx: &mut Context<Self>) {
        if matches!(refresh, StoreRefreshKind::Runtimes | StoreRefreshKind::All) {
            self.initialize_mcp_removal();
            self.initialize_skill_defaults();
            self.initialize_command_default();
        }
        if matches!(refresh, StoreRefreshKind::Activity | StoreRefreshKind::All) {
            self.refresh_activity_inner();
        }
        if matches!(refresh, StoreRefreshKind::Nodes | StoreRefreshKind::All) {
            if refresh == StoreRefreshKind::All {
                self.refresh_sessions_inner();
            }
            self.refresh_nodes_inner();
        }
        if refresh == StoreRefreshKind::All {
            self.refresh_projects_inner();
            self.revisions.full_refresh = self.revisions.full_refresh.wrapping_add(1);
        }
        if matches!(refresh, StoreRefreshKind::Missions | StoreRefreshKind::All) {
            let core = self.client.clone();
            cx.spawn(async move |weak, cx| {
                let result = core.mission_list_summary_impl(None);
                let _ = weak.update(cx, |this, cx| {
                    match result {
                        Ok(missions) => {
                            #[cfg(any(windows, test))]
                            match mission_agent_ids(&core, &missions) {
                                Ok(ids) => this.mission_agent_ids = ids,
                                Err(error) => this.record_error(error.to_string()),
                            }
                            this.missions = missions;
                            this.revisions.missions = this.revisions.missions.wrapping_add(1);
                        }
                        Err(error) => this.record_error(error.to_string()),
                    }
                    cx.notify();
                });
            })
            .detach();
        }
        self.refresh_render_snapshots();
        cx.notify();
    }

    pub(crate) fn refresh_sessions(&mut self, cx: &mut Context<Self>) {
        self.refresh_sessions_inner();
        cx.notify();
    }

    pub(crate) fn refresh_nodes(
        &mut self,
        cx: &mut Context<Self>,
    ) -> std::result::Result<(), runner_core::protocol::ClientError> {
        match self.client.node_list() {
            Ok(nodes) => {
                self.nodes = nodes;
                self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
                self.revisions.tab_rows = self.revisions.tab_rows.wrapping_add(1);
                cx.notify();
                Ok(())
            }
            Err(error) => Err(error),
        }
    }

    pub(crate) fn replace_roles(&mut self, roles: Vec<Role>, cx: &mut Context<Self>) {
        self.roles = roles;
        self.revisions.roles = self.revisions.roles.wrapping_add(1);
        cx.notify();
    }

    pub(crate) fn replace_nodes(&mut self, nodes: Vec<NodeRow>, cx: &mut Context<Self>) {
        self.nodes = nodes;
        self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
        self.revisions.tab_rows = self.revisions.tab_rows.wrapping_add(1);
        cx.notify();
    }

    pub(crate) fn replace_node(&mut self, node: NodeRow, cx: &mut Context<Self>) {
        if let Some(current) = self.nodes.iter_mut().find(|current| current.id == node.id) {
            *current = node;
        }
        self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
        cx.notify();
    }

    pub(crate) fn remove_session_activity(&mut self, session_id: &str, cx: &mut Context<Self>) {
        self.session_activity.remove(session_id);
        self.session_statuses.remove(session_id);
        self.revisions.activity = self.revisions.activity.wrapping_add(1);
        cx.notify();
    }

    pub(crate) fn set_session_pinned(
        &mut self,
        session_id: &str,
        pinned: bool,
        cx: &mut Context<Self>,
    ) {
        if let Some(entry) = self
            .sessions
            .iter_mut()
            .find(|entry| entry.session_id == session_id)
        {
            entry.pinned = pinned;
            self.revisions.sessions = self.revisions.sessions.wrapping_add(1);
            cx.notify();
        }
    }

    pub(crate) fn theme_changed(
        &mut self,
        previous: crate::theme::ThemeVariant,
        cx: &mut Context<Self>,
    ) {
        if TerminalSettingsSnapshot::for_variant(&self.settings, previous)
            != TerminalSettingsSnapshot::from(&self.settings)
        {
            self.revisions.terminal_settings = self.revisions.terminal_settings.wrapping_add(1);
        }
        self.revisions.shell_settings = self.revisions.shell_settings.wrapping_add(1);
        cx.notify();
    }

    pub(crate) fn update_settings(
        &mut self,
        update: impl FnOnce(&mut AppSettings) -> bool,
        persist: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let terminal_settings = TerminalSettingsSnapshot::from(&self.settings);
        let mission_settings = MissionSettingsSnapshot::from(&self.settings);
        let shell_settings = ShellSettingsSnapshot::from(&self.settings);
        let agent_settings = (
            self.settings.enabled_agents.clone(),
            self.settings.disabled_agents.clone(),
        );
        if !update(&mut self.settings) {
            return false;
        }
        if agent_settings
            != (
                self.settings.enabled_agents.clone(),
                self.settings.disabled_agents.clone(),
            )
        {
            self.initialize_mcp_removal();
            self.initialize_skill_defaults();
            self.initialize_command_default();
        }
        if persist {
            self.save_settings();
        }
        self.revisions.settings = self.revisions.settings.wrapping_add(1);
        if TerminalSettingsSnapshot::from(&self.settings) != terminal_settings {
            self.revisions.terminal_settings = self.revisions.terminal_settings.wrapping_add(1);
        }
        if MissionSettingsSnapshot::from(&self.settings) != mission_settings {
            self.revisions.mission_settings = self.revisions.mission_settings.wrapping_add(1);
        }
        if ShellSettingsSnapshot::from(&self.settings) != shell_settings {
            self.revisions.shell_settings = self.revisions.shell_settings.wrapping_add(1);
        }
        cx.notify();
        true
    }

    pub(crate) fn save_settings(&self) {
        if let Err(error) = self.settings.save(&self.settings_path) {
            eprintln!("Runner UI settings save failed: {error:#}");
        }
    }

    fn refresh_sessions_inner(&mut self) {
        match self.client.session_list_recent_direct() {
            Ok(sessions) => {
                self.sessions = sessions;
                self.session_details = self.client.session_details().unwrap_or_default();
                self.revisions.sessions = self.revisions.sessions.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    fn refresh_roles_inner(&mut self) {
        match self.client.role_list() {
            Ok(roles) => {
                self.roles = roles;
                self.revisions.roles = self.revisions.roles.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    fn refresh_crews_inner(&mut self) {
        let result = self.client.crew_list_all();
        match result {
            Ok(crews) => {
                self.crews = crews;
                self.revisions.crews = self.revisions.crews.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    fn refresh_nodes_inner(&mut self) {
        match self.client.node_list() {
            Ok(nodes) => {
                self.nodes = nodes;
                self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
                self.revisions.tab_rows = self.revisions.tab_rows.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
        self.refresh_activity_inner();
    }

    fn refresh_projects_inner(&mut self) {
        match self.client.project_list() {
            Ok(projects) => {
                self.projects = projects;
                self.revisions.projects = self.revisions.projects.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    fn refresh_missions_blocking_inner(&mut self) {
        match self.client.mission_list_summary_impl(None) {
            Ok(missions) => {
                #[cfg(any(windows, test))]
                match mission_agent_ids(&self.client, &missions) {
                    Ok(ids) => self.mission_agent_ids = ids,
                    Err(error) => self.record_error(error.to_string()),
                }
                self.missions = missions;
                self.revisions.missions = self.revisions.missions.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    #[cfg(any(windows, test))]
    pub(crate) fn working_agent_count(&self) -> usize {
        self.session_statuses
            .iter()
            .filter(|(id, status)| {
                status.lifecycle == runner_core::protocol::status::Lifecycle::Running
                    && status.observation.activity
                        == runner_core::protocol::status::Activity::Working
                    && (self
                        .session_details
                        .get(*id)
                        .is_some_and(|entry| entry.agent_runtime != "shell")
                        || self.mission_agent_ids.contains(*id))
            })
            .count()
    }

    fn refresh_activity_inner(&mut self) {
        match self.client.live_session_counts() {
            Ok(counts) => {
                self.live_session_count = counts.values().sum();
                if !self.daemon_disconnected {
                    self.stopped_session_count = self.live_session_count;
                }
            }
            Err(_) => self.live_session_count = 0,
        }
        self.session_activity = self.client.session_activity_snapshot().unwrap_or_default();
        match self.client.session_status_snapshot() {
            Ok(statuses) => {
                for summary in &mut self.missions {
                    for (id, status) in &mut summary.session_statuses {
                        if let Some(current) = statuses.get(id) {
                            *status = current.clone();
                        }
                    }
                }
                self.session_statuses = statuses;
            }
            Err(error) => self.record_error(error.to_string()),
        }
        self.revisions.activity = self.revisions.activity.wrapping_add(1);
    }

    fn refresh_render_snapshots(&mut self) {
        self.window_entries = self.client.window_snapshot().unwrap_or_default();
        self.usage = self.client.usage_snapshot().unwrap_or_default();
        self.file_link_environment = self.client.file_link_environment().unwrap_or_default();
    }

    fn record_error(&mut self, error: String) {
        if self.daemon_disconnected {
            tracing::debug!("refresh while runnerd disconnected: {error}");
            return;
        }
        self.daemon_notice = None;
        if self.collecting_startup_errors {
            if let Some(current) = &mut self.error {
                current.push('\n');
                current.push_str(&error);
            } else {
                self.error = Some(error);
            }
        } else {
            self.error = Some(error);
        }
        self.revisions.error = self.revisions.error.wrapping_add(1);
    }
}

#[cfg(any(windows, test))]
fn mission_agent_ids(
    client: &DaemonClient,
    missions: &[MissionSummary],
) -> runner_core::protocol::ClientResult<std::collections::BTreeSet<String>> {
    let mut ids = std::collections::BTreeSet::new();
    for mission in missions {
        for row in client.session_list(&mission.mission.id)? {
            if row.runtime != "shell" {
                ids.insert(row.session.id);
            }
        }
    }
    Ok(ids)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntityRefreshKind {
    None,
    Roles,
    All,
}

impl EntityRefreshKind {
    fn for_event(event: &AppEvent) -> Self {
        match event.name.as_str() {
            "role/activity" => Self::Roles,
            "role/changed" | "crew/changed" | "slot/changed" => Self::All,
            _ => Self::None,
        }
    }

    fn merge(self, other: Self) -> Self {
        if self == Self::None || self == other {
            return other;
        }
        if other == Self::None {
            return self;
        }
        Self::All
    }

    fn roles(self) -> bool {
        matches!(self, Self::Roles | Self::All)
    }

    fn crews(self) -> bool {
        self == Self::All
    }
}

pub(crate) fn global_app_store(cx: &App) -> Entity<AppStore> {
    cx.global::<GlobalAppStore>().0.clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn working_count_excludes_busy_shells_and_uses_effective_mission_runtime() {
        use gpui::TestAppContext;
        use runner_core::protocol::status::{Activity, Lifecycle};
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let store = crate::app_store::test_lifecycle_store(&mut cx, temp.path());
        store.update(&mut cx, |store, _| {
            crate::app_store::seed_mixed_working_sessions(store);
            assert!(store.error.is_none(), "{:?}", store.error);
            assert_eq!(store.working_agent_count(), 2);
            store
                .session_statuses
                .get_mut("direct-agent")
                .unwrap()
                .observation
                .activity = Activity::Idle;
            assert_eq!(store.working_agent_count(), 1);
            store
                .session_statuses
                .get_mut("mission-agent")
                .unwrap()
                .lifecycle = Lifecycle::Stopped;
            assert_eq!(store.working_agent_count(), 0);
        });
    }

    fn event(name: &'static str) -> AppEvent {
        AppEvent {
            name: name.to_owned(),
            payload: serde_json::Value::Null,
        }
    }

    #[test]
    fn daemon_notice_survives_refresh_errors_and_clears_on_reconnect() {
        use gpui::{AppContext as _, TestAppContext};
        use runner_daemon::{db, session, shell_path};
        use std::sync::RwLock;
        let root = tempfile::tempdir().unwrap();
        let env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
        let discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
        let core = crate::test_support::core(
            Arc::new(db::open_pool(&root.path().join("runner.db")).unwrap()),
            root.path().into(),
            session::SessionManager::new(
                env.clone(),
                discovery.clone(),
                Arc::new(session::pty_runtime::PtyRuntime::new()),
            ),
            env,
            discovery,
        );
        let events = core.events.clone();
        let mut cx = TestAppContext::single();
        let store = cx.new(|cx| {
            AppStore::new(
                core,
                None,
                None,
                root.path().join("settings.json"),
                AppSettings::default(),
                None,
                cx,
            )
        });
        let before = store.read_with(&cx, |store, _| store.revisions.error);
        events.emit(
            "daemon/disconnected",
            &serde_json::json!({"restart_limit":true}),
        );
        cx.run_until_parked();
        store.update(&mut cx, |store, _| {
            store.record_error("connection closed".into())
        });
        store.read_with(&cx, |store, _| {
            assert!(store.error.is_none());
            assert_eq!(
                store.daemon_notice,
                Some(runner_app::lifecycle::DaemonNotice::Repeated)
            );
            assert!(store.revisions.error > before);
        });
        events.emit("daemon/reconnected", &serde_json::Value::Null);
        cx.run_until_parked();
        store.read_with(&cx, |store, _| {
            assert!(!store.daemon_disconnected);
            assert!(store.error.is_none() && store.daemon_notice.is_none());
        });
        store.update(&mut cx, |store, _| {
            store.live_session_count = 6;
            store.stopped_session_count = 6;
        });
        events.emit("daemon/disconnected", &serde_json::Value::Null);
        cx.run_until_parked();
        store.read_with(&cx, |store, _| {
            assert!(store.daemon_disconnected && store.error.is_none());
        });
        events.emit("daemon/reconnected", &serde_json::Value::Null);
        cx.run_until_parked();
        store.read_with(&cx, |store, _| {
            assert!(!store.daemon_disconnected);
            assert_eq!(
                store.error.as_deref(),
                Some("Background service restarted · 6 sessions stopped")
            );
        });
    }

    #[test]
    fn refresh_kinds_preserve_each_data_dependency() {
        assert_eq!(
            StoreRefreshKind::for_event(&event("session/status")),
            Some(StoreRefreshKind::Activity)
        );
        assert_eq!(
            StoreRefreshKind::for_event(&event("chat/layout-changed")),
            Some(StoreRefreshKind::Nodes)
        );
        assert_eq!(
            StoreRefreshKind::for_event(&event("mission/changed")),
            Some(StoreRefreshKind::All)
        );
        assert_eq!(
            StoreRefreshKind::for_event(&event("session/fork-started")),
            Some(StoreRefreshKind::All)
        );
        assert_eq!(StoreRefreshKind::for_event(&event("unrelated")), None);
    }

    #[test]
    fn coalesced_refreshes_cannot_drop_a_data_domain() {
        assert_eq!(
            StoreRefreshKind::Activity.merge(StoreRefreshKind::Missions),
            StoreRefreshKind::All
        );
        assert_eq!(
            StoreRefreshKind::Nodes.merge(StoreRefreshKind::Nodes),
            StoreRefreshKind::Nodes
        );
    }

    #[test]
    fn entity_refresh_events_match_role_and_crew_dependencies() {
        assert_eq!(
            EntityRefreshKind::for_event(&event("role/activity")),
            EntityRefreshKind::Roles
        );
        for name in ["role/changed", "crew/changed", "slot/changed"] {
            assert_eq!(
                EntityRefreshKind::for_event(&event(name)),
                EntityRefreshKind::All
            );
        }
        assert_eq!(
            EntityRefreshKind::for_event(&event("session/status")),
            EntityRefreshKind::None
        );
    }

    #[test]
    fn entity_refresh_merge_covers_every_pair() {
        use EntityRefreshKind::{All, None, Roles};

        for (left, right, expected) in [
            (None, None, None),
            (None, Roles, Roles),
            (None, All, All),
            (Roles, None, Roles),
            (Roles, Roles, Roles),
            (Roles, All, All),
            (All, None, All),
            (All, Roles, All),
            (All, All, All),
        ] {
            assert_eq!(left.merge(right), expected, "{left:?} + {right:?}");
        }
    }

    #[test]
    fn revision_reactions_match_root_side_effects() {
        let before = StoreRevisions::default();
        let mut after_settings = before;
        after_settings.settings = 1;
        assert_eq!(
            after_settings.reactions_since(before),
            StoreReactions {
                notify: true,
                ..Default::default()
            }
        );

        let mut after_terminal_settings = before;
        after_terminal_settings.settings = 1;
        after_terminal_settings.terminal_settings = 1;
        assert_eq!(
            after_terminal_settings.reactions_since(before),
            StoreReactions {
                apply_terminal_settings: true,
                notify: true,
                notify_without_settings: true,
                ..Default::default()
            }
        );

        let mut after_tabs = before;
        after_tabs.nodes = 1;
        after_tabs.tab_rows = 1;
        assert_eq!(
            after_tabs.reactions_since(before),
            StoreReactions {
                reload_tabs: true,
                notify: true,
                notify_without_settings: true,
                ..Default::default()
            }
        );

        let mut after_full_refresh = before;
        after_full_refresh.sessions = 1;
        after_full_refresh.full_refresh = 1;
        assert_eq!(
            after_full_refresh.reactions_since(before),
            StoreReactions {
                prune_window_state: true,
                notify: true,
                notify_without_settings: true,
                ..Default::default()
            }
        );

        let mut after_mission_settings = before;
        after_mission_settings.settings = 1;
        after_mission_settings.mission_settings = 1;
        let reactions = after_mission_settings.reactions_since(before);
        assert!(reactions.mission_settings);
        assert!(!reactions.notify_without_settings);
        assert!(!reactions.notify_shell());

        after_mission_settings.shell_settings = 1;
        assert!(after_mission_settings
            .reactions_since(before)
            .notify_shell());

        after_mission_settings.sessions = 1;
        assert!(after_mission_settings
            .reactions_since(before)
            .notify_shell());
    }

    #[test]
    fn role_data_and_surface_reload_revisions_are_independent() {
        let before = StoreRevisions::default();
        let mut after_data = before;
        after_data.roles = 1;
        assert!(!after_data.reactions_since(before).reload_role_surfaces);

        let mut after_event = before;
        after_event.role_surfaces = 1;
        assert!(after_event.reactions_since(before).reload_role_surfaces);
    }

    #[test]
    fn terminal_settings_snapshot_tracks_the_resolved_mode_and_both_picks() {
        use crate::theme::ThemeVariant;
        let settings = AppSettings::default();
        assert_ne!(
            TerminalSettingsSnapshot::for_variant(&settings, ThemeVariant::Carbon),
            TerminalSettingsSnapshot::for_variant(&settings, ThemeVariant::RunnerLight)
        );
        assert_eq!(
            TerminalSettingsSnapshot::for_variant(&settings, ThemeVariant::RunnerLight),
            TerminalSettingsSnapshot::for_variant(&settings, ThemeVariant::CatppuccinLatte)
        );
        assert_eq!(
            TerminalSettingsSnapshot::for_variant(&settings, ThemeVariant::Carbon),
            TerminalSettingsSnapshot::for_variant(&settings, ThemeVariant::CatppuccinMocha)
        );

        let mut light_pick = settings.clone();
        light_pick.light_terminal_theme = LightTerminalTheme::Runner;
        let mut dark_pick = settings.clone();
        dark_pick.dark_terminal_theme = DarkTerminalTheme::CatppuccinMocha;
        for variant in [ThemeVariant::Carbon, ThemeVariant::RunnerLight] {
            assert_ne!(
                TerminalSettingsSnapshot::for_variant(&settings, variant),
                TerminalSettingsSnapshot::for_variant(&light_pick, variant),
                "{variant:?}"
            );
            assert_ne!(
                TerminalSettingsSnapshot::for_variant(&settings, variant),
                TerminalSettingsSnapshot::for_variant(&dark_pick, variant),
                "{variant:?}"
            );
        }
    }

    #[test]
    fn terminal_settings_snapshot_ignores_unrelated_preferences() {
        let before = AppSettings {
            dark_terminal_theme: DarkTerminalTheme::CatppuccinMocha,
            ..AppSettings::default()
        };
        let mut after = before.clone();
        after.sidebar_width += 1.;
        assert_eq!(
            TerminalSettingsSnapshot::from(&before),
            TerminalSettingsSnapshot::from(&after)
        );

        after.terminal_font_size += 1;
        assert_ne!(
            TerminalSettingsSnapshot::from(&before),
            TerminalSettingsSnapshot::from(&after)
        );
    }

    #[test]
    fn mission_settings_snapshot_tracks_only_workspace_preferences() {
        let before = AppSettings::default();
        let mut after = before.clone();
        after.sidebar_width += 1.;
        assert_eq!(
            MissionSettingsSnapshot::from(&before),
            MissionSettingsSnapshot::from(&after)
        );

        after.mission_rail_width += 1.;
        assert_ne!(
            MissionSettingsSnapshot::from(&before),
            MissionSettingsSnapshot::from(&after)
        );
    }

    #[test]
    fn shell_settings_snapshot_ignores_mission_preferences() {
        let before = AppSettings {
            dark_terminal_theme: DarkTerminalTheme::CatppuccinMocha,
            ..AppSettings::default()
        };
        let mut after = before.clone();
        after.mission_rail_width += 1.;
        after
            .last_mission_terminal_ids
            .insert("mission".into(), "session".into());
        assert_eq!(
            ShellSettingsSnapshot::from(&before),
            ShellSettingsSnapshot::from(&after)
        );

        after.sidebar_width += 1.;
        assert_ne!(
            ShellSettingsSnapshot::from(&before),
            ShellSettingsSnapshot::from(&after)
        );
    }
}

#[derive(Clone, Copy)]
enum DaemonEvent {
    Disconnected(runner_app::lifecycle::DaemonNotice),
    Reconnected,
    Restarted(usize),
}

#[cfg(test)]
pub(crate) fn test_lifecycle_store(
    cx: &mut gpui::TestAppContext,
    root: &std::path::Path,
) -> gpui::Entity<crate::app_store::AppStore> {
    use gpui::AppContext as _;
    use runner_daemon::{db, session, shell_path};
    use std::sync::RwLock;
    let env = Arc::new(RwLock::new(shell_path::LoginShellEnv::default()));
    let discovery = Arc::new(RwLock::new(shell_path::DiscoveryState::startup(None, None)));
    let core = crate::test_support::core(
        Arc::new(db::open_pool(&root.join("runner.db")).unwrap()),
        root.to_owned(),
        session::SessionManager::new(
            env.clone(),
            discovery.clone(),
            Arc::new(session::pty_runtime::PtyRuntime::new()),
        ),
        env,
        discovery,
    );
    let store = cx.new(|cx| {
        crate::app_store::AppStore::new(
            core,
            None,
            None,
            root.join("settings.json"),
            Default::default(),
            None,
            cx,
        )
    });
    cx.update(|cx| {
        cx.set_global(crate::app_store::GlobalAppStore(store.clone()));
        cx.set_global(crate::WindowLayoutCheckpoint::default());
        cx.set_global(runner_app::lifecycle::QuitState::default());
        #[cfg(not(windows))]
        let updater = cx.new(|cx| crate::Updater::new(false, cx));
        #[cfg(windows)]
        let updater = cx.new(|cx| crate::Updater::new(false, root.join("updates"), cx));
        cx.set_global(crate::GlobalUpdater(updater));
    });
    store
}

#[cfg(test)]
pub(crate) fn seed_mixed_working_sessions(store: &mut AppStore) {
    use runner_core::protocol::status::{Activity, AgentObservation, AgentStatus, Lifecycle};
    store.test_core.db.get().unwrap().execute_batch(
        "INSERT INTO roles (id, handle, display_name, runtime, command, created_at, updated_at)
         VALUES ('test-shell-role', 'test-shell-role', 'Shell', 'shell', 'shell', '2026-10-06T00:00:00Z', '2026-10-06T00:00:00Z');
         INSERT INTO crews (id, name, created_at, updated_at) VALUES ('test-crew', 'Test', '2026-10-06T00:00:00Z', '2026-10-06T00:00:00Z');
         INSERT INTO missions (id, crew_id, title, status, started_at) VALUES ('test-mission', 'test-crew', 'Test', 'running', '2026-10-06T00:00:00Z');
         INSERT INTO sessions (id, role_id, mission_id, status, agent_runtime, agent_command)
         VALUES ('direct-agent', 'test-shell-role', NULL, 'running', 'codex', 'codex'),
                ('busy-shell', 'test-shell-role', NULL, 'running', 'shell', 'shell'),
                ('mission-agent', 'test-shell-role', 'test-mission', 'running', 'codex', 'codex'),
                ('mission-shell', 'test-shell-role', 'test-mission', 'running', 'shell', 'shell');"
    ).unwrap();
    store.refresh_sessions_inner();
    store.refresh_missions_blocking_inner();
    for id in [
        "direct-agent",
        "busy-shell",
        "mission-agent",
        "mission-shell",
    ] {
        store.session_statuses.insert(
            id.into(),
            AgentStatus {
                lifecycle: Lifecycle::Running,
                observation: AgentObservation {
                    activity: Activity::Working,
                    ..Default::default()
                },
                ..Default::default()
            },
        );
    }
}
