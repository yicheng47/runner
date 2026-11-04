use super::*;
use crate::surfaces::front_page::model::{
    MissionMemberDisplay, MissionMemberSession, WorkspaceMission,
};
use crate::surfaces::front_page::FrontPageClient;
use crate::surfaces::remote_client::{feed::MissionFeed, model::ClientState};
use runner_core::companion::{FeedContent, FeedEntry, MissionState, Overview};

#[derive(Clone)]
struct SavedPresentation {
    layout: MissionLayout,
    active: MissionTab,
    open_handles: Vec<String>,
    selected_handle: Option<String>,
    rail: MissionRailView,
}

pub(super) struct RemoteMission {
    pub(super) settings: Entity<AppSettings>,
    pub(super) terminal_sizes: HashMap<String, (u16, u16)>,
    pub(super) overview: Option<Overview>,
    presentations: HashMap<String, SavedPresentation>,
    pub(super) feed: Option<Entity<MissionFeed>>,
    pub(super) feed_subscription: Option<Subscription>,
    feed_ids: Vec<String>,
    pending: HashSet<String>,
    pub(super) shell_pending: bool,
    pub(super) shell_started: Arc<std::sync::Mutex<Option<String>>>,
    pub(super) drawer_expected: Option<(Vec<String>, Vec<String>)>,
    pub(super) activate_shell: Option<String>,
    dialog_focus_reset: bool,
    selected_handle: Option<String>,
}

impl MissionWorkspace {
    pub(crate) fn new_remote(
        window_label: String,
        shell: WeakEntity<NativeRoot>,
        app_store: Entity<AppStore>,
        client: FrontPageClient,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let settings = app_store.read(cx).settings.clone();
        let mut workspace = Self::new(window_label, shell, app_store, window, cx);
        workspace.sidebar_collapsed = false;
        workspace.client = client;
        workspace.presentation = MissionPresentation::new_remote(settings, cx);
        workspace
    }

    pub(crate) fn set_remote_sidebar_width(&mut self, width: f32, cx: &mut Context<Self>) {
        self.presentation.set_sidebar_width(width, cx);
        cx.notify();
    }

    pub(crate) fn set_remote_terminal_sizes(&mut self, sizes: HashMap<String, (u16, u16)>) {
        self.presentation.set_terminal_sizes(sizes);
    }

    pub(crate) fn sync_remote(
        &mut self,
        client: FrontPageClient,
        mission_id: Option<String>,
        overview: Option<Overview>,
        clients: Entity<ClientState>,
        cx: &mut Context<Self>,
    ) {
        let previous = self.members();
        let mut open_handles = self
            .open_tabs
            .iter()
            .filter_map(|id| {
                previous
                    .iter()
                    .find(|member| &member.session.id == id)
                    .map(|member| member.handle.clone())
            })
            .collect::<Vec<_>>();
        let changed = self.mission_id != mission_id;
        let target_changed = self.client.target != client.target;
        if changed || target_changed {
            let had_dialog = self.rename_modal.is_some()
                || self.stop_all_confirm
                || self.restart_confirm.is_some()
                || self.remove_confirm.is_some();
            self.rename_modal = None;
            self.stop_all_confirm = false;
            self.restart_confirm = None;
            self.remove_confirm = None;
            self.presentation.reset_dialog_focus(had_dialog);
        }
        let mut restored = false;
        if changed {
            let saved = self.presentation.switch_mission(
                self.mission_id.as_deref(),
                mission_id.as_deref(),
                SavedPresentation {
                    layout: self.layout.clone(),
                    active: self.active_tab.clone(),
                    open_handles: open_handles.clone(),
                    selected_handle: None,
                    rail: self.rail_view,
                },
            );
            if let Some(id) = mission_id.clone() {
                self.prepare_mission(id.clone(), MissionRailView::Roles);
                if let Some(saved) = saved {
                    self.layout = saved.layout;
                    self.active_tab = saved.active;
                    self.rail_view = saved.rail;
                    self.presentation.set_selected_handle(saved.selected_handle);
                    open_handles = saved.open_handles;
                    restored = true;
                }
            } else {
                self.generation += 1;
                self.active = false;
                self.mission_id = None;
            }
        }
        if !self.presentation.is_remote() {
            return;
        }
        if target_changed {
            self.generation += 1;
            self.stopping = false;
            self.resuming = false;
            self.archiving = false;
            self.composer_posting = false;
            self.submitting_asks.clear();
            self.slot_actions.clear();
        }
        self.client = client.clone();
        self.presentation.update_overview(overview, target_changed);
        self.active = mission_id.is_some();
        self.loading = false;
        let members = self.members();
        self.open_tabs = if changed && !restored {
            members
                .iter()
                .map(|member| member.session.id.clone())
                .collect()
        } else {
            let mut tabs = open_handles
                .iter()
                .filter_map(|handle| {
                    members
                        .iter()
                        .find(|member| &member.handle == handle)
                        .map(|member| member.session.id.clone())
                })
                .collect::<Vec<_>>();
            if !changed {
                tabs.extend(
                    members
                        .iter()
                        .filter(|member| !previous.iter().any(|old| old.handle == member.handle))
                        .map(|member| member.session.id.clone()),
                );
            }
            tabs
        };
        if let MissionTab::Session(id) = &self.active_tab {
            if !members.iter().any(|member| &member.session.id == id) {
                self.active_tab = self
                    .presentation
                    .selected_handle()
                    .and_then(|handle| members.iter().find(|member| &member.handle == handle))
                    .map(|member| MissionTab::Session(member.session.id.clone()))
                    .unwrap_or(MissionTab::Feed);
            }
        }
        let selected_handle = match &self.active_tab {
            MissionTab::Session(id) => members
                .iter()
                .find(|member| &member.session.id == id)
                .map(|member| member.handle.clone()),
            MissionTab::Feed => None,
        };
        self.presentation.set_selected_handle(selected_handle);
        self.presentation
            .sync_drawer(self.mission_id.as_deref(), &mut self.layout);
        self.presentation.bind_feed(client, mission_id, clients, cx);
        self.sync_remote_feed(cx);
        cx.notify();
    }

    pub(crate) fn restore_remote_dialog_focus(
        &mut self,
        fallback: Option<&gpui::FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.presentation.take_dialog_focus_reset()
            || self.rename_modal.is_some()
            || self.stop_all_confirm
            || self.restart_confirm.is_some()
            || self.remove_confirm.is_some()
        {
            return;
        }
        let Some(fallback) = fallback else {
            return;
        };
        if self.active {
            self.focus_active_mission_terminal(window, cx);
        } else {
            fallback.focus(window, cx);
        }
    }

    pub(super) fn remote_summary(&self) -> Option<&runner_core::companion::Mission> {
        self.presentation.summary(self.mission_id.as_deref())
    }

    pub(super) fn mission_view(&self) -> Option<WorkspaceMission> {
        self.presentation.mission_view(
            self.mission_id.as_deref(),
            self.mission.as_ref(),
            &self.sessions,
        )
    }

    pub(super) fn members(&self) -> Vec<MissionMemberDisplay> {
        self.presentation
            .members(self.mission_id.as_deref(), &self.sessions)
    }

    pub(super) fn remote_supports(&self, features: &[&str]) -> bool {
        self.client.supports(features) && (!self.client.is_remote() || self.active)
    }

    pub(crate) fn remote_visible_sessions(&self) -> Vec<String> {
        if !self.active {
            return Vec::new();
        }
        let mut ids = match &self.active_tab {
            MissionTab::Session(id) => vec![id.clone()],
            MissionTab::Feed => Vec::new(),
        };
        if self.layout.drawer.open() {
            if let Some(id) = self.layout.drawer.active_shell() {
                ids.push(id.into());
            }
        }
        ids
    }

    fn sync_remote_feed(&mut self, cx: &mut Context<Self>) {
        if let Some((appended, events)) = self
            .presentation
            .feed_events(self.mission_id.as_deref(), cx)
        {
            for event in appended {
                self.handle_feed_append(&event);
            }
            self.events = events;
        }
        self.rebuild_event_projection();
        cx.notify();
    }

    pub(super) fn append_remote_questions(&mut self) {
        let Some(mission) = self.remote_summary().cloned() else {
            return;
        };
        for ask in mission.asks {
            let represented = self.events.iter().any(|event| {
                event
                    .signal_type
                    .as_ref()
                    .is_some_and(|kind| kind.as_str() == "human_question")
                    && event
                        .payload
                        .get("question_id")
                        .and_then(serde_json::Value::as_str)
                        .unwrap_or(&event.id)
                        == ask.question
            });
            if !represented {
                self.feed_blocks.push(FeedBlock::PendingAsk(
                    crate::surfaces::front_page::model::MissionQuestionDisplay {
                        id: ask.question.clone(),
                        markdown_id: ask.question,
                        asker: ask.from,
                        prompt: ask.prompt,
                        choices: ask
                            .choices
                            .filter(|choices| !choices.is_empty())
                            .unwrap_or_else(|| vec!["yes".into(), "no".into()]),
                        chain: crate::i18n::text("ui.you").into(),
                        time: None,
                    },
                ));
            }
        }
    }

    pub(super) fn remote_terminal_content(&self, id: &str, cx: &mut Context<Self>) -> AnyElement {
        self.shell
            .upgrade()
            .map(|shell| {
                shell.update(cx, |shell, cx| {
                    shell.remote_client.render_remote_terminal_content(id, cx)
                })
            })
            .unwrap_or_else(|| div().into_any_element())
    }

    pub(super) fn remote_navigation_changed(&self, cx: &mut Context<Self>) {
        if !self.presentation.is_remote() {
            return;
        }
        let shell = self.shell.clone();
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.remote_client.attach_remote_panes(cx);
                shell.remote_client.mark_remote_stopped_sessions_viewed(cx);
                cx.notify();
            });
        });
    }
}

fn remote_event(mission: &str, entry: &FeedEntry) -> Option<Event> {
    let ts = chrono::DateTime::parse_from_rfc3339(&entry.ts)
        .ok()?
        .with_timezone(&chrono::Utc);
    let (kind, signal_type, payload) = match &entry.content {
        FeedContent::Message { text } => {
            (EventKind::Message, None, serde_json::json!({"text": text}))
        }
        FeedContent::Ask {
            question,
            prompt,
            choices,
        } => (
            EventKind::Signal,
            Some("human_question".into()),
            serde_json::json!({"question_id": question, "prompt": prompt, "choices": choices, "remote_from": entry.from}),
        ),
        FeedContent::Answer { question, text } => (
            EventKind::Signal,
            Some("human_response".into()),
            serde_json::json!({"question_id": question, "choice": text}),
        ),
        FeedContent::Signal { signal, payload } => (
            EventKind::Signal,
            Some(signal.as_str().into()),
            payload.clone(),
        ),
        FeedContent::Unknown(payload) => {
            (EventKind::Signal, Some("unknown".into()), payload.clone())
        }
    };
    Some(Event {
        id: entry.id.clone(),
        ts,
        crew_id: None,
        mission_id: mission.into(),
        kind,
        from: entry.from.clone(),
        to: runner_core::Recipients::new(entry.to.clone()),
        signal_type,
        payload,
    })
}

pub(super) struct MissionAction {
    client: FrontPageClient,
    generation: u64,
    key: String,
    remote: bool,
}

impl MissionWorkspace {
    pub(super) fn prepare_mission_action(
        &mut self,
        feature: &str,
        key: String,
    ) -> Option<MissionAction> {
        self.presentation
            .prepare_action(&self.client, self.active, self.generation, feature, key)
    }

    pub(super) fn complete_mission_action<T>(
        &mut self,
        action: MissionAction,
        result: Result<T, String>,
        remote_ids: impl FnOnce(T) -> Vec<String>,
        cx: &mut Context<Self>,
    ) -> Option<Result<T, String>> {
        if !action.remote {
            return Some(result);
        }
        let done = self.presentation.complete_action(
            action,
            &self.client,
            self.generation,
            self.mission_id.as_deref(),
            result,
            remote_ids,
        )?;
        let MissionCompletion {
            key,
            client: guard,
            generation,
            result,
        } = done;
        match key.as_str() {
            "stopMission" => self.stopping = false,
            "resumeMission" => self.resuming = false,
            "archive" => self.archiving = false,
            "rename" => {
                if let Some(modal) = &mut self.rename_modal {
                    modal.submitting = false;
                    modal.error = result.as_ref().err().cloned();
                }
                if result.is_ok() {
                    self.rename_modal = None;
                }
            }
            id => {
                self.slot_actions.remove(id);
            }
        }
        if let Ok(ids) = &result {
            if !ids.is_empty() {
                let ids = ids.clone();
                let shell = self.shell.clone();
                let workspace = cx.weak_entity();
                cx.defer(move |cx| {
                    if !workspace
                        .upgrade()
                        .is_some_and(|workspace| workspace.read(cx).generation == generation)
                    {
                        return;
                    }
                    let _ = shell.update(cx, |shell, cx| {
                        if shell.remote_client.reopen_sessions(&guard.target, ids) {
                            shell.sync_remote_state(cx);
                        }
                    });
                });
            }
        }
        if key != "rename" {
            if let Err(error) = result {
                self.error = Some(action_failure(
                    crate::i18n::message("mission.update_action"),
                    error,
                ));
            }
        }

        cx.notify();
        None
    }
}

pub(super) enum MissionPresentation {
    Local,
    Remote(Box<RemoteMission>),
}

impl MissionPresentation {
    pub(super) fn is_remote(&self) -> bool {
        matches!(self, Self::Remote(_))
    }
    fn remote(&self) -> Option<&RemoteMission> {
        match self {
            Self::Local => None,
            Self::Remote(remote) => Some(remote),
        }
    }
    fn remote_mut(&mut self) -> Option<&mut RemoteMission> {
        match self {
            Self::Local => None,
            Self::Remote(remote) => Some(remote),
        }
    }
    /// Whether the host's tab holding `chat` is pinned; `None` when no tab
    /// holds it or the window is local.
    pub(super) fn chat_tab_pinned(&self, chat: &str) -> Option<bool> {
        fn holds(pane: &runner_core::companion::Pane, chat: &str) -> bool {
            match pane {
                runner_core::companion::Pane::Split { a, b, .. } => {
                    holds(a, chat) || holds(b, chat)
                }
                runner_core::companion::Pane::Leaf { session } => session.as_deref() == Some(chat),
                runner_core::companion::Pane::Unknown(_) => false,
            }
        }
        self.remote()?
            .overview
            .as_ref()?
            .tabs
            .iter()
            .find(|tab| holds(&tab.panes, chat))
            .map(|tab| tab.pinned)
    }
    pub(super) fn summary(
        &self,
        mission_id: Option<&str>,
    ) -> Option<&runner_core::companion::Mission> {
        let id = mission_id?;
        self.remote()?
            .overview
            .as_ref()?
            .missions
            .iter()
            .find(|mission| mission.id == id)
    }
    pub(super) fn mission_view(
        &self,
        mission_id: Option<&str>,
        local: Option<&Mission>,
        sessions: &[SessionRow],
    ) -> Option<WorkspaceMission> {
        if !self.is_remote() {
            return local.map(WorkspaceMission::from);
        }
        let mission = self.summary(mission_id)?;
        let members = self.members(mission_id, sessions);
        Some(WorkspaceMission {
            id: mission.id.clone(),
            title: mission.title.clone(),
            status: match mission.state {
                MissionState::Running => MissionStatus::Running,
                MissionState::Stopped | MissionState::Unrecognized(_) => MissionStatus::Completed,
            },
            cwd: members.iter().find_map(|member| member.session.cwd.clone()),
            project_id: mission.project.clone(),
            started_at: None,
            archived: false,
            pinned: mission.pinned,
            linked_chat: mission.linked_chat.clone().map(|chat| {
                runner_core::protocol::LinkedChat {
                    session_id: chat.session_id,
                    title: chat.title,
                    archived: chat.archived,
                    reports: chat.reports,
                }
            }),
        })
    }
    pub(super) fn members(
        &self,
        mission_id: Option<&str>,
        sessions: &[SessionRow],
    ) -> Vec<MissionMemberDisplay> {
        let Some(remote) = self.remote() else {
            return sessions.iter().map(MissionMemberDisplay::from).collect();
        };
        let Some(mission) = self.summary(mission_id) else {
            return Vec::new();
        };
        let Some(overview) = &remote.overview else {
            return Vec::new();
        };
        mission
            .members
            .iter()
            .map(|member| {
                let session = overview.sessions.get(&member.session);
                MissionMemberDisplay {
                    session: MissionMemberSession {
                        id: member.session.clone(),
                        status: session
                            .map(|session| match session.status.status.effective() {
                                runner_core::companion::Status::Stopped => SessionStatus::Stopped,
                                runner_core::companion::Status::Error => SessionStatus::Crashed,
                                _ => SessionStatus::Running,
                            })
                            .unwrap_or(SessionStatus::Stopped),
                        cwd: session.and_then(|session| session.cwd.clone()),
                    },
                    handle: member.handle.clone(),
                    runtime: session
                        .and_then(|session| session.runtime.clone())
                        .unwrap_or_default(),
                    live_title: session.map(|session| session.title.clone()),
                    agent_session_key: None,
                }
            })
            .collect()
    }
}

impl MissionPresentation {
    pub(super) fn settings<'a>(
        &self,
        app_store: &Entity<AppStore>,
        cx: &'a App,
    ) -> &'a AppSettings {
        self.remote()
            .map(|remote| remote.settings.read(cx))
            .unwrap_or(&app_store.read(cx).settings)
    }
    pub(super) fn update_app_settings(
        &self,
        app_store: &Entity<AppStore>,
        cx: &mut Context<MissionWorkspace>,
        persist: bool,
        update: impl FnOnce(&mut AppSettings) -> bool,
    ) -> bool {
        if let Some(remote) = self.remote() {
            return remote.settings.update(cx, |settings, _| update(settings));
        }
        app_store.update(cx, |store, store_cx| {
            store.update_settings(update, persist, store_cx)
        })
    }
    pub(super) fn drawer_session_entry(
        &self,
        app_store: &Entity<AppStore>,
        client: &FrontPageClient,
        session_id: &str,
        cx: &App,
    ) -> Option<crate::surfaces::front_page::model::SessionDisplay> {
        use crate::surfaces::front_page::model::SessionDisplay;
        if let Some(remote) = self.remote() {
            let session = remote.overview.as_ref()?.sessions.get(session_id)?;
            return Some(SessionDisplay::remote(
                session_id,
                session,
                false,
                false,
                client.supports(&["resume"]),
            ));
        }
        app_store
            .read(cx)
            .sessions
            .iter()
            .find(|entry| entry.session_id == session_id)
            .map(SessionDisplay::local)
    }
    pub(super) fn slot_status(
        &self,
        session_id: &str,
    ) -> Option<runner_core::protocol::status::AgentStatus> {
        let remote = self.remote()?;
        Some(
            remote
                .overview
                .as_ref()
                .and_then(|overview| overview.sessions.get(session_id))
                .map(|session| crate::surfaces::remote_client::model::agent_status(&session.status))
                .unwrap_or_default(),
        )
    }
}

impl MissionPresentation {
    pub(super) fn selected_handle(&self) -> Option<&String> {
        self.remote()?.selected_handle.as_ref()
    }
    pub(super) fn set_selected_handle(&mut self, handle: Option<String>) {
        if let Some(remote) = self.remote_mut() {
            remote.selected_handle = handle;
        }
    }
    pub(super) fn feed(&self) -> Option<Entity<MissionFeed>> {
        self.remote()?.feed.clone()
    }
    fn feed_events(
        &mut self,
        mission_id: Option<&str>,
        cx: &App,
    ) -> Option<(Vec<Event>, Vec<Event>)> {
        let remote = self.remote_mut()?;
        let entries = remote
            .feed
            .as_ref()
            .map(|feed| feed.read(cx).entries.clone())
            .unwrap_or_default();
        let ids = entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        if remote.feed_ids == ids {
            return None;
        }
        let appended = remote
            .feed_ids
            .last()
            .and_then(|id| entries.iter().position(|entry| &entry.id == id))
            .map(|tail| {
                entries[tail + 1..]
                    .iter()
                    .filter_map(|entry| remote_event(mission_id?, entry))
                    .collect()
            })
            .unwrap_or_default();
        remote.feed_ids = ids;
        let events = entries
            .iter()
            .filter_map(|entry| remote_event(mission_id?, entry))
            .collect();
        Some((appended, events))
    }
    pub(super) fn terminal_size(&self, tab: &MissionTab) -> Option<(u16, u16)> {
        let MissionTab::Session(id) = tab else {
            return None;
        };
        self.remote()?.terminal_sizes.get(id).copied()
    }
    pub(super) fn refresh_settings(
        &self,
        store: &Entity<AppStore>,
        cx: &mut Context<MissionWorkspace>,
    ) {
        let Some(remote) = self.remote() else {
            return;
        };
        let mut global = store.read(cx).settings.clone();
        let own = remote.settings.read(cx);
        global.sidebar_width = own.sidebar_width;
        global.mission_rail_width = own.mission_rail_width;
        global.mission_rail_open = own.mission_rail_open;
        global
            .last_mission_terminal_ids
            .clone_from(&own.last_mission_terminal_ids);
        remote.settings.update(cx, |settings, cx| {
            *settings = global;
            cx.notify();
        });
    }
}

struct MissionCompletion {
    key: String,
    client: FrontPageClient,
    generation: u64,
    result: Result<Vec<String>, String>,
}
impl MissionPresentation {
    fn switch_mission(
        &mut self,
        previous: Option<&str>,
        next: Option<&str>,
        mut snapshot: SavedPresentation,
    ) -> Option<SavedPresentation> {
        let remote = self.remote_mut()?;
        snapshot.selected_handle = remote.selected_handle.clone();
        if let Some(id) = previous {
            remote.presentations.insert(id.into(), snapshot);
        }
        remote.feed = None;
        remote.feed_subscription = None;
        remote.feed_ids.clear();
        remote.reset_actions();
        remote.selected_handle = None;
        next.and_then(|id| remote.presentations.get(id)).cloned()
    }
    fn reset_dialog_focus(&mut self, had_dialog: bool) {
        if let Some(remote) = self.remote_mut() {
            remote.dialog_focus_reset |= had_dialog;
        }
    }
    fn take_dialog_focus_reset(&mut self) -> bool {
        self.remote_mut()
            .is_some_and(|remote| std::mem::take(&mut remote.dialog_focus_reset))
    }
    fn update_overview(&mut self, overview: Option<Overview>, changed: bool) {
        if let Some(remote) = self.remote_mut() {
            if changed {
                remote.reset_actions();
            }
            remote.overview = overview;
        }
    }
    fn sync_drawer(&mut self, mission_id: Option<&str>, layout: &mut MissionLayout) {
        let Some(summary) = self.summary(mission_id).cloned() else {
            return;
        };
        let remote = self.remote_mut().unwrap();
        if remote
            .drawer_expected
            .as_ref()
            .is_some_and(|(before, expected)| {
                summary.shells == *expected || summary.shells != *before
            })
        {
            remote.drawer_expected = None;
        }
        let was_open = layout.drawer.open();
        let active = layout.drawer.active_shell().map(str::to_owned);
        let height = layout.drawer.height();
        let mut drawer = MissionLayout::default().drawer;
        for shell in summary.shells {
            drawer.add(shell);
        }
        if let Some(active) = active {
            drawer.activate(&active);
        }
        drawer.set_height(height);
        drawer.set_open(was_open && !drawer.shells().is_empty());
        layout.drawer = drawer;
        remote.activate_drawer(layout);
    }
    fn bind_feed(
        &mut self,
        client: FrontPageClient,
        mission: Option<String>,
        clients: Entity<ClientState>,
        cx: &mut Context<MissionWorkspace>,
    ) {
        let Some(id) = mission.filter(|_| client.supports(&["feed", "follow", "unfollow"])) else {
            return;
        };
        let feed = clients.update(cx, |state, cx| state.mission_feed(client, id, cx));
        let Some(remote) = self.remote_mut() else {
            return;
        };
        if remote.feed.as_ref() != Some(&feed) {
            remote.feed_subscription =
                Some(cx.observe(&feed, |workspace, _, cx| workspace.sync_remote_feed(cx)));
            remote.feed = Some(feed);
        }
    }
    pub(super) fn release_feed(&mut self) -> bool {
        let Some(remote) = self.remote_mut() else {
            return false;
        };
        remote.feed = None;
        remote.feed_subscription = None;
        true
    }
    fn set_sidebar_width(&self, width: f32, cx: &mut Context<MissionWorkspace>) {
        if let Some(remote) = self.remote() {
            remote
                .settings
                .update(cx, |settings, _| settings.sidebar_width = width);
        }
    }
    fn set_terminal_sizes(&mut self, sizes: HashMap<String, (u16, u16)>) {
        if let Some(remote) = self.remote_mut() {
            remote.terminal_sizes = sizes;
        }
    }
    fn prepare_action(
        &mut self,
        client: &FrontPageClient,
        active: bool,
        generation: u64,
        feature: &str,
        key: String,
    ) -> Option<MissionAction> {
        if !client.supports(&[feature]) || (client.is_remote() && !active) {
            return None;
        }
        if let Some(remote) = self.remote_mut() {
            if !remote.pending.insert(key.clone()) {
                return None;
            }
        }
        Some(MissionAction {
            client: client.clone(),
            generation,
            key,
            remote: self.is_remote(),
        })
    }
    fn complete_action<T>(
        &mut self,
        action: MissionAction,
        current: &FrontPageClient,
        generation: u64,
        mission_id: Option<&str>,
        result: Result<T, String>,
        remote_ids: impl FnOnce(T) -> Vec<String>,
    ) -> Option<MissionCompletion> {
        if generation != action.generation
            || !action.client.valid()
            || current.target != action.client.target
        {
            return None;
        }
        let result = result.map(remote_ids);
        let before = action
            .key
            .strip_prefix("shell-close:")
            .filter(|_| result.is_ok())
            .and_then(|id| {
                self.summary(mission_id)
                    .map(|mission| (id.to_owned(), mission.shells.clone()))
            })
            .filter(|(id, shells)| shells.contains(id));
        let remote = self.remote_mut()?;
        remote.pending.remove(&action.key);
        if let Some((id, before)) = before {
            let after = before
                .iter()
                .filter(|shell| **shell != id)
                .cloned()
                .collect();
            remote.drawer_expected = Some((before, after));
        }
        Some(MissionCompletion {
            key: action.key,
            client: action.client,
            generation: action.generation,
            result,
        })
    }
    pub(super) fn begin_shell(&mut self) -> Option<Arc<std::sync::Mutex<Option<String>>>> {
        let remote = self.remote_mut()?;
        if remote.shell_pending || remote.drawer_expected.is_some() {
            return None;
        }
        remote.shell_pending = true;
        Some(remote.shell_started.clone())
    }
    pub(super) fn complete_shell(
        &mut self,
        mission_id: Option<&str>,
        before: Vec<String>,
        layout: &mut MissionLayout,
        result: runner_core::protocol::ClientResult<(String, Vec<String>)>,
    ) -> Option<runner_core::protocol::ClientResult<()>> {
        let current = self
            .summary(mission_id)
            .map(|summary| summary.shells.clone());
        let remote = self.remote_mut()?;
        remote.shell_pending = false;
        Some(result.map(|(id, shells)| {
            *remote.shell_started.lock().unwrap() = None;
            remote.activate_shell = Some(id);
            if current.as_ref() == Some(&before) && before != shells {
                remote.drawer_expected = Some((before, shells));
            }
            remote.activate_drawer(layout);
        }))
    }
    pub(super) fn drawer_actions_available(&self) -> bool {
        self.remote()
            .is_none_or(|remote| !remote.shell_pending && remote.drawer_expected.is_none())
    }
    pub(super) fn shell_pending(&self) -> bool {
        self.remote().is_some_and(|remote| remote.shell_pending)
    }
    pub(super) fn shell_retry_available(&self) -> bool {
        self.remote()
            .is_some_and(|remote| remote.shell_started.lock().unwrap().is_some())
    }
}
impl RemoteMission {
    fn reset_actions(&mut self) {
        self.pending.clear();
        self.shell_pending = false;
        self.shell_started = Arc::new(std::sync::Mutex::new(None));
        self.drawer_expected = None;
        self.activate_shell = None;
    }
    fn activate_drawer(&mut self, layout: &mut MissionLayout) {
        if let Some(id) = self
            .activate_shell
            .clone()
            .filter(|id| layout.drawer.shells().contains(id))
        {
            layout.drawer.activate(&id);
            layout.drawer.set_open(true);
            self.activate_shell = None;
        }
    }
}

impl MissionPresentation {
    pub(super) fn terminal_style(
        &self,
        settings: &AppSettings,
        cx: &App,
    ) -> crate::terminal::element::TerminalStyle {
        let host = self
            .remote()
            .and_then(|remote| remote.overview.as_ref()?.terminal_palette.clone());
        let mut style = crate::surfaces::app_shell::terminal_style_for(settings, cx);
        if let Some(palette) = host {
            style.palette = runner_terminal::terminal::palette_from_wire(palette.into());
        }
        style
    }
}

#[cfg(test)]
impl MissionPresentation {
    pub(super) fn remote_for_test(&self) -> Option<&RemoteMission> {
        self.remote()
    }
}

impl MissionPresentation {
    fn new_remote(settings: AppSettings, cx: &mut Context<MissionWorkspace>) -> Self {
        MissionPresentation::Remote(Box::new(RemoteMission {
            settings: cx.new(|_| settings),
            terminal_sizes: HashMap::new(),
            overview: None,
            presentations: HashMap::new(),
            feed: None,
            feed_subscription: None,
            feed_ids: Vec::new(),
            pending: HashSet::new(),
            shell_pending: false,
            shell_started: Arc::new(std::sync::Mutex::new(None)),
            drawer_expected: None,
            activate_shell: None,
            dialog_focus_reset: false,
            selected_handle: None,
        }))
    }
}
