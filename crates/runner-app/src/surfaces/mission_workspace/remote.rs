use super::*;
use crate::surfaces::front_page::model::{
    MissionMemberDisplay, MissionMemberSession, WorkspaceMission,
};
use crate::surfaces::front_page::FrontPageClient;
use crate::surfaces::remote_client::{feed::MissionFeed, model::ClientState};
use runner_core::companion::{FeedContent, FeedEntry, MissionState, Overview};

#[derive(Clone)]
struct MissionPresentation {
    layout: MissionLayout,
    active: MissionTab,
    open_handles: Vec<String>,
    selected_handle: Option<String>,
    rail: MissionRailView,
}

pub(super) struct RemoteMission {
    pub(super) client: FrontPageClient,
    pub(super) settings: Entity<AppSettings>,
    pub(super) terminal_sizes: HashMap<String, (u16, u16)>,
    pub(super) overview: Option<Overview>,
    presentations: HashMap<String, MissionPresentation>,
    pub(super) feed: Option<Entity<MissionFeed>>,
    pub(super) feed_subscription: Option<Subscription>,
    feed_ids: Vec<String>,
    pending: HashSet<String>,
    pub(super) shell_pending: bool,
    pub(super) shell_started: Arc<std::sync::Mutex<Option<String>>>,
    pub(super) drawer_expected: Option<(Vec<String>, Vec<String>)>,
    pub(super) activate_shell: Option<String>,
    dialog_focus_reset: bool,
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
        workspace.remote = Some(RemoteMission {
            client,
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
        });
        workspace
    }

    pub(crate) fn set_remote_sidebar_width(&mut self, width: f32, cx: &mut Context<Self>) {
        if let Some(remote) = &self.remote {
            remote
                .settings
                .update(cx, |settings, _| settings.sidebar_width = width);
        }
        cx.notify();
    }

    pub(crate) fn set_remote_terminal_sizes(&mut self, sizes: HashMap<String, (u16, u16)>) {
        if let Some(remote) = &mut self.remote {
            remote.terminal_sizes = sizes;
        }
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
        let target_changed = self
            .remote
            .as_ref()
            .is_some_and(|remote| remote.client.target != client.target);
        if changed || target_changed {
            let had_dialog = self.rename_modal.is_some()
                || self.stop_all_confirm
                || self.restart_confirm.is_some()
                || self.remove_confirm.is_some();
            self.rename_modal = None;
            self.stop_all_confirm = false;
            self.restart_confirm = None;
            self.remove_confirm = None;
            self.remote.as_mut().unwrap().dialog_focus_reset |= had_dialog;
        }
        let mut restored = false;
        if changed {
            if let (Some(id), Some(remote)) = (self.mission_id.clone(), self.remote.as_mut()) {
                remote.presentations.insert(
                    id,
                    MissionPresentation {
                        layout: self.layout.clone(),
                        active: self.active_tab.clone(),
                        open_handles: open_handles.clone(),
                        selected_handle: self.remote_selected_handle.clone(),
                        rail: self.rail_view,
                    },
                );
            }
            if let Some(remote) = self.remote.as_mut() {
                remote.feed = None;
                remote.feed_subscription = None;
                remote.feed_ids.clear();
                remote.pending.clear();
                remote.shell_pending = false;
                remote.shell_started = Arc::new(std::sync::Mutex::new(None));
                remote.drawer_expected = None;
                remote.activate_shell = None;
            }
            self.remote_selected_handle = None;
            if let Some(id) = mission_id.clone() {
                self.prepare_mission(id.clone(), MissionRailView::Roles);
                if let Some(saved) = self
                    .remote
                    .as_ref()
                    .and_then(|remote| remote.presentations.get(&id))
                    .cloned()
                {
                    self.layout = saved.layout;
                    self.active_tab = saved.active;
                    self.rail_view = saved.rail;
                    self.remote_selected_handle = saved.selected_handle;
                    open_handles = saved.open_handles;
                    restored = true;
                }
            } else {
                self.generation += 1;
                self.active = false;
                self.mission_id = None;
            }
        }
        let Some(remote) = self.remote.as_mut() else {
            return;
        };
        if target_changed {
            self.generation += 1;
            remote.pending.clear();
            remote.shell_pending = false;
            remote.drawer_expected = None;
            remote.shell_started = Arc::new(std::sync::Mutex::new(None));
            remote.activate_shell = None;
            self.stopping = false;
            self.resuming = false;
            self.archiving = false;
            self.composer_posting = false;
            self.submitting_asks.clear();
            self.slot_actions.clear();
        }
        remote.client = client.clone();
        remote.overview = overview;
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
                    .remote_selected_handle
                    .as_ref()
                    .and_then(|handle| members.iter().find(|member| &member.handle == handle))
                    .map(|member| MissionTab::Session(member.session.id.clone()))
                    .unwrap_or(MissionTab::Feed);
            }
        }
        self.remote_selected_handle = match &self.active_tab {
            MissionTab::Session(id) => members
                .iter()
                .find(|member| &member.session.id == id)
                .map(|member| member.handle.clone()),
            MissionTab::Feed => None,
        };
        if let Some(summary) = self.remote_summary().cloned() {
            let remote = self.remote.as_mut().unwrap();
            if remote
                .drawer_expected
                .as_ref()
                .is_some_and(|(before, expected)| {
                    summary.shells == *expected || summary.shells != *before
                })
            {
                remote.drawer_expected = None;
            }

            let was_open = self.layout.drawer.open();
            let active = self.layout.drawer.active_shell().map(str::to_owned);
            let height = self.layout.drawer.height();
            let mut drawer = MissionLayout::default().drawer;
            for shell in summary.shells {
                drawer.add(shell);
            }
            if let Some(active) = active {
                drawer.activate(&active);
            }
            drawer.set_height(height);
            drawer.set_open(was_open && !drawer.shells().is_empty());
            self.layout.drawer = drawer;
            let activate = self
                .remote
                .as_ref()
                .and_then(|remote| remote.activate_shell.clone());
            if let Some(id) = activate.filter(|id| self.layout.drawer.shells().contains(id)) {
                self.layout.drawer.activate(&id);
                self.layout.drawer.set_open(true);
                self.remote.as_mut().unwrap().activate_shell = None;
            }
        }
        if let Some(id) = mission_id.filter(|_| client.supports(&["feed", "follow", "unfollow"])) {
            let feed = clients.update(cx, |state, cx| state.mission_feed(client, id, cx));
            if self.remote.as_ref().unwrap().feed.as_ref() != Some(&feed) {
                let subscription = cx.observe(&feed, |this, _, cx| this.sync_remote_feed(cx));
                let remote = self.remote.as_mut().unwrap();
                remote.feed = Some(feed);
                remote.feed_subscription = Some(subscription);
            }
        }
        self.sync_remote_feed(cx);
        cx.notify();
    }

    pub(crate) fn restore_remote_dialog_focus(
        &mut self,
        fallback: Option<&gpui::FocusHandle>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self
            .remote
            .as_mut()
            .is_some_and(|remote| std::mem::take(&mut remote.dialog_focus_reset))
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
        let id = self.mission_id.as_ref()?;
        self.remote
            .as_ref()?
            .overview
            .as_ref()?
            .missions
            .iter()
            .find(|mission| &mission.id == id)
    }

    pub(super) fn mission_view(&self) -> Option<WorkspaceMission> {
        if self.remote.is_none() {
            return self.mission.as_ref().map(WorkspaceMission::from);
        }
        let mission = self.remote_summary()?;
        let members = self.members();
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
        })
    }

    pub(super) fn members(&self) -> Vec<MissionMemberDisplay> {
        let Some(remote) = &self.remote else {
            return self
                .sessions
                .iter()
                .map(MissionMemberDisplay::from)
                .collect();
        };
        let Some(mission) = self.remote_summary() else {
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
                    lead: member.lead,
                    runtime: session
                        .and_then(|session| session.runtime.clone())
                        .unwrap_or_default(),
                    live_title: session.map(|session| session.title.clone()),
                    agent_session_key: None,
                }
            })
            .collect()
    }

    pub(super) fn remote_supports(&self, features: &[&str]) -> bool {
        self.remote
            .as_ref()
            .is_none_or(|remote| self.active && remote.client.supports(features))
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
        let entries = self
            .remote
            .as_ref()
            .and_then(|remote| remote.feed.as_ref())
            .map(|feed| feed.read(cx).entries.clone())
            .unwrap_or_default();
        let ids = entries
            .iter()
            .map(|entry| entry.id.clone())
            .collect::<Vec<_>>();
        let changed = self
            .remote
            .as_ref()
            .is_some_and(|remote| remote.feed_ids != ids);
        if changed {
            let previous = self.remote.as_ref().unwrap().feed_ids.clone();
            if let Some(tail) = previous
                .last()
                .and_then(|id| entries.iter().position(|entry| &entry.id == id))
            {
                for entry in &entries[tail + 1..] {
                    if let Some(event) = self
                        .mission_id
                        .as_deref()
                        .and_then(|id| remote_event(id, entry))
                    {
                        self.handle_feed_append(&event);
                    }
                }
            }
            self.remote.as_mut().unwrap().feed_ids = ids;
            self.events = entries
                .iter()
                .filter_map(|entry| remote_event(self.mission_id.as_deref()?, entry))
                .collect();
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
            .map(|shell| shell.update(cx, |shell, cx| shell.render_remote_terminal_content(id, cx)))
            .unwrap_or_else(|| div().into_any_element())
    }

    pub(super) fn remote_navigation_changed(&self, cx: &mut Context<Self>) {
        if self.remote.is_none() {
            return;
        }
        let shell = self.shell.clone();
        cx.defer(move |cx| {
            let _ = shell.update(cx, |shell, cx| {
                shell.attach_remote_panes(cx);
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

impl MissionWorkspace {
    pub(super) fn remote_action(
        &mut self,
        feature: &'static str,
        key: String,
        operation: impl FnOnce(FrontPageClient) -> runner_core::protocol::ClientResult<Vec<String>>
            + Send
            + 'static,
        cx: &mut Context<Self>,
    ) {
        if !self.remote_supports(&[feature]) {
            return;
        }
        let remote = self.remote.as_mut().unwrap();
        if !remote.pending.insert(key.clone()) {
            return;
        }
        let client = remote.client.clone();
        let guard = client.clone();
        let generation = self.generation;
        let task = cx
            .background_spawn(async move { operation(client).map_err(|error| error.to_string()) });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this: &mut Self, cx| {
                if this.generation != generation || !guard.valid() {
                    return;
                }
                let Some(remote) = this
                    .remote
                    .as_mut()
                    .filter(|remote| remote.client.target == guard.target)
                else {
                    return;
                };
                remote.pending.remove(&key);
                match key.as_str() {
                    "stopMission" => this.stopping = false,
                    "resumeMission" => this.resuming = false,
                    "archive" => this.archiving = false,
                    "rename" => {
                        if let Some(modal) = &mut this.rename_modal {
                            modal.submitting = false;
                            modal.error = result.as_ref().err().cloned();
                        }
                        if result.is_ok() {
                            this.rename_modal = None;
                        }
                    }
                    id => {
                        this.slot_actions.remove(id);
                    }
                }
                if let Some(id) = key.strip_prefix("shell-close:").filter(|_| result.is_ok()) {
                    if let Some(before) = this
                        .remote_summary()
                        .map(|mission| mission.shells.clone())
                        .filter(|shells| shells.iter().any(|shell| shell == id))
                    {
                        let after = before
                            .iter()
                            .filter(|shell| shell.as_str() != id)
                            .cloned()
                            .collect();
                        this.remote.as_mut().unwrap().drawer_expected = Some((before, after));
                    }
                }
                if let Ok(ids) = &result {
                    if !ids.is_empty() {
                        let ids = ids.clone();
                        let shell = this.shell.clone();
                        let workspace = cx.weak_entity();
                        cx.defer(move |cx| {
                            if !workspace.upgrade().is_some_and(|workspace| {
                                workspace.read(cx).generation == generation
                            }) {
                                return;
                            }
                            let _ = shell.update(cx, |shell, cx| {
                                if let Some(remote) = shell
                                    .remote_window
                                    .as_mut()
                                    .filter(|remote| remote.front_page.target == guard.target)
                                {
                                    remote.reopen_sessions.extend(ids);
                                    shell.sync_remote_state(cx);
                                }
                            });
                        });
                    }
                }
                if key != "rename" {
                    if let Err(error) = result {
                        this.error = Some(action_failure(
                            crate::i18n::message("mission.update_action"),
                            error,
                        ));
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn remote_slot_action(
        &mut self,
        session: &str,
        action: runner_app::ui::SessionControlKind,
        size: (u16, u16),
        cx: &mut Context<Self>,
    ) {
        use runner_app::ui::SessionControlKind;
        let feature = match action {
            SessionControlKind::Stop => "stop",
            SessionControlKind::Restart => "restart",
            _ => "resume",
        };
        if !self.remote_supports(&[feature])
            || !self
                .mission_view()
                .is_some_and(|mission| mission.status == MissionStatus::Running)
            || !self
                .members()
                .iter()
                .any(|member| member.session.id == session)
            || self.slot_actions.contains(session)
            || self.lifecycle_busy()
        {
            return;
        }
        let session = session.to_owned();
        self.slot_actions.insert(session.clone());
        self.remote_action(
            feature,
            session.clone(),
            move |client| match action {
                SessionControlKind::Stop => client.stop(&session).map(|_| Vec::new()),
                SessionControlKind::Restart => client.restart(&session, size).map(|id| vec![id]),
                _ => client.resume(&session, size).map(|id| vec![id]),
            },
            cx,
        );
    }

    pub(super) fn remote_answer(
        &mut self,
        question: String,
        choice: String,
        cx: &mut Context<Self>,
    ) {
        if !self.remote_supports(&["answer"])
            || !self
                .remote_summary()
                .is_some_and(|mission| mission.asks.iter().any(|ask| ask.question == question))
            || !self.submitting_asks.insert(question.clone())
        {
            return;
        }
        self.pending_ask_choices
            .insert(question.clone(), choice.clone());
        self.invalidate_feed_ask(&question);
        let remote = self.remote.as_ref().unwrap();
        let client = remote.client.clone();
        let guard = client.clone();
        let mission = self.mission_id.clone().unwrap();
        let generation = self.generation;
        let task_question = question.clone();
        let task = cx.background_spawn(async move {
            client
                .answer(mission, task_question, choice)
                .map_err(|error| error.to_string())
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this: &mut Self, cx| {
                if this.generation != generation || !guard.valid() {
                    return;
                }
                this.submitting_asks.remove(&question);
                if let Err(error) = result {
                    this.pending_ask_choices.remove(&question);
                    this.error = Some(action_failure(crate::i18n::message("action.answer"), error));
                }
                this.invalidate_feed_ask(&question);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }

    pub(super) fn remote_post(
        &mut self,
        post: crate::surfaces::mission_composer::ComposerPost,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.composer_posting
            || !self.remote_supports(&["post"])
            || !self
                .mission_view()
                .is_some_and(|mission| mission.status == MissionStatus::Running)
        {
            return;
        }
        let handles = self
            .members()
            .into_iter()
            .map(|member| member.handle)
            .collect::<Vec<_>>();
        if handles.is_empty() {
            return;
        }
        let resolved =
            match runner_core::addressing::resolve(&post.text, post.to.as_slice(), &handles) {
                Ok(resolved) => resolved,
                Err(error) => {
                    self.error = Some(action_failure(
                        crate::i18n::message("action.send_message"),
                        error.to_string(),
                    ));
                    cx.notify();
                    return;
                }
            };
        let client = self.remote.as_ref().unwrap().client.clone();
        let guard = client.clone();
        let mission = self.mission_id.clone().unwrap();
        let generation = self.generation;
        self.composer_posting = true;
        let task = cx.background_spawn(async move {
            client
                .post(
                    mission,
                    resolved.recipients.as_slice().to_vec(),
                    resolved.text,
                )
                .map_err(|error| error.to_string())
        });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this: &mut Self, window, cx| {
                if this.generation != generation || !guard.valid() {
                    return;
                }
                this.composer_posting = false;
                match result {
                    Ok(()) => {
                        this.composer = ComposerState::default();
                        this.composer_input
                            .update(cx, |input, cx| input.reset("", cx));
                    }
                    Err(error) => {
                        this.error = Some(action_failure(
                            crate::i18n::message("action.send_message"),
                            error,
                        ))
                    }
                }
                this.composer_input
                    .read(cx)
                    .focus_handle()
                    .focus(window, cx);
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl MissionWorkspace {
    pub(super) fn add_remote_mission_shell(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.remote_supports(&["startShell", "layout"]) {
            return;
        }
        let Some(mission) = self.mission_view() else {
            return;
        };
        let Some(summary) = self.remote_summary().cloned() else {
            return;
        };
        let size = self.estimated_mission_drawer_terminal_size(window, cx);
        let remote = self.remote.as_mut().unwrap();
        if remote.shell_pending || remote.drawer_expected.is_some() {
            return;
        }
        remote.shell_pending = true;
        let stage = remote.shell_started.clone();
        let client = remote.client.clone();
        let guard = client.clone();
        let generation = self.generation;
        let before = summary.shells;
        let mut shells = before.clone();
        let task = cx.background_spawn(async move {
            let existing = stage.lock().unwrap().clone();
            let id = match existing {
                Some(id) => id,
                None => {
                    let id = client.start_shell(mission.project_id, mission.cwd, size)?;
                    *stage.lock().unwrap() = Some(id.clone());
                    id
                }
            };
            if !shells.contains(&id) {
                shells.push(id.clone());
            }
            client.mission_layout(mission.id, shells.clone())?;
            Ok::<_, runner_core::protocol::ClientError>((id, shells))
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this: &mut Self, cx| {
                if this.generation != generation || !guard.valid() {
                    return;
                }
                let current = this.remote_summary().map(|summary| summary.shells.clone());
                let Some(remote) = this
                    .remote
                    .as_mut()
                    .filter(|remote| remote.client.target == guard.target)
                else {
                    return;
                };
                remote.shell_pending = false;
                match result {
                    Ok((id, shells)) => {
                        *remote.shell_started.lock().unwrap() = None;
                        remote.activate_shell = Some(id);
                        if current.as_ref() == Some(&before) && before != shells {
                            remote.drawer_expected = Some((before, shells));
                        }
                        this.error = None;
                        if let Some(shell) = remote
                            .activate_shell
                            .clone()
                            .filter(|id| this.layout.drawer.shells().contains(id))
                        {
                            this.layout.drawer.activate(&shell);
                            this.layout.drawer.set_open(true);
                            remote.activate_shell = None;
                        }
                        this.remote_navigation_changed(cx);
                    }
                    Err(error) => {
                        this.error = Some(action_failure(
                            crate::i18n::message("mission.shell_action"),
                            error.to_string(),
                        ))
                    }
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
}

impl NativeRoot {
    pub(crate) fn remote_mission_workspace(&self) -> Option<Entity<MissionWorkspace>> {
        let remote = self.remote_window.as_ref()?;
        matches!(
            remote.active,
            Some(crate::surfaces::remote_client::RemoteItem::Mission(_))
        )
        .then(|| remote.mission_workspace.clone())
    }
}
