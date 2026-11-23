use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::hash::{Hash as _, Hasher as _};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use futures::{FutureExt as _, StreamExt as _};
use gpui::{App, AppContext as _, Context, Entity, EventEmitter, Global, Task};
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

use crate::app_settings::{AppSettings, TerminalCursorStyle, TerminalFontFamily};
use crate::theme::{DarkTheme, LightTheme};

mod command_default;
mod mcp_removal;
mod skill_defaults;
mod window_sync;
mod worktrees;
pub(crate) use command_default::{
    mark_command_install_initialized, run_user_command_action_serialized, CommandInstallSupport,
    UserCommandAction,
};
pub(crate) use skill_defaults::RunnerSkillStatus;
pub(crate) use worktrees::WorktreeQueryKey;

#[derive(Clone)]
pub(crate) struct GlobalAppStore(pub(crate) Entity<AppStore>);

impl Global for GlobalAppStore {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoreRefreshKind {
    Windows,
    Activity,
    Nodes,
    Missions,
    Runtimes,
    Onboarding,
    All,
}

impl StoreRefreshKind {
    pub(crate) fn for_event(event: &AppEvent) -> Option<Self> {
        match event.name.as_str() {
            "window_focus_map" => Some(Self::Windows),
            "runtime/changed" => Some(Self::Runtimes),
            "onboarding/changed" => Some(Self::Onboarding),
            "claude_accounts/changed" | "usage/updated" | "session/status" => Some(Self::Activity),
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
                                | "mission_reopened"
                                | "role_joined"
                                | "role_removed"
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
    pub(crate) remote_notice: u64,
    pub(crate) remote_enrollment: u64,
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
    pub(crate) locale: u64,
    pub(crate) terminal_settings: u64,
    pub(crate) mission_settings: u64,
    pub(crate) shell_settings: u64,
    pub(crate) full_refresh: u64,
    pub(crate) error: u64,
    pub(crate) restart: u64,
    pub(crate) onboarding: u64,
    pub(crate) worktrees: u64,
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

#[derive(Clone, Debug, Eq, PartialEq)]
struct TerminalSettingsSnapshot {
    light_theme: LightTheme,
    dark_theme: DarkTheme,
    is_light: bool,
    font_family: TerminalFontFamily,
    font_size: u16,
    line_height_tenths: u16,
    cursor_style: TerminalCursorStyle,
    cursor_blink: bool,
    scrollback_lines: usize,
    copy_on_select: bool,
}

impl From<&AppSettings> for TerminalSettingsSnapshot {
    fn from(settings: &AppSettings) -> Self {
        Self::for_variant(settings, crate::theme::active_variant())
    }
}

impl TerminalSettingsSnapshot {
    fn for_variant(settings: &AppSettings, variant: crate::theme::ThemeVariant) -> Self {
        Self {
            light_theme: settings.light_app_theme,
            dark_theme: settings.dark_app_theme,
            is_light: variant.is_light(),
            font_family: settings.terminal_font_family.clone(),
            font_size: settings.terminal_font_size,
            line_height_tenths: (settings.terminal_line_height * 10.).round() as u16,
            cursor_style: settings.terminal_cursor_style,
            cursor_blink: settings.terminal_cursor_blink,
            scrollback_lines: settings.terminal_scrollback_lines,
            copy_on_select: settings.terminal_copy_on_select,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct MissionSettingsSnapshot {
    show_side_panel: bool,
    rail_width: u32,
    fingerprint: u64,
}

impl From<&AppSettings> for MissionSettingsSnapshot {
    fn from(settings: &AppSettings) -> Self {
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        settings.last_mission_terminal_ids.hash(&mut hasher);
        settings.files_view.hash(&mut hasher);
        settings.changes_view.hash(&mut hasher);
        Self {
            show_side_panel: settings.show_side_panel,
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
        settings.language.hash(&mut hasher);
        std::mem::discriminant(&settings.app_theme).hash(&mut hasher);
        std::mem::discriminant(&settings.window_material).hash(&mut hasher);
        std::mem::discriminant(&settings.light_app_theme).hash(&mut hasher);
        std::mem::discriminant(&settings.dark_app_theme).hash(&mut hasher);
        settings.app_zoom.to_bits().hash(&mut hasher);
        settings.terminal_font_family.hash(&mut hasher);
        settings.terminal_font_size.hash(&mut hasher);
        std::mem::discriminant(&settings.terminal_cursor_style).hash(&mut hasher);
        settings.sidebar_width.to_bits().hash(&mut hasher);
        settings.sidebar_active_only.hash(&mut hasher);
        settings.sidebar_projects_open.hash(&mut hasher);
        settings.sidebar_chats_open.hash(&mut hasher);
        settings.sidebar_collapsed_projects.hash(&mut hasher);
        settings.show_side_panel.hash(&mut hasher);
        settings.files_view.hash(&mut hasher);
        settings.changes_view.hash(&mut hasher);
        settings.file_icons.hash(&mut hasher);
        settings.files_font_family.hash(&mut hasher);
        settings.files_font_size.hash(&mut hasher);
        settings.markdown_open_as.hash(&mut hasher);
        settings.chat_panel_width.to_bits().hash(&mut hasher);
        settings.default_crew_id.hash(&mut hasher);
        settings.default_working_dir.hash(&mut hasher);
        settings.resume_on_launch.hash(&mut hasher);
        settings.default_runtime.hash(&mut hasher);
        settings.disabled_agents.hash(&mut hasher);
        settings.enabled_agents.hash(&mut hasher);
        settings.usage.sidebar.hash(&mut hasher);
        settings.usage.codex.hash(&mut hasher);
        settings.usage.claude_code.hash(&mut hasher);
        settings.usage.antigravity.hash(&mut hasher);
        settings.usage.five_hour.hash(&mut hasher);
        settings.notifications.usage.near_limit.hash(&mut hasher);
        settings.notifications.usage.threshold.hash(&mut hasher);
        settings.runner_skill_enabled.hash(&mut hasher);
        settings.keymap_overrides.hash(&mut hasher);
        Self(hasher.finish())
    }
}

pub(crate) struct TerminalUpdates;

impl EventEmitter<String> for TerminalUpdates {}

#[derive(Clone, serde::Deserialize)]
pub(crate) struct RemotePairing {
    pub name: String,
    pub fingerprint: String,
}

impl EventEmitter<RemotePairing> for AppStore {}

enum RemoteUpdate {
    Status(runner_core::remote::RemoteStatus),
    Trust(runner_core::remote::RemoteTrust),
    Paired(RemotePairing),
}

#[derive(Clone)]
pub(crate) enum RemoteWarning {
    Stopped(Box<runner_core::remote::RemoteStatus>),
    PinLocked,
}

pub(crate) struct AppStore {
    pub(crate) remote_clients: Entity<crate::surfaces::remote_client::model::ClientState>,
    pub(crate) remote_status: Option<runner_core::remote::RemoteStatus>,
    pub(crate) enrollment_requests: Arc<crate::surfaces::settings::remote::EnrollmentRequests>,
    pub(crate) remote_trust: Option<runner_core::remote::RemoteTrust>,
    pub(crate) remote_warning: Option<RemoteWarning>,
    pub(crate) terminal_updates: Entity<TerminalUpdates>,
    terminal_labels: BTreeMap<
        String,
        (
            String,
            Option<PathBuf>,
            runner_terminal::terminal::TerminalControl,
        ),
    >,
    pub(crate) client: DaemonClient,
    pub(crate) app_data_dir: PathBuf,
    pub(crate) launcher_tabs_pending: crate::surfaces::start_chat::launcher::PendingLauncherTabs,
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
    pub(crate) mission_sessions: BTreeMap<String, Vec<runner_core::protocol::SessionRow>>,
    pub(crate) session_statuses: BTreeMap<String, runner_core::protocol::status::AgentStatus>,
    pub(crate) session_activity: BTreeMap<String, SessionActivityState>,
    mission_agent_ids: std::collections::BTreeSet<String>,
    pub(crate) live_session_count: usize,
    pub(crate) settings: AppSettings,
    pub(crate) resolved_locale: runner_app::i18n::Locale,
    settings_path: PathBuf,
    pub(crate) home_dir: Option<PathBuf>,
    pub(crate) usage: runner_core::protocol::UsageSnapshot,
    pub(crate) claude_accounts: runner_core::protocol::ClaudeAccounts,
    pub(crate) claude_account_usage: BTreeMap<String, runner_core::protocol::RuntimeUsage>,
    pub(crate) file_link_environment: (String, std::collections::HashMap<String, bool>),
    pub(crate) onboarding: runner_core::protocol::OnboardingStatus,
    command_install_support: Option<CommandInstallSupport>,
    runner_command_status: Option<runner_core::command_install::RunnerCommandStatus>,
    command_install_requires_escalation: bool,
    command_action_target: Option<PathBuf>,
    runner_skill_status: RunnerSkillStatus,
    pub(crate) revisions: StoreRevisions,
    pub(crate) error: Option<runner_app::i18n::Text>,
    collecting_startup_errors: bool,
    pub(crate) daemon_disconnected: bool,
    pub(crate) daemon_notice: Option<runner_app::lifecycle::DaemonNotice>,
    pub(crate) stopped_session_count: usize,
    pub(crate) restarted_sessions: Option<usize>,
    pub(crate) worktree_checkouts:
        BTreeMap<WorktreeQueryKey, runner_core::protocol::worktree::WorktreeCheckout>,
    pub(crate) worktree_extra_keys: BTreeSet<WorktreeQueryKey>,
    worktree_in_flight: BTreeSet<WorktreeQueryKey>,
    worktree_refresh_needed: BTreeSet<WorktreeQueryKey>,
    pub(crate) worktree_auto_bases: BTreeMap<PathBuf, Option<String>>,
    worktree_base_in_flight: BTreeSet<PathBuf>,
    worktree_base_refresh_needed: BTreeSet<PathBuf>,
    watched_worktree_roots: BTreeSet<PathBuf>,
    worktree_watch_consumers: usize,
    worktree_watch_task: Option<Task<()>>,
    window_entries_epoch: u64,
    session_activity_epoch: u64,
    node_row_revisions: HashMap<String, u64>,
    node_viewed_floors: HashMap<String, String>,
    full_nodes_replaced_revision: u64,
    desired_skill_enabled: Arc<AtomicBool>,
    skill_status_seq: Arc<AtomicU64>,
    applied_skill_status_seq: u64,
    skill_reconcile_lock: Arc<Mutex<()>>,
    skill_reconcile_task: Option<Task<()>>,
    command_install_initialized: Arc<AtomicBool>,
    command_status_seq: Arc<AtomicU64>,
    applied_command_status_seq: u64,
    command_reconcile_lock: Arc<Mutex<()>>,
    window_coordinator: window_sync::WindowCoordinator,
    refresh_in_flight: bool,
    pending_refresh: Option<(StoreRefreshKind, EntityRefreshKind)>,
    refresh_task: Option<Task<()>>,
    _tasks: [Task<()>; 5],
}

impl AppStore {
    pub(crate) fn remote_signed_in(&self) -> bool {
        self.remote_status
            .as_ref()
            .is_some_and(crate::surfaces::settings::remote::account::signed_in)
    }

    fn apply_remote_update(&mut self, update: RemoteUpdate, live: bool) {
        let was_signed_in = self.remote_signed_in();
        let warning = match update {
            RemoteUpdate::Paired(_) => return,
            RemoteUpdate::Status(status) => {
                if live
                    && self.remote_status.as_ref().is_some_and(|previous| {
                        matches!(
                            previous.state,
                            runner_core::remote::RemoteState::Enrolling { .. }
                                | runner_core::remote::RemoteState::Confirming { .. }
                        )
                    })
                    && !matches!(
                        status.state,
                        runner_core::remote::RemoteState::Enrolling { .. }
                            | runner_core::remote::RemoteState::Confirming { .. }
                    )
                {
                    self.revisions.remote_enrollment =
                        self.revisions.remote_enrollment.wrapping_add(1);
                }
                let warning = (live
                    && crate::surfaces::settings::remote::warn_transition(
                        self.remote_status.as_ref(),
                        &status,
                    ))
                .then(|| RemoteWarning::Stopped(Box::new(status.clone())));
                self.remote_status = Some(status);
                warning
            }
            RemoteUpdate::Trust(trust) => {
                let warning = (live
                    && crate::surfaces::settings::remote::warn_pin_lock(
                        self.remote_trust.as_ref(),
                        &trust,
                    ))
                .then_some(RemoteWarning::PinLocked);
                self.remote_trust = Some(trust);
                warning
            }
        };
        if self.remote_signed_in() != was_signed_in {
            self.revisions.shell_settings = self.revisions.shell_settings.wrapping_add(1);
        }
        if let Some(warning) = warning {
            self.remote_warning = Some(warning);
            self.revisions.remote_notice = self.revisions.remote_notice.wrapping_add(1);
        }
    }

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
        #[cfg(test)]
        crate::test_support::drain_tasks_when_app_drops(cx);
        let remote_clients = cx.new(|cx| {
            crate::surfaces::remote_client::model::ClientState::new(host.client.clone(), cx)
        });
        let terminal_updates = cx.new(|_| TerminalUpdates);
        let (wake_tx, mut wake_rx) = futures::channel::mpsc::unbounded::<String>();
        let waker: Arc<dyn Fn(&str) + Send + Sync> = Arc::new(move |id| {
            let _ = wake_tx.unbounded_send(id.to_owned());
        });
        let bridge = TerminalBridge::with_session_waker(host.client.clone(), waker)
            .expect("terminal event bridge installation is infallible");

        let wake_task = cx.spawn(async move |weak, cx| {
            while let Some(id) = wake_rx.next().await {
                let mut changed = BTreeSet::from([id]);
                let delay = cx
                    .background_executor()
                    .timer(Duration::from_millis(4))
                    .fuse();
                futures::pin_mut!(delay);
                loop {
                    futures::select_biased! {
                        _ = delay => break,
                        wake = wake_rx.next().fuse() => {
                            if let Some(id) = wake {
                                changed.insert(id);
                            } else {
                                break;
                            }
                        }
                    }
                }
                if weak
                    .update(cx, |this, cx| {
                        let mut labels_changed = false;
                        for id in changed {
                            match this.bridge.session(&id) {
                                Some(terminal) => {
                                    let label =
                                        (terminal.title(), terminal.live_cwd(), terminal.control());
                                    if this.terminal_labels.get(&id) != Some(&label) {
                                        this.terminal_labels.insert(id.clone(), label);
                                        labels_changed = true;
                                    }
                                }
                                _ => {
                                    labels_changed |= this.terminal_labels.remove(&id).is_some();
                                }
                            }
                            this.terminal_updates.update(cx, |_, cx| cx.emit(id));
                        }
                        if labels_changed {
                            this.revisions.terminal_wake =
                                this.revisions.terminal_wake.wrapping_add(1);
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        let (event_tx, mut event_rx) = futures::channel::mpsc::unbounded::<(
            Option<StoreRefreshKind>,
            EntityRefreshKind,
            Option<DaemonEvent>,
        )>();
        #[cfg(not(test))]
        let client = host.client.clone();
        #[cfg(test)]
        let client = crate::test_support::client(&core);
        let mut events = client.subscribe();
        let events_bg_task = cx.background_spawn(async move {
            loop {
                let refresh = match events.recv().await {
                    Ok(event) if event.name == "daemon/disconnected" => Some((
                        Some(StoreRefreshKind::All),
                        EntityRefreshKind::All,
                        Some(DaemonEvent::Disconnected(
                            if event.payload["in_use"].as_bool().unwrap_or(false) {
                                runner_app::lifecycle::DaemonNotice::InUse
                            } else if event.payload["restart_limit"].as_bool().unwrap_or(false) {
                                runner_app::lifecycle::DaemonNotice::Repeated
                            } else {
                                runner_app::lifecycle::DaemonNotice::Stopped
                            },
                        )),
                    )),
                    Ok(event) if event.name == "daemon/reconnected" => Some((
                        Some(StoreRefreshKind::All),
                        EntityRefreshKind::All,
                        Some(DaemonEvent::Reconnected),
                    )),
                    Ok(event) if event.name == "daemon/restarted" => Some((
                        Some(StoreRefreshKind::All),
                        EntityRefreshKind::All,
                        Some(DaemonEvent::Restarted(
                            event.payload["count"].as_u64().unwrap_or(0) as usize,
                        )),
                    )),
                    Ok(event) if event.name == runner_core::protocol::files::FILES_STATUS_EVENT => {
                        serde_json::from_value::<runner_core::protocol::files::FilesStatusChanged>(
                            event.payload,
                        )
                        .ok()
                        .map(|changed| {
                            (
                                None,
                                EntityRefreshKind::None,
                                Some(DaemonEvent::FilesRootChanged(changed.root)),
                            )
                        })
                    }
                    Ok(event)
                        if event.name == runner_core::protocol::files::FILES_CHANGED_EVENT =>
                    {
                        serde_json::from_value::<runner_core::protocol::files::FilesChanged>(
                            event.payload,
                        )
                        .ok()
                        .filter(|changed| {
                            changed.folders.iter().any(|folder| folder.is_empty())
                                && !runner_core::app_paths::is_existing_dir(&changed.root)
                        })
                        .map(|changed| {
                            (
                                None,
                                EntityRefreshKind::None,
                                Some(DaemonEvent::FilesRootChanged(changed.root)),
                            )
                        })
                    }
                    Ok(event) => StoreRefreshKind::for_event(&event)
                        .map(|store| (Some(store), EntityRefreshKind::for_event(&event), None)),
                    Err(runner_core::protocol::EventError::Lagged(_)) => Some((
                        Some(StoreRefreshKind::All),
                        EntityRefreshKind::All,
                        Some(DaemonEvent::Lagged),
                    )),
                    Err(runner_core::protocol::EventError::Closed) => break,
                };
                if refresh.is_some_and(|refresh| event_tx.unbounded_send(refresh).is_err()) {
                    break;
                }
            }
        });
        let events_fg_task = cx.spawn(async move |weak, cx| {
            while let Some((mut refresh, mut entity_refresh, event)) = event_rx.next().await {
                let mut daemon_events = event.into_iter().collect::<Vec<_>>();
                while let Ok(next) = event_rx.try_recv() {
                    refresh = match (refresh, next.0) {
                        (Some(a), Some(b)) => Some(a.merge(b)),
                        (Some(a), None) | (None, Some(a)) => Some(a),
                        (None, None) => None,
                    };
                    entity_refresh = entity_refresh.merge(next.1);
                    daemon_events.extend(next.2);
                }
                if weak
                    .update(cx, |this, cx| {
                        let mut notify_for_daemon = false;
                        let mut reconnected = false;
                        let mut lagged = false;
                        let mut files_roots = Vec::new();
                        for event in daemon_events {
                            match event {
                                DaemonEvent::Disconnected(notice) => {
                                    notify_for_daemon = true;
                                    this.daemon_disconnected = true;
                                    this.daemon_notice = Some(notice);
                                    this.error = None;
                                    this.revisions.error = this.revisions.error.wrapping_add(1);
                                }
                                DaemonEvent::Reconnected => {
                                    notify_for_daemon = true;
                                    reconnected = true;
                                    this.daemon_disconnected = false;
                                    if matches!(
                                        this.daemon_notice,
                                        Some(
                                            runner_app::lifecycle::DaemonNotice::Repeated
                                                | runner_app::lifecycle::DaemonNotice::InUse
                                        )
                                    ) {
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
                                DaemonEvent::Lagged => {
                                    lagged = true;
                                }
                                DaemonEvent::Restarted(count) => {
                                    notify_for_daemon = true;
                                    reconnected = true;
                                    this.restarted_sessions = Some(count);
                                    this.revisions.restart = this.revisions.restart.wrapping_add(1);
                                }
                                DaemonEvent::FilesRootChanged(root) => {
                                    files_roots.push(root);
                                }
                            }
                        }
                        if let Some(refresh) = refresh {
                            this.schedule_refresh(refresh, entity_refresh, cx);
                        } else if entity_refresh != EntityRefreshKind::None {
                            this.schedule_refresh(StoreRefreshKind::Windows, entity_refresh, cx);
                        }
                        if reconnected {
                            this.rewatch_worktrees_after_reconnect(cx);
                        } else if lagged {
                            this.refresh_all_worktrees_after_lag(cx);
                        } else if !files_roots.is_empty() {
                            files_roots.sort();
                            files_roots.dedup();
                            for root in files_roots {
                                this.refresh_worktrees_for_root(&root, cx);
                            }
                        }
                        if notify_for_daemon {
                            cx.notify();
                        }
                    })
                    .is_err()
                {
                    break;
                }
            }
        });

        if let Err(error) = bridge.attach_live_sessions() {
            eprintln!("attach live terminals: {error}");
        }
        let (remote_tx, mut remote_rx) = futures::channel::mpsc::unbounded();
        let remote_client = host.client.clone();
        let mut remote_events = remote_client.subscribe();
        let remote_events_task = cx.background_spawn(async move {
            if let Ok(status) = remote_client.remote_status() {
                let _ = remote_tx.unbounded_send((RemoteUpdate::Status(status), false));
            }
            if let Ok(trust) = remote_client.remote_trust() {
                let _ = remote_tx.unbounded_send((RemoteUpdate::Trust(trust), false));
            }
            loop {
                match remote_events.recv().await {
                    Ok(event) if event.name == "remote/status" => {
                        if let Ok(status) = serde_json::from_value(event.payload) {
                            if remote_tx
                                .unbounded_send((RemoteUpdate::Status(status), true))
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                    Ok(event) if event.name == "remote/paired" => {
                        if let Ok(pairing) = serde_json::from_value(event.payload) {
                            if remote_tx
                                .unbounded_send((RemoteUpdate::Paired(pairing), true))
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                    Ok(event) if event.name == "remote/trust" => {
                        if let Ok(trust) = serde_json::from_value(event.payload) {
                            if remote_tx
                                .unbounded_send((RemoteUpdate::Trust(trust), true))
                                .is_err()
                            {
                                break;
                            }
                        }
                    }
                    Ok(event) if event.name == "daemon/reconnected" => {
                        if let Ok(trust) = remote_client.remote_trust() {
                            let _ = remote_tx.unbounded_send((RemoteUpdate::Trust(trust), false));
                        }
                        if let Ok(status) = remote_client.remote_status() {
                            let _ = remote_tx.unbounded_send((RemoteUpdate::Status(status), false));
                        }
                    }
                    Ok(_) => {}
                    Err(runner_core::protocol::EventError::Lagged(_)) => {
                        if let Ok(trust) = remote_client.remote_trust() {
                            let _ = remote_tx.unbounded_send((RemoteUpdate::Trust(trust), false));
                        }
                        if let Ok(status) = remote_client.remote_status() {
                            let _ = remote_tx.unbounded_send((RemoteUpdate::Status(status), false));
                        }
                    }
                    Err(_) => break,
                }
            }
        });
        let remote_fg_task = cx.spawn(async move |weak, cx| {
            while let Some((update, live)) = remote_rx.next().await {
                if weak
                    .update(cx, |store, cx| {
                        match update {
                            RemoteUpdate::Paired(pairing) => cx.emit(pairing),
                            update => store.apply_remote_update(update, live),
                        }
                        cx.notify();
                    })
                    .is_err()
                {
                    break;
                }
            }
        });
        let initial_skill_enabled = settings.runner_skill_enabled;
        let initial_command_initialized = settings.initialized_command_install;
        let mut store = Self {
            remote_clients,
            remote_status: None,
            enrollment_requests: Arc::default(),
            remote_trust: None,
            remote_warning: None,
            terminal_updates,
            terminal_labels: BTreeMap::new(),
            launcher_tabs_pending: Arc::default(),
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
            mission_sessions: BTreeMap::new(),
            session_statuses: BTreeMap::new(),
            session_activity: BTreeMap::new(),
            mission_agent_ids: Default::default(),
            live_session_count: 0,
            resolved_locale: runner_app::i18n::active_locale(),
            settings,
            settings_path,
            home_dir,
            usage: Default::default(),
            claude_accounts: Default::default(),
            claude_account_usage: Default::default(),
            file_link_environment: Default::default(),
            onboarding: Default::default(),
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
            worktree_checkouts: BTreeMap::new(),
            worktree_extra_keys: BTreeSet::new(),
            worktree_in_flight: BTreeSet::new(),
            worktree_refresh_needed: BTreeSet::new(),
            worktree_auto_bases: BTreeMap::new(),
            worktree_base_in_flight: BTreeSet::new(),
            worktree_base_refresh_needed: BTreeSet::new(),
            watched_worktree_roots: BTreeSet::new(),
            worktree_watch_consumers: 0,
            worktree_watch_task: None,
            window_entries_epoch: 0,
            session_activity_epoch: 0,
            node_row_revisions: HashMap::new(),
            node_viewed_floors: HashMap::new(),
            full_nodes_replaced_revision: 0,
            desired_skill_enabled: Arc::new(AtomicBool::new(initial_skill_enabled)),
            skill_status_seq: Arc::new(AtomicU64::new(0)),
            applied_skill_status_seq: 0,
            skill_reconcile_lock: Arc::new(Mutex::new(())),
            skill_reconcile_task: None,
            command_install_initialized: Arc::new(AtomicBool::new(initial_command_initialized)),
            command_status_seq: Arc::new(AtomicU64::new(0)),
            applied_command_status_seq: 0,
            command_reconcile_lock: Arc::new(Mutex::new(())),
            window_coordinator: window_sync::WindowCoordinator::default(),
            refresh_in_flight: false,
            pending_refresh: None,
            refresh_task: None,
            _tasks: [
                wake_task,
                events_bg_task,
                events_fg_task,
                remote_events_task,
                remote_fg_task,
            ],
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
        store.refresh_onboarding_inner();
        store.collecting_startup_errors = false;
        store.sync_worktree_checkouts(cx);
        store
    }

    pub(crate) fn refresh(&mut self, refresh: StoreRefreshKind, cx: &mut Context<Self>) {
        self.schedule_refresh(refresh, EntityRefreshKind::None, cx);
    }

    pub(crate) fn set_window_entries(
        &mut self,
        mut entries: Vec<runner_core::protocol::WindowEntry>,
        cx: &mut Context<Self>,
    ) {
        entries.retain(|entry| {
            !self
                .window_coordinator
                .should_exclude_from_snapshot(&entry.label)
        });
        self.window_entries_epoch = self.window_entries_epoch.wrapping_add(1);
        self.window_entries = entries;
        self.revisions.activity = self.revisions.activity.wrapping_add(1);
        cx.notify();
    }

    fn schedule_refresh(
        &mut self,
        refresh: StoreRefreshKind,
        entity_refresh: EntityRefreshKind,
        cx: &mut Context<Self>,
    ) {
        self.pending_refresh = Some(match self.pending_refresh.take() {
            Some((prev_store, prev_entity)) => {
                (prev_store.merge(refresh), prev_entity.merge(entity_refresh))
            }
            None => (refresh, entity_refresh),
        });
        if !self.refresh_in_flight {
            self.start_next_refresh(cx);
        }
    }

    fn queue_retry_refresh(
        &mut self,
        refresh: StoreRefreshKind,
        entity_refresh: EntityRefreshKind,
    ) {
        self.pending_refresh = Some(match self.pending_refresh.take() {
            Some((prev_store, prev_entity)) => {
                (prev_store.merge(refresh), prev_entity.merge(entity_refresh))
            }
            None => (refresh, entity_refresh),
        });
    }

    fn start_next_refresh(&mut self, cx: &mut Context<Self>) {
        let Some((refresh, entity_refresh)) = self.pending_refresh.take() else {
            self.refresh_task = None;
            return;
        };
        self.refresh_in_flight = true;
        let started_revisions = self.revisions;
        let started_window_epoch = self.window_entries_epoch;
        let started_activity_epoch = self.session_activity_epoch;
        let client = self.client.clone();

        if refresh == StoreRefreshKind::Windows && entity_refresh == EntityRefreshKind::None {
            let bg =
                cx.background_spawn(async move { client.window_snapshot().unwrap_or_default() });
            self.refresh_task = Some(cx.spawn(async move |weak, cx| {
                let mut entries = bg.await;
                let _ = weak.update(cx, |this, cx| {
                    if !this.window_coordinator.in_flight
                        && !this.window_coordinator.has_pending_work()
                        && this.window_entries_epoch == started_window_epoch
                    {
                        entries.retain(|entry| {
                            !this
                                .window_coordinator
                                .should_exclude_from_snapshot(&entry.label)
                        });
                        this.window_entries = entries;
                    }
                    this.revisions.activity = this.revisions.activity.wrapping_add(1);
                    cx.notify();
                    this.refresh_in_flight = false;
                    this.start_next_refresh(cx);
                });
            }));
            return;
        }

        let runtime_inputs = matches!(refresh, StoreRefreshKind::Runtimes | StoreRefreshKind::All)
            .then(|| RuntimeRefreshInputs {
                settings: self.settings.clone(),
                home_dir: self.home_dir.clone(),
                app_data_dir: self.app_data_dir.clone(),
                command_install_support: self.command_install_support.clone(),
                desired_skill_enabled: Arc::clone(&self.desired_skill_enabled),
                skill_status_seq: Arc::clone(&self.skill_status_seq),
                skill_reconcile_lock: Arc::clone(&self.skill_reconcile_lock),
                command_install_initialized: Arc::clone(&self.command_install_initialized),
                command_status_seq: Arc::clone(&self.command_status_seq),
                command_reconcile_lock: Arc::clone(&self.command_reconcile_lock),
            });
        let bg = cx.background_spawn(async move {
            fetch_store_refresh(client, refresh, entity_refresh, runtime_inputs)
        });
        self.refresh_task = Some(cx.spawn(async move |weak, cx| {
            let fetched = bg.await;
            let _ = weak.update(cx, |this, cx| {
                this.apply_fetched_refresh(
                    refresh,
                    entity_refresh,
                    started_revisions,
                    started_window_epoch,
                    started_activity_epoch,
                    fetched,
                );
                if matches!(
                    refresh,
                    StoreRefreshKind::Nodes | StoreRefreshKind::Missions | StoreRefreshKind::All
                ) {
                    this.sync_worktree_checkouts(cx);
                }
                cx.notify();
                this.refresh_in_flight = false;
                this.start_next_refresh(cx);
            });
        }));
    }

    #[allow(clippy::too_many_arguments)]
    fn apply_fetched_refresh(
        &mut self,
        refresh: StoreRefreshKind,
        entity_refresh: EntityRefreshKind,
        started_revisions: StoreRevisions,
        started_window_epoch: u64,
        started_activity_epoch: u64,
        fetched: FetchedStoreRefresh,
    ) {
        let _turn = crate::stall_watchdog::enter_turn("store.apply_refresh");
        if let Some(next) = fetched.onboarding {
            if self.onboarding != next {
                self.onboarding = next;
                self.revisions.onboarding = self.revisions.onboarding.wrapping_add(1);
            }
        }
        if let Some(runtime_init) = fetched.runtime_init {
            if runtime_init.mcp_completed && !self.settings.mcp_registrations_removed {
                self.settings.mcp_registrations_removed = true;
                self.save_settings();
            }
            self.apply_skill_reconcile_outcome(runtime_init.skill_outcome);
            self.apply_command_default(runtime_init.command_default);
        }
        if let Some(roles_res) = fetched.roles {
            match roles_res {
                Ok(roles) => {
                    if self.revisions.roles == started_revisions.roles {
                        self.roles = roles;
                        self.revisions.roles = self.revisions.roles.wrapping_add(1);
                    } else {
                        self.queue_retry_refresh(StoreRefreshKind::Windows, EntityRefreshKind::All);
                    }
                }
                Err(error) => self.record_error(error.to_string()),
            }
        }
        if entity_refresh.roles() {
            self.revisions.role_surfaces = self.revisions.role_surfaces.wrapping_add(1);
        }
        if let Some(crews_res) = fetched.crews {
            match crews_res {
                Ok(crews) => {
                    self.crews = crews;
                    self.revisions.crews = self.revisions.crews.wrapping_add(1);
                }
                Err(error) => self.record_error(error.to_string()),
            }
        }
        if let Some(sessions_res) = fetched.sessions {
            match sessions_res {
                Ok((sessions, details)) => {
                    if self.revisions.sessions == started_revisions.sessions {
                        self.sessions = sessions;
                        self.session_details = details;
                        self.revisions.sessions = self.revisions.sessions.wrapping_add(1);
                    } else {
                        self.queue_retry_refresh(StoreRefreshKind::All, EntityRefreshKind::None);
                    }
                }
                Err(error) => self.record_error(error.to_string()),
            }
        }
        if let Some(nodes_res) = fetched.nodes {
            crate::stall_watchdog::breadcrumb("store.apply_refresh.nodes");
            match nodes_res {
                Ok(mut nodes) => {
                    if self.full_nodes_replaced_revision > started_revisions.nodes {
                        self.queue_retry_refresh(StoreRefreshKind::Nodes, EntityRefreshKind::None);
                    } else {
                        for row in &mut nodes {
                            if self
                                .node_row_revisions
                                .get(&row.id)
                                .is_some_and(|&rev| rev > started_revisions.nodes)
                            {
                                if let Some(current) =
                                    self.nodes.iter().find(|existing| existing.id == row.id)
                                {
                                    *row = current.clone();
                                }
                            }
                            if let Some(floor) = self.node_viewed_floors.get(&row.id) {
                                if newer_viewed_timestamp(
                                    Some(floor.as_str()),
                                    row.last_viewed_at.as_deref(),
                                ) {
                                    row.last_viewed_at = Some(floor.clone());
                                }
                            }
                        }
                        self.node_row_revisions
                            .retain(|_, &mut rev| rev > started_revisions.nodes);
                        self.nodes = nodes;
                        self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
                        self.revisions.tab_rows = self.revisions.tab_rows.wrapping_add(1);
                    }
                }
                Err(error) => self.record_error(error.to_string()),
            }
        }
        if let Some(projects_res) = fetched.projects {
            match projects_res {
                Ok(projects) => {
                    self.projects = projects;
                    self.revisions.projects = self.revisions.projects.wrapping_add(1);
                }
                Err(error) => self.record_error(error.to_string()),
            }
        }
        if refresh == StoreRefreshKind::All {
            self.revisions.full_refresh = self.revisions.full_refresh.wrapping_add(1);
        }
        if let Some(missions_res) = fetched.missions {
            crate::stall_watchdog::breadcrumb("store.apply_refresh.missions");
            match missions_res {
                Ok((mut missions, idx_res)) => {
                    match idx_res {
                        Ok((ids, sessions)) => {
                            self.mission_agent_ids = ids;
                            self.mission_sessions = sessions;
                        }
                        Err(error) => self.record_error(error.to_string()),
                    }
                    for summary in &mut missions {
                        for (id, status) in &mut summary.session_statuses {
                            if let Some(current) = self.session_statuses.get(id) {
                                *status = current.clone();
                            }
                        }
                    }
                    self.missions = missions;
                    self.revisions.missions = self.revisions.missions.wrapping_add(1);
                }
                Err(error) => self.record_error(error.to_string()),
            }
        }
        if let Some(activity) = fetched.activity {
            crate::stall_watchdog::breadcrumb("store.apply_refresh.activity");
            match activity.counts {
                Ok(counts) => {
                    self.live_session_count = counts.values().sum();
                    if !self.daemon_disconnected {
                        self.stopped_session_count = self.live_session_count;
                    }
                }
                Err(_) => self.live_session_count = 0,
            }
            if self.session_activity_epoch == started_activity_epoch {
                self.session_activity = activity.activity;
                match activity.statuses {
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
            }
            self.revisions.activity = self.revisions.activity.wrapping_add(1);
        }
        if !self.window_coordinator.in_flight
            && !self.window_coordinator.has_pending_work()
            && self.window_entries_epoch == started_window_epoch
        {
            let mut entries = fetched.render.window_entries;
            entries.retain(|entry| {
                !self
                    .window_coordinator
                    .should_exclude_from_snapshot(&entry.label)
            });
            self.window_entries = entries;
        }
        self.usage = fetched.render.usage;
        self.claude_accounts = fetched.render.claude_accounts;
        self.claude_account_usage = fetched.render.claude_account_usage;
        self.file_link_environment = fetched.render.file_link_environment;
    }

    pub(crate) fn refresh_sessions(&mut self, cx: &mut Context<Self>) {
        self.refresh_sessions_inner();
        self.sync_worktree_checkouts(cx);
        cx.notify();
    }

    pub(crate) fn refresh_nodes(
        &mut self,
        cx: &mut Context<Self>,
    ) -> std::result::Result<(), runner_core::protocol::ClientError> {
        match self.client.node_list() {
            Ok(nodes) => {
                self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
                self.full_nodes_replaced_revision = self.revisions.nodes;
                self.node_row_revisions.clear();
                self.nodes = nodes;
                self.revisions.tab_rows = self.revisions.tab_rows.wrapping_add(1);
                self.sync_worktree_checkouts(cx);
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
        self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
        self.full_nodes_replaced_revision = self.revisions.nodes;
        self.node_row_revisions.clear();
        self.nodes = nodes;
        self.revisions.tab_rows = self.revisions.tab_rows.wrapping_add(1);
        self.sync_worktree_checkouts(cx);
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn replace_node(&mut self, node: NodeRow, cx: &mut Context<Self>) {
        self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
        let rev = self.revisions.nodes;
        self.node_row_revisions.insert(node.id.clone(), rev);
        if let Some(current) = self.nodes.iter_mut().find(|current| current.id == node.id) {
            *current = node;
        }
        cx.notify();
    }

    pub(crate) fn apply_node_viewed(
        &mut self,
        node_id: &str,
        last_viewed_at: Option<String>,
        cx: &mut Context<Self>,
    ) {
        if let Some(viewed_at) = last_viewed_at.as_deref() {
            let update_floor = self
                .node_viewed_floors
                .get(node_id)
                .is_none_or(|prev| newer_viewed_timestamp(Some(viewed_at), Some(prev.as_str())));
            if update_floor {
                self.node_viewed_floors
                    .insert(node_id.to_owned(), viewed_at.to_owned());
            }
        }
        if let Some(current) = self.nodes.iter_mut().find(|current| current.id == node_id) {
            if newer_viewed_timestamp(last_viewed_at.as_deref(), current.last_viewed_at.as_deref())
            {
                current.last_viewed_at = last_viewed_at;
            }
        }
        self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
        cx.notify();
    }

    pub(crate) fn remove_session_activity(&mut self, session_id: &str, cx: &mut Context<Self>) {
        self.session_activity_epoch = self.session_activity_epoch.wrapping_add(1);
        self.session_activity.remove(session_id);
        self.session_statuses.remove(session_id);
        self.revisions.activity = self.revisions.activity.wrapping_add(1);
        cx.notify();
    }

    pub(crate) fn sync_locale(&mut self, cx: &mut Context<Self>) {
        #[cfg(not(test))]
        let system = sys_locale::get_locale();
        #[cfg(test)]
        let system = Some("en-US".to_owned());
        self.sync_locale_from(system.as_deref(), cx);
    }

    pub(crate) fn sync_locale_from(&mut self, system: Option<&str>, cx: &mut Context<Self>) {
        let resolved = self.settings.language.resolve(system);
        if self.resolved_locale == resolved {
            return;
        }
        self.resolved_locale = resolved;
        runner_app::i18n::set_active_locale(resolved);
        self.revisions.locale = self.revisions.locale.wrapping_add(1);
        cx.set_menus(crate::app_menus());
        cx.refresh_windows();
        cx.notify();
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
        let language = self.settings.language;
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
        self.desired_skill_enabled
            .store(self.settings.runner_skill_enabled, Ordering::Release);
        self.command_install_initialized
            .store(self.settings.initialized_command_install, Ordering::Release);
        if agent_settings
            != (
                self.settings.enabled_agents.clone(),
                self.settings.disabled_agents.clone(),
            )
        {
            self.refresh(StoreRefreshKind::Runtimes, cx);
        }
        if self.settings.language != language {
            self.sync_locale(cx);
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

    pub(crate) fn refresh_roles_inner(&mut self) {
        match self.client.role_list() {
            Ok(roles) => {
                self.roles = roles;
                self.revisions.roles = self.revisions.roles.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    pub(crate) fn refresh_crews_inner(&mut self) {
        let result = self.client.crew_list_all();
        match result {
            Ok(crews) => {
                self.crews = crews;
                self.revisions.crews = self.revisions.crews.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    pub(crate) fn onboarding_completed(&self) -> bool {
        self.onboarding.completed
    }

    pub(crate) fn has_crews(&self) -> bool {
        self.onboarding.has_crews || !self.crews.is_empty()
    }

    pub(crate) fn refresh_onboarding_inner(&mut self) {
        if let Ok(next) = self.client.onboarding_status() {
            if self.onboarding != next {
                self.onboarding = next;
                self.revisions.onboarding = self.revisions.onboarding.wrapping_add(1);
            }
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
                match mission_sessions_and_agent_ids(&self.client, &missions) {
                    Ok((ids, sessions)) => {
                        self.mission_agent_ids = ids;
                        self.mission_sessions = sessions;
                    }
                    Err(error) => self.record_error(error.to_string()),
                }
                self.missions = missions;
                self.revisions.missions = self.revisions.missions.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    pub(crate) fn live_terminal_cwd(&self, session_id: &str) -> Option<&Path> {
        self.terminal_labels
            .get(session_id)
            .and_then(|(_, cwd, _)| cwd.as_deref())
    }

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
        self.claude_accounts = self.client.claude_accounts().unwrap_or_default();
        if self.claude_accounts.multiple() {
            self.claude_account_usage = self.client.claude_account_usage().unwrap_or_default();
        } else {
            self.claude_account_usage.clear();
        }
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
                *current = format!("{}\n{error}", current.resolve()).into();
            } else {
                self.error = Some(error.into());
            }
        } else {
            self.error = Some(error.into());
        }
        self.revisions.error = self.revisions.error.wrapping_add(1);
    }
}

type MissionSessionIndex = (
    BTreeSet<String>,
    BTreeMap<String, Vec<runner_core::protocol::SessionRow>>,
);

fn newer_viewed_timestamp(candidate: Option<&str>, current: Option<&str>) -> bool {
    let Some(candidate) = candidate else {
        return false;
    };
    let Some(current) = current else {
        return true;
    };
    match (
        chrono::DateTime::parse_from_rfc3339(candidate),
        chrono::DateTime::parse_from_rfc3339(current),
    ) {
        (Ok(cand), Ok(curr)) => cand > curr,
        _ => candidate > current,
    }
}

struct RuntimeRefreshInputs {
    settings: AppSettings,
    home_dir: Option<PathBuf>,
    app_data_dir: PathBuf,
    command_install_support: Option<CommandInstallSupport>,
    desired_skill_enabled: Arc<AtomicBool>,
    skill_status_seq: Arc<AtomicU64>,
    skill_reconcile_lock: Arc<Mutex<()>>,
    command_install_initialized: Arc<AtomicBool>,
    command_status_seq: Arc<AtomicU64>,
    command_reconcile_lock: Arc<Mutex<()>>,
}

struct RuntimeInitSnapshot {
    mcp_completed: bool,
    skill_outcome: skill_defaults::SkillReconcileOutcome,
    command_default: command_default::CommandDefaultSnapshot,
}

struct ActivityRefreshSnapshot {
    counts: runner_core::protocol::ClientResult<
        std::collections::HashMap<runner_core::protocol::model::Runtime, usize>,
    >,
    activity: BTreeMap<String, SessionActivityState>,
    statuses: runner_core::protocol::ClientResult<
        BTreeMap<String, runner_core::protocol::status::AgentStatus>,
    >,
}

struct RenderRefreshSnapshot {
    window_entries: Vec<runner_core::protocol::WindowEntry>,
    usage: runner_core::protocol::UsageSnapshot,
    claude_accounts: runner_core::protocol::ClaudeAccounts,
    claude_account_usage: BTreeMap<String, runner_core::protocol::RuntimeUsage>,
    file_link_environment: (String, std::collections::HashMap<String, bool>),
}

type SessionsRefreshResult = runner_core::protocol::ClientResult<(
    Vec<DirectSessionEntry>,
    BTreeMap<String, DirectSessionEntry>,
)>;

type MissionsRefreshResult = runner_core::protocol::ClientResult<(
    Vec<MissionSummary>,
    runner_core::protocol::ClientResult<MissionSessionIndex>,
)>;

struct FetchedStoreRefresh {
    onboarding: Option<runner_core::protocol::OnboardingStatus>,
    runtime_init: Option<RuntimeInitSnapshot>,
    roles: Option<runner_core::protocol::ClientResult<Vec<Role>>>,
    crews: Option<runner_core::protocol::ClientResult<Vec<CrewListItem>>>,
    sessions: Option<SessionsRefreshResult>,
    nodes: Option<runner_core::protocol::ClientResult<Vec<NodeRow>>>,
    activity: Option<ActivityRefreshSnapshot>,
    projects: Option<runner_core::protocol::ClientResult<Vec<ProjectRow>>>,
    missions: Option<MissionsRefreshResult>,
    render: RenderRefreshSnapshot,
}

fn fetch_store_refresh(
    client: DaemonClient,
    refresh: StoreRefreshKind,
    entity_refresh: EntityRefreshKind,
    runtime_inputs: Option<RuntimeRefreshInputs>,
) -> FetchedStoreRefresh {
    let onboarding = if matches!(
        refresh,
        StoreRefreshKind::Onboarding | StoreRefreshKind::All
    ) || entity_refresh.crews()
    {
        client.onboarding_status().ok()
    } else {
        None
    };

    let runtime_init = runtime_inputs.map(|inputs| {
        let mcp_completed = mcp_removal::compute_mcp_removal(
            &client,
            inputs.settings.clone(),
            inputs.home_dir.as_deref(),
            &inputs.app_data_dir,
        );
        let skill_outcome = skill_defaults::reconcile_skill_defaults_serialized(
            &client,
            inputs.home_dir.as_deref(),
            &inputs.app_data_dir,
            &inputs.desired_skill_enabled,
            &inputs.skill_status_seq,
            &inputs.skill_reconcile_lock,
        );
        let command_default = command_default::reconcile_command_default_serialized(
            &client,
            inputs.home_dir.as_deref(),
            inputs.app_data_dir.as_path(),
            inputs.command_install_support.as_ref(),
            &inputs.command_install_initialized,
            &inputs.command_status_seq,
            &inputs.command_reconcile_lock,
        );
        RuntimeInitSnapshot {
            mcp_completed,
            skill_outcome,
            command_default,
        }
    });

    let roles = (entity_refresh == EntityRefreshKind::All).then(|| client.role_list());
    let crews = entity_refresh.crews().then(|| client.crew_list_all());
    let sessions = (refresh == StoreRefreshKind::All).then(|| {
        client
            .session_list_recent_direct()
            .map(|sessions| (sessions, client.session_details().unwrap_or_default()))
    });
    let nodes = matches!(refresh, StoreRefreshKind::Nodes | StoreRefreshKind::All)
        .then(|| client.node_list());
    let activity = matches!(
        refresh,
        StoreRefreshKind::Activity | StoreRefreshKind::Nodes | StoreRefreshKind::All
    )
    .then(|| ActivityRefreshSnapshot {
        counts: client.live_session_counts(),
        activity: client.session_activity_snapshot().unwrap_or_default(),
        statuses: client.session_status_snapshot(),
    });
    let projects = (refresh == StoreRefreshKind::All).then(|| client.project_list());
    let missions =
        matches!(refresh, StoreRefreshKind::Missions | StoreRefreshKind::All).then(|| {
            client.mission_list_summary_impl(None).map(|missions| {
                let idx = mission_sessions_and_agent_ids(&client, &missions);
                (missions, idx)
            })
        });

    let window_entries = client.window_snapshot().unwrap_or_default();
    let usage = client.usage_snapshot().unwrap_or_default();
    let claude_accounts = client.claude_accounts().unwrap_or_default();
    let claude_account_usage = if claude_accounts.multiple() {
        client.claude_account_usage().unwrap_or_default()
    } else {
        BTreeMap::new()
    };
    let file_link_environment = client.file_link_environment().unwrap_or_default();

    FetchedStoreRefresh {
        onboarding,
        runtime_init,
        roles,
        crews,
        sessions,
        nodes,
        activity,
        projects,
        missions,
        render: RenderRefreshSnapshot {
            window_entries,
            usage,
            claude_accounts,
            claude_account_usage,
            file_link_environment,
        },
    }
}

fn mission_sessions_and_agent_ids(
    client: &DaemonClient,
    missions: &[MissionSummary],
) -> runner_core::protocol::ClientResult<MissionSessionIndex> {
    let mut ids = std::collections::BTreeSet::new();
    let mut by_mission = BTreeMap::new();
    for mission in missions {
        let rows = client.session_list(&mission.mission.id)?;
        for row in &rows {
            if row.runtime != "shell" {
                ids.insert(row.session.id.clone());
            }
        }
        by_mission.insert(mission.mission.id.clone(), rows);
    }
    Ok((ids, by_mission))
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
mod tests;

#[derive(Clone)]
enum DaemonEvent {
    Disconnected(runner_app::lifecycle::DaemonNotice),
    Reconnected,
    Lagged,
    Restarted(usize),
    FilesRootChanged(PathBuf),
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
        Arc::new(db::open_fixture_pool(&root.join("runner.db")).unwrap()),
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
        #[cfg(target_os = "macos")]
        let updater = cx.new(|cx| crate::Updater::new(false, cx));
        #[cfg(any(windows, target_os = "linux"))]
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
         INSERT INTO slots (id,crew_id,role_id,slot_handle,position,lead,added_at)
         VALUES ('agent-slot','test-crew','test-shell-role','agent',0,1,'2026-10-06T00:00:00Z'),
                ('shell-slot','test-crew','test-shell-role','shell',1,0,'2026-10-06T00:00:00Z');
         INSERT INTO sessions (id, role_id, mission_id, slot_id, status, agent_runtime, agent_command)
         VALUES ('direct-agent', 'test-shell-role', NULL, NULL, 'running', 'codex', 'codex'),
                ('busy-shell', 'test-shell-role', NULL, NULL, 'running', 'shell', 'shell'),
                ('mission-agent', 'test-shell-role', 'test-mission', 'agent-slot', 'running', 'codex', 'codex'),
                ('mission-shell', 'test-shell-role', 'test-mission', 'shell-slot', 'running', 'shell', 'shell');"
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

#[cfg(test)]
mod roster_change_tests {
    use super::*;
    #[test]
    fn joins_and_leaves_refresh_missions() {
        for kind in ["role_joined", "role_removed"] {
            let event = runner_core::protocol::ClientEvent {
                name: "event/appended".into(),
                payload: serde_json::json!({"event":{"type":kind}}),
            };
            assert_eq!(
                StoreRefreshKind::for_event(&event),
                Some(StoreRefreshKind::Missions)
            );
        }
    }
}

#[test]
fn usage_controls_invalidate_shell_but_alert_history_does_not() {
    let settings = AppSettings::default();
    let before = ShellSettingsSnapshot::from(&settings);
    let changes: [fn(&mut AppSettings); 7] = [
        |s| s.usage.sidebar = false,
        |s| s.usage.codex = false,
        |s| s.usage.claude_code = false,
        |s| s.usage.antigravity = false,
        |s| s.usage.five_hour = false,
        |s| s.notifications.usage.near_limit = false,
        |s| s.notifications.usage.threshold = 50,
    ];
    for change in changes {
        let mut next = settings.clone();
        change(&mut next);
        assert_ne!(ShellSettingsSnapshot::from(&next), before);
    }
    let mut next = settings;
    next.notifications
        .usage
        .alerted
        .push(crate::app_settings::UsageAlertPeriod {
            runtime: runner_core::Runtime::Codex,
            account: String::new(),
            window: "Week".into(),
            resets_at: None,
        });
    assert_eq!(ShellSettingsSnapshot::from(&next), before);
}
