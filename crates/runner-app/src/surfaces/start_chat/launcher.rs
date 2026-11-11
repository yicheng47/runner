use super::launcher_model::{
    self as model, CrewEntry, EntryKey, KeyResult, RecentStore, SearchEntry, SlotLine,
};
use super::*;
use crate::surfaces::front_page::folder_picker::{FolderChosen, FolderPicker};
use crate::surfaces::front_page::{
    client::{ChatLaunch, ChatStart},
    FrontPageClient, FrontPageTarget,
};
use gpui::{canvas, Render, SharedString, WeakEntity};
use runner_core::protocol::Runtime;

mod mission;
pub(crate) use mission::LauncherOrigin;

#[derive(Clone, Debug, PartialEq, Eq)]
enum Choice {
    Chat(EntryKey),
    Terminal,
    Mission,
    Crew(String),
    EmptyMission,
}

impl Choice {
    /// Rows pinned below the ranked ones, which keep their shortcut while
    /// searching.
    fn pinned(&self) -> bool {
        matches!(self, Self::Terminal | Self::Mission | Self::EmptyMission)
    }
}

/// The launcher's page: agents and roles, a mission's crews, or its goal
/// once a crew (or none, for an Empty mission) is picked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Step {
    Start,
    Crews,
    Goal(Option<String>),
}

struct Row {
    choice: Choice,
    name: String,
    detail: String,
    runtime: Option<String>,
    avatar: Option<String>,
    crew: Option<Vec<SharedString>>,
    unavailable: bool,
    dimmed: bool,
    used_at: Option<i64>,
    previously_used: bool,
}

#[derive(Clone)]
enum UpdateAction {
    Chat {
        key: Option<EntryKey>,
        pane: Option<PaneKey>,
        scope: ProjectScope,
        cwd: Option<String>,
    },
    Mission(mission::MissionRequest),
    Fork(super::super::chat::ForkRequest),
}

#[derive(Clone)]
struct UpdateRetry {
    runtime: Runtime,
    gate: Option<runner_core::protocol::RuntimeVersionGate>,
    action: UpdateAction,
    requested: bool,
    ready: bool,
}

pub(crate) struct ChatLauncher {
    update_retry: Option<UpdateRetry>,
    root: WeakEntity<NativeRoot>,
    client: FrontPageClient,
    pane: Option<PaneKey>,
    default_page: bool,
    project: ProjectField,
    initial_project: Option<String>,
    /// The project whose + opened this page: the row sits under it and the
    /// picker cannot leave it.
    page_project: Option<String>,
    cwd: Entity<TextField>,
    changed_directory: bool,
    height: Option<f32>,
    query: Entity<TextField>,
    selected: usize,
    runtimes: Vec<RuntimeChoice>,
    roles: Vec<RoleChoice>,
    default_runtime: String,
    keymap_overrides: keymap::KeymapOverrides,
    app_data_dir: PathBuf,
    recents: RecentStore,
    recent_revision: u64,
    catalog_revisions: (u64, u64, u64),
    host_account: Option<String>,
    loading: bool,
    catalog_requested: bool,
    pending: Option<Choice>,
    launched_session: Option<String>,
    started: Option<(EntryKey, String)>,
    error: Option<String>,
    folder_picker: Option<Entity<FolderPicker>>,
    step: Step,
    crews: Vec<CrewEntry>,
    goal: Entity<TextField>,
    slots: Option<Vec<SlotLine>>,
    origin: Option<LauncherOrigin>,
    store: Entity<AppStore>,
    subscriptions: Vec<Subscription>,
}

fn launcher_input(
    key: &EntryKey,
    project: Option<String>,
    cwd: Option<String>,
    size: (u16, u16),
) -> ChatStart {
    let (role, runtime) = match key {
        EntryKey::Agent(key) => (None, Some(key.clone())),
        EntryKey::Role(key) => (Some(key.clone()), None),
    };
    ChatStart {
        role,
        runtime,
        model: None,
        effort: None,
        speed: None,
        project,
        cwd,
        size,
    }
}

fn launcher_directory(
    role: Option<&RoleChoice>,
    shown: Option<String>,
    changed: bool,
) -> Option<String> {
    if changed {
        shown
    } else {
        role.and_then(|role| normalized_value(role.working_dir.as_deref().unwrap_or_default()))
            .or(shown)
    }
}

pub(crate) type PendingLauncherTabs =
    std::sync::Arc<std::sync::Mutex<std::collections::BTreeMap<String, Option<PaneLayout>>>>;

struct TabStart {
    pending: PendingLauncherTabs,
    key: String,
    retained: bool,
    before: PaneLayout,
}
impl TabStart {
    fn begin(pending: PendingLauncherTabs, key: String, before: PaneLayout) -> Option<Self> {
        let mut entries = pending.lock().unwrap();
        if entries.contains_key(&key) {
            return None;
        }
        entries.insert(key.clone(), None);
        drop(entries);
        Some(Self {
            pending,
            key,
            retained: false,
            before,
        })
    }

    fn await_host(&mut self) {
        self.pending
            .lock()
            .unwrap()
            .insert(self.key.clone(), Some(self.before.clone()));
        self.retained = true;
    }
}
impl Drop for TabStart {
    fn drop(&mut self) {
        if !self.retained {
            self.pending.lock().unwrap().remove(&self.key);
        }
    }
}

fn computer_key(target: &FrontPageTarget) -> String {
    match target {
        FrontPageTarget::Local => "local".into(),
        FrontPageTarget::Remote {
            computer,
            fingerprint,
            ..
        } => format!("remote:{computer}:{fingerprint}"),
    }
}

static RECENT_REVISION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
static RECENT_WRITER: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(crate) fn record_launcher_use(directory: &Path, target: &FrontPageTarget, key: EntryKey) {
    let _writer = RECENT_WRITER.lock().unwrap();
    let mut store = RecentStore::load(directory);
    store.record(&computer_key(target), key, chrono::Utc::now().timestamp());
    match store.save(directory) {
        Ok(()) => {
            RECENT_REVISION.fetch_add(1, std::sync::atomic::Ordering::Release);
        }
        Err(error) => tracing::warn!(%error, "could not save launcher history"),
    }
}

impl ChatLauncher {
    fn reload_recents(&mut self, cx: &mut Context<Self>) {
        self.recent_revision = RECENT_REVISION.load(std::sync::atomic::Ordering::Acquire);
        self.recents = RecentStore::load(&self.app_data_dir);
        cx.notify();
    }

    fn scope(&self) -> ProjectScope {
        if self.pane.is_some() && !self.default_page {
            ProjectScope::or_root(self.initial_project.clone())
        } else {
            self.project.scope()
        }
    }

    /// Points a page at `project`, also when it is already showing. Locked, the
    /// row sits under the project and the picker stays on it; otherwise the row
    /// is in Recents and the picker is free. No project starts over from the
    /// default folder, as a new page does.
    fn retarget_page(&mut self, project: Option<&str>, locked: bool, cx: &mut Context<Self>) {
        self.initial_project = project.map(str::to_owned);
        self.page_project = project.filter(|_| locked).map(str::to_owned);
        self.project.set_locked(false, cx);
        if project.is_none() {
            let directory = self.default_directory(cx);
            self.cwd.update(cx, |input, cx| input.reset(directory, cx));
        }
        let project = project.unwrap_or_default();
        self.project
            .select()
            .update(cx, |select, cx| select.set_value(project, cx));
        self.choose_project(project, cx);
        self.project.set_locked(self.page_project.is_some(), cx);
        self.redraw_page_row(cx);
    }

    /// Frees a page from a project's +: the row goes to Recents, and the picker
    /// stays on the project and folder it shows, now editable.
    fn release_page_lock(&mut self, cx: &mut Context<Self>) {
        if self.pending.is_none() && self.page_project.take().is_some() {
            self.project.set_locked(false, cx);
            self.redraw_page_row(cx);
        }
    }

    fn default_directory(&self, cx: &App) -> String {
        if self.client.is_remote() {
            return String::new();
        }
        local_default_directory(&self.store.read(cx).settings).unwrap_or_default()
    }

    /// Deferred because `open_launcher_page` retargets a page with the root
    /// already borrowed.
    fn redraw_page_row(&self, cx: &mut Context<Self>) {
        let root = self.root.clone();
        cx.defer(move |cx| {
            if let Some(root) = root.upgrade() {
                let sidebar = root.read(cx).front_page_sidebar();
                sidebar.update(cx, |_, cx| cx.notify());
            }
        });
    }

    /// The field that takes the launcher's keys: the goal on the goal step,
    /// else the search.
    fn field(&self) -> &Entity<TextField> {
        if matches!(self.step, Step::Goal(_)) {
            &self.goal
        } else {
            &self.query
        }
    }

    pub(crate) fn focus(&self, window: &mut Window, cx: &mut App) {
        self.field().read(cx).focus_handle().focus(window, cx);
    }
    fn new(
        root: &NativeRoot,
        pane: Option<PaneKey>,
        default_page: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        root_entity: WeakEntity<NativeRoot>,
    ) -> Self {
        let client = root.front_page_client(cx);
        let remote = client.is_remote();
        let layout = root.front_page_layout();
        let locked = pane.is_some() && !default_page;
        let scope = if pane.is_some() {
            root.active_tab_project_id(cx)
        } else if root.launcher_is_empty_state() {
            None
        } else {
            root.active_project_id(cx)
        };
        let sibling = locked
            .then(|| {
                layout.and_then(|layout| {
                    layout.focused_session_id().or_else(|| {
                        layout
                            .root
                            .leaves()
                            .into_iter()
                            .find_map(|leaf| leaf.session_id.as_deref())
                    })
                })
            })
            .flatten();
        let (runtimes, roles, crews, choices, default_runtime, cwd_value, error) = if remote {
            (
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                String::new(),
                sibling
                    .and_then(|id| root.front_page_session(id, cx))
                    .and_then(|session| session.cwd)
                    .unwrap_or_default(),
                None,
            )
        } else {
            let (runtimes, _, error) = load_selectable_runtimes(root.core(cx), root.settings(cx));
            let choices = ProjectChoice::local(&root.app_store.read(cx).projects);
            let cwd = if locked {
                root.terminal_start_location(cx).1
            } else {
                choices
                    .iter()
                    .find(|project| Some(&project.id) == scope.as_ref())
                    .map(|project| project.path.clone())
                    .or_else(|| local_default_directory(root.settings(cx)))
            };
            (
                runtimes.into_iter().map(Into::into).collect(),
                root.app_store
                    .read(cx)
                    .roles
                    .clone()
                    .into_iter()
                    .map(Into::into)
                    .collect(),
                mission::local_crews(&root.app_store.read(cx).crews),
                choices,
                root.settings(cx).default_runtime.clone(),
                cwd.unwrap_or_default(),
                error,
            )
        };
        let entity = cx.entity();
        let query = cx.new(|input_cx| {
            let mut input = TextField::new(
                input_cx.focus_handle(),
                "",
                crate::i18n::message("launcher.search"),
                false,
            );
            input.set_bare(true, input_cx);
            input
        });
        let cwd = cx.new(|cx| TextField::new(cx.focus_handle(), cwd_value, "", true));
        let goal = cx.new(|input_cx| {
            let mut input = TextField::textarea(
                input_cx.focus_handle(),
                "",
                crate::i18n::message("ui.describe_what_to_do"),
                1,
                false,
            )
            .auto_grow(6)
            .text_size(theme::text_body());
            input.set_bare(true, input_cx);
            input.set_right_padding(0., input_cx);
            input
        });
        let project_owner = entity.downgrade();
        let project = ProjectField::new(
            "launcher-project",
            choices,
            !remote,
            scope.clone(),
            locked,
            Rc::new(move |id, _, cx| {
                let _ = project_owner.update(cx, |this, cx| this.choose_project(&id, cx));
            }),
            cx,
        );
        project.select().update(cx, |select, cx| {
            select.set_inline_trigger(true, cx);
            let owner = entity.downgrade();
            select.set_on_close(Rc::new(move |window, cx| {
                let _ = owner.update(cx, |this, cx| this.focus(window, cx));
            }));
        });
        let subscription = cx.observe(&query, |this, _, cx| {
            this.selected = 0;
            cx.notify();
        });
        if claims_focus(root, pane.as_ref()) {
            query.read(cx).focus_handle().focus(window, cx);
        }
        let app_data_dir = root.app_store.read(cx).app_data_dir.clone();
        let recent_revision = RECENT_REVISION.load(std::sync::atomic::Ordering::Acquire);
        let recents = RecentStore::load(&app_data_dir);
        let revisions = root.app_store.read(cx).revisions;
        let mut this = Self {
            update_retry: None,
            root: root_entity,
            client,
            pane,
            default_page,
            project,
            initial_project: scope,
            page_project: None,
            cwd,
            changed_directory: false,
            height: None,
            query,
            selected: 0,
            runtimes,
            roles,
            default_runtime,
            keymap_overrides: root.settings(cx).keymap_overrides.clone(),
            app_data_dir,
            recents,
            recent_revision,
            catalog_revisions: (revisions.roles, revisions.projects, revisions.crews),
            host_account: None,
            loading: remote,
            catalog_requested: false,
            pending: None,
            launched_session: None,
            started: None,
            error,
            folder_picker: None,
            step: Step::Start,
            crews,
            goal,
            slots: None,
            origin: None,
            store: root.app_store.clone(),
            subscriptions: vec![subscription],
        };
        if remote {
            this.load_host(root.remote_client.computer().cloned(), window, cx);
        }
        this
    }

    fn load_host(
        &mut self,
        computer: Option<runner_core::remote::Computer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let client = self.client.clone();
        if !client.supports(&["catalog"]) {
            self.loading = false;
            return;
        }
        self.catalog_requested = true;
        self.loading = true;
        let task = cx.background_spawn(async move { client.catalog() });
        cx.spawn_in(window, async move |weak, cx| {
            let result = task.await;
            let _ = weak.update_in(cx, |this, _, cx| {
                this.loading = false;
                if !this.client.valid() {
                    return;
                }
                match result {
                    Ok(catalog) => {
                        let owner = cx.weak_entity();
                        this.project = ProjectField::new(
                            "launcher-project",
                            ProjectChoice::host(&catalog),
                            false,
                            this.initial_project.clone(),
                            (this.pane.is_some() && !this.default_page)
                                || this.page_project.is_some(),
                            Rc::new(move |id, _, cx| {
                                let _ = owner.update(cx, |this, cx| this.choose_project(&id, cx));
                            }),
                            cx,
                        );
                        let owner = cx.weak_entity();
                        this.project.select().update(cx, |select, cx| {
                            select.set_inline_trigger(true, cx);
                            select.set_on_close(Rc::new(move |window, cx| {
                                let _ = owner.update(cx, |this, cx| this.focus(window, cx));
                            }));
                        });
                        if this.page_project.is_some() && this.project.chosen().is_none() {
                            this.retarget_page(None, false, cx);
                        }
                        if this.cwd.read(cx).text().is_empty() {
                            if let Some(project) = this.project.chosen() {
                                let path = project.path.clone();
                                this.cwd.update(cx, |input, cx| input.reset(path, cx));
                            }
                        }
                        this.roles = catalog.roles.clone().into_iter().map(Into::into).collect();
                        this.runtimes = catalog
                            .runtimes
                            .iter()
                            .filter_map(RuntimeChoice::remote)
                            .collect();
                        let supported = this
                            .runtimes
                            .iter()
                            .filter(|runtime| !runtime.below_minimum)
                            .map(|runtime| runtime.name)
                            .collect::<Vec<_>>();
                        let target = this.client.target.clone();
                        let _ = this.root.update(cx, |root, _| {
                            root.clear_unblocked_gates(&target, &supported)
                        });
                        this.crews = mission::host_crews(&catalog);
                        this.settle_goal_crew(cx);
                        this.host_account = catalog
                            .accounts
                            .iter()
                            .find(|account| account.id == catalog.active_account)
                            .map(|account| account.name.clone());
                        if let Some(computer) = computer {
                            let input = this.cwd.clone();
                            let client = this.client.clone();
                            let popup_width = px(320. * this.store.read(cx).settings.app_zoom);
                            let picker = cx.new(|cx| {
                                let mut picker = FolderPicker::new(
                                    client,
                                    computer.name,
                                    computer.info.platform,
                                    input,
                                    crate::surfaces::front_page::folder_picker::choices(&catalog),
                                    cx,
                                );
                                picker.set_popup_width(popup_width);
                                picker.set_align_right(true);
                                picker
                            });
                            this.subscriptions
                                .push(cx.observe(&picker, |_, _, cx| cx.notify()));
                            this.subscriptions.push(cx.subscribe(
                                &picker,
                                |this, _, _: &FolderChosen, cx| {
                                    this.changed_directory = true;
                                    let path = this.cwd.read(cx).text().to_owned();
                                    this.project.picked(&path, cx);
                                    cx.notify();
                                },
                            ));
                            this.folder_picker = Some(picker);
                        }
                    }
                    Err(error) => this.error = Some(error.to_string()),
                }
                cx.notify();
            });
        })
        .detach();
    }

    fn rows(&self, cx: &App) -> (Vec<Row>, usize) {
        match self.step {
            Step::Goal(_) => return (Vec::new(), 0),
            Step::Crews => return (self.crew_rows(cx), self.crews.len()),
            Step::Start => {}
        }
        let mut entries = Vec::new();
        let chat = self.client.supports(&["catalog", "startChat", "layout"]);
        if chat {
            entries.extend(
                self.runtimes
                    .iter()
                    .filter(|runtime| runtime.available)
                    .map(|runtime| SearchEntry {
                        key: EntryKey::Agent(runtime.name.to_string()),
                        name: runtime.display_name.clone(),
                        runtime: runtime.name.to_string(),
                        handle: String::new(),
                    }),
            );
            entries.extend(self.roles.iter().map(|role| SearchEntry {
                key: EntryKey::Role(role.id.clone()),
                name: role.display_name.clone(),
                runtime: role.runtime.clone(),
                handle: role.handle.clone(),
            }));
        }
        let total = entries.len();
        let recent = self.recents.entries(&computer_key(&self.client.target));
        let query = self.query.read(cx).text();
        let mut rows = model::ranked(&entries, recent, &self.default_runtime, query)
            .into_iter()
            .map(|entry| {
                let role = match &entry.key {
                    EntryKey::Role(id) => self.roles.iter().find(|role| &role.id == id),
                    _ => None,
                };
                let runtime = self
                    .runtimes
                    .iter()
                    .find(|runtime| runtime.name.key() == entry.runtime);
                let unavailable = runtime.is_none_or(|runtime| !runtime.available);
                let detail = if unavailable {
                    crate::i18n::text("launcher.unavailable").into()
                } else if let Some(role) = role {
                    let mut detail = format!(
                        "{} · {}",
                        crate::i18n::text("launcher.role"),
                        runtime.unwrap().display_name
                    );
                    for value in [&role.model, &role.effort].into_iter().flatten() {
                        detail.push_str(" · ");
                        detail.push_str(value);
                    }
                    detail
                } else {
                    crate::i18n::text("launcher.agent").into()
                };
                let detail = if entry.runtime == "claude-code" {
                    self.host_account
                        .as_ref()
                        .map(|account| format!("{detail} · {account}"))
                        .unwrap_or(detail)
                } else {
                    detail
                };
                Row {
                    choice: Choice::Chat(entry.key.clone()),
                    name: entry.name.clone(),
                    detail,
                    runtime: Some(entry.runtime.clone()),
                    avatar: role.map(|role| role.handle.clone()),
                    crew: None,
                    unavailable,
                    dimmed: unavailable,
                    previously_used: recent.iter().any(|recent| recent.key == entry.key),
                    used_at: recent
                        .iter()
                        .find(|recent| recent.key == entry.key)
                        .and_then(|recent| recent.used_at),
                }
            })
            .collect::<Vec<_>>();
        if self.offers_missions() && !query.is_empty() {
            rows.extend(self.typed_crew_rows(query, cx));
        }
        if self.client.supports(&["startShell", "layout"]) {
            rows.push(Row::pinned(
                Choice::Terminal,
                crate::i18n::text("launcher.terminal"),
                "",
            ));
        }
        if self.offers_missions() {
            rows.push(Row::pinned(
                Choice::Mission,
                crate::i18n::text("launcher.mission"),
                crate::i18n::text("ui.work_with_a_crew_of_agents"),
            ));
        }
        if let (false, Some(height)) = (self.default_page, self.height) {
            let ranked = rows.iter().filter(|row| !row.choice.pinned()).count();
            let (keep_ranked, keep_pinned) = model::fit_rows(height, ranked, rows.len() - ranked);
            let pinned = rows.split_off(ranked);
            rows.truncate(keep_ranked);
            rows.extend(pinned.into_iter().take(keep_pinned));
        }
        (rows, total)
    }

    fn on_key(
        &mut self,
        event: &KeyDownEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if self.field().read(cx).is_composing()
            || event.keystroke.modifiers.platform
            || event.keystroke.modifiers.control
            || event.keystroke.modifiers.alt
        {
            return false;
        }
        let key = event.keystroke.key.as_str();
        let shift = event.keystroke.modifiers.shift;
        let empty = self.field().read(cx).text().is_empty();
        match (&self.step, key) {
            (Step::Start, "escape") if self.pane.is_none() => {
                self.leave_page(window, cx);
                return true;
            }
            (Step::Goal(_), "enter") if !shift => {
                self.start_mission(window, cx);
                return true;
            }
            (Step::Goal(_), "escape") => {
                self.open_step(Step::Crews, window, cx);
                return true;
            }
            (Step::Goal(_), "backspace") if empty => {
                self.open_step(Step::Crews, window, cx);
                return true;
            }
            (Step::Goal(_), _) => return false,
            (Step::Crews, "enter") if shift => {
                if self.offers_empty_missions() {
                    self.start(Choice::EmptyMission, false, window, cx);
                }
                return true;
            }
            (Step::Crews, "escape" | "backspace") if key == "escape" || empty => {
                self.leave_crews(window, cx);
                return true;
            }
            _ => {}
        }
        let (rows, _) = self.rows(cx);
        let ranked = rows.iter().filter(|row| !row.choice.pinned()).count();
        match model::key_result(key, empty, self.selected, ranked, rows.len() - ranked) {
            KeyResult::Select(index) => {
                self.selected = index;
                cx.notify();
                true
            }
            KeyResult::Start(index) => {
                if let Some(row) = rows.get(index) {
                    self.start(row.choice.clone(), row.unavailable, window, cx);
                }
                true
            }
            KeyResult::Clear => {
                self.query.update(cx, |input, cx| input.reset("", cx));
                true
            }
            KeyResult::Text => false,
        }
    }

    fn start(
        &mut self,
        choice: Choice,
        unavailable: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.pending.is_some() || unavailable || !self.client.valid() {
            return;
        }
        match choice {
            Choice::Mission => return self.open_step(Step::Crews, window, cx),
            Choice::EmptyMission => {
                if self.offers_empty_missions() {
                    self.open_step(Step::Goal(None), window, cx);
                }
                return;
            }
            Choice::Crew(id) => return self.pick_crew(id, window, cx),
            Choice::Chat(_) | Choice::Terminal => {}
        }
        let Some(root) = self.root.upgrade() else {
            return;
        };
        self.error = None;
        self.pending = Some(choice.clone());
        let scope = self.scope();
        let path = normalized_value(self.cwd.read(cx).text());
        let pane = self.pane.clone();
        let launcher = cx.entity();
        match choice {
            Choice::Mission | Choice::EmptyMission | Choice::Crew(_) => {}
            Choice::Terminal => {
                window.defer(cx, move |window, cx| {
                    root.update(cx, |root, cx| {
                        root.launch_from_launcher(
                            launcher, None, pane, scope, path, false, window, cx,
                        )
                    })
                });
            }
            Choice::Chat(key) => {
                let role = match &key {
                    EntryKey::Role(id) => self.roles.iter().find(|role| &role.id == id),
                    _ => None,
                };
                let cwd = launcher_directory(role, path, self.changed_directory);
                window.defer(cx, move |window, cx| {
                    root.update(cx, |root, cx| {
                        root.launch_from_launcher(
                            launcher,
                            Some(key),
                            pane,
                            scope,
                            cwd,
                            false,
                            window,
                            cx,
                        )
                    })
                });
            }
        }
        cx.notify();
    }

    fn choose_project(&mut self, id: &str, cx: &mut Context<Self>) {
        let current = self.cwd.read(cx).text().to_owned();
        if let Some(path) = self.project.choose(id, &current) {
            self.cwd.update(cx, |input, cx| input.reset(path, cx));
        }
        self.changed_directory = false;
        cx.notify();
    }

    fn change_directory(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.pending.is_some() {
            return;
        }
        if let Some(picker) = &self.folder_picker {
            let popup_width = px(320. * self.store.read(cx).settings.app_zoom);
            picker.update(cx, |picker, cx| {
                picker.set_popup_width(popup_width);
                picker.toggle(window, cx);
            });
            return;
        }
        if self.client.is_remote() {
            return;
        }
        let task = cx.prompt_for_paths(PathPromptOptions {
            files: false,
            directories: true,
            multiple: false,
            prompt: Some(crate::i18n::text("ui.pick_a_working_directory").into()),
        });
        cx.spawn(async move |weak, cx| {
            let result = task.await;
            let _ = weak.update(cx, |this, cx| {
                match result {
                    Ok(Ok(Some(paths))) => {
                        if let Some(path) = paths.first() {
                            let path = path.to_string_lossy().into_owned();
                            this.project.picked(&path, cx);
                            this.cwd.update(cx, |input, cx| input.reset(path, cx));
                            this.changed_directory = true;
                        }
                    }
                    Ok(Ok(None)) => {}
                    error => this.error = Some(format!("{error:?}")),
                }
                cx.notify();
            });
        })
        .detach();
    }
}

fn local_default_directory(settings: &AppSettings) -> Option<String> {
    normalized_value(&settings.default_working_dir).or_else(|| {
        runner_core::app_paths::home_dir().map(|path| path.to_string_lossy().into_owned())
    })
}

fn claims_focus(root: &NativeRoot, pane: Option<&PaneKey>) -> bool {
    root.front_page_is_chat()
        && !root.create_modal_open()
        && pane.is_none_or(|pane| {
            root.front_page_layout()
                .is_some_and(|layout| layout.focused_pane_id == pane.pane_id)
        })
}

fn highlighted(name: &str, query: &str) -> AnyElement {
    let query = query.to_lowercase();
    let mut text = div()
        .flex()
        .min_w(px(0.))
        .overflow_hidden()
        .text_size(rems(13. / 16.));
    if query.is_empty() {
        return text.child(name.to_owned()).into_any_element();
    }
    let lower = name.to_lowercase();
    let mut start = 0;
    for (offset, matched) in lower.match_indices(&query) {
        // Case folding can change UTF-8 lengths; keep all slices on original character boundaries.
        let original_start = lower[..offset].chars().count();
        let original_end = original_start + matched.chars().count();
        let boundaries = name
            .char_indices()
            .map(|(index, _)| index)
            .chain([name.len()])
            .collect::<Vec<_>>();
        if let (Some(&a), Some(&b)) = (boundaries.get(original_start), boundaries.get(original_end))
        {
            if a >= start {
                text = text.child(name[start..a].to_owned()).child(
                    div()
                        .text_color(theme::accent())
                        .child(name[a..b].to_owned()),
                );
                start = b;
            }
        }
    }
    text.child(name[start..].to_owned()).into_any_element()
}

fn last_use(timestamp: Option<i64>) -> String {
    let Some(time) = timestamp.and_then(|time| chrono::DateTime::from_timestamp(time, 0)) else {
        return crate::i18n::text("launcher.never_used").into();
    };
    let now = chrono::Local::now();
    let local = time.with_timezone(&chrono::Local);
    let hours = (now.timestamp() - time.timestamp()).max(0) / 3600;
    if hours == 0 {
        crate::i18n::text("launcher.just_now").into()
    } else if hours < 24 && local.date_naive() == now.date_naive() {
        crate::i18n::tf("launcher.hours_ago", &[("hours", hours.to_string())])
    } else if local.date_naive() == now.date_naive() - chrono::Duration::days(1) {
        crate::i18n::text("launcher.yesterday").into()
    } else {
        local.format("%Y-%m-%d").to_string()
    }
}

impl Render for ChatLauncher {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.update_retry.is_some() {
            return self.render_update_wait(cx);
        }

        // Opening a tab or closing a dialog parks focus on the window or tab, where the
        // launcher's keys never arrive.
        let reclaim = self.pending.is_none()
            && self.root.upgrade().is_some_and(|root| {
                let root = root.read(cx);
                window
                    .focused(cx)
                    .is_none_or(|focus| focus == root.root_focus || focus == root.chat_focus)
                    && claims_focus(root, self.pane.as_ref())
            });
        if reclaim {
            self.focus(window, cx);
        }
        let (rows, total) = self.rows(cx);
        let query = self.query.read(cx).text().to_owned();
        self.selected = self.selected.min(rows.len().saturating_sub(1));
        let ranked_count = rows.iter().filter(|row| !row.choice.pinned()).count();
        let overrides = self.keymap_overrides.clone();
        let split = !self.default_page;
        let goal_step = matches!(self.step, Step::Goal(_));
        let hide_card = goal_step || (split && rows.is_empty() && !self.loading && total > 0);
        let tight = split && self.height.is_some_and(model::search_is_tight);
        let mut card = div()
            .flex()
            .flex_col()
            .p(rems(4. / 16.))
            .rounded(rems(10. / 16.))
            .border_1()
            .border_color(theme::border())
            .bg(theme::panel());
        if self.loading {
            card = card.child(div().p_3().child(crate::i18n::text("ui.loading")));
        }
        if self.step == Step::Start
            && total == 0
            && !self.loading
            && self.client.supports(&["catalog", "startChat"])
        {
            card = card.child(
                div()
                    .p_3()
                    .text_size(theme::text_ui())
                    .text_color(theme::muted())
                    .child(crate::i18n::text("ui.no_agents_available"))
                    .children((!self.client.is_remote()).then(|| {
                        div()
                            .id("launcher-agents-settings")
                            .cursor_pointer()
                            .text_color(theme::accent())
                            .child(crate::i18n::text("launcher.agents_settings"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                if let Some(root) = this.root.upgrade() {
                                    root.update(cx, |root, cx| {
                                        root.enter_settings_route(Some("agents"), window, cx);
                                    });
                                }
                            }))
                    })),
            );
        }
        for (index, row) in rows.into_iter().enumerate() {
            if index == ranked_count && ranked_count > 0 {
                card = card.child(div().h(px(1.)).mx_2().my_1().bg(theme::border()));
            }
            let selected = index == self.selected;
            let pending = self.pending.as_ref() == Some(&row.choice);
            let badge_width = if row.choice.pinned() { 40. } else { 18. };
            let badge = match &row.choice {
                Choice::Chat(_) => query.is_empty().then(|| (index + 1).to_string()),
                Choice::Crew(_) => {
                    (self.step == Step::Crews && query.is_empty()).then(|| (index + 1).to_string())
                }
                Choice::Terminal => keymap::effective_binding("new-terminal", &overrides)
                    .map(|key| keymap::format_combo(&key)),
                Choice::Mission => keymap::effective_binding("new-mission", &overrides)
                    .map(|key| keymap::format_combo(&key)),
                Choice::EmptyMission => Some(mission::SHIFT_ENTER.into()),
            };
            let mark_size = 20.;
            let mark = if let Some(handles) = &row.crew {
                crate::surfaces::crews::crew_picture(handles, mark_size)
            } else if let Some(seed) = row.avatar {
                RoleAvatar::new(seed, mark_size).into_any_element()
            } else if let Some(runtime) = &row.runtime {
                runtime_mark(runtime, mark_size)
            } else {
                div()
                    .size(rems(mark_size / 16.))
                    .rounded(rems(5. / 16.))
                    .bg(theme::raised())
                    .flex()
                    .items_center()
                    .justify_center()
                    .child(
                        svg()
                            .path(match row.choice {
                                Choice::Terminal => "terminal.svg",
                                Choice::EmptyMission => "plus.svg",
                                _ => "users.svg",
                            })
                            .size(rems((mark_size - 8.) / 16.))
                            .text_color(theme::muted()),
                    )
                    .into_any_element()
            };
            let choice = row.choice.clone();
            let unavailable = row.unavailable;
            card = card.child(
                div()
                    .id(SharedString::from(format!("launcher-row-{index}")))
                    .debug_selector(move || format!("LAUNCHER_ROW:{index}"))
                    .min_h(rems(model::COMPACT_ROW / 16.))
                    .px(rems(10. / 16.))
                    .py(rems(4. / 16.))
                    .flex()
                    .items_center()
                    .gap(rems(8. / 16.))
                    .rounded(rems(6. / 16.))
                    .when(selected, |row| row.bg(theme::sidebar_selected()))
                    .when(row.dimmed, |row| row.opacity(0.45))
                    .cursor_pointer()
                    .hover(|row| row.bg(theme::sidebar_selected()))
                    .children(badge.map(|badge| {
                        div()
                            .debug_selector(move || format!("LAUNCHER_BADGE:{index}"))
                            .flex_none()
                            .flex()
                            .items_center()
                            .justify_center()
                            .min_w(rems(badge_width / 16.))
                            .px(rems(4. / 16.))
                            .py(rems(2. / 16.))
                            .rounded(rems(4. / 16.))
                            .bg(theme::raised())
                            .text_size(rems(11. / 16.))
                            .text_color(theme::muted())
                            .child(badge)
                    }))
                    .child(mark)
                    .child(
                        div()
                            .flex_1()
                            .min_w(px(0.))
                            .flex()
                            .items_center()
                            .gap_2()
                            .child(highlighted(&row.name, &query))
                            .child(
                                div()
                                    .min_w(px(0.))
                                    .truncate()
                                    .text_size(rems(12. / 16.))
                                    .text_color(theme::muted())
                                    .child(row.detail),
                            ),
                    )
                    .children(
                        row.runtime
                            .as_deref()
                            .and_then(|key| {
                                self.runtimes
                                    .iter()
                                    .find(|runtime| runtime.name.key() == key)
                            })
                            .and_then(|runtime| {
                                if !matches!(row.choice, Choice::Chat(EntryKey::Agent(_))) {
                                    return None;
                                }
                                if runtime.below_minimum {
                                    return Some(
                                        div()
                                            .debug_selector(|| {
                                                format!("LAUNCHER_UPDATE_NEEDED:{}", row.name)
                                            })
                                            .text_size(theme::text_meta())
                                            .text_color(theme::warning())
                                            .child(crate::i18n::text("agent_updates.update_needed"))
                                            .into_any_element(),
                                    );
                                }
                                let latest = runtime.available_version.as_ref()?;
                                let name = runtime.name;
                                let root = self.root.clone();
                                let client = self.client.clone();
                                let task = root
                                    .upgrade()
                                    .and_then(|root| root.read(cx).target_task(&client.target, name).cloned())
                                    .filter(|task| task.state.active());
                                let action_label = task.as_ref().map_or(crate::i18n::text("ui.update"), |task| {
                                    if task.state == runner_core::protocol::RuntimeUpdateState::MayNeedInput {
                                        crate::i18n::text("agent_updates.answer")
                                    } else {
                                        crate::i18n::text("agent_updates.show_output")
                                    }
                                });
                                Some(
                                    div()
                                        .id(SharedString::from(format!(
                                            "launcher-update-action-{name}"
                                        )))
                                        .debug_selector(move || format!("LAUNCHER_OPTIONAL_UPDATE:{name}"))
                                        .flex()
                                        .items_center()
                                        .gap_2()
                                        .on_click(|_, _, cx| cx.stop_propagation())
                                        .child(
                                            div()
                                                .text_size(theme::text_meta())
                                                .text_color(theme::muted())
                                                .child(format!(
                                                    "{} → {latest}",
                                                    runtime.installed_version.as_deref().unwrap_or(
                                                        crate::i18n::text("agent_updates.unknown")
                                                    )
                                                )),
                                        )
                                        .child(
                                            Button::new(
                                                SharedString::from(format!(
                                                    "launcher-update-{name}"
                                                )),
                                                action_label,
                                            )
                                            .size(runner_app::ui::ButtonSize::Sm)
                                            .variant(ButtonVariant::Ghost)
                                            .on_press(
                                                move |window, cx| {
                                                    let _ = root.update(cx, |root, cx| {
                                                        if let Some(task) = &task {
                                                            root.show_target_task(
                                                                task.clone(),
                                                                Some(client.clone()),
                                                                window,
                                                                cx,
                                                            );
                                                        } else {
                                                            root.start_target_update(
                                                                client.clone(),
                                                                name,
                                                                window,
                                                                cx,
                                                            );
                                                        }
                                                    });
                                                },
                                            ),
                                        )
                                        .into_any_element(),
                                )
                            }),
                    )
                    .when(
                        !query.is_empty()
                            && matches!(row.choice, Choice::Chat(_))
                            && (row.used_at.is_some() || !row.previously_used),
                        |row_div| {
                            row_div.child(
                                div()
                                    .flex_none()
                                    .text_size(rems(10. / 16.))
                                    .text_color(theme::muted())
                                    .child(last_use(row.used_at)),
                            )
                        },
                    )
                    .when(pending, |row| {
                        row.child(runner_app::ui::button::spinner(
                            "launcher-starting",
                            14.,
                            theme::accent(),
                        ))
                    })
                    .when(selected && !pending, |row| {
                        row.child(div().text_color(theme::muted()).child("↵"))
                    })
                    .on_click(cx.listener(move |this, _, window, cx| {
                        this.start(choice.clone(), unavailable, window, cx)
                    })),
            );
        }
        // A split pane's chat files with its tab and starts in its neighbour's folder.
        let location = (!split).then(|| {
            let change = div()
                .id("launcher-change")
                .debug_selector(|| "LAUNCHER_CHANGE".into())
                .relative()
                .cursor_pointer()
                .text_size(rems(12. / 16.))
                .text_color(theme::accent())
                .child(crate::i18n::text("launcher.change"))
                .children(
                    self.folder_picker
                        .as_ref()
                        .map(|picker| FolderPicker::anchor(picker.clone())),
                )
                .on_click(cx.listener(|this, _, window, cx| this.change_directory(window, cx)));
            div()
                .debug_selector(|| "LAUNCHER_LOCATION".into())
                .h(rems(20. / 16.))
                .flex_none()
                .min_w(px(0.))
                .flex()
                .items_center()
                .gap_2()
                .child(
                    svg()
                        .path("folder.svg")
                        .debug_selector(|| "LAUNCHER_LOCATION_FOLDER".into())
                        .size(rems(1.))
                        .text_color(theme::muted()),
                )
                .child(div().max_w(rems(140. / 16.)).child(self.project.select()))
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(rems(11. / 16.))
                        .text_color(theme::faint())
                        .child(self.cwd.read(cx).text().to_owned()),
                )
                .child(change)
        });
        let field = if goal_step {
            self.render_goal_field(window, cx)
        } else {
            div()
                .h(rems(model::COMPACT_SEARCH / 16.))
                .debug_selector(|| "LAUNCHER_SEARCH".into())
                .key_context("LauncherSearch")
                .on_mouse_down(
                    MouseButton::Left,
                    cx.listener(|this, _, window, cx| {
                        this.focus(window, cx);
                        cx.stop_propagation();
                    }),
                )
                .px(rems(12. / 16.))
                .flex()
                .items_center()
                .gap_2()
                .rounded(rems(8. / 16.))
                .border_1()
                .border_color(theme::border())
                .bg(theme::panel())
                .child(
                    svg()
                        .path("search.svg")
                        .size(rems(1.))
                        .text_color(theme::muted()),
                )
                .children((self.step == Step::Crews).then(|| self.render_mission_chip(cx)))
                .child(div().flex_1().min_w(px(0.)).child(self.query.clone()))
                .into_any_element()
        };
        // The mark stays on every step, so a tab reads as the same tab throughout.
        let brand = self.default_page.then(|| {
            div()
                .debug_selector(|| "LAUNCHER_BRAND".into())
                .flex_none()
                .pb(rems(12. / 16.))
                .flex()
                .flex_col()
                .items_center()
                .gap(rems(12. / 16.))
                .child(
                    div()
                        .w(rems(52. / 16.))
                        .h(rems(40. / 16.))
                        .flex()
                        .items_center()
                        .justify_center()
                        .child(
                            svg()
                                .flex_none()
                                .path("brand-mark.svg")
                                .size(rems(70. / 16.))
                                .text_color(theme::accent()),
                        ),
                )
                .child(
                    div()
                        .text_size(theme::text_body())
                        .text_color(theme::muted())
                        .child(crate::i18n::text("ui.start_something_new")),
                )
        });
        let launcher = cx.entity().downgrade();
        div()
            .debug_selector({
                let default_page = self.default_page;
                move || {
                    if default_page {
                        "START_SURFACE".into()
                    } else {
                        "EMPTY_PANE_START".into()
                    }
                }
            })
            .capture_key_down(cx.listener(|this, event: &KeyDownEvent, window, cx| {
                if this.field().read(cx).focus_handle().is_focused(window)
                    && this.on_key(event, window, cx)
                {
                    cx.stop_propagation();
                    window.prevent_default();
                }
            }))
            .relative()
            .flex_1()
            .min_h(px(0.))
            .min_w(px(0.))
            .flex()
            .items_center()
            .justify_center()
            .px(rems(20. / 16.))
            .py(rems(
                if tight {
                    0.
                } else if split {
                    model::COMPACT_PADDING
                } else {
                    20.
                } / 16.,
            ))
            .overflow_hidden()
            .child(
                div()
                    .id("launcher-column")
                    .max_h_full()
                    .overflow_y_scroll()
                    .debug_selector(|| "START_SURFACE_COLUMN".into())
                    .w(rems(if self.default_page { 440. } else { 400. } / 16.))
                    .max_w_full()
                    .min_w(px(0.))
                    .flex()
                    .flex_col()
                    .gap(rems(if split { model::COMPACT_GAP } else { 12. } / 16.))
                    .children(brand)
                    .child(field)
                    .children(location)
                    .children(self.error.as_ref().map(|error| {
                        runner_app::ui::notice_banner(error.clone(), runner_app::ui::Tone::Danger)
                    }))
                    .when(!hide_card, |column| column.child(card))
                    .children(
                        self.runtimes
                            .iter()
                            .find(|runtime| runtime.below_minimum)
                            .map(|runtime| {
                                div()
                                    .debug_selector(|| "LAUNCHER_VERSION_NOTICE".into())
                                    .flex()
                                    .flex_col()
                                    .gap_1()
                                    .child(
                                        div()
                                            .text_size(theme::text_ui())
                                            .text_color(theme::warning())
                                            .child(crate::i18n::tf(
                                                "agent_updates.minimum_title",
                                                &[
                                                    ("runtime", runtime.display_name.clone()),
                                                    (
                                                        "installed",
                                                        runtime
                                                            .installed_version
                                                            .clone()
                                                            .unwrap_or_default(),
                                                    ),
                                                ],
                                            )),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme::text_meta())
                                            .text_color(theme::muted())
                                            .child(crate::i18n::tf(
                                                "agent_updates.minimum_notice",
                                                &[
                                                    (
                                                        "minimum",
                                                        runtime
                                                            .min_version
                                                            .clone()
                                                            .unwrap_or_default(),
                                                    ),
                                                    ("runtime", runtime.display_name.clone()),
                                                    (
                                                        "latest",
                                                        runtime
                                                            .available_version
                                                            .clone()
                                                            .unwrap_or_else(|| {
                                                                crate::i18n::text(
                                                                    "agent_updates.latest_version",
                                                                )
                                                                .into()
                                                            }),
                                                    ),
                                                ],
                                            )),
                                    )
                            }),
                    )
                    .when(goal_step, |column| {
                        column.children(self.render_goal_body(cx))
                    }),
            )
            .children(self.folder_picker.clone())
            .child(
                canvas(
                    |_, _, _| {},
                    move |bounds, _, window, cx| {
                        let height =
                            f32::from(bounds.size.height) / f32::from(window.rem_size()) * 16.;
                        // A notify while the frame is painting would not schedule the next one.
                        cx.defer(move |cx| {
                            let _ = launcher.update(cx, |launcher, cx| {
                                if launcher.height != Some(height) {
                                    launcher.height = Some(height);
                                    cx.notify();
                                }
                            });
                        });
                    },
                )
                .absolute()
                .inset_0(),
            )
    }
}

impl ChatLauncher {
    fn render_update_wait(&self, cx: &mut Context<Self>) -> gpui::Div {
        let retry = self.update_retry.as_ref().unwrap();
        let task = self.root.upgrade().and_then(|root| {
            root.read(cx)
                .target_task(&self.client.target, retry.runtime)
                .cloned()
        });
        let answer = task.as_ref().is_some_and(|task| {
            task.state == runner_core::protocol::RuntimeUpdateState::MayNeedInput
        });
        let waiting = retry.requested && task.as_ref().is_some_and(|task| task.state.active());
        let output_task = task.clone();
        let name = retry.runtime.display_name();
        let title = if waiting {
            crate::i18n::tf("agent_updates.waiting", &[("runtime", name.into())])
        } else {
            crate::i18n::tf("agent_updates.needs_update", &[("runtime", name.into())])
        };
        let reason = task
            .and_then(|task| task.message)
            .or_else(|| {
                retry.gate.as_ref().map(|gate| {
                    crate::i18n::tf(
                        "agent_updates.minimum_reason",
                        &[
                            ("runtime", name.into()),
                            ("installed", gate.installed_version.clone()),
                            ("minimum", gate.min_version.clone()),
                        ],
                    )
                })
            })
            .unwrap_or_else(|| {
                crate::i18n::tf("agent_updates.update_to_use", &[("runtime", name.into())])
            });
        let root = self.root.clone();
        let owner = cx.weak_entity();
        let runtime = retry.runtime;
        let client = self.client.clone();
        div().size_full().relative().child(
            runner_app::ui::SessionOverlay::ended(
                "launcher-update-wait",
                reason,
                move |window, cx| {
                    let _ = owner.update(cx, |view, cx| {
                        if let Some(retry) = &mut view.update_retry {
                            retry.requested = true;
                        }
                        cx.notify();
                    });
                    let _ = root.update(cx, |root, cx| {
                        if answer {
                            if let Some(task) = &output_task {
                                root.show_target_task(
                                    task.clone(),
                                    Some(client.clone()),
                                    window,
                                    cx,
                                );
                            }
                        } else {
                            root.start_target_update(client.clone(), runtime, window, cx);
                        }
                    });
                },
                |_, _| {},
            )
            .title(title)
            .primary_debug_selector("LAUNCHER_UPDATE_ACTION")
            .label(if answer {
                crate::i18n::text("agent_updates.answer").into()
            } else {
                crate::i18n::tf("agent_updates.update_action", &[("runtime", name.into())])
            })
            .actions_visible(!waiting || answer, false),
        )
    }
}

impl NativeRoot {
    pub(crate) fn refresh_remote_agent_versions(
        &self,
        client: FrontPageClient,
        cx: &mut Context<Self>,
    ) {
        let target = client.target.clone();
        let work = cx.background_spawn(async move { client.catalog() });
        cx.spawn(async move |owner, cx| {
            let Ok(catalog) = work.await else { return };
            let _ = owner.update(cx, |root, cx| {
                let supported = catalog
                    .runtimes
                    .iter()
                    .filter(|runtime| !runtime.below_minimum)
                    .filter_map(|runtime| Runtime::parse(&runtime.name))
                    .collect::<Vec<_>>();
                root.clear_unblocked_gates(&target, &supported);
                for launcher in root.launchers.values() {
                    if launcher.read(cx).client.target == target {
                        launcher.update(cx, |launcher, cx| {
                            launcher.runtimes = catalog
                                .runtimes
                                .iter()
                                .filter_map(RuntimeChoice::remote)
                                .collect();
                            cx.notify();
                        });
                    }
                }
            });
        })
        .detach();
    }

    pub(crate) fn show_fork_update(
        &mut self,
        gate: runner_core::protocol::RuntimeVersionGate,
        request: super::super::chat::ForkRequest,
        launcher: Option<Entity<ChatLauncher>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let launcher = if let Some(launcher) = launcher {
            launcher
        } else {
            if request.client.target != self.front_page_client(cx).target {
                return;
            }
            self.open_launcher_page(ProjectScope::Root, Step::Start, false, window, cx);
            self.render_launcher(None, true, window, cx)
        };
        launcher.update(cx, |launcher, cx| {
            launcher.update_retry = Some(UpdateRetry {
                runtime: gate.runtime,
                gate: Some(gate),
                action: UpdateAction::Fork(request),
                requested: false,
                ready: false,
            });
            launcher.pending = None;
            cx.notify();
        });
    }

    pub(crate) fn update_launcher_visible(
        &self,
        launcher: &Entity<ChatLauncher>,
        cx: &App,
    ) -> bool {
        let view = launcher.read(cx);
        if view.client.target != self.front_page_client(cx).target {
            return false;
        }
        if let Some(pane) = &view.pane {
            self.front_page_is_chat()
                && self.front_page_layout().is_some_and(|layout| {
                    layout.id == pane.tab_id && layout.focused_pane_id == pane.pane_id
                })
        } else {
            self.launcher_page_visible() && self.page_launcher(cx) == Some(launcher)
        }
    }

    pub(crate) fn notify_update_launchers(&self, cx: &mut Context<Self>) {
        for launcher in self.launchers.values() {
            launcher.update(cx, |_, cx| cx.notify());
        }
    }
    pub(crate) fn retry_gated_launches(
        &mut self,
        runtime: Runtime,
        target: &FrontPageTarget,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        for launcher in self.launchers.values() {
            launcher.update(cx, |view, _| {
                if &view.client.target == target {
                    if let Some(retry) = &mut view.update_retry {
                        if retry.runtime == runtime && retry.requested {
                            retry.ready = true;
                        }
                    }
                }
            });
        }
        self.drain_ready_launches(window, cx);
    }

    pub(crate) fn drain_ready_launches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.front_page_client(cx).target;
        let launchers = self.launchers.values().cloned().collect::<Vec<_>>();
        for launcher in launchers {
            let view = launcher.read(cx);
            let Some(retry) = view.update_retry.as_ref().filter(|retry| retry.ready) else {
                continue;
            };
            if !view.client.valid() || (view.client.is_remote() && view.client.target != current) {
                continue;
            }
            if let UpdateAction::Chat {
                pane: Some(pane), ..
            } = &retry.action
            {
                let key = format!("{}:{}", computer_key(&view.client.target), pane.tab_id);
                let blocked = if view.client.is_remote() {
                    self.launcher_tab_pending(&pane.tab_id, cx)
                } else {
                    self.app_store
                        .read(cx)
                        .launcher_tabs_pending
                        .lock()
                        .unwrap()
                        .contains_key(&key)
                };
                if blocked {
                    continue;
                }
            }
            let retry = launcher
                .update(cx, |view, _| view.update_retry.take())
                .unwrap();
            match retry.action {
                UpdateAction::Chat {
                    key,
                    pane,
                    scope,
                    cwd,
                } => self.launch_from_launcher(launcher, key, pane, scope, cwd, true, window, cx),
                UpdateAction::Mission(request) => {
                    self.launch_mission_from_launcher(launcher, request, true, window, cx)
                }
                UpdateAction::Fork(request) => self.retry_agent_fork(request, launcher, window, cx),
            }
        }
    }
}

impl NativeRoot {
    pub(crate) fn refresh_launchers(&mut self, cx: &mut Context<Self>) {
        let (runtimes, _, error) = load_selectable_runtimes(self.core(cx), self.settings(cx));
        let runtimes = runtimes
            .into_iter()
            .map(Into::into)
            .collect::<Vec<RuntimeChoice>>();
        let roles = self
            .app_store
            .read(cx)
            .roles
            .clone()
            .into_iter()
            .map(Into::into)
            .collect::<Vec<RoleChoice>>();
        let default = self.settings(cx).default_runtime.clone();
        let overrides = self.settings(cx).keymap_overrides.clone();
        for launcher in self.launchers.values() {
            launcher.update(cx, |launcher, cx| {
                if !launcher.client.is_remote() {
                    launcher.runtimes = runtimes.clone();
                    launcher.roles = roles.clone();
                    launcher.default_runtime = default.clone();
                    launcher.keymap_overrides = overrides.clone();
                    launcher.error = error.clone();
                    cx.notify();
                }
            });
        }
    }

    pub(crate) fn prune_launchers(&mut self, cx: &App) {
        let target = self.front_page_client(cx).target;
        let tabs = &self.tabs;
        let remote = &self.remote_client;
        let active_layout = self.front_page_layout().cloned();
        self.launchers.retain(|_, entity| {
            let launcher = entity.read(cx);
            if launcher.update_retry.is_some() && launcher.client.valid() {
                return true;
            }
            if launcher.client.target != target {
                // A reconnect's new launcher takes the page over when it is built.
                return launcher.pane.is_none()
                    && computer_key(&launcher.client.target) == computer_key(&target);
            }
            if launcher.launched_session.as_ref().is_some_and(|session| {
                active_layout
                    .as_ref()
                    .is_some_and(|layout| layout.contains_session(session))
            }) {
                return false;
            }
            let Some(pane) = &launcher.pane else {
                return true;
            };
            let layout = if launcher.client.is_remote() {
                remote.tab_layout(&pane.tab_id)
            } else {
                tabs.tabs().iter().find(|layout| layout.id == pane.tab_id)
            };
            layout.is_none_or(|layout| {
                layout
                    .root
                    .leaves()
                    .iter()
                    .find(|leaf| leaf.id == pane.pane_id)
                    .is_none_or(|leaf| leaf.session_id.is_none())
            })
        });
    }

    pub(crate) fn focus_visible_launcher(&self, window: &mut Window, cx: &mut App) -> bool {
        if !self.front_page_is_chat() || self.create_modal_open() {
            return false;
        }
        let target = self.front_page_client(cx).target;
        let pane = self
            .front_page_layout()
            .map(|layout| PaneKey::new(&layout.id, &layout.focused_pane_id));
        if self
            .front_page_layout()
            .is_some_and(|layout| layout.focused_session_id().is_some())
        {
            return false;
        }
        if let Some(launcher) = self
            .launchers
            .get(&format!("{target:?}:{pane:?}"))
            .filter(|launcher| self.update_launcher_visible(launcher, cx))
            .cloned()
        {
            if launcher.read(cx).update_retry.is_some() {
                self.root_focus.focus(window, cx);
            } else {
                launcher.update(cx, |launcher, cx| launcher.focus(window, cx));
            }
            return true;
        }
        false
    }

    pub(crate) fn launcher_session_pending(&self, session: &str, cx: &App) -> bool {
        let layout = if self.remote_client.is_active() {
            self.remote_client.session_tab_layout(session)
        } else {
            self.tabs
                .tabs()
                .iter()
                .find(|layout| layout.contains_session(session))
        };
        layout.is_some_and(|layout| self.launcher_tab_pending(&layout.id, cx))
    }

    pub(crate) fn launcher_tab_pending(&self, tab: &str, cx: &App) -> bool {
        let key = format!("{}:{tab}", computer_key(&self.front_page_client(cx).target));
        let pending = self.app_store.read(cx).launcher_tabs_pending.clone();
        let mut pending = pending.lock().unwrap();
        if let (FrontPageTarget::Remote { computer, .. }, Some(Some(before))) =
            (&self.front_page_client(cx).target, pending.get(&key))
        {
            let clients = self.app_store.read(cx).remote_clients.read(cx);
            if clients.overviews.get(computer).is_some_and(|overview| {
                overview
                    .tabs
                    .iter()
                    .find(|candidate| candidate.id == tab)
                    .is_none_or(|tab| {
                        !crate::surfaces::front_page::model::matches_layout(before, tab)
                    })
            }) {
                pending.remove(&key);
            }
        }
        pending.contains_key(&key) || self.remote_client.layout_pending(tab)
    }

    pub(crate) fn launcher_start_pending(&self, cx: &App) -> bool {
        let target = self.front_page_client(cx).target;
        self.launchers.values().any(|launcher| {
            let launcher = launcher.read(cx);
            launcher.client.target == target
                && launcher.pending.is_some()
                && launcher.pane.as_ref().is_none_or(|pane| {
                    self.front_page_layout().is_some_and(|layout| {
                        layout.id == pane.tab_id
                            && layout
                                .root
                                .leaves()
                                .iter()
                                .any(|leaf| leaf.id == pane.pane_id && leaf.session_id.is_none())
                    })
                })
        })
    }

    pub(crate) fn start_launcher_terminal(
        &mut self,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> bool {
        if !self.front_page_is_chat() {
            return false;
        }
        let pane = self
            .front_page_layout()
            .map(|layout| PaneKey::new(&layout.id, &layout.focused_pane_id));
        if self
            .front_page_layout()
            .is_some_and(|layout| layout.focused_session_id().is_some())
        {
            return false;
        }
        let target = self.front_page_client(cx).target;
        let launcher = self
            .launchers
            .values()
            .find(|launcher| {
                let launcher = launcher.read(cx);
                launcher.client.target == target && launcher.pane == pane
            })
            .cloned();
        if let Some(launcher) = launcher {
            let input = launcher.update(cx, |launcher, cx| {
                if !launcher.client.supports(&["startShell", "layout"])
                    || launcher.pending.is_some()
                {
                    return None;
                }
                launcher.pending = Some(Choice::Terminal);
                cx.notify();
                Some((
                    launcher.pane.clone(),
                    launcher.scope(),
                    normalized_value(launcher.cwd.read(cx).text()),
                ))
            });
            if let Some((pane, scope, cwd)) = input {
                self.launch_from_launcher(launcher, None, pane, scope, cwd, false, window, cx);
            }
            return true;
        }
        false
    }

    pub(crate) fn render_launcher(
        &mut self,
        pane: Option<PaneKey>,
        default_page: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ChatLauncher> {
        self.cached_launcher(pane, default_page, window, cx)
    }

    fn cached_launcher(
        &mut self,
        pane: Option<PaneKey>,
        default_page: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ChatLauncher> {
        let default_page = pane
            .as_ref()
            .and_then(|pane| {
                self.front_page_layout()
                    .filter(|layout| layout.id == pane.tab_id)
            })
            .map_or(default_page, |layout| {
                let leaves = layout.root.leaves();
                leaves.len() == 1 && leaves[0].session_id.is_none()
            });
        let target = self.front_page_client(cx).target;
        let key = format!("{target:?}:{pane:?}");
        if let Some(launcher) = self.launchers.get(&key).cloned() {
            let context_changed = launcher.read(cx).default_page != default_page
                || (pane.is_some()
                    && !default_page
                    && launcher.read(cx).initial_project != self.active_tab_project_id(cx));
            if context_changed {
                let (query, directory) = {
                    let previous = launcher.read(cx);
                    (
                        previous.query.read(cx).text().to_owned(),
                        previous
                            .changed_directory
                            .then(|| previous.cwd.read(cx).text().to_owned()),
                    )
                };
                let root = cx.weak_entity();
                let next =
                    cx.new(|cx| ChatLauncher::new(self, pane, default_page, window, cx, root));
                next.update(cx, |launcher, cx| {
                    launcher
                        .query
                        .update(cx, |field, cx| field.reset(query, cx));
                    if let Some(directory) = directory {
                        launcher.project.picked(&directory, cx);
                        launcher
                            .cwd
                            .update(cx, |field, cx| field.reset(directory, cx));
                        launcher.changed_directory = true;
                    }
                });
                self.launchers.insert(key, next.clone());
                return next;
            }
            if launcher
                .read(cx)
                .update_retry
                .as_ref()
                .is_some_and(|retry| retry.ready)
            {
                cx.defer_in(window, |root, window, cx| {
                    root.drain_ready_launches(window, cx)
                });
            }
            let computer = self.remote_client.computer().cloned();
            let overrides = self.settings(cx).keymap_overrides.clone();
            let revisions = self.app_store.read(cx).revisions;
            let catalog_revisions = (revisions.roles, revisions.projects, revisions.crews);
            let catalogs = (!launcher.read(cx).client.is_remote()
                && launcher.read(cx).catalog_revisions != catalog_revisions)
                .then(|| {
                    let store = self.app_store.read(cx);
                    (
                        store
                            .roles
                            .clone()
                            .into_iter()
                            .map(Into::into)
                            .collect::<Vec<RoleChoice>>(),
                        ProjectChoice::local(&store.projects),
                        mission::local_crews(&store.crews),
                    )
                });
            let recent_revision = RECENT_REVISION.load(std::sync::atomic::Ordering::Acquire);
            let project_deleted = launcher
                .read(cx)
                .page_project
                .as_deref()
                .is_some_and(|project| self.front_page_project_deleted(project, cx));
            launcher.update(cx, |launcher, cx| {
                if launcher.recent_revision != recent_revision {
                    launcher.reload_recents(cx);
                }
                if let Some((roles, projects, crews)) = catalogs {
                    if launcher.roles != roles {
                        launcher.roles = roles;
                        launcher.selected = 0;
                        cx.notify();
                    }
                    if launcher.crews != crews {
                        launcher.crews = crews;
                        cx.notify();
                    }
                    launcher.project.set_choices(projects, cx);
                    launcher.catalog_revisions = catalog_revisions;
                }
                if project_deleted {
                    launcher.retarget_page(None, false, cx);
                }
                if launcher.keymap_overrides != overrides {
                    launcher.keymap_overrides = overrides;
                    cx.notify();
                }
                if launcher.client.is_remote()
                    && !launcher.catalog_requested
                    && launcher.client.supports(&["catalog"])
                {
                    launcher.load_host(computer, window, cx);
                }
            });
            return launcher;
        }
        let locked_project = pane
            .is_none()
            .then(|| self.take_replaced_page_project(cx))
            .flatten();
        let root = cx.weak_entity();
        let launcher = cx.new(|cx| ChatLauncher::new(self, pane, default_page, window, cx, root));
        if locked_project.is_some() {
            launcher.update(cx, |launcher, cx| {
                launcher.retarget_page(locked_project.as_deref(), true, cx);
            });
        }
        self.launchers.insert(key, launcher.clone());
        launcher
    }

    /// Removes the page launcher a reconnect left behind, whose client is no
    /// longer valid, and returns the project it was locked to.
    fn take_replaced_page_project(&mut self, cx: &App) -> Option<String> {
        let target = self.front_page_client(cx).target;
        let mut project = None;
        self.launchers.retain(|_, launcher| {
            let launcher = launcher.read(cx);
            let replaced = launcher.pane.is_none()
                && launcher.client.target != target
                && computer_key(&launcher.client.target) == computer_key(&target);
            if replaced {
                project = launcher.page_project.clone();
            }
            !replaced
        });
        project
    }

    pub(super) fn new_tab_project_id(&self, cx: &App) -> Option<String> {
        let visible = self.launcher_page_visible();
        if !visible && self.remote_client.is_local() {
            return self.active_tab_project_id(cx);
        }
        let origin = self
            .page_launcher(cx)
            .filter(|_| visible)
            .and_then(|page| page.read(cx).origin.clone());
        if self.remote_client.is_active() {
            let tab = match origin {
                Some(LauncherOrigin::Remote(Some(
                    crate::surfaces::remote_client::RemoteItem::Tab(id),
                ))) => self.remote_client.tab_layout(&id),
                _ => self.remote_client.layout(),
            };
            return tab
                .or_else(|| self.remote_client.recently_viewed_tab_layout())
                .and_then(|layout| layout.parent_id.clone());
        }
        let tab = match origin {
            Some(LauncherOrigin::Local { tab, .. }) => tab,
            _ => None,
        };
        let tab = tab
            .filter(|id| self.tabs.tabs().iter().any(|tab| &tab.id == id))
            .or_else(|| self.recently_viewed_tab_id(cx))?;
        let nodes = &self.app_store.read(cx).nodes;
        let node = nodes.iter().find(|node| node.id == tab)?;
        crate::surfaces::sidebar::node_project_id(nodes, node)
    }

    /// The launcher page in `scope`. `locked` is a project's +, where the row
    /// sits under the project and the picker stays on it; every other entry
    /// draws the row in Recents and leaves the picker free.
    pub(crate) fn open_launcher_page(
        &mut self,
        scope: ProjectScope,
        step: Step,
        locked: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.create_modal_open() || self.launcher_start_pending(cx) {
            return;
        }
        let visible = self.launcher_page_visible()
            && self
                .page_launcher(cx)
                .is_none_or(|launcher| launcher.read(cx).update_retry.is_none());
        let origin = if self.remote_client.is_active() {
            LauncherOrigin::Remote(self.remote_client.active_item().cloned())
        } else {
            LauncherOrigin::Local {
                route: self.route.clone(),
                tab: self.tabs.active_tab_id().map(str::to_owned),
            }
        };
        if self.remote_client.is_active() {
            self.select_remote_item(crate::surfaces::remote_client::RemoteItem::Launcher, cx);
        } else {
            self.set_route(AppRoute::Launcher, cx);
        }
        if !visible {
            self.reset_launcher_page(cx);
        }
        self.front_page_sidebar().update(cx, |sidebar, cx| {
            sidebar.set_active_project(scope.project_id().map(str::to_owned), cx)
        });
        let launcher = self.render_launcher(None, true, window, cx);
        launcher.update(cx, |launcher, cx| {
            if !visible {
                launcher.origin = Some(origin);
            }
            launcher.retarget_page(scope.project_id(), locked, cx);
            launcher.open_step(step, window, cx);
        });
        cx.notify();
    }

    /// The project the launcher page's row sits under, Recents when `None`.
    /// Until a reconnect's new launcher is built, the one it replaces still
    /// says, so the row does not move in between.
    pub(crate) fn launcher_page_project(&self, cx: &App) -> Option<String> {
        let computer = computer_key(&self.front_page_client(cx).target);
        let launcher = self.page_launcher(cx).or_else(|| {
            self.launchers.values().find(|launcher| {
                let launcher = launcher.read(cx);
                launcher.pane.is_none() && computer_key(&launcher.client.target) == computer
            })
        })?;
        launcher.read(cx).page_project.clone()
    }

    fn page_launcher(&self, cx: &App) -> Option<&Entity<ChatLauncher>> {
        let target = self.front_page_client(cx).target;
        self.launchers
            .get(&format!("{target:?}:{:?}", None::<PaneKey>))
    }

    /// Forgets the page launcher, which is cached between visits and could hold
    /// an older project, lock and origin. The next page starts fresh.
    pub(crate) fn reset_launcher_page(&mut self, cx: &App) {
        let computer = computer_key(&self.front_page_client(cx).target);
        let mut waiting = Vec::new();
        self.launchers.retain(|_, entity| {
            let launcher = entity.read(cx);
            if launcher.pane.is_some() || computer_key(&launcher.client.target) != computer {
                return true;
            }
            if launcher.update_retry.is_some() {
                waiting.push((
                    format!("update-wait:{:?}", entity.entity_id()),
                    entity.clone(),
                ));
            }
            false
        });
        self.launchers.extend(waiting);
    }

    fn front_page_project_deleted(&self, project: &str, cx: &App) -> bool {
        match &self.front_page_client(cx).target {
            FrontPageTarget::Local => !self
                .app_store
                .read(cx)
                .projects
                .iter()
                .any(|row| row.id == project),
            FrontPageTarget::Remote { computer, .. } => self
                .app_store
                .read(cx)
                .remote_clients
                .read(cx)
                .overviews
                .get(computer)
                .is_some_and(|overview| !overview.projects.iter().any(|row| row.id == project)),
        }
    }

    /// With no tab or page behind it the launcher is the window's empty state,
    /// and there is nothing to close it back to. A page with no recorded origin
    /// is that state after a tab arrived from another window, and closes to it.
    pub(crate) fn launcher_page_closable(&self, cx: &App) -> bool {
        self.launcher_page_visible()
            && self
                .page_launcher(cx)
                .is_some_and(|launcher| match &launcher.read(cx).origin {
                    Some(mission::LauncherOrigin::Local {
                        route: AppRoute::Chat,
                        ..
                    }) => !self.tabs.tabs().is_empty(),
                    None => !self.remote_client.is_active() && !self.tabs.tabs().is_empty(),
                    Some(mission::LauncherOrigin::Remote(None)) => false,
                    Some(_) => true,
                })
    }

    pub(crate) fn close_launcher_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.launcher_page_closable(cx) {
            return;
        }
        if let Some(launcher) = self.page_launcher(cx).cloned() {
            launcher.update(cx, |launcher, cx| launcher.leave_page(window, cx));
        }
    }

    pub(crate) fn launcher_page_visible(&self) -> bool {
        self.launcher_is_empty_state()
            || if self.remote_client.is_active() {
                matches!(
                    self.remote_client.active_item(),
                    Some(crate::surfaces::remote_client::RemoteItem::Launcher)
                )
            } else {
                self.route == AppRoute::Launcher
            }
    }

    /// The launcher as the window's empty state: nothing was opened, and there
    /// is no tab to show. It is always Recents with no project.
    pub(crate) fn launcher_is_empty_state(&self) -> bool {
        if self.remote_client.is_active() {
            self.remote_client.active_item().is_none() && self.front_page_layout().is_none()
        } else {
            self.route == AppRoute::Chat && self.tabs.active().is_none()
        }
    }

    fn reserve_update_pane(
        &mut self,
        launcher: &Entity<ChatLauncher>,
        pane: Option<PaneKey>,
        scope: &ProjectScope,
        cx: &mut Context<Self>,
    ) -> Option<PaneKey> {
        if pane.is_some() || launcher.read(cx).client.is_remote() {
            return pane;
        }
        let mut layout = PaneLayout::single(None, &[]);
        layout.id = runner_core::event_log::UlidGen::new().next().unwrap();
        layout.parent_id = scope.project_id().map(str::to_owned);
        if let Err(error) = launcher
            .read(cx)
            .client
            .launcher_layout(&layout, layout.parent_id.clone())
        {
            launcher.update(cx, |view, cx| {
                view.error = Some(error.to_string());
                cx.notify();
            });
            return None;
        }
        let _ = self.reload_tabs(cx);
        self.tabs.activate(&layout.id);
        self.set_route(AppRoute::Chat, cx);
        let pane = PaneKey::new(&layout.id, &layout.focused_pane_id);
        self.launchers.retain(|_, cached| cached != launcher);
        launcher.update(cx, |view, _| {
            view.pane = Some(pane.clone());
            view.initial_project = scope.project_id().map(str::to_owned);
        });
        self.launchers.insert(
            format!("{:?}:{:?}", launcher.read(cx).client.target, Some(&pane)),
            launcher.clone(),
        );
        Some(pane)
    }

    #[allow(clippy::too_many_arguments)]
    fn launch_from_launcher(
        &mut self,
        launcher: Entity<ChatLauncher>,
        key: Option<EntryKey>,
        pane: Option<PaneKey>,
        scope: ProjectScope,
        cwd: Option<String>,
        background: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let client = launcher.read(cx).client.clone();
        if !client.valid()
            || (client.target != self.front_page_client(cx).target
                && (!background || client.is_remote()))
        {
            return;
        }
        let gated = key.as_ref().and_then(|key| {
            let view = launcher.read(cx);
            let runtime_key = match key {
                EntryKey::Agent(runtime) => Some(runtime.as_str()),
                EntryKey::Role(id) => view
                    .roles
                    .iter()
                    .find(|role| role.id == *id)
                    .map(|role| role.runtime.as_str()),
            }?;
            view.runtimes
                .iter()
                .find(|runtime| runtime.name.key() == runtime_key && runtime.below_minimum)
                .map(|runtime| runtime.name)
        });
        if let Some(runtime) = gated.filter(|runtime| {
            !self
                .target_task(&client.target, *runtime)
                .is_some_and(|task| {
                    task.state == runner_core::protocol::RuntimeUpdateState::Succeeded
                })
        }) {
            let pane = self.reserve_update_pane(&launcher, pane, &scope, cx);
            launcher.update(cx, |view, cx| {
                view.pending = None;
                view.update_retry = Some(UpdateRetry {
                    runtime,
                    gate: view
                        .runtimes
                        .iter()
                        .find(|entry| entry.name == runtime)
                        .and_then(|entry| {
                            Some(runner_core::protocol::RuntimeVersionGate {
                                runtime,
                                installed_version: entry.installed_version.clone()?,
                                min_version: entry.min_version.clone()?,
                            })
                        }),
                    action: UpdateAction::Chat {
                        key,
                        pane,
                        scope,
                        cwd,
                    },
                    requested: true,
                    ready: false,
                });
                cx.notify();
            });
            self.start_target_update(client, runtime, window, cx);
            return;
        }
        let has_pane = pane.is_some();
        let mut placement = pane.and_then(|pane| {
            let layout = if client.is_remote() {
                self.remote_client.tab_layout(&pane.tab_id)
            } else {
                self.tabs
                    .tabs()
                    .iter()
                    .find(|layout| layout.id == pane.tab_id)
            };
            layout
                .filter(|layout| {
                    layout.id == pane.tab_id
                        && layout
                            .root
                            .leaves()
                            .iter()
                            .any(|leaf| leaf.id == pane.pane_id && leaf.session_id.is_none())
                })
                .map(|layout| (layout.clone(), pane.pane_id))
        });
        if has_pane && placement.is_none() {
            launcher.update(cx, |launcher, cx| {
                launcher.pending = None;
                launcher.error =
                    Some(crate::i18n::text("ui.the_target_tab_is_no_longer_active").into());
                cx.notify();
            });
            return;
        }
        if launcher.read(cx).default_page {
            if let Some((layout, _)) = placement.as_mut() {
                layout.parent_id = scope.project_id().map(str::to_owned);
            }
        }
        let mut tab_guard = if let Some((layout, _)) = &placement {
            let guard = if self.launcher_tab_pending(&layout.id, cx) {
                None
            } else {
                let key = format!("{}:{}", computer_key(&client.target), layout.id);
                TabStart::begin(
                    self.app_store.read(cx).launcher_tabs_pending.clone(),
                    key,
                    layout.clone(),
                )
            };
            let Some(guard) = guard else {
                launcher.update(cx, |launcher, cx| {
                    launcher.pending = None;
                    cx.notify();
                });
                return;
            };
            Some(guard)
        } else {
            None
        };
        let size = placement
            .as_ref()
            .map(|(layout, pane)| self.estimated_terminal_size(layout, pane, window, cx))
            .unwrap_or((INITIAL_COLS, INITIAL_ROWS));
        if key.is_none() {
            let mut draft = placement
                .as_ref()
                .map(|(layout, _)| layout.clone())
                .unwrap_or_else(|| PaneLayout::single(None, &[]));
            if draft.id.is_empty() {
                draft.id = runner_core::event_log::UlidGen::new()
                    .next()
                    .expect("new tab ULID");
            }
            draft.parent_id = scope.project_id().map(str::to_owned);
            let pane = placement
                .map(|(_, pane)| pane)
                .unwrap_or_else(|| draft.focused_pane_id.clone());
            let local_placement = (draft.clone(), pane.clone());
            let placement_client = client.clone();
            let placement_project = scope.project_id().map(str::to_owned);
            let input = crate::surfaces::front_page::actions::ShellStart {
                draft,
                pane: Some(pane),
                project: scope.project_id().map(str::to_owned),
                cwd,
                size,
            };
            use crate::surfaces::front_page::actions::WorkspaceAction;
            let Some((action, operation)) = WorkspaceAction::shell(self, input, cx) else {
                launcher.update(cx, |launcher, cx| {
                    launcher.pending = None;
                    cx.notify();
                });
                return;
            };
            let remote = action.client.is_remote();
            let task = cx.background_spawn(async move {
                let result = (|| {
                    let mut done = operation()?;
                    if !remote {
                        let (mut layout, pane) = local_placement;
                        layout
                            .assign_session(&pane, done.session.as_ref().unwrap())
                            .map_err(|error| error.to_string())?;
                        if let Err(error) =
                            placement_client.launcher_layout(&layout, placement_project)
                        {
                            let _ = placement_client.close(done.session.as_ref().unwrap());
                            return Err(error.to_string());
                        }
                        done.layout = Some(layout);
                    }
                    Ok(done)
                })();
                if remote {
                    if let (Some(guard), Ok(_)) = (tab_guard.as_mut(), &result) {
                        guard.await_host();
                    }
                }
                (tab_guard, result)
            });
            cx.spawn_in(window, async move |weak, cx| {
                let (tab_guard, result) = task.await;
                drop(tab_guard);
                let _ = weak.update_in(cx, |root, window, cx| {
                    cx.defer_in(window, |root, window, cx| {
                        root.drain_ready_launches(window, cx)
                    });
                    launcher.update(cx, |launcher, cx| {
                        if !remote || result.is_err() {
                            launcher.pending = None;
                        }
                        launcher.error = result.as_ref().err().cloned();
                        if let Ok(done) = &result {
                            launcher.launched_session = done.session.clone();
                        }
                        cx.notify();
                    });
                    let reveal_remote = remote
                        && result.is_ok()
                        && action.client.valid()
                        && root.front_page_client(cx).target == action.client.target;
                    let result = WorkspaceAction::complete(root, action, result, |done| done, cx);
                    if reveal_remote {
                        root.reveal_started_scope(&scope, cx);
                    }
                    if !remote {
                        if let Some(Ok(done)) = result {
                            if let Some(session) = done.session {
                                root.refresh_sessions(cx);
                                let result = root.reload_tabs(cx).and_then(|_| {
                                    root.tabs.activate_session(&session);
                                    root.sync_active_project_from_active_tab(cx);
                                    root.set_route(AppRoute::Chat, cx);
                                    root.ensure_active_tab_attached(window, cx)
                                });
                                if let Err(error) = result {
                                    launcher.update(cx, |launcher, cx| {
                                        launcher.error = Some(error.to_string());
                                        cx.notify();
                                    });
                                } else {
                                    if root.remote_client.is_local() {
                                        root.reveal_started_scope(&scope, cx);
                                    }
                                    root.launchers.retain(|_, cached| cached != &launcher);
                                    root.mark_active_tab_viewed(window, cx);
                                    root.sync_active_chat_detail(cx);
                                    root.begin_chat_transition(
                                        &session,
                                        chat_lifecycle::TransitionKind::Starting,
                                        Some(0),
                                        window,
                                        cx,
                                    );
                                }
                            }
                        }
                    }
                    cx.notify();
                });
            })
            .detach();
            return;
        }
        let key = key.unwrap();
        let mut launch = ChatLaunch {
            input: launcher_input(&key, scope.project_id().map(str::to_owned), cwd, size),
            placement,
            started: launcher
                .read(cx)
                .started
                .as_ref()
                .filter(|(entry, _)| entry == &key)
                .map(|(_, session)| session.clone()),
        };
        let remote = client
            .is_remote()
            .then(|| {
                self.remote_client
                    .prepare_chat_launch(client.clone(), launch.placement.as_ref())
            })
            .flatten();
        if client.is_remote() && remote.is_none() {
            launcher.update(cx, |launcher, cx| {
                launcher.pending = None;
                cx.notify();
            });
            return;
        }
        let directory = self.app_store.read(cx).app_data_dir.clone();
        let target = client.target.clone();
        let use_role_directory = client.is_remote()
            && !launcher.read(cx).changed_directory
            && client.supports(&["role"]);
        let recent_key = key.clone();
        let completion_target = client.target.clone();
        let remote_target = client.is_remote();
        let task = cx.background_spawn(async move {
            let result = (|| {
                if use_role_directory && launch.started.is_none() {
                    if let Some(role) = &launch.input.role {
                        if let Some(directory) = client.role_working_dir(role.clone())? {
                            launch.input.cwd = Some(directory);
                        }
                    }
                }
                client.launch_chat(&mut launch)
            })();
            if launch.started.is_some() {
                record_launcher_use(&directory, &target, recent_key);
            }
            if remote_target {
                if let (Some(guard), Ok(_)) = (tab_guard.as_mut(), &result) {
                    guard.await_host();
                }
            }
            (tab_guard, launch, result)
        });
        cx.spawn_in(window, async move |weak, cx| {
            let (tab_guard, launch, result) = task.await;
            drop(tab_guard);
            let _ = weak.update_in(cx, |root, window, cx| {
                cx.defer_in(window, |root, window, cx| {
                    root.drain_ready_launches(window, cx)
                });
                if let Some(gate) = result
                    .as_ref()
                    .err()
                    .and_then(|error| error.version_gate.clone())
                {
                    let pane = root.reserve_update_pane(
                        &launcher,
                        launch
                            .placement
                            .as_ref()
                            .map(|(layout, pane)| PaneKey::new(&layout.id, pane)),
                        &scope,
                        cx,
                    );
                    let retry = UpdateRetry {
                        runtime: gate.runtime,
                        gate: Some(gate),
                        action: UpdateAction::Chat {
                            key: Some(key.clone()),
                            pane,
                            scope: scope.clone(),
                            cwd: launch.input.cwd.clone(),
                        },
                        requested: false,
                        ready: false,
                    };
                    launcher.update(cx, |view, cx| {
                        view.update_retry = Some(retry);
                        view.pending = None;
                        cx.notify();
                    });
                }
                if let Some(action) = remote {
                    if !root.remote_client.settle_chat_launch(&action) {
                        return;
                    }
                    launcher.update(cx, |launcher, cx| {
                        if result.is_err() {
                            launcher.pending = None;
                        }
                        launcher.launched_session =
                            result.as_ref().ok().map(|output| output.session.clone());
                        launcher.started = launch.started.map(|session| (key.clone(), session));
                        launcher.error = result.as_ref().err().map(ToString::to_string);
                        cx.notify();
                    });
                    if let Ok(output) = result {
                        root.remote_client.complete_chat_launch(
                            action,
                            output.session,
                            output.layout,
                            cx,
                        );
                        root.sync_remote_state(cx);
                        root.reveal_started_scope(&scope, cx);
                    }
                } else {
                    launcher.update(cx, |launcher, cx| {
                        launcher.pending = None;
                        launcher.started =
                            launch.started.clone().map(|session| (key.clone(), session));
                        launcher.error = result.as_ref().err().map(ToString::to_string);
                        cx.notify();
                    });
                    if root.front_page_client(cx).target != completion_target {
                        root.refresh_sessions(cx);
                        let _ = root.reload_tabs(cx);
                        return;
                    }
                    if result.is_ok() {
                        let visible = root.front_page_is_chat()
                            && launcher.read(cx).pane.as_ref().map_or_else(
                                || root.launcher_page_visible(),
                                |pane| {
                                    root.front_page_layout().is_some_and(|layout| {
                                        layout.id == pane.tab_id
                                            && layout.focused_pane_id == pane.pane_id
                                    })
                                },
                            );
                        root.launchers.retain(|_, cached| cached != &launcher);
                        if background && !visible {
                            root.refresh_sessions(cx);
                            let _ = root.reload_tabs(cx);
                        } else {
                            root.finish_local_chat_launch(launch.started, result, window, cx);
                        }
                    }
                }
                cx.notify();
            });
        })
        .detach();
    }
}

#[cfg(test)]
impl NativeRoot {
    pub(crate) fn test_open_empty_tab(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let mut layout = PaneLayout::single(None, &[]);
        layout.id = runner_core::event_log::UlidGen::new().next().unwrap();
        self.front_page_client(cx)
            .launcher_layout(&layout, None)
            .unwrap();
        self.reload_tabs(cx).unwrap();
        self.tabs.activate(&layout.id);
        self.set_route(AppRoute::Chat, cx);
        self.render_launcher(
            Some(PaneKey::new(&layout.id, &layout.focused_pane_id)),
            true,
            window,
            cx,
        );
    }

    /// Whether a mission entry point reached the crew or goal step.
    pub(crate) fn mission_launcher_open(&self, cx: &App) -> bool {
        self.launchers
            .values()
            .any(|launcher| launcher.read(cx).step != Step::Start)
    }

    pub(crate) fn launcher_search_focused(&self, window: &Window, cx: &App) -> bool {
        self.launchers.values().any(|launcher| {
            launcher
                .read(cx)
                .field()
                .read(cx)
                .focus_handle()
                .is_focused(window)
        })
    }

    pub(crate) fn open_pane_chat_modal(
        &mut self,
        pane_id: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.focus_pane(pane_id, cx);
        let layout = self.front_page_layout().unwrap();
        let pane = PaneKey::new(&layout.id, pane_id);
        self.render_launcher(Some(pane), false, window, cx);
    }
    pub(crate) fn test_start_launcher(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let layout = self.front_page_layout().unwrap();
        let pane = PaneKey::new(&layout.id, &layout.focused_pane_id);
        let launcher =
            self.render_launcher(Some(pane), layout.root.leaves().len() == 1, window, cx);
        let (key, scope, cwd) = launcher.update(cx, |launcher, cx| {
            let (rows, _) = launcher.rows(cx);
            let row = &rows[0];
            let Choice::Chat(key) = &row.choice else {
                panic!("expected a chat row");
            };
            launcher.pending = Some(row.choice.clone());
            (
                key.clone(),
                launcher.project.scope(),
                normalized_value(launcher.cwd.read(cx).text()),
            )
        });
        let pane = launcher.read(cx).pane.clone();
        self.launch_from_launcher(launcher, Some(key), pane, scope, cwd, false, window, cx);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::surfaces::start_chat::tests::{modal_harness, project_fixture, ModalHarness};
    use runner_core::protocol::model::Runtime;

    fn agent() -> RuntimeChoice {
        RuntimeChoice {
            installed_version: None,
            available_version: None,
            min_version: None,
            below_minimum: false,
            name: Runtime::Codex,
            display_name: "Codex".into(),
            command: Some("codex".into()),
            available: true,
            default_model: Some("illustrative-default".into()),
            default_effort: Some("high".into()),
            models: Vec::new(),
            efforts: Vec::new(),
            speed: true,
            effort_needs_launch_model: false,
        }
    }
    fn role() -> RoleChoice {
        RoleChoice {
            id: "role".into(),
            handle: "coder".into(),
            display_name: "Bounded role".into(),
            runtime: "codex".into(),
            model: Some("model".into()),
            effort: Some("low".into()),
            codex_speed: None,
            working_dir: Some("/role-directory".into()),
        }
    }

    struct UpdateFixture {
        inner: runner_daemon::daemon::InProcessTransport,
        starts: std::sync::Arc<std::sync::atomic::AtomicUsize>,
    }
    impl runner_core::protocol::Transport for UpdateFixture {
        fn call(
            &self,
            request: runner_core::protocol::Request,
        ) -> runner_core::protocol::ClientResult<runner_core::protocol::Response> {
            if matches!(
                request,
                runner_core::protocol::Request::runtime_task_start { .. }
            ) {
                self.starts
                    .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Ok(runner_core::protocol::Response::runtime_task_start(Err(
                    runner_core::protocol::ClientError::msg("scratch updater is offline"),
                )));
            }
            if let runner_core::protocol::Request::session_start_runtime_with_speed {
                cwd, ..
            } = &request
            {
                let conn = self.inner.0.db.get().unwrap();
                let count: i64 = conn
                    .query_row(
                        "SELECT count(*) FROM sessions WHERE id LIKE 'updated-chat%'",
                        [],
                        |row| row.get(0),
                    )
                    .unwrap();
                let id = if count == 0 {
                    "updated-chat".to_owned()
                } else {
                    format!("updated-chat-{count}")
                };
                conn.execute("INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES (?1, 'stopped', 'codex', 'codex')", [&id]).unwrap();
                return Ok(
                    runner_core::protocol::Response::session_start_runtime_with_speed(Ok(
                        runner_core::protocol::session::StartDirectSessionOutput {
                            session: runner_core::protocol::session::SpawnedSession {
                                id,
                                mission_id: None,
                                role_id: None,
                                handle: "codex".into(),
                                pid: None,
                            },
                            project_id: None,
                            cwd: cwd.clone(),
                        },
                    )),
                );
            }
            self.inner.call(request)
        }
        fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
            self.inner.subscribe()
        }
    }

    #[test]
    fn required_updates_mark_only_agents_and_defer_agent_and_role_launch_intents() {
        for key in [
            EntryKey::Agent("codex".into()),
            EntryKey::Role("role".into()),
        ] {
            let mut catalog = RuntimeCatalogEntry::for_runtime(Runtime::Codex).unwrap();
            catalog.available = true;
            let mut harness =
                modal_harness(1200., 900., Vec::new(), vec![catalog], ChatMode::Runtime);
            let starts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let mut page = None;
            harness.act(|root, window, cx| {
                root.start_chat_modal = None;
                root.app_store.update(cx, |store, _| {
                    store.client = runner_core::protocol::DaemonClient::new(std::sync::Arc::new(
                        UpdateFixture {
                            inner: runner_daemon::daemon::InProcessTransport(
                                store.test_core.clone(),
                            ),
                            starts: starts.clone(),
                        },
                    ));
                });
                root.open_launcher_page(ProjectScope::Root, Step::Start, false, window, cx);
                let launcher = root.render_launcher(None, true, window, cx);
                launcher.update(cx, |view, cx| {
                    let mut runtime = agent();
                    runtime.installed_version = Some("0.147.0".into());
                    runtime.min_version = Some("0.149.0".into());
                    runtime.available_version = Some("0.167.0".into());
                    runtime.below_minimum = true;
                    view.runtimes = vec![runtime];
                    view.roles = vec![role()];
                    cx.notify();
                });
                page = Some(launcher);
            });
            assert!(harness.bounds("LAUNCHER_UPDATE_NEEDED:Codex").is_some());
            assert!(harness
                .bounds("LAUNCHER_UPDATE_NEEDED:Bounded role")
                .is_none());
            assert!(harness.bounds("LAUNCHER_VERSION_NOTICE").is_some());
            let launcher = page.unwrap();
            harness.act(|root, window, cx| {
                launcher.update(cx, |view, _| view.pending = Some(Choice::Chat(key.clone())));
                root.launch_from_launcher(launcher.clone(), Some(key.clone()), None, ProjectScope::Root, None, false, window, cx);
                assert!(!root.launcher_start_pending(cx));
                assert!(launcher.read(cx).pane.is_some());
                assert!(root.agent_update.is_none());
                let retry = launcher.read(cx).update_retry.as_ref().unwrap();
                assert_eq!(retry.runtime, Runtime::Codex);
                assert!(retry.requested);
                assert!(matches!(&retry.action, UpdateAction::Chat { key: Some(saved), .. } if saved == &key));
                assert_eq!(retry.gate.as_ref().unwrap().min_version, "0.149.0");
                assert!(root.core(cx).session_list_recent_direct().unwrap().is_empty());
            });
            assert_eq!(starts.load(std::sync::atomic::Ordering::Relaxed), 1);
        }
    }

    #[test]
    fn a_gated_pane_launches_after_success_without_replacing_new_work_or_focus() {
        use runner_core::protocol::{RuntimeUpdateState, RuntimeUpdateTask};
        let mut harness = modal_harness(1200., 900., Vec::new(), Vec::new(), ChatMode::Runtime);
        let mut waiting = String::new();
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.app_store.update(cx, |store, _| {
                store.client =
                    runner_core::protocol::DaemonClient::new(std::sync::Arc::new(UpdateFixture {
                        inner: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                        starts: Default::default(),
                    }));
            });
            root.reset_launcher_page(cx);
            let launcher = root.render_launcher(None, true, window, cx);
            launcher.update(cx, |view, _| {
                let mut runtime = agent();
                runtime.below_minimum = true;
                view.runtimes = vec![runtime];
                view.pending = Some(Choice::Chat(EntryKey::Agent("codex".into())));
            });
            root.launch_from_launcher(
                launcher.clone(),
                Some(EntryKey::Agent("codex".into())),
                None,
                ProjectScope::Root,
                None,
                false,
                window,
                cx,
            );
            waiting = launcher.read(cx).pane.as_ref().unwrap().tab_id.clone();
            assert!(!root.launcher_start_pending(cx));
            root.open_new_tab_modal(&NewTab, window, cx);
            assert!(launcher.read(cx).update_retry.is_some());
        });
        let mut focus = None;
        harness.act(|root, window, cx| {
            focus = window.focused(cx);
            assert!(root.launcher_page_visible());
            root.apply_agent_tasks(
                vec![RuntimeUpdateTask {
                    id: "successful-update".into(),
                    runtime: Runtime::Codex,
                    state: RuntimeUpdateState::Succeeded,
                    from_version: Some("0.147.0".into()),
                    to_version: Some("0.167.0".into()),
                    installed_version: Some("0.167.0".into()),
                    step: "Done".into(),
                    message: None,
                    exit_code: Some(0),
                    running_sessions: 0,
                }],
                window,
                cx,
            );
        });
        harness.act(|root, window, cx| {
            assert!(root.launcher_page_visible());
            assert_eq!(window.focused(cx), focus);
            let tab = root
                .tabs
                .tabs()
                .iter()
                .find(|tab| tab.id == waiting)
                .unwrap();
            assert!(tab.contains_session("updated-chat"));
            assert!(root.agent_update.is_none());
        });
    }

    #[test]
    fn shared_update_queues_both_panes_behind_the_tabs_active_start() {
        use runner_core::protocol::{RuntimeUpdateState, RuntimeUpdateTask};
        let mut harness = modal_harness(1200., 900., Vec::new(), Vec::new(), ChatMode::Runtime);
        let mut waiting_tab = String::new();
        let mut guard = None;
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.app_store.update(cx, |store, _| {
                store.client =
                    runner_core::protocol::DaemonClient::new(std::sync::Arc::new(UpdateFixture {
                        inner: runner_daemon::daemon::InProcessTransport(store.test_core.clone()),
                        starts: Default::default(),
                    }))
            });
            root.reset_launcher_page(cx);
            root.test_open_empty_tab(window, cx);
            let mut layout = root.tabs.active().unwrap().clone();
            layout.split("p1", SplitOrientation::Row).unwrap();
            root.front_page_client(cx)
                .launcher_layout(&layout, None)
                .unwrap();
            root.reload_tabs(cx).unwrap();
            root.tabs.activate(&layout.id);
            waiting_tab = layout.id.clone();
            for leaf in layout.root.leaves() {
                let pane = PaneKey::new(&layout.id, &leaf.id);
                let launcher = root.render_launcher(Some(pane.clone()), false, window, cx);
                launcher.update(cx, |view, _| {
                    let mut runtime = agent();
                    runtime.below_minimum = true;
                    view.runtimes = vec![runtime];
                });
                root.launch_from_launcher(
                    launcher,
                    Some(EntryKey::Agent("codex".into())),
                    Some(pane),
                    ProjectScope::Root,
                    None,
                    false,
                    window,
                    cx,
                );
            }
            guard = TabStart::begin(
                root.app_store.read(cx).launcher_tabs_pending.clone(),
                format!("local:{}", layout.id),
                layout,
            );
            root.apply_agent_tasks(
                vec![RuntimeUpdateTask {
                    id: "one-update".into(),
                    runtime: Runtime::Codex,
                    state: RuntimeUpdateState::Succeeded,
                    from_version: None,
                    to_version: None,
                    installed_version: Some("0.167.0".into()),
                    step: "Done".into(),
                    message: None,
                    exit_code: Some(0),
                    running_sessions: 0,
                }],
                window,
                cx,
            );
            assert!(root
                .core(cx)
                .session_list_recent_direct()
                .unwrap()
                .is_empty());
            assert_eq!(
                root.launchers
                    .values()
                    .filter(|view| view
                        .read(cx)
                        .update_retry
                        .as_ref()
                        .is_some_and(|retry| retry.ready))
                    .count(),
                2
            );
        });
        drop(guard);
        harness.act(|root, window, cx| root.drain_ready_launches(window, cx));
        harness.act(|root, _, cx| {
            let tab = root
                .tabs
                .tabs()
                .iter()
                .find(|tab| tab.id == waiting_tab)
                .unwrap();
            assert_eq!(tab.session_ids().len(), 2);
            assert!(tab.contains_session("updated-chat"));
            assert!(tab.contains_session("updated-chat-1"));
            assert_eq!(root.core(cx).session_list_recent_direct().unwrap().len(), 2);
        });
    }

    struct DeferredOrigin {
        base: std::sync::Arc<crate::surfaces::remote_client::tests::Fake>,
        core: runner_daemon::AppCore,
        calls: std::sync::atomic::AtomicUsize,
    }
    impl runner_core::protocol::Transport for DeferredOrigin {
        fn call(
            &self,
            request: runner_core::protocol::Request,
        ) -> runner_core::protocol::ClientResult<runner_core::protocol::Response> {
            use runner_core::protocol::{Request, Response};
            use std::sync::atomic::Ordering;
            if matches!(request, Request::runtime_task_start { .. }) {
                let mut task: runner_core::protocol::RuntimeUpdateTask = serde_json::from_value(
                    crate::surfaces::remote_client::tests::completed_update(),
                )
                .unwrap();
                task.state = runner_core::protocol::RuntimeUpdateState::Running;
                self.base.emit(
                    "runtime/task-changed",
                    serde_json::to_value(vec![task.clone()]).unwrap(),
                );
                return Ok(Response::runtime_task_start(Ok(task)));
            }
            if matches!(
                request,
                Request::session_fork { .. } | Request::mission_start_impl_with_size { .. }
            ) {
                let attempt = self.calls.fetch_add(1, Ordering::Relaxed);
                if attempt == 0 {
                    let error = crate::surfaces::remote_client::tests::update_gate_error();
                    return Ok(match request {
                        Request::session_fork { .. } => Response::session_fork(Err(error)),
                        _ => Response::mission_start_impl_with_size(Err(error)),
                    });
                }
                if matches!(request, Request::session_fork { .. }) {
                    let conn = self.core.db.get().unwrap();
                    conn.execute("INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES ('forked-origin', 'stopped', 'codex', 'codex')", []).unwrap();
                    let layout =
                        PaneLayout::single(Some("forked-origin"), &["forked-origin".into()]);
                    runner_daemon::repo::node::create_tab(
                        &conn,
                        None,
                        "Fork result",
                        1,
                        &layout.serialize().unwrap(),
                    )
                    .unwrap();
                    return Ok(Response::session_fork(Ok(
                        runner_core::protocol::session::SpawnedSession {
                            id: "forked-origin".into(),
                            mission_id: None,
                            role_id: None,
                            handle: "codex".into(),
                            pid: None,
                        },
                    )));
                }
            }
            self.base.call(request)
        }
        fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
            self.base.subscribe()
        }
    }

    #[test]
    fn deferred_mission_and_fork_keep_the_saved_host_and_later_work_focused() {
        use crate::surfaces::remote_client::tests as remote_test;
        use std::sync::atomic::Ordering;
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for kind in ["mission", "fork"] {
            for switch_host in [false, true] {
                let temp = tempfile::tempdir().unwrap();
                let mut cx = gpui::TestAppContext::single();
                let (store, fake, _) = remote_test::fixture(&mut cx, temp.path());
                remote_test::enable_front_page(&fake);
                let core = store.read_with(&cx, |store, _| store.test_core.clone());
                let trace = std::sync::Arc::new(DeferredOrigin {
                    base: fake.clone(),
                    core: core.clone(),
                    calls: Default::default(),
                });
                store.update(&mut cx, |store, _| {
                    store.client = runner_core::protocol::DaemonClient::new(trace.clone())
                });
                let window = remote_test::add_window(&mut cx, &store, temp.path(), kind);
                window.update(&mut cx, |root, window, cx| {
                    if kind == "mission" {
                        root.open_launcher_page(ProjectScope::Root, Step::Goal(None), false, window, cx);
                        let launcher = root.render_launcher(None, true, window, cx);
                        launcher.update(cx, |view, cx| view.start_mission(window, cx));
                    } else {
                        core.db.get().unwrap().execute("INSERT INTO sessions(id, status, agent_runtime, agent_command, agent_session_key) VALUES ('origin', 'stopped', 'codex', 'codex', 'key')", []).unwrap();
                        let mut layout = PaneLayout::single(Some("origin"), &["origin".into()]);
                        layout.id = runner_core::event_log::UlidGen::new().next().unwrap();
                        root.front_page_client(cx).launcher_layout(&layout, None).unwrap();
                        root.refresh_sessions(cx);
                        root.reload_tabs(cx).unwrap();
                        root.tabs.activate_session("origin");
                        root.set_route(AppRoute::Chat, cx);
                        root.fork_chat("origin", window, cx);
                        root.confirm_fork_chat(window, cx);
                    }
                }).unwrap();
                cx.run_until_parked();
                assert_eq!(
                    trace.calls.load(Ordering::Relaxed),
                    1,
                    "{kind}: original request reaches the local gate"
                );
                let mut visual = gpui::VisualTestContext::from_window(window.into(), &cx);
                visual.run_until_parked();
                let update = visual
                    .debug_bounds("LAUNCHER_UPDATE_ACTION")
                    .expect("refused origin has recovery action");
                visual.simulate_click(update.center(), Default::default());
                visual.run_until_parked();
                window
                    .update(&mut visual, |root, window, cx| {
                        if switch_host {
                            remote_test::choose_host(root, window, cx);
                        } else {
                            root.open_new_tab_modal(&NewTab, window, cx);
                        }
                    })
                    .unwrap();
                visual.run_until_parked();
                if !switch_host {
                    let (visible, waiting, old_query, old_goal) = window
                        .read_with(&visual, |root, cx| {
                            let visible = root.page_launcher(cx).unwrap().clone();
                            let waiting = root
                                .launchers
                                .iter()
                                .find(|(key, _)| key.starts_with("update-wait:"))
                                .unwrap()
                                .1
                                .clone();
                            let old_query = waiting.read(cx).query.read(cx).text().to_owned();
                            let old_goal = waiting.read(cx).goal.read(cx).text().to_owned();
                            (visible, waiting, old_query, old_goal)
                        })
                        .unwrap();
                    window
                        .update(&mut visual, |root, window, cx| {
                            root.open_this_computer_settings(Some("agents"), window, cx)
                        })
                        .unwrap();
                    visual.run_until_parked();
                    window
                        .update(&mut visual, |root, window, cx| {
                            root.leave_settings(window, cx)
                        })
                        .unwrap();
                    visual.run_until_parked();
                    window
                        .update(&mut visual, |root, window, cx| {
                            assert_eq!(root.page_launcher(cx), Some(&visible));
                            assert!(visible
                                .read(cx)
                                .query
                                .read(cx)
                                .focus_handle()
                                .is_focused(window));
                        })
                        .unwrap();
                    visual.simulate_keystrokes("v i s i b l e");
                    visual.run_until_parked();
                    window
                        .read_with(&visual, |_, cx| {
                            assert_eq!(visible.read(cx).query.read(cx).text(), "visible");
                            assert_eq!(waiting.read(cx).query.read(cx).text(), old_query);
                            assert_eq!(waiting.read(cx).goal.read(cx).text(), old_goal);
                        })
                        .unwrap();
                }
                let mut before = None;
                window
                    .update(&mut visual, |root, window, cx| {
                        before = Some((
                            root.front_page_client(cx).target,
                            root.route.clone(),
                            window.focused(cx),
                        ));
                        let task = serde_json::from_value(remote_test::completed_update()).unwrap();
                        root.apply_agent_tasks(vec![task], window, cx);
                    })
                    .unwrap();
                visual.run_until_parked();
                assert_eq!(
                    trace.calls.load(Ordering::Relaxed),
                    2,
                    "{kind} retried exactly once"
                );
                assert_eq!(
                    fake.request_count(if kind == "mission" {
                        "startMission"
                    } else {
                        "fork"
                    }),
                    0,
                    "a local request must never be sent to the selected host"
                );
                window
                    .read_with(&visual, |root, cx| {
                        let (target, route, _) = before.as_ref().unwrap();
                        assert_eq!(&root.front_page_client(cx).target, target);
                        assert_eq!(&root.route, route);
                    })
                    .unwrap();
                window
                    .update(&mut visual, |_, window, cx| {
                        assert_eq!(window.focused(cx), before.as_ref().unwrap().2)
                    })
                    .unwrap();
                if kind == "mission" {
                    assert_eq!(
                        runner_daemon::ops::mission::mission_list(&core, None)
                            .unwrap()
                            .len(),
                        1
                    );
                } else {
                    assert!(runner_daemon::repo::session::get_row(
                        &core.db.get().unwrap(),
                        "forked-origin"
                    )
                    .unwrap()
                    .is_some());
                }
            }
        }
    }

    #[test]
    fn optional_updates_only_tag_the_agent_row_and_keep_the_launcher_available() {
        let mut catalog = RuntimeCatalogEntry::for_runtime(Runtime::Codex).unwrap();
        catalog.available = true;
        let mut harness = modal_harness(1200., 900., Vec::new(), vec![catalog], ChatMode::Runtime);
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.open_launcher_page(ProjectScope::Root, Step::Start, false, window, cx);
            let launcher = root.render_launcher(None, true, window, cx);
            launcher.update(cx, |view, cx| {
                let mut runtime = agent();
                runtime.installed_version = Some("0.166.0".into());
                runtime.available_version = Some("0.167.0".into());
                view.runtimes = vec![runtime];
                view.roles = vec![role()];
                assert!(view.update_retry.is_none());
                let (rows, _) = view.rows(cx);
                assert!(rows
                    .iter()
                    .filter(|row| matches!(row.choice, Choice::Chat(_)))
                    .all(|row| !row.unavailable));
                cx.notify();
            });
        });
        assert!(harness.bounds("LAUNCHER_OPTIONAL_UPDATE:codex").is_some());
        assert!(harness
            .bounds("LAUNCHER_UPDATE_NEEDED:Bounded role")
            .is_none());
        assert!(harness.bounds("LAUNCHER_VERSION_NOTICE").is_none());
        harness.read_root(|root, _| assert!(root.agent_update.is_none()));
    }

    #[test]
    fn requests_have_no_form_overrides_and_directory_change_wins_over_role_directory() {
        for key in [
            EntryKey::Agent("codex".into()),
            EntryKey::Role("role".into()),
        ] {
            let request = launcher_input(
                &key,
                Some("project".into()),
                Some("/shown".into()),
                (90, 30),
            );
            assert_eq!(request.project.as_deref(), Some("project"));
            assert_eq!(request.cwd.as_deref(), Some("/shown"));
            assert!(request.model.is_none() && request.effort.is_none() && request.speed.is_none());
            assert_eq!(request.runtime.is_some(), matches!(key, EntryKey::Agent(_)));
            assert_eq!(request.role.is_some(), matches!(key, EntryKey::Role(_)));
        }
        let mut role = role();
        assert_eq!(
            launcher_directory(Some(&role), Some("/shown".into()), false).as_deref(),
            Some("/role-directory")
        );
        assert_eq!(
            launcher_directory(Some(&role), Some("/changed".into()), true).as_deref(),
            Some("/changed")
        );
        role.working_dir = None;
        assert_eq!(
            launcher_directory(Some(&role), Some("/shown".into()), false).as_deref(),
            Some("/shown")
        );
    }

    #[test]
    fn recency_identity_ignores_connection_generation_but_separates_computers() {
        let remote = |computer: &str, generation| FrontPageTarget::Remote {
            computer: computer.into(),
            fingerprint: "fingerprint".into(),
            generation,
        };
        assert_eq!(computer_key(&remote("a", 1)), computer_key(&remote("a", 2)));
        assert_ne!(computer_key(&remote("a", 1)), computer_key(&remote("b", 1)));
        assert_ne!(
            computer_key(&FrontPageTarget::Local),
            computer_key(&remote("a", 1))
        );
    }

    fn row_scope(harness: &ModalHarness) -> Option<ProjectScope> {
        harness.read_root(|root, cx| root.front_page_sidebar().read(cx).launcher_row_scope(cx))
    }

    fn assert_row_under(harness: &mut ModalHarness, project: &str) {
        assert_eq!(
            row_scope(harness),
            Some(ProjectScope::Project(project.to_owned()))
        );
        let row = harness
            .bounds("SIDEBAR_LAUNCHER_TAB")
            .expect("launcher row");
        let header = harness
            .bounds(&format!("SIDEBAR_PROJECT:{project}"))
            .expect("project header");
        assert!(row.origin.y > header.origin.y);
        assert!(row.origin.y < header.origin.y + px(48.));
    }

    fn assert_row_in_recents(harness: &mut ModalHarness) {
        assert_eq!(row_scope(harness), Some(ProjectScope::Root));
        let row = harness
            .bounds("SIDEBAR_LAUNCHER_TAB")
            .expect("launcher row");
        let header = harness
            .bounds("SIDEBAR_SECTION:RECENTS")
            .expect("Recents header");
        assert!(row.origin.y > header.origin.y);
    }

    /// The project the page's picker shows, and whether it is locked there.
    fn page_picker(harness: &mut ModalHarness) -> (ProjectScope, bool) {
        let mut picker = None;
        harness.act(|root, window, cx| {
            let launcher = root.render_launcher(None, true, window, cx);
            let launcher = launcher.read(cx);
            let select = launcher.project.select();
            let select = select.read(cx);
            assert_eq!(
                select.value(),
                launcher.scope().project_id().unwrap_or_default()
            );
            picker = Some((launcher.scope(), select.is_disabled()));
        });
        picker.unwrap()
    }

    fn page_directory(harness: &mut ModalHarness) -> String {
        let mut directory = String::new();
        harness.act(|root, window, cx| {
            let launcher = root.render_launcher(None, true, window, cx);
            directory = launcher.read(cx).cwd.read(cx).text().to_owned();
        });
        directory
    }

    fn open_tab_in(harness: &mut ModalHarness, project: Option<&str>) -> String {
        let project = project.map(str::to_owned);
        let mut tab = String::new();
        harness.act(|root, _, cx| {
            root.start_chat_modal = None;
            let mut layout = PaneLayout::single(None, &[]);
            layout.id = runner_core::event_log::UlidGen::new().next().unwrap();
            root.front_page_client(cx)
                .launcher_layout(&layout, project.clone())
                .unwrap();
            root.reload_tabs(cx).unwrap();
            root.tabs.activate(&layout.id);
            root.set_route(AppRoute::Chat, cx);
            root.sync_active_project_from_active_tab(cx);
            assert_eq!(root.active_project_id(cx), project);
            tab = layout.id;
        });
        tab
    }

    fn open_plus(harness: &mut ModalHarness, scope: ProjectScope) {
        harness.act(move |root, window, cx| {
            root.front_page_sidebar().update(cx, |sidebar, cx| {
                sidebar.open_launcher_in(scope, window, cx);
            });
        });
    }

    /// Saved chats or terminals, each in a tab of its own, the last one viewed.
    fn saved_tabs(harness: &mut ModalHarness, sessions: &[(&str, &str)]) -> Vec<String> {
        let mut tabs = Vec::new();
        harness.act(|root, _, cx| {
            root.start_chat_modal = None;
            let core = root.app_store.read(cx).test_core.clone();
            let conn = core.db.get().unwrap();
            for (position, (session, runtime)) in sessions.iter().enumerate() {
                conn.execute(
                    "INSERT INTO sessions(id, status, agent_runtime, agent_command) VALUES (?1, 'stopped', ?2, ?2)",
                    [session, runtime],
                )
                .unwrap();
                let layout = PaneLayout::single(Some(session), &[(*session).into()]);
                tabs.push(
                    runner_daemon::repo::node::create_tab(
                        &conn,
                        None,
                        session,
                        position as i64,
                        &layout.serialize().unwrap(),
                    )
                    .unwrap()
                    .id,
                );
            }
            drop(conn);
            root.app_store.update(cx, |store, cx| {
                store.refresh_sessions(cx);
                store.refresh_nodes(cx).unwrap();
            });
            root.reload_tabs(cx).unwrap();
            root.tabs.activate(tabs.last().unwrap());
            root.set_route(AppRoute::Chat, cx);
        });
        tabs
    }

    /// Another window with `session` on screen.
    fn open_in_second_window(harness: &mut ModalHarness, session: &str) {
        let session = session.to_owned();
        harness.act(move |root, _, cx| {
            let core = root.core(cx);
            core.window_register("second").unwrap();
            core.report_subjects(
                "second",
                vec![runner_core::protocol::window::Subject::DirectChat(session)],
                None,
            )
            .unwrap();
        });
    }

    fn assert_landed_on(harness: &mut ModalHarness, tab: &str, session: &str) {
        let (tab, session) = (tab.to_owned(), session.to_owned());
        harness.act(move |root, window, _| {
            assert_eq!(root.route, AppRoute::Chat);
            assert_eq!(root.tabs.active_tab_id(), Some(tab.as_str()));
            assert!(!root.launcher_page_visible());
            assert!(root.chat_focus.is_focused(window));
            assert_eq!(
                root.runtime_navigation_history.last(),
                Some(&RuntimeLocation::Chat(session))
            );
        });
    }

    #[test]
    fn new_chat_entries_show_a_page_without_saving_a_tab() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let projects = project_fixture(&mut harness);
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.open_new_tab_modal(&NewTab, window, cx);
            root.open_new_tab_modal(&NewTab, window, cx);
            assert_eq!(root.route, AppRoute::Launcher);
            assert!(root.tabs.tabs().is_empty());
            assert!(root
                .core(cx)
                .node_list()
                .unwrap()
                .iter()
                .all(|node| node.node_type != runner_core::protocol::node::NodeType::Tab));
            root.open_sidebar_chat_modal(ProjectScope::Project(projects[0].id.clone()), window, cx);
            let launcher = root.render_launcher(None, true, window, cx);
            assert_eq!(
                launcher.read(cx).scope(),
                ProjectScope::Project(projects[0].id.clone())
            );
            assert_eq!(launcher.read(cx).cwd.read(cx).text(), projects[0].cwd);
            assert!(root.tabs.tabs().is_empty());
        });
    }

    fn update_settings(
        harness: &mut ModalHarness,
        update: impl FnOnce(&mut AppSettings) -> bool + 'static,
    ) {
        harness.act(move |root, _, cx| {
            root.app_store
                .update(cx, |store, cx| store.update_settings(update, false, cx));
        });
    }

    #[test]
    fn launcher_page_row_stays_where_the_page_opened_and_plus_reveals_it() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let projects = project_fixture(&mut harness);
        let (alpha, beta) = (projects[0].id.clone(), projects[1].id.clone());
        let (alpha_path, beta_path) = (projects[0].cwd.clone(), projects[1].cwd.clone());
        harness.act(|root, _, cx| {
            root.start_chat_modal = None;
            let nodes = root.core(cx).node_list().unwrap();
            root.app_store.update(cx, |store, cx| {
                store.nodes = nodes;
                cx.notify();
            });
        });
        let collapsed = beta.clone();
        update_settings(&mut harness, move |settings| {
            settings.sidebar_collapsed_projects.insert(collapsed)
        });

        // Opening the page under a collapsed project leaves it collapsed, so
        // there is no row and New tab keeps the selection.
        let scope = ProjectScope::Project(beta.clone());
        harness.act(move |root, window, cx| root.open_sidebar_chat_modal(scope, window, cx));
        assert_eq!(row_scope(&harness), None);
        assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_none());

        // + on a collapsed project expands it, so the row is visible.
        open_plus(&mut harness, ProjectScope::Project(beta.clone()));
        assert_row_under(&mut harness, &beta);

        // + on another project moves the row while the page already shows.
        open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
        assert_row_under(&mut harness, &alpha);

        // Neither the locked picker nor a folder in another project moves it.
        let (chosen, folder) = (beta.clone(), beta_path.clone());
        harness.act(move |root, window, cx| {
            let launcher = root.render_launcher(None, true, window, cx);
            launcher.update(cx, |launcher, cx| {
                launcher.choose_project(&chosen, cx);
                launcher.project.picked(&folder, cx);
            });
        });
        assert_row_under(&mut harness, &alpha);
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::Project(alpha.clone()), true)
        );

        // ⌘N puts it in Recents, where the free picker and folder don't move it.
        harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
        assert_row_in_recents(&mut harness);
        let chosen = beta.clone();
        harness.act(move |root, window, cx| {
            let launcher = root.render_launcher(None, true, window, cx);
            launcher.update(cx, |launcher, cx| {
                let picker = launcher.project.select();
                picker.update(cx, |select, cx| select.set_value(chosen.clone(), cx));
                launcher.choose_project(&chosen, cx);
            });
        });
        assert_row_in_recents(&mut harness);
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::Project(beta.clone()), false)
        );
        harness.act(move |root, window, cx| {
            let launcher = root.render_launcher(None, true, window, cx);
            launcher.update(cx, |launcher, cx| launcher.project.picked(&alpha_path, cx));
        });
        assert_row_in_recents(&mut harness);
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::Project(alpha.clone()), false)
        );

        // + on Recents is the same, from a page locked to a project too.
        open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
        open_plus(&mut harness, ProjectScope::Root);
        assert_row_in_recents(&mut harness);
        assert_eq!(page_picker(&mut harness), (ProjectScope::Root, false));

        // With nowhere to draw it, New tab keeps the selection.
        update_settings(&mut harness, |settings| {
            settings.sidebar_active_only = true;
            true
        });
        assert_eq!(row_scope(&harness), None);
        assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_none());
        update_settings(&mut harness, |settings| {
            settings.sidebar_active_only = false;
            true
        });

        // Leaving the page removes it.
        harness.act(|root, window, cx| root.open_roles(window, cx));
        assert_eq!(row_scope(&harness), None);
        assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_none());
    }

    #[test]
    fn new_tab_entries_use_the_recent_tabs_project_and_mission_entries_keep_the_active_one() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let projects = project_fixture(&mut harness);
        let alpha = projects[0].id.clone();
        for entry in [
            "shortcut",
            "sidebar row",
            "recents plus",
            "shortcut on a locked page",
        ] {
            open_tab_in(&mut harness, Some(&alpha));
            let default_directory =
                harness.read_root(|root, cx| local_default_directory(root.settings(cx)).unwrap());
            match entry {
                "shortcut" => {
                    harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
                }
                "sidebar row" => {
                    let row = harness.bounds("SIDEBAR_WORKSPACE_NEW_TAB").unwrap();
                    harness
                        .visual
                        .simulate_click(row.center(), gpui::Modifiers::default());
                    harness.visual.run_until_parked();
                }
                "recents plus" => open_plus(&mut harness, ProjectScope::Root),
                _ => {
                    open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
                    assert_row_under(&mut harness, &alpha);
                    assert_eq!(
                        page_picker(&mut harness),
                        (ProjectScope::Project(alpha.clone()), true)
                    );
                    assert_eq!(page_directory(&mut harness), projects[0].cwd);
                    harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
                }
            }
            harness.read_root(|root, _| assert_eq!(root.route, AppRoute::Launcher, "{entry}"));
            assert_row_in_recents(&mut harness);
            let project = (entry != "recents plus").then(|| alpha.clone());
            assert_eq!(
                page_picker(&mut harness),
                (ProjectScope::or_root(project.clone()), false),
                "{entry}"
            );
            assert_eq!(
                page_directory(&mut harness),
                if project.is_some() {
                    projects[0].cwd.clone()
                } else {
                    default_directory
                },
                "{entry}"
            );
            harness.read_root(|root, cx| assert_eq!(root.active_project_id(cx), None));
        }

        // ⇧⌘M keeps the project the tab is in, still editable, with the row in
        // Recents.
        open_tab_in(&mut harness, Some(&alpha));
        harness.act(|root, window, cx| root.new_mission_action(&NewMission, window, cx));
        harness.read_root(|root, _| assert_eq!(root.route, AppRoute::Launcher));
        assert_row_in_recents(&mut harness);
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::Project(alpha.clone()), false)
        );

        // So do it and a crew start from a page a project's + locked, keeping
        // the page's folder and where it closes back to.
        for crew in [None, Some("crew")] {
            let tab = open_tab_in(&mut harness, Some(&alpha));
            open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
            assert_row_under(&mut harness, &alpha);
            let folder = tempfile::tempdir().unwrap();
            let folder = folder.path().to_string_lossy().into_owned();
            let changed = folder.clone();
            harness.act(move |root, window, cx| {
                let launcher = root.render_launcher(None, true, window, cx);
                launcher.update(cx, |launcher, cx| {
                    launcher.project.picked(&changed, cx);
                    launcher
                        .cwd
                        .update(cx, |cwd, cx| cwd.reset(changed.clone(), cx));
                });
            });
            let scope = ProjectScope::Project(alpha.clone());
            harness.act(move |root, window, cx| {
                root.open_mission_launcher(crew.map(str::to_owned), scope, window, cx);
            });
            assert_row_in_recents(&mut harness);
            assert_eq!(
                page_picker(&mut harness),
                (ProjectScope::Project(alpha.clone()), false)
            );
            assert_eq!(page_directory(&mut harness), folder);
            harness.act(close_tab);
            harness.read_root(|root, _| {
                assert_eq!(root.route, AppRoute::Chat);
                assert_eq!(root.tabs.active_tab_id(), Some(tab.as_str()));
            });
        }

        // The page's own Mission row is not an entry: the lock stays.
        open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
        harness.act(|root, window, cx| {
            let launcher = root.render_launcher(None, true, window, cx);
            launcher.update(cx, |launcher, cx| {
                launcher.start(Choice::Mission, false, window, cx);
            });
        });
        assert_row_under(&mut harness, &alpha);
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::Project(alpha), true)
        );
    }

    #[test]
    fn new_tab_uses_the_opening_tab_instead_of_the_pickers_project_and_falls_back_if_gone() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let projects = project_fixture(&mut harness);
        let alpha = projects[0].id.clone();
        let beta = projects[1].id.clone();
        let fallback = open_tab_in(&mut harness, Some(&beta));
        let opening = open_tab_in(&mut harness, Some(&alpha));
        harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
        let chosen = beta.clone();
        harness.act(move |root, window, cx| {
            let launcher = root.render_launcher(None, true, window, cx);
            launcher.update(cx, |launcher, cx| launcher.choose_project(&chosen, cx));
            root.open_new_tab_modal(&NewTab, window, cx);
        });
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::Project(alpha), false)
        );
        assert_eq!(page_directory(&mut harness), projects[0].cwd);
        harness.act(move |root, window, cx| {
            root.front_page_client(cx)
                .archive(runner_core::companion::ActionTarget {
                    kind: runner_core::companion::ActionKind::Tab,
                    id: opening,
                })
                .unwrap();
            root.reload_tabs(cx).unwrap();
            assert_eq!(root.recently_viewed_tab_id(cx), Some(fallback));
            root.open_new_tab_modal(&NewTab, window, cx);
        });
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::Project(beta), false)
        );
        assert_eq!(page_directory(&mut harness), projects[1].cwd);

        open_tab_in(&mut harness, None);
        harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
        assert_eq!(page_picker(&mut harness), (ProjectScope::Root, false));
        assert_row_in_recents(&mut harness);
    }

    #[test]
    fn a_preselected_project_keeps_sidebar_selection_in_recents_when_the_row_is_hidden() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let alpha = project_fixture(&mut harness)[0].id.clone();
        for active_only in [false, true] {
            open_tab_in(&mut harness, Some(&alpha));
            let collapsed = alpha.clone();
            update_settings(&mut harness, move |settings| {
                settings.sidebar_chats_open = active_only;
                settings.sidebar_active_only = active_only;
                settings.sidebar_collapsed_projects.insert(collapsed);
                true
            });
            harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
            assert_eq!(
                page_picker(&mut harness),
                (ProjectScope::Project(alpha.clone()), false)
            );
            assert_eq!(row_scope(&harness), None);
            assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_none());
            harness.read_root(|root, cx| {
                assert_eq!(root.active_project_id(cx), None);
                assert!(root
                    .settings(cx)
                    .sidebar_collapsed_projects
                    .contains(&alpha));
            });
        }
    }

    #[test]
    fn closing_the_project_picker_returns_focus_to_search_and_arrows_move_launcher_rows() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        project_fixture(&mut harness);
        open_tab_in(&mut harness, None);
        harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
        for close in ["mouse", "keyboard", "escape"] {
            let picker = harness.bounds("STYLED_SELECT_launcher-project").unwrap();
            harness
                .visual
                .simulate_click(picker.center(), Default::default());
            harness.visual.run_until_parked();
            match close {
                "mouse" => {
                    let option = harness.bounds("STYLED_SELECT_OPTION_1").unwrap();
                    harness
                        .visual
                        .simulate_click(option.center(), Default::default());
                }
                "keyboard" => harness.visual.simulate_keystrokes("home down enter"),
                _ => harness.visual.simulate_keystrokes("escape"),
            }
            harness.visual.run_until_parked();
            let mut selected = 0;
            harness.act(|root, window, cx| {
                let launcher = root.render_launcher(None, true, window, cx);
                assert!(
                    launcher
                        .read(cx)
                        .query
                        .read(cx)
                        .focus_handle()
                        .is_focused(window),
                    "{close}"
                );
                selected = launcher.read(cx).selected;
            });
            harness.visual.simulate_keystrokes("down");
            harness.visual.run_until_parked();
            harness.act(|root, window, cx| {
                let launcher = root.render_launcher(None, true, window, cx);
                assert_ne!(launcher.read(cx).selected, selected, "{close}");
            });
        }
    }

    #[test]
    fn page_close_row_close_and_close_tab_return_to_where_the_launcher_page_opened() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);

        // The window's empty state has nothing behind it: no ×, and ⌘W keeps it.
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.open_new_tab_modal(&NewTab, window, cx);
        });
        assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_some());
        assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB_CLOSE").is_none());
        assert!(harness.bounds("LAUNCHER_PAGE_CLOSE").is_none());
        harness.act(close_tab);
        assert!(harness.read_root(|root, _| root.launcher_page_visible()));

        let mut previous = String::new();
        harness.act(|root, window, cx| {
            root.test_open_empty_tab(window, cx);
            previous = root.tabs.active_tab_id().unwrap().to_owned();
            root.open_new_tab_modal(&NewTab, window, cx);
        });

        // The page's × sits in its top-right corner, and the row's does the same.
        for selector in ["LAUNCHER_PAGE_CLOSE", "SIDEBAR_LAUNCHER_TAB_CLOSE"] {
            let close = harness.bounds(selector).expect("close button");
            if selector == "LAUNCHER_PAGE_CLOSE" {
                let page = harness.bounds("LAUNCHER_PAGE").expect("launcher page");
                assert!(close.right() <= page.right());
                assert!(page.right() - close.right() < px(16.));
                assert!(close.top() - page.top() < px(16.));
            }
            harness
                .visual
                .simulate_click(close.center(), gpui::Modifiers::default());
            harness.visual.run_until_parked();
            harness.read_root(|root, _| {
                assert_eq!(root.route, AppRoute::Chat);
                assert_eq!(root.tabs.active_tab_id(), Some(previous.as_str()));
            });
            assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_none());
            assert!(harness.bounds("LAUNCHER_PAGE_CLOSE").is_none());
            harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
        }
        harness.act(close_tab);

        harness.act(|root, window, cx| root.open_new_tab_modal(&NewTab, window, cx));
        harness.act(close_tab);
        harness.read_root(|root, _| {
            assert_eq!(root.route, AppRoute::Chat);
            assert_eq!(root.tabs.active_tab_id(), Some(previous.as_str()));
            assert_eq!(root.tabs.tabs().len(), 1);
        });

        // ⌘W from the goal step closes the page too, and returns to a page.
        harness.act(|root, window, cx| {
            root.open_crews(window, cx);
            root.open_launcher_page(ProjectScope::Root, Step::Goal(None), false, window, cx);
        });
        harness.act(close_tab);
        harness.read_root(|root, _| assert_eq!(root.route, AppRoute::Crews));
    }

    #[test]
    fn closing_the_active_tab_lands_on_the_last_viewed_tab_with_no_launcher_between() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let tabs = saved_tabs(
            &mut harness,
            &[("first-chat", "codex"), ("viewed-chat", "codex")],
        );
        let close_active_empty_tab = |harness: &mut ModalHarness| {
            harness.act(|root, window, cx| {
                root.test_open_empty_tab(window, cx);
                root.tab_landed_at = None;
            });
            harness.act(close_tab);
            harness.read_root(|root, _| assert_eq!(root.tabs.tabs().len(), 2));
        };

        close_active_empty_tab(&mut harness);
        assert_landed_on(&mut harness, &tabs[1], "viewed-chat");
        assert!(harness.bounds("LAUNCHER_PAGE").is_none());
        assert!(harness.bounds("SIDEBAR_LAUNCHER_TAB").is_none());

        // The tab in another window is passed over, though this window is the
        // more recently focused and would keep the chat.
        open_in_second_window(&mut harness, "viewed-chat");
        close_active_empty_tab(&mut harness);
        assert_landed_on(&mut harness, &tabs[0], "first-chat");

        // With every tab open elsewhere, the last viewed wins.
        harness.act(|root, window, cx| {
            root.tabs.activate(&tabs[1]);
            root.ensure_active_tab_attached(window, cx).unwrap();
            root.settle_active_tab(window, cx);
            root.core(cx).window_register("third").unwrap();
            root.core(cx)
                .report_subjects(
                    "third",
                    vec![runner_core::protocol::window::Subject::DirectChat(
                        "first-chat".into(),
                    )],
                    None,
                )
                .unwrap();
        });
        close_active_empty_tab(&mut harness);
        assert_landed_on(&mut harness, &tabs[1], "viewed-chat");
        harness.act(|root, _, cx| {
            for window in ["second", "third"] {
                root.core(cx)
                    .report_subjects(window, Vec::new(), None)
                    .unwrap();
            }
        });

        // With no tab in the navigation history, it lands on the first tab.
        harness.act(|root, window, cx| {
            root.test_open_empty_tab(window, cx);
            root.tab_landed_at = None;
            root.runtime_navigation_history.clear();
            root.runtime_navigation_index = None;
        });
        harness.act(close_tab);
        assert_landed_on(&mut harness, &tabs[0], "first-chat");
    }

    #[test]
    fn archiving_a_mission_on_screen_lands_on_the_last_viewed_tab() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let tabs = saved_tabs(
            &mut harness,
            &[("first-chat", "codex"), ("viewed-chat", "codex")],
        );
        harness.act(|root, window, cx| {
            root.set_route(AppRoute::Mission("mission".into()), cx);
            root.tabs.activate(&tabs[0]);
            root.leave_archived_mission("mission", Some(false), true, window, cx);
        });
        assert_landed_on(&mut harness, &tabs[1], "viewed-chat");
        harness.read_root(|root, _| assert!(root.tab_close_guarded()));
    }

    #[test]
    fn with_no_tab_left_the_launcher_is_recents_with_no_project_and_nothing_to_close() {
        for path in ["empty tab", "mission", "esc"] {
            let mut harness =
                modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
            let projects = project_fixture(&mut harness);
            let alpha = projects[0].id.clone();
            let tab = open_tab_in(&mut harness, Some(&alpha));
            // The page launcher is cached between visits, locked to the project.
            open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
            assert_row_under(&mut harness, &alpha);
            match path {
                "empty tab" => {
                    let tab = tab.clone();
                    harness.act(move |root, window, cx| {
                        root.tabs.activate(&tab);
                        root.set_route(AppRoute::Chat, cx);
                        root.ensure_active_tab_attached(window, cx).unwrap();
                    });
                    harness.act(close_tab);
                }
                "mission" => {
                    let tab = tab.clone();
                    harness.act(move |root, window, cx| {
                        root.set_route(AppRoute::Mission("mission".into()), cx);
                        root.front_page_client(cx)
                            .archive(runner_core::companion::ActionTarget {
                                kind: runner_core::companion::ActionKind::Tab,
                                id: tab,
                            })
                            .unwrap();
                        root.reload_tabs(cx).unwrap();
                        root.leave_archived_mission("mission", Some(false), true, window, cx);
                    });
                }
                _ => {
                    let tab = tab.clone();
                    harness.act(move |root, _, cx| {
                        root.front_page_client(cx)
                            .archive(runner_core::companion::ActionTarget {
                                kind: runner_core::companion::ActionKind::Tab,
                                id: tab,
                            })
                            .unwrap();
                        root.reload_tabs(cx).unwrap();
                        assert_eq!(root.route, AppRoute::Launcher);
                    });
                    harness.visual.simulate_keystrokes("escape");
                    harness.visual.run_until_parked();
                }
            }
            harness.read_root(|root, _| {
                assert_eq!(root.route, AppRoute::Chat, "{path}");
                assert!(root.tabs.tabs().is_empty(), "{path}");
                assert!(root.launcher_is_empty_state(), "{path}");
            });
            assert_row_in_recents(&mut harness);
            assert_eq!(
                page_picker(&mut harness),
                (ProjectScope::Root, false),
                "{path}"
            );
            assert_ne!(page_directory(&mut harness), projects[0].cwd, "{path}");
            assert!(harness.bounds("LAUNCHER_PAGE_CLOSE").is_none(), "{path}");
            assert!(
                harness.bounds("SIDEBAR_LAUNCHER_TAB_CLOSE").is_none(),
                "{path}"
            );
        }
    }

    #[test]
    fn closing_the_last_tab_while_the_page_shows_leaves_the_page_alone() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let projects = project_fixture(&mut harness);
        let alpha = projects[0].id.clone();
        let tab = open_tab_in(&mut harness, Some(&alpha));
        open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
        harness.act(|root, _, cx| {
            root.front_page_client(cx)
                .archive(runner_core::companion::ActionTarget {
                    kind: runner_core::companion::ActionKind::Tab,
                    id: tab,
                })
                .unwrap();
            root.reload_tabs(cx).unwrap();
            assert_eq!(root.route, AppRoute::Launcher);
        });
        assert_row_under(&mut harness, &alpha);
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::Project(alpha), true)
        );
    }

    #[test]
    fn a_locked_page_files_under_its_project_retargets_and_falls_back_when_it_is_deleted() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let projects = project_fixture(&mut harness);
        let (alpha, beta) = (projects[0].id.clone(), projects[1].id.clone());
        let outside = tempfile::tempdir().unwrap();
        let outside = outside.path().to_string_lossy().into_owned();
        harness.act(|root, _, cx| {
            root.start_chat_modal = None;
            let nodes = root.core(cx).node_list().unwrap();
            root.app_store.update(cx, |store, cx| {
                store.nodes = nodes;
                cx.notify();
            });
        });

        // A folder outside the project stays filed under it, and the menu is off.
        open_plus(&mut harness, ProjectScope::Project(alpha.clone()));
        let folder = outside.clone();
        harness.act(move |root, window, cx| {
            let launcher = root.render_launcher(None, true, window, cx);
            launcher.update(cx, |launcher, cx| {
                launcher.project.picked(&folder, cx);
                launcher
                    .cwd
                    .update(cx, |cwd, cx| cwd.reset(folder.clone(), cx));
                launcher.changed_directory = true;
            });
        });
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::Project(alpha.clone()), true)
        );
        assert_eq!(page_directory(&mut harness), outside);
        let trigger = harness.bounds("STYLED_SELECT_launcher-project").unwrap();
        harness
            .visual
            .simulate_click(trigger.center(), gpui::Modifiers::default());
        harness.visual.run_until_parked();
        assert!(harness.bounds("STYLED_SELECT_MENU").is_none());

        // + on another project takes the row, the lock and the folder with it.
        open_plus(&mut harness, ProjectScope::Project(beta.clone()));
        assert_row_under(&mut harness, &beta);
        assert_eq!(
            page_picker(&mut harness),
            (ProjectScope::Project(beta.clone()), true)
        );
        assert_eq!(page_directory(&mut harness), projects[1].cwd);

        // A deleted project gives the page back.
        let deleted = beta.clone();
        harness.act(move |root, _, cx| {
            root.app_store.update(cx, |store, cx| {
                store.projects.retain(|project| project.id != deleted);
                cx.notify();
            });
        });
        harness.act(|_, _, _| {});
        assert_row_in_recents(&mut harness);
        assert_eq!(page_picker(&mut harness), (ProjectScope::Root, false));
        assert_ne!(page_directory(&mut harness), projects[1].cwd);
    }

    #[cfg(unix)]
    #[test]
    fn a_second_close_right_after_a_landing_leaves_the_terminal_it_landed_on() {
        use std::time::{Duration, Instant};

        fn close_until_gone(harness: &mut ModalHarness, session: &'static str) {
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let mut gone = false;
                harness.act(|root, _, cx| {
                    root.reload_tabs(cx).unwrap();
                    gone = root.session_entry(session, cx).is_none()
                        && !root.stopping_sessions.contains(session);
                });
                if gone {
                    return;
                }
                assert!(Instant::now() < deadline, "{session} did not close");
            }
        }

        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let tabs = saved_tabs(
            &mut harness,
            &[
                ("kept-shell", "shell"),
                ("landing-shell", "shell"),
                ("closing-shell", "shell"),
            ],
        );
        // The history runs kept, landing, closing.
        harness.act(|root, window, cx| {
            for tab in [&tabs[0], &tabs[1], &tabs[2]] {
                root.tabs.activate(tab);
                root.ensure_active_tab_attached(window, cx).unwrap();
                root.settle_active_tab(window, cx);
            }
        });

        harness.act(close_tab);
        close_until_gone(&mut harness, "closing-shell");
        assert_landed_on(&mut harness, &tabs[1], "landing-shell");

        // ⌘W and the pane's × do nothing inside the guard.
        harness.act(close_tab);
        let close = harness.bounds("PANE_CLOSE:p1").expect("pane close");
        harness
            .visual
            .simulate_click(close.center(), gpui::Modifiers::default());
        harness.visual.run_until_parked();
        harness.act(|root, _, cx| {
            root.reload_tabs(cx).unwrap();
            assert!(root.stopping_sessions.is_empty());
            assert!(root.session_entry("landing-shell", cx).is_some());
            assert_eq!(root.tabs.active_tab_id(), Some(tabs[1].as_str()));
        });

        // A later press closes it, and lands again.
        harness.act(|root, _, _| {
            root.tab_landed_at = Some(Instant::now() - TAB_LANDING_CLOSE_GUARD);
        });
        harness.act(close_tab);
        close_until_gone(&mut harness, "landing-shell");
        assert_landed_on(&mut harness, &tabs[0], "kept-shell");
        harness.read_root(|root, _| assert!(root.tab_close_guarded()));
    }

    #[test]
    fn sidebar_plus_keeps_reused_page_project_picker_and_start_scope_in_sync() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let projects = project_fixture(&mut harness);
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.open_new_tab_modal(&NewTab, window, cx);
        });
        for (step, project) in [
            (Step::Start, Some(projects[0].id.clone())),
            (Step::Crews, Some(projects[1].id.clone())),
            (Step::Goal(None), None),
        ] {
            harness.act(|root, window, cx| {
                let launcher = root.render_launcher(None, true, window, cx);
                launcher.update(cx, |launcher, cx| launcher.open_step(step, window, cx));
                root.open_sidebar_chat_modal(ProjectScope::or_root(project.clone()), window, cx);
            });
            harness.act(|root, window, cx| {
                let launcher = root.render_launcher(None, true, window, cx);
                let launcher = launcher.read(cx);
                assert_eq!(launcher.step, Step::Start);
                assert_eq!(launcher.scope(), ProjectScope::or_root(project.clone()));
                assert_eq!(
                    launcher.project.select().read(cx).value(),
                    project.as_deref().unwrap_or_default()
                );
                assert_eq!(
                    launcher.project.select().read(cx).is_disabled(),
                    project.is_some()
                );
                assert_eq!(root.launcher_page_project(cx), project);
                if let Some(project) = &project {
                    assert_eq!(
                        launcher.cwd.read(cx).text(),
                        projects
                            .iter()
                            .find(|entry| &entry.id == project)
                            .unwrap()
                            .cwd
                    );
                }
                assert!(root.tabs.tabs().is_empty());
            });
        }
    }

    #[test]
    fn cached_launchers_follow_split_and_collapse_while_preserving_explicit_change() {
        for changed in [false, true] {
            let mut harness =
                modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
            harness.act(|root, window, cx| {
                root.start_chat_modal = None;
                root.set_route(AppRoute::Chat, cx);
                root.test_open_empty_tab(window, cx);
                let original = root.tabs.active().unwrap().clone();
                let key = PaneKey::new(&original.id, &original.focused_pane_id);
                let first = root.render_launcher(Some(key.clone()), true, window, cx);
                first.update(cx, |launcher, cx| {
                    launcher
                        .query
                        .update(cx, |field, cx| field.reset("find", cx));
                    if changed {
                        launcher
                            .cwd
                            .update(cx, |field, cx| field.reset("/chosen", cx));
                        launcher.changed_directory = true;
                    }
                });
                root.split_pane(&key.pane_id, SplitOrientation::Row, window, cx);
                let sibling = root.tabs.active().unwrap().focused_pane_id.clone();
                let split = root.render_launcher(Some(key.clone()), false, window, cx);
                assert_ne!(first, split);
                let split_state = split.read(cx);
                assert!(!split_state.default_page);
                assert!(split_state.project.select().read(cx).is_disabled());
                assert_eq!(split_state.query.read(cx).text(), "find");
                if changed {
                    assert_eq!(split_state.cwd.read(cx).text(), "/chosen");
                }
                root.close_pane(&sibling, window, cx);
                let single = root.render_launcher(Some(key), true, window, cx);
                assert_ne!(split, single);
                let single = single.read(cx);
                assert!(single.default_page);
                assert!(!single.project.select().read(cx).is_disabled());
                assert_eq!(single.query.read(cx).text(), "find");
                assert_eq!(single.changed_directory, changed);
                if changed {
                    assert_eq!(single.cwd.read(cx).text(), "/chosen");
                }
            });
        }
    }

    struct HeldLocalStart {
        base: runner_daemon::daemon::InProcessTransport,
        calls: std::sync::atomic::AtomicUsize,
        entered: std::sync::mpsc::Sender<()>,
        release: std::sync::Mutex<std::sync::mpsc::Receiver<()>>,
    }
    impl runner_core::protocol::Transport for HeldLocalStart {
        fn call(
            &self,
            request: runner_core::protocol::Request,
        ) -> Result<runner_core::protocol::Response, runner_core::protocol::ClientError> {
            if matches!(
                &request,
                runner_core::protocol::Request::session_start_runtime_with_speed { .. }
            ) {
                self.calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                self.entered.send(()).unwrap();
                self.release
                    .lock()
                    .unwrap()
                    .recv_timeout(std::time::Duration::from_secs(5))
                    .unwrap();
                return Ok(
                    runner_core::protocol::Response::session_start_runtime_with_speed(Err(
                        runner_core::protocol::ClientError::msg("bounded start failure"),
                    )),
                );
            }
            self.base.call(request)
        }
        fn subscribe(&self) -> Box<dyn runner_core::protocol::EventSubscription> {
            self.base.subscribe()
        }
    }

    #[test]
    fn delayed_local_start_blocks_other_panes_and_releases_its_reservation_on_failure() {
        for split in [false, true] {
            use std::sync::{atomic::Ordering, mpsc, Arc, Mutex};
            let mut harness =
                modal_harness(1800., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
            let (entered, started) = mpsc::channel();
            let (release, blocked) = mpsc::channel();
            let (attempted, attempts) = mpsc::channel();
            let observer = std::thread::spawn(move || {
                let started = started.recv_timeout(std::time::Duration::from_secs(5));
                let attempted = attempts.recv_timeout(std::time::Duration::from_secs(5));
                let _ = release.send(());
                (started, attempted)
            });
            let mut trace = None;
            let mut tab = String::new();
            harness.act(|root, window, cx| {
                root.start_chat_modal = None;
                root.set_route(AppRoute::Chat, cx);
                let core = root.app_store.read(cx).test_core.clone();
                let transport = Arc::new(HeldLocalStart {
                    base: runner_daemon::daemon::InProcessTransport(core),
                    calls: Default::default(),
                    entered,
                    release: Mutex::new(blocked),
                });
                root.app_store.update(cx, |store, _| {
                    store.client = runner_core::protocol::DaemonClient::new(transport.clone())
                });
                trace = Some(transport);
                root.test_open_empty_tab(window, cx);
                let first = root.front_page_layout().unwrap().focused_pane_id.clone();
                if split {
                    root.split_pane(&first, SplitOrientation::Row, window, cx);
                }
                let original = root.front_page_layout().unwrap().clone();
                tab = original.id.clone();
                let second = original.focused_pane_id.clone();
                for pane in [&first, &second] {
                    let view =
                        root.render_launcher(Some(PaneKey::new(&tab, pane)), false, window, cx);
                    view.update(cx, |launcher, _| launcher.runtimes = vec![agent()]);
                }
                root.focus_pane(&first, cx);
                root.test_start_launcher(window, cx);
                assert!(root.launcher_tab_pending(&tab, cx));
                root.focus_pane(&second, cx);
                root.test_start_launcher(window, cx);
                crate::surfaces::windowing::close_tab(root, window, cx);
                root.close_pane(&second, window, cx);
                root.split_pane(&second, SplitOrientation::Row, window, cx);
                assert_eq!(root.front_page_layout().unwrap().root, original.root);
                attempted.send(()).unwrap();
            });
            let (started, attempted) = observer.join().unwrap();
            started.unwrap();
            attempted.unwrap();
            assert_eq!(trace.unwrap().calls.load(Ordering::SeqCst), 1);
            harness.act(|root, _, cx| assert!(!root.launcher_tab_pending(&tab, cx)));
        }
    }

    #[test]
    fn daemon_role_changes_refresh_the_catalog_in_an_existing_launcher() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let mut view = None;
        let mut role_id = String::new();
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            view = Some(root.render_launcher(None, true, window, cx));
            role_id = root.core(cx).role_create(serde_json::from_value(serde_json::json!({
                "handle": "qa-launcher", "display_name": "QA launcher", "runtime": "codex", "command": "codex", "working_dir": "/created"
            })).unwrap()).unwrap().id;
        });
        harness.act(|root, window, cx| {
            let cached = root.render_launcher(None, true, window, cx);
            assert_eq!(cached, *view.as_ref().unwrap());
            let role = cached.read(cx).roles.iter().find(|role| role.id == role_id).expect("created role reaches the launcher through its daemon event");
            assert_eq!(role.working_dir.as_deref(), Some("/created"));
            root.core(cx).role_update(&role_id, serde_json::from_value(serde_json::json!({"display_name":"Edited launcher", "working_dir":"/edited"})).unwrap()).unwrap();
        });
        harness.act(|root, window, cx| {
            let cached = root.render_launcher(None, true, window, cx);
            let role = cached
                .read(cx)
                .roles
                .iter()
                .find(|role| role.id == role_id)
                .unwrap();
            assert_eq!(role.display_name, "Edited launcher");
            assert_eq!(role.working_dir.as_deref(), Some("/edited"));
            root.core(cx).role_delete(&role_id).unwrap();
        });
        harness.act(|root, window, cx| {
            let cached = root.render_launcher(None, true, window, cx);
            assert!(!cached.read(cx).roles.iter().any(|role| role.id == role_id));
        });
    }

    #[test]
    fn cli_project_create_and_rename_update_an_open_launcher_picker() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let directory = tempfile::tempdir().unwrap();
        let mut view = None;
        let mut project_id = String::new();
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            view = Some(root.render_launcher(None, true, window, cx));
            project_id = root
                .core(cx)
                .project_create_checked(
                    "CLI project".into(),
                    directory.path().to_string_lossy().into_owned(),
                )
                .unwrap()
                .id;
        });
        harness.act(|root, window, cx| {
            let cached = root.render_launcher(None, true, window, cx);
            assert_eq!(cached, *view.as_ref().unwrap());
            cached.update(cx, |launcher, cx| {
                launcher.project.choose(&project_id, "");
                let picker = launcher.project.select();
                picker.update(cx, |select, cx| select.set_value(project_id.clone(), cx));
                assert_eq!(picker.read(cx).selected().unwrap().label, "CLI project");
            });
            root.core(cx)
                .project_rename(project_id.clone(), "Renamed CLI project".into())
                .unwrap();
        });
        harness.act(|root, window, cx| {
            let cached = root.render_launcher(None, true, window, cx);
            assert_eq!(cached, *view.as_ref().unwrap());
            let picker = cached.read(cx).project.select();
            assert_eq!(
                picker.read(cx).selected().unwrap().label,
                "Renamed CLI project"
            );
        });
    }

    #[test]
    fn cached_launcher_reconciles_role_and_project_edits_without_losing_search_or_change() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let projects = project_fixture(&mut harness);
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            let original = root.render_launcher(None, true, window, cx);
            original.update(cx, |launcher, cx| {
                launcher
                    .query
                    .update(cx, |field, cx| field.reset("coder", cx));
                launcher
                    .cwd
                    .update(cx, |field, cx| field.reset("/changed", cx));
                launcher.changed_directory = true;
                launcher.project.choose(&projects[0].id, "/changed");
            });
            let role = crate::surfaces::start_chat::tests::test_role("coder", "codex");
            root.app_store
                .update(cx, |store, cx| store.replace_roles(vec![role.clone()], cx));
            let reused = root.render_launcher(None, true, window, cx);
            assert_eq!(original, reused);
            assert_eq!(reused.read(cx).roles.len(), 1);
            root.app_store.update(cx, |store, _| {
                store.roles[0].working_dir = Some("/edited-role".into());
                store.projects[0].name = "Renamed project".into();
                store.revisions.roles += 1;
                store.revisions.projects += 1;
            });
            root.render_launcher(None, true, window, cx);
            let cached = reused.read(cx);
            assert_eq!(cached.roles[0].working_dir.as_deref(), Some("/edited-role"));
            assert_eq!(cached.query.read(cx).text(), "coder");
            assert_eq!(cached.cwd.read(cx).text(), "/changed");
            assert!(cached.changed_directory);
            assert_eq!(cached.project.chosen().unwrap().name, "Renamed project");
            reused.update(cx, |launcher, _| {
                launcher.project.choose(&projects[0].id, "/changed");
            });
            root.app_store.update(cx, |store, _| {
                store.roles.clear();
                store.projects.clear();
                store.revisions.roles += 1;
                store.revisions.projects += 1;
            });
            root.render_launcher(None, true, window, cx);
            assert!(reused.read(cx).roles.is_empty());
            assert!(reused.read(cx).project.chosen().is_none());
        });
    }

    #[test]
    fn recency_is_cached_across_redraws_and_reloads_after_a_recorded_use() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let mut view = None;
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            root.app_store.update(cx, |store, cx| {
                store.replace_roles(
                    vec![crate::surfaces::start_chat::tests::test_role(
                        "coder", "codex",
                    )],
                    cx,
                )
            });
            let launcher = root.render_launcher(None, true, window, cx);
            launcher.update(cx, |launcher, _| {
                launcher.runtimes = vec![agent()];
                launcher.default_runtime = "codex".into();
            });
            let directory = launcher.read(cx).app_data_dir.clone();
            let mut disk = RecentStore::default();
            disk.record(
                "local",
                EntryKey::Role("role-coder".into()),
                chrono::Utc::now().timestamp(),
            );
            disk.save(&directory).unwrap();
            for _ in 0..3 {
                root.render_launcher(None, true, window, cx);
                assert_eq!(
                    launcher.read(cx).rows(cx).0[0].choice,
                    Choice::Chat(EntryKey::Agent("codex".into()))
                );
            }
            launcher.update(cx, |launcher, cx| launcher.reload_recents(cx));
            assert_eq!(
                launcher.read(cx).rows(cx).0[0].choice,
                Choice::Chat(EntryKey::Role("role-coder".into()))
            );
            record_launcher_use(
                &directory,
                &FrontPageTarget::Local,
                EntryKey::Agent("codex".into()),
            );
            root.render_launcher(None, true, window, cx);
            assert_eq!(
                launcher.read(cx).rows(cx).0[0].choice,
                Choice::Chat(EntryKey::Agent("codex".into()))
            );
            view = Some(launcher);
        });
        assert!(harness.visual.debug_bounds("LAUNCHER_BADGE:0").is_some());
        harness.act(|_, _, cx| {
            view.as_ref().unwrap().update(cx, |launcher, cx| {
                launcher
                    .query
                    .update(cx, |field, cx| field.reset("Role coder", cx))
            })
        });
        assert!(harness.visual.debug_bounds("LAUNCHER_BADGE:0").is_none());
        assert!(
            harness.visual.debug_bounds("LAUNCHER_BADGE:1").is_some(),
            "Terminal keeps its shortcut badge while searching"
        );
    }

    #[test]
    fn recency_under_one_hour_reads_just_now() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for seconds in [0, 60, 3599] {
            assert_eq!(
                last_use(Some(chrono::Utc::now().timestamp() - seconds)),
                crate::i18n::text("launcher.just_now")
            );
        }
    }

    #[test]
    fn no_tab_terminal_click_and_shortcut_persist_a_nonempty_tab_id() {
        for shortcut in [false, true] {
            let mut harness =
                modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
            harness.act(|root, window, cx| {
                root.start_chat_modal = None;
                root.set_route(AppRoute::Chat, cx);
                let view = root.render_launcher(None, true, window, cx);
                if shortcut {
                    root.new_terminal_action(&NewTerminal, window, cx);
                } else {
                    view.update(cx, |launcher, cx| {
                        launcher.start(Choice::Terminal, false, window, cx)
                    });
                }
            });
            harness.act(|root, _, cx| {
                let layout = root.tabs.active().expect("terminal has its own tab");
                assert!(!layout.id.is_empty());
                let session = layout
                    .focused_session_id()
                    .expect("terminal is assigned")
                    .to_owned();
                assert_eq!(root.tabs.tabs().len(), 1);
                root.core(cx).session_close(&session).unwrap();
            });
        }
    }

    #[test]
    fn search_digits_escape_and_arrows_use_the_real_text_field_without_starting_unavailable_roles()
    {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let mut launcher = None;
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            root.test_open_empty_tab(window, cx);
            let first = root.tabs.active().unwrap().focused_pane_id.clone();
            root.split_pane(&first, SplitOrientation::Row, window, cx);
            let layout = root.front_page_layout().unwrap();
            let pane = PaneKey::new(&layout.id, &layout.focused_pane_id);
            let view = root.render_launcher(Some(pane), false, window, cx);
            view.update(cx, |launcher, cx| {
                launcher.roles = vec![role()];
                launcher.runtimes.clear();
                launcher
                    .query
                    .update(cx, |query, cx| query.reset("Bounded", cx));
                launcher.focus(window, cx);
            });
            launcher = Some(view);
        });
        harness.visual.simulate_keystrokes("2");
        harness.visual.run_until_parked();
        harness.act(|_, _, cx| {
            assert_eq!(
                launcher.as_ref().unwrap().read(cx).query.read(cx).text(),
                "Bounded2"
            )
        });
        harness.visual.simulate_keystrokes("escape down up enter");
        harness.visual.run_until_parked();
        harness.act(|_, _, cx| {
            let launcher = launcher.as_ref().unwrap().read(cx);
            assert_eq!(launcher.query.read(cx).text(), "");
            assert!(launcher.pending.is_none());
            assert_eq!(launcher.selected, 0);
        });
    }

    #[test]
    fn launcher_takes_focus_back_from_the_window_and_tab_so_arrows_move_the_selection() {
        for tab in [false, true] {
            let mut harness =
                modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
            let mut launcher = None;
            harness.act(|root, window, cx| {
                root.start_chat_modal = None;
                root.set_route(AppRoute::Chat, cx);
                if tab {
                    root.open_new_tab_modal(&NewTab, window, cx);
                }
                let pane = root
                    .front_page_layout()
                    .map(|layout| PaneKey::new(&layout.id, &layout.focused_pane_id));
                let view = root.render_launcher(pane, true, window, cx);
                view.update(cx, |launcher, _| {
                    launcher.runtimes = vec![agent()];
                    launcher.roles = vec![role()];
                });
                if tab {
                    root.chat_focus.focus(window, cx);
                } else {
                    root.root_focus.focus(window, cx);
                }
                launcher = Some(view);
            });
            harness.visual.run_until_parked();
            for (keys, expected) in [("down", 1), ("down", 2), ("up up", 0), ("up", usize::MAX)] {
                harness.visual.simulate_keystrokes(keys);
                harness.visual.run_until_parked();
                harness.act(|_, _, cx| {
                    let launcher = launcher.as_ref().unwrap().read(cx);
                    assert_eq!(
                        launcher.selected,
                        expected.min(launcher.rows(cx).0.len() - 1),
                        "after {keys} with the {} focused",
                        if tab { "tab" } else { "window" }
                    )
                });
            }
        }
    }

    #[test]
    fn clicking_the_sidebar_search_keeps_launcher_keys_after_the_palette_closes() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let mut launcher = None;
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            root.open_new_tab_modal(&NewTab, window, cx);
            let view = root.render_launcher(None, true, window, cx);
            view.update(cx, |launcher, cx| {
                launcher.runtimes = vec![agent()];
                launcher.roles = vec![role()];
                launcher.focus(window, cx);
            });
            launcher = Some(view);
        });
        harness.visual.run_until_parked();
        let search = harness.visual.debug_bounds("SIDEBAR_SEARCH").unwrap();
        harness
            .visual
            .simulate_click(search.center(), gpui::Modifiers::default());
        harness.visual.run_until_parked();
        harness.visual.simulate_keystrokes("escape");
        harness.visual.run_until_parked();
        harness.visual.simulate_keystrokes("down");
        harness.visual.run_until_parked();
        harness.act(|_, window, cx| {
            let launcher = launcher.as_ref().unwrap().read(cx);
            assert!(launcher.query.read(cx).focus_handle().is_focused(window));
            assert_eq!(launcher.selected, 1);
        });
    }

    #[test]
    fn command_palette_opens_from_the_launcher_search_and_returns_to_it() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let mut launcher = None;
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            root.open_new_tab_modal(&NewTab, window, cx);
            let view = root.render_launcher(None, true, window, cx);
            view.update(cx, |launcher, cx| launcher.focus(window, cx));
            launcher = Some(view);
        });
        harness.visual.run_until_parked();
        for (keys, focused) in [
            (keymap::platform_default("cmd-k"), false),
            ("escape".to_owned(), true),
        ] {
            harness.visual.simulate_keystrokes(&keys);
            harness.visual.run_until_parked();
            harness.act(|_, window, cx| {
                let launcher = launcher.as_ref().unwrap().read(cx);
                assert_eq!(
                    launcher.query.read(cx).focus_handle().is_focused(window),
                    focused,
                    "after {keys}"
                );
            });
        }
    }

    #[test]
    fn clicking_search_after_a_directory_picker_keeps_launcher_keys_active() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let chosen = tempfile::tempdir().unwrap();
        let mut launcher = None;
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            let view = root.render_launcher(None, true, window, cx);
            view.update(cx, |launcher, cx| {
                launcher.roles = vec![role()];
                launcher.runtimes.clear();
                launcher.change_directory(window, cx);
            });
            launcher = Some(view);
        });
        assert!(harness.visual.did_prompt_for_paths());
        harness
            .visual
            .simulate_path_prompt_response(|_| Some(vec![chosen.path().to_owned()]));
        harness.visual.run_until_parked();
        let bounds = harness.visual.debug_bounds("LAUNCHER_SEARCH").unwrap();
        for position in [
            gpui::point(bounds.center().x, bounds.top() + px(2.)),
            bounds.center(),
        ] {
            harness
                .visual
                .simulate_click(position, gpui::Modifiers::default());
            harness.visual.run_until_parked();
            harness.act(|_, window, cx| {
                assert!(
                    launcher
                        .as_ref()
                        .unwrap()
                        .read(cx)
                        .query
                        .read(cx)
                        .focus_handle()
                        .is_focused(window),
                    "the entire search field focuses its input"
                )
            });
            harness.visual.simulate_keystrokes("1");
            harness.visual.run_until_parked();
            harness.act(|_, _, cx| {
                assert_eq!(
                    launcher.as_ref().unwrap().read(cx).query.read(cx).text(),
                    ""
                )
            });
            harness.visual.simulate_keystrokes("q 1");
            harness.visual.run_until_parked();
            harness.act(|_, _, cx| {
                assert_eq!(
                    launcher.as_ref().unwrap().read(cx).query.read(cx).text(),
                    "q1"
                );
                launcher.as_ref().unwrap().update(cx, |launcher, cx| {
                    launcher.query.update(cx, |query, cx| query.reset("", cx))
                });
            });
        }
    }

    #[test]
    fn project_trigger_is_inline_with_one_location_icon_and_keeps_its_menu() {
        let mut harness = modal_harness(1800., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        let projects = project_fixture(&mut harness);
        let long_project = tempfile::tempdir().unwrap();
        let long_path = long_project
            .path()
            .join("project-with-a-long-folder-name-for-preserving-the-project-picker-menu-width");
        std::fs::create_dir_all(&long_path).unwrap();
        let mut launcher = None;
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            root.chat_panel_visibility = SidebarVisibilityTransition::new(false);
            root.core(cx)
                .project_create_checked(
                    "Zeta destination with a long project name and directory preview".into(),
                    long_path.to_string_lossy().into_owned(),
                )
                .unwrap();
            root.open_launcher_page(
                ProjectScope::Project(projects[0].id.clone()),
                Step::Start,
                false,
                window,
                cx,
            );
            launcher = Some(root.render_launcher(None, true, window, cx));
        });
        for zoom in [1., 1.3] {
            harness.act(|root, window, cx| root.set_zoom(zoom, window, cx));
            let line = harness.visual.debug_bounds("LAUNCHER_LOCATION").unwrap();
            let trigger = harness
                .visual
                .debug_bounds("STYLED_SELECT_launcher-project")
                .unwrap();
            let icon = harness
                .visual
                .debug_bounds("LAUNCHER_LOCATION_FOLDER")
                .unwrap();
            assert!((line.size.height - px(20. * zoom)).abs() < px(0.1));
            assert!((trigger.size.height - px(20. * zoom)).abs() < px(0.1));
            assert!((trigger.center().y - line.center().y).abs() < px(0.1));
            assert!(
                (icon.size.height - px(16. * zoom)).abs() <= px(1.),
                "zoom {zoom}: {icon:?}"
            );
            assert!(
                harness
                    .visual
                    .debug_bounds("STYLED_SELECT_TRIGGER_LEADING")
                    .is_none(),
                "the location line owns the single folder icon"
            );
        }
        let trigger = harness
            .visual
            .debug_bounds("STYLED_SELECT_launcher-project")
            .unwrap();
        harness
            .visual
            .simulate_click(trigger.center(), gpui::Modifiers::default());
        harness.visual.run_until_parked();
        let menu = harness.visual.debug_bounds("STYLED_SELECT_MENU").unwrap();
        let trigger = harness
            .visual
            .debug_bounds("STYLED_SELECT_launcher-project")
            .unwrap();
        assert!(
            (menu.left() - trigger.left()).abs() <= px(1.),
            "the existing menu stays left-anchored: trigger={trigger:?}, menu={menu:?}"
        );
        assert!(
            menu.size.width > px(240. * 1.3 + 1.),
            "long metadata retains content-based width: {menu:?}"
        );
        assert!(menu.size.width <= px(480. * 1.3 + 1.));
        harness.visual.simulate_keystrokes("down enter");
        harness.visual.run_until_parked();
        harness.act(|root, window, cx| {
            assert_eq!(
                launcher.as_ref().unwrap().read(cx).scope(),
                ProjectScope::Project(projects[1].id.clone())
            );
            root.test_open_empty_tab(window, cx);
            let pane = root.front_page_layout().unwrap().focused_pane_id.clone();
            root.split_pane(&pane, SplitOrientation::Row, window, cx);
        });
        assert!(harness
            .visual
            .debug_bounds("STYLED_SELECT_launcher-project")
            .is_none());
        assert!(harness
            .visual
            .debug_bounds("STYLED_SELECT_TRIGGER_LEADING")
            .is_none());
        assert!(harness.visual.debug_bounds("LAUNCHER_LOCATION").is_none());
        assert!(
            harness.visual.debug_bounds("LAUNCHER_SEARCH").is_some(),
            "a split pane shows its search field with no project or folder control"
        );
    }

    #[test]
    fn a_short_split_measures_its_launcher_and_drops_recents_to_fit() {
        let mut harness = modal_harness(1200., 440., Vec::new(), Vec::new(), ChatMode::Runtime);
        let mut launcher = None;
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            root.set_route(AppRoute::Chat, cx);
            root.test_open_empty_tab(window, cx);
            let first = root.front_page_layout().unwrap().focused_pane_id.clone();
            root.split_pane(&first, SplitOrientation::Column, window, cx);
            let layout = root.front_page_layout().unwrap().clone();
            let pane = PaneKey::new(&layout.id, &layout.focused_pane_id);
            let view = root.render_launcher(Some(pane), false, window, cx);
            view.update(cx, |launcher, _| {
                launcher.runtimes = vec![agent()];
                launcher.roles = (1..=4)
                    .map(|n| RoleChoice {
                        id: format!("role-{n}"),
                        display_name: format!("Role {n}"),
                        ..role()
                    })
                    .collect();
            });
            launcher = Some(view);
        });
        harness.visual.run_until_parked();
        harness.act(|_, _, cx| {
            let launcher = launcher.as_ref().unwrap().read(cx);
            let height = launcher.height.expect("the launcher measures its pane");
            let rows = launcher.rows(cx).0;
            let ranked = rows
                .iter()
                .filter(|row| matches!(row.choice, Choice::Chat(_)))
                .count();
            assert!(height < 225., "the split is short: {height}");
            assert!((1..4).contains(&ranked), "{ranked} recents in {height}");
            assert_eq!((ranked, rows.len() - ranked), model::fit_rows(height, 4, 1));
        });
        harness.act(|root, _, cx| {
            let layout = root.tabs.active_mut().unwrap();
            let focused = layout.focused_pane_id.clone();
            let runner_core::pane_layout::PaneNode::Split(split) = &layout.root else {
                panic!("a split tab");
            };
            let sizes = if split.a.leaves()[0].id == focused {
                [15., 85.]
            } else {
                [85., 15.]
            };
            let id = split.id.clone();
            assert!(layout.set_split_sizes(&id, sizes));
            cx.notify();
        });
        harness.visual.run_until_parked();
        harness.act(|_, _, cx| {
            let launcher = launcher.as_ref().unwrap().read(cx);
            let height = launcher.height.unwrap();
            assert!(
                model::search_is_tight(height),
                "the divider at its 15% limit: {height}"
            );
            assert!(launcher.rows(cx).0.is_empty());
        });
        harness.act(|root, _, cx| {
            let layout = root.tabs.active_mut().unwrap();
            let runner_core::pane_layout::PaneNode::Split(split) = &layout.root else {
                panic!("a split tab");
            };
            let id = split.id.clone();
            assert!(layout.set_split_sizes(&id, [50., 50.]));
            cx.notify();
        });
        harness
            .visual
            .simulate_resize(gpui::size(px(1200.), px(240.)));
        harness.visual.run_until_parked();
        harness.act(|_, _, cx| {
            let launcher = launcher.as_ref().unwrap().read(cx);
            let height = launcher.height.unwrap();
            assert!(height < 95., "a window shrunk after the split: {height}");
            assert!(
                launcher.rows(cx).0.is_empty(),
                "below one row only the search field is left in {height}"
            );
        });
    }

    #[test]
    fn capability_hiding_keeps_pinned_rows_out_of_ranking() {
        let mut harness = modal_harness(1200., 1000., Vec::new(), Vec::new(), ChatMode::Runtime);
        harness.act(|root, window, cx| {
            root.start_chat_modal = None;
            let view = root.render_launcher(None, true, window, cx);
            let access = crate::surfaces::front_page::RemoteAccess::new(FrontPageTarget::Remote {
                computer: "host".into(),
                fingerprint: "fp".into(),
                generation: 1,
            });
            let remote = FrontPageClient::remote(root.core(cx).clone(), access.clone());
            view.update(cx, |launcher, cx| {
                launcher.client = remote;
                launcher.runtimes = vec![agent()];
                launcher.roles = (0..6)
                    .map(|index| {
                        let mut role = role();
                        role.id = index.to_string();
                        role.display_name = format!("Role {index}");
                        role
                    })
                    .collect();
                access.set_features(vec![
                    "catalog".into(),
                    "startChat".into(),
                    "layout".into(),
                    "startShell".into(),
                    "startMission".into(),
                ]);
                launcher
                    .query
                    .update(cx, |query, cx| query.reset("Role", cx));
                let (rows, total) = launcher.rows(cx);
                assert_eq!(total, 7);
                assert_eq!(rows.len(), 6);
                assert_eq!(rows[4].choice, Choice::Terminal);
                assert_eq!(rows[5].choice, Choice::Mission);
                access.set_features(vec!["catalog".into(), "startChat".into()]);
                assert!(launcher.rows(cx).0.is_empty());
                access.set_features(vec!["layout".into(), "startShell".into()]);
                let rows = launcher.rows(cx).0;
                assert_eq!(rows.len(), 1);
                assert_eq!(rows[0].choice, Choice::Terminal);
            });
        });
    }
}

#[cfg(test)]
mod remote_tests {
    use super::*;
    use crate::surfaces::remote_client::tests::{
        action_trace, add_window, choose_host, enable_front_page, fixture, overview,
    };
    use gpui::{TestAppContext, VisualTestContext};

    #[test]
    fn direct_role_start_refreshes_recency_in_both_remote_launcher_windows() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        let temp = tempfile::tempdir().unwrap();
        let mut cx = TestAppContext::single();
        let (store, fake, _) = fixture(&mut cx, temp.path());
        enable_front_page(&fake);
        let mut data = overview();
        data.tabs.clear();
        data.sessions.clear();
        fake.set_overview(data);
        let first = add_window(&mut cx, &store, temp.path(), "recent-first");
        let second = add_window(&mut cx, &store, temp.path(), "recent-second");
        for window in [first, second] {
            window.update(&mut cx, choose_host).unwrap();
        }
        cx.run_until_parked();
        for window in [first, second] {
            window
                .update(&mut cx, |root, window, cx| {
                    root.render_launcher(None, true, window, cx);
                })
                .unwrap();
        }
        cx.run_until_parked();

        for window in [first, second] {
            window
                .update(&mut cx, |root, window, cx| {
                    let launcher = root.render_launcher(None, true, window, cx);
                    assert_eq!(
                        launcher.read(cx).rows(cx).0[0].choice,
                        Choice::Chat(EntryKey::Agent("codex".into()))
                    );
                })
                .unwrap();
        }
        first
            .update(&mut cx, |root, window, cx| {
                root.open_remote_role_chat("host-role".into(), window, cx)
            })
            .unwrap();
        cx.run_until_parked();
        assert_eq!(action_trace(&fake, "startChat").len(), 1);
        for window in [first, second] {
            window
                .update(&mut cx, |root, window, cx| {
                    let launcher = root.render_launcher(None, true, window, cx);
                    assert_eq!(
                        launcher.read(cx).rows(cx).0[0].choice,
                        Choice::Chat(EntryKey::Role("host-role".into()))
                    );
                })
                .unwrap();
        }
    }

    #[test]
    fn no_tab_remote_terminal_click_and_shortcut_send_a_nonempty_tab_id() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for shortcut in [false, true] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, _) = fixture(&mut cx, temp.path());
            enable_front_page(&fake);
            let mut data = overview();
            data.tabs.clear();
            data.sessions.clear();
            fake.set_overview(data);
            let window = add_window(&mut cx, &store, temp.path(), "no-tab-terminal");
            window.update(&mut cx, choose_host).unwrap();
            cx.run_until_parked();
            window
                .update(&mut cx, |root, window, cx| {
                    let view = root.render_launcher(None, true, window, cx);
                    if shortcut {
                        root.new_terminal_action(&NewTerminal, window, cx);
                    } else {
                        view.update(cx, |launcher, cx| {
                            launcher.start(Choice::Terminal, false, window, cx)
                        });
                    }
                })
                .unwrap();
            cx.run_until_parked();
            assert_eq!(action_trace(&fake, "startShell").len(), 1);
            let layouts = action_trace(&fake, "layout");
            assert_eq!(layouts.len(), 1);
            assert!(!layouts[0]["target"]["id"].as_str().unwrap().is_empty());
            assert_eq!(layouts[0]["panes"]["session"], "started-session");
        }
    }

    #[test]
    fn host_role_directory_metadata_and_explicit_change_keep_role_settings_without_overrides() {
        let _theme = crate::theme_snapshot::ThemeGuard::new();
        for (metadata, changed) in [(true, false), (true, true), (false, false)] {
            let temp = tempfile::tempdir().unwrap();
            let mut cx = TestAppContext::single();
            let (store, fake, _) = fixture(&mut cx, temp.path());
            enable_front_page(&fake);
            let mut data = overview();
            data.tabs[0].panes = runner_core::companion::Pane::Leaf { session: None };
            fake.set_overview(data);
            let window = add_window(&mut cx, &store, temp.path(), "launcher-role-folder");
            window.update(&mut cx, choose_host).unwrap();
            cx.run_until_parked();
            if metadata && !changed {
                fake.queue_result("role", Ok(serde_json::json!({"id":"host-role","handle":"host-coder","name":"Host coder","runtime":"codex","command":"codex","args":[],"workingDir":"/role-directory","systemPrompt":null,"model":"host-model","effort":"low","speed":null,"permissionMode":null})));
            }
            let view = window
                .update(&mut cx, |root, window, cx| {
                    let layout = root.front_page_layout().unwrap();
                    let pane = PaneKey::new(&layout.id, &layout.focused_pane_id);
                    let view = root.render_launcher(Some(pane), true, window, cx);
                    let access = crate::surfaces::front_page::RemoteAccess::new(
                        root.front_page_client(cx).target,
                    );
                    let mut features = vec!["catalog".into(), "startChat".into(), "layout".into()];
                    if metadata {
                        features.push("role".into());
                    }
                    access.set_features(features);
                    let client = FrontPageClient::remote(root.core(cx).clone(), access);
                    view.update(cx, |launcher, cx| {
                        launcher.client = client;
                        launcher
                            .cwd
                            .update(cx, |field, cx| field.reset("/shown", cx));
                        launcher.changed_directory = changed;
                        launcher.start(
                            Choice::Chat(EntryKey::Role("host-role".into())),
                            false,
                            window,
                            cx,
                        );
                    });
                    view
                })
                .unwrap();
            cx.run_until_parked();
            let calls = action_trace(&fake, "startChat");
            assert_eq!(calls.len(), 1);
            assert_eq!(calls[0]["role"], "host-role");
            assert_eq!(
                calls[0]["cwd"],
                if metadata && !changed {
                    "/role-directory"
                } else {
                    "/shown"
                }
            );
            for field in ["runtime", "model", "effort", "speed"] {
                assert!(calls[0][field].is_null(), "{field}");
            }
            view.read_with(&cx, |launcher, _| assert!(launcher.pending.is_some()));
            let mut visual = VisualTestContext::from_window(window.into(), &cx);
            view.update(&mut visual, |launcher, cx| {
                assert!(launcher.pending.is_some());
                launcher.selected = 0;
                cx.notify();
            });
            visual.run_until_parked();
        }
    }
}
