use super::launcher_model::{
    self as model, CrewEntry, EntryKey, KeyResult, RecentStore, SearchEntry, SlotLine,
};
use super::*;
use crate::surfaces::front_page::folder_picker::{FolderChosen, FolderPicker};
use crate::surfaces::front_page::{
    client::{ChatLaunch, ChatStart},
    FrontPageClient, FrontPageTarget,
};
use gpui::{canvas, Bounds, Pixels, Render, SharedString, WeakEntity};
use runner_core::protocol::worktree::{WorktreeCheckout, WorktreeSpec};
use runner_core::protocol::Runtime;
use std::cell::Cell;
use std::rc::Rc;

mod mission;
mod state;
mod worktree;
pub(crate) use mission::LauncherOrigin;
pub(crate) use state::LauncherState;
pub(super) use state::TerminalStart;

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
pub(super) struct LaunchRequest {
    pub(super) key: Option<EntryKey>,
    pub(super) pane: Option<PaneKey>,
    pub(super) scope: ProjectScope,
    pub(super) cwd: Option<String>,
    pub(super) worktree: Option<WorktreeSpec>,
    pub(super) initial_prompt: Option<String>,
}

#[derive(Clone)]
enum UpdateAction {
    Chat(LaunchRequest),
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
    checkout: Option<WorktreeCheckout>,
    checkout_gen: u64,
    checkout_anchor: Rc<Cell<Bounds<Pixels>>>,
    checkout_picker: Option<worktree::LauncherCheckoutPicker>,
    checkout_picker_gen: u64,
    new_worktree: Option<worktree::NewWorktreeState>,
    new_worktree_gen: u64,
    new_worktree_check_gen: u64,
    base_picker_gen: u64,
    #[cfg(test)]
    worktree_test_gates: worktree::WorktreeTestGates,
    step: Step,
    crews: Vec<CrewEntry>,
    goal: Entity<TextField>,
    slots: Option<Vec<SlotLine>>,
    origin: Option<LauncherOrigin>,
    sample_prompt: Option<Entity<TextField>>,
    sample_prompt_auto: Option<String>,
    store: Entity<AppStore>,
    subscriptions: Vec<Subscription>,
}

pub(super) fn launcher_input(
    key: &EntryKey,
    project: Option<String>,
    cwd: Option<String>,
    worktree: Option<WorktreeSpec>,
    size: (u16, u16),
    initial_prompt: Option<String>,
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
        worktree,
        size,
        initial_prompt,
    }
}

fn sample_prompt_for_folder(cwd: &str) -> String {
    let trimmed = cwd.trim();
    if trimmed.is_empty() {
        return crate::i18n::text("onboarding.sample_prompt_fallback").into();
    }
    let folder = Path::new(trimmed)
        .file_name()
        .and_then(|name| name.to_str())
        .filter(|name| !name.is_empty())
        .unwrap_or(trimmed);
    crate::i18n::tf(
        "onboarding.sample_prompt_folder",
        &[("folder", folder.to_owned()), ("path", trimmed.to_owned())],
    )
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
    pub(crate) fn has_update_retry(&self) -> bool {
        self.update_retry.is_some()
    }

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
            self.sync_sample_prompt_to_cwd(cx);
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
            checkout: None,
            checkout_gen: 0,
            checkout_anchor: Rc::new(Cell::new(Bounds::default())),
            checkout_picker: None,
            checkout_picker_gen: 0,
            new_worktree: None,
            new_worktree_gen: 0,
            new_worktree_check_gen: 0,
            base_picker_gen: 0,
            #[cfg(test)]
            worktree_test_gates: Default::default(),
            step: Step::Start,
            crews,
            goal,
            slots: None,
            origin: None,
            sample_prompt: None,
            sample_prompt_auto: None,
            store: root.app_store.clone(),
            subscriptions: vec![subscription],
        };
        if remote {
            this.load_host(root.remote_client.computer().cloned(), window, cx);
        } else if default_page {
            this.refresh_checkout_state(cx);
        }
        this
    }

    pub(crate) fn enable_sample_prompt(&mut self, _window: &mut Window, cx: &mut Context<Self>) {
        let prompt = sample_prompt_for_folder(self.cwd.read(cx).text());
        let field = cx.new(|input_cx| {
            let mut input = TextField::textarea(
                input_cx.focus_handle(),
                prompt.clone(),
                crate::i18n::text("onboarding.sample_prompt_label"),
                2,
                false,
            )
            .auto_grow(5)
            .text_size(rems(12. / 16.));
            input.set_bare(true, input_cx);
            input.set_right_padding(0., input_cx);
            input
        });
        self.sample_prompt = Some(field);
        self.sample_prompt_auto = Some(prompt);
        self.step = Step::Start;
        self.selected = 0;
        self.query.update(cx, |query, cx| query.reset("", cx));
        cx.notify();
    }

    fn sync_sample_prompt_to_cwd(&mut self, cx: &mut Context<Self>) {
        let Some(field) = self.sample_prompt.clone() else {
            return;
        };
        let next = sample_prompt_for_folder(self.cwd.read(cx).text());
        let previous = self.sample_prompt_auto.as_deref();
        if previous.is_some_and(|auto| field.read(cx).text() == auto) {
            field.update(cx, |input, cx| input.reset(next.clone(), cx));
            self.sample_prompt_auto = Some(next);
        }
    }

    #[cfg(test)]
    pub(crate) fn set_cwd_for_test(&mut self, cwd: &str, cx: &mut Context<Self>) {
        self.new_worktree = None;
        self.checkout_picker = None;
        self.cwd.update(cx, |input, cx| input.reset(cwd, cx));
        self.sync_sample_prompt_to_cwd(cx);
        self.refresh_checkout_state(cx);
        cx.notify();
    }

    #[cfg(test)]
    pub(crate) fn set_sample_prompt_for_test(&mut self, text: &str, cx: &mut Context<Self>) {
        if let Some(field) = &self.sample_prompt {
            field.update(cx, |input, cx| input.reset(text, cx));
            cx.notify();
        }
    }

    #[cfg(test)]
    pub(crate) fn sample_prompt_text_for_test(&self, cx: &App) -> Option<String> {
        self.sample_prompt
            .as_ref()
            .map(|field| field.read(cx).text().to_owned())
    }

    #[cfg(test)]
    pub(crate) fn set_update_retry_for_test(&mut self, runtime: Runtime, cx: &mut Context<Self>) {
        self.update_retry = Some(UpdateRetry {
            runtime,
            gate: None,
            action: UpdateAction::Chat(LaunchRequest {
                key: None,
                pane: None,
                scope: ProjectScope::Root,
                cwd: None,
                worktree: None,
                initial_prompt: None,
            }),
            requested: false,
            ready: false,
        });
        cx.notify();
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
                                    this.sync_sample_prompt_to_cwd(cx);
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
                if self.crews.is_empty() {
                    crate::i18n::text("launcher.no_crews_yet")
                } else {
                    crate::i18n::text("ui.work_with_a_crew_of_agents")
                },
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
            (Step::Goal(_), "enter") if !shift && self.new_worktree_allows_start() => {
                self.start_mission(window, cx);
                return true;
            }
            (Step::Goal(_), "enter") if !shift => {
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
        if self.pending.is_some()
            || unavailable
            || !self.client.valid()
            || !self.new_worktree_allows_start()
        {
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
        let worktree = self.worktree_spec();
        let worktree_cwd = self.worktree_repo_cwd();
        let pane = self.pane.clone();
        let launcher = cx.entity();
        match choice {
            Choice::Mission | Choice::EmptyMission | Choice::Crew(_) => {}
            Choice::Terminal => {
                let cwd = if worktree.is_some() {
                    worktree_cwd.or(path)
                } else {
                    path
                };
                let request = LaunchRequest {
                    key: None,
                    pane,
                    scope,
                    cwd,
                    worktree,
                    initial_prompt: None,
                };
                window.defer(cx, move |window, cx| {
                    root.update(cx, |root, cx| {
                        root.launch_from_launcher(launcher, request, false, window, cx)
                    })
                });
            }
            Choice::Chat(key) => {
                let role = match &key {
                    EntryKey::Role(id) => self.roles.iter().find(|role| &role.id == id),
                    _ => None,
                };
                let cwd = if worktree.is_some() {
                    worktree_cwd.or(path)
                } else {
                    launcher_directory(role, path, self.changed_directory)
                };
                let request = LaunchRequest {
                    key: Some(key),
                    pane,
                    scope,
                    cwd,
                    worktree,
                    initial_prompt: None,
                };
                window.defer(cx, move |window, cx| {
                    root.update(cx, |root, cx| {
                        root.launch_from_launcher(launcher, request, false, window, cx)
                    })
                });
            }
        }
        cx.notify();
    }

    fn choose_project(&mut self, id: &str, cx: &mut Context<Self>) {
        let current = self.cwd.read(cx).text().to_owned();
        let default = self.default_directory(cx);
        let start = if default.is_empty() {
            &current
        } else {
            &default
        };
        if let Some(path) = self.project.choose(id, start) {
            self.new_worktree = None;
            self.checkout_picker = None;
            self.error = None;
            if path != current.trim() {
                self.cwd.update(cx, |input, cx| input.reset(path, cx));
                self.sync_sample_prompt_to_cwd(cx);
            }
            self.refresh_checkout_details(cx);
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
                            this.new_worktree = None;
                            this.checkout_picker = None;
                            this.error = None;
                            this.project.picked(&path, cx);
                            this.cwd.update(cx, |input, cx| input.reset(path, cx));
                            this.changed_directory = true;
                            this.sync_sample_prompt_to_cwd(cx);
                            this.refresh_checkout_state(cx);
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
                && !root.panes.is_pane_closing(pane)
                && !root.remote_client.pane_closing(pane)
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
        let zero_crews = self.step == Step::Crews && self.crews.is_empty() && !self.loading;
        if zero_crews {
            let root_create = self.root.clone();
            let root_setup = self.root.clone();
            let remote = self.client.is_remote();
            card = card.child(
                div()
                    .debug_selector(|| "LAUNCHER_NO_CREWS".into())
                    .p_3()
                    .flex()
                    .flex_col()
                    .gap(rems(8. / 16.))
                    .child(
                        div()
                            .text_size(theme::text_ui())
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(theme::text())
                            .child(crate::i18n::text("launcher.no_crews_yet")),
                    )
                    .child(
                        div()
                            .whitespace_normal()
                            .text_size(theme::text_meta())
                            .text_color(theme::muted())
                            .child(crate::i18n::text("launcher.no_crews_hint")),
                    )
                    .when(!remote, |banner| {
                        banner.child(
                            div()
                                .flex()
                                .items_center()
                                .gap_2()
                                .child(
                                    Button::new(
                                        "launcher-no-crews-create",
                                        crate::i18n::text("launcher.create_crew"),
                                    )
                                    .debug_selector("LAUNCHER_NO_CREWS_CREATE")
                                    .size(runner_app::ui::ButtonSize::Sm)
                                    .variant(ButtonVariant::Secondary)
                                    .on_press(
                                        move |window, cx| {
                                            let _ = root_create.update(cx, |root, cx| {
                                                root.open_create_crew(window, cx);
                                            });
                                        },
                                    ),
                                )
                                .child(
                                    Button::new(
                                        "launcher-no-crews-setup",
                                        crate::i18n::text("launcher.open_setup"),
                                    )
                                    .debug_selector("LAUNCHER_NO_CREWS_SETUP")
                                    .icon("list-checks.svg")
                                    .size(runner_app::ui::ButtonSize::Sm)
                                    .variant(ButtonVariant::Secondary)
                                    .on_press(
                                        move |window, cx| {
                                            let _ = root_setup.update(cx, |root, cx| {
                                                root.open_setup(window, cx);
                                            });
                                        },
                                    ),
                                ),
                        )
                    }),
            );
            if !rows.is_empty() {
                card = card.child(div().h(px(1.)).mx_2().my_1().bg(theme::border()));
            }
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
            let start_allowed = self.new_worktree_allows_start();
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
                    .when(row.dimmed || !start_allowed, |row| row.opacity(0.45))
                    .when(start_allowed, |row| {
                        row.cursor_pointer()
                            .hover(|row| row.bg(theme::sidebar_selected()))
                    })
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
                                if !crate::surfaces::agent_recovery::runtime_updatable_for_version(
                                    runtime.name,
                                    runtime.installed_version.as_deref(),
                                ) {
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
            let checkout = self.render_checkout_control(cx);
            let (path_text, path_dimmed) = self.location_display_path(cx);
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
                .children(checkout)
                .child(
                    div()
                        .flex_1()
                        .min_w(px(0.))
                        .truncate()
                        .font_family(theme::UI_MONOSPACE_FONT)
                        .text_size(rems(11. / 16.))
                        .text_color(theme::faint())
                        .when(path_dimmed, |el| el.opacity(0.65))
                        .child(path_text),
                )
                .child(change)
        });
        let new_worktree_form = (!split)
            .then(|| self.render_new_worktree_form(cx))
            .flatten();
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
        let sample_prompt = (self.step == Step::Start && !split)
            .then_some(self.sample_prompt.as_ref())
            .flatten()
            .map(|prompt_field| {
                div()
                    .debug_selector(|| "LAUNCHER_SAMPLE_PROMPT".into())
                    .w_full()
                    .flex()
                    .flex_col()
                    .gap(rems(6. / 16.))
                    .px(rems(12. / 16.))
                    .py(rems(8. / 16.))
                    .rounded(rems(8. / 16.))
                    .border_1()
                    .border_color(theme::border())
                    .bg(theme::panel())
                    .child(
                        div()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_2()
                            .child(
                                div()
                                    .flex()
                                    .items_center()
                                    .gap(rems(6. / 16.))
                                    .child(
                                        svg()
                                            .path("message-square.svg")
                                            .size(rems(12. / 16.))
                                            .text_color(theme::muted()),
                                    )
                                    .child(
                                        div()
                                            .text_size(theme::text_meta())
                                            .font_weight(FontWeight::SEMIBOLD)
                                            .text_color(theme::muted())
                                            .child(crate::i18n::text(
                                                "onboarding.sample_prompt_label",
                                            )),
                                    ),
                            )
                            .child(
                                div()
                                    .debug_selector(|| "LAUNCHER_CLEAR_SAMPLE_PROMPT".into())
                                    .child(
                                        runner_app::ui::IconButton::new(
                                            "launcher-clear-sample-prompt",
                                            "close.svg",
                                        )
                                        .size(runner_app::ui::IconButtonSize::Sm)
                                        .tooltip(crate::i18n::text(
                                            "onboarding.clear_sample_prompt",
                                        ))
                                        .on_press({
                                            let this_entity = cx.weak_entity();
                                            move |_, cx| {
                                                let _ = this_entity.update(cx, |this, cx| {
                                                    this.sample_prompt = None;
                                                    this.sample_prompt_auto = None;
                                                    cx.notify();
                                                });
                                            }
                                        }),
                                    ),
                            ),
                    )
                    .child(prompt_field.clone())
            });
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
                    .children(new_worktree_form)
                    .children(sample_prompt)
                    .children(self.error.as_ref().map(|error| {
                        runner_app::ui::notice_banner(error.clone(), runner_app::ui::Tone::Danger)
                    }))
                    .when(!hide_card, |column| column.child(card))
                    .children(
                        self.runtimes
                            .iter()
                            .find(|runtime| {
                                runtime.available
                                    && runtime.below_minimum
                                    && crate::surfaces::agent_recovery::runtime_updatable_for_version(
                                        runtime.name,
                                        runtime.installed_version.as_deref(),
                                    )
                            })
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
            .children(self.render_checkout_picker(window, cx))
            .children(self.render_base_picker(window, cx))
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
        let updatable = crate::surfaces::agent_recovery::runtime_updatable_for_version(
            retry.runtime,
            retry
                .gate
                .as_ref()
                .map(|gate| gate.installed_version.as_str()),
        );
        let remote = self.client.is_remote();
        let task = updatable
            .then(|| {
                self.root.upgrade().and_then(|root| {
                    root.read(cx)
                        .target_task(&self.client.target, retry.runtime)
                        .cloned()
                })
            })
            .flatten();
        let answer = task.as_ref().is_some_and(|task| {
            task.state == runner_core::protocol::RuntimeUpdateState::MayNeedInput
        });
        let waiting = retry.requested && task.as_ref().is_some_and(|task| task.state.active());
        let output_task = task.clone();
        let name = retry.runtime.display_name();
        let title = if !updatable {
            crate::i18n::tf("agent_updates.too_old", &[("runtime", name.into())])
        } else if waiting {
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
                            (
                                "runtime",
                                crate::surfaces::agent_recovery::installed_runtime_name(
                                    gate.runtime,
                                )
                                .into(),
                            ),
                            ("installed", gate.installed_version.clone()),
                            ("minimum", gate.min_version.clone()),
                        ],
                    )
                })
            })
            .unwrap_or_else(|| {
                if updatable {
                    crate::i18n::tf("agent_updates.update_to_use", &[("runtime", name.into())])
                } else {
                    crate::i18n::tf(
                        "format.install_or_set_an_explicit_executable_path",
                        &[("runtime", name.into())],
                    )
                }
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
                    if !updatable {
                        if remote {
                            return;
                        }
                        let _ = owner.update(cx, |view, cx| {
                            view.update_retry = None;
                            cx.notify();
                        });
                        let _ = root.update(cx, |root, cx| {
                            root.start_chat_modal = None;
                            root.enter_settings_route(Some("agents"), window, cx);
                        });
                        return;
                    }
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
            .primary_debug_selector(if updatable {
                "LAUNCHER_UPDATE_ACTION"
            } else {
                "LAUNCHER_OPEN_SETTINGS"
            })
            .label(if !updatable {
                crate::i18n::text("ui.open_settings").into()
            } else if answer {
                crate::i18n::text("agent_updates.answer").into()
            } else {
                crate::i18n::tf("agent_updates.update_action", &[("runtime", name.into())])
            })
            .actions_visible(
                if updatable {
                    !waiting || answer
                } else {
                    !remote
                },
                false,
            ),
        )
    }
}

struct LocalCatalogSnapshot {
    roles: Vec<RoleChoice>,
    projects: Vec<ProjectChoice>,
    crews: Vec<CrewEntry>,
    revisions: (u64, u64, u64),
}

impl ChatLauncher {
    fn version_gate_for(
        &self,
        runtime: Runtime,
    ) -> Option<runner_core::protocol::RuntimeVersionGate> {
        self.runtimes
            .iter()
            .find(|entry| entry.name == runtime)
            .and_then(|entry| {
                Some(runner_core::protocol::RuntimeVersionGate {
                    runtime,
                    installed_version: entry.installed_version.clone()?,
                    min_version: entry.min_version.clone()?,
                })
            })
    }

    fn set_fork_update_retry(
        &mut self,
        gate: runner_core::protocol::RuntimeVersionGate,
        request: super::super::chat::ForkRequest,
        cx: &mut Context<Self>,
    ) {
        self.update_retry = Some(UpdateRetry {
            runtime: gate.runtime,
            gate: Some(gate),
            action: UpdateAction::Fork(request),
            requested: false,
            ready: false,
        });
        self.pending = None;
        cx.notify();
    }

    fn set_chat_update_retry(
        &mut self,
        runtime: Runtime,
        gate: Option<runner_core::protocol::RuntimeVersionGate>,
        request: LaunchRequest,
        requested: bool,
        cx: &mut Context<Self>,
    ) {
        self.pending = None;
        self.update_retry = Some(UpdateRetry {
            runtime,
            gate,
            action: UpdateAction::Chat(request),
            requested,
            ready: false,
        });
        cx.notify();
    }

    fn apply_remote_runtimes(
        &mut self,
        runtimes: &[runner_core::companion::CatalogRuntime],
        cx: &mut Context<Self>,
    ) {
        self.runtimes = runtimes.iter().filter_map(RuntimeChoice::remote).collect();
        cx.notify();
    }

    fn apply_local_refresh(
        &mut self,
        runtimes: &[RuntimeChoice],
        roles: &[RoleChoice],
        default_runtime: &str,
        keymap_overrides: &keymap::KeymapOverrides,
        error: Option<&String>,
        cx: &mut Context<Self>,
    ) {
        if !self.client.is_remote() {
            self.runtimes = runtimes.to_vec();
            self.roles = roles.to_vec();
            self.default_runtime = default_runtime.to_owned();
            self.keymap_overrides = keymap_overrides.clone();
            self.error = error.cloned();
            cx.notify();
        }
    }

    fn mark_update_ready(&mut self, runtime: Runtime, target: &FrontPageTarget) {
        if &self.client.target == target {
            if let Some(retry) = &mut self.update_retry {
                if retry.runtime == runtime && retry.requested {
                    retry.ready = true;
                }
            }
        }
    }

    fn carry_over_draft(
        &mut self,
        query: String,
        directory: Option<String>,
        cx: &mut Context<Self>,
    ) {
        self.query.update(cx, |field, cx| field.reset(query, cx));
        if let Some(directory) = directory {
            self.project.picked(&directory, cx);
            self.cwd.update(cx, |field, cx| field.reset(directory, cx));
            self.changed_directory = true;
            self.refresh_checkout_state(cx);
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn reconcile(
        &mut self,
        recent_revision: u64,
        catalogs: Option<LocalCatalogSnapshot>,
        project_deleted: bool,
        overrides: keymap::KeymapOverrides,
        computer: Option<runner_core::remote::Computer>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.recent_revision != recent_revision {
            self.reload_recents(cx);
        }
        if let Some(catalogs) = catalogs {
            if self.roles != catalogs.roles {
                self.roles = catalogs.roles;
                self.selected = 0;
                cx.notify();
            }
            if self.crews != catalogs.crews {
                self.crews = catalogs.crews;
                cx.notify();
            }
            self.project.set_choices(catalogs.projects, cx);
            self.catalog_revisions = catalogs.revisions;
        }
        if project_deleted {
            self.retarget_page(None, false, cx);
        }
        if self.keymap_overrides != overrides {
            self.keymap_overrides = overrides;
            cx.notify();
        }
        if self.client.is_remote() && !self.catalog_requested && self.client.supports(&["catalog"])
        {
            self.load_host(computer, window, cx);
        }
    }

    fn open_on_page(
        &mut self,
        origin: Option<LauncherOrigin>,
        project: Option<&str>,
        locked: bool,
        step: Step,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if let Some(origin) = origin {
            self.origin = Some(origin);
        }
        self.retarget_page(project, locked, cx);
        self.open_step(step, window, cx);
    }

    fn begin_terminal_start(&mut self, cx: &mut Context<Self>) -> Option<LaunchRequest> {
        if !self.client.supports(&["startShell", "layout"])
            || self.pending.is_some()
            || !self.new_worktree_allows_start()
        {
            return None;
        }
        self.pending = Some(Choice::Terminal);
        cx.notify();
        let worktree = self.worktree_spec();
        let cwd = if worktree.is_some() {
            self.worktree_repo_cwd()
                .or_else(|| normalized_value(self.cwd.read(cx).text()))
        } else {
            normalized_value(self.cwd.read(cx).text())
        };
        Some(LaunchRequest {
            key: None,
            pane: self.pane.clone(),
            scope: self.scope(),
            cwd,
            worktree,
            initial_prompt: None,
        })
    }

    fn resolve_initial_prompt(&self, initial_prompt: Option<String>, cx: &App) -> Option<String> {
        initial_prompt.or_else(|| {
            self.sample_prompt
                .as_ref()
                .and_then(|field| normalized_value(field.read(cx).text()))
        })
    }

    fn gated_runtime(&self, key: &EntryKey) -> Option<(Runtime, Option<String>)> {
        let runtime_key = match key {
            EntryKey::Agent(runtime) => Some(runtime.as_str()),
            EntryKey::Role(id) => self
                .roles
                .iter()
                .find(|role| role.id == *id)
                .map(|role| role.runtime.as_str()),
        }?;
        self.runtimes
            .iter()
            .find(|runtime| runtime.name.key() == runtime_key && runtime.below_minimum)
            .map(|runtime| (runtime.name, runtime.installed_version.clone()))
    }

    fn clear_pending(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        cx.notify();
    }

    fn set_error(&mut self, error: String, cx: &mut Context<Self>) {
        self.error = Some(error);
        cx.notify();
    }

    fn fail_missing_target_tab(&mut self, cx: &mut Context<Self>) {
        self.pending = None;
        self.error = Some(crate::i18n::text("ui.the_target_tab_is_no_longer_active").into());
        cx.notify();
    }

    #[allow(clippy::too_many_arguments)]
    fn build_chat_launch(
        &self,
        key: &EntryKey,
        scope: &ProjectScope,
        cwd: Option<String>,
        worktree: Option<WorktreeSpec>,
        placement: Option<(PaneLayout, String)>,
        size: (u16, u16),
        initial_prompt: Option<String>,
    ) -> ChatLaunch {
        ChatLaunch {
            input: launcher_input(
                key,
                scope.project_id().map(str::to_owned),
                cwd,
                worktree,
                size,
                initial_prompt,
            ),
            placement,
            started: self
                .started
                .as_ref()
                .filter(|(entry, _)| entry == key)
                .map(|(_, session)| session.clone()),
        }
    }

    fn finish_shell_launch(
        &mut self,
        remote: bool,
        result: &Result<crate::surfaces::front_page::actions::Completion, String>,
        cx: &mut Context<Self>,
    ) {
        if !remote || result.is_err() {
            self.pending = None;
        }
        self.error = result.as_ref().err().cloned();
        if let Ok(done) = result {
            self.launched_session = done.session.clone();
        }
        cx.notify();
    }

    fn finish_remote_chat_launch(
        &mut self,
        key: &EntryKey,
        started: Option<String>,
        result: &runner_core::protocol::ClientResult<
            crate::surfaces::front_page::client::ChatLaunched,
        >,
        cx: &mut Context<Self>,
    ) {
        if result.is_ok() {
            self.sample_prompt = None;
            self.sample_prompt_auto = None;
        } else {
            self.pending = None;
        }
        self.launched_session = result.as_ref().ok().map(|output| output.session.clone());
        self.started = started.map(|session| (key.clone(), session));
        self.error = result.as_ref().err().map(ToString::to_string);
        cx.notify();
    }

    fn finish_local_chat_launch(
        &mut self,
        key: &EntryKey,
        started: Option<String>,
        result: &runner_core::protocol::ClientResult<
            crate::surfaces::front_page::client::ChatLaunched,
        >,
        cx: &mut Context<Self>,
    ) {
        if result.is_ok() {
            self.sample_prompt = None;
            self.sample_prompt_auto = None;
        }
        self.pending = None;
        self.started = started.map(|session| (key.clone(), session));
        self.error = result.as_ref().err().map(ToString::to_string);
        cx.notify();
    }
}

fn resolve_pane_placement(
    pane: Option<PaneKey>,
    remote: bool,
    tabs: &TabSet,
    remote_client: &crate::surfaces::remote_client::RemoteClient,
) -> Option<(PaneLayout, String)> {
    pane.and_then(|pane| {
        let layout = if remote {
            remote_client.tab_layout(&pane.tab_id)
        } else {
            tabs.tabs().iter().find(|layout| layout.id == pane.tab_id)
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
    })
}

fn prepare_shell_start(
    placement: Option<(PaneLayout, String)>,
    scope: &ProjectScope,
    cwd: Option<String>,
    worktree: Option<WorktreeSpec>,
    size: (u16, u16),
) -> (
    crate::surfaces::front_page::actions::ShellStart,
    (PaneLayout, String),
) {
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
    let input = crate::surfaces::front_page::actions::ShellStart {
        draft,
        pane: Some(pane),
        project: scope.project_id().map(str::to_owned),
        cwd,
        worktree,
        size,
    };
    (input, local_placement)
}

fn spawn_shell_launch_task(
    remote: bool,
    operation: impl FnOnce() -> Result<crate::surfaces::front_page::actions::Completion, String>
        + Send
        + 'static,
    local_placement: (PaneLayout, String),
    placement_client: FrontPageClient,
    placement_project: Option<String>,
    mut tab_guard: Option<TabStart>,
    cx: &App,
) -> gpui::Task<(
    Option<TabStart>,
    Result<crate::surfaces::front_page::actions::Completion, String>,
)> {
    cx.background_spawn(async move {
        let result = (|| {
            let mut done = operation()?;
            if !remote {
                let (mut layout, pane) = local_placement;
                layout
                    .assign_session(&pane, done.session.as_ref().unwrap())
                    .map_err(|error| error.to_string())?;
                if let Err(error) = placement_client.launcher_layout(&layout, placement_project) {
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
    })
}

fn spawn_chat_launch_task(
    client: FrontPageClient,
    mut launch: ChatLaunch,
    directory: PathBuf,
    recent_key: EntryKey,
    use_role_directory: bool,
    mut tab_guard: Option<TabStart>,
    cx: &App,
) -> gpui::Task<(
    Option<TabStart>,
    ChatLaunch,
    runner_core::protocol::ClientResult<crate::surfaces::front_page::client::ChatLaunched>,
)> {
    let target = client.target.clone();
    let remote_target = client.is_remote();
    cx.background_spawn(async move {
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
    })
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
                root.launchers
                    .apply_remote_runtimes(&target, &catalog.runtimes, cx);
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
            launcher.set_fork_update_retry(gate, request, cx);
        });
    }

    pub(crate) fn update_launcher_visible(
        &self,
        launcher: &Entity<ChatLauncher>,
        cx: &App,
    ) -> bool {
        let target = self.front_page_client(cx).target;
        self.launchers.is_visible(
            launcher,
            &target,
            self.front_page_is_chat(),
            self.launcher_page_visible(),
            self.front_page_layout(),
            cx,
        )
    }

    pub(crate) fn drain_ready_launches(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let current = self.front_page_client(cx).target;
        for launcher in self.launchers.snapshot() {
            let Some(retry) = LauncherState::take_ready_launch(
                &launcher,
                &current,
                &self.app_store,
                &self.remote_client,
                cx,
            ) else {
                continue;
            };
            match retry.action {
                UpdateAction::Chat(request) => {
                    self.launch_from_launcher(launcher, request, true, window, cx)
                }
                UpdateAction::Mission(request) => {
                    self.launch_mission_from_launcher(launcher, request, true, window, cx)
                }
                UpdateAction::Fork(request) => self.retry_agent_fork(request, launcher, window, cx),
            }
        }
    }

    pub(crate) fn focus_visible_launcher(&self, window: &mut Window, cx: &mut App) -> bool {
        let target = self.front_page_client(cx).target;
        self.launchers.focus_visible(
            &target,
            self.front_page_is_chat(),
            self.create_modal_open(),
            self.launcher_page_visible(),
            self.front_page_layout(),
            &self.root_focus,
            window,
            cx,
        )
    }

    pub(crate) fn launcher_session_pending(&self, session: &str, cx: &App) -> bool {
        let target = self.front_page_client(cx).target;
        LauncherState::session_pending(
            &target,
            session,
            &self.tabs,
            &self.app_store,
            &self.remote_client,
            cx,
        )
    }

    pub(crate) fn launcher_tab_pending(&self, tab: &str, cx: &App) -> bool {
        let target = self.front_page_client(cx).target;
        LauncherState::tab_pending(&target, tab, &self.app_store, &self.remote_client, cx)
    }

    pub(crate) fn render_launcher(
        &mut self,
        pane: Option<PaneKey>,
        default_page: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Entity<ChatLauncher> {
        let default_page = LauncherState::effective_default_page(
            pane.as_ref(),
            default_page,
            self.front_page_layout(),
        );
        let target = self.front_page_client(cx).target;
        if let Some(launcher) = self.launchers.get(&target, pane.as_ref()).cloned() {
            if LauncherState::context_changed(
                &launcher,
                pane.as_ref(),
                default_page,
                || self.active_tab_project_id(cx),
                cx,
            ) {
                let root = cx.weak_entity();
                let next = cx.new(|cx| {
                    ChatLauncher::new(self, pane.clone(), default_page, window, cx, root)
                });
                return self.launchers.replace_with_carry_over(
                    &target,
                    pane.as_ref(),
                    &launcher,
                    next,
                    cx,
                );
            }
            let computer = self.remote_client.computer().cloned();
            let store = self.app_store.clone();
            LauncherState::reconcile_cached(&launcher, &target, &store, computer, window, cx);
            return launcher;
        }
        let locked_project = pane
            .is_none()
            .then(|| self.launchers.take_replaced_page_project(&target, cx))
            .flatten();
        let root = cx.weak_entity();
        let launcher =
            cx.new(|cx| ChatLauncher::new(self, pane.clone(), default_page, window, cx, root));
        if locked_project.is_some() {
            launcher.update(cx, |launcher, cx| {
                launcher.retarget_page(locked_project.as_deref(), true, cx);
            });
        }
        self.launchers
            .insert(&target, pane.as_ref(), launcher.clone());
        launcher
    }

    pub(super) fn new_tab_project_id(&self, cx: &App) -> Option<String> {
        let visible = self.launcher_page_visible();
        if !visible && self.remote_client.is_local() {
            return self.active_tab_project_id(cx);
        }
        let target = self.front_page_client(cx).target;
        let origin = self.launchers.page_origin(&target, visible, cx);
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
        let target = self.front_page_client(cx).target;
        if self.create_modal_open()
            || self
                .launchers
                .start_pending(&target, self.front_page_layout(), cx)
        {
            return;
        }
        let visible =
            self.launcher_page_visible() && !self.launchers.page_has_update_retry(&target, cx);
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
            self.setup.open = false;
            self.set_route(AppRoute::Launcher, cx);
        }
        if !visible {
            let target = self.front_page_client(cx).target;
            self.launchers.reset_page(&target, cx);
        }
        self.front_page_sidebar().update(cx, |sidebar, cx| {
            sidebar.set_active_project(scope.project_id().map(str::to_owned), cx)
        });
        let launcher = self.render_launcher(None, true, window, cx);
        launcher.update(cx, |launcher, cx| {
            launcher.open_on_page(
                (!visible).then_some(origin),
                scope.project_id(),
                locked,
                step,
                window,
                cx,
            );
        });
        cx.notify();
    }

    pub(crate) fn launcher_page_closable(&self, cx: &App) -> bool {
        let target = self.front_page_client(cx).target;
        self.launchers.page_closable(
            &target,
            self.launcher_page_visible(),
            self.remote_client.is_active(),
            !self.tabs.tabs().is_empty(),
            cx,
        )
    }

    pub(crate) fn close_launcher_page(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let target = self.front_page_client(cx).target;
        self.launchers.close_page(
            &target,
            self.launcher_page_visible(),
            self.remote_client.is_active(),
            !self.tabs.tabs().is_empty(),
            window,
            cx,
        );
    }

    pub(crate) fn launcher_page_visible(&self) -> bool {
        if self.route == AppRoute::Settings {
            return false;
        }
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
            launcher.update(cx, |view, cx| view.set_error(error.to_string(), cx));
            return None;
        }
        let _ = self.reload_tabs(cx);
        self.tabs.activate(&layout.id);
        self.set_route(AppRoute::Chat, cx);
        let pane = PaneKey::new(&layout.id, &layout.focused_pane_id);
        self.launchers
            .bind_reserved_pane(launcher, pane.clone(), scope, cx);
        Some(pane)
    }

    pub(super) fn launch_from_launcher(
        &mut self,
        launcher: Entity<ChatLauncher>,
        request: LaunchRequest,
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
        let initial_prompt = request.key.as_ref().and_then(|_| {
            launcher
                .read(cx)
                .resolve_initial_prompt(request.initial_prompt.clone(), cx)
        });
        let gated = request
            .key
            .as_ref()
            .and_then(|key| launcher.read(cx).gated_runtime(key));
        if let Some((runtime, installed)) = gated.filter(|(runtime, _)| {
            !self
                .target_task(&client.target, *runtime)
                .is_some_and(|task| {
                    task.state == runner_core::protocol::RuntimeUpdateState::Succeeded
                })
        }) {
            let updatable = crate::surfaces::agent_recovery::runtime_updatable_for_version(
                runtime,
                installed.as_deref(),
            );
            let pane = self.reserve_update_pane(&launcher, request.pane, &request.scope, cx);
            let retry_request = LaunchRequest {
                pane,
                initial_prompt,
                ..request
            };
            launcher.update(cx, |view, cx| {
                let gate = view.version_gate_for(runtime);
                view.set_chat_update_retry(runtime, gate, retry_request, updatable, cx);
            });
            if updatable {
                self.start_target_update(client, runtime, window, cx);
            }
            return;
        }
        let has_pane = request.pane.is_some();
        let mut placement = resolve_pane_placement(
            request.pane,
            client.is_remote(),
            &self.tabs,
            &self.remote_client,
        );
        if has_pane && placement.is_none() {
            launcher.update(cx, |launcher, cx| launcher.fail_missing_target_tab(cx));
            return;
        }
        if launcher.read(cx).default_page {
            if let Some((layout, _)) = placement.as_mut() {
                layout.parent_id = request.scope.project_id().map(str::to_owned);
            }
        }
        let tab_guard = if let Some((layout, _)) = &placement {
            let displayed_target = self.front_page_client(cx).target;
            let Some(guard) = LauncherState::begin_tab_start(
                &displayed_target,
                &client.target,
                layout,
                &self.app_store,
                &self.remote_client,
                cx,
            ) else {
                launcher.update(cx, |launcher, cx| launcher.clear_pending(cx));
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
        let Some(key) = request.key else {
            let (input, local_placement) = prepare_shell_start(
                placement,
                &request.scope,
                request.cwd,
                request.worktree,
                size,
            );
            use crate::surfaces::front_page::actions::WorkspaceAction;
            let Some((action, operation)) = WorkspaceAction::shell(self, input, cx) else {
                launcher.update(cx, |launcher, cx| launcher.clear_pending(cx));
                return;
            };
            let remote = action.client.is_remote();
            let scope = request.scope;
            let task = spawn_shell_launch_task(
                remote,
                operation,
                local_placement,
                client,
                scope.project_id().map(str::to_owned),
                tab_guard,
                cx,
            );
            cx.spawn_in(window, async move |weak, cx| {
                let (tab_guard, result) = task.await;
                drop(tab_guard);
                let _ = weak.update_in(cx, |root, window, cx| {
                    cx.defer_in(window, |root, window, cx| {
                        root.drain_ready_launches(window, cx)
                    });
                    launcher.update(cx, |launcher, cx| {
                        launcher.finish_shell_launch(remote, &result, cx);
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
                                        launcher.set_error(error.to_string(), cx);
                                    });
                                } else {
                                    if root.remote_client.is_local() {
                                        root.reveal_started_scope(&scope, cx);
                                    }
                                    root.launchers.remove(&launcher);
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
        };
        let scope = request.scope;
        let worktree = request.worktree;
        let launch = launcher.read(cx).build_chat_launch(
            &key,
            &scope,
            request.cwd,
            worktree.clone(),
            placement,
            size,
            initial_prompt,
        );
        let remote = client
            .is_remote()
            .then(|| {
                self.remote_client
                    .prepare_chat_launch(client.clone(), launch.placement.as_ref())
            })
            .flatten();
        if client.is_remote() && remote.is_none() {
            launcher.update(cx, |launcher, cx| launcher.clear_pending(cx));
            return;
        }
        let directory = self.app_store.read(cx).app_data_dir.clone();
        let use_role_directory = client.is_remote()
            && !launcher.read(cx).changed_directory
            && client.supports(&["role"]);
        let completion_target = client.target.clone();
        let task = spawn_chat_launch_task(
            client,
            launch,
            directory,
            key.clone(),
            use_role_directory,
            tab_guard,
            cx,
        );
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
                    let retry_request = LaunchRequest {
                        key: Some(key.clone()),
                        pane,
                        scope: scope.clone(),
                        cwd: launch.input.cwd.clone(),
                        worktree: worktree.clone(),
                        initial_prompt: launch.input.initial_prompt.clone(),
                    };
                    launcher.update(cx, |view, cx| {
                        view.set_chat_update_retry(
                            gate.runtime,
                            Some(gate),
                            retry_request,
                            false,
                            cx,
                        );
                    });
                }
                if let Some(action) = remote {
                    if !root.remote_client.settle_chat_launch(&action) {
                        return;
                    }
                    launcher.update(cx, |launcher, cx| {
                        launcher.finish_remote_chat_launch(&key, launch.started, &result, cx);
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
                        launcher.finish_local_chat_launch(
                            &key,
                            launch.started.clone(),
                            &result,
                            cx,
                        );
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
                        root.launchers.remove(&launcher);
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
        self.launch_from_launcher(
            launcher,
            LaunchRequest {
                key: Some(key),
                pane,
                scope,
                cwd,
                worktree: None,
                initial_prompt: None,
            },
            false,
            window,
            cx,
        );
    }
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod remote_tests;
