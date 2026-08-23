use std::collections::BTreeMap;
use std::hash::{Hash as _, Hasher as _};
use std::path::PathBuf;
use std::sync::Arc;

use futures::StreamExt as _;
use gpui::{App, AppContext as _, Context, Entity, Global};
use runner_backend::events::AppEvent;
use runner_backend::model::Runner;
use runner_backend::ops::crew::CrewListItem;
use runner_backend::ops::mission::MissionSummary;
use runner_backend::ops::session::DirectSessionEntry;
use runner_backend::repo::node::NodeRow;
use runner_backend::repo::project::ProjectRow;
use runner_backend::session::manager::SessionActivityState;
use runner_backend::AppCore;
use runner_terminal::terminal::TerminalBridge;

use crate::app_settings::{AppSettings, TerminalCursorStyle, TerminalFontFamily, TerminalTheme};

#[derive(Clone)]
pub(crate) struct GlobalAppStore(pub(crate) Entity<AppStore>);

impl Global for GlobalAppStore {}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StoreRefreshKind {
    Activity,
    Nodes,
    Missions,
    All,
}

impl StoreRefreshKind {
    pub(crate) fn for_event(event: &AppEvent) -> Option<Self> {
        match event.name {
            "session/status" => Some(Self::Activity),
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
                                | "runner_status"
                        )
                    }) =>
            {
                Some(Self::Missions)
            }
            "session/exit" | "session/spawned" | "session/archived" | "session/updated"
            | "runner/activity" | "runner/changed" | "crew/changed" | "slot/changed"
            | "mission/changed" | "project/changed" => Some(Self::All),
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
    pub(crate) runners: u64,
    pub(crate) runner_surfaces: u64,
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
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct StoreReactions {
    pub(crate) sync_error: bool,
    pub(crate) reload_tabs: bool,
    pub(crate) prune_sidebar: bool,
    pub(crate) prune_window_state: bool,
    pub(crate) reload_runner_surfaces: bool,
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
            reload_runner_surfaces: self.runner_surfaces != previous.runner_surfaces,
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
    theme: TerminalTheme,
    font_family: TerminalFontFamily,
    font_size: u16,
    cursor_style: TerminalCursorStyle,
}

impl From<&AppSettings> for TerminalSettingsSnapshot {
    fn from(settings: &AppSettings) -> Self {
        Self {
            theme: settings.terminal_theme,
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
        settings.mission_rail_view.hash(&mut hasher);
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
        std::mem::discriminant(&settings.app_font_family).hash(&mut hasher);
        settings.app_zoom.to_bits().hash(&mut hasher);
        std::mem::discriminant(&settings.terminal_theme).hash(&mut hasher);
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
        settings.keymap_overrides.hash(&mut hasher);
        Self(hasher.finish())
    }
}

pub(crate) struct AppStore {
    pub(crate) core: AppCore,
    pub(crate) bridge: Arc<TerminalBridge>,
    pub(crate) sessions: Vec<DirectSessionEntry>,
    pub(crate) runners: Vec<Runner>,
    pub(crate) crews: Vec<CrewListItem>,
    pub(crate) nodes: Vec<NodeRow>,
    pub(crate) projects: Vec<ProjectRow>,
    pub(crate) missions: Vec<MissionSummary>,
    pub(crate) session_activity: BTreeMap<String, SessionActivityState>,
    pub(crate) settings: AppSettings,
    settings_path: PathBuf,
    pub(crate) revisions: StoreRevisions,
    pub(crate) error: Option<String>,
    collecting_startup_errors: bool,
}

impl AppStore {
    pub(crate) fn new(
        core: AppCore,
        settings_path: PathBuf,
        settings: AppSettings,
        settings_error: Option<String>,
        cx: &mut Context<Self>,
    ) -> Self {
        let (wake_tx, mut wake_rx) = futures::channel::mpsc::unbounded::<()>();
        let waker: Arc<dyn Fn() + Send + Sync> = Arc::new(move || {
            let _ = wake_tx.unbounded_send(());
        });
        let bridge =
            TerminalBridge::new(core.clone(), Arc::clone(&waker)).expect("start event bridge");

        cx.spawn(async move |weak, cx| {
            while wake_rx.next().await.is_some() {
                while wake_rx.try_recv().is_ok() {}
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

        let (event_tx, mut event_rx) =
            futures::channel::mpsc::unbounded::<(StoreRefreshKind, EntityRefreshKind)>();
        let mut events = core.events.subscribe();
        cx.background_spawn(async move {
            loop {
                let refresh = match events.recv().await {
                    Ok(event) => StoreRefreshKind::for_event(&event)
                        .map(|store| (store, EntityRefreshKind::for_event(&event))),
                    Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {
                        Some((StoreRefreshKind::All, EntityRefreshKind::All))
                    }
                    Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                };
                if refresh.is_some_and(|refresh| event_tx.unbounded_send(refresh).is_err()) {
                    break;
                }
            }
        })
        .detach();
        cx.spawn(async move |weak, cx| {
            while let Some((mut refresh, mut entity_refresh)) = event_rx.next().await {
                while let Ok(next) = event_rx.try_recv() {
                    refresh = refresh.merge(next.0);
                    entity_refresh = entity_refresh.merge(next.1);
                }
                if weak
                    .update(cx, |this, cx| {
                        this.refresh(refresh, cx);
                        if entity_refresh.runners() {
                            this.revisions.runner_surfaces =
                                this.revisions.runner_surfaces.wrapping_add(1);
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

        let mut store = Self {
            core,
            bridge,
            sessions: Vec::new(),
            runners: Vec::new(),
            crews: Vec::new(),
            nodes: Vec::new(),
            projects: Vec::new(),
            missions: Vec::new(),
            session_activity: BTreeMap::new(),
            settings,
            settings_path,
            revisions: StoreRevisions::default(),
            error: None,
            collecting_startup_errors: true,
        };
        if let Some(error) = settings_error {
            store.record_error(error);
        }
        store.refresh_sessions_inner();
        store.refresh_runners_inner();
        store.refresh_crews_inner();
        store.refresh_nodes_inner();
        store.refresh_projects_inner();
        store.refresh_missions_blocking_inner();
        store.refresh_activity_inner();
        store.collecting_startup_errors = false;
        store
    }

    pub(crate) fn refresh(&mut self, refresh: StoreRefreshKind, cx: &mut Context<Self>) {
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
            let core = self.core.clone();
            cx.spawn(async move |weak, cx| {
                let result =
                    runner_backend::ops::mission::mission_list_summary_impl(&core, None).await;
                let _ = weak.update(cx, |this, cx| {
                    match result {
                        Ok(missions) => {
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
        cx.notify();
    }

    pub(crate) fn refresh_sessions(&mut self, cx: &mut Context<Self>) {
        self.refresh_sessions_inner();
        cx.notify();
    }

    pub(crate) fn refresh_nodes(
        &mut self,
        cx: &mut Context<Self>,
    ) -> runner_backend::error::Result<()> {
        match runner_backend::ops::node::node_list(&self.core) {
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

    pub(crate) fn replace_runners(&mut self, runners: Vec<Runner>, cx: &mut Context<Self>) {
        self.runners = runners;
        self.revisions.runners = self.revisions.runners.wrapping_add(1);
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

    pub(crate) fn update_settings(
        &mut self,
        update: impl FnOnce(&mut AppSettings) -> bool,
        persist: bool,
        cx: &mut Context<Self>,
    ) -> bool {
        let terminal_settings = TerminalSettingsSnapshot::from(&self.settings);
        let mission_settings = MissionSettingsSnapshot::from(&self.settings);
        let shell_settings = ShellSettingsSnapshot::from(&self.settings);
        if !update(&mut self.settings) {
            return false;
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
        match runner_backend::ops::session::session_list_recent_direct(&self.core) {
            Ok(sessions) => {
                self.sessions = sessions;
                self.revisions.sessions = self.revisions.sessions.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    fn refresh_runners_inner(&mut self) {
        match runner_backend::ops::runner::runner_list(&self.core) {
            Ok(runners) => {
                self.runners = runners;
                self.revisions.runners = self.revisions.runners.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    fn refresh_crews_inner(&mut self) {
        let result = self
            .core
            .db
            .get()
            .map_err(runner_backend::error::Error::from)
            .and_then(|conn| runner_backend::ops::crew::list(&conn));
        match result {
            Ok(crews) => {
                self.crews = crews;
                self.revisions.crews = self.revisions.crews.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    fn refresh_nodes_inner(&mut self) {
        match runner_backend::ops::node::node_list(&self.core) {
            Ok(nodes) => {
                self.nodes = nodes;
                self.revisions.nodes = self.revisions.nodes.wrapping_add(1);
                self.revisions.tab_rows = self.revisions.tab_rows.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    fn refresh_projects_inner(&mut self) {
        match runner_backend::ops::project::project_list(&self.core) {
            Ok(projects) => {
                self.projects = projects;
                self.revisions.projects = self.revisions.projects.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    fn refresh_missions_blocking_inner(&mut self) {
        match futures::executor::block_on(runner_backend::ops::mission::mission_list_summary_impl(
            &self.core, None,
        )) {
            Ok(missions) => {
                self.missions = missions;
                self.revisions.missions = self.revisions.missions.wrapping_add(1);
            }
            Err(error) => self.record_error(error.to_string()),
        }
    }

    fn refresh_activity_inner(&mut self) {
        self.session_activity = runner_backend::ops::session::session_activity_snapshot(&self.core);
        self.revisions.activity = self.revisions.activity.wrapping_add(1);
    }

    fn record_error(&mut self, error: String) {
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum EntityRefreshKind {
    None,
    Runners,
    All,
}

impl EntityRefreshKind {
    fn for_event(event: &AppEvent) -> Self {
        match event.name {
            "runner/activity" => Self::Runners,
            "runner/changed" | "crew/changed" | "slot/changed" => Self::All,
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

    fn runners(self) -> bool {
        matches!(self, Self::Runners | Self::All)
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

    fn event(name: &'static str) -> AppEvent {
        AppEvent {
            name,
            payload: serde_json::Value::Null,
        }
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
    fn entity_refresh_events_match_runner_and_crew_dependencies() {
        assert_eq!(
            EntityRefreshKind::for_event(&event("runner/activity")),
            EntityRefreshKind::Runners
        );
        for name in ["runner/changed", "crew/changed", "slot/changed"] {
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
        use EntityRefreshKind::{All, None, Runners};

        for (left, right, expected) in [
            (None, None, None),
            (None, Runners, Runners),
            (None, All, All),
            (Runners, None, Runners),
            (Runners, Runners, Runners),
            (Runners, All, All),
            (All, None, All),
            (All, Runners, All),
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
    fn runner_data_and_surface_reload_revisions_are_independent() {
        let before = StoreRevisions::default();
        let mut after_data = before;
        after_data.runners = 1;
        assert!(!after_data.reactions_since(before).reload_runner_surfaces);

        let mut after_event = before;
        after_event.runner_surfaces = 1;
        assert!(after_event.reactions_since(before).reload_runner_surfaces);
    }

    #[test]
    fn terminal_settings_snapshot_ignores_unrelated_preferences() {
        let before = AppSettings::default();
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
        let before = AppSettings::default();
        let mut after = before.clone();
        after.mission_rail_width += 1.;
        after.mission_rail_view = "meta".into();
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
